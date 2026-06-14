//! THIR tree-walk evaluator — the only THIR-specific file in this crate.
//!
//! This is deliberately the single "swappable" file: when TLC is built, a
//! parallel `eval_tlc.rs` will be added here reusing everything in `value`,
//! `thunk`, and `env` unchanged.
//!
//! The `Evaluator` holds read-only references to the `ThirFile` arenas.  It is
//! cheaply copyable (two references) and is passed by reference to helpers.

use std::collections::HashMap;
use std::rc::Rc;

use zutai_hir::BindingId;
use zutai_syntax::ast::BinOp;
use zutai_thir::{
    ImportKey, ThirClause, ThirDeclId, ThirDeclKind, ThirExprId, ThirExprKind, ThirFile, ThirPatId,
};
use zutai_thir::{ThirPatKind, ThirTupleItem, ThirTuplePatItem};

use crate::{
    EvalError,
    env::Env,
    thunk::Thunk,
    value::{Closure, TupleField, Value, values_equal},
};

/// Holds read-only access to the THIR arenas while evaluating.
#[derive(Clone, Copy)]
pub struct Evaluator<'a> {
    pub file: &'a ThirFile,
    /// Used when looking up function declarations by their `BindingId` to build
    /// `Closure` values at env-building time.
    pub decls_by_binding: &'a HashMap<BindingId, ThirDeclId>,
    /// Pre-resolved `.zti` import values, keyed by import source.  An `Import`
    /// node evaluates by looking up its source here.
    pub imports: &'a HashMap<ImportKey, Value>,
}

impl<'a> Evaluator<'a> {
    pub fn new(
        file: &'a ThirFile,
        decls_by_binding: &'a HashMap<BindingId, ThirDeclId>,
        imports: &'a HashMap<ImportKey, Value>,
    ) -> Self {
        Self {
            file,
            decls_by_binding,
            imports,
        }
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

            // ── binding reference ────────────────────────────────────────────
            ThirExprKind::BindingRef(b) => {
                let thunk = env.lookup(*b)?;
                thunk.force(self)
            }

            // ── data constructors (lazy) ─────────────────────────────────────
            ThirExprKind::List(items) => {
                let thunks: Rc<[Thunk]> = items
                    .iter()
                    .map(|&item| Thunk::deferred(item, env.clone()))
                    .collect();
                Ok(Value::List(thunks))
            }
            ThirExprKind::Record(fields) => {
                let vec: Vec<(Rc<str>, Thunk)> = fields
                    .iter()
                    .map(|f| {
                        (
                            Rc::from(f.name.as_str()),
                            Thunk::deferred(f.value, env.clone()),
                        )
                    })
                    .collect();
                Ok(Value::Record(Rc::new(vec)))
            }
            ThirExprKind::Tuple(items) => {
                let fields: Rc<[TupleField]> = items
                    .iter()
                    .map(|item| match item {
                        ThirTupleItem::Named { name, value, .. } => TupleField {
                            name: Some(Rc::from(name.as_str())),
                            value: Thunk::deferred(*value, env.clone()),
                        },
                        ThirTupleItem::Positional(e) => TupleField {
                            name: None,
                            value: Thunk::deferred(*e, env.clone()),
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
                    let thunk = Thunk::deferred(local.value, child.clone());
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
                    other => Err(EvalError::TypeMismatch {
                        expected: "Record",
                        found: value_type_name(&other),
                    }),
                }
            }

            // ── function application ──────────────────────────────────────────
            ThirExprKind::Apply { func, arg, .. } => {
                // Force the function position.
                let fv = self.eval(*func, env)?;
                match fv {
                    Value::Closure(c) => {
                        let mut applied = c.applied.clone();
                        applied.push(Thunk::deferred(*arg, env.clone()));
                        if applied.len() < c.arity {
                            // Partial application — return a new closure.
                            Ok(Value::Closure(Rc::new(Closure {
                                binding: c.binding,
                                arity: c.arity,
                                clauses: c.clauses.clone(),
                                env: c.env.clone(),
                                applied,
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
                let eq = values_equal(&lv, &rv, self)?;
                Ok(Value::Bool(eq))
            }
            BinOp::Ne => {
                let eq = values_equal(&lv, &rv, self)?;
                Ok(Value::Bool(!eq))
            }
            BinOp::Lt => cmp_op(lv, rv, std::cmp::Ordering::Less, false),
            BinOp::Le => cmp_op(lv, rv, std::cmp::Ordering::Less, true),
            BinOp::Gt => cmp_op(lv, rv, std::cmp::Ordering::Greater, false),
            BinOp::Ge => cmp_op(lv, rv, std::cmp::Ordering::Greater, true),
            // Already handled above.
            BinOp::And | BinOp::Or | BinOp::Coalesce => unreachable!(),
        }
    }

    // ── clause / pattern matching ─────────────────────────────────────────────

    /// Try all clauses of `closure` with the given argument thunks.
    fn apply_closure(&self, closure: &Closure, args: Vec<Thunk>) -> Result<Value, EvalError> {
        for clause in closure.clauses.iter() {
            let mut child = closure.env.push_frame();
            if self.match_all_patterns(&clause.patterns, &args, &mut child)? {
                // Check guard (if any).
                if let Some(guard_id) = clause.guard {
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
                return self.eval(clause.body, &child);
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
        }
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
                    // Deferred thunk — captures `top` (letrec).
                    let thunk = Thunk::deferred(*value, top.clone());
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
                    };
                    // Functions are pre-evaluated to closures.
                    top.insert(decl.binding, Thunk::ready(Value::Closure(Rc::new(closure))));
                }
                ThirDeclKind::TypeAlias { ty, .. } => {
                    // Type aliases are available as type values.
                    top.insert(decl.binding, Thunk::ready(Value::TypeValue(*ty)));
                }
            }
        }
        top
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

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
        Value::Nothing => "Nothing",
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
