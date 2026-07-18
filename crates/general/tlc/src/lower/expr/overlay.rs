use rustc_hash::FxHashMap;

use zutai_hir::BindingId;
use zutai_syntax::Span;
use zutai_thir::{ThirExprId, ThirExprKind, TypeId, TypeKind, TypeRecordField};

use crate::ir::{
    Literal, Row, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcPatItem, TlcTupleField, TlcTupleItem,
    TlcType, TlcTypeId,
};

use crate::lower::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn lower_overlay_apply(
        &mut self,
        func: ThirExprId,
        arg: ThirExprId,
        result_ty: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        if let Some((patch, deep)) = self.overlay_full_apply_parts(func) {
            let base = self.lower_expr(arg);
            let patch = self.lower_expr(patch);
            return Some(self.lower_overlay_record(
                base,
                patch,
                result_ty,
                &FxHashMap::default(),
                deep,
                span,
            ));
        }

        let (deep, target) = self.overlay_partial_apply_parts(func, result_ty)?;
        let patch = self.lower_expr(arg);
        let overlay = self.lower_overlay_function_for_target(target, deep, span);
        let target_ty = self.lower_type(target);
        let applied_ty = self.alloc_type(TlcType::Fun(target_ty, target_ty, Row::REmpty));
        Some(self.alloc_expr(TlcExpr::App(overlay, patch), applied_ty, span))
    }

    pub(super) fn lower_overlay_function(
        &mut self,
        binding: BindingId,
        function_ty: TypeId,
        span: Span,
    ) -> TlcExprId {
        let deep = self
            .builtin_overlay_deep(binding)
            .expect("overlay function binding");
        let TypeKind::Function { from: patch, to } =
            self.thir.type_arena[function_ty.0 as usize].kind
        else {
            unreachable!("overlay binding must have a function type")
        };
        let TypeKind::Function {
            from: target,
            to: result,
        } = self.thir.type_arena[to.0 as usize].kind
        else {
            unreachable!("overlay binding must be curried")
        };
        debug_assert_eq!(target, result);
        let patch_tlc_ty = self.lower_type(patch);
        let function_tlc_ty = self.lower_type(function_ty);
        let patch_binding = self.fresh_synth_binding();
        let patch_value = self.alloc_expr(TlcExpr::Var(patch_binding), patch_tlc_ty, span);
        let body = self.lower_overlay_function_for_target(target, deep, span);
        let applied_ty = self.lower_type(to);
        let applied = self.alloc_expr(TlcExpr::App(body, patch_value), applied_ty, span);
        self.alloc_expr(
            TlcExpr::Lam(patch_binding, patch_tlc_ty, applied),
            function_tlc_ty,
            span,
        )
    }

    pub(super) fn lower_overlay_function_for_target(
        &mut self,
        target: TypeId,
        deep: bool,
        span: Span,
    ) -> TlcExprId {
        let patch_ty = self.lower_patch_type_with_subst(target, deep, &FxHashMap::default());
        let base_ty = self.lower_type(target);
        let result_ty = self.alloc_type(TlcType::Fun(base_ty, base_ty, Row::REmpty));
        let patch_binding = self.fresh_synth_binding();
        let patch = self.alloc_expr(TlcExpr::Var(patch_binding), patch_ty, span);
        let base_binding = self.fresh_synth_binding();
        let base = self.alloc_expr(TlcExpr::Var(base_binding), base_ty, span);
        let body =
            self.lower_overlay_record(base, patch, target, &FxHashMap::default(), deep, span);
        let base_lambda =
            self.alloc_expr(TlcExpr::Lam(base_binding, base_ty, body), result_ty, span);
        let function_ty = self.alloc_type(TlcType::Fun(patch_ty, result_ty, Row::REmpty));
        self.alloc_expr(
            TlcExpr::Lam(patch_binding, patch_ty, base_lambda),
            function_ty,
            span,
        )
    }

    pub(super) fn overlay_full_apply_parts(&self, func: ThirExprId) -> Option<(ThirExprId, bool)> {
        let ThirExprKind::Apply {
            func: builtin,
            arg: patch,
            ..
        } = &self.thir.expr_arena[func].kind
        else {
            return None;
        };
        let ThirExprKind::BindingRef { binding, .. } = &self.thir.expr_arena[*builtin].kind else {
            return None;
        };
        Some((*patch, self.builtin_overlay_deep(*binding)?))
    }

    pub(super) fn overlay_partial_apply_parts(
        &self,
        func: ThirExprId,
        result_ty: TypeId,
    ) -> Option<(bool, TypeId)> {
        let ThirExprKind::BindingRef { binding, .. } = &self.thir.expr_arena[func].kind else {
            return None;
        };
        let deep = self.builtin_overlay_deep(*binding)?;
        let TypeKind::Function { from, to } = self.thir.type_arena[result_ty.0 as usize].kind
        else {
            return None;
        };
        (from == to).then_some((deep, from))
    }

    pub(super) fn builtin_overlay_deep(&self, binding: BindingId) -> Option<bool> {
        if !self
            .thir
            .binding_kinds
            .get(binding.0 as usize)
            .is_some_and(|kind| *kind == zutai_hir::BindingKind::BuiltinValue)
        {
            return None;
        }
        match self.thir.binding_names.get(binding.0 as usize)?.as_str() {
            "overlay" => Some(false),
            "overlayDeep" => Some(true),
            _ => None,
        }
    }

    pub(super) fn lower_overlay_record(
        &mut self,
        base: TlcExprId,
        patch: TlcExprId,
        target: TypeId,
        subst: &FxHashMap<BindingId, TypeId>,
        deep: bool,
        span: Span,
    ) -> TlcExprId {
        let Some((target_fields, tail, env)) = self.record_shape_with_subst(target, subst) else {
            return base;
        };
        let result_ty = self.lower_type_with_subst(target, subst);
        let patch_ty = self.lower_patch_type_with_subst(target, deep, subst);
        let patch_binding = self.fresh_synth_binding();
        let patch_value = self.alloc_expr(TlcExpr::Var(patch_binding), patch_ty, span);
        let updates = target_fields
            .iter()
            .map(|field| {
                let value = self.lower_overlay_field(base, patch_value, field, &env, deep, span);
                (field.name.clone(), value)
            })
            .collect();
        let updated = self.alloc_expr(
            TlcExpr::RecordUpdate {
                receiver: base,
                fields: updates,
            },
            result_ty,
            span,
        );
        let body = if matches!(tail, zutai_thir::RowTail::Closed) {
            updated
        } else {
            base
        };
        self.alloc_expr(
            TlcExpr::Let {
                binding: patch_binding,
                ty: patch_ty,
                value: patch,
                body,
            },
            result_ty,
            span,
        )
    }

    pub(super) fn lower_overlay_field(
        &mut self,
        base: TlcExprId,
        patch: TlcExprId,
        field: &TypeRecordField,
        subst: &FxHashMap<BindingId, TypeId>,
        deep: bool,
        span: Span,
    ) -> TlcExprId {
        let field_ty = self.lower_type_with_subst(field.ty, subst);
        let patch_field_ty = if deep && self.record_shape_with_subst(field.ty, subst).is_some() {
            self.lower_patch_type_with_subst(field.ty, true, subst)
        } else {
            field_ty
        };
        let maybe_patch_field_ty = self.alloc_type(TlcType::Maybe(patch_field_ty));
        let patch_field = self.alloc_expr(
            TlcExpr::GetField(patch, field.name.clone()),
            maybe_patch_field_ty,
            span,
        );
        let present_binding = self.fresh_synth_binding();
        let present = self.alloc_expr(TlcExpr::Var(present_binding), patch_field_ty, span);
        let base_field_ty = if field.optional {
            self.alloc_type(TlcType::Maybe(field_ty))
        } else {
            field_ty
        };
        let base_field = self.alloc_expr(
            TlcExpr::GetField(base, field.name.clone()),
            base_field_ty,
            span,
        );
        let merged = if deep && self.record_shape_with_subst(field.ty, subst).is_some() {
            self.lower_optional_nested_overlay(base_field, present, field, subst, span)
        } else if field.optional {
            self.wrap_maybe_present(present, field_ty, base_field_ty, span)
        } else {
            present
        };
        self.alloc_expr(
            TlcExpr::Case(
                patch_field,
                vec![
                    TlcAlt {
                        pat: TlcPat::Atom("absent".to_string()),
                        guard: None,
                        body: base_field,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "present".to_string(),
                            Box::new(TlcPat::Tuple(vec![TlcPatItem::Positional(TlcPat::Bind(
                                present_binding,
                            ))])),
                        ),
                        guard: None,
                        body: merged,
                    },
                ],
            ),
            base_field_ty,
            span,
        )
    }

    pub(super) fn lower_optional_nested_overlay(
        &mut self,
        base_field: TlcExprId,
        patch_field: TlcExprId,
        field: &TypeRecordField,
        subst: &FxHashMap<BindingId, TypeId>,
        span: Span,
    ) -> TlcExprId {
        if !field.optional {
            return self.lower_overlay_record(base_field, patch_field, field.ty, subst, true, span);
        }

        let target_ty = self.lower_type_with_subst(field.ty, subst);
        let maybe_target_ty = self.alloc_type(TlcType::Maybe(target_ty));

        let existing_binding = self.fresh_synth_binding();
        let existing = self.alloc_expr(TlcExpr::Var(existing_binding), target_ty, span);
        let merged_existing =
            self.lower_overlay_record(existing, patch_field, field.ty, subst, true, span);
        let base_absent = self.alloc_expr(
            TlcExpr::Lit(Literal::Atom("absent".to_string())),
            maybe_target_ty,
            span,
        );
        let merged_existing =
            self.wrap_maybe_present(merged_existing, target_ty, maybe_target_ty, span);
        self.alloc_expr(
            TlcExpr::Case(
                base_field,
                vec![
                    TlcAlt {
                        pat: TlcPat::Atom("absent".to_string()),
                        guard: None,
                        body: base_absent,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "present".to_string(),
                            Box::new(TlcPat::Tuple(vec![TlcPatItem::Positional(TlcPat::Bind(
                                existing_binding,
                            ))])),
                        ),
                        guard: None,
                        body: merged_existing,
                    },
                ],
            ),
            maybe_target_ty,
            span,
        )
    }

    pub(super) fn wrap_maybe_present(
        &mut self,
        value: TlcExprId,
        inner_ty: TlcTypeId,
        maybe_ty: TlcTypeId,
        span: Span,
    ) -> TlcExprId {
        let payload_ty = self.alloc_type(TlcType::Tuple(vec![TlcTupleField::Positional(inner_ty)]));
        let payload = self.alloc_expr(
            TlcExpr::Tuple(vec![TlcTupleItem::Positional(value)]),
            payload_ty,
            span,
        );
        self.alloc_expr(
            TlcExpr::Variant("present".to_string(), payload),
            maybe_ty,
            span,
        )
    }
}
