use super::*;
use rustc_hash::FxHashMap;
use std::cell::Cell;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

static LISTENERS: LazyLock<Mutex<FxHashMap<u64, TcpListener>>> =
    LazyLock::new(|| Mutex::new(FxHashMap::default()));
static CONNECTIONS: LazyLock<Mutex<FxHashMap<u64, TcpStream>>> =
    LazyLock::new(|| Mutex::new(FxHashMap::default()));
static CURRENT_CONNECTION: AtomicU64 = AtomicU64::new(0);
static NEXT_NET_ID: AtomicU64 = AtomicU64::new(1);
static READERS: LazyLock<Mutex<FxHashMap<u64, Option<BufReader<File>>>>> =
    LazyLock::new(|| Mutex::new(FxHashMap::default()));
static WRITERS: LazyLock<Mutex<FxHashMap<u64, Option<BufWriter<File>>>>> =
    LazyLock::new(|| Mutex::new(FxHashMap::default()));
static NEXT_FS_ID: AtomicU64 = AtomicU64::new(1);

fn next_net_id() -> u64 {
    NEXT_NET_ID.fetch_add(1, Ordering::Relaxed)
}

fn next_fs_id() -> u64 {
    NEXT_FS_ID.fetch_add(1, Ordering::Relaxed)
}

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
                Ok(EvalControl::Tail {
                    ev: home_ev,
                    id: c.body,
                    env: child,
                    resume,
                })
            }
            Value::Builtin(func) => self.apply_builtin(func, SmallVec::new(), arg),
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
        mut args: SmallVec<[Thunk; 2]>,
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
                        finalizers: Finalizers::new(),
                        cont: value_cont(),
                    }),
                    other => Err(EvalError::TypeMismatch {
                        expected: "Text",
                        found: value_type_name(&other),
                    }),
                }
            }
            BuiltinFn::LoadZti | BuiltinFn::LoadZt => {
                let arg = args[0].force_tlc(&self)?;
                match arg {
                    Value::Text(_) => Ok(EvalControl::Perform {
                        op: match func {
                            BuiltinFn::LoadZti => "load.zti".to_string(),
                            BuiltinFn::LoadZt => "load.zt".to_string(),
                            _ => unreachable!(),
                        },
                        arg,
                        finalizers: Finalizers::new(),
                        cont: value_cont(),
                    }),
                    other => Err(EvalError::TypeMismatch {
                        expected: "Text",
                        found: value_type_name(&other),
                    }),
                }
            }
            BuiltinFn::Fields | BuiltinFn::Variants | BuiltinFn::Schema => {
                Err(EvalError::EffectfulNotExecutable(
                    "reflection builtins execute through the THIR type-value evaluator".to_string(),
                ))
            }
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
            BuiltinFn::ListEmpty => Ok(EvalControl::Value(Value::List(Rc::from(
                Vec::<Thunk>::new(),
            )))),
            BuiltinFn::ListCons => {
                // The head thunk stays unforced — `listCons` is lazy in its element.
                let head = args[0].clone();
                match args[1].force_tlc(&self)? {
                    Value::List(items) => {
                        let mut elems = Vec::with_capacity(items.len() + 1);
                        elems.push(head);
                        elems.extend(items.iter().cloned());
                        Ok(EvalControl::Value(Value::List(Rc::from(elems))))
                    }
                    other => Err(EvalError::TypeMismatch {
                        expected: "List",
                        found: value_type_name(&other),
                    }),
                }
            }
            BuiltinFn::ListAppend => {
                let left = args[0].force_tlc(&self)?;
                let right = args[1].force_tlc(&self)?;
                Ok(EvalControl::Value(append_list_values(left, right)?))
            }
            BuiltinFn::ListIsNil => match args[0].force_tlc(&self)? {
                Value::List(items) => Ok(EvalControl::Value(Value::Bool(items.is_empty()))),
                other => Err(EvalError::TypeMismatch {
                    expected: "List",
                    found: value_type_name(&other),
                }),
            },
            BuiltinFn::ListHead => match args[0].force_tlc(&self)? {
                Value::List(items) => match items.first() {
                    Some(head) => Ok(EvalControl::Value(head.force_tlc(&self)?)),
                    None => Err(EvalError::Internal("listHead on an empty list")),
                },
                other => Err(EvalError::TypeMismatch {
                    expected: "List",
                    found: value_type_name(&other),
                }),
            },
            BuiltinFn::ListTail => match args[0].force_tlc(&self)? {
                Value::List(items) => match items.split_first() {
                    Some((_, rest)) => Ok(EvalControl::Value(Value::List(
                        rest.iter().cloned().collect(),
                    ))),
                    None => Err(EvalError::Internal("listTail on an empty list")),
                },
                other => Err(EvalError::TypeMismatch {
                    expected: "List",
                    found: value_type_name(&other),
                }),
            },
            BuiltinFn::ListFoldlStrict => {
                let func = args[0].force_tlc(&self)?;
                let mut acc = args[1].force_tlc(&self)?;
                match args[2].force_tlc(&self)? {
                    Value::List(items) => {
                        for elem in items.iter() {
                            let partially_applied = self.apply_to_value(func.clone(), acc)?;
                            let elem = elem.force_tlc(&self)?;
                            acc = self.apply_to_value(partially_applied, elem)?;
                        }
                        Ok(EvalControl::Value(acc))
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
                    .map(|arg| arg.force_tlc(&self))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(EvalControl::Value(eval_num_builtin_values(func, &values)?))
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
                    .map(|arg| {
                        let value = arg.force_tlc(&self)?;
                        tlc_force_deep(value, &self)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(EvalControl::Value(eval_text_builtin_values(func, &values)?))
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
        match settle(control)? {
            EvalControl::Value(value) => {
                self.apply_value_clause(value, value_clause, env, outer_resume)
            }
            EvalControl::Perform {
                op,
                arg,
                finalizers,
                cont,
            } => {
                if let Some(clause) = ops.iter().find(|clause| clause.op == op).cloned() {
                    let this = self;
                    let ops_for_resume = Rc::clone(&ops);
                    let env_for_resume = env.clone();
                    let outer_for_resume = outer_resume.clone();
                    // Abort detection is only needed when the effect is suspended
                    // inside one or more `finally` teardowns: a clause that
                    // returns without resuming aborts, discarding the continuation
                    // carrying those teardowns. Keep the common path — no escaped
                    // finalizer — allocation-free by tracking the resume flag only
                    // when it can matter.
                    let resumed: Option<Rc<Cell<bool>>> =
                        (!finalizers.is_empty()).then(|| Rc::new(Cell::new(false)));
                    let resumed_flag = resumed.clone();
                    let resume_cont: EvalCont<'eval> = Rc::new(move |resume_value| {
                        if let Some(flag) = &resumed_flag {
                            flag.set(true);
                        }
                        let resumed = cont(resume_value)?;
                        this.handle_control(
                            resumed,
                            value_clause,
                            Rc::clone(&ops_for_resume),
                            env_for_resume.clone(),
                            outer_for_resume.clone(),
                        )
                    });
                    let resume_for_apply = Rc::clone(&resume_cont);
                    let handler_control =
                        self.eval_control(clause.body, &env, Some(Rc::clone(&resume_cont)))?;
                    let applied = self.bind_control(handler_control, move |handler, this| {
                        this.apply(handler, arg.clone(), Some(Rc::clone(&resume_for_apply)))
                    })?;
                    match resumed {
                        // No escaped finalizer: original behavior, no extra layer.
                        None => Ok(applied),
                        // When the handle settles to a value without `resume`
                        // having fired, the clause aborted. Run the finalizers
                        // that the discarded continuation would have run, using
                        // the current handler for their effects and the existing
                        // finalizer semantics for any abort they trigger.
                        Some(resumed) => {
                            let finalizers: Rc<[EvalCont<'eval>]> = finalizers.into_vec().into();
                            self.bind_control(applied, move |value, this| {
                                if resumed.get() {
                                    return Ok(EvalControl::Value(value));
                                }
                                this.unwind_finalizers(
                                    value,
                                    Rc::clone(&finalizers),
                                    0,
                                    Rc::clone(&ops),
                                    env.clone(),
                                    outer_resume.clone(),
                                )
                            })
                        }
                    }
                } else {
                    let this = self;
                    let ops_for_resume = Rc::clone(&ops);
                    Ok(EvalControl::Perform {
                        op,
                        arg,
                        finalizers,
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
            EvalControl::Tail { .. } => unreachable!("settle drains tail bounces"),
        }
    }

    /// Run a `finally` teardown once the handled computation has reduced to its
    /// final value, threading the value through unchanged. `bind_finally`
    /// preserves any outer-effect `Perform` escapes and re-enters on resume, so
    /// the teardown fires exactly when the terminal value emerges — covering both
    /// normal completion and handler abort. The teardown runs for its effects in
    /// the outer row; its result is discarded.
    ///
    /// Each escaping `Perform` carries the teardown as an explicit finalizer. If
    /// a later handler aborts the effect without resuming, `handle_control`
    /// unwinds those finalizers inner-to-outer instead of leaking them.
    pub(super) fn run_finally<'eval>(
        self,
        control: EvalControl<'eval>,
        finally: TlcExprId,
        env: Env,
        outer_resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        self.bind_finally(
            control,
            Rc::new(move |value, this: TlcEvaluator<'a>| {
                let finally_control = this.eval_control(finally, &env, outer_resume.clone())?;
                this.bind_control(finally_control, move |_discarded, _this| {
                    Ok(EvalControl::Value(value.clone()))
                })
            }),
        )
    }

    /// `bind_rc`, but every `Perform` that escapes before the value emerges is
    /// marked as nested inside one more unwindable `finally` teardown.
    fn bind_finally<'eval>(
        self,
        control: EvalControl<'eval>,
        f: BindFn<'eval, 'a>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        match settle(control)? {
            EvalControl::Value(value) => f(value, self),
            EvalControl::Perform {
                op,
                arg,
                mut finalizers,
                cont,
            } => {
                let this = self;
                let finalizer = {
                    let f = Rc::clone(&f);
                    Rc::new(move |value| f(value, this))
                };
                finalizers.push(finalizer);
                Ok(EvalControl::Perform {
                    op,
                    arg,
                    finalizers,
                    cont: Rc::new(move |resume_value| {
                        let next = cont(resume_value)?;
                        this.bind_finally(next, Rc::clone(&f))
                    }),
                })
            }
            EvalControl::Tail { .. } => unreachable!("settle drains tail bounces"),
        }
    }

    fn unwind_finalizers<'eval>(
        self,
        value: Value,
        finalizers: Rc<[EvalCont<'eval>]>,
        index: usize,
        ops: Rc<Vec<TlcHandleClause>>,
        env: Env,
        outer_resume: Option<EvalCont<'eval>>,
    ) -> Result<EvalControl<'eval>, EvalError>
    where
        'a: 'eval,
    {
        let Some(finalizer) = finalizers.get(index).cloned() else {
            return Ok(EvalControl::Value(value));
        };
        let finalizer_control = finalizer(value)?;
        let handled = self.handle_control(
            finalizer_control,
            None,
            Rc::clone(&ops),
            env.clone(),
            outer_resume.clone(),
        )?;
        self.bind_control(handled, move |value, this| {
            this.unwind_finalizers(
                value,
                Rc::clone(&finalizers),
                index + 1,
                Rc::clone(&ops),
                env.clone(),
                outer_resume.clone(),
            )
        })
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
        match settle(control)? {
            EvalControl::Value(value) => f(value, self),
            EvalControl::Perform {
                op,
                arg,
                finalizers,
                cont,
            } => {
                let this = self;
                Ok(EvalControl::Perform {
                    op,
                    arg,
                    finalizers,
                    cont: Rc::new(move |resume_value| {
                        let next = cont(resume_value)?;
                        this.bind_rc(next, Rc::clone(&f))
                    }),
                })
            }
            EvalControl::Tail { .. } => unreachable!("settle drains tail bounces"),
        }
    }

    pub(super) fn finish_top<'eval>(self, control: EvalControl<'eval>) -> Result<Value, EvalError>
    where
        'a: 'eval,
    {
        match settle(control)? {
            EvalControl::Value(value) => Ok(value),
            EvalControl::Perform { op, arg, cont, .. } => {
                let value = eval_host_op(&op, arg, self)?;
                let next = cont(value)?;
                self.finish_top(next)
            }
            EvalControl::Tail { .. } => unreachable!("settle drains tail bounces"),
        }
    }
}

static RNG_STATE: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

fn eval_host_op(op: &str, arg: Value, evaluator: TlcEvaluator<'_>) -> Result<Value, EvalError> {
    match op {
        "io.print" => match arg {
            Value::Text(text) => {
                println!("{text}");
                Ok(Value::Text(text))
            }
            other => Err(EvalError::TypeMismatch {
                expected: "Text",
                found: value_type_name(&other),
            }),
        },
        "fs.read" => {
            let Value::Text(path) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&arg),
                });
            };
            std::fs::read_to_string(path.as_ref())
                .map(|text| Value::Text(Rc::from(text)))
                .map_err(|err| EvalError::EffectfulNotExecutable(format!("fs.read failed: {err}")))
        }
        "fs.write" => {
            let Value::Record(fields) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Record",
                    found: value_type_name(&arg),
                });
            };
            let path = force_record_text(&fields, "path", evaluator)?;
            let contents = force_record_text(&fields, "contents", evaluator)?;
            std::fs::write(path.as_ref(), contents.as_ref()).map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("fs.write failed: {err}"))
            })?;
            Ok(Value::Tuple(Rc::from([])))
        }
        "fs.openRead" => {
            let Value::Text(path) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&arg),
                });
            };
            let file = File::open(path.as_ref()).map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("fs.openRead failed: {err}"))
            })?;
            let id = next_fs_id();
            READERS.lock().insert(id, Some(BufReader::new(file)));
            Ok(Value::HostHandle(HostHandle {
                kind: HostHandleKind::Reader,
                id: id as i64,
            }))
        }
        "fs.readLine" => {
            let handle = expect_host_handle(arg, HostHandleKind::Reader, "Reader")?;
            let mut readers = READERS.lock();
            let Some(slot) = readers.get_mut(&(handle.id as u64)) else {
                return Err(EvalError::EffectfulNotExecutable(format!(
                    "fs.readLine: reader {} not found",
                    handle.id
                )));
            };
            let Some(reader) = slot.as_mut() else {
                return Err(EvalError::EffectfulNotExecutable(format!(
                    "fs.readLine: reader {} is closed",
                    handle.id
                )));
            };
            let mut line = String::new();
            let bytes = reader.read_line(&mut line).map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("fs.readLine failed: {err}"))
            })?;
            if bytes == 0 {
                Ok(Value::Atom(Rc::from("none")))
            } else {
                let trimmed = strip_read_line_ending(&line);
                Ok(tagged_slot_value(
                    "some",
                    Value::Text(Rc::from(trimmed.to_string())),
                ))
            }
        }
        "fs.closeRead" => {
            let handle = expect_host_handle(arg, HostHandleKind::Reader, "Reader")?;
            let mut readers = READERS.lock();
            let Some(slot) = readers.get_mut(&(handle.id as u64)) else {
                return Err(EvalError::EffectfulNotExecutable(format!(
                    "fs.closeRead: reader {} not found",
                    handle.id
                )));
            };
            *slot = None;
            Ok(Value::Tuple(Rc::from([])))
        }
        "fs.openWrite" => {
            let Value::Text(path) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&arg),
                });
            };
            let file = File::create(path.as_ref()).map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("fs.openWrite failed: {err}"))
            })?;
            let id = next_fs_id();
            WRITERS.lock().insert(id, Some(BufWriter::new(file)));
            Ok(Value::HostHandle(HostHandle {
                kind: HostHandleKind::Writer,
                id: id as i64,
            }))
        }
        "fs.writeText" => {
            let Value::Record(fields) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Record",
                    found: value_type_name(&arg),
                });
            };
            let contents = force_record_text(&fields, "contents", evaluator)?;
            let handle = force_record_host_handle(
                &fields,
                "writer",
                evaluator,
                HostHandleKind::Writer,
                "Writer",
            )?;
            let mut writers = WRITERS.lock();
            let Some(slot) = writers.get_mut(&(handle.id as u64)) else {
                return Err(EvalError::EffectfulNotExecutable(format!(
                    "fs.writeText: writer {} not found",
                    handle.id
                )));
            };
            let Some(writer) = slot.as_mut() else {
                return Err(EvalError::EffectfulNotExecutable(format!(
                    "fs.writeText: writer {} is closed",
                    handle.id
                )));
            };
            writer.write_all(contents.as_bytes()).map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("fs.writeText failed: {err}"))
            })?;
            Ok(Value::Tuple(Rc::from([])))
        }
        "fs.flush" => {
            let handle = expect_host_handle(arg, HostHandleKind::Writer, "Writer")?;
            let mut writers = WRITERS.lock();
            let Some(slot) = writers.get_mut(&(handle.id as u64)) else {
                return Err(EvalError::EffectfulNotExecutable(format!(
                    "fs.flush: writer {} not found",
                    handle.id
                )));
            };
            let Some(writer) = slot.as_mut() else {
                return Err(EvalError::EffectfulNotExecutable(format!(
                    "fs.flush: writer {} is closed",
                    handle.id
                )));
            };
            writer.flush().map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("fs.flush failed: {err}"))
            })?;
            Ok(Value::Tuple(Rc::from([])))
        }
        "fs.closeWrite" => {
            let handle = expect_host_handle(arg, HostHandleKind::Writer, "Writer")?;
            let mut writers = WRITERS.lock();
            let Some(slot) = writers.get_mut(&(handle.id as u64)) else {
                return Err(EvalError::EffectfulNotExecutable(format!(
                    "fs.closeWrite: writer {} not found",
                    handle.id
                )));
            };
            if let Some(mut writer) = slot.take() {
                writer.flush().map_err(|err| {
                    EvalError::EffectfulNotExecutable(format!("fs.closeWrite flush failed: {err}"))
                })?;
            }
            Ok(Value::Tuple(Rc::from([])))
        }
        "env.get" => {
            let Value::Text(name) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&arg),
                });
            };
            match std::env::var(name.as_ref()) {
                Ok(value) => Ok(tagged_slot_value("some", Value::Text(Rc::from(value)))),
                Err(_) => Ok(Value::Atom(Rc::from("none"))),
            }
        }
        "clock.now" => {
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            Ok(Value::Text(Rc::from(millis.to_string())))
        }
        "rng.next" => Ok(Value::Int(next_rng())),
        "load.zti" => {
            let Value::Text(path) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&arg),
                });
            };
            let source = std::fs::read_to_string(path.as_ref()).map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("load.zti failed: {err}"))
            })?;
            let block = zutai_im::parse(&source).map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("load.zti parse failed: {err}"))
            })?;
            Ok(data_from_zti_block(&block))
        }
        "load.zt" => {
            let Value::Text(path) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&arg),
                });
            };
            let value =
                crate::eval_tlc_path(std::path::Path::new(path.as_ref())).map_err(|err| {
                    EvalError::EffectfulNotExecutable(format!("load.zt failed: {err}"))
                })?;
            data_from_value(&value)
        }
        "net.listen" => {
            let Value::Int(port) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Int",
                    found: value_type_name(&arg),
                });
            };
            let port = u16::try_from(port).map_err(|_| {
                EvalError::EffectfulNotExecutable(format!("net.listen: invalid port {port}"))
            })?;
            let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("net.listen failed: {err}"))
            })?;
            let id = next_net_id();
            LISTENERS.lock().insert(id, listener);
            Ok(Value::Int(id as i64))
        }
        "net.accept" => {
            let Value::Int(listener_id) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Int",
                    found: value_type_name(&arg),
                });
            };
            let listeners = LISTENERS.lock();
            let listener = listeners.get(&(listener_id as u64)).ok_or_else(|| {
                EvalError::EffectfulNotExecutable(format!(
                    "net.accept: listener {} not found",
                    listener_id
                ))
            })?;
            let (stream, _addr) = listener.accept().map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("net.accept failed: {err}"))
            })?;
            drop(listeners);
            let conn_id = next_net_id();
            CONNECTIONS.lock().insert(conn_id, stream);
            CURRENT_CONNECTION.store(conn_id, Ordering::Relaxed);
            Ok(Value::Int(conn_id as i64))
        }
        "net.read" => {
            let Value::Int(conn_id) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Int",
                    found: value_type_name(&arg),
                });
            };
            let mut conns = CONNECTIONS.lock();
            let stream = conns.get_mut(&(conn_id as u64)).ok_or_else(|| {
                EvalError::EffectfulNotExecutable(format!(
                    "net.read: connection {} not found",
                    conn_id
                ))
            })?;
            let mut reader = BufReader::new(&mut *stream);
            let mut line = String::new();
            reader.read_line(&mut line).map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("net.read failed: {err}"))
            })?;
            // Trim trailing \r\n or \n
            let trimmed = line.trim_end_matches(['\r', '\n']);
            Ok(Value::Text(Rc::from(trimmed.to_string())))
        }
        "net.write" => {
            let Value::Text(text) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Text",
                    found: value_type_name(&arg),
                });
            };
            let conn_id = CURRENT_CONNECTION.load(Ordering::Relaxed);
            if conn_id == 0 {
                return Err(EvalError::EffectfulNotExecutable(
                    "net.write: no current connection".to_string(),
                ));
            }
            let mut conns = CONNECTIONS.lock();
            let stream = conns.get_mut(&conn_id).ok_or_else(|| {
                EvalError::EffectfulNotExecutable(format!(
                    "net.write: connection {} not found",
                    conn_id
                ))
            })?;
            write!(stream, "{text}").map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("net.write failed: {err}"))
            })?;
            stream.flush().map_err(|err| {
                EvalError::EffectfulNotExecutable(format!("net.write flush failed: {err}"))
            })?;
            Ok(Value::Tuple(Rc::from([])))
        }
        "net.close" => {
            let Value::Int(conn_id) = arg else {
                return Err(EvalError::TypeMismatch {
                    expected: "Int",
                    found: value_type_name(&arg),
                });
            };
            CONNECTIONS.lock().remove(&(conn_id as u64));
            if CURRENT_CONNECTION.load(Ordering::Relaxed) == conn_id as u64 {
                CURRENT_CONNECTION.store(0, Ordering::Relaxed);
            }
            Ok(Value::Tuple(Rc::from([])))
        }
        _ => Err(EvalError::UnhandledEffect(op.to_string())),
    }
}

fn tagged_slot_value(tag: &'static str, value: Value) -> Value {
    Value::TaggedValue {
        tag: Rc::from(tag),
        payload: Rc::new(vec![(Rc::from("0"), Thunk::ready(value))]),
    }
}

fn data_from_zti_block(block: &zutai_im::Block) -> Value {
    let fields = block
        .iter()
        .map(|pair| {
            data_record(vec![
                ("name", Value::Text(Rc::from(pair.field_name.as_str()))),
                ("value", data_from_zti_value(&pair.value)),
            ])
        })
        .collect();
    data_tagged("record", vec![("fields", data_list(fields))])
}

fn strip_read_line_ending(line: &str) -> &str {
    let Some(stripped) = line.strip_suffix('\n') else {
        return line;
    };
    stripped.strip_suffix('\r').unwrap_or(stripped)
}

fn data_from_zti_value(value: &zutai_im::Value) -> Value {
    match value {
        zutai_im::Value::True => data_tagged("bool", vec![("value", Value::Bool(true))]),
        zutai_im::Value::False => data_tagged("bool", vec![("value", Value::Bool(false))]),
        zutai_im::Value::Atom(atom) => data_tagged(
            "atom",
            vec![("value", Value::Text(Rc::from(atom.as_str())))],
        ),
        zutai_im::Value::String(text) => data_tagged(
            "text",
            vec![("value", Value::Text(Rc::from(text.as_str())))],
        ),
        zutai_im::Value::Float(value) => {
            data_tagged("float", vec![("value", Value::Float(*value))])
        }
        zutai_im::Value::Integer(value) => data_tagged("int", vec![("value", Value::Int(*value))]),
        zutai_im::Value::Array(items) => {
            let items = items.iter().map(data_from_zti_value).collect();
            data_tagged("list", vec![("items", data_list(items))])
        }
        zutai_im::Value::Block(block) => data_from_zti_block(block),
    }
}

fn data_from_value(value: &Value) -> Result<Value, EvalError> {
    match value {
        Value::Bool(value) => Ok(data_tagged("bool", vec![("value", Value::Bool(*value))])),
        Value::Int(value) => Ok(data_tagged("int", vec![("value", Value::Int(*value))])),
        Value::Float(value) => Ok(data_tagged("float", vec![("value", Value::Float(*value))])),
        Value::Posit(literal) => Ok(data_tagged(
            "float",
            vec![("value", Value::Float(literal.bits as f64))],
        )),
        Value::Text(value) => Ok(data_tagged(
            "text",
            vec![("value", Value::Text(value.clone()))],
        )),
        Value::Atom(value) => Ok(data_tagged(
            "atom",
            vec![("value", Value::Text(value.clone()))],
        )),
        Value::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                let value = item.peek().ok_or(EvalError::Internal(
                    "load.zt result contains an unforced list item",
                ))?;
                out.push(data_from_value(&value)?);
            }
            Ok(data_tagged("list", vec![("items", data_list(out))]))
        }
        Value::Tuple(items) => {
            let mut fields = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let value = item.value.peek().ok_or(EvalError::Internal(
                    "load.zt result contains an unforced tuple item",
                ))?;
                let name = item
                    .name
                    .as_ref()
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| index.to_string());
                fields.push(data_record(vec![
                    ("name", Value::Text(Rc::from(name))),
                    ("value", data_from_value(&value)?),
                ]));
            }
            Ok(data_tagged("record", vec![("fields", data_list(fields))]))
        }
        Value::Record(source_fields) => {
            let mut fields = Vec::with_capacity(source_fields.len());
            for (name, thunk) in source_fields.iter() {
                let value = thunk.peek().ok_or(EvalError::Internal(
                    "load.zt result contains an unforced record field",
                ))?;
                fields.push(data_record(vec![
                    ("name", Value::Text(name.clone())),
                    ("value", data_from_value(&value)?),
                ]));
            }
            Ok(data_tagged("record", vec![("fields", data_list(fields))]))
        }
        Value::TaggedValue { tag, payload } => {
            let payload = data_from_value(&Value::Record(payload.clone()))?;
            Ok(data_tagged(
                "tagged",
                vec![("payload", payload), ("tag", Value::Text(tag.clone()))],
            ))
        }
        Value::Nothing => Ok(data_tagged(
            "atom",
            vec![("value", Value::Text(Rc::from("absent")))],
        )),
        Value::Closure(_)
        | Value::TypeValue(_)
        | Value::WitnessDict(_)
        | Value::TlcClosure(_)
        | Value::HostHandle(_)
        | Value::Builtin(_)
        | Value::BuiltinPartial { .. } => Err(EvalError::EffectfulNotExecutable(
            "load.zt final value is not first-order serializable data".to_string(),
        )),
    }
}

fn data_list(items: Vec<Value>) -> Value {
    Value::List(items.into_iter().map(Thunk::ready).collect())
}

fn data_record(fields: Vec<(&'static str, Value)>) -> Value {
    Value::Record(Rc::new(
        fields
            .into_iter()
            .map(|(name, value)| (Rc::from(name), Thunk::ready(value)))
            .collect(),
    ))
}

fn data_tagged(tag: &'static str, fields: Vec<(&'static str, Value)>) -> Value {
    Value::TaggedValue {
        tag: Rc::from(tag),
        payload: Rc::new(
            fields
                .into_iter()
                .map(|(name, value)| (Rc::from(name), Thunk::ready(value)))
                .collect(),
        ),
    }
}

fn force_record_text(
    fields: &[(Rc<str>, Thunk)],
    name: &str,
    evaluator: TlcEvaluator<'_>,
) -> Result<Rc<str>, EvalError> {
    let Some((_, thunk)) = fields.iter().find(|(field, _)| field.as_ref() == name) else {
        return Err(EvalError::TypeMismatch {
            expected: "Record field",
            found: "Record",
        });
    };
    match thunk.force_tlc(&evaluator)? {
        Value::Text(text) => Ok(text),
        other => Err(EvalError::TypeMismatch {
            expected: "Text",
            found: value_type_name(&other),
        }),
    }
}

fn expect_host_handle(
    value: Value,
    kind: HostHandleKind,
    expected: &'static str,
) -> Result<HostHandle, EvalError> {
    match value {
        Value::HostHandle(handle) if handle.kind == kind => Ok(handle),
        other => Err(EvalError::TypeMismatch {
            expected,
            found: value_type_name(&other),
        }),
    }
}

fn force_record_host_handle(
    fields: &[(Rc<str>, Thunk)],
    name: &str,
    evaluator: TlcEvaluator<'_>,
    kind: HostHandleKind,
    expected: &'static str,
) -> Result<HostHandle, EvalError> {
    let Some((_, thunk)) = fields.iter().find(|(field, _)| field.as_ref() == name) else {
        return Err(EvalError::TypeMismatch {
            expected: "Record field",
            found: "Record",
        });
    };
    expect_host_handle(thunk.force_tlc(&evaluator)?, kind, expected)
}

fn next_rng() -> i64 {
    let mut state = RNG_STATE.load(Ordering::Relaxed);
    loop {
        let next = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        match RNG_STATE.compare_exchange_weak(state, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return (next >> 1) as i64,
            Err(found) => state = found,
        }
    }
}
