use zutai_hir::BindingId;
use zutai_thir::TypeId;

use crate::ir::{Literal, PrimTy, Row, TlcAlt, TlcExpr, TlcExprId, TlcPat, TlcPatItem, TlcType};

use super::*;
use crate::lower::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn synthesize_show_method(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        sig: TypeId,
        constraint_param: BindingId,
        target: TypeId,
    ) -> Option<TlcExprId> {
        let (arg_ty, result_ty) = self.unary_text_method_parts(sig, constraint_param, target)?;
        let span = zutai_syntax::Span::default();
        let arg = self.fresh_synth_binding();
        let arg_tlc_ty = self.lower_type(arg_ty);
        let result_tlc_ty = self.lower_type(result_ty);
        let arg_expr = self.alloc_expr(TlcExpr::Var(arg), arg_tlc_ty, span);
        let body = self.derive_show_expr(constraint, method_name, target, arg_expr);
        let fn_ty = self.alloc_type(TlcType::Fun(arg_tlc_ty, result_tlc_ty, Row::REmpty));
        Some(self.alloc_expr(TlcExpr::Lam(arg, arg_tlc_ty, body), fn_ty, span))
    }

    pub(super) fn derive_show_expr(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        ty: TypeId,
        arg: TlcExprId,
    ) -> TlcExprId {
        let span = zutai_syntax::Span::default();
        let text_ty = self.alloc_type(TlcType::Prim(PrimTy::Str));
        match self.derive_shape(ty) {
            DeriveShape::Union(variants) => {
                let mut alts = Vec::with_capacity(variants.len() + 1);
                for variant in variants {
                    let (pat, body) = if variant.payload_fields.is_empty() {
                        (
                            TlcPat::Atom(variant.name.clone()),
                            self.str_lit(&format!("#{}", variant.name)),
                        )
                    } else {
                        let mut field_pats = Vec::with_capacity(variant.payload_fields.len());
                        let mut pieces = Vec::new();
                        pieces.push(self.str_lit(&format!("#{} {{", variant.name)));
                        for (index, (field_name, field_ty)) in
                            variant.payload_fields.clone().into_iter().enumerate()
                        {
                            let binding = self.fresh_synth_binding();
                            let field_tlc_ty = self.lower_type(field_ty);
                            let field_expr =
                                self.alloc_expr(TlcExpr::Var(binding), field_tlc_ty, span);
                            field_pats.push((field_name.clone(), TlcPat::Bind(binding)));
                            if index > 0 {
                                pieces.push(self.str_lit(", "));
                            }
                            pieces.push(self.str_lit(&format!("{field_name} = ")));
                            let shown = self.derive_component_show(
                                constraint,
                                method_name,
                                field_ty,
                                field_expr,
                            );
                            pieces.push(shown);
                        }
                        pieces.push(self.str_lit("}"));
                        (
                            TlcPat::Variant(
                                variant.name.clone(),
                                Box::new(TlcPat::Record(field_pats)),
                            ),
                            self.text_join(pieces),
                        )
                    };
                    alts.push(TlcAlt {
                        pat,
                        guard: None,
                        body,
                    });
                }
                let fallback = self.str_lit("union");
                alts.push(TlcAlt {
                    pat: TlcPat::Wildcard,
                    guard: None,
                    body: fallback,
                });
                self.alloc_expr(TlcExpr::Case(arg, alts), text_ty, span)
            }
            DeriveShape::Record(fields) => {
                let mut pieces = Vec::new();
                pieces.push(self.str_lit("{"));
                for (index, (name, field_ty)) in fields.into_iter().enumerate() {
                    if index > 0 {
                        pieces.push(self.str_lit(", "));
                    }
                    pieces.push(self.str_lit(&format!("{name} = ")));
                    let field = self.derive_get_field(arg, name.as_str(), field_ty);
                    let shown =
                        self.derive_component_show(constraint, method_name, field_ty, field);
                    pieces.push(shown);
                }
                pieces.push(self.str_lit("}"));
                self.text_join(pieces)
            }
            DeriveShape::Tuple(items) => {
                let mut item_pats = Vec::with_capacity(items.len());
                let mut pieces = Vec::new();
                pieces.push(self.str_lit("("));
                for (index, (name, item_ty)) in items.into_iter().enumerate() {
                    let binding = self.fresh_synth_binding();
                    let item_tlc_ty = self.lower_type(item_ty);
                    let item_var = self.alloc_expr(TlcExpr::Var(binding), item_tlc_ty, span);
                    let pat = TlcPat::Bind(binding);
                    match name {
                        Some(name) => item_pats.push(TlcPatItem::Named { name, pat }),
                        None => item_pats.push(TlcPatItem::Positional(pat)),
                    }
                    if index > 0 {
                        pieces.push(self.str_lit(", "));
                    }
                    let shown =
                        self.derive_component_show(constraint, method_name, item_ty, item_var);
                    pieces.push(shown);
                }
                pieces.push(self.str_lit(")"));
                let body = self.text_join(pieces);
                let fallback = self.str_lit("()");
                self.alloc_expr(
                    TlcExpr::Case(
                        arg,
                        vec![
                            TlcAlt {
                                pat: TlcPat::Tuple(item_pats),
                                guard: None,
                                body,
                            },
                            TlcAlt {
                                pat: TlcPat::Wildcard,
                                guard: None,
                                body: fallback,
                            },
                        ],
                    ),
                    text_ty,
                    span,
                )
            }
            DeriveShape::Leaf => self.alloc_expr(
                TlcExpr::Lit(Literal::Str(self.type_label_for_derive(ty))),
                text_ty,
                span,
            ),
        }
    }

    /// Render one component under `Show` by delegating to its witness's `show`
    /// method, mirroring `derive_component_ord`. For a recursive component the
    /// derived witness is itself registered, so `has_witness_binding` yields a
    /// runtime dictionary self-reference (via `get_dict_expr`) instead of a
    /// compile-time structural expansion — the same recursion-termination
    /// mechanism the Ord/Eq builders rely on. The structural fallback is only
    /// reachable for programs the component-witness check already rejected.
    pub(super) fn derive_component_show(
        &mut self,
        constraint: BindingId,
        method_name: &str,
        ty: TypeId,
        value: TlcExprId,
    ) -> TlcExprId {
        if self.has_witness_binding(constraint, ty) {
            let span = zutai_syntax::Span::default();
            let text_ty = self.alloc_type(TlcType::Prim(PrimTy::Str));
            let component_ty = self.lower_type(ty);
            let method_ty = self.alloc_type(TlcType::Fun(component_ty, text_ty, Row::REmpty));
            let dict = self.get_dict_expr(constraint, ty, span);
            let method = self.alloc_expr(
                TlcExpr::GetField(dict, method_name.to_string()),
                method_ty,
                span,
            );
            self.register_dict_field_slot(method, constraint, method_name);
            return self.alloc_expr(TlcExpr::App(method, value), text_ty, span);
        }
        self.derive_show_expr(constraint, method_name, ty, value)
    }
}
