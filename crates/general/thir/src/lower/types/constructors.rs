use super::*;
use crate::ir::FixedWidth;
use zutai_hir::NumberType;
use zutai_syntax::posit::PositSpec;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn alias_type(&mut self, binding: BindingId, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Alias(binding),
            span,
        })
    }

    pub(in crate::lower) fn bool_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Bool,
            span,
        })
    }

    pub(in crate::lower) fn int_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Int,
            span,
        })
    }

    pub(in crate::lower) fn float_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Float,
            span,
        })
    }

    pub(in crate::lower) fn posit_type(&mut self, spec: PositSpec, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Posit(spec),
            span,
        })
    }

    pub(in crate::lower) fn fixed_num_type(&mut self, fw: FixedWidth, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::FixedNum(fw),
            span,
        })
    }

    pub(in crate::lower) fn integer_literal_type(
        &mut self,
        value: i64,
        postfix: Option<NumberType>,
        span: Span,
    ) -> TypeId {
        match postfix {
            None | Some(NumberType::I64) => self.int_type(span),
            Some(NumberType::I8) => self.range_checked_fixed_num_type(value, FixedWidth::I8, span),
            Some(NumberType::I16) => {
                self.range_checked_fixed_num_type(value, FixedWidth::I16, span)
            }
            Some(NumberType::I32) => {
                self.range_checked_fixed_num_type(value, FixedWidth::I32, span)
            }
            Some(NumberType::U8) => self.range_checked_fixed_num_type(value, FixedWidth::U8, span),
            Some(NumberType::U16) => {
                self.range_checked_fixed_num_type(value, FixedWidth::U16, span)
            }
            Some(NumberType::U32) => {
                self.range_checked_fixed_num_type(value, FixedWidth::U32, span)
            }
            Some(NumberType::U64) => {
                self.range_checked_fixed_num_type(value, FixedWidth::U64, span)
            }
            Some(NumberType::F32) => self.fixed_num_type(FixedWidth::F32, span),
            Some(NumberType::F64) => self.float_type(span),
            Some(NumberType::Posit(spec)) => self.posit_type(spec, span),
        }
    }

    pub(in crate::lower) fn float_literal_type(
        &mut self,
        postfix: Option<NumberType>,
        span: Span,
    ) -> TypeId {
        match postfix {
            None | Some(NumberType::F64) => self.float_type(span),
            Some(NumberType::F32) => self.fixed_num_type(FixedWidth::F32, span),
            Some(NumberType::I64) => self.int_type(span),
            Some(NumberType::I8) => self.fixed_num_type(FixedWidth::I8, span),
            Some(NumberType::I16) => self.fixed_num_type(FixedWidth::I16, span),
            Some(NumberType::I32) => self.fixed_num_type(FixedWidth::I32, span),
            Some(NumberType::U8) => self.fixed_num_type(FixedWidth::U8, span),
            Some(NumberType::U16) => self.fixed_num_type(FixedWidth::U16, span),
            Some(NumberType::U32) => self.fixed_num_type(FixedWidth::U32, span),
            Some(NumberType::U64) => self.fixed_num_type(FixedWidth::U64, span),
            Some(NumberType::Posit(spec)) => self.posit_type(spec, span),
        }
    }

    fn range_checked_fixed_num_type(&mut self, value: i64, fw: FixedWidth, span: Span) -> TypeId {
        if let Some((min, max)) = fw.int_range()
            && !(min..=max).contains(&value)
        {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::NumericLiteralOutOfRange {
                    value,
                    ty: fw.name().to_string(),
                },
                span,
            });
        }
        self.fixed_num_type(fw, span)
    }

    pub(in crate::lower) fn text_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Text,
            span,
        })
    }

    pub(in crate::lower) fn optional_type(&mut self, inner: TypeId, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Optional(inner),
            span,
        })
    }

    pub(in crate::lower) fn maybe_type(&mut self, inner: TypeId, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Maybe(inner),
            span,
        })
    }

    pub(in crate::lower) fn code_type(&mut self, inner: TypeId, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Code(inner),
            span,
        })
    }
}
