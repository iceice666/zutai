use rustc_hash::FxHashSet;

use zutai_tlc::{BuiltinOp, PrimTy, TlcExpr, TlcExprId, TlcTupleItem, TlcType};

use crate::{
    DfArm, DfBuiltinOp, DfListPrimOp, DfLit, DfNodeKind, DfNumPrimOp, DfPattern, DfTextPrimOp,
    DfTupleNodeItem, NodeId,
};

use super::*;

impl<'m> Lowerer<'m> {
    // ── Expression lowering ───────────────────────────────────────────────────

    pub(super) fn lower_expr(&mut self, id: TlcExprId) -> NodeId {
        let expr = self.module.expr_arena[id].clone();
        let span = self.module.spans.get(&id).copied();
        let df_ty = match self.module.expr_types.get(&id).copied() {
            Some(t) => self.lower_type(t),
            None => self.error_ty,
        };

        match expr {
            TlcExpr::Var(binding) => {
                // Local env first — this is where sharing happens.
                if let Some(&node_id) = self.local_env.get(&binding) {
                    return node_id;
                }
                // Global reference.
                if let Some(name) = self.global_names.get(&binding).cloned() {
                    return self.alloc_node(DfNodeKind::GlobalRef(name), df_ty, span);
                }
                // Extern witness global (virtual BindingId allocated by the TLC lowerer).
                if let Some(name) = self.module.extern_global_bindings.get(&binding).cloned() {
                    return self.alloc_node(DfNodeKind::GlobalRef(name), df_ty, span);
                }
                self.alloc_node(DfNodeKind::Error, self.error_ty, span)
            }

            TlcExpr::Lit(lit) => match lower_lit(&lit) {
                Some(df_lit) => self.alloc_node(DfNodeKind::Lit(df_lit), df_ty, span),
                None => self.alloc_node(DfNodeKind::Error, self.error_ty, span),
            },

            TlcExpr::Lam(param_binding, param_ty, body) => {
                let param_df_ty = self.lower_type(param_ty);
                let bind_node = self.alloc_node(DfNodeKind::Bind, param_df_ty, None);
                self.local_env.insert(param_binding, bind_node);
                let body_node = self.lower_expr(body);
                self.local_env.remove(&param_binding);
                self.alloc_node(
                    DfNodeKind::Lambda {
                        param: bind_node,
                        body: body_node,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::App(func, arg) => {
                // Saturated calls to list/numeric/text bridge builtins lower to
                // dedicated primitive nodes rather than closure `Apply`, since
                // builtin values have no closure object to call.
                if let Some(node) = self.try_lower_list_bridge(id, df_ty, span) {
                    return node;
                }
                if let Some(node) = self.try_lower_num_prim(id, df_ty, span) {
                    return node;
                }
                if let Some(node) = self.try_lower_text_prim(id, df_ty, span) {
                    return node;
                }
                let func_node = self.lower_expr(func);
                let arg_node = self.lower_expr(arg);
                self.alloc_node(
                    DfNodeKind::Apply {
                        func: func_node,
                        arg: arg_node,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::TyLam(tyvar, _kind, body) => {
                let body_node = self.lower_expr(body);
                self.alloc_node(
                    DfNodeKind::TyLam {
                        ty_params: vec![lower_tyvar(tyvar)],
                        body: body_node,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::TyApp(expr, ty_arg) => {
                let poly_node = self.lower_expr(expr);
                let df_ty_arg = self.lower_type(ty_arg);
                self.alloc_node(
                    DfNodeKind::TyApp {
                        poly: poly_node,
                        ty_args: vec![df_ty_arg],
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::Let {
                binding,
                ty: _,
                value,
                body,
            } => {
                // Tree-to-graph: lower value once, register its NodeId.
                // All references to `binding` in `body` will reuse this NodeId directly.
                let value_node = self.lower_expr(value);
                self.local_env.insert(binding, value_node);
                let body_node = self.lower_expr(body);
                self.local_env.remove(&binding);
                // The `let` itself disappears; graph edges carry the sharing.
                body_node
            }

            TlcExpr::Letrec { bindings, body } => {
                // TlcExpr::Letrec is defined in the IR but never generated by the v0 TLC
                // lowerer. Lower defensively without mutual visibility: each binding's value
                // is lowered before the next binding's name is in scope (not true letrec
                // semantics). This is acceptable because this code path is unreachable in
                // well-formed v0 programs; the ANF phase handles recursion via GlobalRef SCC
                // analysis on globals, not via local letrec.
                for (binding, _, value_id) in &bindings {
                    let value_node = self.lower_expr(*value_id);
                    self.local_env.insert(*binding, value_node);
                }
                let body_node = self.lower_expr(body);
                for (binding, _, _) in &bindings {
                    self.local_env.remove(binding);
                }
                body_node
            }

            TlcExpr::Case(scrutinee, alts) => {
                // Resolve the scrutinee's type so arm-bound variables get the right Bind type.
                let scrutinee_df_ty = self
                    .module
                    .expr_types
                    .get(&scrutinee)
                    .copied()
                    .map(|t| self.lower_type(t))
                    .unwrap_or(self.error_ty);
                let scrutinee_node = self.lower_expr(scrutinee);
                let arms: Vec<DfArm> = alts
                    .iter()
                    .map(|alt| self.lower_alt(alt, scrutinee_df_ty))
                    .collect();
                self.alloc_node(
                    DfNodeKind::Match {
                        scrutinee: scrutinee_node,
                        arms,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::Record(fields) => {
                let df_fields: Vec<(String, NodeId)> = fields
                    .iter()
                    .map(|(name, expr_id)| (name.clone(), self.lower_expr(*expr_id)))
                    .collect();
                let df_fields = if let Some(tlc_ty) = self.module.expr_types.get(&id).copied() {
                    self.record_storage_fields_for_tlc_type(tlc_ty, df_ty, df_fields, span)
                } else {
                    self.record_storage_fields(df_ty, df_fields, span)
                };
                self.alloc_node(DfNodeKind::Record(df_fields), df_ty, span)
            }

            TlcExpr::RecordUpdate { receiver, fields } => {
                let base = self.lower_expr(receiver);
                let result_ty = self.module.expr_types.get(&id).copied();
                let updates: Vec<(String, usize, NodeId)> = fields
                    .iter()
                    .map(|(name, expr_id)| {
                        let raw_value = self.lower_expr(*expr_id);
                        let value = self.record_storage_update_value(
                            df_ty,
                            name,
                            raw_value,
                            self.module.spans.get(expr_id).copied().or(span),
                        );
                        let slot = result_ty
                            .and_then(|ty| {
                                self.record_slot_for_tlc_type(ty, name, &mut FxHashSet::default())
                            })
                            .or_else(|| self.record_slot_for_df_ty(df_ty, name))
                            .unwrap_or(0);
                        (name.clone(), slot, value)
                    })
                    .collect();
                self.alloc_node(DfNodeKind::RecordUpdate { base, updates }, df_ty, span)
            }

            TlcExpr::GetField(expr, field) => {
                let slot = self
                    .module
                    .dict_field_slots
                    .get(&id)
                    .copied()
                    .or_else(|| self.record_slot_for_expr_type(expr, &field))
                    .unwrap_or(0);
                let base_node = self.lower_expr(expr);
                self.alloc_node(
                    DfNodeKind::Select {
                        base: base_node,
                        field,
                        slot,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::Tuple(items) => {
                let df_items: Vec<DfTupleNodeItem> = items
                    .iter()
                    .map(|item| match item {
                        TlcTupleItem::Named { name, value } => DfTupleNodeItem::Named {
                            name: name.clone(),
                            value: self.lower_expr(*value),
                        },
                        TlcTupleItem::Positional(v) => {
                            DfTupleNodeItem::Positional(self.lower_expr(*v))
                        }
                    })
                    .collect();
                self.alloc_node(DfNodeKind::Tuple(df_items), df_ty, span)
            }

            TlcExpr::List(items) => {
                let df_items: Vec<NodeId> = items.iter().map(|&e| self.lower_expr(e)).collect();
                self.alloc_node(DfNodeKind::List(df_items), df_ty, span)
            }

            TlcExpr::Builtin(op, lhs, rhs) => {
                if let Some(node) = self.lower_bool_short_circuit(op, lhs, rhs, df_ty, span) {
                    return node;
                }
                if let Some(node) = self.lower_coalesce(op, lhs, rhs, df_ty, span) {
                    return node;
                }

                let lhs_node = self.lower_expr(lhs);
                let rhs_node = self.lower_expr(rhs);
                if let Some(float_op) = self.lower_float_builtin_op(op, lhs) {
                    self.alloc_node(
                        DfNodeKind::NumPrim {
                            op: float_op,
                            args: vec![lhs_node, rhs_node],
                        },
                        df_ty,
                        span,
                    )
                } else if let Some(text_op) = self.lower_text_builtin_op(op, lhs) {
                    self.alloc_node(
                        DfNodeKind::TextPrim {
                            op: text_op,
                            args: vec![lhs_node, rhs_node],
                        },
                        df_ty,
                        span,
                    )
                } else {
                    let df_op = self
                        .lower_posit_builtin_op(op, lhs)
                        .unwrap_or_else(|| lower_builtin_op(op));
                    self.alloc_node(DfNodeKind::Builtin(df_op, lhs_node, rhs_node), df_ty, span)
                }
            }

            TlcExpr::Variant(tag, payload) => {
                let payload_node = self.lower_expr(payload);
                let tag_index = self.variant_tag_index_for_df_ty(df_ty, &tag);
                self.alloc_node(
                    DfNodeKind::Variant {
                        tag,
                        tag_index,
                        value: payload_node,
                    },
                    df_ty,
                    span,
                )
            }

            TlcExpr::Import(source) => {
                // Imports resolved by the front end lower natively: `.zti` data
                // becomes an inline Dataflow Core constant; a `.zt` module becomes a
                // reference to that dependency's merged module-value global. Any
                // unresolved import falls through to an `Import` leaf — the lowering
                // gate has already rejected such programs.
                match self.current_imports.get(&source).copied() {
                    Some(ImportTarget::Zti(value)) => self.lower_immediate(value, df_ty),
                    Some(ImportTarget::Zt(idx)) => {
                        let name = dep_value_global(idx);
                        self.alloc_node(DfNodeKind::GlobalRef(name), df_ty, span)
                    }
                    None => {
                        let (path, kind) = lower_import_source(&source);
                        self.alloc_node(DfNodeKind::Import { path, kind }, df_ty, span)
                    }
                }
            }

            TlcExpr::Sequence(items) => {
                let nodes = items
                    .into_iter()
                    .map(|item| self.lower_expr(item))
                    .collect::<Vec<_>>();
                if nodes.is_empty() {
                    self.alloc_node(DfNodeKind::Error, self.error_ty, span)
                } else {
                    self.alloc_node(DfNodeKind::Sequence(nodes), df_ty, span)
                }
            }

            TlcExpr::Perform { op, arg } => match zutai_tlc::HostOp::from_name(&op) {
                Some(zutai_tlc::HostOp::IoPrint) => {
                    let arg = self.lower_expr(arg);
                    self.alloc_node(DfNodeKind::HostPrint { arg }, df_ty, span)
                }
                Some(op) => {
                    let arg = self.lower_expr(arg);
                    self.alloc_node(DfNodeKind::HostOp { op, arg }, df_ty, span)
                }
                None => self.alloc_node(DfNodeKind::Error, self.error_ty, span),
            },

            TlcExpr::Handle { .. } | TlcExpr::Resume { .. } => {
                self.alloc_node(DfNodeKind::Error, self.error_ty, span)
            }
        }
    }

    /// Lower a saturated application of a list-bridge builtin (`listEmpty`,
    /// `listCons`, `listIsNil`, `listHead`, `listTail`) to a primitive node.
    /// Returns `None` for any other callee or an under-saturated call, so the
    /// caller falls back to the ordinary closure `Apply` path.
    fn try_lower_list_bridge(
        &mut self,
        id: TlcExprId,
        df_ty: crate::DfTyId,
        span: Option<Span>,
    ) -> Option<NodeId> {
        // Peel the App/TyApp spine to the head Var, collecting value-arg ids.
        let mut args_rev: Vec<TlcExprId> = Vec::new();
        let mut cur = id;
        let name = loop {
            match &self.module.expr_arena[cur] {
                TlcExpr::App(func, arg) => {
                    args_rev.push(*arg);
                    cur = *func;
                }
                TlcExpr::TyApp(inner, _) => cur = *inner,
                TlcExpr::Var(binding) => break self.list_bridge_builtin_name(*binding)?,
                _ => return None,
            }
        };
        args_rev.reverse();
        let args = args_rev;

        let op = match (name, args.len()) {
            // `listEmpty ()` is just an empty list literal of the result type; the
            // unit argument carries no runtime value, so it is discarded.
            ("listEmpty", 1) => {
                return Some(self.alloc_node(DfNodeKind::List(Vec::new()), df_ty, span));
            }
            ("listCons", 2) => DfListPrimOp::Cons,
            ("listIsNil", 1) => DfListPrimOp::IsNil,
            ("listHead", 1) => DfListPrimOp::Head,
            ("listTail", 1) => DfListPrimOp::Tail,
            ("listFoldlStrict", 3) => DfListPrimOp::FoldlStrict,
            // Under-saturated (or unknown arity) — fall back to closure Apply.
            _ => return None,
        };
        let arg_nodes: Vec<NodeId> = args.iter().map(|&a| self.lower_expr(a)).collect();
        Some(self.alloc_node(
            DfNodeKind::ListPrim {
                op,
                args: arg_nodes,
            },
            df_ty,
            span,
        ))
    }

    /// Lower a saturated application of a numeric bridge builtin to a primitive
    /// node. Returns `None` for any other callee or an under-saturated call.
    fn try_lower_num_prim(
        &mut self,
        id: TlcExprId,
        df_ty: crate::DfTyId,
        span: Option<Span>,
    ) -> Option<NodeId> {
        let mut args_rev: Vec<TlcExprId> = Vec::new();
        let mut cur = id;
        let name = loop {
            match &self.module.expr_arena[cur] {
                TlcExpr::App(func, arg) => {
                    args_rev.push(*arg);
                    cur = *func;
                }
                TlcExpr::TyApp(inner, _) => cur = *inner,
                TlcExpr::Var(binding) => break self.num_prim_builtin_name(*binding)?,
                _ => return None,
            }
        };
        args_rev.reverse();
        let args = args_rev;

        let op = match (name, args.len()) {
            ("__numAbs", 1) => DfNumPrimOp::Abs,
            ("__numRem", 2) => DfNumPrimOp::Rem,
            ("__numPow", 2) => DfNumPrimOp::Pow,
            ("__numToFloat", 1) => DfNumPrimOp::ToFloat,
            ("__numRound", 1) => DfNumPrimOp::Round,
            ("__numTruncate", 1) => DfNumPrimOp::Truncate,
            _ => return None,
        };
        let arg_nodes: Vec<NodeId> = args.iter().map(|&a| self.lower_expr(a)).collect();
        Some(self.alloc_node(
            DfNodeKind::NumPrim {
                op,
                args: arg_nodes,
            },
            df_ty,
            span,
        ))
    }

    fn lower_bool_short_circuit(
        &mut self,
        op: BuiltinOp,
        lhs: TlcExprId,
        rhs: TlcExprId,
        df_ty: crate::DfTyId,
        span: Option<Span>,
    ) -> Option<NodeId> {
        if !matches!(op, BuiltinOp::And | BuiltinOp::Or) {
            return None;
        }

        let scrutinee = self.lower_expr(lhs);
        let lit = |this: &mut Self, value| {
            this.alloc_node(DfNodeKind::Lit(DfLit::Bool(value)), df_ty, span)
        };
        let arms = match op {
            BuiltinOp::And => vec![
                DfArm {
                    pattern: DfPattern::Lit(DfLit::Bool(true)),
                    guard: None,
                    body: self.lower_expr(rhs),
                },
                DfArm {
                    pattern: DfPattern::Lit(DfLit::Bool(false)),
                    guard: None,
                    body: lit(self, false),
                },
            ],
            BuiltinOp::Or => vec![
                DfArm {
                    pattern: DfPattern::Lit(DfLit::Bool(true)),
                    guard: None,
                    body: lit(self, true),
                },
                DfArm {
                    pattern: DfPattern::Lit(DfLit::Bool(false)),
                    guard: None,
                    body: self.lower_expr(rhs),
                },
            ],
            _ => unreachable!("only logical operators short-circuit"),
        };

        Some(self.alloc_node(DfNodeKind::Match { scrutinee, arms }, df_ty, span))
    }

    fn lower_coalesce(
        &mut self,
        op: BuiltinOp,
        value: TlcExprId,
        fallback: TlcExprId,
        df_ty: crate::DfTyId,
        span: Option<Span>,
    ) -> Option<NodeId> {
        if op != BuiltinOp::Coalesce {
            return None;
        }

        let scrutinee = self.lower_expr(value);
        let scrutinee_ty = self.nodes[scrutinee].ty;
        let fallback = self.lower_expr(fallback);
        let some_bind = self.alloc_node(DfNodeKind::Bind, df_ty, None);
        let present_bind = self.alloc_node(DfNodeKind::Bind, df_ty, None);
        let payload_slot =
            |bind| DfPattern::Record(vec![("0".to_string(), 0, DfPattern::Bind(bind))]);
        let variant = |this: &Self, tag: &str, pattern| DfPattern::Variant {
            tag: tag.to_string(),
            tag_index: this.variant_tag_index_for_df_ty(scrutinee_ty, tag),
            pattern: Box::new(pattern),
        };
        let arms = vec![
            DfArm {
                pattern: DfPattern::Atom("none".to_string()),
                guard: None,
                body: fallback,
            },
            DfArm {
                pattern: DfPattern::Atom("absent".to_string()),
                guard: None,
                body: fallback,
            },
            DfArm {
                pattern: variant(self, "none", DfPattern::Wildcard),
                guard: None,
                body: fallback,
            },
            DfArm {
                pattern: variant(self, "absent", DfPattern::Wildcard),
                guard: None,
                body: fallback,
            },
            DfArm {
                pattern: variant(self, "some", payload_slot(some_bind)),
                guard: None,
                body: some_bind,
            },
            DfArm {
                pattern: variant(self, "present", payload_slot(present_bind)),
                guard: None,
                body: present_bind,
            },
        ];

        Some(self.alloc_node(DfNodeKind::Match { scrutinee, arms }, df_ty, span))
    }

    /// Lower a saturated application of a text bridge builtin to a primitive
    /// node. Returns `None` for any other callee or an under-saturated call.
    fn try_lower_text_prim(
        &mut self,
        id: TlcExprId,
        df_ty: crate::DfTyId,
        span: Option<Span>,
    ) -> Option<NodeId> {
        let mut args_rev: Vec<TlcExprId> = Vec::new();
        let mut cur = id;
        let name = loop {
            match &self.module.expr_arena[cur] {
                TlcExpr::App(func, arg) => {
                    args_rev.push(*arg);
                    cur = *func;
                }
                TlcExpr::TyApp(inner, _) => cur = *inner,
                TlcExpr::Var(binding) => break self.text_prim_builtin_name(*binding)?,
                _ => return None,
            }
        };
        args_rev.reverse();
        let args = args_rev;

        let op = match (name, args.len()) {
            ("__textLength", 1) => DfTextPrimOp::Length,
            ("__textSplit", 2) => DfTextPrimOp::Split,
            ("__textJoin", 2) => DfTextPrimOp::Join,
            ("__textTrim", 1) => DfTextPrimOp::Trim,
            ("__textToUpper", 1) => DfTextPrimOp::ToUpper,
            ("__textToLower", 1) => DfTextPrimOp::ToLower,
            ("__textContains", 2) => DfTextPrimOp::Contains,
            ("__textReplace", 3) => DfTextPrimOp::Replace,
            ("__textShow", 1) => DfTextPrimOp::Show,
            ("__textParseInt", 1) => DfTextPrimOp::ParseInt,
            ("__textParseFloat", 1) => DfTextPrimOp::ParseFloat,
            _ => return None,
        };
        let arg_nodes: Vec<NodeId> = args.iter().map(|&a| self.lower_expr(a)).collect();
        Some(self.alloc_node(
            DfNodeKind::TextPrim {
                op,
                args: arg_nodes,
            },
            df_ty,
            span,
        ))
    }

    /// Name of a list-bridge builtin value binding, or `None` if `binding` is not
    /// one of them.
    fn list_bridge_builtin_name(&self, binding: BindingId) -> Option<&'static str> {
        let b = self.hir_bindings.get(binding.0 as usize)?;
        if b.kind != BindingKind::BuiltinValue {
            return None;
        }
        match b.name.as_str() {
            "listEmpty" => Some("listEmpty"),
            "listCons" => Some("listCons"),
            "listIsNil" => Some("listIsNil"),
            "listHead" => Some("listHead"),
            "listTail" => Some("listTail"),
            "listFoldlStrict" => Some("listFoldlStrict"),
            _ => None,
        }
    }

    /// Name of a numeric bridge builtin value binding, or `None` if `binding` is
    /// not one of them.
    fn num_prim_builtin_name(&self, binding: BindingId) -> Option<&'static str> {
        let b = self.hir_bindings.get(binding.0 as usize)?;
        if b.kind != BindingKind::BuiltinValue {
            return None;
        }
        match b.name.as_str() {
            "__numAbs" => Some("__numAbs"),
            "__numRem" => Some("__numRem"),
            "__numPow" => Some("__numPow"),
            "__numToFloat" => Some("__numToFloat"),
            "__numRound" => Some("__numRound"),
            "__numTruncate" => Some("__numTruncate"),
            _ => None,
        }
    }

    /// Name of a text bridge builtin value binding, or `None` if `binding` is
    /// not one of them.
    fn text_prim_builtin_name(&self, binding: BindingId) -> Option<&'static str> {
        let b = self.hir_bindings.get(binding.0 as usize)?;
        if b.kind != BindingKind::BuiltinValue {
            return None;
        }
        match b.name.as_str() {
            "__textLength" => Some("__textLength"),
            "__textSplit" => Some("__textSplit"),
            "__textJoin" => Some("__textJoin"),
            "__textTrim" => Some("__textTrim"),
            "__textToUpper" => Some("__textToUpper"),
            "__textToLower" => Some("__textToLower"),
            "__textContains" => Some("__textContains"),
            "__textReplace" => Some("__textReplace"),
            "__textShow" => Some("__textShow"),
            "__textParseInt" => Some("__textParseInt"),
            "__textParseFloat" => Some("__textParseFloat"),
            _ => None,
        }
    }

    pub(super) fn lower_posit_builtin_op(
        &self,
        op: BuiltinOp,
        lhs: TlcExprId,
    ) -> Option<DfBuiltinOp> {
        let ty = self.module.expr_types.get(&lhs)?;
        let TlcType::Prim(PrimTy::Posit(spec)) = self.module.type_arena[*ty] else {
            return None;
        };
        let op = lower_posit_op(op)?;
        Some(DfBuiltinOp::Posit { op, spec })
    }

    pub(super) fn lower_text_builtin_op(
        &self,
        op: BuiltinOp,
        lhs: TlcExprId,
    ) -> Option<DfTextPrimOp> {
        let op = match op {
            BuiltinOp::Eq => DfTextPrimOp::Eq,
            BuiltinOp::Ne => DfTextPrimOp::Ne,
            BuiltinOp::Lt => DfTextPrimOp::Lt,
            BuiltinOp::Le => DfTextPrimOp::Le,
            BuiltinOp::Gt => DfTextPrimOp::Gt,
            BuiltinOp::Ge => DfTextPrimOp::Ge,
            _ => return None,
        };
        let ty = self.module.expr_types.get(&lhs)?;
        match &self.module.type_arena[*ty] {
            TlcType::Prim(PrimTy::Str) | TlcType::Singleton(zutai_tlc::Literal::Str(_)) => Some(op),
            _ => None,
        }
    }

    pub(super) fn lower_float_builtin_op(
        &self,
        op: BuiltinOp,
        lhs: TlcExprId,
    ) -> Option<DfNumPrimOp> {
        let ty = self.module.expr_types.get(&lhs)?;
        let (TlcType::Prim(PrimTy::Float) | TlcType::Singleton(zutai_tlc::Literal::Float(_))) =
            self.module.type_arena[*ty]
        else {
            return None;
        };
        match op {
            BuiltinOp::Add => Some(DfNumPrimOp::FloatAdd),
            BuiltinOp::Sub => Some(DfNumPrimOp::FloatSub),
            BuiltinOp::Mul => Some(DfNumPrimOp::FloatMul),
            BuiltinOp::Div => Some(DfNumPrimOp::FloatDiv),
            BuiltinOp::Lt => Some(DfNumPrimOp::FloatLt),
            BuiltinOp::Le => Some(DfNumPrimOp::FloatLe),
            BuiltinOp::Gt => Some(DfNumPrimOp::FloatGt),
            BuiltinOp::Ge => Some(DfNumPrimOp::FloatGe),
            _ => None,
        }
    }
}
