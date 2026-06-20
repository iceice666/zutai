//! Eager (call-by-value) TLC evaluator.
//!
//! Walks a `TlcModule` produced by `zutai-tlc::lower_thir`.  Because TLC has
//! fully elaborated all type abstractions, the evaluator skips `TyLam`/`TyApp`
//! (type-erasure semantics) and dispatches constraint methods via `GetField` on
//! the already-injected dict record — no witness-resolution needed at eval time.
//!
//! Phase 16 adds algebraic-effect execution with delimited continuations:
//! `perform` suspends the current TLC continuation, source `handle` clauses may
//! return directly or `resume`, and the host boundary handles residual
//! `io.print`. All produced values are wrapped in `Thunk::ready(…)`, so
//! `peek()` always returns `Some`; there are no deferred thunks in TLC
//! evaluation.

use std::collections::HashMap;
use std::rc::Rc;

use zutai_thir::ImportKey;
use zutai_tlc::{
    BuiltinOp, Literal, Row, TlcAlt, TlcDecl, TlcExpr, TlcExprId, TlcHandleClause, TlcModule,
    TlcPat, TlcPatItem, TlcTupleItem, TlcType, TlcTypeId, TlcTypeVar,
};

use crate::{
    EvalError,
    env::Env,
    thunk::Thunk,
    value::{BuiltinFn, TlcClosure, TupleField, Value},
};

type EvalCont<'eval> = Rc<dyn Fn(Value) -> Result<EvalControl<'eval>, EvalError> + 'eval>;
type BindFn<'eval, 'module> = Rc<
    dyn Fn(Value, &'eval TlcEvaluator<'module>) -> Result<EvalControl<'eval>, EvalError> + 'eval,
>;
type FinishValues<'eval> = Rc<dyn Fn(Vec<Value>) -> Value + 'eval>;

enum EvalControl<'eval> {
    Value(Value),
    Perform {
        op: String,
        arg: Value,
        cont: EvalCont<'eval>,
    },
}

pub struct TlcEvaluator<'a> {
    pub module: &'a TlcModule,
    imports: Option<&'a HashMap<ImportKey, Value>>,
}

impl<'a> TlcEvaluator<'a> {
    pub fn new(module: &'a TlcModule) -> Self {
        Self {
            module,
            imports: None,
        }
    }

    pub fn new_with_imports(module: &'a TlcModule, imports: &'a HashMap<ImportKey, Value>) -> Self {
        Self {
            module,
            imports: Some(imports),
        }
    }

    pub fn eval_expr(&self, id: TlcExprId, env: &Env) -> Result<Value, EvalError> {
        let control = self.eval_control(id, env, None)?;
        self.finish_top(control)
    }

    fn eval_control<'eval>(
        &'eval self,
        id: TlcExprId,
        env: &Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError> {
        match self.module.expr_arena[id].clone() {
            TlcExpr::Lit(lit) => Ok(EvalControl::Value(eval_literal(&lit))),

            TlcExpr::Var(b) => {
                let thunk = env.lookup(b)?;
                thunk
                    .peek()
                    .map(EvalControl::Value)
                    .ok_or(EvalError::Internal("unforced thunk in TLC evaluator"))
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
            TlcExpr::TyLam(_, _, body) => self.eval_control(body, env, resume),
            TlcExpr::TyApp(func, _) => self.eval_control(func, env, resume),

            TlcExpr::Lam(param, _, body) => {
                Ok(EvalControl::Value(Value::TlcClosure(Rc::new(TlcClosure {
                    param,
                    body,
                    env: env.clone(),
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
                let env_for_body = env.clone();
                let resume_for_body = resume.clone();
                let value_control = self.eval_control(value, env, resume)?;
                self.bind_control(value_control, move |v, this| {
                    let child = env_for_body.push_frame();
                    child.insert(binding, Thunk::ready(v));
                    this.eval_control(body, &child, resume_for_body.clone())
                })
            }

            TlcExpr::Letrec { bindings, body } => {
                let child = env.push_frame();
                // TLC lowering does not currently generate local letrec for
                // effectful value initializers. If that changes, this eager
                // path must become an explicit sequencing decision rather than
                // accidentally host-handling effects during environment setup.
                for (binding, _, value_id) in bindings {
                    let v = self.eval_expr(value_id, &child)?;
                    child.insert(binding, Thunk::ready(v));
                }
                self.eval_control(body, &child, resume)
            }

            TlcExpr::Record(fields) => self.eval_record(fields, env.clone(), resume),
            TlcExpr::Tuple(items) => self.eval_tuple(items, env.clone(), resume),
            TlcExpr::List(items) => self.eval_list(items, env.clone(), resume),

            TlcExpr::GetField(expr_id, field) => {
                let recv_ty = self
                    .module
                    .expr_types
                    .get(&expr_id)
                    .copied()
                    .map(|ty_id| self.resolve_tlc_alias_chain(ty_id));
                match recv_ty {
                    Some(ty_id) => match &self.module.type_arena[ty_id] {
                        TlcType::Optional(inner_id) => {
                            let inner_id = *inner_id;
                            let recv_control = self.eval_control(expr_id, env, resume)?;
                            self.bind_control(recv_control, move |recv, this| match recv {
                                Value::Atom(atom) if atom.as_ref() == "none" => {
                                    Ok(EvalControl::Value(Value::Atom(Rc::from("none"))))
                                }
                                Value::TaggedValue { tag, .. } if tag.as_ref() == "none" => {
                                    Ok(EvalControl::Value(Value::Atom(Rc::from("none"))))
                                }
                                Value::Nothing => {
                                    Ok(EvalControl::Value(Value::Atom(Rc::from("none"))))
                                }
                                Value::TaggedValue { tag, payload } if tag.as_ref() == "some" => {
                                    let inner = match payload
                                        .iter()
                                        .find(|(name, _)| name.as_ref() == "value")
                                    {
                                        Some((_, thunk)) => thunk.peek().ok_or(
                                            EvalError::Internal("unforced #some payload in TLC"),
                                        )?,
                                        None => {
                                            return Err(EvalError::TypeMismatch {
                                                expected: "Record",
                                                found: "TaggedValue",
                                            });
                                        }
                                    };
                                    match inner {
                                        Value::Record(inner_fields) => {
                                            if let Some((true, value_ty)) =
                                                this.tlc_field_meta(inner_id, field.as_str())
                                            {
                                                return this
                                                    .project_optional_field(
                                                        &inner_fields,
                                                        field.as_str(),
                                                        this.tlc_type_is_optional(value_ty),
                                                    )
                                                    .map(EvalControl::Value);
                                            }
                                            match inner_fields
                                                .iter()
                                                .find(|(name, _)| name.as_ref() == field.as_str())
                                            {
                                                Some((_, thunk)) => {
                                                    Ok(EvalControl::Value(Value::TaggedValue {
                                                        tag: Rc::from("some"),
                                                        payload: Rc::new(vec![(
                                                            Rc::from("value"),
                                                            thunk.clone(),
                                                        )]),
                                                    }))
                                                }
                                                None => Ok(EvalControl::Value(Value::Atom(
                                                    Rc::from("none"),
                                                ))),
                                            }
                                        }
                                        other => Err(EvalError::TypeMismatch {
                                            expected: "Record",
                                            found: value_type_name(&other),
                                        }),
                                    }
                                }
                                Value::Record(inner_fields) => {
                                    if let Some((true, value_ty)) =
                                        this.tlc_field_meta(inner_id, field.as_str())
                                    {
                                        return this
                                            .project_optional_field(
                                                &inner_fields,
                                                field.as_str(),
                                                this.tlc_type_is_optional(value_ty),
                                            )
                                            .map(EvalControl::Value);
                                    }
                                    match inner_fields
                                        .iter()
                                        .find(|(name, _)| name.as_ref() == field.as_str())
                                    {
                                        Some((_, thunk)) => {
                                            Ok(EvalControl::Value(Value::TaggedValue {
                                                tag: Rc::from("some"),
                                                payload: Rc::new(vec![(
                                                    Rc::from("value"),
                                                    thunk.clone(),
                                                )]),
                                            }))
                                        }
                                        None => {
                                            Ok(EvalControl::Value(Value::Atom(Rc::from("none"))))
                                        }
                                    }
                                }
                                other => Err(EvalError::TypeMismatch {
                                    expected: "Record",
                                    found: value_type_name(&other),
                                }),
                            })
                        }
                        TlcType::Record(_) => {
                            let recv_control = self.eval_control(expr_id, env, resume)?;
                            self.bind_control(recv_control, move |recv, this| match recv {
                                Value::Record(fields) => {
                                    if let Some((true, value_ty)) =
                                        this.tlc_field_meta(ty_id, field.as_str())
                                    {
                                        return this
                                            .project_optional_field(
                                                &fields,
                                                field.as_str(),
                                                this.tlc_type_is_optional(value_ty),
                                            )
                                            .map(EvalControl::Value);
                                    }
                                    for (name, thunk) in fields.iter() {
                                        if name.as_ref() == field.as_str() {
                                            return thunk.peek().map(EvalControl::Value).ok_or(
                                                EvalError::Internal("unforced field thunk in TLC"),
                                            );
                                        }
                                    }
                                    Ok(EvalControl::Value(Value::Nothing))
                                }
                                other => Err(EvalError::TypeMismatch {
                                    expected: "Record",
                                    found: value_type_name(&other),
                                }),
                            })
                        }
                        _ => {
                            let recv_control = self.eval_control(expr_id, env, resume)?;
                            self.bind_control(recv_control, move |recv, _this| match recv {
                                Value::Record(fields) => {
                                    for (name, thunk) in fields.iter() {
                                        if name.as_ref() == field.as_str() {
                                            return thunk.peek().map(EvalControl::Value).ok_or(
                                                EvalError::Internal("unforced field thunk in TLC"),
                                            );
                                        }
                                    }
                                    Ok(EvalControl::Value(Value::Nothing))
                                }
                                other => Err(EvalError::TypeMismatch {
                                    expected: "Record",
                                    found: value_type_name(&other),
                                }),
                            })
                        }
                    },
                    None => {
                        let recv_control = self.eval_control(expr_id, env, resume)?;
                        self.bind_control(recv_control, move |recv, _this| match recv {
                            Value::Record(fields) => {
                                for (name, thunk) in fields.iter() {
                                    if name.as_ref() == field.as_str() {
                                        return thunk.peek().map(EvalControl::Value).ok_or(
                                            EvalError::Internal("unforced field thunk in TLC"),
                                        );
                                    }
                                }
                                Ok(EvalControl::Value(Value::Nothing))
                            }
                            other => Err(EvalError::TypeMismatch {
                                expected: "Record",
                                found: value_type_name(&other),
                            }),
                        })
                    }
                }
            }

            TlcExpr::Variant(tag, payload_id) => {
                let payload_control = self.eval_control(payload_id, env, resume)?;
                self.bind_control(payload_control, move |payload, _this| {
                    let pairs: Rc<Vec<(Rc<str>, Thunk)>> = match payload {
                        Value::Record(fields) => fields,
                        Value::Nothing => Rc::new(vec![]),
                        v => Rc::new(vec![(Rc::from("value"), Thunk::ready(v))]),
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

    fn eval_record<'eval>(
        &'eval self,
        fields: Vec<(String, TlcExprId)>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError> {
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

    fn eval_tuple<'eval>(
        &'eval self,
        items: Vec<TlcTupleItem>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError> {
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

    fn eval_list<'eval>(
        &'eval self,
        items: Vec<TlcExprId>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError> {
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

    fn eval_expr_values<'eval>(
        &'eval self,
        ids: Rc<Vec<TlcExprId>>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
        index: usize,
        acc: Vec<Value>,
        finish: FinishValues<'eval>,
    ) -> Result<EvalControl<'eval>, EvalError> {
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

    fn eval_case<'eval>(
        &'eval self,
        scrutinee: Value,
        alts: Rc<Vec<TlcAlt>>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
        index: usize,
    ) -> Result<EvalControl<'eval>, EvalError> {
        let Some(alt) = alts.get(index).cloned() else {
            return Err(EvalError::NoMatchingClause);
        };
        let match_env = env.push_frame();
        if !self.match_pattern(&alt.pat, &scrutinee, &match_env) {
            return self.eval_case(scrutinee, Rc::clone(&alts), env, resume, index + 1);
        }
        if let Some(guard_id) = alt.guard {
            let guard_control = self.eval_control(guard_id, &match_env, resume.clone())?;
            return self.bind_control(guard_control, move |guard, this| match guard {
                Value::Bool(true) => this.eval_control(alt.body, &match_env, resume.clone()),
                _ => this.eval_case(
                    scrutinee.clone(),
                    Rc::clone(&alts),
                    env.clone(),
                    resume.clone(),
                    index + 1,
                ),
            });
        }
        self.eval_control(alt.body, &match_env, resume)
    }

    fn eval_builtin_expr<'eval>(
        &'eval self,
        op: BuiltinOp,
        lhs_id: TlcExprId,
        rhs_id: TlcExprId,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError> {
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
            this.bind_control(rhs_control, move |rhs, _this| {
                eval_builtin(op, lhs_saved.clone(), rhs).map(EvalControl::Value)
            })
        })
    }

    fn apply<'eval>(
        &'eval self,
        fv: Value,
        arg: Value,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError> {
        match fv {
            Value::TlcClosure(c) => {
                let child = c.env.push_frame();
                child.insert(c.param, Thunk::ready(arg));
                self.eval_control(c.body, &child, resume)
            }
            Value::Builtin(BuiltinFn::Print) => match arg {
                Value::Text(_) => Ok(EvalControl::Perform {
                    op: "io.print".to_string(),
                    arg,
                    cont: value_cont(),
                }),
                other => Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&other),
                }),
            },
            Value::Builtin(BuiltinFn::Fields | BuiltinFn::Schema) => {
                Err(EvalError::EffectfulNotExecutable(
                    "reflection builtins execute through the THIR type-value evaluator".to_string(),
                ))
            }
            other => Err(EvalError::TypeMismatch {
                expected: "Function",
                found: value_type_name(&other),
            }),
        }
    }

    fn handle_control<'eval>(
        &'eval self,
        control: EvalControl<'eval>,
        value_clause: Option<TlcExprId>,
        ops: Rc<Vec<TlcHandleClause>>,
        env: Env,
        outer_resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError> {
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

    fn apply_value_clause<'eval>(
        &'eval self,
        value: Value,
        value_clause: Option<TlcExprId>,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError> {
        let Some(value_clause) = value_clause else {
            return Ok(EvalControl::Value(value));
        };
        let clause_control = self.eval_control(value_clause, &env, resume.clone())?;
        self.bind_control(clause_control, move |handler, this| {
            this.apply(handler, value.clone(), resume.clone())
        })
    }

    fn bind_control<'eval>(
        &'eval self,
        control: EvalControl<'eval>,
        f: impl Fn(Value, &'eval TlcEvaluator<'a>) -> Result<EvalControl<'eval>, EvalError> + 'eval,
    ) -> Result<EvalControl<'eval>, EvalError> {
        self.bind_rc(control, Rc::new(f))
    }

    fn bind_rc<'eval>(
        &'eval self,
        control: EvalControl<'eval>,
        f: BindFn<'eval, 'a>,
    ) -> Result<EvalControl<'eval>, EvalError> {
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

    fn resolve_tlc_alias_chain(&self, mut ty_id: TlcTypeId) -> TlcTypeId {
        let mut fuel = 64u8;
        while fuel > 0 {
            match &self.module.type_arena[ty_id] {
                TlcType::TyVar(TlcTypeVar::Named(binding), _) => {
                    let Some(next) = self.type_alias_body(*binding) else {
                        break;
                    };
                    ty_id = next;
                    fuel -= 1;
                }
                _ => break,
            }
        }
        ty_id
    }

    fn type_alias_body(&self, binding: u32) -> Option<TlcTypeId> {
        self.module
            .decls
            .iter()
            .find_map(|&decl_id| match &self.module.decl_arena[decl_id] {
                TlcDecl::TypeAlias {
                    binding: alias,
                    params,
                    body,
                } if alias.0 == binding && params.is_empty() => Some(*body),
                _ => None,
            })
    }

    fn tlc_field_meta(&self, ty_id: TlcTypeId, field: &str) -> Option<(bool, TlcTypeId)> {
        let ty_id = self.resolve_tlc_alias_chain(ty_id);
        match &self.module.type_arena[ty_id] {
            TlcType::Record(row) => {
                let mut current = row;
                loop {
                    match current {
                        Row::REmpty | Row::RVar(_) => return None,
                        Row::RExtend {
                            label,
                            ty,
                            optional,
                            tail,
                        } => {
                            if label == field {
                                return Some((*optional, *ty));
                            }
                            current = tail;
                        }
                    }
                }
            }
            _ => None,
        }
    }

    fn tlc_type_is_optional(&self, ty_id: TlcTypeId) -> bool {
        let ty_id = self.resolve_tlc_alias_chain(ty_id);
        matches!(&self.module.type_arena[ty_id], TlcType::Optional(_))
    }

    fn project_optional_field(
        &self,
        fields: &Rc<Vec<(Rc<str>, Thunk)>>,
        field: &str,
        value_already_optional: bool,
    ) -> Result<Value, EvalError> {
        match fields.iter().find(|(name, _)| name.as_ref() == field) {
            None => Ok(Value::Atom(Rc::from("none"))),
            Some((_, thunk)) if value_already_optional => thunk
                .peek()
                .ok_or(EvalError::Internal("unforced optional field thunk in TLC")),
            Some((_, thunk)) => Ok(Value::TaggedValue {
                tag: Rc::from("some"),
                payload: Rc::new(vec![(Rc::from("value"), thunk.clone())]),
            }),
        }
    }

    fn finish_top<'eval>(&'eval self, control: EvalControl<'eval>) -> Result<Value, EvalError> {
        match control {
            EvalControl::Value(value) => Ok(value),
            EvalControl::Perform { op, arg, cont } if op == "io.print" => match arg {
                Value::Text(text) => {
                    println!("{text}");
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
        self.build_top_env_from(Env::empty())
    }

    /// Build top-level declarations on top of a pre-seeded environment.
    pub fn build_top_env_from(&self, top: Env) -> Result<Env, EvalError> {
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
fn value_cont<'eval>() -> EvalCont<'eval> {
    Rc::new(|value| Ok(EvalControl::Value(value)))
}

fn bool_control<'eval, 'module>(
    value: Value,
    _ev: &'eval TlcEvaluator<'module>,
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
