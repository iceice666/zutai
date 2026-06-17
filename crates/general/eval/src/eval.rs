//! THIR tree-walk evaluator — the only THIR-specific file in this crate.
//!
//! This is deliberately the single "swappable" file: when TLC is built, a
//! parallel `eval_tlc.rs` will be added here reusing everything in `value`,
//! `thunk`, and `env` unchanged.
//!
//! The `Evaluator` holds a module registry (`&[ThirFile]`) plus an
//! `active_module: ModuleId` index.  Arena helpers route through the active
//! file.  When applying a cross-module closure, callers use `for_module(home)`
//! to obtain a copy with the correct active file before calling `eval`.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use zutai_hir::BindingId;
use zutai_syntax::ast::BinOp;
use zutai_thir::{
    ImportKey, ThirClause, ThirDeclId, ThirDeclKind, ThirExprId, ThirExprKind, ThirFile, ThirPatId,
};
use zutai_thir::{
    ThirPatKind, ThirTupleItem, ThirTuplePatItem, Type, TypeId, TypeKind, TypeTupleItem,
};

use crate::{
    EvalError,
    env::Env,
    thunk::Thunk,
    value::{Closure, ModuleId, TupleField, Value, values_equal},
};

/// A slice of all evaluated modules for this run, keyed by position = `ModuleId`.
pub type ModuleRegistry = Vec<Arc<ThirFile>>;

/// Holds read-only access to the THIR arenas while evaluating.
///
/// `Evaluator` is cheaply `Copy` — it's two references plus a `usize`.
/// Use `for_module(m)` to get a copy that operates in module `m`'s arenas.
#[derive(Clone, Copy)]
pub struct Evaluator<'a> {
    /// The *active* file — arena helpers read from here.
    pub file: &'a ThirFile,
    /// All evaluated modules in this run.  Index = `ModuleId(i).0`.
    pub registry: &'a [Arc<ThirFile>],
    /// Which module is currently active.  Matches `file`.
    pub active_module: ModuleId,
    /// Pre-resolved import values (`.zti` data + `.zt` final-expr values),
    /// keyed by import source.  An `Import` node looks up its source here.
    pub imports: &'a HashMap<ImportKey, Value>,
}

impl<'a> Evaluator<'a> {
    pub fn new(
        file: &'a ThirFile,
        registry: &'a [Arc<ThirFile>],
        active_module: ModuleId,
        imports: &'a HashMap<ImportKey, Value>,
    ) -> Self {
        Self {
            file,
            registry,
            active_module,
            imports,
        }
    }

    /// Return a copy of this evaluator re-pointed at module `m`.
    ///
    /// Used by `apply_closure` and `Thunk::force` to switch arenas when
    /// evaluating a closure or thunk that was created in a different module.
    pub fn for_module(&self, m: ModuleId) -> Self {
        Self {
            file: &self.registry[m.0],
            active_module: m,
            registry: self.registry,
            imports: self.imports,
        }
    }

    /// Create a deferred thunk for `expr` in the current module.
    ///
    /// Stamps `home = self.active_module` so the thunk evaluates against the
    /// correct arena regardless of which module forces it later.
    pub fn defer(&self, expr: ThirExprId, env: Env) -> Thunk {
        Thunk::deferred(expr, env, self.active_module)
    }

    // ── arena helpers ────────────────────────────────────────────────────────

    fn expr(&self, id: ThirExprId) -> &'a zutai_thir::ThirExpr {
        &self.file.expr_arena[id]
    }

    fn pat(&self, id: ThirPatId) -> &'a zutai_thir::ThirPat {
        &self.file.pat_arena[id]
    }

    fn decl(&self, id: ThirDeclId) -> &'a zutai_thir::ThirDecl {
        &self.file.decl_arena[id]
    }

    // ── main entry point ─────────────────────────────────────────────────────

    /// Evaluate expression `id` in environment `env`, returning a `Value`.
    ///
    /// This does NOT force thunks for sub-expressions; those are only forced
    /// when the runtime semantics require it (e.g. the condition of `if`).
    pub fn eval(&self, id: ThirExprId, env: &Env) -> Result<Value, EvalError> {
        let expr = self.expr(id);
        match &expr.kind {
            // ── literals ─────────────────────────────────────────────────────
            ThirExprKind::True => Ok(Value::Bool(true)),
            ThirExprKind::False => Ok(Value::Bool(false)),
            ThirExprKind::Integer(n) => Ok(Value::Int(*n)),
            ThirExprKind::Float(f) => Ok(Value::Float(*f)),
            ThirExprKind::String(s) => Ok(Value::Text(Rc::from(s.as_str()))),
            ThirExprKind::Atom(a) => Ok(Value::Atom(Rc::from(a.as_str()))),
            ThirExprKind::TypeValue(ty) => Ok(Value::TypeValue(*ty)),
            ThirExprKind::TaggedValue { tag, payload } => {
                let payload_val = self.eval(*payload, env)?;
                let fields = match payload_val {
                    Value::Record(f) => (*f).clone(),
                    _ => vec![],
                };
                Ok(Value::TaggedValue {
                    tag: Rc::from(tag.as_str()),
                    payload: Rc::new(fields),
                })
            }

            // ── binding reference ────────────────────────────────────────────
            ThirExprKind::BindingRef(b) => {
                let thunk = env.lookup(*b)?;
                thunk.force(self)
            }

            // ── data constructors (lazy) ─────────────────────────────────────
            ThirExprKind::List(items) => {
                let thunks: Rc<[Thunk]> = items
                    .iter()
                    .map(|&item| self.defer(item, env.clone()))
                    .collect();
                Ok(Value::List(thunks))
            }
            ThirExprKind::Record(fields) => {
                let vec: Vec<(Rc<str>, Thunk)> = fields
                    .iter()
                    .map(|f| (Rc::from(f.name.as_str()), self.defer(f.value, env.clone())))
                    .collect();
                Ok(Value::Record(Rc::new(vec)))
            }
            ThirExprKind::Tuple(items) => {
                let fields: Rc<[TupleField]> = items
                    .iter()
                    .map(|item| match item {
                        ThirTupleItem::Named { name, value, .. } => TupleField {
                            name: Some(Rc::from(name.as_str())),
                            value: self.defer(*value, env.clone()),
                        },
                        ThirTupleItem::Positional(e) => TupleField {
                            name: None,
                            value: self.defer(*e, env.clone()),
                        },
                    })
                    .collect();
                Ok(Value::Tuple(fields))
            }

            // ── block ─────────────────────────────────────────────────────────
            ThirExprKind::Block { bindings, result } => {
                let child = env.push_frame();
                for local in bindings {
                    // Each local captures the env extended so far (sequential
                    // scoping, matching lower_block_expr).
                    let thunk = self.defer(local.value, child.clone());
                    child.insert(local.binding, thunk);
                }
                self.eval(*result, &child)
            }

            // ── conditional ──────────────────────────────────────────────────
            ThirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cv = self.eval(*cond, env)?;
                match cv {
                    Value::Bool(true) => self.eval(*then_branch, env),
                    Value::Bool(false) => self.eval(*else_branch, env),
                    other => Err(EvalError::TypeMismatch {
                        expected: "Bool",
                        found: value_type_name(&other),
                    }),
                }
            }

            // ── binary operators ──────────────────────────────────────────────
            ThirExprKind::Binary { op, lhs, rhs } => self.eval_binary(*op, *lhs, *rhs, env),

            // ── field access ─────────────────────────────────────────────────
            ThirExprKind::Access { receiver, field } => {
                let rv = self.eval(*receiver, env)?;
                match rv {
                    Value::Record(fields) => {
                        for (name, thunk) in fields.iter() {
                            if name.as_ref() == field.as_str() {
                                return thunk.force(self);
                            }
                        }
                        // Record field access on a record where the field is
                        // absent means optional + was not present.
                        Ok(Value::Nothing)
                    }
                    Value::TaggedValue { tag, payload } => {
                        if field == "tag" {
                            return Ok(Value::Atom(tag));
                        }
                        for (name, thunk) in payload.iter() {
                            if name.as_ref() == field.as_str() {
                                return thunk.force(self);
                            }
                        }
                        Ok(Value::Nothing)
                    }
                    other => Err(EvalError::TypeMismatch {
                        expected: "Record",
                        found: value_type_name(&other),
                    }),
                }
            }

            // ── function application ──────────────────────────────────────────
            ThirExprKind::Apply {
                func,
                arg,
                instantiation,
            } => {
                // Witness-dict injection: if func is a BindingRef to a top-level
                // function with param_bounds, inject a WitnessDict for each
                // concrete (non-ambiguous) bound into the caller's env so that
                // method dispatch inside the body can fall back to it.
                if let ThirExprKind::BindingRef(bid) = &self.expr(*func).kind {
                    let bid = *bid;
                    // Find the Function decl with params/param_bounds.
                    let maybe_bounds: Option<(Vec<BindingId>, Vec<Vec<BindingId>>)> =
                        self.file.decls.iter().find_map(|&did| {
                            let decl = self.decl(did);
                            if decl.binding == bid
                                && let ThirDeclKind::Function {
                                    params,
                                    param_bounds,
                                    ..
                                } = &decl.kind
                            {
                                return Some((params.clone(), param_bounds.clone()));
                            }
                            None
                        });
                    if let Some((params, param_bounds)) = maybe_bounds {
                        let aliases = self.build_alias_map();
                        for (i, constraint_bindings) in param_bounds.iter().enumerate() {
                            if i >= params.len() || i >= instantiation.len() {
                                break;
                            }
                            let key = type_key(&self.file.type_arena, &aliases, instantiation[i]);
                            if key.starts_with('@') || key.starts_with('?') || key.starts_with('$')
                            {
                                continue; // ambiguous — can't resolve witness
                            }
                            for &constraint_binding in constraint_bindings {
                                // Find a matching witness.
                                for &decl_id in &self.file.decls {
                                    let decl = self.decl(decl_id);
                                    if let ThirDeclKind::Witness {
                                        constraint: Some(c),
                                        target,
                                        fields,
                                        ..
                                    } = &decl.kind
                                    {
                                        if *c != constraint_binding
                                            || type_key(&self.file.type_arena, &aliases, *target)
                                                != key
                                        {
                                            continue;
                                        }
                                        // Build the witness dict from the witness fields.
                                        let mut dict: HashMap<String, Value> = HashMap::new();
                                        for field in fields {
                                            let v = self.eval(field.value, env)?;
                                            dict.insert(field.name.clone(), v);
                                        }
                                        // NOTE: injecting into the caller's frame only works when
                                        // the callee's closure.env is an ancestor of this frame —
                                        // i.e. direct top-level calls. Indirect calls (bounded fn
                                        // called from another fn) won't see this dict because
                                        // apply_closure builds the body env as
                                        // closure.env.push_frame(), not as env.push_frame(). Those
                                        // cases are caught by try_method_dispatch returning
                                        // EvalError::UnresolvedWitness. Full dictionary-passing
                                        // (threading witnesses through call chains) is deferred to
                                        // the TLC elaboration layer.
                                        //
                                        // Keyed by constraint BindingId. Limitation: if two
                                        // distinct type params are bounded by the same constraint
                                        // (e.g. <A: Eq, B: Eq>), the second insertion clobbers
                                        // the first. Document here but don't fix — the
                                        // indirect-call limitation (see above) is the more
                                        // fundamental boundary.
                                        env.insert(
                                            constraint_binding,
                                            Thunk::ready(Value::WitnessDict(dict)),
                                        );
                                        break; // one witness per constraint
                                    }
                                }
                            }
                        }
                    }
                }

                // Type-directed constraint dispatch: if func is a BindingRef to a
                // named constraint method, look up the matching witness and use its
                // field body as the function value. Falls through to normal eval if
                // no witness matches (returns UnboundBinding via env.lookup).
                let fv = if let Some(v) = self.try_method_dispatch(*func, instantiation, env)? {
                    v
                } else {
                    self.eval(*func, env)?
                };
                match fv {
                    Value::Closure(c) => {
                        let mut applied = c.applied.clone();
                        // The arg is evaluated in the *caller's* module (this
                        // evaluator's active_module), not in the closure's home.
                        applied.push(self.defer(*arg, env.clone()));
                        if applied.len() < c.arity {
                            // Partial application — return a new closure.
                            Ok(Value::Closure(Rc::new(Closure {
                                binding: c.binding,
                                arity: c.arity,
                                clauses: c.clauses.clone(),
                                env: c.env.clone(),
                                applied,
                                home: c.home,
                            })))
                        } else {
                            // All arguments present — try each clause.
                            self.apply_closure(&c, applied)
                        }
                    }
                    other => Err(EvalError::TypeMismatch {
                        expected: "Function",
                        found: value_type_name(&other),
                    }),
                }
            }

            // ── lambda / match ───────────────────────────────────────────────
            ThirExprKind::Lambda { params, body } => {
                let clause = ThirClause {
                    patterns: params.clone(),
                    guard: None,
                    body: *body,
                    span: expr.span,
                };
                let closure = Closure {
                    binding: None,
                    arity: params.len(),
                    clauses: Rc::from([clause]),
                    env: env.clone(),
                    applied: Vec::new(),
                    home: self.active_module,
                };
                Ok(Value::Closure(Rc::new(closure)))
            }
            ThirExprKind::Match { scrutinee, arms } => {
                let sv = self.eval(*scrutinee, env)?;
                let scrutinee_thunk = Thunk::ready(sv);
                for arm in arms {
                    debug_assert_eq!(
                        arm.patterns.len(),
                        1,
                        "match arm must have exactly 1 pattern"
                    );
                    let mut child = env.push_frame();
                    if self.match_pattern(arm.patterns[0], scrutinee_thunk.clone(), &mut child)? {
                        if let Some(guard_id) = arm.guard {
                            match self.eval(guard_id, &child)? {
                                Value::Bool(true) => {}
                                Value::Bool(false) => continue,
                                other => {
                                    return Err(EvalError::TypeMismatch {
                                        expected: "Bool",
                                        found: value_type_name(&other),
                                    });
                                }
                            }
                        }
                        return self.eval(arm.body, &child);
                    }
                }
                Err(EvalError::NoMatchingClause)
            }
            ThirExprKind::Import(source) => match self.imports.get(source) {
                Some(value) => Ok(value.clone()),
                None => Err(EvalError::Internal(
                    "import not resolved (unreachable past gate)",
                )),
            },
            ThirExprKind::OptionalAccess { receiver, field } => {
                let rv = self.eval(*receiver, env)?;
                match rv {
                    Value::Nothing => Ok(Value::Nothing),
                    Value::Record(fields) => {
                        for (name, thunk) in fields.iter() {
                            if name.as_ref() == field.as_str() {
                                return thunk.force(self);
                            }
                        }
                        Ok(Value::Nothing)
                    }
                    other => Err(EvalError::TypeMismatch {
                        expected: "Record or Nothing",
                        found: value_type_name(&other),
                    }),
                }
            }
            ThirExprKind::Error => Err(EvalError::Internal(
                "Error node reached evaluator (unreachable past gate)",
            )),
        }
    }

    // ── binary operator dispatch ──────────────────────────────────────────────

    fn eval_binary(
        &self,
        op: BinOp,
        lhs: ThirExprId,
        rhs: ThirExprId,
        env: &Env,
    ) -> Result<Value, EvalError> {
        // Short-circuit operators first (do not force rhs eagerly).
        match op {
            BinOp::And => {
                let lv = self.eval(lhs, env)?;
                match lv {
                    Value::Bool(false) => return Ok(Value::Bool(false)),
                    Value::Bool(true) => {
                        let rv = self.eval(rhs, env)?;
                        return match rv {
                            Value::Bool(b) => Ok(Value::Bool(b)),
                            other => Err(EvalError::TypeMismatch {
                                expected: "Bool",
                                found: value_type_name(&other),
                            }),
                        };
                    }
                    other => {
                        return Err(EvalError::TypeMismatch {
                            expected: "Bool",
                            found: value_type_name(&other),
                        });
                    }
                }
            }
            BinOp::Or => {
                let lv = self.eval(lhs, env)?;
                match lv {
                    Value::Bool(true) => return Ok(Value::Bool(true)),
                    Value::Bool(false) => {
                        let rv = self.eval(rhs, env)?;
                        return match rv {
                            Value::Bool(b) => Ok(Value::Bool(b)),
                            other => Err(EvalError::TypeMismatch {
                                expected: "Bool",
                                found: value_type_name(&other),
                            }),
                        };
                    }
                    other => {
                        return Err(EvalError::TypeMismatch {
                            expected: "Bool",
                            found: value_type_name(&other),
                        });
                    }
                }
            }
            BinOp::Coalesce => {
                let lv = self.eval(lhs, env)?;
                return match lv {
                    Value::Nothing => self.eval(rhs, env),
                    other => Ok(other),
                };
            }
            _ => {}
        }

        // Eager operands for all remaining operators.
        let lv = self.eval(lhs, env)?;
        let rv = self.eval(rhs, env)?;

        match op {
            BinOp::Add => numeric_binop(lv, rv, i64::checked_add, f64::add, "+"),
            BinOp::Sub => numeric_binop(lv, rv, i64::checked_sub, f64::sub, "-"),
            BinOp::Mul => numeric_binop(lv, rv, i64::checked_mul, f64::mul, "*"),
            BinOp::Div => match (&lv, &rv) {
                (Value::Int(_), Value::Int(0)) => Err(EvalError::DivByZero),
                _ => numeric_binop(lv, rv, i64::checked_div, f64::div, "/"),
            },
            BinOp::Eq => {
                if let Some(v) =
                    self.try_operator_dispatch(BinOp::Eq, self.expr(lhs).ty, &lv, &rv, env)?
                {
                    return Ok(v);
                }
                // D5: if the operand type is unresolved and an equality witness
                // exists, refuse rather than silently returning a structural bool.
                let aliases = self.build_alias_map();
                let key = type_key(&self.file.type_arena, &aliases, self.expr(lhs).ty);
                if key_is_ambiguous(&key) && self.has_eq_operator_witness() {
                    return Err(EvalError::Internal(
                        "equality dispatch: unresolved operand type with (==) witnesses in scope",
                    ));
                }
                let eq = values_equal(&lv, &rv, self)?;
                Ok(Value::Bool(eq))
            }
            BinOp::Ne => {
                if let Some(v) =
                    self.try_operator_dispatch(BinOp::Ne, self.expr(lhs).ty, &lv, &rv, env)?
                {
                    return Ok(v);
                }
                let aliases = self.build_alias_map();
                let key = type_key(&self.file.type_arena, &aliases, self.expr(lhs).ty);
                if key_is_ambiguous(&key) && self.has_eq_operator_witness() {
                    return Err(EvalError::Internal(
                        "equality dispatch: unresolved operand type with (==) witnesses in scope",
                    ));
                }
                let eq = values_equal(&lv, &rv, self)?;
                Ok(Value::Bool(!eq))
            }
            BinOp::Lt => {
                if let Some(v) =
                    self.try_operator_dispatch(BinOp::Lt, self.expr(lhs).ty, &lv, &rv, env)?
                {
                    return Ok(v);
                }
                cmp_op(lv, rv, std::cmp::Ordering::Less, false)
            }
            BinOp::Le => {
                if let Some(v) =
                    self.try_operator_dispatch(BinOp::Le, self.expr(lhs).ty, &lv, &rv, env)?
                {
                    return Ok(v);
                }
                cmp_op(lv, rv, std::cmp::Ordering::Less, true)
            }
            BinOp::Gt => {
                if let Some(v) =
                    self.try_operator_dispatch(BinOp::Gt, self.expr(lhs).ty, &lv, &rv, env)?
                {
                    return Ok(v);
                }
                cmp_op(lv, rv, std::cmp::Ordering::Greater, false)
            }
            BinOp::Ge => {
                if let Some(v) =
                    self.try_operator_dispatch(BinOp::Ge, self.expr(lhs).ty, &lv, &rv, env)?
                {
                    return Ok(v);
                }
                cmp_op(lv, rv, std::cmp::Ordering::Greater, true)
            }
            // Already handled above.
            BinOp::And | BinOp::Or | BinOp::Coalesce => unreachable!(),
        }
    }

    // ── clause / pattern matching ─────────────────────────────────────────────

    /// Try all clauses of `closure` with the given argument thunks.
    ///
    /// Switches to the closure's home module before evaluating clause bodies
    /// and guards so arena look-ups (`expr_arena`, `pat_arena`) hit the file
    /// where the clause was originally lowered.
    fn apply_closure(&self, closure: &Closure, args: Vec<Thunk>) -> Result<Value, EvalError> {
        let home_ev = self.for_module(closure.home);
        for clause in closure.clauses.iter() {
            let mut child = closure.env.push_frame();
            if home_ev.match_all_patterns(&clause.patterns, &args, &mut child)? {
                // Check guard (if any) in the home module.
                if let Some(guard_id) = clause.guard {
                    match home_ev.eval(guard_id, &child)? {
                        Value::Bool(true) => {}
                        Value::Bool(false) => continue,
                        other => {
                            return Err(EvalError::TypeMismatch {
                                expected: "Bool",
                                found: value_type_name(&other),
                            });
                        }
                    }
                }
                return home_ev.eval(clause.body, &child);
            }
        }
        Err(EvalError::NoMatchingClause)
    }

    /// Match a sequence of patterns against a sequence of argument thunks.
    /// Returns `true` if all match and `child` is populated with bindings.
    fn match_all_patterns(
        &self,
        pattern_ids: &[ThirPatId],
        args: &[Thunk],
        child: &mut Env,
    ) -> Result<bool, EvalError> {
        debug_assert_eq!(pattern_ids.len(), args.len());
        for (&pat_id, thunk) in pattern_ids.iter().zip(args.iter()) {
            if !self.match_pattern(pat_id, thunk.clone(), child)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Match a single pattern against a thunk.  Populates bindings into
    /// `child_env` and returns whether the match succeeded.
    fn match_pattern(
        &self,
        pat_id: ThirPatId,
        thunk: Thunk,
        child_env: &mut Env,
    ) -> Result<bool, EvalError> {
        let pat = self.pat(pat_id);
        match &pat.kind {
            ThirPatKind::Error => Err(EvalError::Internal(
                "Error pattern reached evaluator (unreachable past gate)",
            )),
            ThirPatKind::Wildcard => Ok(true),
            ThirPatKind::Bind(b) => {
                // Insert the thunk UNFORCED — lazy binding.
                child_env.insert(*b, thunk);
                Ok(true)
            }
            ThirPatKind::True => match thunk.force(self)? {
                Value::Bool(true) => Ok(true),
                _ => Ok(false),
            },
            ThirPatKind::False => match thunk.force(self)? {
                Value::Bool(false) => Ok(true),
                _ => Ok(false),
            },
            ThirPatKind::Integer(n) => match thunk.force(self)? {
                Value::Int(v) => Ok(v == *n),
                _ => Ok(false),
            },
            ThirPatKind::Float(f) => match thunk.force(self)? {
                Value::Float(v) => Ok(v == *f),
                _ => Ok(false),
            },
            ThirPatKind::String(s) => match thunk.force(self)? {
                Value::Text(v) => Ok(v.as_ref() == s.as_str()),
                _ => Ok(false),
            },
            ThirPatKind::Atom(a) => match thunk.force(self)? {
                Value::Atom(v) => Ok(v.as_ref() == a.as_str()),
                _ => Ok(false),
            },
            ThirPatKind::Tuple(items) => {
                let v = thunk.force(self)?;
                match v {
                    Value::Tuple(fields) => {
                        if fields.len() != items.len() {
                            return Ok(false);
                        }
                        // Clone the pattern items so we can iterate with access
                        // to child_env without borrowing issues.
                        let items_owned: Vec<_> = items.clone();
                        for (item, field) in items_owned.iter().zip(fields.iter()) {
                            match item {
                                ThirTuplePatItem::Named { name, pattern, .. } => {
                                    if field.name.as_deref() != Some(name.as_str()) {
                                        return Ok(false);
                                    }
                                    if !self.match_pattern(
                                        *pattern,
                                        field.value.clone(),
                                        child_env,
                                    )? {
                                        return Ok(false);
                                    }
                                }
                                ThirTuplePatItem::Positional(p) => {
                                    if field.name.is_some() {
                                        return Ok(false);
                                    }
                                    if !self.match_pattern(*p, field.value.clone(), child_env)? {
                                        return Ok(false);
                                    }
                                }
                            }
                        }
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            ThirPatKind::Record(pat_fields) => {
                let v = thunk.force(self)?;
                match v {
                    Value::Record(rec_fields) => {
                        let pat_fields_owned: Vec<_> = pat_fields.clone();
                        for pf in &pat_fields_owned {
                            // Find the field in the record by name.
                            let maybe_thunk = rec_fields
                                .iter()
                                .find(|(n, _)| n.as_ref() == pf.name.as_str())
                                .map(|(_, t)| t.clone());
                            match maybe_thunk {
                                None => return Ok(false),
                                Some(t) => {
                                    if !self.match_pattern(pf.pattern, t, child_env)? {
                                        return Ok(false);
                                    }
                                }
                            }
                        }
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            ThirPatKind::TaggedValue {
                tag: pat_tag,
                payload: pat_fields,
            } => {
                let v = thunk.force(self)?;
                match v {
                    Value::TaggedValue {
                        tag: val_tag,
                        payload,
                    } => {
                        if val_tag.as_ref() != pat_tag.as_str() {
                            return Ok(false);
                        }
                        let pat_fields_owned: Vec<_> = pat_fields.clone();
                        for pf in &pat_fields_owned {
                            let maybe_thunk = payload
                                .iter()
                                .find(|(n, _)| n.as_ref() == pf.name.as_str())
                                .map(|(_, t)| t.clone());
                            match maybe_thunk {
                                None => return Ok(false),
                                Some(t) => {
                                    if !self.match_pattern(pf.pattern, t, child_env)? {
                                        return Ok(false);
                                    }
                                }
                            }
                        }
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
        }
    }

    // ── constraint method dispatch ────────────────────────────────────────────

    /// Try to dispatch a constraint method call to a matching witness field.
    ///
    /// Returns `Ok(Some(value))` when `func` is a `BindingRef` to a named
    /// constraint method AND a witness with a matching target type is in scope.
    /// Returns `Ok(None)` in all other cases (not a method, no instantiation,
    /// no concrete witness found) — the caller then falls through to normal `eval`.
    /// Returns `Err(EvalError::UnresolvedWitness)` when the type key is ambiguous
    /// (TypeVar inside a bounded function body) AND no WitnessDict is visible in
    /// the current env — this covers the indirect-call case where the witness
    /// was injected into the caller's frame but is not an ancestor of the callee's
    /// captured env.
    fn try_method_dispatch(
        &self,
        func: ThirExprId,
        instantiation: &[TypeId],
        env: &Env,
    ) -> Result<Option<Value>, EvalError> {
        // Step 1: func must be a BindingRef to a constraint method.
        let method_binding = match &self.expr(func).kind {
            ThirExprKind::BindingRef(b) => *b,
            _ => return Ok(None),
        };

        // Step 2: find which constraint owns this method binding.
        let mut found = None;
        'outer: for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::Constraint { methods, .. } = &decl.kind {
                for m in methods {
                    if m.binding == Some(method_binding) {
                        found = Some((decl.binding, m.name.clone()));
                        break 'outer;
                    }
                }
            }
        }
        let (constraint_binding, method_name) = match found {
            Some(x) => x,
            None => return Ok(None),
        };

        // Step 3: guard — only dispatch when exactly one type var was instantiated.
        if instantiation.len() != 1 {
            return Ok(None);
        }
        let aliases = self.build_alias_map();
        let key = type_key(&self.file.type_arena, &aliases, instantiation[0]);

        // Step 4: dispatch based on key ambiguity.
        //
        // **Concrete key** (not starting with `@`, `?`, `$`):
        //   - Scan witnesses for a matching (constraint_binding, target_key).
        //   - If a matching witness is found and contains the field → return it.
        //   - If a matching witness is found but omits the field AND the method
        //     has a default body → return the default closure (valid omission).
        //   - If no matching witness exists at all → return Ok(None) so the
        //     caller falls through to normal eval (UnboundBinding).
        //
        // **Ambiguous key** (TypeVar / InferVar / AliasApply, starts with `@`, `?`, `$`):
        //   - Try env fallback: look up the constraint binding; if it holds a
        //     WitnessDict injected at the direct call site, dispatch from it.
        //   - If env fallback misses → return EvalError::UnresolvedWitness.
        //   - NEVER fall through to the default-body fallback for ambiguous keys:
        //     the failure here is "can't resolve which witness to use", not
        //     "witness omitted an optional method".

        let key_is_concrete =
            !key.starts_with('@') && !key.starts_with('?') && !key.starts_with('$');

        if key_is_concrete {
            // Scan all witnesses for one matching this constraint + type key.
            let mut found_witness = false;
            for &decl_id in &self.file.decls {
                let decl = self.decl(decl_id);
                if let ThirDeclKind::Witness {
                    constraint: Some(c),
                    target,
                    fields,
                    ..
                } = &decl.kind
                    && *c == constraint_binding
                    && type_key(&self.file.type_arena, &aliases, *target) == key
                {
                    found_witness = true;
                    for field in fields {
                        if field.name == method_name {
                            return Ok(Some(self.eval(field.value, env)?));
                        }
                    }
                    // Matching witness found but field absent — fall through to
                    // default-body check below (only reachable for concrete keys).
                    break;
                }
            }

            if found_witness {
                // Matching witness exists but omitted this method — check for a
                // default body in the constraint declaration.
                for &decl_id in &self.file.decls {
                    let decl = self.decl(decl_id);
                    if let ThirDeclKind::Constraint { methods, .. } = &decl.kind
                        && decl.binding == constraint_binding
                    {
                        for m in methods {
                            if m.name == method_name
                                && let Some(clauses) = &m.default
                            {
                                let arity = clauses.first().map(|c| c.patterns.len()).unwrap_or(0);
                                return Ok(Some(Value::Closure(Rc::new(Closure {
                                    binding: m.binding,
                                    arity,
                                    clauses: clauses.as_slice().into(),
                                    env: env.clone(),
                                    applied: Vec::new(),
                                    home: self.active_module,
                                }))));
                            }
                        }
                    }
                }
            }

            // No matching witness at all (or witness had no field and no default).
            return Ok(None);
        }

        // Ambiguous key: TypeVar/InferVar/AliasApply inside a bounded function body.
        //
        // Env fallback: for direct top-level calls, the dict was injected into
        // an ancestor frame. For indirect calls, lookup returns Err — we fall
        // through to UnresolvedWitness rather than a wrong-answer default.
        if let Ok(thunk) = env.lookup(constraint_binding)
            && let Value::WitnessDict(dict) = thunk.force(self)?
            && let Some(v) = dict.get(&method_name)
        {
            return Ok(Some(v.clone()));
        }

        // Env fallback failed — dict is not visible (indirect call case).
        // Return a clean refusal instead of silently using the default body.
        Err(EvalError::UnresolvedWitness {
            method: method_name,
        })
    }

    // ── operator-method dispatch ──────────────────────────────────────────────

    /// Dispatch a comparison operator to a user-defined witness field.
    ///
    /// Returns `Ok(Some(v))` when a matching `(op)` field is found and applied.
    /// Returns `Ok(None)` when no witness matches — caller falls through to builtin.
    fn try_operator_dispatch(
        &self,
        op: BinOp,
        operand_ty: TypeId,
        lv: &Value,
        rv: &Value,
        env: &Env,
    ) -> Result<Option<Value>, EvalError> {
        let op_name = match op {
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            _ => return Ok(None),
        };
        let aliases = self.build_alias_map();
        let key = type_key(&self.file.type_arena, &aliases, operand_ty);

        if op == BinOp::Ne {
            // Try (!=) first; fall back to negating (==).
            if let Some(v) = self.dispatch_operator_field("!=", &key, &aliases, lv, rv, env)? {
                return Ok(Some(v));
            }
            if let Some(v) = self.dispatch_operator_field("==", &key, &aliases, lv, rv, env)? {
                return Ok(match v {
                    Value::Bool(b) => Some(Value::Bool(!b)),
                    _ => None,
                });
            }
            return Ok(None);
        }

        self.dispatch_operator_field(op_name, &key, &aliases, lv, rv, env)
    }

    /// Find a witness field matching `op_name` for type key `key` and apply it.
    fn dispatch_operator_field(
        &self,
        op_name: &str,
        key: &str,
        aliases: &HashMap<BindingId, TypeId>,
        lv: &Value,
        rv: &Value,
        env: &Env,
    ) -> Result<Option<Value>, EvalError> {
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::Witness { target, fields, .. } = &decl.kind {
                if type_key(&self.file.type_arena, aliases, *target) == key {
                    for field in fields {
                        if field.is_operator && field.name == op_name {
                            let fv = self.eval(field.value, env)?;
                            match fv {
                                Value::Closure(c) => {
                                    let args =
                                        vec![Thunk::ready(lv.clone()), Thunk::ready(rv.clone())];
                                    return Ok(Some(self.apply_closure(&c, args)?));
                                }
                                _ => return Ok(None),
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Build a map from alias `BindingId` to its underlying `TypeId` for
    /// alias-resolved `type_key` calls.
    fn build_alias_map(&self) -> HashMap<BindingId, TypeId> {
        let mut m = HashMap::new();
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::TypeAlias { ty, .. } = &decl.kind {
                m.insert(decl.binding, *ty);
            }
        }
        m
    }

    /// Return `true` if the file has any witness with a `(==)` or `(!=)` operator field.
    fn has_eq_operator_witness(&self) -> bool {
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::Witness { fields, .. } = &decl.kind {
                for f in fields {
                    if f.is_operator && (f.name == "==" || f.name == "!=") {
                        return true;
                    }
                }
            }
        }
        false
    }

    // ── building top-level env ────────────────────────────────────────────────

    /// Build the top-level letrec environment from the file's declarations.
    ///
    /// The `top` frame is created first and shared across all thunks so that
    /// mutual recursion works.
    pub fn build_top_env(&self) -> Env {
        let top = Env::empty();
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            match &decl.kind {
                ThirDeclKind::Value { value, .. } => {
                    // Deferred thunk stamped with this module so it evaluates
                    // against the correct arenas when forced.
                    let thunk = self.defer(*value, top.clone());
                    top.insert(decl.binding, thunk);
                }
                ThirDeclKind::Function { clauses, .. } => {
                    let arity = clauses.first().map(|c| c.patterns.len()).unwrap_or(0);
                    let closure = Closure {
                        binding: Some(decl.binding),
                        arity,
                        clauses: clauses.as_slice().into(),
                        env: top.clone(),
                        applied: Vec::new(),
                        home: self.active_module,
                    };
                    // Functions are pre-evaluated to closures.
                    top.insert(decl.binding, Thunk::ready(Value::Closure(Rc::new(closure))));
                }
                ThirDeclKind::TypeAlias { ty, .. } => {
                    // Type aliases are available as type values.
                    top.insert(decl.binding, Thunk::ready(Value::TypeValue(*ty)));
                }
                // Constraint/witness decls contribute nothing to the eval environment
                // this increment; dictionary-passing elaboration is deferred.
                ThirDeclKind::Constraint { .. } | ThirDeclKind::Witness { .. } => {}
            }
        }
        top
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Structural type key mirroring `witness_target_key` in the THIR lowerer.
///
/// Resolves top-level `Alias` chains via `aliases` so that a witness target
/// written as a named alias matches an operand whose inferred type is the
/// equivalent structural type. `AliasApply` (parametric aliases) is not
/// resolved and stays as `$<id>[...]` — dispatch on those is deferred.
fn type_key(type_arena: &[Type], aliases: &HashMap<BindingId, TypeId>, ty: TypeId) -> String {
    let ty = resolve_alias_chain(type_arena, aliases, ty);
    match &type_arena[ty.0 as usize].kind {
        TypeKind::Int => "Int".into(),
        TypeKind::Bool => "Bool".into(),
        TypeKind::Text => "Text".into(),
        TypeKind::Float => "Float".into(),
        TypeKind::Type => "Type".into(),
        TypeKind::True => "true".into(),
        TypeKind::False => "false".into(),
        TypeKind::Atom(a) => format!("#{a}"),
        TypeKind::List(inner) => format!("[{}]", type_key(type_arena, aliases, *inner)),
        TypeKind::Optional(inner) => format!("{}?", type_key(type_arena, aliases, *inner)),
        TypeKind::Record(fields) => {
            let mut parts: Vec<String> = fields
                .iter()
                .map(|f| format!("{}:{}", f.name, type_key(type_arena, aliases, f.ty)))
                .collect();
            parts.sort();
            format!("{{{}}}", parts.join(","))
        }
        TypeKind::Union(variants) => {
            let parts: Vec<String> = variants
                .iter()
                .map(|v| match v.payload {
                    Some(p) => format!("{}({})", v.name, type_key(type_arena, aliases, p)),
                    None => v.name.clone(),
                })
                .collect();
            format!("<{}>", parts.join("|"))
        }
        TypeKind::Tuple(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| match item {
                    TypeTupleItem::Named { name, ty, .. } => {
                        format!("{}:{}", name, type_key(type_arena, aliases, *ty))
                    }
                    TypeTupleItem::Positional(ty) => type_key(type_arena, aliases, *ty),
                })
                .collect();
            format!("({})", parts.join(","))
        }
        TypeKind::Function { from, to } => {
            format!(
                "({}->{}",
                type_key(type_arena, aliases, *from),
                type_key(type_arena, aliases, *to)
            )
        }
        TypeKind::TypeVar(b) | TypeKind::Alias(b) => format!("@{}", b.0),
        TypeKind::AliasApply { binding, args } => {
            let arg_parts: Vec<String> = args
                .iter()
                .map(|a| type_key(type_arena, aliases, *a))
                .collect();
            format!("${}[{}]", binding.0, arg_parts.join(","))
        }
        TypeKind::InferVar(v) => format!("?{v}"),
        TypeKind::Error => "<error>".into(),
    }
}

/// Repeatedly resolves `TypeKind::Alias` entries through `aliases` until
/// a non-alias type or an unknown alias is reached.
fn resolve_alias_chain(
    type_arena: &[Type],
    aliases: &HashMap<BindingId, TypeId>,
    mut ty: TypeId,
) -> TypeId {
    let mut fuel = 64u8;
    while fuel > 0 {
        match &type_arena[ty.0 as usize].kind {
            TypeKind::Alias(b) => match aliases.get(b) {
                Some(&next) => {
                    ty = next;
                    fuel -= 1;
                }
                None => break,
            },
            _ => break,
        }
    }
    ty
}

/// Returns `true` if the type key contains unresolvable components
/// (`?` for `InferVar`, `$` for `AliasApply`) that could cause a
/// dispatch miss despite a witness being present.
fn key_is_ambiguous(key: &str) -> bool {
    key.contains('?') || key.contains('$')
}

fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Text(_) => "Text",
        Value::Atom(_) => "Atom",
        Value::List(_) => "List",
        Value::Tuple(_) => "Tuple",
        Value::Record(_) => "Record",
        Value::Closure(_) => "Function",
        Value::TypeValue(_) => "Type",
        Value::TaggedValue { .. } => "TaggedValue",
        Value::Nothing => "Nothing",
        Value::WitnessDict(_) => "WitnessDict",
    }
}

/// Helper for arithmetic binary ops with overflow-checking for `Int` and
/// IEEE semantics for `Float`.
fn numeric_binop(
    lv: Value,
    rv: Value,
    int_op: fn(i64, i64) -> Option<i64>,
    float_op: fn(f64, f64) -> f64,
    op_name: &'static str,
) -> Result<Value, EvalError> {
    match (lv, rv) {
        (Value::Int(a), Value::Int(b)) => int_op(a, b)
            .map(Value::Int)
            .ok_or(EvalError::IntOverflow(op_name)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(a, b))),
        (a, b) => Err(EvalError::TypeMismatch {
            expected: "Int or Float",
            found: if matches!(a, Value::Int(_) | Value::Float(_)) {
                value_type_name(&b)
            } else {
                value_type_name(&a)
            },
        }),
    }
}

/// Comparison operators for Int, Float, and Text.
fn cmp_op(
    lv: Value,
    rv: Value,
    target: std::cmp::Ordering,
    or_equal: bool,
) -> Result<Value, EvalError> {
    let ord = match (&lv, &rv) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Less),
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        _ => {
            return Err(EvalError::TypeMismatch {
                expected: "Int, Float, or Text",
                found: value_type_name(&lv),
            });
        }
    };
    let result = ord == target || (or_equal && ord == std::cmp::Ordering::Equal);
    Ok(Value::Bool(result))
}

// Float arithmetic via std ops.
trait FloatBinOp {
    fn add(a: f64, b: f64) -> f64;
    fn sub(a: f64, b: f64) -> f64;
    fn mul(a: f64, b: f64) -> f64;
    fn div(a: f64, b: f64) -> f64;
}

impl FloatBinOp for f64 {
    fn add(a: f64, b: f64) -> f64 {
        a + b
    }
    fn sub(a: f64, b: f64) -> f64 {
        a - b
    }
    fn mul(a: f64, b: f64) -> f64 {
        a * b
    }
    fn div(a: f64, b: f64) -> f64 {
        a / b
    }
}
