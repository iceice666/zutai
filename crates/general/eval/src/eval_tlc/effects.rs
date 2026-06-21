use super::*;

impl<'a> TlcEvaluator<'a> {
    pub(super) fn apply<'eval>(
        self,
        fv: Value,
        arg: Value,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        match fv {
            Value::TlcClosure(c) => {
                let child = c.env.push_frame();
                child.insert(c.param, Thunk::ready(arg));
                let home_ev = self.for_module(c.home)?;
                home_ev.eval_control(c.body, &child, resume)
            }
            Value::Builtin(func) => self.apply_builtin(func, Vec::new(), arg),
            Value::BuiltinPartial { func, args } => self.apply_builtin(func, args, arg),
            other => Err(EvalError::TypeMismatch {
                expected: "Function",
                found: value_type_name(&other),
            }),
        }
    }

    fn apply_builtin<'eval>(
        self,
        func: BuiltinFn,
        mut args: Vec<Thunk>,
        arg: Value,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        args.push(Thunk::ready(arg));
        if args.len() < func.arity() {
            return Ok(EvalControl::Value(Value::BuiltinPartial { func, args }));
        }
        if args.len() != func.arity() {
            return Err(EvalError::TypeMismatch {
                expected: "Function",
                found: "Function",
            });
        }
        match func {
            BuiltinFn::Print => {
                let arg = args[0].force_tlc(&self)?;
                match arg {
                    Value::Text(_) => Ok(EvalControl::Perform {
                        op: "io.print".to_string(),
                        arg,
                        cont: value_cont(),
                    }),
                    other => Err(EvalError::TypeMismatch {
                        expected: "Text",
                        found: value_type_name(&other),
                    }),
                }
            }
            BuiltinFn::Fields | BuiltinFn::Schema => Err(EvalError::EffectfulNotExecutable(
                "reflection builtins execute through the THIR type-value evaluator".to_string(),
            )),
            BuiltinFn::Overlay | BuiltinFn::OverlayDeep => {
                let patch = args[0].force_tlc(&self)?;
                let base = args[1].force_tlc(&self)?;
                let mut force = |thunk: &Thunk| thunk.force_tlc(&self);
                Ok(EvalControl::Value(overlay_value(
                    base,
                    patch,
                    func == BuiltinFn::OverlayDeep,
                    &mut force,
                )?))
            }
        }
    }

    pub(super) fn handle_control<'eval>(
        self,
        control: EvalControl<'eval>,
        value_clause: Option<TlcExprId>,
        ops: Rc<Vec<TlcHandleClause>>,
        env: Env,
        outer_resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        match control {
            EvalControl::Value(value) => {
                self.apply_value_clause(value, value_clause, env, outer_resume)
            }
            EvalControl::Perform { op, arg, cont } => {
                if let Some(clause) = ops.iter().find(|clause| clause.op == op).cloned() {
                    let this = self;
                    let ops_for_resume = Rc::clone(&ops);
                    let env_for_resume = env.clone();
                    let outer_for_resume = outer_resume.clone();
                    let resume_cont: EvalCont<'eval> = Rc::new(move |resume_value| {
                        let resumed = cont(resume_value)?;
                        this.handle_control(
                            resumed,
                            value_clause,
                            Rc::clone(&ops_for_resume),
                            env_for_resume.clone(),
                            outer_for_resume.clone(),
                        )
                    });
                    let handler_control =
                        self.eval_control(clause.body, &env, Some(Rc::clone(&resume_cont)))?;
                    self.bind_control(handler_control, move |handler, this| {
                        this.apply(handler, arg.clone(), Some(Rc::clone(&resume_cont)))
                    })
                } else {
                    let this = self;
                    let ops_for_resume = Rc::clone(&ops);
                    Ok(EvalControl::Perform {
                        op,
                        arg,
                        cont: Rc::new(move |resume_value| {
                            let resumed = cont(resume_value)?;
                            this.handle_control(
                                resumed,
                                value_clause,
                                Rc::clone(&ops_for_resume),
                                env.clone(),
                                outer_resume.clone(),
                            )
                        }),
                    })
                }
            }
        }
    }

    pub(super) fn apply_value_clause<'eval>(
        self,
        value: Value,
        value_clause: Option<TlcExprId>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        let Some(value_clause) = value_clause else {
            return Ok(EvalControl::Value(value));
        };
        let clause_control = self.eval_control(value_clause, &env, resume.clone())?;
        self.bind_control(clause_control, move |handler, this| {
            this.apply(handler, value.clone(), resume.clone())
        })
    }

    pub(super) fn bind_control<'eval>(
        self,
        control: EvalControl<'eval>,
        f: impl Fn(Value, TlcEvaluator<'a>) -> Result<EvalControl<'eval>, EvalError> + 'eval,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        self.bind_rc(control, Rc::new(f))
    }

    pub(super) fn bind_rc<'eval>(
        self,
        control: EvalControl<'eval>,
        f: BindFn<'eval, 'a>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        match control {
            EvalControl::Value(value) => f(value, self),
            EvalControl::Perform { op, arg, cont } => {
                let this = self;
                Ok(EvalControl::Perform {
                    op,
                    arg,
                    cont: Rc::new(move |resume_value| {
                        let next = cont(resume_value)?;
                        this.bind_rc(next, Rc::clone(&f))
                    }),
                })
            }
        }
    }

    pub(super) fn finish_top<'eval>(self, control: EvalControl<'eval>) -> Result<Value, EvalError>
    where
        'a: 'eval,
    {
        match control {
            EvalControl::Value(value) => Ok(value),
            EvalControl::Perform { op, arg, cont } if op == "io.print" => match arg {
                Value::Text(text) => {
                    if let Some(host_prints) = self.host_prints {
                        host_prints.borrow_mut().push(text.to_string());
                    } else {
                        println!("{text}");
                    }
                    let next = cont(Value::Text(text))?;
                    self.finish_top(next)
                }
                other => Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&other),
                }),
            },
            EvalControl::Perform { op, .. } => Err(EvalError::UnhandledEffect(op)),
        }
    }
}
