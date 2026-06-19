//! Eager (call-by-value) TLC evaluator.
//!
//! Walks a `TlcModule` produced by `zutai-tlc::lower_thir`.  Because TLC has
//! fully elaborated all type abstractions, the evaluator skips `TyLam`/`TyApp`
//! (type-erasure semantics) and dispatches constraint methods via `GetField` on
//! the already-injected dict record — no witness-resolution needed at eval time.
//!
//! All values produced here are wrapped in `Thunk::ready(…)`, so `peek()` always
//! returns `Some`.  `force()` is never called; there are no deferred thunks in
//! TLC evaluation.

use std::rc::Rc;

use zutai_tlc::{
    BuiltinOp, Literal, TlcDecl, TlcExpr, TlcExprId, TlcModule, TlcPat, TlcPatItem, TlcTupleItem,
};

use crate::{
    EvalError,
    env::Env,
    thunk::Thunk,
    value::{BuiltinFn, TlcClosure, TupleField, Value},
};

pub struct TlcEvaluator<'a> {
    pub module: &'a TlcModule,
}

impl<'a> TlcEvaluator<'a> {
    pub fn new(module: &'a TlcModule) -> Self {
        Self { module }
    }

    pub fn eval_expr(&self, id: TlcExprId, env: &Env) -> Result<Value, EvalError> {
        match &self.module.expr_arena[id] {
            TlcExpr::Lit(lit) => Ok(eval_literal(lit)),

            TlcExpr::Var(b) => {
                let thunk = env.lookup(*b)?;
                thunk
                    .peek()
                    .ok_or(EvalError::Internal("unforced thunk in TLC evaluator"))
            }

            // Type erasure: TyLam and TyApp are semantic no-ops at runtime.
            TlcExpr::TyLam(_, _, body) => self.eval_expr(*body, env),
            TlcExpr::TyApp(func, _) => self.eval_expr(*func, env),

            TlcExpr::Lam(param, _, body) => Ok(Value::TlcClosure(Rc::new(TlcClosure {
                param: *param,
                body: *body,
                env: env.clone(),
            }))),

            TlcExpr::App(func, arg) => {
                let fv = self.eval_expr(*func, env)?;
                let av = self.eval_expr(*arg, env)?;
                self.apply(fv, av)
            }

            TlcExpr::Let {
                binding,
                value,
                body,
                ..
            } => {
                let v = self.eval_expr(*value, env)?;
                let child = env.push_frame();
                child.insert(*binding, Thunk::ready(v));
                self.eval_expr(*body, &child)
            }

            TlcExpr::Letrec { bindings, body } => {
                let child = env.push_frame();
                for (binding, _, value_id) in bindings {
                    let v = self.eval_expr(*value_id, &child)?;
                    child.insert(*binding, Thunk::ready(v));
                }
                self.eval_expr(*body, &child)
            }

            TlcExpr::Record(fields) => {
                let pairs: Vec<(Rc<str>, Thunk)> = fields
                    .iter()
                    .map(|(name, expr_id)| {
                        self.eval_expr(*expr_id, env)
                            .map(|v| (Rc::from(name.as_str()), Thunk::ready(v)))
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Value::Record(Rc::new(pairs)))
            }

            TlcExpr::GetField(expr_id, field) => match self.eval_expr(*expr_id, env)? {
                Value::Record(fields) => {
                    for (name, thunk) in fields.iter() {
                        if name.as_ref() == field.as_str() {
                            return thunk
                                .peek()
                                .ok_or(EvalError::Internal("unforced field thunk in TLC"));
                        }
                    }
                    Ok(Value::Nothing)
                }
                other => Err(EvalError::TypeMismatch {
                    expected: "Record",
                    found: value_type_name(&other),
                }),
            },

            TlcExpr::Tuple(items) => {
                let fields: Vec<TupleField> = items
                    .iter()
                    .map(|item| match item {
                        TlcTupleItem::Named {
                            name,
                            value: expr_id,
                        } => self.eval_expr(*expr_id, env).map(|v| TupleField {
                            name: Some(Rc::from(name.as_str())),
                            value: Thunk::ready(v),
                        }),
                        TlcTupleItem::Positional(expr_id) => {
                            self.eval_expr(*expr_id, env).map(|v| TupleField {
                                name: None,
                                value: Thunk::ready(v),
                            })
                        }
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Value::Tuple(fields.into()))
            }

            TlcExpr::List(items) => {
                let thunks: Vec<Thunk> = items
                    .iter()
                    .map(|&expr_id| self.eval_expr(expr_id, env).map(Thunk::ready))
                    .collect::<Result<_, _>>()?;
                Ok(Value::List(thunks.into()))
            }

            TlcExpr::Variant(tag, payload_id) => {
                let payload = self.eval_expr(*payload_id, env)?;
                let pairs: Rc<Vec<(Rc<str>, Thunk)>> = match payload {
                    Value::Record(fields) => fields,
                    Value::Nothing => Rc::new(vec![]),
                    v => Rc::new(vec![(Rc::from("value"), Thunk::ready(v))]),
                };
                Ok(Value::TaggedValue {
                    tag: Rc::from(tag.as_str()),
                    payload: pairs,
                })
            }

            TlcExpr::Case(scrutinee_id, alts) => {
                let scrutinee = self.eval_expr(*scrutinee_id, env)?;
                for alt in alts {
                    let match_env = env.push_frame();
                    if self.match_pattern(&alt.pat, &scrutinee, &match_env) {
                        if let Some(guard_id) = alt.guard {
                            match self.eval_expr(guard_id, &match_env)? {
                                Value::Bool(true) => {}
                                _ => continue,
                            }
                        }
                        return self.eval_expr(alt.body, &match_env);
                    }
                }
                Err(EvalError::NoMatchingClause)
            }

            TlcExpr::Builtin(op, lhs_id, rhs_id) => {
                // And/Or short-circuit: only evaluate rhs when needed.
                match op {
                    BuiltinOp::And => {
                        let lv = self.eval_expr(*lhs_id, env)?;
                        return match lv {
                            Value::Bool(false) => Ok(Value::Bool(false)),
                            Value::Bool(true) => match self.eval_expr(*rhs_id, env)? {
                                Value::Bool(b) => Ok(Value::Bool(b)),
                                other => Err(EvalError::TypeMismatch {
                                    expected: "Bool",
                                    found: value_type_name(&other),
                                }),
                            },
                            other => Err(EvalError::TypeMismatch {
                                expected: "Bool",
                                found: value_type_name(&other),
                            }),
                        };
                    }
                    BuiltinOp::Or => {
                        let lv = self.eval_expr(*lhs_id, env)?;
                        return match lv {
                            Value::Bool(true) => Ok(Value::Bool(true)),
                            Value::Bool(false) => match self.eval_expr(*rhs_id, env)? {
                                Value::Bool(b) => Ok(Value::Bool(b)),
                                other => Err(EvalError::TypeMismatch {
                                    expected: "Bool",
                                    found: value_type_name(&other),
                                }),
                            },
                            other => Err(EvalError::TypeMismatch {
                                expected: "Bool",
                                found: value_type_name(&other),
                            }),
                        };
                    }
                    _ => {}
                }
                let lhs = self.eval_expr(*lhs_id, env)?;
                let rhs = self.eval_expr(*rhs_id, env)?;
                eval_builtin(*op, lhs, rhs)
            }
        }
    }

    fn apply(&self, fv: Value, arg: Value) -> Result<Value, EvalError> {
        match fv {
            Value::TlcClosure(c) => {
                let child = c.env.push_frame();
                child.insert(c.param, Thunk::ready(arg));
                self.eval_expr(c.body, &child)
            }
            Value::Builtin(BuiltinFn::Print) => match arg {
                Value::Text(s) => {
                    println!("{s}");
                    Ok(Value::Text(s))
                }
                other => Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&other),
                }),
            },
            other => Err(EvalError::TypeMismatch {
                expected: "Function",
                found: value_type_name(&other),
            }),
        }
    }

    /// Try to match `pat` against `val`, inserting bindings into `env`.
    /// Returns `true` on a successful match.
    fn match_pattern(&self, pat: &TlcPat, val: &Value, env: &Env) -> bool {
        match pat {
            TlcPat::Wildcard => true,
            TlcPat::Bind(b) => {
                env.insert(*b, Thunk::ready(val.clone()));
                true
            }
            TlcPat::Lit(lit) => lit_matches(lit, val),
            TlcPat::Atom(s) => matches!(val, Value::Atom(a) if a.as_ref() == s.as_str()),
            TlcPat::Tuple(items) => {
                if let Value::Tuple(fields) = val {
                    if items.len() != fields.len() {
                        return false;
                    }
                    for (item, field) in items.iter().zip(fields.iter()) {
                        let fv = match field.value.peek() {
                            Some(v) => v,
                            None => return false,
                        };
                        let sub_pat = match item {
                            TlcPatItem::Positional(p) => p,
                            TlcPatItem::Named { pat, .. } => pat,
                        };
                        if !self.match_pattern(sub_pat, &fv, env) {
                            return false;
                        }
                    }
                    true
                } else {
                    false
                }
            }
            TlcPat::Record(field_pats) => {
                if let Value::Record(record_fields) = val {
                    for (name, sub_pat) in field_pats {
                        let found = record_fields
                            .iter()
                            .find(|(n, _)| n.as_ref() == name.as_str());
                        match found {
                            Some((_, thunk)) => {
                                let fv = match thunk.peek() {
                                    Some(v) => v,
                                    None => return false,
                                };
                                if !self.match_pattern(sub_pat, &fv, env) {
                                    return false;
                                }
                            }
                            None => return false,
                        }
                    }
                    true
                } else {
                    false
                }
            }
            TlcPat::Variant(tag, inner_pat) => {
                if let Value::TaggedValue {
                    tag: val_tag,
                    payload,
                } = val
                {
                    if val_tag.as_ref() != tag.as_str() {
                        return false;
                    }
                    // Match inner pattern against a synthetic Record of the payload.
                    let payload_val = Value::Record(Rc::clone(payload));
                    self.match_pattern(inner_pat, &payload_val, env)
                } else if let Value::Atom(a) = val {
                    // Bare atom variant — no payload; inner must be Wildcard.
                    a.as_ref() == tag.as_str() && matches!(inner_pat.as_ref(), TlcPat::Wildcard)
                } else {
                    false
                }
            }
        }
    }

    /// Build the top-level environment by evaluating all value decls in order.
    pub fn build_top_env(&self) -> Result<Env, EvalError> {
        let top = Env::empty();
        for &decl_id in &self.module.decls {
            match &self.module.decl_arena[decl_id] {
                TlcDecl::Value { binding, body, .. } => {
                    let v = self.eval_expr(*body, &top)?;
                    top.insert(*binding, Thunk::ready(v));
                }
                TlcDecl::TypeAlias { .. } => {}
            }
        }
        Ok(top)
    }
}

// ── standalone helpers ─────────────────────────────────────────────────────────

pub fn eval_literal(lit: &Literal) -> Value {
    match lit {
        Literal::Int(n) => Value::Int(*n),
        Literal::Float(f) => Value::Float(*f),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Str(s) => Value::Text(Rc::from(s.as_str())),
        Literal::Atom(s) => Value::Atom(Rc::from(s.as_str())),
        Literal::Nothing => Value::Nothing,
    }
}

fn lit_matches(lit: &Literal, val: &Value) -> bool {
    match (lit, val) {
        (Literal::Int(n), Value::Int(m)) => n == m,
        (Literal::Float(a), Value::Float(b)) => a == b,
        (Literal::Bool(a), Value::Bool(b)) => a == b,
        (Literal::Str(a), Value::Text(b)) => a.as_str() == b.as_ref(),
        (Literal::Atom(a), Value::Atom(b)) => a.as_str() == b.as_ref(),
        (Literal::Nothing, Value::Nothing) => true,
        _ => false,
    }
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
        Value::Closure(_) | Value::TlcClosure(_) | Value::Builtin(_) => "Function",
        Value::TypeValue(_) => "Type",
        Value::TaggedValue { .. } => "TaggedValue",
        Value::Nothing => "Nothing",
        Value::WitnessDict(_) => "WitnessDict",
    }
}

fn eval_builtin(op: BuiltinOp, lhs: Value, rhs: Value) -> Result<Value, EvalError> {
    match op {
        BuiltinOp::Add => int_float_op(lhs, rhs, "add", |a, b| a.checked_add(b), |a, b| a + b),
        BuiltinOp::Sub => int_float_op(lhs, rhs, "sub", |a, b| a.checked_sub(b), |a, b| a - b),
        BuiltinOp::Mul => int_float_op(lhs, rhs, "mul", |a, b| a.checked_mul(b), |a, b| a * b),
        BuiltinOp::Div => {
            if matches!((&lhs, &rhs), (Value::Int(_), Value::Int(0))) {
                return Err(EvalError::DivByZero);
            }
            int_float_op(lhs, rhs, "div", |a, b| a.checked_div(b), |a, b| a / b)
        }
        BuiltinOp::Eq => Ok(Value::Bool(structural_eq(&lhs, &rhs))),
        BuiltinOp::Ne => Ok(Value::Bool(!structural_eq(&lhs, &rhs))),
        BuiltinOp::Lt => compare_op(lhs, rhs, std::cmp::Ordering::Less, false),
        BuiltinOp::Le => compare_op(lhs, rhs, std::cmp::Ordering::Less, true),
        BuiltinOp::Gt => compare_op(lhs, rhs, std::cmp::Ordering::Greater, false),
        BuiltinOp::Ge => compare_op(lhs, rhs, std::cmp::Ordering::Greater, true),
        // And/Or are handled with short-circuit evaluation in eval_expr before
        // reaching this function; these arms are unreachable in normal execution.
        BuiltinOp::And | BuiltinOp::Or => unreachable!("And/Or are short-circuited in eval_expr"),
        BuiltinOp::Coalesce => match lhs {
            // Absent optional (implicit Nothing or explicit `#none`) → fallback.
            Value::Nothing => Ok(rhs),
            Value::Atom(a) if a.as_ref() == "none" => Ok(rhs),
            Value::TaggedValue { tag, .. } if tag.as_ref() == "none" => Ok(rhs),
            // Explicit `#some { value = x }` → unwrap to `x` (spec desugaring).
            Value::TaggedValue { tag, payload } if tag.as_ref() == "some" => {
                match payload.iter().find(|(n, _)| n.as_ref() == "value") {
                    Some((_, thunk)) => thunk
                        .peek()
                        .ok_or(EvalError::Internal("unforced #some payload in TLC")),
                    None => Ok(Value::TaggedValue { tag, payload }),
                }
            }
            // Present optional already unwrapped to a bare value → pass through.
            other => Ok(other),
        },
    }
}

fn structural_eq(a: &Value, b: &Value) -> bool {
    // Delegates to Value's PartialEq which handles all variants including
    // compound types (Record, List, Tuple, TaggedValue) via peek().
    a == b
}

fn compare_op(
    lhs: Value,
    rhs: Value,
    target: std::cmp::Ordering,
    or_equal: bool,
) -> Result<Value, EvalError> {
    let ord = match (&lhs, &rhs) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        _ => {
            return Err(EvalError::TypeMismatch {
                expected: "comparable type",
                found: value_type_name(&lhs),
            });
        }
    };
    Ok(Value::Bool(
        ord == target || (or_equal && ord == std::cmp::Ordering::Equal),
    ))
}

fn int_float_op(
    lhs: Value,
    rhs: Value,
    name: &'static str,
    int_op: impl Fn(i64, i64) -> Option<i64>,
    float_op: impl Fn(f64, f64) -> f64,
) -> Result<Value, EvalError> {
    match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => int_op(a, b)
            .map(Value::Int)
            .ok_or(EvalError::IntOverflow(name)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(a, b))),
        (l, r) => Err(EvalError::TypeMismatch {
            expected: "numeric",
            found: if !matches!(l, Value::Int(_) | Value::Float(_)) {
                value_type_name(&l)
            } else {
                value_type_name(&r)
            },
        }),
    }
}

/// Recursively force all thunks in a TLC value.
///
/// Since the TLC evaluator wraps every value in `Thunk::ready(…)`, all thunks
/// are already `Forced`; this function just descends into containers to ensure
/// Display can peek at every level.
pub fn tlc_force_deep(v: Value) -> Result<Value, EvalError> {
    match v {
        Value::List(thunks) => {
            let forced: Result<Vec<_>, _> = thunks
                .iter()
                .map(|t| {
                    let inner = t
                        .peek()
                        .ok_or(EvalError::Internal("unforced TLC list element"))?;
                    Ok(Thunk::ready(tlc_force_deep(inner)?))
                })
                .collect();
            Ok(Value::List(forced?.into()))
        }
        Value::Tuple(fields) => {
            let forced: Result<Vec<_>, _> = fields
                .iter()
                .map(|f| {
                    let inner = f
                        .value
                        .peek()
                        .ok_or(EvalError::Internal("unforced TLC tuple field"))?;
                    Ok(TupleField {
                        name: f.name.clone(),
                        value: Thunk::ready(tlc_force_deep(inner)?),
                    })
                })
                .collect();
            Ok(Value::Tuple(forced?.into()))
        }
        Value::Record(fields) => {
            let forced: Result<Vec<_>, _> = fields
                .iter()
                .map(|(name, t)| {
                    let inner = t
                        .peek()
                        .ok_or(EvalError::Internal("unforced TLC record field"))?;
                    Ok((name.clone(), Thunk::ready(tlc_force_deep(inner)?)))
                })
                .collect();
            Ok(Value::Record(Rc::new(forced?)))
        }
        Value::TaggedValue { tag, payload } => {
            let forced: Result<Vec<_>, _> = payload
                .iter()
                .map(|(name, t)| {
                    let inner = t
                        .peek()
                        .ok_or(EvalError::Internal("unforced TLC tagged payload"))?;
                    Ok((name.clone(), Thunk::ready(tlc_force_deep(inner)?)))
                })
                .collect();
            Ok(Value::TaggedValue {
                tag,
                payload: Rc::new(forced?),
            })
        }
        other => Ok(other),
    }
}
