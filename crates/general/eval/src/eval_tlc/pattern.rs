use super::*;

impl<'a> TlcEvaluator<'a> {
    /// Try to match `pat` against `val`, inserting bindings into `env`.
    /// Returns `true` on a successful match.
    pub(super) fn match_pattern(
        &self,
        pat: &TlcPat,
        val: &Value,
        env: &Env,
    ) -> Result<bool, EvalError> {
        match pat {
            TlcPat::Wildcard => Ok(true),
            TlcPat::Bind(b) => {
                env.insert(*b, Thunk::ready(val.clone()));
                Ok(true)
            }
            TlcPat::Lit(lit) => Ok(lit_matches(lit, val)),
            TlcPat::Atom(s) => Ok(matches!(val, Value::Atom(a) if a.as_ref() == s.as_str())),
            TlcPat::Tuple(items) => {
                if let Value::Tuple(fields) = val {
                    if items.len() != fields.len() {
                        return Ok(false);
                    }
                    for (item, field) in items.iter().zip(fields.iter()) {
                        let fv = field.value.force_tlc(self)?;
                        let sub_pat = match item {
                            TlcPatItem::Positional(p) => p,
                            TlcPatItem::Named { pat, .. } => pat,
                        };
                        if !self.match_pattern(sub_pat, &fv, env)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            TlcPat::ListNil => Ok(matches!(val, Value::List(items) if items.is_empty())),
            TlcPat::ListCons(head, tail) => {
                let Value::List(items) = val else {
                    return Ok(false);
                };
                let Some(first) = items.first() else {
                    return Ok(false);
                };
                let head_val = first.force_tlc(self)?;
                if !self.match_pattern(head, &head_val, env)? {
                    return Ok(false);
                }
                let tail_val =
                    Value::List(items.iter().skip(1).cloned().collect::<Vec<_>>().into());
                self.match_pattern(tail, &tail_val, env)
            }
            TlcPat::Record(field_pats) => {
                if let Value::Record(record_fields) = val {
                    for (name, sub_pat) in field_pats {
                        let found = record_fields
                            .iter()
                            .find(|(n, _)| n.as_ref() == name.as_str());
                        match found {
                            Some((_, thunk)) => {
                                let fv = thunk.force_tlc(self)?;
                                if !self.match_pattern(sub_pat, &fv, env)? {
                                    return Ok(false);
                                }
                            }
                            None => return Ok(false),
                        }
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            TlcPat::Variant(tag, inner_pat) => {
                if let Value::TaggedValue {
                    tag: val_tag,
                    payload,
                } = val
                {
                    if val_tag.as_ref() != tag.as_str() {
                        return Ok(false);
                    }
                    if let TlcPat::Tuple(items) = inner_pat.as_ref()
                        && items.len() == 1
                        && payload.len() == 1
                    {
                        let thunk = payload[0].1.clone();
                        let item = match &items[0] {
                            TlcPatItem::Positional(pattern)
                            | TlcPatItem::Named { pat: pattern, .. } => pattern,
                        };
                        match item {
                            TlcPat::Wildcard => Ok(true),
                            TlcPat::Bind(binding) => {
                                env.insert(*binding, thunk);
                                Ok(true)
                            }
                            _ => {
                                let value = thunk.force_tlc(self)?;
                                self.match_pattern(item, &value, env)
                            }
                        }
                    } else {
                        // Match record-style payload patterns against the field envelope.
                        let payload_val = Value::Record(Rc::clone(payload));
                        self.match_pattern(inner_pat, &payload_val, env)
                    }
                } else if let Value::Atom(a) = val {
                    // Bare atom variant — no payload; inner must be Wildcard.
                    Ok(
                        a.as_ref() == tag.as_str()
                            && matches!(inner_pat.as_ref(), TlcPat::Wildcard),
                    )
                } else {
                    Ok(false)
                }
            }
        }
    }
}
