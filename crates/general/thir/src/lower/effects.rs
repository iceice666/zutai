use super::*;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn unit_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Tuple(Vec::new()),
            span,
        })
    }

    pub(in crate::lower) fn never_type(&mut self, span: Span) -> TypeId {
        self.alloc_type(Type {
            kind: TypeKind::Never,
            span,
        })
    }

    pub(in crate::lower) fn lookup_op(&self, name: &str) -> Option<(TypeId, TypeId)> {
        for layer in self.handled_stack.iter().rev() {
            if let Some(&sig) = layer.get(name) {
                return Some(sig);
            }
        }
        self.effect_ambient
            .find(name)
            .map(|op| (op.param, op.result))
    }

    pub(in crate::lower) fn enter_effectful_result(
        &mut self,
        return_ty: TypeId,
    ) -> (TypeId, EffectRow) {
        let saved = std::mem::replace(&mut self.effect_ambient, EffectRow::closed_empty());
        let resolved = self.resolve_alias(
            return_ty,
            &mut FxHashSet::default(),
            self.ty(return_ty).span,
        );
        match self.type_arena[resolved.0 as usize].kind.clone() {
            TypeKind::Effect { base, row } => {
                self.effect_ambient = row;
                (base, saved)
            }
            _ => (return_ty, saved),
        }
    }

    pub(in crate::lower) fn exit_effectful_result(&mut self, saved: EffectRow) {
        self.effect_ambient = saved;
    }

    pub(in crate::lower) fn enter_host_effect_boundary(&mut self, span: Span) -> EffectRow {
        let row = self.host_boundary_effect_row(span);
        std::mem::replace(&mut self.effect_ambient, row)
    }

    pub(in crate::lower) fn io_print_effect_row(&mut self, span: Span) -> EffectRow {
        let text = self.text_type(span);
        EffectRow {
            ops: vec![EffectOp {
                name: "io.print".to_string(),
                param: text,
                result: text,
                span,
            }],
            tail: RowTail::Closed,
        }
    }

    pub(in crate::lower) fn host_boundary_effect_row(&mut self, span: Span) -> EffectRow {
        let text = self.text_type(span);
        let unit = self.unit_type(span);
        let int = self.int_type(span);
        let reader = self.alloc_type(Type {
            kind: TypeKind::Opaque("Reader".to_string()),
            span,
        });
        let writer = self.alloc_type(Type {
            kind: TypeKind::Opaque("Writer".to_string()),
            span,
        });
        let maybe_text = self.alloc_type(Type {
            kind: TypeKind::Optional(text),
            span,
        });
        let write_arg = self.alloc_type(Type {
            kind: TypeKind::Record(
                vec![
                    TypeRecordField {
                        name: "contents".to_string(),
                        optional: false,
                        ty: text,
                        span,
                    },
                    TypeRecordField {
                        name: "path".to_string(),
                        optional: false,
                        ty: text,
                        span,
                    },
                ],
                RowTail::Closed,
            ),
            span,
        });
        let write_text_arg = self.alloc_type(Type {
            kind: TypeKind::Record(
                vec![
                    TypeRecordField {
                        name: "contents".to_string(),
                        optional: false,
                        ty: text,
                        span,
                    },
                    TypeRecordField {
                        name: "writer".to_string(),
                        optional: false,
                        ty: writer,
                        span,
                    },
                ],
                RowTail::Closed,
            ),
            span,
        });
        let mut ops = vec![
            EffectOp {
                name: "io.print".to_string(),
                param: text,
                result: text,
                span,
            },
            EffectOp {
                name: "fs.read".to_string(),
                param: text,
                result: text,
                span,
            },
            EffectOp {
                name: "fs.write".to_string(),
                param: write_arg,
                result: unit,
                span,
            },
            EffectOp {
                name: "fs.openRead".to_string(),
                param: text,
                result: reader,
                span,
            },
            EffectOp {
                name: "fs.readLine".to_string(),
                param: reader,
                result: maybe_text,
                span,
            },
            EffectOp {
                name: "fs.closeRead".to_string(),
                param: reader,
                result: unit,
                span,
            },
            EffectOp {
                name: "fs.openWrite".to_string(),
                param: text,
                result: writer,
                span,
            },
            EffectOp {
                name: "fs.writeText".to_string(),
                param: write_text_arg,
                result: unit,
                span,
            },
            EffectOp {
                name: "fs.flush".to_string(),
                param: writer,
                result: unit,
                span,
            },
            EffectOp {
                name: "fs.closeWrite".to_string(),
                param: writer,
                result: unit,
                span,
            },
            EffectOp {
                name: "env.get".to_string(),
                param: text,
                result: maybe_text,
                span,
            },
            EffectOp {
                name: "clock.now".to_string(),
                param: unit,
                result: text,
                span,
            },
            EffectOp {
                name: "rng.next".to_string(),
                param: unit,
                result: int,
                span,
            },
            EffectOp {
                name: "net.listen".to_string(),
                param: int,
                result: int,
                span,
            },
            EffectOp {
                name: "net.accept".to_string(),
                param: int,
                result: int,
                span,
            },
            EffectOp {
                name: "net.read".to_string(),
                param: int,
                result: text,
                span,
            },
            EffectOp {
                name: "net.write".to_string(),
                param: text,
                result: unit,
                span,
            },
            EffectOp {
                name: "net.close".to_string(),
                param: int,
                result: unit,
                span,
            },
        ];
        if let Some(data) = self.data_prelude_type(span) {
            ops.push(EffectOp {
                name: "load.zti".to_string(),
                param: text,
                result: data,
                span,
            });
            ops.push(EffectOp {
                name: "load.zt".to_string(),
                param: text,
                result: data,
                span,
            });
        }
        EffectRow {
            ops,
            tail: RowTail::Closed,
        }
    }

    pub(in crate::lower) fn data_prelude_type(&mut self, span: Span) -> Option<TypeId> {
        let binding = self
            .hir
            .bindings
            .iter()
            .position(|binding| binding.name == "Data")
            .map(|index| zutai_hir::BindingId(index as u32))?;
        Some(self.alias_or_builtin_type(binding, span))
    }

    pub(in crate::lower) fn is_non_function_effect_type(&mut self, ty: TypeId) -> bool {
        let span = self.ty(ty).span;
        let resolved = self.resolve_alias(ty, &mut FxHashSet::default(), span);
        matches!(
            &self.type_arena[resolved.0 as usize].kind,
            TypeKind::Effect { row, .. } if !row.is_pure()
        )
    }

    pub(in crate::lower) fn is_never_type(&mut self, ty: TypeId) -> bool {
        let span = self.ty(ty).span;
        let resolved = self.resolve_alias(ty, &mut FxHashSet::default(), span);
        matches!(self.type_arena[resolved.0 as usize].kind, TypeKind::Never)
    }

    pub(in crate::lower) fn discharge_row(&mut self, row: &EffectRow, span: Span) {
        // Flatten any solved flexible tail so ops captured by a call-site row
        // variable (e.g. an instantiated `...e`) are discharged into the ambient
        // handler too, not just the ops written inline.
        let (ops, _tail) = self.flatten_effect_row(row.ops.clone(), row.tail);
        let row = &EffectRow {
            ops,
            tail: row.tail,
        };
        for op in &row.ops {
            match self.lookup_op(&op.name) {
                Some((param, result)) => {
                    self.unify(param, op.param, span);
                    self.unify(result, op.result, span);
                }
                None => self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::EffectNotInRow {
                        op: op.name.clone(),
                    },
                    span,
                }),
            }
        }
    }
}
