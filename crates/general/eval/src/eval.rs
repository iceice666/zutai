//! THIR tree-walk evaluator — the THIR-specific reference oracle in this crate.
//!
//! The parallel `eval_tlc.rs` walker evaluates TLC modules for compiler-path
//! parity checks while reusing the same `value`, `thunk`, and `env` runtime
//! structures.
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
use zutai_thir::ir::UnionVariant;
use zutai_thir::{
    ImportKey, ThirClause, ThirDeclId, ThirDeclKind, ThirExprId, ThirExprKind, ThirFile, ThirPatId,
};
use zutai_thir::{
    RowTail, ThirPatKind, ThirTupleItem, ThirTuplePatItem, Type, TypeId, TypeKind, TypeRecordField,
    TypeTupleItem,
};

use crate::{
    EvalError,
    env::Env,
    thunk::Thunk,
    value::{BuiltinFn, Closure, ModuleId, RuntimeType, TupleField, Value, values_equal},
};

/// A slice of all evaluated modules for this run, keyed by position = `ModuleId`.
pub type ModuleRegistry = Vec<Arc<ThirFile>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWitness {
    pub module: ModuleId,
    pub constraint: String,
    pub target_key: String,
}

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
    pub witnesses: &'a [RuntimeWitness],
}

impl<'a> Evaluator<'a> {
    pub fn new(
        file: &'a ThirFile,
        registry: &'a [Arc<ThirFile>],
        active_module: ModuleId,
        imports: &'a HashMap<ImportKey, Value>,
        witnesses: &'a [RuntimeWitness],
    ) -> Self {
        Self {
            file,
            registry,
            active_module,
            imports,
            witnesses,
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
            witnesses: self.witnesses,
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

    /// Returns (field_may_be_absent, field_value_ty) for `field` on the record
    /// type `ty` (alias-resolved), or None if `ty` is not a record or the field
    /// is not declared.
    fn record_field_meta(&self, ty: TypeId, field: &str) -> Option<(bool, TypeId)> {
        let aliases = self.build_alias_map();
        let resolved = resolve_alias_chain(&self.file.type_arena, &aliases, ty);
        match &self.file.type_arena[resolved.0 as usize].kind {
            TypeKind::Record(fields, _) => fields
                .iter()
                .find(|f| f.name == field)
                .map(|f| (f.optional, f.ty)),
            _ => None,
        }
    }

    fn type_is_optional(&self, ty: TypeId) -> bool {
        let aliases = self.build_alias_map();
        let resolved = resolve_alias_chain(&self.file.type_arena, &aliases, ty);
        matches!(
            self.file.type_arena[resolved.0 as usize].kind,
            TypeKind::Optional(_)
        )
    }

    fn project_optional_field(
        &self,
        fields: &Rc<Vec<(Rc<str>, Thunk)>>,
        field: &str,
        value_already_optional: bool,
    ) -> Result<Value, EvalError> {
        match fields.iter().find(|(name, _)| name.as_ref() == field) {
            None => Ok(Value::Atom(Rc::from("none"))),
            Some((_, thunk)) if value_already_optional => thunk.force(self),
            Some((_, thunk)) => Ok(Value::TaggedValue {
                tag: Rc::from("some"),
                payload: Rc::new(vec![(Rc::from("value"), thunk.clone())]),
            }),
        }
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
            ThirExprKind::TypeValue(ty) => Ok(Value::TypeValue(RuntimeType::new(
                self.active_module,
                *ty,
            ))),
            ThirExprKind::TaggedValue { tag, payload } => {
                let payload_val = self.eval(*payload, env)?;
                let fields = match payload_val {
                    Value::Record(f) => (*f).clone(),
                    Value::Tuple(f) => f
                        .iter()
                        .enumerate()
                        .map(|(index, field)| {
                            let name = field
                                .name
                                .clone()
                                .unwrap_or_else(|| Rc::from(index.to_string()));
                            (name, field.value.clone())
                        })
                        .collect(),
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
                        if let Some((true, value_ty)) =
                            self.record_field_meta(self.expr(*receiver).ty, field)
                        {
                            return self.project_optional_field(
                                &fields,
                                field,
                                self.type_is_optional(value_ty),
                            );
                        }
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
                    Value::Builtin(BuiltinFn::Print) => {
                        // `print :: Text -> Text` — arity 1, applied immediately.
                        // Side effect: write the text plus a newline to stdout;
                        // return the argument unchanged so it stays inspectable.
                        match self.eval(*arg, env)? {
                            Value::Text(s) => {
                                println!("{s}");
                                Ok(Value::Text(s))
                            }
                            other => Err(EvalError::TypeMismatch {
                                expected: "Text",
                                found: value_type_name(&other),
                            }),
                        }
                    }
                    Value::Builtin(BuiltinFn::Fields) => {
                        let arg = self.eval(*arg, env)?;
                        self.reflect_fields_value(arg)
                    }
                    Value::Builtin(BuiltinFn::Schema) => {
                        let arg = self.eval(*arg, env)?;
                        self.reflect_schema_value(arg)
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
                let aliases = self.build_alias_map();
                let receiver_ty =
                    resolve_alias_chain(&self.file.type_arena, &aliases, self.expr(*receiver).ty);
                let inner_ty = match &self.file.type_arena[receiver_ty.0 as usize].kind {
                    TypeKind::Optional(inner) => *inner,
                    _ => receiver_ty,
                };
                let project_inner_field =
                    |fields: &Rc<Vec<(Rc<str>, Thunk)>>| -> Result<Value, EvalError> {
                        if let Some((true, value_ty)) = self.record_field_meta(inner_ty, field) {
                            return self.project_optional_field(
                                fields,
                                field,
                                self.type_is_optional(value_ty),
                            );
                        }
                        match fields.iter().find(|(name, _)| name.as_ref() == field.as_str()) {
                            Some((_, thunk)) => Ok(Value::TaggedValue {
                                tag: Rc::from("some"),
                                payload: Rc::new(vec![(Rc::from("value"), thunk.clone())]),
                            }),
                            None => Ok(Value::Atom(Rc::from("none"))),
                        }
                    };

                let rv = self.eval(*receiver, env)?;
                match rv {
                    Value::Atom(atom) if atom.as_ref() == "none" => Ok(Value::Atom(Rc::from("none"))),
                    Value::TaggedValue { tag, .. } if tag.as_ref() == "none" => {
                        Ok(Value::Atom(Rc::from("none")))
                    }
                    Value::Nothing => Ok(Value::Atom(Rc::from("none"))),
                    Value::TaggedValue { tag, payload } if tag.as_ref() == "some" => {
                        let inner = match payload.iter().find(|(name, _)| name.as_ref() == "value") {
                            Some((_, thunk)) => thunk.force(self)?,
                            None => {
                                return Err(EvalError::TypeMismatch {
                                    expected: "Record",
                                    found: "TaggedValue",
                                });
                            }
                        };
                        match inner {
                            Value::Record(inner_fields) => project_inner_field(&inner_fields),
                            other => Err(EvalError::TypeMismatch {
                                expected: "Record",
                                found: value_type_name(&other),
                            }),
                        }
                    }
                    Value::Record(fields) => project_inner_field(&fields),
                    other => Err(EvalError::TypeMismatch {
                        expected: "Optional",
                        found: value_type_name(&other),
                    }),
                }
            }
            ThirExprKind::Sequence(items) => {
                let mut value = Value::Nothing;
                for &item in items {
                    value = self.eval(item, env)?;
                }
                Ok(value)
            }
            ThirExprKind::Perform { .. }
            | ThirExprKind::Handle { .. }
            | ThirExprKind::Resume { .. } => Err(EvalError::EffectfulNotExecutable(
                "algebraic effects execute through the TLC evaluator; the legacy THIR evaluator remains pure-only"
                    .to_string(),
            )),
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
                    // Absent optional: implicit `Value::Nothing` or an explicit
                    // `#none` value (atom or zero-payload tagged) → fallback.
                    Value::Nothing => self.eval(rhs, env),
                    Value::Atom(a) if a.as_ref() == "none" => self.eval(rhs, env),
                    Value::TaggedValue { tag, .. } if tag.as_ref() == "none" => self.eval(rhs, env),
                    // Explicit `#some { value = x }` → unwrap to `x`, matching the
                    // spec desugaring `match v { #none => d; #some { value = x } => x; }`.
                    Value::TaggedValue { tag, payload } if tag.as_ref() == "some" => {
                        match payload.iter().find(|(n, _)| n.as_ref() == "value") {
                            Some((_, thunk)) => thunk.force(self),
                            None => Ok(Value::TaggedValue { tag, payload }),
                        }
                    }
                    // A present optional already unwrapped to a bare value → pass through.
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

            let constraint_name = self.binding_name(constraint_binding);
            if let Some(value) =
                self.eval_imported_witness_field(constraint_name, &method_name, &key)?
            {
                return Ok(Some(value));
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

        if key_is_ambiguous(&key) {
            if op == BinOp::Ne {
                let ne_err = match self.dispatch_operator_dict_field("!=", lv, rv, env) {
                    Ok(Some(v)) => return Ok(Some(v)),
                    Ok(None) => None,
                    Err(err) => Some(err),
                };
                match self.dispatch_operator_dict_field("==", lv, rv, env) {
                    Ok(Some(v)) => {
                        return Ok(match v {
                            Value::Bool(b) => Some(Value::Bool(!b)),
                            _ => None,
                        });
                    }
                    Ok(None) => {}
                    Err(err) => return Err(err),
                }
                if let Some(err) = ne_err {
                    return Err(err);
                }
                return Ok(None);
            }

            return self.dispatch_operator_dict_field(op_name, lv, rv, env);
        }

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
        aliases: &HashMap<BindingId, (Vec<BindingId>, TypeId)>,
        lv: &Value,
        rv: &Value,
        env: &Env,
    ) -> Result<Option<Value>, EvalError> {
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::Witness { target, fields, .. } = &decl.kind
                && type_key(&self.file.type_arena, aliases, *target) == key
            {
                for field in fields {
                    if field.is_operator && field.name == op_name {
                        let fv = self.eval(field.value, env)?;
                        match fv {
                            Value::Closure(c) => {
                                let args = vec![Thunk::ready(lv.clone()), Thunk::ready(rv.clone())];
                                return Ok(Some(self.apply_closure(&c, args)?));
                            }
                            _ => return Ok(None),
                        }
                    }
                }
            }
        }
        if let Some(fv) = self.eval_imported_witness_field("", op_name, key)? {
            match fv {
                Value::Closure(c) => {
                    let args = vec![Thunk::ready(lv.clone()), Thunk::ready(rv.clone())];
                    return Ok(Some(self.apply_closure(&c, args)?));
                }
                _ => return Ok(None),
            }
        }

        Ok(None)
    }

    /// Find an operator constraint method's active dictionary field and apply it.
    fn dispatch_operator_dict_field(
        &self,
        op_name: &str,
        lv: &Value,
        rv: &Value,
        env: &Env,
    ) -> Result<Option<Value>, EvalError> {
        let mut found_operator_constraint = false;

        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::Constraint { methods, .. } = &decl.kind {
                let has_operator_method = methods
                    .iter()
                    .any(|method| method.is_operator && method.name == op_name);
                if !has_operator_method {
                    continue;
                }

                found_operator_constraint = true;
                if let Ok(thunk) = env.lookup(decl.binding)
                    && let Value::WitnessDict(dict) = thunk.force(self)?
                    && let Some(Value::Closure(c)) = dict.get(op_name)
                {
                    let args = vec![Thunk::ready(lv.clone()), Thunk::ready(rv.clone())];
                    return Ok(Some(self.apply_closure(c, args)?));
                }
            }
        }

        if found_operator_constraint {
            Err(EvalError::UnresolvedWitness {
                method: op_name.to_string(),
            })
        } else {
            Ok(None)
        }
    }

    fn eval_imported_witness_field(
        &self,
        constraint_name: &str,
        field_name: &str,
        target_key: &str,
    ) -> Result<Option<Value>, EvalError> {
        for witness in self.witnesses {
            if witness.module == self.active_module || witness.target_key != target_key {
                continue;
            }
            if !constraint_name.is_empty() && witness.constraint != constraint_name {
                continue;
            }
            let ev = self.for_module(witness.module);
            let top = ev.build_top_env();
            let aliases = ev.build_alias_map();
            for &decl_id in &ev.file.decls {
                let decl = ev.decl(decl_id);
                if let ThirDeclKind::Witness {
                    constraint: Some(c),
                    target,
                    fields,
                    ..
                } = &decl.kind
                    && ev.binding_name(*c) == witness.constraint
                    && type_key(&ev.file.type_arena, &aliases, *target) == witness.target_key
                {
                    for field in fields {
                        if field.name == field_name {
                            return Ok(Some(ev.eval(field.value, &top)?));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    fn binding_name(&self, binding: BindingId) -> &str {
        self.file
            .binding_names
            .get(binding.0 as usize)
            .map_or("", String::as_str)
    }

    /// Build a map from alias `BindingId` to its underlying `TypeId` for
    /// alias-resolved `type_key` calls.
    fn build_alias_map(&self) -> HashMap<BindingId, (Vec<BindingId>, TypeId)> {
        let mut m = HashMap::new();
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::TypeAlias { params, ty } = &decl.kind {
                m.insert(decl.binding, (params.clone(), *ty));
            }
        }
        m
    }

    fn reflect_fields_value(&self, value: Value) -> Result<Value, EvalError> {
        match value {
            Value::TypeValue(ty) => self.reflect_fields(&ty),
            other => Err(EvalError::TypeMismatch {
                expected: "Type",
                found: value_type_name(&other),
            }),
        }
    }

    fn reflect_schema_value(&self, value: Value) -> Result<Value, EvalError> {
        match value {
            Value::TypeValue(ty) => self.reflect_schema(&ty),
            other => Err(EvalError::TypeMismatch {
                expected: "Type",
                found: value_type_name(&other),
            }),
        }
    }

    fn reflect_fields(&self, ty: &RuntimeType) -> Result<Value, EvalError> {
        match self.runtime_type_view(ty, 0)? {
            RuntimeTypeView::Record(fields, RowTail::Closed) => fields
                .into_iter()
                .map(|field| {
                    Ok(record_value(vec![
                        ("name", Value::Text(Rc::from(field.name.as_str()))),
                        ("Type", Value::TypeValue(field.ty)),
                        ("optional", Value::Bool(field.optional)),
                    ]))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(list_value),
            RuntimeTypeView::Record(_, _) => Err(open_row_reflection_error("record")),
            RuntimeTypeView::Union(_, _) => Err(EvalError::ReflectionUnsupported(
                "`fields` reflects record fields; use `schema` for union variants".to_string(),
            )),
            _ => Err(EvalError::ReflectionUnsupported(
                "`fields` expects a record type".to_string(),
            )),
        }
    }

    fn reflect_schema(&self, ty: &RuntimeType) -> Result<Value, EvalError> {
        match self.runtime_type_view(ty, 0)? {
            RuntimeTypeView::Record(fields, RowTail::Closed) => Ok(record_value(vec![
                ("kind", Value::Atom(Rc::from("record"))),
                ("fields", self.schema_fields(fields)?),
            ])),
            RuntimeTypeView::Record(_, _) => Err(open_row_reflection_error("record")),
            RuntimeTypeView::Union(variants, RowTail::Closed) => {
                let variants = variants
                    .into_iter()
                    .map(|variant| self.schema_variant(variant))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(record_value(vec![
                    ("kind", Value::Atom(Rc::from("union"))),
                    ("variants", list_value(variants)),
                ]))
            }
            RuntimeTypeView::Union(_, _) => Err(open_row_reflection_error("union")),
            _ => Err(EvalError::ReflectionUnsupported(
                "`schema` expects a record or union type".to_string(),
            )),
        }
    }

    fn schema_fields(&self, fields: Vec<ReflectedRecordField>) -> Result<Value, EvalError> {
        fields
            .into_iter()
            .map(|field| self.schema_field(field))
            .collect::<Result<Vec<_>, _>>()
            .map(list_value)
    }

    fn schema_field(&self, field: ReflectedRecordField) -> Result<Value, EvalError> {
        Ok(record_value(vec![
            ("name", Value::Text(Rc::from(field.name.as_str()))),
            (
                "type",
                Value::Text(Rc::from(self.type_label(&field.ty)?.as_str())),
            ),
            ("optional", Value::Bool(field.optional)),
        ]))
    }

    fn schema_variant(&self, variant: ReflectedUnionVariant) -> Result<Value, EvalError> {
        let fields = match variant.payload {
            Some(payload) => match self.runtime_type_view(&payload, 0)? {
                RuntimeTypeView::Record(fields, RowTail::Closed) => self.schema_fields(fields)?,
                RuntimeTypeView::Record(_, _) => return Err(open_row_reflection_error("record")),
                _ => {
                    return Err(EvalError::ReflectionUnsupported(
                        "union variant payload reflection expects a record payload".to_string(),
                    ));
                }
            },
            None => list_value(Vec::new()),
        };
        Ok(record_value(vec![
            ("name", Value::Text(Rc::from(variant.name.as_str()))),
            ("fields", fields),
        ]))
    }

    fn type_label(&self, ty: &RuntimeType) -> Result<String, EvalError> {
        match self.runtime_type_view(ty, 0)? {
            RuntimeTypeView::Type => Ok("Type".to_string()),
            RuntimeTypeView::Bool => Ok("Bool".to_string()),
            RuntimeTypeView::Text => Ok("Text".to_string()),
            RuntimeTypeView::Int => Ok("Int".to_string()),
            RuntimeTypeView::Float => Ok("Float".to_string()),
            RuntimeTypeView::Atom(name) => Ok(format!("#{name}")),
            RuntimeTypeView::True => Ok("true".to_string()),
            RuntimeTypeView::False => Ok("false".to_string()),
            RuntimeTypeView::Never => Ok("Never".to_string()),
            RuntimeTypeView::List(inner) => Ok(format!("[{}]", self.type_label(&inner)?)),
            RuntimeTypeView::Optional(inner) => Ok(format!("{}?", self.type_label(&inner)?)),
            RuntimeTypeView::Record(_, RowTail::Closed) => Ok("record".to_string()),
            RuntimeTypeView::Record(_, _) => Err(open_row_reflection_error("record")),
            RuntimeTypeView::Union(_, RowTail::Closed) => Ok("union".to_string()),
            RuntimeTypeView::Union(_, _) => Err(open_row_reflection_error("union")),
            RuntimeTypeView::Tuple(items) => {
                let parts = items
                    .into_iter()
                    .map(|item| match item {
                        ReflectedTupleItem::Named { name, ty } => {
                            Ok(format!("{name}: {}", self.type_label(&ty)?))
                        }
                        ReflectedTupleItem::Positional(ty) => self.type_label(&ty),
                    })
                    .collect::<Result<Vec<_>, EvalError>>()?;
                Ok(format!("({})", parts.join(", ")))
            }
            RuntimeTypeView::Function { from, to } => Ok(format!(
                "{} -> {}",
                self.type_label(&from)?,
                self.type_label(&to)?
            )),
            RuntimeTypeView::Effect { base } => Ok(format!("{} ! effect", self.type_label(&base)?)),
        }
    }

    fn runtime_type_view(
        &self,
        ty: &RuntimeType,
        depth: u16,
    ) -> Result<RuntimeTypeView, EvalError> {
        if depth > 256 {
            return Err(EvalError::ReflectionUnsupported(
                "type alias expansion exceeded reflection fuel".to_string(),
            ));
        }
        let file = self.file_for_module(ty.module)?;
        let Some(type_node) = file.type_arena.get(ty.ty.0 as usize) else {
            return Err(EvalError::Internal(
                "type value points outside its module arena",
            ));
        };
        match type_node.kind.clone() {
            TypeKind::Type => Ok(RuntimeTypeView::Type),
            TypeKind::Bool => Ok(RuntimeTypeView::Bool),
            TypeKind::Text => Ok(RuntimeTypeView::Text),
            TypeKind::Int => Ok(RuntimeTypeView::Int),
            TypeKind::Float => Ok(RuntimeTypeView::Float),
            TypeKind::Atom(name) => Ok(RuntimeTypeView::Atom(name)),
            TypeKind::True => Ok(RuntimeTypeView::True),
            TypeKind::False => Ok(RuntimeTypeView::False),
            TypeKind::Never => Ok(RuntimeTypeView::Never),
            TypeKind::List(inner) => Ok(RuntimeTypeView::List(ty.with_ty(inner))),
            TypeKind::Optional(inner) => Ok(RuntimeTypeView::Optional(ty.with_ty(inner))),
            TypeKind::Record(fields, tail) => Ok(RuntimeTypeView::Record(
                reflect_record_fields(ty, fields),
                tail,
            )),
            TypeKind::Union(variants, tail) => Ok(RuntimeTypeView::Union(
                reflect_union_variants(ty, variants),
                tail,
            )),
            TypeKind::Tuple(items) => Ok(RuntimeTypeView::Tuple(reflect_tuple_items(ty, items))),
            TypeKind::Function { from, to } => Ok(RuntimeTypeView::Function {
                from: ty.with_ty(from),
                to: ty.with_ty(to),
            }),
            TypeKind::Effect { base, .. } => Ok(RuntimeTypeView::Effect {
                base: ty.with_ty(base),
            }),
            TypeKind::TypeVar(binding) => {
                match ty.subst.iter().rev().find(|(b, _)| *b == binding) {
                    Some((_, replacement)) => self.runtime_type_view(replacement, depth + 1),
                    None => Err(EvalError::ReflectionUnsupported(format!(
                        "unsubstituted type parameter `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    ))),
                }
            }
            TypeKind::Alias(binding) => {
                let Some((params, body)) = alias_decl(file, binding) else {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "unknown type alias `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    )));
                };
                if !params.is_empty() {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "unapplied type constructor `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    )));
                }
                self.runtime_type_view(
                    &RuntimeType::with_subst(ty.module, body, ty.subst.clone()),
                    depth + 1,
                )
            }
            TypeKind::AliasApply { binding, args } => {
                let Some((params, body)) = alias_decl(file, binding) else {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "unknown type alias `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    )));
                };
                if params.len() != args.len() {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "type constructor `{}` has arity {}, got {}",
                        binding_name_in_file(file, binding),
                        params.len(),
                        args.len()
                    )));
                }
                let subst = extend_subst(ty, &params, &args);
                self.runtime_type_view(&RuntimeType::with_subst(ty.module, body, subst), depth + 1)
            }
            TypeKind::Apply { .. } => self.runtime_apply_view(ty, depth),
            TypeKind::Con(binding) => Err(EvalError::ReflectionUnsupported(format!(
                "unapplied builtin type constructor `{}` cannot be reflected",
                binding_name_in_file(file, binding)
            ))),
            TypeKind::InferVar(_) | TypeKind::Error => Err(EvalError::ReflectionUnsupported(
                "incomplete or erroneous types cannot be reflected".to_string(),
            )),
        }
    }

    fn runtime_apply_view(
        &self,
        ty: &RuntimeType,
        depth: u16,
    ) -> Result<RuntimeTypeView, EvalError> {
        let file = self.file_for_module(ty.module)?;
        let mut args = Vec::new();
        let mut head = ty.ty;
        while let TypeKind::Apply { func, arg } = file.type_arena[head.0 as usize].kind.clone() {
            args.push(arg);
            head = func;
        }
        args.reverse();
        match file.type_arena[head.0 as usize].kind.clone() {
            TypeKind::Alias(binding) => {
                let Some((params, body)) = alias_decl(file, binding) else {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "unknown type alias `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    )));
                };
                if params.len() != args.len() {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "type constructor `{}` has arity {}, got {}",
                        binding_name_in_file(file, binding),
                        params.len(),
                        args.len()
                    )));
                }
                let subst = extend_subst(ty, &params, &args);
                self.runtime_type_view(&RuntimeType::with_subst(ty.module, body, subst), depth + 1)
            }
            TypeKind::Con(binding)
                if binding_name_in_file(file, binding) == "List" && args.len() == 1 =>
            {
                Ok(RuntimeTypeView::List(ty.with_ty(args[0])))
            }
            TypeKind::Con(binding)
                if binding_name_in_file(file, binding) == "Optional" && args.len() == 1 =>
            {
                Ok(RuntimeTypeView::Optional(ty.with_ty(args[0])))
            }
            _ => Err(EvalError::ReflectionUnsupported(
                "higher-kinded or partial type application cannot be reflected".to_string(),
            )),
        }
    }

    fn file_for_module(&self, module: ModuleId) -> Result<&ThirFile, EvalError> {
        self.registry
            .get(module.0)
            .map(Arc::as_ref)
            .ok_or(EvalError::Internal("type value module is not registered"))
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
        // Seed prelude builtins (e.g. `print`). HIR seeds these into the root
        // scope first, so the lowest-id binding for each name is the prelude
        // one (a user lambda param sharing the name lives at a higher id and is
        // shadowed in a child frame at apply time).
        for &name in zutai_hir::BUILTIN_VALUE_NAMES {
            if let Some(builtin) = BuiltinFn::from_name(name)
                && let Some(index) = self.file.binding_names.iter().position(|n| n == name)
            {
                top.insert(
                    zutai_hir::BindingId(index as u32),
                    Thunk::ready(Value::Builtin(builtin)),
                );
            }
        }
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
                    top.insert(
                        decl.binding,
                        Thunk::ready(Value::TypeValue(RuntimeType::new(self.active_module, *ty))),
                    );
                }
                // Constraint/witness decls contribute nothing to the eval environment
                // this increment; dictionary-passing elaboration is deferred.
                ThirDeclKind::Constraint { .. } | ThirDeclKind::Witness { .. } => {}
            }
        }
        top
    }
}

#[derive(Clone)]
struct ReflectedRecordField {
    name: String,
    optional: bool,
    ty: RuntimeType,
}

#[derive(Clone)]
struct ReflectedUnionVariant {
    name: String,
    payload: Option<RuntimeType>,
}

enum ReflectedTupleItem {
    Named { name: String, ty: RuntimeType },
    Positional(RuntimeType),
}

enum RuntimeTypeView {
    Type,
    Bool,
    Text,
    Int,
    Float,
    Atom(String),
    True,
    False,
    Never,
    List(RuntimeType),
    Optional(RuntimeType),
    Record(Vec<ReflectedRecordField>, RowTail),
    Union(Vec<ReflectedUnionVariant>, RowTail),
    Tuple(Vec<ReflectedTupleItem>),
    Function { from: RuntimeType, to: RuntimeType },
    Effect { base: RuntimeType },
}

fn record_value(fields: Vec<(&'static str, Value)>) -> Value {
    Value::Record(Rc::new(
        fields
            .into_iter()
            .map(|(name, value)| (Rc::from(name), Thunk::ready(value)))
            .collect(),
    ))
}

fn list_value(values: Vec<Value>) -> Value {
    Value::List(
        values
            .into_iter()
            .map(Thunk::ready)
            .collect::<Vec<_>>()
            .into(),
    )
}

fn reflect_record_fields(
    owner: &RuntimeType,
    fields: Vec<TypeRecordField>,
) -> Vec<ReflectedRecordField> {
    fields
        .into_iter()
        .map(|field| ReflectedRecordField {
            name: field.name,
            optional: field.optional,
            ty: owner.with_ty(field.ty),
        })
        .collect()
}

fn reflect_union_variants(
    owner: &RuntimeType,
    variants: Vec<UnionVariant>,
) -> Vec<ReflectedUnionVariant> {
    variants
        .into_iter()
        .map(|variant| ReflectedUnionVariant {
            name: variant.name,
            payload: variant.payload.map(|payload| owner.with_ty(payload)),
        })
        .collect()
}

fn reflect_tuple_items(owner: &RuntimeType, items: Vec<TypeTupleItem>) -> Vec<ReflectedTupleItem> {
    items
        .into_iter()
        .map(|item| match item {
            TypeTupleItem::Named { name, ty, .. } => ReflectedTupleItem::Named {
                name,
                ty: owner.with_ty(ty),
            },
            TypeTupleItem::Positional(ty) => ReflectedTupleItem::Positional(owner.with_ty(ty)),
        })
        .collect()
}

fn alias_decl(file: &ThirFile, binding: BindingId) -> Option<(Vec<BindingId>, TypeId)> {
    file.decls.iter().find_map(|decl_id| {
        let decl = &file.decl_arena[*decl_id];
        match &decl.kind {
            ThirDeclKind::TypeAlias { params, ty } if decl.binding == binding => {
                Some((params.clone(), *ty))
            }
            _ => None,
        }
    })
}

fn binding_name_in_file(file: &ThirFile, binding: BindingId) -> &str {
    file.binding_names
        .get(binding.0 as usize)
        .map_or("<unknown>", String::as_str)
}

fn extend_subst(
    owner: &RuntimeType,
    params: &[BindingId],
    args: &[TypeId],
) -> Rc<[(BindingId, RuntimeType)]> {
    let mut subst: Vec<(BindingId, RuntimeType)> = owner.subst.iter().cloned().collect();
    subst.extend(
        params
            .iter()
            .zip(args.iter())
            .map(|(param, arg)| (*param, owner.with_ty(*arg))),
    );
    Rc::from(subst.into_boxed_slice())
}

fn open_row_reflection_error(kind: &str) -> EvalError {
    EvalError::ReflectionUnsupported(format!(
        "reflection rejects open {kind} rows; close the row before calling `fields` or `schema`"
    ))
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Structural type key mirroring `witness_target_key` in the THIR lowerer.
///
/// Resolves named `Alias` chains and expands parametric `AliasApply` (with its
/// type args substituted) so a witness target written as `Pair A` matches an
/// operand whose inferred type is the equivalent structural record.
fn type_key(
    type_arena: &[Type],
    aliases: &HashMap<BindingId, (Vec<BindingId>, TypeId)>,
    ty: TypeId,
) -> String {
    type_key_subst(type_arena, aliases, &HashMap::new(), ty, 0)
}

fn type_key_subst(
    type_arena: &[Type],
    aliases: &HashMap<BindingId, (Vec<BindingId>, TypeId)>,
    subst: &HashMap<BindingId, TypeId>,
    ty: TypeId,
    depth: u32,
) -> String {
    // A structurally-recursive parametric alias (e.g. `Rec :: <A> type { #rec: A; }`)
    // would expand forever; cap the depth and fall back to an ambiguous marker so
    // dispatch refuses rather than overflowing the stack.
    if depth > 256 {
        return format!("$deep{}", ty.0);
    }
    let d = depth + 1;
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
        TypeKind::List(inner) => {
            format!(
                "[{}]",
                type_key_subst(type_arena, aliases, subst, *inner, d)
            )
        }
        TypeKind::Optional(inner) => {
            format!("{}?", type_key_subst(type_arena, aliases, subst, *inner, d))
        }
        TypeKind::Record(fields, tail) => {
            let mut parts: Vec<String> = fields
                .iter()
                .map(|f| {
                    format!(
                        "{}:{}",
                        f.name,
                        type_key_subst(type_arena, aliases, subst, f.ty, d)
                    )
                })
                .collect();
            parts.sort();
            format!("{{{}{}}}", parts.join(","), row_tail_key(*tail))
        }
        TypeKind::Union(variants, tail) => {
            let parts: Vec<String> = variants
                .iter()
                .map(|v| match v.payload {
                    Some(p) => {
                        format!(
                            "{}({})",
                            v.name,
                            type_key_subst(type_arena, aliases, subst, p, d)
                        )
                    }
                    None => v.name.clone(),
                })
                .collect();
            format!("<{}{}>", parts.join("|"), row_tail_key(*tail))
        }
        TypeKind::Tuple(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| match item {
                    TypeTupleItem::Named { name, ty, .. } => {
                        format!(
                            "{}:{}",
                            name,
                            type_key_subst(type_arena, aliases, subst, *ty, d)
                        )
                    }
                    TypeTupleItem::Positional(ty) => {
                        type_key_subst(type_arena, aliases, subst, *ty, d)
                    }
                })
                .collect();
            format!("({})", parts.join(","))
        }
        TypeKind::Function { from, to } => {
            format!(
                "({}->{}",
                type_key_subst(type_arena, aliases, subst, *from, d),
                type_key_subst(type_arena, aliases, subst, *to, d)
            )
        }
        TypeKind::Effect { base, .. } => type_key_subst(type_arena, aliases, subst, *base, d),
        TypeKind::Never => "Never".into(),
        TypeKind::Alias(b) => format!("@{}", b.0),
        TypeKind::TypeVar(b) => match subst.get(b) {
            Some(&t) => type_key_subst(type_arena, aliases, subst, t, d),
            None => format!("@{}", b.0),
        },
        TypeKind::AliasApply { binding, args } => {
            // Expand the parametric alias: substitute its params with the applied
            // args and re-key the body, so `Pair Int` keys as `{fst:Int,snd:Int}`
            // and matches a structurally-keyed witness target.
            if let Some((params, body)) = aliases.get(binding)
                && params.len() == args.len()
            {
                let mut child = subst.clone();
                for (p, a) in params.iter().zip(args.iter()) {
                    child.insert(*p, *a);
                }
                return type_key_subst(type_arena, aliases, &child, *body, d);
            }
            let arg_parts: Vec<String> = args
                .iter()
                .map(|a| type_key_subst(type_arena, aliases, subst, *a, d))
                .collect();
            format!("${}[{}]", binding.0, arg_parts.join(","))
        }
        TypeKind::Con(b) => format!("@{}", b.0),
        TypeKind::Apply { .. } => {
            // Flatten the curried spine to head + args.
            let mut args_acc: Vec<TypeId> = Vec::new();
            let mut cur = ty;
            while let TypeKind::Apply { func, arg } = &type_arena[cur.0 as usize].kind {
                args_acc.push(*arg);
                cur = *func;
            }
            args_acc.reverse();
            // Saturated named-alias head: expand + substitute (mirror AliasApply).
            if let TypeKind::Alias(b) = &type_arena[cur.0 as usize].kind
                && let Some((params, body)) = aliases.get(b)
                && params.len() == args_acc.len()
            {
                let mut child = subst.clone();
                for (p, a) in params.iter().zip(args_acc.iter()) {
                    child.insert(*p, *a);
                }
                return type_key_subst(type_arena, aliases, &child, *body, d);
            }
            let head_key = type_key_subst(type_arena, aliases, subst, cur, d);
            let arg_parts: Vec<String> = args_acc
                .iter()
                .map(|a| type_key_subst(type_arena, aliases, subst, *a, d))
                .collect();
            format!("{}[{}]", head_key, arg_parts.join(","))
        }
        TypeKind::InferVar(v) => format!("?{v}"),
        TypeKind::Error => "<error>".into(),
    }
}

/// Row-tail key suffix, mirroring the THIR lowerer's `row_tail_key`. `Closed`
/// adds nothing so concrete witness targets key exactly as before; open and
/// row-variable tails get a distinct marker.
fn row_tail_key(tail: RowTail) -> String {
    match tail {
        RowTail::Closed => String::new(),
        RowTail::Open => "...".to_string(),
        RowTail::Param(b) => format!("...#{}", b.0),
        RowTail::Infer(v) => format!("...?{v}"),
    }
}

/// Repeatedly resolves `TypeKind::Alias` entries through `aliases` until
/// a non-alias type or an unknown alias is reached.
fn resolve_alias_chain(
    type_arena: &[Type],
    aliases: &HashMap<BindingId, (Vec<BindingId>, TypeId)>,
    mut ty: TypeId,
) -> TypeId {
    let mut fuel = 64u8;
    while fuel > 0 {
        match &type_arena[ty.0 as usize].kind {
            TypeKind::Alias(b) => match aliases.get(b) {
                Some((_, next)) => {
                    ty = *next;
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
    key.starts_with('@') || key.contains('?') || key.contains('$')
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
        Value::TlcClosure(_) => "Function",
        Value::Builtin(_) => "Function",
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
