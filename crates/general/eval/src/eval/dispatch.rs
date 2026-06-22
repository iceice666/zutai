use super::*;

impl<'a> Evaluator<'a> {
    // ── constraint method dispatch ────────────────────────────────────────────

    /// Try to dispatch a constraint method call to a matching witness field.
    ///
    /// Returns `Ok(Some(value))` when `func` is a `BindingRef` to a named
    /// constraint method AND a witness with a matching target type is in scope.
    /// Returns `Ok(None)` in all other cases (not a method, no instantiation,
    /// no concrete witness found) — the caller then falls through to normal `eval`.
    /// Returns `Err(EvalError::UnresolvedWitness)` when the type key is ambiguous
    /// (TypeVar inside a bounded function body) AND no WitnessDict is visible in
    /// the current env — this covers the indirect-call case where the witness
    /// was injected into the caller's frame but is not an ancestor of the callee's
    /// captured env.
    pub(super) fn try_method_dispatch(
        &self,
        func: ThirExprId,
        instantiation: &[TypeId],
        env: &Env,
    ) -> Result<Option<Value>, EvalError> {
        // Step 1: func must be a BindingRef to a constraint method.
        let method_binding = match &self.expr(func).kind {
            ThirExprKind::BindingRef(b) => *b,
            _ => return Ok(None),
        };

        // Step 2: find which constraint owns this method binding.
        let mut found = None;
        'outer: for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::Constraint { methods, .. } = &decl.kind {
                for m in methods {
                    if m.binding == Some(method_binding) {
                        found = Some((decl.binding, m.name.clone()));
                        break 'outer;
                    }
                }
            }
        }
        let (constraint_binding, method_name) = match found {
            Some(x) => x,
            None => return Ok(None),
        };

        // Step 3: guard — only dispatch when exactly one type var was instantiated.
        if instantiation.len() != 1 {
            return Ok(None);
        }
        let aliases = self.build_alias_map();
        let key = type_key(&self.file.type_arena, &aliases, instantiation[0]);

        // Step 4: dispatch based on key ambiguity.
        //
        // **Concrete key** (not starting with `@`, `?`, `$`):
        //   - Scan witnesses for a matching (constraint_binding, target_key).
        //   - If a matching witness is found and contains the field → return it.
        //   - If a matching witness is found but omits the field AND the method
        //     has a default body → return the default closure (valid omission).
        //   - If no matching witness exists at all → return Ok(None) so the
        //     caller falls through to normal eval (UnboundBinding).
        //
        // **Ambiguous key** (TypeVar / InferVar / AliasApply, starts with `@`, `?`, `$`):
        //   - Try env fallback: look up the constraint binding; if it holds a
        //     WitnessDict injected at the direct call site, dispatch from it.
        //   - If env fallback misses → return EvalError::UnresolvedWitness.
        //   - NEVER fall through to the default-body fallback for ambiguous keys:
        //     the failure here is "can't resolve which witness to use", not
        //     "witness omitted an optional method".

        let key_is_concrete =
            !key.starts_with('@') && !key.starts_with('?') && !key.starts_with('$');

        if key_is_concrete {
            // Scan all witnesses for one matching this constraint + type key.
            let mut found_witness = false;
            for &decl_id in &self.file.decls {
                let decl = self.decl(decl_id);
                if let ThirDeclKind::Witness {
                    constraint: Some(c),
                    target,
                    fields,
                    ..
                } = &decl.kind
                    && *c == constraint_binding
                    && type_key(&self.file.type_arena, &aliases, *target) == key
                {
                    found_witness = true;
                    for field in fields {
                        if field.name == method_name {
                            return Ok(Some(self.eval(field.value, env)?));
                        }
                    }
                    // Matching witness found but field absent — fall through to
                    // default-body check below (only reachable for concrete keys).
                    break;
                }
            }

            let constraint_name = self.binding_name(constraint_binding);
            if let Some(value) =
                self.eval_imported_witness_field(constraint_name, &method_name, &key)?
            {
                return Ok(Some(value));
            }

            if found_witness {
                // Matching witness exists but omitted this method — check for a
                // default body in the constraint declaration.
                for &decl_id in &self.file.decls {
                    let decl = self.decl(decl_id);
                    if let ThirDeclKind::Constraint { methods, .. } = &decl.kind
                        && decl.binding == constraint_binding
                    {
                        for m in methods {
                            if m.name == method_name
                                && let Some(clauses) = &m.default
                            {
                                let arity = clauses.first().map(|c| c.patterns.len()).unwrap_or(0);
                                return Ok(Some(Value::Closure(Rc::new(Closure {
                                    binding: m.binding,
                                    arity,
                                    clauses: clauses.as_slice().into(),
                                    env: env.clone(),
                                    applied: SmallVec::new(),
                                    home: self.active_module,
                                }))));
                            }
                        }
                    }
                }
            }

            // No matching witness at all (or witness had no field and no default).
            return Ok(None);
        }

        // Ambiguous key: TypeVar/InferVar/AliasApply inside a bounded function body.
        //
        // Env fallback: for direct top-level calls, the dict was injected into
        // an ancestor frame. For indirect calls, lookup returns Err — we fall
        // through to UnresolvedWitness rather than a wrong-answer default.
        if let Ok(thunk) = env.lookup(constraint_binding)
            && let Value::WitnessDict(dict) = thunk.force(self)?
            && let Some(v) = dict.get(method_name.as_str())
        {
            return Ok(Some(v.clone()));
        }

        // Env fallback failed — dict is not visible (indirect call case).
        // Return a clean refusal instead of silently using the default body.
        Err(EvalError::UnresolvedWitness {
            method: method_name,
        })
    }

    // ── operator-method dispatch ──────────────────────────────────────────────

    /// Dispatch a comparison operator to a user-defined witness field.
    ///
    /// Returns `Ok(Some(v))` when a matching `(op)` field is found and applied.
    /// Returns `Ok(None)` when no witness matches — caller falls through to builtin.
    pub(super) fn try_operator_dispatch(
        &self,
        op: BinOp,
        operand_ty: TypeId,
        lv: &Value,
        rv: &Value,
        env: &Env,
    ) -> Result<Option<Value>, EvalError> {
        let op_name = match op {
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            _ => return Ok(None),
        };
        let aliases = self.build_alias_map();
        let key = type_key(&self.file.type_arena, &aliases, operand_ty);

        if key_is_ambiguous(&key) {
            if op == BinOp::Ne {
                let ne_err = match self.dispatch_operator_dict_field("!=", lv, rv, env) {
                    Ok(Some(v)) => return Ok(Some(v)),
                    Ok(None) => None,
                    Err(err) => Some(err),
                };
                match self.dispatch_operator_dict_field("==", lv, rv, env) {
                    Ok(Some(v)) => {
                        return Ok(match v {
                            Value::Bool(b) => Some(Value::Bool(!b)),
                            _ => None,
                        });
                    }
                    Ok(None) => {}
                    Err(err) => return Err(err),
                }
                if let Some(err) = ne_err {
                    return Err(err);
                }
                return Ok(None);
            }

            return self.dispatch_operator_dict_field(op_name, lv, rv, env);
        }

        if op == BinOp::Ne {
            // Try (!=) first; fall back to negating (==).
            if let Some(v) = self.dispatch_operator_field("!=", &key, &aliases, lv, rv, env)? {
                return Ok(Some(v));
            }
            if let Some(v) = self.dispatch_operator_field("==", &key, &aliases, lv, rv, env)? {
                return Ok(match v {
                    Value::Bool(b) => Some(Value::Bool(!b)),
                    _ => None,
                });
            }
            return Ok(None);
        }

        self.dispatch_operator_field(op_name, &key, &aliases, lv, rv, env)
    }

    /// Find a witness field matching `op_name` for type key `key` and apply it.
    pub(super) fn dispatch_operator_field(
        &self,
        op_name: &str,
        key: &str,
        aliases: &FxHashMap<BindingId, (Vec<BindingId>, TypeId)>,
        lv: &Value,
        rv: &Value,
        env: &Env,
    ) -> Result<Option<Value>, EvalError> {
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::Witness { target, fields, .. } = &decl.kind
                && type_key(&self.file.type_arena, aliases, *target) == key
            {
                for field in fields {
                    if field.is_operator && field.name == op_name {
                        let fv = self.eval(field.value, env)?;
                        match fv {
                            Value::Closure(c) => {
                                let args =
                                    smallvec![Thunk::ready(lv.clone()), Thunk::ready(rv.clone())];
                                return Ok(Some(self.apply_closure(&c, args)?));
                            }
                            _ => return Ok(None),
                        }
                    }
                }
            }
        }
        if let Some(fv) = self.eval_imported_witness_field("", op_name, key)? {
            match fv {
                Value::Closure(c) => {
                    let args = smallvec![Thunk::ready(lv.clone()), Thunk::ready(rv.clone())];
                    return Ok(Some(self.apply_closure(&c, args)?));
                }
                _ => return Ok(None),
            }
        }

        Ok(None)
    }

    /// Find an operator constraint method's active dictionary field and apply it.
    pub(super) fn dispatch_operator_dict_field(
        &self,
        op_name: &str,
        lv: &Value,
        rv: &Value,
        env: &Env,
    ) -> Result<Option<Value>, EvalError> {
        let mut found_operator_constraint = false;

        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::Constraint { methods, .. } = &decl.kind {
                let has_operator_method = methods
                    .iter()
                    .any(|method| method.is_operator && method.name == op_name);
                if !has_operator_method {
                    continue;
                }

                found_operator_constraint = true;
                if let Ok(thunk) = env.lookup(decl.binding)
                    && let Value::WitnessDict(dict) = thunk.force(self)?
                    && let Some(Value::Closure(c)) = dict.get(op_name)
                {
                    let args = smallvec![Thunk::ready(lv.clone()), Thunk::ready(rv.clone())];
                    return Ok(Some(self.apply_closure(c, args)?));
                }
            }
        }

        if found_operator_constraint {
            Err(EvalError::UnresolvedWitness {
                method: op_name.to_string(),
            })
        } else {
            Ok(None)
        }
    }

    pub(super) fn eval_imported_witness_field(
        &self,
        constraint_name: &str,
        field_name: &str,
        target_key: &str,
    ) -> Result<Option<Value>, EvalError> {
        for witness in self.witnesses {
            if witness.module == self.active_module || witness.target_key != target_key {
                continue;
            }
            if !constraint_name.is_empty() && witness.constraint != constraint_name {
                continue;
            }
            let ev = self.for_module(witness.module);
            let top = ev.build_top_env();
            let aliases = ev.build_alias_map();
            for &decl_id in &ev.file.decls {
                let decl = ev.decl(decl_id);
                if let ThirDeclKind::Witness {
                    constraint: Some(c),
                    target,
                    fields,
                    ..
                } = &decl.kind
                    && ev.binding_name(*c) == witness.constraint
                    && type_key(&ev.file.type_arena, &aliases, *target) == witness.target_key
                {
                    for field in fields {
                        if field.name == field_name {
                            return Ok(Some(ev.eval(field.value, &top)?));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    pub(super) fn binding_name(&self, binding: BindingId) -> &str {
        self.file
            .binding_names
            .get(binding.0 as usize)
            .map_or("", String::as_str)
    }

    /// Build a map from alias `BindingId` to its underlying `TypeId` for
    /// alias-resolved `type_key` calls.
    pub(super) fn build_alias_map(&self) -> Rc<AliasMap> {
        if let Some(m) = self
            .caches
            .alias_maps
            .borrow()
            .get(&self.active_module)
            .cloned()
        {
            return m;
        }
        let mut m: AliasMap = FxHashMap::default();
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::TypeAlias { params, ty } = &decl.kind {
                m.insert(decl.binding, (params.clone(), *ty));
            }
        }
        let rc = Rc::new(m);
        self.caches
            .alias_maps
            .borrow_mut()
            .insert(self.active_module, rc.clone());
        rc
    }

    /// Structural type key for `ty`, memoized per `(active_module, ty)`.
    ///
    /// Use only at dispatch sites that need the key alone; sites that also reuse
    /// the alias map keep calling `build_alias_map` (now O(1) cached).
    pub(super) fn cached_type_key(&self, ty: TypeId) -> Rc<str> {
        let ck = (self.active_module, ty);
        if let Some(k) = self.caches.type_keys.borrow().get(&ck).cloned() {
            return k;
        }
        let aliases = self.build_alias_map();
        let key: Rc<str> = Rc::from(type_key(&self.file.type_arena, &aliases, ty).as_str());
        self.caches.type_keys.borrow_mut().insert(ck, key.clone());
        key
    }

    /// Return `true` if the file has any witness with a `(==)` or `(!=)` operator field.
    pub(super) fn has_eq_operator_witness(&self) -> bool {
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            if let ThirDeclKind::Witness { fields, .. } = &decl.kind {
                for f in fields {
                    if f.is_operator && (f.name == "==" || f.name == "!=") {
                        return true;
                    }
                }
            }
        }
        false
    }
}
