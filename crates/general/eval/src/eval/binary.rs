use super::*;

impl<'a> Evaluator<'a> {
    // ── binary operator dispatch ──────────────────────────────────────────────

    pub(super) fn apply_builtin_expr(
        &self,
        func: BuiltinFn,
        mut args: SmallVec<[Thunk; 2]>,
        arg: ThirExprId,
        env: &Env,
    ) -> Result<Value, EvalError> {
        args.push(self.defer(arg, env.clone()));
        if args.len() < func.arity() {
            return Ok(Value::BuiltinPartial { func, args });
        }
        if args.len() != func.arity() {
            return Err(EvalError::TypeMismatch {
                expected: "Function",
                found: "Function",
            });
        }
        self.eval_builtin(func, &args)
    }

    pub(super) fn eval_builtin(&self, func: BuiltinFn, args: &[Thunk]) -> Result<Value, EvalError> {
        match func {
            BuiltinFn::Print => match args[0].force(self)? {
                Value::Text(s) => {
                    println!("{s}");
                    Ok(Value::Text(s))
                }
                other => Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&other),
                }),
            },
            BuiltinFn::Fields => {
                let arg = args[0].force(self)?;
                self.reflect_fields_value(arg)
            }
            BuiltinFn::Variants => {
                let arg = args[0].force(self)?;
                self.reflect_variants_value(arg)
            }
            BuiltinFn::Schema => {
                let arg = args[0].force(self)?;
                self.reflect_schema_value(arg)
            }
            BuiltinFn::Overlay | BuiltinFn::OverlayDeep => {
                let patch = args[0].force(self)?;
                let base = args[1].force(self)?;
                let mut force = |thunk: &Thunk| thunk.force(self);
                overlay_value(base, patch, func == BuiltinFn::OverlayDeep, &mut force)
            }
        }
    }

    pub(super) fn eval_binary(
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
                    Value::Atom(a) if a.as_ref() == "none" || a.as_ref() == "absent" => {
                        self.eval(rhs, env)
                    }
                    Value::TaggedValue { tag, .. }
                        if tag.as_ref() == "none" || tag.as_ref() == "absent" =>
                    {
                        self.eval(rhs, env)
                    }
                    Value::TaggedValue { tag, payload }
                        if tag.as_ref() == "some" || tag.as_ref() == "present" =>
                    {
                        force_tagged_slot(&payload, self)
                    }
                    other => Err(EvalError::TypeMismatch {
                        expected: "Optional or Maybe",
                        found: value_type_name(&other),
                    }),
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
                let key = self.cached_type_key(self.expr(lhs).ty);
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
                let key = self.cached_type_key(self.expr(lhs).ty);
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
}

pub(super) fn tagged_slot_thunk(tag: &'static str, thunk: Thunk) -> Value {
    Value::TaggedValue {
        tag: Rc::from(tag),
        payload: Rc::new(vec![(Rc::from("0"), thunk)]),
    }
}

pub(super) fn tagged_slot_value(tag: &'static str, value: Value) -> Value {
    tagged_slot_thunk(tag, Thunk::ready(value))
}

pub(super) fn force_tagged_slot(
    payload: &Rc<Vec<(Rc<str>, Thunk)>>,
    evaluator: &Evaluator<'_>,
) -> Result<Value, EvalError> {
    match payload.iter().find(|(name, _)| name.as_ref() == "0") {
        Some((_, thunk)) => thunk.force(evaluator),
        None => Err(EvalError::TypeMismatch {
            expected: "Tuple slot 0",
            found: "TaggedValue",
        }),
    }
}
