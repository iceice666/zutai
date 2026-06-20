use super::*;

// ---------------------------------------------------------------------------
// Constraint / witness HIR tests (v1)
// ---------------------------------------------------------------------------

/// H1: bound name resolves to the constraint binding
#[test]
fn h1_bound_resolves_to_constraint() {
    let lowered = lower_no_diag(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nOrd :: <A: Eq> @A { lt :: A -> A -> Bool; }\n1",
    );
    let eq_id = find_binding_by_name(&lowered.file, "Eq").expect("Eq binding");
    // Find the Ord constraint decl and check its param has Eq as a bound
    let ord_decl = lowered
        .file
        .decl_arena
        .iter()
        .map(|(_, d)| d)
        .find(|d| binding_name(&lowered.file, d.binding) == "Ord")
        .expect("Ord decl");
    if let HirDeclKind::Constraint { params, .. } = &ord_decl.kind {
        assert_eq!(params[0].bounds.len(), 1);
        assert_eq!(params[0].bounds[0], eq_id);
    } else {
        panic!("expected Constraint kind");
    }
}

/// H2: unknown bound produces UnknownIdentifier diagnostic
#[test]
fn h2_unknown_bound_produces_diagnostic() {
    let parsed = zutai_syntax::parse("Foo :: <A: Unknown> @A { f :: A -> A; }\n1");
    assert!(!parsed.has_errors());
    let lowered = lower_file(parsed.ast().unwrap());
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            HirDiagnosticKind::UnknownIdentifier { name } if name == "Unknown"
        )),
        "expected UnknownIdentifier for 'Unknown', got {:?}",
        lowered.diagnostics
    );
}

/// H3: type param A is scoped to the constraint decl and invisible outside
#[test]
fn h3_type_param_scoped_to_constraint() {
    let lowered = lower_no_diag(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = intEq; }\nintEq := 1\n1",
    );
    // Top-level `A` should NOT appear as a binding in the file's bindings
    let a_top = lowered
        .file
        .bindings
        .iter()
        .enumerate()
        .filter(|(_, b)| b.name == "A" && b.kind == BindingKind::TopValue)
        .count();
    assert_eq!(a_top, 0, "A should not be a top-level value binding");
}

/// H4: method-level params are scoped to the method
#[test]
fn h4_method_params_scoped() {
    let lowered = lower_no_diag("Conv :: <F> @F { convert :: <A, B> A -> F B; }\n1");
    let decl = lowered
        .file
        .decl_arena
        .iter()
        .map(|(_, d)| d)
        .find(|d| binding_name(&lowered.file, d.binding) == "Conv")
        .expect("Conv decl");
    if let HirDeclKind::Constraint { methods, .. } = &decl.kind {
        assert_eq!(methods[0].params.len(), 2);
    } else {
        panic!("expected Constraint kind");
    }
}

/// H5: duplicate constraint method produces DuplicateConstraintMethod diagnostic
#[test]
fn h5_duplicate_constraint_method() {
    let parsed =
        zutai_syntax::parse("Eq :: <A> @A { eq :: A -> A -> Bool; eq :: A -> A -> Bool; }\n1");
    assert!(!parsed.has_errors());
    let lowered = lower_file(parsed.ast().unwrap());
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            HirDiagnosticKind::DuplicateConstraintMethod { name, .. } if name == "eq"
        )),
        "expected DuplicateConstraintMethod, got {:?}",
        lowered.diagnostics
    );
}

/// H6: duplicate witness field produces DuplicateWitnessField diagnostic
#[test]
fn h6_duplicate_witness_field() {
    let parsed =
        zutai_syntax::parse("Eq @Int :: { eq = intEq; eq = intEq2; }\nintEq := 1\nintEq2 := 2\n1");
    assert!(!parsed.has_errors());
    let lowered = lower_file(parsed.ast().unwrap());
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            HirDiagnosticKind::DuplicateWitnessField { name, .. } if name == "eq"
        )),
        "expected DuplicateWitnessField, got {:?}",
        lowered.diagnostics
    );
}

/// H7: witness target `Int` resolves correctly
#[test]
fn h7_witness_target_resolves() {
    let lowered = lower_no_diag(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = intEq; }\nintEq := 1\n1",
    );
    let witness_decl = lowered
        .file
        .decl_arena
        .iter()
        .map(|(_, d)| d)
        .find(|d| lowered.file.bindings[d.binding.0 as usize].kind == BindingKind::TopWitness)
        .expect("witness decl");
    if let HirDeclKind::Witness { target, .. } = &witness_decl.kind {
        let ty = &lowered.file.type_arena[*target];
        assert!(
            matches!(&ty.kind, HirTypeKind::BindingRef(_)),
            "expected BindingRef for Int target, got {:?}",
            ty.kind
        );
    } else {
        panic!("expected Witness kind");
    }
}

/// H8: unknown witness constraint produces UnknownConstraint diagnostic
#[test]
fn h8_unknown_witness_constraint() {
    let parsed = zutai_syntax::parse("Nonexistent @Int :: { eq = 1; }\n1");
    assert!(!parsed.has_errors());
    let lowered = lower_file(parsed.ast().unwrap());
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            HirDiagnosticKind::UnknownConstraint { name } if name == "Nonexistent"
        )),
        "expected UnknownConstraint, got {:?}",
        lowered.diagnostics
    );
}

/// H9: two `Eq @Int` witnesses do NOT raise DuplicateBinding (coherence is out of scope)
#[test]
fn h9_two_witnesses_no_duplicate_binding() {
    let parsed = zutai_syntax::parse(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = f; }\nEq @Int :: { eq = g; }\nf := 1\ng := 2\n1",
    );
    assert!(!parsed.has_errors());
    let lowered = lower_file(parsed.ast().unwrap());
    let dup_bindings: Vec<_> = lowered
        .diagnostics
        .iter()
        .filter(
            |d| matches!(&d.kind, HirDiagnosticKind::DuplicateBinding { name, .. } if name == "Eq"),
        )
        .collect();
    assert!(
        dup_bindings.is_empty(),
        "two witnesses for Eq should not produce DuplicateBinding, got {:?}",
        dup_bindings
    );
}

/// H10: witness field RHS resolves against top scope
#[test]
fn h10_witness_field_rhs_resolves() {
    let lowered = lower_no_diag(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nintEq := 1\nEq @Int :: { eq = intEq; }\n1",
    );
    let witness_decl = lowered
        .file
        .decl_arena
        .iter()
        .map(|(_, d)| d)
        .find(|d| lowered.file.bindings[d.binding.0 as usize].kind == BindingKind::TopWitness)
        .expect("witness decl");
    if let HirDeclKind::Witness { fields, .. } = &witness_decl.kind {
        let field_expr = &lowered.file.expr_arena[fields[0].value];
        assert!(
            matches!(field_expr.kind, HirExprKind::BindingRef(_)),
            "expected field RHS to resolve to a binding, got {:?}",
            field_expr.kind
        );
    } else {
        panic!("expected Witness kind");
    }
}

// Increment 5: method-name resolution tests
// ---------------------------------------------------------------------------

/// H10: a named constraint method gets a `ConstraintMethod` binding in Pass 1.
#[test]
fn h10_named_method_gets_constraint_method_binding() {
    let lowered = lower_no_diag("Eq :: <A> @A { eq :: A -> A -> Bool; }\n1");
    let eq_decl = lowered
        .file
        .decl_arena
        .iter()
        .map(|(_, d)| d)
        .find(|d| lowered.file.bindings[d.binding.0 as usize].name == "Eq")
        .expect("Eq decl");
    let methods = match &eq_decl.kind {
        HirDeclKind::Constraint { methods, .. } => methods,
        other => panic!("expected Constraint kind, got {other:?}"),
    };
    assert_eq!(methods.len(), 1);
    let method = &methods[0];
    assert!(!method.is_operator, "eq should not be an operator");
    let binding = method.binding.expect("named method must have a binding");
    let b = &lowered.file.bindings[binding.0 as usize];
    assert_eq!(b.name, "eq");
    assert_eq!(b.kind, BindingKind::ConstraintMethod);
}

/// H11: a method name is resolvable in the final expression after constraint declaration.
#[test]
fn h11_method_name_resolves_as_binding_ref_in_final_expr() {
    let lowered = lower_no_diag("Eq :: <A> @A { eq :: A -> A -> Bool; }\neq");
    // final expr: BindingRef(eq_binding) — no UnresolvedIdent
    let final_expr = &lowered.file.expr_arena[lowered.file.final_expr];
    assert!(
        matches!(final_expr.kind, HirExprKind::BindingRef(_)),
        "final expr should be BindingRef(eq_binding), got {:?}",
        final_expr.kind
    );
}

/// H12: operator methods get `binding: None` (deferred to a later increment).
#[test]
fn h12_operator_method_gets_binding() {
    // D6/4b: operator methods now get an unscoped BindingId (previously None).
    // Operator methods use parenthesised operator syntax: `(==)`.
    let lowered = lower_no_diag("Eq :: <A> @A { (==) :: A -> A -> Bool; }\n1");
    let eq_decl = lowered
        .file
        .decl_arena
        .iter()
        .map(|(_, d)| d)
        .find(|d| lowered.file.bindings[d.binding.0 as usize].name == "Eq")
        .expect("Eq decl");
    let methods = match &eq_decl.kind {
        HirDeclKind::Constraint { methods, .. } => methods,
        other => panic!("expected Constraint kind, got {other:?}"),
    };
    assert_eq!(methods.len(), 1);
    let method = &methods[0];
    assert!(method.is_operator, "== should be an operator method");
    assert!(
        method.binding.is_some(),
        "operator method must have Some(binding) after D6/4b, got None"
    );
}
