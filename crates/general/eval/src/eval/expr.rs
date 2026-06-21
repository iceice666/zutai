use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RuntimeWrapperKind {
    Optional,
    Maybe,
}

impl<'a> Evaluator<'a> {
    /// Returns (field_may_be_absent, field_value_ty) for `field` on the record
    /// type `ty` (alias-resolved), or None if `ty` is not a record or the field
    /// is not declared.
    pub(super) fn record_field_meta(&self, ty: TypeId, field: &str) -> Option<(bool, TypeId)> {
        let aliases = self.build_alias_map();
        let resolved = resolve_alias_chain(&self.file.type_arena, &aliases, ty);
        match &self.file.type_arena[resolved.0 as usize].kind {
            TypeKind::Record(fields, _) => fields
                .iter()
                .find(|f| f.name == field)
                .map(|f| (f.optional, f.ty)),
            _ => None,
        }
    }

    pub(super) fn record_field_order(&self, ty: TypeId) -> Option<Vec<(String, bool)>> {
        let aliases = self.build_alias_map();
        let resolved = resolve_alias_chain(&self.file.type_arena, &aliases, ty);
        match &self.file.type_arena[resolved.0 as usize].kind {
            TypeKind::Record(fields, _) => Some(
                fields
                    .iter()
                    .map(|field| (field.name.clone(), field.optional))
                    .collect(),
            ),
            _ => None,
        }
    }

    pub(super) fn type_wrapper_kind(&self, ty: TypeId) -> Option<RuntimeWrapperKind> {
        let aliases = self.build_alias_map();
        let resolved = resolve_alias_chain(&self.file.type_arena, &aliases, ty);
        match &self.file.type_arena[resolved.0 as usize].kind {
            TypeKind::Optional(_) => Some(RuntimeWrapperKind::Optional),
            TypeKind::Maybe(_) => Some(RuntimeWrapperKind::Maybe),
            _ => None,
        }
    }

    pub(super) fn project_maybe_field(
        &self,
        fields: &Rc<Vec<(Rc<str>, Thunk)>>,
        field: &str,
    ) -> Value {
        match fields.iter().find(|(name, _)| name.as_ref() == field) {
            None => Value::Atom(Rc::from("absent")),
            Some((_, thunk)) => tagged_slot_thunk("present", thunk.clone()),
        }
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
            ThirExprKind::Posit(literal) => Ok(Value::Posit(*literal)),
            ThirExprKind::String(s) => Ok(Value::Text(Rc::from(s.as_str()))),
            ThirExprKind::Atom(a) => Ok(Value::Atom(Rc::from(a.as_str()))),
            ThirExprKind::TypeValue(ty) => Ok(Value::TypeValue(RuntimeType::new(
                self.active_module,
                *ty,
            ))),
            ThirExprKind::TaggedValue { tag, payload } => {
                let payload_val = self.eval(*payload, env)?;
                let fields = match payload_val {
                    Value::Record(f) => (*f).clone(),
                    Value::Tuple(f) => f
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
                    _ => vec![],
                };
                Ok(Value::TaggedValue {
                    tag: Rc::from(tag.as_str()),
                    payload: Rc::new(fields),
                })
            }

            // ── binding reference ────────────────────────────────────────────
            ThirExprKind::BindingRef(b) => {
                let thunk = env.lookup(*b)?;
                thunk.force(self)
            }

            // ── data constructors (lazy) ─────────────────────────────────────
            ThirExprKind::List(items) => {
                let thunks: Rc<[Thunk]> = items
                    .iter()
                    .map(|&item| self.defer(item, env.clone()))
                    .collect();
                Ok(Value::List(thunks))
            }
            ThirExprKind::Record(fields) => {
                let vec: Vec<(Rc<str>, Thunk)> = fields
                    .iter()
                    .map(|f| (Rc::from(f.name.as_str()), self.defer(f.value, env.clone())))
                    .collect();
                Ok(Value::Record(Rc::new(vec)))
            }
            ThirExprKind::RecordUpdate { receiver, fields } => {
                let rv = self.eval(*receiver, env)?;
                let Value::Record(base_fields) = rv else {
                    return Err(EvalError::TypeMismatch {
                        expected: "Record",
                        found: value_type_name(&rv),
                    });
                };
                let metadata = self
                    .record_field_order(self.expr(*receiver).ty)
                    .unwrap_or_else(|| {
                        base_fields
                            .iter()
                            .map(|(name, _)| (name.to_string(), false))
                            .collect()
                    });
                let updates: Vec<(String, Thunk)> = fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.clone(),
                            self.defer(field.value, env.clone()),
                        )
                    })
                    .collect();
                Ok(update_record_value(&metadata, &base_fields, &updates))
            }
            ThirExprKind::Tuple(items) => {
                let fields: Rc<[TupleField]> = items
                    .iter()
                    .map(|item| match item {
                        ThirTupleItem::Named { name, value, .. } => TupleField {
                            name: Some(Rc::from(name.as_str())),
                            value: self.defer(*value, env.clone()),
                        },
                        ThirTupleItem::Positional(e) => TupleField {
                            name: None,
                            value: self.defer(*e, env.clone()),
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
                    let thunk = self.defer(local.value, child.clone());
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
                        if let Some((true, _)) = self.record_field_meta(self.expr(*receiver).ty, field) {
                            return Ok(self.project_maybe_field(&fields, field));
                        }
                        for (name, thunk) in fields.iter() {
                            if name.as_ref() == field.as_str() {
                                return thunk.force(self);
                            }
                        }
                        Ok(Value::Nothing)
                    }
                    Value::TaggedValue { tag, payload } => {
                        if field == "tag" {
                            return Ok(Value::Atom(tag));
                        }
                        for (name, thunk) in payload.iter() {
                            if name.as_ref() == field.as_str() {
                                return thunk.force(self);
                            }
                        }
                        Ok(Value::Nothing)
                    }
                    other => Err(EvalError::TypeMismatch {
                        expected: "Record",
                        found: value_type_name(&other),
                    }),
                }
            }

            // ── function application ──────────────────────────────────────────
            ThirExprKind::Apply {
                func,
                arg,
                instantiation,
            } => {
                // Witness-dict injection: if func is a BindingRef to a top-level
                // function with param_bounds, inject a WitnessDict for each
                // concrete (non-ambiguous) bound into the caller's env so that
                // method dispatch inside the body can fall back to it.
                if let ThirExprKind::BindingRef(bid) = &self.expr(*func).kind {
                    let bid = *bid;
                    // Find the Function decl with params/param_bounds.
                    let maybe_bounds: Option<(Vec<BindingId>, Vec<Vec<BindingId>>)> =
                        self.file.decls.iter().find_map(|&did| {
                            let decl = self.decl(did);
                            if decl.binding == bid
                                && let ThirDeclKind::Function {
                                    params,
                                    param_bounds,
                                    ..
                                } = &decl.kind
                            {
                                return Some((params.clone(), param_bounds.clone()));
                            }
                            None
                        });
                    if let Some((params, param_bounds)) = maybe_bounds {
                        let aliases = self.build_alias_map();
                        for (i, constraint_bindings) in param_bounds.iter().enumerate() {
                            if i >= params.len() || i >= instantiation.len() {
                                break;
                            }
                            let key = type_key(&self.file.type_arena, &aliases, instantiation[i]);
                            if key.starts_with('@') || key.starts_with('?') || key.starts_with('$')
                            {
                                continue; // ambiguous — can't resolve witness
                            }
                            for &constraint_binding in constraint_bindings {
                                // Find a matching witness.
                                for &decl_id in &self.file.decls {
                                    let decl = self.decl(decl_id);
                                    if let ThirDeclKind::Witness {
                                        constraint: Some(c),
                                        target,
                                        fields,
                                        ..
                                    } = &decl.kind
                                    {
                                        if *c != constraint_binding
                                            || type_key(&self.file.type_arena, &aliases, *target)
                                                != key
                                        {
                                            continue;
                                        }
                                        // Build the witness dict from the witness fields.
                                        let mut dict: HashMap<String, Value> = HashMap::new();
                                        for field in fields {
                                            let v = self.eval(field.value, env)?;
                                            dict.insert(field.name.clone(), v);
                                        }
                                        // NOTE: injecting into the caller's frame only works when
                                        // the callee's closure.env is an ancestor of this frame —
                                        // i.e. direct top-level calls. Indirect calls (bounded fn
                                        // called from another fn) won't see this dict because
                                        // apply_closure builds the body env as
                                        // closure.env.push_frame(), not as env.push_frame(). Those
                                        // cases are caught by try_method_dispatch returning
                                        // EvalError::UnresolvedWitness. Full dictionary-passing
                                        // (threading witnesses through call chains) is deferred to
                                        // the TLC elaboration layer.
                                        //
                                        // Keyed by constraint BindingId. Limitation: if two
                                        // distinct type params are bounded by the same constraint
                                        // (e.g. <A: Eq, B: Eq>), the second insertion clobbers
                                        // the first. Document here but don't fix — the
                                        // indirect-call limitation (see above) is the more
                                        // fundamental boundary.
                                        env.insert(
                                            constraint_binding,
                                            Thunk::ready(Value::WitnessDict(dict)),
                                        );
                                        break; // one witness per constraint
                                    }
                                }
                            }
                        }
                    }
                }

                // Type-directed constraint dispatch: if func is a BindingRef to a
                // named constraint method, look up the matching witness and use its
                // field body as the function value. Falls through to normal eval if
                // no witness matches (returns UnboundBinding via env.lookup).
                let fv = if let Some(v) = self.try_method_dispatch(*func, instantiation, env)? {
                    v
                } else {
                    self.eval(*func, env)?
                };
                match fv {
                    Value::Closure(c) => {
                        let mut applied = c.applied.clone();
                        // The arg is evaluated in the *caller's* module (this
                        // evaluator's active_module), not in the closure's home.
                        applied.push(self.defer(*arg, env.clone()));
                        if applied.len() < c.arity {
                            // Partial application — return a new closure.
                            Ok(Value::Closure(Rc::new(Closure {
                                binding: c.binding,
                                arity: c.arity,
                                clauses: c.clauses.clone(),
                                env: c.env.clone(),
                                applied,
                                home: c.home,
                            })))
                        } else {
                            // All arguments present — try each clause.
                            self.apply_closure(&c, applied)
                        }
                    }
                    Value::Builtin(func) => self.apply_builtin_expr(func, Vec::new(), *arg, env),
                    Value::BuiltinPartial { func, args } => {
                        self.apply_builtin_expr(func, args, *arg, env)
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
                    home: self.active_module,
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
                let Some(wrapper_kind) = self.type_wrapper_kind(self.expr(*receiver).ty) else {
                    return Err(EvalError::TypeMismatch {
                        expected: "Optional or Maybe",
                        found: "non-wrapper",
                    });
                };
                let aliases = self.build_alias_map();
                let receiver_ty =
                    resolve_alias_chain(&self.file.type_arena, &aliases, self.expr(*receiver).ty);
                let inner_ty = match &self.file.type_arena[receiver_ty.0 as usize].kind {
                    TypeKind::Optional(inner) | TypeKind::Maybe(inner) => *inner,
                    _ => receiver_ty,
                };
                let project_inner_field =
                    |fields: &Rc<Vec<(Rc<str>, Thunk)>>| -> Result<Value, EvalError> {
                        if let Some((true, _)) = self.record_field_meta(inner_ty, field) {
                            return Ok(self.project_maybe_field(fields, field));
                        }
                        match fields.iter().find(|(name, _)| name.as_ref() == field.as_str()) {
                            Some((_, thunk)) => thunk.force(self),
                            None => Ok(Value::Nothing),
                        }
                    };

                let rv = self.eval(*receiver, env)?;
                match wrapper_kind {
                    RuntimeWrapperKind::Optional => match rv {
                        Value::Atom(atom) if atom.as_ref() == "none" => Ok(Value::Atom(Rc::from("none"))),
                        Value::TaggedValue { tag, .. } if tag.as_ref() == "none" => {
                            Ok(Value::Atom(Rc::from("none")))
                        }
                        Value::Nothing => Ok(Value::Atom(Rc::from("none"))),
                        Value::TaggedValue { tag, payload } if tag.as_ref() == "some" => {
                            let inner = force_tagged_slot(&payload, self)?;
                            match inner {
                                Value::Record(inner_fields) => {
                                    let projected = project_inner_field(&inner_fields)?;
                                    Ok(tagged_slot_value("some", projected))
                                }
                                other => Err(EvalError::TypeMismatch {
                                    expected: "Record",
                                    found: value_type_name(&other),
                                }),
                            }
                        }
                        other => Err(EvalError::TypeMismatch {
                            expected: "Optional",
                            found: value_type_name(&other),
                        }),
                    },
                    RuntimeWrapperKind::Maybe => match rv {
                        Value::Atom(atom) if atom.as_ref() == "absent" => Ok(Value::Atom(Rc::from("absent"))),
                        Value::TaggedValue { tag, .. } if tag.as_ref() == "absent" => {
                            Ok(Value::Atom(Rc::from("absent")))
                        }
                        Value::Nothing => Ok(Value::Atom(Rc::from("absent"))),
                        Value::TaggedValue { tag, payload } if tag.as_ref() == "present" => {
                            let inner = force_tagged_slot(&payload, self)?;
                            match inner {
                                Value::Record(inner_fields) => {
                                    let projected = project_inner_field(&inner_fields)?;
                                    Ok(tagged_slot_value("present", projected))
                                }
                                other => Err(EvalError::TypeMismatch {
                                    expected: "Record",
                                    found: value_type_name(&other),
                                }),
                            }
                        }
                        other => Err(EvalError::TypeMismatch {
                            expected: "Maybe",
                            found: value_type_name(&other),
                        }),
                    },
                }
            }
            ThirExprKind::Sequence(items) => {
                let mut value = Value::Nothing;
                for &item in items {
                    value = self.eval(item, env)?;
                }
                Ok(value)
            }
            ThirExprKind::Perform { .. }
            | ThirExprKind::Handle { .. }
            | ThirExprKind::Resume { .. } => Err(EvalError::EffectfulNotExecutable(
                "algebraic effects execute through the TLC evaluator; the legacy THIR evaluator remains pure-only"
                    .to_string(),
            )),
            ThirExprKind::Error => Err(EvalError::Internal(
                "Error node reached evaluator (unreachable past gate)",
            )),
        }
    }

    // ── binary operator dispatch ──────────────────────────────────────────────

    fn apply_builtin_expr(
        &self,
        func: BuiltinFn,
        mut args: Vec<Thunk>,
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

    fn eval_builtin(&self, func: BuiltinFn, args: &[Thunk]) -> Result<Value, EvalError> {
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
                let aliases = self.build_alias_map();
                let key = type_key(&self.file.type_arena, &aliases, self.expr(lhs).ty);
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
                let aliases = self.build_alias_map();
                let key = type_key(&self.file.type_arena, &aliases, self.expr(lhs).ty);
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

fn tagged_slot_thunk(tag: &'static str, thunk: Thunk) -> Value {
    Value::TaggedValue {
        tag: Rc::from(tag),
        payload: Rc::new(vec![(Rc::from("0"), thunk)]),
    }
}

fn tagged_slot_value(tag: &'static str, value: Value) -> Value {
    tagged_slot_thunk(tag, Thunk::ready(value))
}

fn force_tagged_slot(
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
