use super::*;

impl<'a> TlcEvaluator<'a> {
    pub(super) fn eval_record<'eval>(
        self,
        fields: Vec<(String, TlcExprId)>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        if self.defer_aggregates {
            let pairs = fields
                .into_iter()
                .map(|(name, id)| {
                    (
                        Rc::from(name.as_str()),
                        Thunk::tlc_deferred(id, env.clone(), self.active_module),
                    )
                })
                .collect();
            return Ok(EvalControl::Value(Value::Record(Rc::new(pairs))));
        }

        let names = Rc::new(
            fields
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
        );
        let ids = Rc::new(fields.into_iter().map(|(_, id)| id).collect::<Vec<_>>());
        let finish: FinishValues<'eval> = Rc::new(move |values| {
            let pairs = names
                .iter()
                .cloned()
                .zip(values)
                .map(|(name, value)| (Rc::from(name.as_str()), Thunk::ready(value)))
                .collect();
            Value::Record(Rc::new(pairs))
        });
        self.eval_expr_values(ids, env, resume, 0, Vec::new(), finish)
    }

    pub(super) fn eval_record_update<'eval>(
        self,
        receiver: TlcExprId,
        fields: Vec<(String, TlcExprId)>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        let metadata = self
            .module
            .expr_types
            .get(&receiver)
            .copied()
            .and_then(|ty| self.tlc_record_field_order(ty));
        let receiver_ty = self.module.expr_types.get(&receiver).copied();
        let updates = Rc::new(fields);
        let receiver_control = self.eval_control(receiver, &env, resume.clone())?;
        self.bind_control(receiver_control, move |base, this| {
            let Value::Record(base_fields) = base else {
                return Err(EvalError::TypeMismatch {
                    expected: "Record",
                    found: value_type_name(&base),
                });
            };
            let metadata = metadata.clone().unwrap_or_else(|| {
                base_fields
                    .iter()
                    .map(|(name, _)| (name.to_string(), false))
                    .collect()
            });
            if this.defer_aggregates {
                let update_thunks: Vec<(String, Thunk)> = updates
                    .iter()
                    .map(|(name, id)| {
                        (
                            name.clone(),
                            Thunk::tlc_deferred(*id, env.clone(), this.active_module),
                        )
                    })
                    .collect();
                return Ok(EvalControl::Value(this.update_tlc_record_value(
                    receiver_ty,
                    &metadata,
                    &base_fields,
                    &update_thunks,
                )));
            }

            let names = Rc::new(
                updates
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>(),
            );
            let ids = Rc::new(updates.iter().map(|(_, id)| *id).collect::<Vec<_>>());
            let finish_ev = this;
            let finish: FinishValues<'eval> = Rc::new(move |values| {
                let update_thunks: Vec<(String, Thunk)> = names
                    .iter()
                    .cloned()
                    .zip(values)
                    .map(|(name, value)| (name, Thunk::ready(value)))
                    .collect();
                finish_ev.update_tlc_record_value(
                    receiver_ty,
                    &metadata,
                    &base_fields,
                    &update_thunks,
                )
            });
            this.eval_expr_values(ids, env.clone(), resume.clone(), 0, Vec::new(), finish)
        })
    }

    fn update_tlc_record_value(
        &self,
        receiver_ty: Option<TlcTypeId>,
        metadata: &[(String, bool)],
        base_fields: &Rc<Vec<(Rc<str>, Thunk)>>,
        updates: &[(String, Thunk)],
    ) -> Value {
        let updates: Vec<(String, Thunk)> = updates
            .iter()
            .map(|(name, thunk)| {
                let optional = receiver_ty
                    .and_then(|ty| self.tlc_field_meta(ty, name))
                    .is_some_and(|(optional, _)| optional);
                if !optional {
                    return (name.clone(), thunk.clone());
                }
                let value = match thunk.peek() {
                    Some(Value::Atom(tag)) if tag.as_ref() == "absent" => thunk.clone(),
                    Some(Value::TaggedValue { tag, .. })
                        if tag.as_ref() == "absent" || tag.as_ref() == "present" =>
                    {
                        thunk.clone()
                    }
                    Some(value) => Thunk::ready(Value::TaggedValue {
                        tag: Rc::from("present"),
                        payload: Rc::new(vec![(Rc::from("0"), Thunk::ready(value))]),
                    }),
                    None => thunk.clone(),
                };
                (name.clone(), value)
            })
            .collect();
        update_record_value(metadata, base_fields, &updates)
    }

    pub(super) fn eval_tuple<'eval>(
        self,
        items: Vec<TlcTupleItem>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        if self.defer_aggregates {
            let fields: Vec<TupleField> = items
                .into_iter()
                .map(|item| match item {
                    TlcTupleItem::Named { name, value } => TupleField {
                        name: Some(Rc::from(name.as_str())),
                        value: Thunk::tlc_deferred(value, env.clone(), self.active_module),
                    },
                    TlcTupleItem::Positional(value) => TupleField {
                        name: None,
                        value: Thunk::tlc_deferred(value, env.clone(), self.active_module),
                    },
                })
                .collect();
            return Ok(EvalControl::Value(Value::Tuple(fields.into())));
        }

        let mut names = Vec::with_capacity(items.len());
        let mut ids = Vec::with_capacity(items.len());
        for item in items {
            match item {
                TlcTupleItem::Named { name, value } => {
                    names.push(Some(name));
                    ids.push(value);
                }
                TlcTupleItem::Positional(value) => {
                    names.push(None);
                    ids.push(value);
                }
            }
        }
        let names = Rc::new(names);
        let finish: FinishValues<'eval> = Rc::new(move |values| {
            let fields: Vec<TupleField> = values
                .into_iter()
                .enumerate()
                .map(|(index, value)| TupleField {
                    name: names[index].as_ref().map(|name| Rc::from(name.as_str())),
                    value: Thunk::ready(value),
                })
                .collect();
            Value::Tuple(fields.into())
        });
        self.eval_expr_values(Rc::new(ids), env, resume, 0, Vec::new(), finish)
    }

    pub(super) fn eval_list<'eval>(
        self,
        items: Vec<TlcExprId>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        if self.defer_aggregates {
            let elements = items
                .into_iter()
                .map(|id| Thunk::tlc_deferred(id, env.clone(), self.active_module))
                .collect::<Vec<_>>();
            return Ok(EvalControl::Value(Value::List(elements.into())));
        }

        let finish: FinishValues<'eval> = Rc::new(|values| {
            Value::List(
                values
                    .into_iter()
                    .map(Thunk::ready)
                    .collect::<Vec<_>>()
                    .into(),
            )
        });
        self.eval_expr_values(Rc::new(items), env, resume, 0, Vec::new(), finish)
    }

    pub(super) fn eval_expr_values<'eval>(
        self,
        ids: Rc<Vec<TlcExprId>>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
        index: usize,
        acc: Vec<Value>,
        finish: FinishValues<'eval>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        let Some(&id) = ids.get(index) else {
            return Ok(EvalControl::Value(finish(acc)));
        };
        let control = self.eval_control(id, &env, resume.clone())?;
        self.bind_control(control, move |value, this| {
            let mut next = acc.clone();
            next.push(value);
            this.eval_expr_values(
                Rc::clone(&ids),
                env.clone(),
                resume.clone(),
                index + 1,
                next,
                Rc::clone(&finish),
            )
        })
    }

    pub(super) fn eval_case<'eval>(
        self,
        scrutinee: Value,
        alts: Rc<Vec<TlcAlt>>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
        index: usize,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        let Some(alt) = alts.get(index).cloned() else {
            return Err(EvalError::NoMatchingClause);
        };
        let match_env = env.push_frame();
        if !self.match_pattern(&alt.pat, &scrutinee, &match_env)? {
            return self.eval_case(scrutinee, Rc::clone(&alts), env, resume, index + 1);
        }
        if let Some(guard_id) = alt.guard {
            let guard_control = self.eval_control(guard_id, &match_env, resume.clone())?;
            return self.bind_control(guard_control, move |guard, this| match guard {
                Value::Bool(true) => Ok(EvalControl::Tail {
                    ev: this,
                    id: alt.body,
                    env: match_env.clone(),
                    resume: resume.clone(),
                }),
                _ => this.eval_case(
                    scrutinee.clone(),
                    Rc::clone(&alts),
                    env.clone(),
                    resume.clone(),
                    index + 1,
                ),
            });
        }
        Ok(EvalControl::Tail {
            ev: self,
            id: alt.body,
            env: match_env,
            resume,
        })
    }
}
