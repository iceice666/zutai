use super::*;

// ── v1 Constraint / Witness THIR representation (Increment 2) ────────────────

/// A constraint def + witness + normal binding all lower to a complete THIR file
/// with the expected structural presence.
#[test]
fn constraint_and_witness_produce_thir_decls() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = intEq; }\nintEq ::= \\a b. true;\n42";
    let file = completed_file(src);

    // Constraint decl is present.
    let cst = find_decl_kind(&file, |k| matches!(k, ThirDeclKind::Constraint { .. }))
        .expect("expected a ThirDeclKind::Constraint");
    let (n_methods, derivable) = match cst {
        ThirDeclKind::Constraint {
            methods, derivable, ..
        } => (methods.len(), *derivable),
        _ => unreachable!(),
    };
    assert_eq!(n_methods, 1, "Eq should have one method");
    assert!(!derivable, "Eq is not derivable in this source");

    // Witness decl is present.
    let wit = find_decl_kind(&file, |k| matches!(k, ThirDeclKind::Witness { .. }))
        .expect("expected a ThirDeclKind::Witness");
    let n_fields = match wit {
        ThirDeclKind::Witness { fields, .. } => fields.len(),
        _ => unreachable!(),
    };
    assert_eq!(n_fields, 1, "Eq @Int should have one field");
}

#[test]
fn witness_reflection_without_dictionary_emits_diagnostic() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\n(witness Eq @Int).eq";
    let lowered = lower(src);
    assert!(
        lowered.file.is_none(),
        "missing reflected witness should null LoweredThir.file"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::WitnessReflectNotInScope { constraint, target }
                if constraint == "Eq" && target == "Int"
        )),
        "expected WitnessReflectNotInScope; diagnostics: {:?}",
        lowered.diagnostics
    );
}

/// Constraint method sig resolves to a `Function { from: TypeVar, … }` — not Error.
/// This confirms the `BindingKind::TypeParam → TypeKind::TypeVar` path (types.rs:246).
#[test]
fn constraint_method_sig_resolves_to_typevar() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\n1";
    let file = completed_file(src);

    let cst = find_decl_kind(&file, |k| matches!(k, ThirDeclKind::Constraint { .. }))
        .expect("expected a ThirDeclKind::Constraint");
    let methods = match cst {
        ThirDeclKind::Constraint { methods, .. } => methods,
        _ => unreachable!(),
    };
    let sig_id = methods[0].sig;
    let sig_kind = &file.type_arena[sig_id.0 as usize].kind;
    // Outermost sig should be `A -> …`, i.e. `Function { from: TypeVar(_), to: … }`.
    assert!(
        matches!(sig_kind, TypeKind::Function { from, .. }
            if matches!(file.type_arena[from.0 as usize].kind, TypeKind::TypeVar(_))),
        "method sig `from` should be TypeVar, got {sig_kind:?}"
    );
}

/// Witness with a real lambda body (non-trivial `infer_expr`): file stays complete.
#[test]
fn witness_with_lambda_field_completes_thir() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. true; }\n1";
    let file = completed_file(src);

    let wit = find_decl_kind(&file, |k| matches!(k, ThirDeclKind::Witness { .. }))
        .expect("expected a ThirDeclKind::Witness");
    let ThirDeclKind::Witness { fields, .. } = wit else {
        unreachable!()
    };
    let field_expr = &file.expr_arena[fields[0].value];
    assert!(
        matches!(field_expr.kind, ThirExprKind::Lambda { .. }),
        "witness field should be a Lambda expr, got {:?}",
        field_expr.kind
    );
}

/// D5: a witness that forward-references a binding defined *after* it in source
/// order must still produce a complete THIR (no `ValueTypeUnavailable`).
#[test]
fn witness_forward_reference_completes_thir() {
    // `laterFn` is defined *after* the witness — tests the two-phase ordering.
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = laterFn; }\nlaterFn ::= \\a b. true;\n1";
    let lowered = lower(src);
    assert!(
        lowered.file.is_some(),
        "forward-reference in witness field should not null LoweredThir.file; diagnostics: {:?}",
        lowered.diagnostics
    );
}

/// A `derive` witness lowers to `ThirDeclKind::Witness { derive: true, fields: {} }`.
#[test]
fn derive_witness_lowers_with_derive_flag() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: derive\n1";
    let file = completed_file(src);

    let wit = find_decl_kind(&file, |k| matches!(k, ThirDeclKind::Witness { .. }))
        .expect("expected a ThirDeclKind::Witness");
    let (fields, derive) = match wit {
        ThirDeclKind::Witness { fields, derive, .. } => (fields.len(), *derive),
        _ => unreachable!(),
    };
    assert_eq!(fields, 0, "derive witness should have no explicit fields");
    assert!(derive, "derive witness should have derive=true");

    // Constraint should also carry derivable=true.
    let cst = find_decl_kind(&file, |k| matches!(k, ThirDeclKind::Constraint { .. }))
        .expect("expected a ThirDeclKind::Constraint");
    assert!(
        matches!(
            cst,
            ThirDeclKind::Constraint {
                derivable: true,
                ..
            }
        ),
        "constraint with 'derive' marker should have derivable=true"
    );
}

// ─── Increment 3: witness checking ───────────────────────────────────────────

/// Concrete-typed field passes checking (discriminator: proves substitution fires).
/// `realEq :: Int -> Int -> Bool` is fully concrete — no infer vars. The check
/// passes only if `{A → Int}` rewrites the method sig to `Int -> Int -> Bool`.
#[test]
fn witness_concrete_field_type_matches_passes() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = realEq; }\nrealEq :: Int -> Int -> Bool = \\a b. true;\n1";
    let lowered = lower(src);
    assert!(
        lowered.file.is_some(),
        "concrete-typed witness field should pass checking; diagnostics: {:?}",
        lowered.diagnostics
    );
}

/// Negative: field type does not match the expected method signature.
#[test]
fn witness_field_type_mismatch_emits_diagnostic() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = intEq; }\nintEq ::= 1;\n1";
    let lowered = lower(src);
    assert!(
        lowered.file.is_none(),
        "type-incorrect witness field should null LoweredThir.file"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::WitnessFieldTypeMismatch { name, .. } if name == "eq"
        )),
        "expected WitnessFieldTypeMismatch for `eq`; diagnostics: {:?}",
        lowered.diagnostics
    );
}

/// Negative: witness is missing a required method field.
/// Two-method constraint so only the missing `neq` fires; `eq` is provided correctly.
#[test]
fn witness_missing_required_field_emits_diagnostic() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; neq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. true; }\n1";
    let lowered = lower(src);
    assert!(
        lowered.file.is_none(),
        "witness missing required field should null LoweredThir.file"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::MissingWitnessField { name } if name == "neq"
        )),
        "expected MissingWitnessField for `neq`; diagnostics: {:?}",
        lowered.diagnostics
    );
    assert!(
        !lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::WitnessFieldTypeMismatch { name, .. } if name == "eq"
        )),
        "provided `eq` field should not trigger WitnessFieldTypeMismatch"
    );
}

/// Negative: witness provides a field that does not exist in the constraint.
/// One-method constraint; `eq` is provided correctly, `neq` is unknown.
#[test]
fn witness_unknown_field_emits_diagnostic() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. true; neq = \\a b. true; }\n1";
    let lowered = lower(src);
    assert!(
        lowered.file.is_none(),
        "witness with unknown field should null LoweredThir.file"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::UnknownWitnessField { name } if name == "neq"
        )),
        "expected UnknownWitnessField for `neq`; diagnostics: {:?}",
        lowered.diagnostics
    );
}

/// Derive witnesses have no explicit fields but still run derive-specific validation.
#[test]
fn derive_witness_keeps_no_explicit_fields() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: derive\n1";
    let lowered = lower(src);
    assert!(
        lowered.file.is_some(),
        "derive witness should produce a complete THIR; diagnostics: {:?}",
        lowered.diagnostics
    );
    assert!(
        !lowered.diagnostics.iter().any(|d| {
            matches!(
                &d.kind,
                ThirDiagnosticKind::MissingWitnessField { .. }
                    | ThirDiagnosticKind::WitnessFieldTypeMismatch { .. }
            )
        }),
        "derive witness should emit no ordinary field diagnostics; diagnostics: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn derive_witness_rejects_non_derivable_constraint() {
    let lowered = lower("Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: derive\n1");
    assert!(
        lowered.file.is_none(),
        "non-derivable constraint should reject derive witness"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::DeriveConstraintNotDerivable { constraint, .. } if constraint == "Eq"
        )),
        "expected DeriveConstraintNotDerivable; diagnostics: {:?}",
        lowered.diagnostics
    );
}

/// A derive diagnostic's secondary location resolves to the constraint's name
/// token when the constraint is declared in the same (entry) buffer, and is
/// suppressed when the constraint lives in a prelude/imported buffer whose spans
/// index a different source. Mirrors the request/definition split the macro
/// diagnostics render (primary span = derive request, secondary = definition).
#[test]
fn derive_diagnostic_related_location_points_at_local_constraint() {
    let src = "Ord :: <A> @A { compare :: A -> A -> Bool; } derive\nOrd @Int :: derive\n1";
    let lowered = lower(src);
    let diag = lowered
        .diagnostics
        .iter()
        .find(|d| matches!(&d.kind, ThirDiagnosticKind::DeriveUnsupportedMethod { .. }))
        .expect("expected DeriveUnsupportedMethod");
    // Primary span is the derive request on line 2, not the declaration.
    assert!(
        &src[diag.span.start as usize..diag.span.end as usize].starts_with("Ord @Int"),
        "primary span should cover the derive request, got {:?}",
        &src[diag.span.start as usize..diag.span.end as usize]
    );
    // Secondary location resolves to the constraint's name token on line 1.
    let (related, label) = diag
        .related_location_in(src)
        .expect("local constraint should yield a related definition location");
    assert_eq!(&src[related.start as usize..related.end as usize], "Ord");
    assert_eq!(label, "constraint defined here");
}

/// A constraint declared in a prelude (here `FromData`) shares the THIR decl
/// arena but carries spans into a different buffer; the content-verified guard
/// must suppress the secondary label rather than mislocate it into entry bytes.
#[test]
fn derive_diagnostic_related_location_suppressed_for_prelude_constraint() {
    let src = "Pair :: type (Int, Int);\nFromData @Pair :: derive\nvalue :: Validation DecodeIssue Pair = fromData (#int { value = 1; });\nvalue";
    let lowered = lower(src);
    let diag = lowered
        .diagnostics
        .iter()
        .find(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::DeriveRecipeTypeMismatch { constraint, .. } if constraint == "FromData"
        ))
        .expect("expected DeriveRecipeTypeMismatch for FromData");
    assert!(
        diag.related_location_in(src).is_none(),
        "a prelude constraint's definition span must not resolve against the entry buffer"
    );
}

#[test]
fn to_data_derive_rejects_unsupported_targets() {
    let cases = [
        (
            "Pair :: type (Int, Int);\nToData @Pair :: derive\n1",
            "Pair",
        ),
        (
            "Open :: type { x : Int; ...; };\nToData @Open :: derive\n1",
            "Open",
        ),
        (
            "Tree :: type { value : Int; children : List Tree; };\nToData @Tree :: derive\n1",
            "Tree",
        ),
        (
            "Wrapped :: type { value : Posit32; };\nToData @Wrapped :: derive\n1",
            "Wrapped",
        ),
        (
            "Wrapped :: type { value : Reader; };\nToData @Wrapped :: derive\n1",
            "Wrapped",
        ),
        (
            "Wrapped :: type { value : Int -> Int; };\nToData @Wrapped :: derive\n1",
            "Wrapped",
        ),
        (
            "Wrapped :: type { value : Type; };\nToData @Wrapped :: derive\n1",
            "Wrapped",
        ),
    ];
    for (src, target) in cases {
        let lowered = lower(src);
        assert!(
            lowered.diagnostics.iter().any(|diagnostic| matches!(
                &diagnostic.kind,
                ThirDiagnosticKind::DeriveRecipeTypeMismatch { constraint, .. }
                    | ThirDiagnosticKind::DeriveOpenRowTarget { constraint, .. }
                    if constraint == "ToData"
            )),
            "expected ToData refusal for {target}; diagnostics: {:?}",
            lowered.diagnostics
        );
    }
}

#[test]
fn derive_witness_requires_component_witness() {
    let lowered = lower(
        "Box :: type { value : Text; };\nPair :: type { box : Box; };\nEq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Pair :: derive\n1",
    );
    assert!(
        lowered.file.is_none(),
        "component without witness should reject derive witness"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::DeriveComponentMissingWitness { constraint, .. } if constraint == "Eq"
        )),
        "expected DeriveComponentMissingWitness; diagnostics: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn derive_witness_accepts_component_witness() {
    let file = completed_file(
        "Box :: type { value : Text; };\nPair :: type { box : Box; };\nEq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Box :: derive\nEq @Pair :: derive\n1",
    );
    let _ = file;
}

#[test]
fn derive_witness_rejects_non_equality_method() {
    // `compare` has no structural derivation recipe; deriving it must be refused.
    let lowered =
        lower("Ord :: <A> @A { compare :: A -> A -> Bool; } derive\nOrd @Int :: derive\n1");
    assert!(
        lowered.file.is_none(),
        "deriving a non-equality method should reject"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::DeriveUnsupportedMethod { constraint, method, .. }
                if constraint == "Ord" && method == "compare"
        )),
        "expected DeriveUnsupportedMethod for compare; diagnostics: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn derive_over_open_row_target_is_refused() {
    // A structural derive over an open record row is unsound (the hidden tail is
    // not enumerable), so it must be refused with a dedicated diagnostic rather
    // than synthesizing a witness over only the visible members.
    let lowered = lower(
        "Point :: type { x : Int; ...; };\nEq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Point :: derive\n0",
    );
    assert!(
        lowered.file.is_none(),
        "open-row derive target should null LoweredThir.file"
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::DeriveOpenRowTarget { constraint, .. } if constraint == "Eq"
        )),
        "expected DeriveOpenRowTarget for Eq; diagnostics: {:?}",
        lowered.diagnostics
    );
}

// ── Increment 4: coherence checking ──────────────────────────────────────────

#[test]
fn coherence_distinct_targets_pass() {
    // Two witnesses for the same constraint but different types should not conflict.
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: { eq = \\a b. true; }\nEq @Text :: { eq = \\a b. true; }\n1";
    // completed_file panics on any diagnostic — success means no conflict.
    let _file = completed_file(src);
}

#[test]
fn coherence_same_target_different_constraints_pass() {
    // Two witnesses for different constraints at the same type should not conflict.
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nOrd :: <A> @A { cmp :: A -> A -> Bool; } derive\nEq @Int :: { eq = \\a b. true; }\nOrd @Int :: { cmp = \\a b. true; }\n1";
    // completed_file panics on any diagnostic — success means no conflict.
    let _file = completed_file(src);
}

#[test]
fn coherence_duplicate_pair_emits_conflicting_witness() {
    // Two witnesses for the same (Constraint, Type) pair: the second must be rejected.
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: { eq = \\a b. true; }\nEq @Int :: { eq = \\a b. true; }\n1";
    let lowered = lower(src);
    assert!(
        lowered.file.is_none(),
        "duplicate witness pair should nullify LoweredThir.file; diagnostics: {:?}",
        lowered.diagnostics
    );
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(
                &d.kind,
                ThirDiagnosticKind::ConflictingWitness { constraint, target }
                    if constraint == "Eq" && target == "Int"
            )
        }),
        "expected ConflictingWitness{{constraint=Eq, target=Int}}; diagnostics: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn coherence_derive_and_explicit_same_pair_conflict() {
    // A `derive` witness and an explicit witness for the same pair must conflict.
    // Coherence includes derive witnesses (D12).
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: derive\nEq @Int :: { eq = \\a b. true; }\n1";
    let lowered = lower(src);
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| { matches!(&d.kind, ThirDiagnosticKind::ConflictingWitness { .. }) }),
        "derive + explicit witness for same pair should emit ConflictingWitness; diagnostics: {:?}",
        lowered.diagnostics
    );
}
#[test]
fn coherence_overlapping_conditional_witnesses_conflict() {
    // Two `Eq @(List A)` conditional witnesses overlap: param-normalized keys
    // collide and the second is reported ambiguous despite distinct param vars.
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @(List A) :: <A: Eq> { eq = \\xs ys. true; }\nEq @(List A) :: <A: Eq> { eq = \\xs ys. false; }\n1";
    let lowered = lower(src);
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(&d.kind, ThirDiagnosticKind::ConflictingWitness { constraint, .. } if constraint == "Eq")
        }),
        "overlapping conditional witnesses should emit ConflictingWitness; diagnostics: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn recursive_witness_target_is_own_param_rejected() {
    // `Eq @A :: <A: Eq>` — the target is the witness's own param, so resolving it
    // for any type needs a witness for the same type: non-terminating.
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @A :: <A: Eq> { eq = \\x y. true; }\n1";
    let lowered = lower(src);
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(&d.kind, ThirDiagnosticKind::RecursiveWitness { constraint } if constraint == "Eq")
        }),
        "self-referential conditional witness should emit RecursiveWitness; diagnostics: {:?}",
        lowered.diagnostics
    );
}
#[test]
fn witness_target_is_param_bounded_by_other_constraint_not_recursive() {
    // `Eq @A :: <A: Ord>` — the target is the param, but the bound is a *different*
    // constraint, so resolution consumes an `Ord` dict to produce an `Eq` dict and
    // makes progress. It must NOT be flagged as a recursive witness.
    let src = "Ord :: <A> @A { cmp :: A -> A -> Bool; }\nEq :: <A> @A { eq :: A -> A -> Bool; }\nEq @A :: <A: Ord> { eq = \\x y. true; }\n1";
    let lowered = lower(src);
    assert!(
        !lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::RecursiveWitness { .. })),
        "witness bounded by a different constraint should not emit RecursiveWitness; diagnostics: {:?}",
        lowered.diagnostics
    );
}
// ── Phase 14: higher-kinded constraints & method-level type params ───────────

#[test]
fn hkt_constraint_method_sig_checks() {
    // `F A` applies a higher-kinded type param; the method has its own `<A, B>`.
    let src = "Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\n1";
    let lowered = lower(src);
    assert!(
        lowered.diagnostics.is_empty(),
        "HKT constraint should check clean: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn method_level_type_params_preserved() {
    let src = "Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\n1";
    let file = completed_file(src);
    let has_params = file.decl_arena.iter().any(|(_, d)| {
        matches!(&d.kind, ThirDeclKind::Constraint { methods, .. }
            if methods.iter().any(|m| m.name == "map" && m.params.len() == 2))
    });
    assert!(
        has_params,
        "method `map` should keep its 2 method-level params in THIR"
    );
}

#[test]
fn functor_witness_and_polymorphic_use_checks() {
    let src = "Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nFunctor @List :: { map = \\f xs. xs; }\nmapTwice :: <F: Functor, A> (A -> A) -> F A -> F A\n  = f xs => map f (map f xs);\n1";
    let lowered = lower(src);
    assert!(
        lowered.diagnostics.is_empty(),
        "Functor witness + polymorphic use should check clean: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn partial_application_witness_target_checks() {
    // `Functor @(Result E)` — partial application of a 2-arg constructor yields a
    // `Type -> Type` witness target.
    let src = "Result :: <E, A> type { ok : A; err : E; };\nFunctor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nFunctor @(Result E) :: <E> { map = \\f r. r; }\n1";
    let lowered = lower(src);
    assert!(
        lowered.diagnostics.is_empty(),
        "partial-application witness target should check clean: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn witness_target_kind_mismatch_rejected() {
    // `Functor @Int` — `Int : Type` but `Functor` constrains a `Type -> Type`.
    let src = "Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nFunctor @Int :: { map = \\f x. x; }\n1";
    let lowered = lower(src);
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::WitnessTargetKindMismatch { constraint, .. } if constraint == "Functor"
        )),
        "expected WitnessTargetKindMismatch for `Functor @Int`; got {:?}",
        lowered.diagnostics
    );
}

// ── Multi-param constraint diagnostic (4c) ───────────────────────────────────

/// A constraint with two type params emits `UnsupportedMultiParamConstraint`
/// for any witness that targets it — no panic, no silent pass.
#[test]
fn multi_param_constraint_witness_emits_diagnostic() {
    // `Pair` has two type params `<A, B>` — witness checking is not supported.
    let src =
        "Pair :: <A, B> @A { pair :: A -> B -> Bool; }\nPair @Int :: { pair = \\a b. true; }\n1";
    let lowered = lower(src);
    assert!(
        lowered.file.is_none(),
        "multi-param constraint witness should nullify LoweredThir.file; diagnostics: {:?}",
        lowered.diagnostics
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::UnsupportedMultiParamConstraint { name } if name == "Pair"
        )),
        "expected UnsupportedMultiParamConstraint for `Pair`; diagnostics: {:?}",
        lowered.diagnostics
    );
}

// Increment 5: method-name resolution tests
// ---------------------------------------------------------------------------

/// T5-1: a named method call `eq 1 2` type-checks to Bool (positive, monomorphic).
#[test]
fn method_call_eq_int_typechecks_to_bool() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. true; }\neq 1 2";
    let file = completed_file(src);
    assert!(
        matches!(final_type_kind(&file), TypeKind::Bool),
        "expected final type Bool, got {:?}",
        final_type_kind(&file)
    );
}

/// T5-2: the method's TypeVar is instantiated independently at each call site
/// so `(eq 1 2, eq true false)` type-checks without mixing the two instances.
#[test]
fn method_call_polymorphic_independent_instantiation() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. true; }\nEq @Bool :: { eq = \\a b. true; }\n(eq 1 2, eq true false)";
    let file = completed_file(src);
    // The result is a 2-tuple; just check the file is complete.
    assert!(
        matches!(final_type_kind(&file), TypeKind::Tuple(_)),
        "expected Tuple type, got {:?}",
        final_type_kind(&file)
    );
}

/// T5-3: mismatched argument types `eq true 2` emit TypeMismatch and nullify the file.
#[test]
fn method_call_arg_type_mismatch_emits_diagnostic() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\neq true 2";
    let lowered = lower(src);
    assert!(
        lowered.file.is_none(),
        "type-mismatched call should produce no file"
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch diagnostic; got {:?}",
        lowered.diagnostics
    );
}

/// T5-6: a concrete named target with a witness in scope remains valid dispatch.
#[test]
fn method_call_show_with_witness_preserves_dispatch() {
    let file = completed_file(
        "Show :: <A> @A { show :: A -> Text; }\nShow @Text :: { show = \\s. s; }\nshow \"x\"",
    );
    assert!(
        matches!(final_type_kind(&file), TypeKind::Text),
        "show over Text should return Text, got {:?}",
        final_type_kind(&file)
    );
}

/// T5-7: the lowered `ThirConstraintMethod.binding` is `Some(_)` for a named method.
#[test]
fn thir_constraint_method_binding_is_some_for_named() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\n1";
    let file = completed_file(src);
    let cst = find_decl_kind(&file, |k| matches!(k, ThirDeclKind::Constraint { .. }))
        .expect("expected a ThirDeclKind::Constraint");
    let methods = match cst {
        ThirDeclKind::Constraint { methods, .. } => methods,
        _ => unreachable!(),
    };
    assert_eq!(methods.len(), 1, "expected one method");
    assert!(
        methods[0].binding.is_some(),
        "named method `eq` must have Some(binding), got None"
    );
}
