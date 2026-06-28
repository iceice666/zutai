use zutai_tlc::{TlcAlt, TlcPat, TlcPatItem};

use crate::{DfArm, DfNodeKind, DfPattern, DfTuplePatItem, DfTyId};

use super::*;

impl<'m> Lowerer<'m> {
    // ── Match arm lowering ────────────────────────────────────────────────────

    pub(super) fn lower_alt(&mut self, alt: &TlcAlt, scrutinee_ty: DfTyId) -> DfArm {
        // Pattern lowering inserts Bind nodes into local_env.
        // Passing the scrutinee type gives each Bind node an accurate DfTyId.
        let pattern = self.lower_pat(&alt.pat, scrutinee_ty);
        let guard = alt.guard.map(|g| self.lower_expr(g));
        let body = self.lower_expr(alt.body);
        // Remove pattern bindings from scope (arm body is done).
        remove_pat_bindings(&alt.pat, &mut self.local_env);
        DfArm {
            pattern,
            guard,
            body,
        }
    }

    pub(super) fn lower_pat(&mut self, pat: &TlcPat, context_ty: DfTyId) -> DfPattern {
        match pat {
            TlcPat::Wildcard => DfPattern::Wildcard,
            TlcPat::Bind(binding) => {
                let bind_node = self.alloc_node(DfNodeKind::Bind, context_ty, None);
                self.local_env.insert(*binding, bind_node);
                DfPattern::Bind(bind_node)
            }
            TlcPat::Lit(lit) => match lower_lit(lit) {
                Some(df_lit) => DfPattern::Lit(df_lit),
                None => DfPattern::Wildcard,
            },
            TlcPat::Atom(s) => DfPattern::Atom(s.clone()),
            TlcPat::Tuple(items) => {
                let df_items = items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| match item {
                        TlcPatItem::Named { name, pat } => {
                            let item_ty = self
                                .tuple_field_ty_for_df_ty(context_ty, index, Some(name))
                                .unwrap_or(context_ty);
                            DfTuplePatItem::Named {
                                name: name.clone(),
                                pattern: self.lower_pat(pat, item_ty),
                            }
                        }
                        TlcPatItem::Positional(p) => {
                            let item_ty = self
                                .tuple_field_ty_for_df_ty(context_ty, index, None)
                                .unwrap_or(context_ty);
                            DfTuplePatItem::Positional(self.lower_pat(p, item_ty))
                        }
                    })
                    .collect();
                DfPattern::Tuple(df_items)
            }
            TlcPat::ListNil => DfPattern::ListNil,
            TlcPat::ListCons(head, tail) => {
                let head_ty = self
                    .list_element_ty_for_df_ty(context_ty)
                    .unwrap_or(context_ty);
                DfPattern::ListCons {
                    head: Box::new(self.lower_pat(head, head_ty)),
                    tail: Box::new(self.lower_pat(tail, context_ty)),
                }
            }
            TlcPat::Record(fields) => {
                let df_fields = fields
                    .iter()
                    .map(|(name, p)| {
                        let slot = self.record_slot_for_df_ty(context_ty, name).unwrap_or(0);
                        let field_ty = self
                            .record_field_ty_for_df_ty(context_ty, name)
                            .unwrap_or(context_ty);
                        (name.clone(), slot, self.lower_pat(p, field_ty))
                    })
                    .collect();
                DfPattern::Record(df_fields)
            }
            TlcPat::Variant(tag, inner) => {
                let tag_index = self.variant_tag_index_for_df_ty(context_ty, tag);
                let payload_ty = self.variant_payload_ty_for_df_ty(context_ty, tag);
                DfPattern::Variant {
                    tag: tag.clone(),
                    tag_index,
                    pattern: Box::new(self.lower_pat(inner, payload_ty)),
                }
            }
        }
    }
}
