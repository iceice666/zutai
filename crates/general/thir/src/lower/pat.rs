use zutai_hir::{BindingId, HirPatId, HirPatKind};
use zutai_syntax::Span;

use crate::ir::{ThirPat, ThirPatId, ThirPatKind, TypeId};

use super::Lowerer;

impl<'hir> Lowerer<'hir> {
    pub(super) fn check_pattern(
        &mut self,
        id: HirPatId,
        expected: TypeId,
        scoped_bindings: &mut Vec<BindingId>,
    ) -> ThirPatId {
        use crate::ir::{Type, TypeKind};
        let pattern = self.hir_pat(id);
        let kind = match &pattern.kind {
            HirPatKind::Wildcard => ThirPatKind::Wildcard,
            HirPatKind::Bind(binding) => {
                self.value_types.insert(*binding, expected);
                scoped_bindings.push(*binding);
                ThirPatKind::Bind(*binding)
            }
            HirPatKind::True => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::True,
                    span: pattern.span,
                });
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::True
            }
            HirPatKind::False => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::False,
                    span: pattern.span,
                });
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::False
            }
            HirPatKind::Integer(value) => {
                let ty = self.int_type(pattern.span);
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::Integer(*value)
            }
            HirPatKind::Float(value) => {
                let ty = self.float_type(pattern.span);
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::Float(*value)
            }
            HirPatKind::String(value) => {
                let ty = self.text_type(pattern.span);
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::String(value.clone())
            }
            HirPatKind::Atom(name) => {
                let ty = self.alloc_type(Type {
                    kind: TypeKind::Atom(name.clone()),
                    span: pattern.span,
                });
                self.check_pattern_type(expected, ty, pattern.span);
                ThirPatKind::Atom(name.clone())
            }
            HirPatKind::Tuple(_) => {
                self.unsupported("tuple patterns", pattern.span);
                ThirPatKind::Error
            }
            HirPatKind::Record(_) => {
                self.unsupported("record patterns", pattern.span);
                ThirPatKind::Error
            }
        };
        self.alloc_pat(ThirPat {
            source: id,
            ty: expected,
            kind,
            span: pattern.span,
        })
    }

    pub(super) fn clear_scoped_value_types(&mut self, scoped_bindings: &[BindingId]) {
        for binding in scoped_bindings {
            self.value_types.remove(binding);
        }
    }

    fn check_pattern_type(&mut self, expected: TypeId, found: TypeId, span: Span) {
        if !self.type_matches(expected, found) {
            self.type_mismatch(expected, found, span);
        }
    }
}
