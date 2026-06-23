use super::*;

impl<'a> TlcEvaluator<'a> {
    /// Drive `eval_step` to a settled control (`Value`/`Perform`), bouncing
    /// every `EvalControl::Tail` so a tail-recursive call chain runs in constant
    /// host-stack space. Sub-expression evaluation and `eval_expr` go through
    /// here; only the tail positions inside `eval_step` emit a raw `Tail`.
    pub(super) fn eval_control<'eval>(
        self,
        id: TlcExprId,
        env: &Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        settle(self.eval_step(id, env, resume)?)
    }

    pub(super) fn eval_step<'eval>(
        self,
        id: TlcExprId,
        env: &Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        match self.module.expr_arena[id].clone() {
            TlcExpr::Lit(lit) => Ok(EvalControl::Value(eval_literal(&lit))),

            TlcExpr::Var(b) => {
                let thunk = env.lookup(b)?;
                if let Some(value) = thunk.peek() {
                    return Ok(EvalControl::Value(value));
                }
                if thunk.is_in_progress() {
                    return Err(EvalError::BlackHole);
                }
                thunk.force_tlc(&self).map(EvalControl::Value)
            }

            TlcExpr::Import(source) => {
                match self.imports.and_then(|imports| imports.get(&source)) {
                    Some(value) => Ok(EvalControl::Value(value.clone())),
                    None => Err(EvalError::Internal(
                        "import not resolved (unreachable past gate)",
                    )),
                }
            }

            // Type erasure: TyLam and TyApp are semantic no-ops at runtime.
            TlcExpr::TyLam(_, _, body) => Ok(EvalControl::Tail {
                ev: self,
                id: body,
                env: env.clone(),
                resume,
            }),
            TlcExpr::TyApp(func, _) => Ok(EvalControl::Tail {
                ev: self,
                id: func,
                env: env.clone(),
                resume,
            }),

            TlcExpr::Lam(param, _, body) => {
                Ok(EvalControl::Value(Value::TlcClosure(Rc::new(TlcClosure {
                    param,
                    body,
                    env: env.clone(),
                    home: self.active_module,
                }))))
            }

            TlcExpr::App(func, arg) => {
                let env_for_arg = env.clone();
                let resume_for_arg = resume.clone();
                let func_control = self.eval_control(func, env, resume)?;
                self.bind_control(func_control, move |fv, this| {
                    let arg_control =
                        this.eval_control(arg, &env_for_arg, resume_for_arg.clone())?;
                    let resume_for_apply = resume_for_arg.clone();
                    let fv_saved = fv.clone();
                    this.bind_control(arg_control, move |av, this| {
                        this.apply(fv_saved.clone(), av, resume_for_apply.clone())
                    })
                })
            }

            TlcExpr::Let {
                binding,
                value,
                body,
                ..
            } => {
                if self.defer_aggregates {
                    let child = env.push_frame();
                    child.insert(
                        binding,
                        Thunk::tlc_deferred(value, env.clone(), self.active_module),
                    );
                    return Ok(EvalControl::Tail {
                        ev: self,
                        id: body,
                        env: child,
                        resume,
                    });
                }

                let env_for_body = env.clone();
                let resume_for_body = resume.clone();
                let value_control = self.eval_control(value, env, resume)?;
                self.bind_control(value_control, move |v, this| {
                    let child = env_for_body.push_frame();
                    child.insert(binding, Thunk::ready(v));
                    Ok(EvalControl::Tail {
                        ev: this,
                        id: body,
                        env: child,
                        resume: resume_for_body.clone(),
                    })
                })
            }

            TlcExpr::Letrec { bindings, body } => {
                let child = env.push_frame();
                for (binding, _, _) in &bindings {
                    child.insert(*binding, Thunk::in_progress());
                }
                for (binding, _, value_id) in bindings {
                    let v = self.eval_expr(value_id, &child)?;
                    let placeholder = child.lookup(binding)?;
                    placeholder.replace_forced(v);
                }
                Ok(EvalControl::Tail {
                    ev: self,
                    id: body,
                    env: child,
                    resume,
                })
            }

            TlcExpr::Record(fields) => self.eval_record(fields, env.clone(), resume),
            TlcExpr::RecordUpdate { receiver, fields } => {
                self.eval_record_update(receiver, fields, env.clone(), resume)
            }
            TlcExpr::Tuple(items) => self.eval_tuple(items, env.clone(), resume),
            TlcExpr::List(items) => self.eval_list(items, env.clone(), resume),

            TlcExpr::GetField(expr_id, field) => {
                let recv_ty = self
                    .module
                    .expr_types
                    .get(&expr_id)
                    .copied()
                    .map(|ty_id| self.resolve_tlc_alias_chain(ty_id));

                if let Some(ty_id) = recv_ty {
                    if let Some(wrapper_kind) = self.tlc_type_wrapper_kind(ty_id) {
                        let wrapper_ty = self.resolve_tlc_alias_chain(ty_id);
                        let inner_id = match &self.module.type_arena[wrapper_ty] {
                            TlcType::Optional(inner) | TlcType::Maybe(inner) => *inner,
                            _ => ty_id,
                        };
                        let (absent_tag, present_tag, expected) = match wrapper_kind {
                            TlcWrapperKind::Optional => ("none", "some", "Optional"),
                            TlcWrapperKind::Maybe => ("absent", "present", "Maybe"),
                        };
                        let recv_control = self.eval_control(expr_id, env, resume)?;
                        return self.bind_control(recv_control, move |recv, this| match recv {
                            Value::Atom(atom) if atom.as_ref() == absent_tag => {
                                Ok(EvalControl::Value(Value::Atom(Rc::from(absent_tag))))
                            }
                            Value::TaggedValue { tag, .. } if tag.as_ref() == absent_tag => {
                                Ok(EvalControl::Value(Value::Atom(Rc::from(absent_tag))))
                            }
                            Value::Nothing => {
                                Ok(EvalControl::Value(Value::Atom(Rc::from(absent_tag))))
                            }
                            Value::TaggedValue { tag, payload } if tag.as_ref() == present_tag => {
                                let inner = tlc_force_tagged_slot(&payload, &this)?;
                                match inner {
                                    Value::Record(inner_fields) => {
                                        let projected = this.project_record_field(
                                            inner_id,
                                            &inner_fields,
                                            field.as_str(),
                                        )?;
                                        Ok(EvalControl::Value(tlc_tagged_slot_value(
                                            present_tag,
                                            projected,
                                        )))
                                    }
                                    other => Err(EvalError::TypeMismatch {
                                        expected: "Record",
                                        found: value_type_name(&other),
                                    }),
                                }
                            }
                            other => Err(EvalError::TypeMismatch {
                                expected,
                                found: value_type_name(&other),
                            }),
                        });
                    }

                    if matches!(&self.module.type_arena[ty_id], TlcType::Record(_)) {
                        let recv_control = self.eval_control(expr_id, env, resume)?;
                        return self.bind_control(recv_control, move |recv, this| match recv {
                            Value::Record(fields) => this
                                .project_record_field(ty_id, &fields, field.as_str())
                                .map(EvalControl::Value),
                            Value::Nothing => {
                                if let Some(method) = this.imported_method_by_name(field.as_str()) {
                                    Ok(EvalControl::Value(method))
                                } else {
                                    Err(EvalError::UnboundBinding(zutai_hir::BindingId(u32::MAX)))
                                }
                            }
                            other => Err(EvalError::TypeMismatch {
                                expected: "Record",
                                found: value_type_name(&other),
                            }),
                        });
                    }
                }

                let recv_control = self.eval_control(expr_id, env, resume)?;
                self.bind_control(recv_control, move |recv, this| match recv {
                    Value::Record(fields) => {
                        for (name, thunk) in fields.iter() {
                            if name.as_ref() == field.as_str() {
                                let value = thunk.force_tlc(&this)?;
                                return Ok(EvalControl::Value(value));
                            }
                        }
                        Ok(EvalControl::Value(Value::Nothing))
                    }
                    Value::Nothing => {
                        if let Some(method) = this.imported_method_by_name(field.as_str()) {
                            Ok(EvalControl::Value(method))
                        } else {
                            Err(EvalError::UnboundBinding(zutai_hir::BindingId(u32::MAX)))
                        }
                    }
                    other => Err(EvalError::TypeMismatch {
                        expected: "Record",
                        found: value_type_name(&other),
                    }),
                })
            }

            TlcExpr::Variant(tag, payload_id) => {
                let payload_control = self.eval_control(payload_id, env, resume)?;
                self.bind_control(payload_control, move |payload, _this| {
                    let pairs: Rc<Vec<(Rc<str>, Thunk)>> = match payload {
                        Value::Record(fields) => fields,
                        Value::Tuple(fields) => Rc::new(
                            fields
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
                        ),
                        Value::Nothing => Rc::new(vec![]),
                        v => Rc::new(vec![(Rc::from("0"), Thunk::ready(v))]),
                    };
                    Ok(EvalControl::Value(Value::TaggedValue {
                        tag: Rc::from(tag.as_str()),
                        payload: pairs,
                    }))
                })
            }

            TlcExpr::Case(scrutinee_id, alts) => {
                let alts = Rc::new(alts);
                let env_for_arms = env.clone();
                let resume_for_arms = resume.clone();
                let scrutinee_control = self.eval_control(scrutinee_id, env, resume)?;
                self.bind_control(scrutinee_control, move |scrutinee, this| {
                    this.eval_case(
                        scrutinee,
                        Rc::clone(&alts),
                        env_for_arms.clone(),
                        resume_for_arms.clone(),
                        0,
                    )
                })
            }

            TlcExpr::Builtin(op, lhs_id, rhs_id) => {
                self.eval_builtin_expr(op, lhs_id, rhs_id, env.clone(), resume)
            }

            TlcExpr::Perform { op, arg } => {
                let arg_control = self.eval_control(arg, env, resume)?;
                self.bind_control(arg_control, move |arg, _this| {
                    Ok(EvalControl::Perform {
                        op: op.clone(),
                        arg,
                        cont: value_cont(),
                    })
                })
            }

            TlcExpr::Handle { expr, value, ops } => {
                let ops = Rc::new(ops);
                let env_for_handler = env.clone();
                let outer_resume = resume.clone();
                let control = self.eval_control(expr, env, resume)?;
                self.handle_control(control, value, ops, env_for_handler, outer_resume)
            }

            TlcExpr::Resume { value } => {
                let continuation = resume.clone().ok_or(EvalError::ResumeOutsideHandler)?;
                let value_control = self.eval_control(value, env, resume)?;
                self.bind_control(value_control, move |value, _this| continuation(value))
            }

            TlcExpr::Sequence(items) => {
                let ids = Rc::new(items);
                let finish: FinishValues<'eval> =
                    Rc::new(|values| values.last().cloned().unwrap_or(Value::Nothing));
                self.eval_expr_values(ids, env.clone(), resume, 0, Vec::new(), finish)
            }
        }
    }
}

fn tlc_tagged_slot_value(tag: &'static str, value: Value) -> Value {
    Value::TaggedValue {
        tag: Rc::from(tag),
        payload: Rc::new(vec![(Rc::from("0"), Thunk::ready(value))]),
    }
}

fn tlc_force_tagged_slot(
    payload: &Rc<Vec<(Rc<str>, Thunk)>>,
    evaluator: &TlcEvaluator<'_>,
) -> Result<Value, EvalError> {
    match payload.iter().find(|(name, _)| name.as_ref() == "0") {
        Some((_, thunk)) => thunk.force_tlc(evaluator),
        None => Err(EvalError::TypeMismatch {
            expected: "Tuple slot 0",
            found: "TaggedValue",
        }),
    }
}
