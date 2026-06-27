use super::*;
use crate::ir::FixedWidth;
use zutai_syntax::posit::parse_posit_type_name;

impl<'hir> Lowerer<'hir> {
    pub(in crate::lower) fn lower_type_apply(
        &mut self,
        func: HirTypeId,
        arg: HirTypeId,
        span: Span,
    ) -> TypeId {
        // Walk the left-nested Apply spine to collect head + all args left-to-right.
        let mut args = vec![self.lower_predicative_type(arg)];
        let mut head = func;
        loop {
            let head_kind = self.hir_type(head).kind.clone();
            match head_kind {
                HirTypeKind::Apply { func: f, arg: a } => {
                    args.push(self.lower_predicative_type(a));
                    head = f;
                }
                _ => break,
            }
        }
        args.reverse();

        // `from_access` marks a head resolved from an imported constructor
        // (`s.Stream`): it reuses the named-alias path below but skips the
        // builtin-name fast path (a user field could shadow `List`).
        let (binding, from_access) = match self.hir_type(head).kind.clone() {
            HirTypeKind::BindingRef(binding) => (binding, false),
            HirTypeKind::Access { receiver, field } => {
                match self.imported_constructor_binding(receiver, &field) {
                    Some(ctor_binding) => (ctor_binding, true),
                    None => {
                        return self.invalid_type(
                            "type does not name an applicable parametric type constructor",
                            span,
                        );
                    }
                }
            }
            _ => return self.invalid_type("only named type constructors can be applied", span),
        };
        let name = self.binding_name(binding).to_string();

        // Built-in single-arg constructors keep existing handling and report
        // arity precisely instead of falling through to "not parametric".
        if !from_access {
            match name.as_str() {
                "List" | "Optional" | "Maybe" | "Patch" | "DeepPatch" => {
                    if args.len() != 1 {
                        self.diagnostics.push(ThirDiagnostic {
                            kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                                name,
                                expected: 1,
                                found: args.len(),
                            },
                            span,
                        });
                        return self.error_type;
                    }
                    return match name.as_str() {
                        "List" => self.alloc_type(Type {
                            kind: TypeKind::List(args[0]),
                            span,
                        }),
                        "Optional" => self.optional_type(args[0], span),
                        "Maybe" => self.maybe_type(args[0], span),
                        "Patch" => self.patch_type(args[0], false, span),
                        "DeepPatch" => self.patch_type(args[0], true, span),
                        _ => unreachable!(),
                    };
                }
                _ => {}
            }
        }

        // Named parametric alias (includes imported synthetic constructors).
        if let Some(params) = self.alias_params.get(&binding).cloned() {
            if args.len() > params.len() {
                // Over-application: more arguments than the constructor accepts.
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                        name,
                        expected: params.len(),
                        found: args.len(),
                    },
                    span,
                });
                return self.error_type;
            }
            if args.len() == params.len() {
                // Saturated: keep the direct-write `AliasApply` representation
                // (canonicalization-equivalent to the Apply-spine via `app_view`).
                return self.alloc_type(Type {
                    kind: TypeKind::AliasApply { binding, args },
                    span,
                });
            }
            // Partial application (`Result E`): curried `Apply` spine over the bare
            // alias head. `resolve_alias` leaves it inert until saturated.
            let head_ty = self.alias_type(binding, span);
            return self.fold_apply(head_ty, &args, span);
        }

        // Higher-kinded type-variable application (`F A`, F a type param of kind
        // `Type -> Type`). Curried `Apply` over the var head so it composes under
        // substitution (`F := Result E` makes `F A` reduce to `Result E A`).
        if matches!(self.binding_kind(binding), BindingKind::TypeParam) {
            let head_ty = self.alloc_type(Type {
                kind: TypeKind::TypeVar(binding),
                span,
            });
            return self.fold_apply(head_ty, &args, span);
        }

        self.invalid_type("type is not a parametric constructor", span)
    }

    /// Resolve a type-field access head (`s.Stream`) to the synthetic binding of
    /// the imported parametric constructor it names, or `None` if the receiver is
    /// not a simple import binding or the field is not an imported constructor.
    pub(in crate::lower) fn imported_constructor_binding(
        &self,
        receiver: HirTypeId,
        field: &str,
    ) -> Option<BindingId> {
        let HirTypeKind::BindingRef(receiver_binding) = self.hir_type(receiver).kind else {
            return None;
        };
        let source = self.binding_import_key.get(&receiver_binding)?;
        self.import_type_constructors
            .get(&(source.clone(), field.to_string()))
            .copied()
    }

    /// Build a curried `Apply` spine: `fold_apply(F, [A, B])` → `Apply{Apply{F,A},B}`.
    pub(in crate::lower) fn fold_apply(
        &mut self,
        head: TypeId,
        args: &[TypeId],
        span: Span,
    ) -> TypeId {
        let mut spine = head;
        for &arg in args {
            spine = self.alloc_type(Type {
                kind: TypeKind::Apply { func: spine, arg },
                span,
            });
        }
        spine
    }

    pub(in crate::lower) fn alias_or_builtin_type(
        &mut self,
        binding: BindingId,
        span: Span,
    ) -> TypeId {
        // A bare reference to a parametric constructor (without application) is
        // a zero-argument arity error. Check before the binding-kind match so
        // both TopType and TopFunction aliases can be caught here.
        if let Some(params) = self.alias_params.get(&binding).cloned() {
            let name = self.hir.bindings[binding.0 as usize].name.clone();
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::TypeConstructorArityMismatch {
                    name,
                    expected: params.len(),
                    found: 0,
                },
                span,
            });
            return self.error_type;
        }
        let binding_info = &self.hir.bindings[binding.0 as usize];
        match binding_info.kind {
            BindingKind::BuiltinType => match binding_info.name.as_str() {
                // Bare single-argument builtins (kind `Type -> Type`), used
                // unapplied as higher-kinded witness/constraint targets.
                "List" | "Optional" | "Maybe" | "Patch" | "DeepPatch" => self.alloc_type(Type {
                    kind: TypeKind::Con(binding),
                    span,
                }),
                name => self
                    .builtin_type_by_name(name, span)
                    .unwrap_or_else(|| self.invalid_type("unknown built-in type", span)),
            },
            BindingKind::TopType => self.alias_type(binding, span),
            // A type-valued import binding (`MyType ::= import "mytype.zt"`, whose
            // module's final expression is a type) is a plain value binding whose
            // alias denotation was registered in `predeclare_import_decls`. `import`
            // is an expression now, so there is no `TopImport` kind to match.
            BindingKind::TopValue if self.aliases.contains_key(&binding) => {
                self.alias_type(binding, span)
            }
            BindingKind::TypeParam => self.alloc_type(Type {
                kind: TypeKind::TypeVar(binding),
                span,
            }),
            BindingKind::Param | BindingKind::Local if self.type_param_scope.contains(&binding) => {
                // A `Param` or `Local` binding that was registered in
                // `type_param_scope` during type-level function body lowering
                // acts as a substitutable type variable.
                self.alloc_type(Type {
                    kind: TypeKind::TypeVar(binding),
                    span,
                })
            }
            _ => self.invalid_type("value binding used as a type", span),
        }
    }

    pub(in crate::lower) fn builtin_type_by_name(
        &mut self,
        name: &str,
        span: Span,
    ) -> Option<TypeId> {
        if let Some(spec) = parse_posit_type_name(name) {
            return Some(self.posit_type(spec, span));
        }

        let kind = match name {
            "Type" => TypeKind::Type(self.fresh_level_meta()),
            "Text" | "Path" | "Instant" => TypeKind::Text,
            "Unit" => TypeKind::Tuple(Vec::new()),
            "Bool" => TypeKind::Bool,
            "Int" | "i64" => TypeKind::Int,
            "Float" | "f64" => TypeKind::Float,
            "i8" => TypeKind::FixedNum(FixedWidth::I8),
            "i16" => TypeKind::FixedNum(FixedWidth::I16),
            "i32" => TypeKind::FixedNum(FixedWidth::I32),
            "u8" => TypeKind::FixedNum(FixedWidth::U8),
            "u16" => TypeKind::FixedNum(FixedWidth::U16),
            "u32" => TypeKind::FixedNum(FixedWidth::U32),
            "u64" => TypeKind::FixedNum(FixedWidth::U64),
            "f32" => TypeKind::FixedNum(FixedWidth::F32),
            "FsRead" | "FsWrite" | "Env" | "Clock" | "Rng" | "IoPrint" | "Load" => {
                TypeKind::Opaque(name.to_string())
            }
            _ => return None,
        };
        Some(self.alloc_type(Type { kind, span }))
    }
}
