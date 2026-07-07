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
            BuiltinFn::LoadZti | BuiltinFn::LoadZt => Err(EvalError::EffectfulNotExecutable(
                "dynamic load builtins execute through the TLC evaluator".to_string(),
            )),
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
            BuiltinFn::ListEmpty => Ok(Value::List(Rc::from(Vec::<Thunk>::new()))),
            BuiltinFn::ListCons => {
                // The head thunk stays unforced — `listCons` is lazy in its element.
                let head = args[0].clone();
                match args[1].force(self)? {
                    Value::List(items) => {
                        let mut elems = Vec::with_capacity(items.len() + 1);
                        elems.push(head);
                        elems.extend(items.iter().cloned());
                        Ok(Value::List(Rc::from(elems)))
                    }
                    other => Err(EvalError::TypeMismatch {
                        expected: "List",
                        found: value_type_name(&other),
                    }),
                }
            }
            BuiltinFn::ListAppend => {
                let left = args[0].force(self)?;
                let right = args[1].force(self)?;
                append_list_values(left, right)
            }
            BuiltinFn::ListIsNil => match args[0].force(self)? {
                Value::List(items) => Ok(Value::Bool(items.is_empty())),
                other => Err(EvalError::TypeMismatch {
                    expected: "List",
                    found: value_type_name(&other),
                }),
            },
            BuiltinFn::ListHead => match args[0].force(self)? {
                Value::List(items) => match items.first() {
                    Some(head) => head.force(self),
                    None => Err(EvalError::Internal("listHead on an empty list")),
                },
                other => Err(EvalError::TypeMismatch {
                    expected: "List",
                    found: value_type_name(&other),
                }),
            },
            BuiltinFn::ListTail => match args[0].force(self)? {
                Value::List(items) => match items.split_first() {
                    Some((_, rest)) => Ok(Value::List(rest.iter().cloned().collect())),
                    None => Err(EvalError::Internal("listTail on an empty list")),
                },
                other => Err(EvalError::TypeMismatch {
                    expected: "List",
                    found: value_type_name(&other),
                }),
            },
            BuiltinFn::ListFoldlStrict => {
                let func = args[0].force(self)?;
                let mut acc = args[1].force(self)?;
                match args[2].force(self)? {
                    Value::List(items) => {
                        for elem in items.iter() {
                            let partially_applied =
                                self.apply_value_to_thunk(func.clone(), Thunk::ready(acc))?;
                            let next =
                                self.apply_value_to_thunk(partially_applied, elem.clone())?;
                            acc = Thunk::ready(next).force(self)?;
                        }
                        Ok(acc)
                    }
                    other => Err(EvalError::TypeMismatch {
                        expected: "List",
                        found: value_type_name(&other),
                    }),
                }
            }
            BuiltinFn::NumAbs
            | BuiltinFn::NumRem
            | BuiltinFn::NumPow
            | BuiltinFn::NumToFloat
            | BuiltinFn::NumRound
            | BuiltinFn::NumTruncate => {
                let values = args
                    .iter()
                    .map(|arg| arg.force(self))
                    .collect::<Result<Vec<_>, _>>()?;
                eval_num_builtin_values(func, &values)
            }
            BuiltinFn::TextLength
            | BuiltinFn::TextSplit
            | BuiltinFn::TextJoin
            | BuiltinFn::TextTrim
            | BuiltinFn::TextToUpper
            | BuiltinFn::TextToLower
            | BuiltinFn::TextContains
            | BuiltinFn::TextReplace
            | BuiltinFn::TextShow
            | BuiltinFn::TextParseInt
            | BuiltinFn::TextParseFloat => {
                let values = args
                    .iter()
                    .map(|arg| force_deep(arg.force(self)?, self))
                    .collect::<Result<Vec<_>, _>>()?;
                eval_text_builtin_values(func, &values)
            }
        }
    }

    fn apply_value_to_thunk(&self, func: Value, arg: Thunk) -> Result<Value, EvalError> {
        match func {
            Value::Closure(c) => {
                let mut applied = c.applied.clone();
                applied.push(arg);
                if applied.len() < c.arity {
                    Ok(Value::Closure(Rc::new(Closure {
                        binding: c.binding,
                        arity: c.arity,
                        clauses: c.clauses.clone(),
                        applied,
                        env: c.env.clone(),
                        home: c.home,
                    })))
                } else {
                    self.apply_closure(&c, applied)
                }
            }
            Value::Builtin(func) => self.eval_builtin_or_partial(func, SmallVec::new(), arg),
            Value::BuiltinPartial { func, args } => self.eval_builtin_or_partial(func, args, arg),
            other => Err(EvalError::TypeMismatch {
                expected: "Function",
                found: value_type_name(&other),
            }),
        }
    }

    fn eval_builtin_or_partial(
        &self,
        func: BuiltinFn,
        mut args: SmallVec<[Thunk; 2]>,
        arg: Thunk,
    ) -> Result<Value, EvalError> {
        args.push(arg);
        if args.len() < func.arity() {
            Ok(Value::BuiltinPartial { func, args })
        } else if args.len() == func.arity() {
            self.eval_builtin(func, &args)
        } else {
            Err(EvalError::TypeMismatch {
                expected: "Function",
                found: "Function",
            })
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
