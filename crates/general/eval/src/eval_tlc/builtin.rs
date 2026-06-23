use super::*;
use crate::posit::{posit_add, posit_cmp, posit_div, posit_mul, posit_sub};

impl<'a> TlcEvaluator<'a> {
    pub(super) fn eval_builtin_expr<'eval>(
        self,
        op: BuiltinOp,
        lhs_id: TlcExprId,
        rhs_id: TlcExprId,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        if op == BuiltinOp::And || op == BuiltinOp::Or {
            let lhs_control = self.eval_control(lhs_id, &env, resume.clone())?;
            return self.bind_control(lhs_control, move |lhs, this| match (op, lhs) {
                (BuiltinOp::And, Value::Bool(false)) => Ok(EvalControl::Value(Value::Bool(false))),
                (BuiltinOp::And, Value::Bool(true)) | (BuiltinOp::Or, Value::Bool(false)) => {
                    let rhs_control = this.eval_control(rhs_id, &env, resume.clone())?;
                    this.bind_control(rhs_control, bool_control)
                }
                (BuiltinOp::Or, Value::Bool(true)) => Ok(EvalControl::Value(Value::Bool(true))),
                (_, other) => Err(EvalError::TypeMismatch {
                    expected: "Bool",
                    found: value_type_name(&other),
                }),
            });
        }

        let lhs_control = self.eval_control(lhs_id, &env, resume.clone())?;
        self.bind_control(lhs_control, move |lhs, this| {
            let rhs_control = this.eval_control(rhs_id, &env, resume.clone())?;
            let lhs_saved = lhs.clone();
            let resume_for_rhs = resume.clone();
            this.bind_control(rhs_control, move |rhs, this| {
                let (lhs_value, rhs_value) =
                    if matches!(op, BuiltinOp::Eq | BuiltinOp::Ne | BuiltinOp::Coalesce) {
                        (
                            tlc_force_deep(lhs_saved.clone(), &this)?,
                            tlc_force_deep(rhs, &this)?,
                        )
                    } else {
                        (lhs_saved.clone(), rhs)
                    };
                if let Some((method, negate)) = this.imported_operator_method(op, lhs_id) {
                    let rhs_for_method = rhs_value.clone();
                    let resume_for_method = resume_for_rhs.clone();
                    let first = this.apply(method, lhs_value.clone(), resume_for_method.clone())?;
                    return this.bind_control(first, move |method_value, this| {
                        let applied = this.apply(
                            method_value,
                            rhs_for_method.clone(),
                            resume_for_method.clone(),
                        )?;
                        if !negate {
                            return Ok(applied);
                        }
                        this.bind_control(applied, |value, _this| match value {
                            Value::Bool(value) => Ok(EvalControl::Value(Value::Bool(!value))),
                            other => Err(EvalError::TypeMismatch {
                                expected: "Bool",
                                found: value_type_name(&other),
                            }),
                        })
                    });
                }
                eval_builtin(op, lhs_value, rhs_value).map(EvalControl::Value)
            })
        })
    }

    pub(super) fn imported_operator_method(
        &self,
        op: BuiltinOp,
        lhs_id: TlcExprId,
    ) -> Option<(Value, bool)> {
        let witnesses = self.operator_witnesses?;
        let target = self.tlc_expr_target_key(lhs_id)?;
        let method = match op {
            BuiltinOp::Eq => "==",
            BuiltinOp::Ne => "!=",
            BuiltinOp::Lt => "<",
            BuiltinOp::Le => "<=",
            BuiltinOp::Gt => ">",
            BuiltinOp::Ge => ">=",
            _ => return None,
        };
        let key = (method.to_string(), target.clone());
        if let Some(value) = witnesses.get(&key) {
            return Some((value.clone(), false));
        }
        if op == BuiltinOp::Ne {
            let eq_key = ("==".to_string(), target);
            return witnesses.get(&eq_key).cloned().map(|value| (value, true));
        }
        None
    }

    pub(super) fn imported_method_by_name(&self, method: &str) -> Option<Value> {
        let witnesses = self.operator_witnesses?;
        let mut found = None;
        for ((name, _target), value) in witnesses {
            if name == method {
                if found.is_some() {
                    return None;
                }
                found = Some(value.clone());
            }
        }
        found
    }

    /// Type-directed imported-witness method dispatch.
    ///
    /// `getfield_id` is the `GetField(dict, method)` node for a constraint-method
    /// call. Its recorded TLC type is the *generic* method scheme, so the concrete
    /// operand type is not recoverable from it; instead the lowerer records the
    /// call site's concrete dispatch key in `dict_dispatch_keys`. We key the
    /// imported-witness lookup on that string so multiple same-method instances
    /// (`Eq @Int`, `Eq @Bool`) resolve to the witness whose target matches the
    /// operand.
    ///
    /// When a dispatch key was recorded but no witness matches it — including an
    /// abstract/unkeyable operand (empty key) or a parametric/conditional target
    /// no concrete instance covers — we refuse rather than fall back to a
    /// type-unaware by-name pick: a wrong witness is worse than a refused
    /// evaluation. Only nodes with no recorded key (non-constraint-method field
    /// access) use the unambiguous by-name match.
    pub(super) fn imported_method(&self, method: &str, getfield_id: TlcExprId) -> Option<Value> {
        if let Some(target) = self.module.dict_dispatch_keys.get(&getfield_id) {
            let witnesses = self.operator_witnesses?;
            return witnesses
                .get(&(method.to_string(), target.clone()))
                .cloned();
        }
        self.imported_method_by_name(method)
    }
}

// ── standalone helpers ─────────────────────────────────────────────────────────
pub(super) fn tlc_module_can_defer_aggregates(module: &TlcModule) -> bool {
    !module.expr_arena.iter().any(|(_, expr)| {
        matches!(
            expr,
            TlcExpr::Perform { .. } | TlcExpr::Handle { .. } | TlcExpr::Resume { .. }
        )
    })
}

pub(super) fn value_cont<'eval>() -> EvalCont<'eval> {
    Rc::new(|value| Ok(EvalControl::Value(value)))
}

pub(super) fn bool_control<'eval, 'module>(
    value: Value,
    _ev: TlcEvaluator<'module>,
) -> Result<EvalControl<'eval>, EvalError> {
    match value {
        Value::Bool(value) => Ok(EvalControl::Value(Value::Bool(value))),
        other => Err(EvalError::TypeMismatch {
            expected: "Bool",
            found: value_type_name(&other),
        }),
    }
}

pub fn eval_literal(lit: &Literal) -> Value {
    match lit {
        Literal::Int(n) => Value::Int(*n),
        Literal::Float(f) => Value::Float(*f),
        Literal::Posit(literal) => Value::Posit(*literal),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Str(s) => Value::Text(Rc::from(s.as_str())),
        Literal::Atom(s) => Value::Atom(Rc::from(s.as_str())),
        Literal::Nothing => Value::Nothing,
    }
}

pub(super) fn lit_matches(lit: &Literal, val: &Value) -> bool {
    match (lit, val) {
        (Literal::Int(n), Value::Int(m)) => n == m,
        (Literal::Float(a), Value::Float(b)) => a == b,
        (Literal::Posit(a), Value::Posit(b)) => a == b,
        (Literal::Bool(a), Value::Bool(b)) => a == b,
        (Literal::Str(a), Value::Text(b)) => a.as_str() == b.as_ref(),
        (Literal::Atom(a), Value::Atom(b)) => a.as_str() == b.as_ref(),
        (Literal::Nothing, Value::Nothing) => true,
        _ => false,
    }
}

pub(super) fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Posit(_) => "Posit",
        Value::Text(_) => "Text",
        Value::Atom(_) => "Atom",
        Value::List(_) => "List",
        Value::Tuple(_) => "Tuple",
        Value::Record(_) => "Record",
        Value::Closure(_)
        | Value::TlcClosure(_)
        | Value::Builtin(_)
        | Value::BuiltinPartial { .. } => "Function",
        Value::TypeValue(_) => "Type",
        Value::TaggedValue { .. } => "TaggedValue",
        Value::Nothing => "Nothing",
        Value::WitnessDict(_) => "WitnessDict",
    }
}

pub(super) fn eval_builtin(op: BuiltinOp, lhs: Value, rhs: Value) -> Result<Value, EvalError> {
    match op {
        BuiltinOp::Add => int_float_op(lhs, rhs, "+", |a, b| a.checked_add(b), |a, b| a + b),
        BuiltinOp::Sub => int_float_op(lhs, rhs, "-", |a, b| a.checked_sub(b), |a, b| a - b),
        BuiltinOp::Mul => int_float_op(lhs, rhs, "*", |a, b| a.checked_mul(b), |a, b| a * b),
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
            Value::Nothing => Ok(rhs),
            Value::Atom(a) if a.as_ref() == "none" || a.as_ref() == "absent" => Ok(rhs),
            Value::TaggedValue { tag, .. }
                if tag.as_ref() == "none" || tag.as_ref() == "absent" =>
            {
                Ok(rhs)
            }
            Value::TaggedValue { tag, payload }
                if tag.as_ref() == "some" || tag.as_ref() == "present" =>
            {
                match payload.iter().find(|(n, _)| n.as_ref() == "0") {
                    Some((_, thunk)) => thunk
                        .peek()
                        .ok_or(EvalError::Internal("unforced wrapper payload in TLC")),
                    None => Err(EvalError::TypeMismatch {
                        expected: "Tuple slot 0",
                        found: "TaggedValue",
                    }),
                }
            }
            other => Err(EvalError::TypeMismatch {
                expected: "Optional or Maybe",
                found: value_type_name(&other),
            }),
        },
    }
}

pub(super) fn structural_eq(a: &Value, b: &Value) -> bool {
    // Delegates to Value's PartialEq which handles all variants including
    // compound types (Record, List, Tuple, TaggedValue) via peek().
    a == b
}

pub(super) fn compare_op(
    lhs: Value,
    rhs: Value,
    target: std::cmp::Ordering,
    or_equal: bool,
) -> Result<Value, EvalError> {
    let ord = match (&lhs, &rhs) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Float(a), Value::Float(b)) => match a.partial_cmp(b) {
            Some(o) => o,
            // IEEE 754: NaN is unordered, so `<`/`<=`/`>`/`>=` are all false.
            None => return Ok(Value::Bool(false)),
        },
        (Value::Posit(a), Value::Posit(b)) => posit_cmp(*a, *b)?,
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

pub(super) fn int_float_op(
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
        (Value::Posit(a), Value::Posit(b)) => {
            let value = match name {
                "+" => posit_add(a, b)?,
                "-" => posit_sub(a, b)?,
                "*" => posit_mul(a, b)?,
                "/" | "div" => posit_div(a, b)?,
                _ => return Err(EvalError::Internal("unknown posit arithmetic operator")),
            };
            Ok(Value::Posit(value))
        }
        (l, r) => Err(EvalError::TypeMismatch {
            expected: "numeric",
            found: if !matches!(l, Value::Int(_) | Value::Float(_) | Value::Posit(_)) {
                value_type_name(&l)
            } else {
                value_type_name(&r)
            },
        }),
    }
}
