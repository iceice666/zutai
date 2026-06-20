use super::*;

impl<'a> Evaluator<'a> {
    // ── clause / pattern matching ─────────────────────────────────────────────

    /// Try all clauses of `closure` with the given argument thunks.
    ///
    /// Switches to the closure's home module before evaluating clause bodies
    /// and guards so arena look-ups (`expr_arena`, `pat_arena`) hit the file
    /// where the clause was originally lowered.
    pub(super) fn apply_closure(
        &self,
        closure: &Closure,
        args: Vec<Thunk>,
    ) -> Result<Value, EvalError> {
        let home_ev = self.for_module(closure.home);
        for clause in closure.clauses.iter() {
            let mut child = closure.env.push_frame();
            if home_ev.match_all_patterns(&clause.patterns, &args, &mut child)? {
                // Check guard (if any) in the home module.
                if let Some(guard_id) = clause.guard {
                    match home_ev.eval(guard_id, &child)? {
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
                return home_ev.eval(clause.body, &child);
            }
        }
        Err(EvalError::NoMatchingClause)
    }

    /// Match a sequence of patterns against a sequence of argument thunks.
    /// Returns `true` if all match and `child` is populated with bindings.
    pub(super) fn match_all_patterns(
        &self,
        pattern_ids: &[ThirPatId],
        args: &[Thunk],
        child: &mut Env,
    ) -> Result<bool, EvalError> {
        debug_assert_eq!(pattern_ids.len(), args.len());
        for (&pat_id, thunk) in pattern_ids.iter().zip(args.iter()) {
            if !self.match_pattern(pat_id, thunk.clone(), child)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Match a single pattern against a thunk.  Populates bindings into
    /// `child_env` and returns whether the match succeeded.
    pub(super) fn match_pattern(
        &self,
        pat_id: ThirPatId,
        thunk: Thunk,
        child_env: &mut Env,
    ) -> Result<bool, EvalError> {
        let pat = self.pat(pat_id);
        match &pat.kind {
            ThirPatKind::Error => Err(EvalError::Internal(
                "Error pattern reached evaluator (unreachable past gate)",
            )),
            ThirPatKind::Wildcard => Ok(true),
            ThirPatKind::Bind(b) => {
                // Insert the thunk UNFORCED — lazy binding.
                child_env.insert(*b, thunk);
                Ok(true)
            }
            ThirPatKind::True => match thunk.force(self)? {
                Value::Bool(true) => Ok(true),
                _ => Ok(false),
            },
            ThirPatKind::False => match thunk.force(self)? {
                Value::Bool(false) => Ok(true),
                _ => Ok(false),
            },
            ThirPatKind::Integer(n) => match thunk.force(self)? {
                Value::Int(v) => Ok(v == *n),
                _ => Ok(false),
            },
            ThirPatKind::Float(f) => match thunk.force(self)? {
                Value::Float(v) => Ok(v == *f),
                _ => Ok(false),
            },
            ThirPatKind::String(s) => match thunk.force(self)? {
                Value::Text(v) => Ok(v.as_ref() == s.as_str()),
                _ => Ok(false),
            },
            ThirPatKind::Atom(a) => match thunk.force(self)? {
                Value::Atom(v) => Ok(v.as_ref() == a.as_str()),
                _ => Ok(false),
            },
            ThirPatKind::Tuple(items) => {
                let v = thunk.force(self)?;
                match v {
                    Value::Tuple(fields) => {
                        if fields.len() != items.len() {
                            return Ok(false);
                        }
                        // Clone the pattern items so we can iterate with access
                        // to child_env without borrowing issues.
                        let items_owned: Vec<_> = items.clone();
                        for (item, field) in items_owned.iter().zip(fields.iter()) {
                            match item {
                                ThirTuplePatItem::Named { name, pattern, .. } => {
                                    if field.name.as_deref() != Some(name.as_str()) {
                                        return Ok(false);
                                    }
                                    if !self.match_pattern(
                                        *pattern,
                                        field.value.clone(),
                                        child_env,
                                    )? {
                                        return Ok(false);
                                    }
                                }
                                ThirTuplePatItem::Positional(p) => {
                                    if field.name.is_some() {
                                        return Ok(false);
                                    }
                                    if !self.match_pattern(*p, field.value.clone(), child_env)? {
                                        return Ok(false);
                                    }
                                }
                            }
                        }
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            ThirPatKind::Record(pat_fields) => {
                let v = thunk.force(self)?;
                match v {
                    Value::Record(rec_fields) => {
                        let pat_fields_owned: Vec<_> = pat_fields.clone();
                        for pf in &pat_fields_owned {
                            // Find the field in the record by name.
                            let maybe_thunk = rec_fields
                                .iter()
                                .find(|(n, _)| n.as_ref() == pf.name.as_str())
                                .map(|(_, t)| t.clone());
                            match maybe_thunk {
                                None => return Ok(false),
                                Some(t) => {
                                    if !self.match_pattern(pf.pattern, t, child_env)? {
                                        return Ok(false);
                                    }
                                }
                            }
                        }
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            ThirPatKind::TaggedValue {
                tag: pat_tag,
                payload: pat_fields,
            } => {
                let v = thunk.force(self)?;
                match v {
                    Value::TaggedValue {
                        tag: val_tag,
                        payload,
                    } => {
                        if val_tag.as_ref() != pat_tag.as_str() {
                            return Ok(false);
                        }
                        let pat_fields_owned: Vec<_> = pat_fields.clone();
                        for pf in &pat_fields_owned {
                            let maybe_thunk = payload
                                .iter()
                                .find(|(n, _)| n.as_ref() == pf.name.as_str())
                                .map(|(_, t)| t.clone());
                            match maybe_thunk {
                                None => return Ok(false),
                                Some(t) => {
                                    if !self.match_pattern(pf.pattern, t, child_env)? {
                                        return Ok(false);
                                    }
                                }
                            }
                        }
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
        }
    }
}
