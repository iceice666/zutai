use crate::*;

fn lower(src: &str) -> LoweredHir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    lower_file(parsed.ast().expect("parse should produce AST"))
}

fn lower_without_passes(src: &str) -> LoweredHir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    lower_file_with_options(
        parsed.ast().expect("parse should produce AST"),
        HirLowerOptions { run_passes: false },
    )
}

fn binding_name(file: &HirFile, id: BindingId) -> &str {
    &file.bindings[id.0 as usize].name
}

#[test]
fn reports_duplicate_top_level_binding_in_one_namespace() {
    let lowered = lower("Server :: type Text\nServer := 1\nServer");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateBinding { name, .. }) if name == "Server"
    ));
}

#[test]
fn normalizes_no_signature_function_to_function_decl() {
    let lowered = lower("double x = x * 2\ndouble 2");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let decl = &lowered.file.decl_arena[lowered.file.decls[0]];
    match &decl.kind {
        HirDeclKind::Function {
            params,
            sig,
            clauses,
        } => {
            assert!(params.is_empty());
            assert!(sig.is_none());
            assert_eq!(clauses.len(), 1);
            assert_eq!(clauses[0].patterns.len(), 1);
        }
        other => panic!("expected function decl, got {other:?}"),
    }
}

#[test]
fn desugars_forward_pipeline_to_application() {
    let lowered = lower("f x = x\n1 |> f");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Apply { func, arg } = expr.kind else {
        panic!("expected pipeline to become apply, got {:?}", expr.kind);
    };
    let func = &lowered.file.expr_arena[func];
    let arg = &lowered.file.expr_arena[arg];
    assert!(matches!(func.kind, HirExprKind::BindingRef(_)));
    assert!(matches!(arg.kind, HirExprKind::Integer(1)));
}

#[test]
fn desugars_backward_pipeline_to_application() {
    let lowered = lower("f x = x\nf <| 1");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Apply { func, arg } = expr.kind else {
        panic!("expected pipeline to become apply, got {:?}", expr.kind);
    };
    let func = &lowered.file.expr_arena[func];
    let arg = &lowered.file.expr_arena[arg];
    assert!(matches!(func.kind, HirExprKind::BindingRef(_)));
    assert!(matches!(arg.kind, HirExprKind::Integer(1)));
}

#[test]
fn resolves_local_binding_only_after_its_value() {
    let lowered = lower("x := 1\n{ x := x; x }");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let block = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Block { bindings, result } = &block.kind else {
        panic!("expected final block, got {:?}", block.kind);
    };
    let local = bindings[0].binding;
    let value_ref = match lowered.file.expr_arena[bindings[0].value].kind {
        HirExprKind::BindingRef(id) => id,
        ref other => panic!("expected local value ref, got {other:?}"),
    };
    let result_ref = match lowered.file.expr_arena[*result].kind {
        HirExprKind::BindingRef(id) => id,
        ref other => panic!("expected block result ref, got {other:?}"),
    };

    assert_eq!(binding_name(&lowered.file, value_ref), "x");
    assert_ne!(value_ref, local);
    assert_eq!(result_ref, local);
}

#[test]
fn resolves_function_type_params_in_signature_and_body_type_form() {
    let lowered = lower("id :: <A> A -> A\n  = x => type A;\nid");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let decl = &lowered.file.decl_arena[lowered.file.decls[0]];
    let HirDeclKind::Function {
        params,
        sig: Some(sig),
        clauses,
    } = &decl.kind
    else {
        panic!("expected generic function, got {decl:?}");
    };
    let type_param = params[0].binding;
    let sig = &lowered.file.type_arena[*sig];
    assert!(contains_type_binding(&lowered.file, sig, type_param));

    let body = &lowered.file.expr_arena[clauses[0].body];
    let HirExprKind::TypeForm(body_ty) = body.kind else {
        panic!("expected type form body, got {:?}", body.kind);
    };
    let body_ty = &lowered.file.type_arena[body_ty];
    assert_eq!(body_ty.kind, HirTypeKind::BindingRef(type_param));
}

#[test]
fn reports_duplicate_value_record_fields() {
    let lowered = lower("{ a = 1; a = 2; }");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateRecordField { name, .. }) if name == "a"
    ));
}

#[test]
fn reports_duplicate_type_record_fields() {
    let lowered = lower("T :: type { a : Int; a : Text; }\nT");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateTypeRecordField { name, .. }) if name == "a"
    ));
}

#[test]
fn reports_duplicate_record_pattern_fields() {
    let lowered = lower("f x = match x { | { a = one; a = two; } => one; }\nf");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateRecordPatternField { name, .. }) if name == "a"
    ));
}

#[test]
fn reports_duplicate_named_tuple_fields() {
    let lowered = lower("(#point, x = 1, x = 2)");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateTupleField { name, .. }) if name == "x"
    ));
}

#[test]
fn reports_duplicate_named_type_tuple_fields() {
    let lowered = lower("T :: type (#point, x : Int, x : Float)\nT");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateTypeTupleField { name, .. }) if name == "x"
    ));
}

#[test]
fn reports_duplicate_named_tuple_pattern_fields() {
    let lowered = lower("f (#point, x = one, x = two) = one\nf");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateTuplePatternField { name, .. }) if name == "x"
    ));
}

#[test]
fn ignores_positional_tuple_items_for_duplicate_key_validation() {
    let lowered = lower("(#point, 1, 2)");

    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn duplicate_key_pass_can_be_disabled() {
    let lowered = lower_without_passes("{ a = 1; a = 2; }");

    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    assert!(lowered.pass_reports.is_empty());
}

#[test]
fn runs_hir_passes_in_order() {
    struct MarkerPass(&'static str);

    impl HirPass for MarkerPass {
        fn name(&self) -> &'static str {
            self.0
        }

        fn run(&mut self, file: &mut HirFile, _diagnostics: &mut Vec<HirDiagnostic>) {
            file.span = file.span.merge(file.span);
        }
    }

    let mut lowered = lower("1");
    let mut diagnostics = Vec::new();
    let mut first = MarkerPass("first");
    let mut second = MarkerPass("second");
    let mut passes: [&mut dyn HirPass; 2] = [&mut first, &mut second];

    let reports = run_passes(&mut lowered.file, &mut diagnostics, &mut passes);

    assert_eq!(
        reports,
        vec![
            HirPassReport { name: "first" },
            HirPassReport { name: "second" }
        ]
    );
    assert!(diagnostics.is_empty());
}

// ---------------------------------------------------------------------------
// Constraint / witness HIR tests (v1)
// ---------------------------------------------------------------------------

fn lower_no_diag(src: &str) -> LoweredHir {
    let lowered = lower(src);
    assert!(
        lowered.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        lowered.diagnostics
    );
    lowered
}

fn find_binding_by_name(file: &HirFile, name: &str) -> Option<BindingId> {
    file.bindings
        .iter()
        .enumerate()
        .find(|(_, b)| b.name == name)
        .map(|(i, _)| BindingId(i as u32))
}

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

fn contains_type_binding(file: &HirFile, ty: &HirTypeExpr, binding: BindingId) -> bool {
    match &ty.kind {
        HirTypeKind::BindingRef(id) => *id == binding,
        HirTypeKind::Arrow { from, to } => {
            contains_type_binding(file, &file.type_arena[*from], binding)
                || contains_type_binding(file, &file.type_arena[*to], binding)
        }
        _ => false,
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

#[test]
fn h13_anonymous_row_tail_type_lowers_to_open_record() {
    let lowered = lower("T :: type { host : Text; ...; }\nT");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let decl = &lowered.file.decl_arena[lowered.file.decls[0]];
    let HirDeclKind::TypeAlias { ty, .. } = decl.kind else {
        panic!("expected TypeAlias, got {:?}", decl.kind);
    };
    let ty = &lowered.file.type_arena[ty];
    let HirTypeKind::Record { fields, tail } = &ty.kind else {
        panic!("expected Record, got {:?}", ty.kind);
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "host");
    assert!(matches!(
        tail,
        Some(HirRowTail {
            kind: HirRowTailKind::Anonymous,
            ..
        })
    ));
}

// ── Phase 8: v1 HIR lowering ────────────────────────────────────────────────

fn type_kind(file: &HirFile, id: HirTypeId) -> &HirTypeKind {
    &file.type_arena[id].kind
}

#[test]
fn named_row_tail_resolves_to_type_param_as_var() {
    let lowered = lower("f :: <Rest> { host : Text; ...Rest; } -> Text\n  = x => \"ok\";\nf");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let decl = &lowered.file.decl_arena[lowered.file.decls[0]];
    let HirDeclKind::Function { sig: Some(sig), .. } = &decl.kind else {
        panic!("expected Function, got {:?}", decl.kind);
    };
    let HirTypeKind::Arrow { from, .. } = type_kind(&lowered.file, *sig) else {
        panic!("expected Arrow sig");
    };
    let HirTypeKind::Record {
        tail: Some(tail), ..
    } = type_kind(&lowered.file, *from)
    else {
        panic!("expected open record param");
    };
    assert!(
        matches!(tail.kind, HirRowTailKind::Var(_)),
        "in-scope type param must lower to a row variable, got {:?}",
        tail.kind
    );
}

#[test]
fn named_union_row_tail_resolves_to_type_alias_as_spread() {
    let lowered = lower("Shape :: type { #dev; #test; }\nOpen :: type { ...Shape; #prod; }\nOpen");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let decl = &lowered.file.decl_arena[lowered.file.decls[1]];
    let HirDeclKind::TypeAlias { ty, .. } = decl.kind else {
        panic!("expected TypeAlias, got {:?}", decl.kind);
    };
    let HirTypeKind::Union {
        variants,
        tail: Some(tail),
    } = type_kind(&lowered.file, ty)
    else {
        panic!("expected open union");
    };
    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].name, "prod");
    assert!(
        matches!(tail.kind, HirRowTailKind::Spread(_)),
        "named type spread must lower to a spread tail, got {:?}",
        tail.kind
    );
}

#[test]
fn row_tail_naming_a_value_is_an_invalid_target() {
    let lowered = lower("x := 1\nT :: type { a : Int; ...x; }\nT");
    assert!(
        lowered.diagnostics.iter().any(
            |d| matches!(&d.kind, HirDiagnosticKind::InvalidRowTailTarget { name } if name == "x")
        ),
        "{:?}",
        lowered.diagnostics
    );
}

#[test]
fn unknown_row_tail_name_is_an_unknown_identifier() {
    let lowered = lower("T :: type { a : Int; ...Unknown; }\nT");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, HirDiagnosticKind::UnknownIdentifier { name } if name == "Unknown")),
        "{:?}",
        lowered.diagnostics
    );
}

#[test]
fn value_select_preserves_field_order() {
    let lowered = lower("s := { a = 1; b = 2; c = 3; }\nselect s { c; a; }");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Select { fields, .. } = &expr.kind else {
        panic!("expected Select, got {:?}", expr.kind);
    };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["c", "a"]);
}

#[test]
fn duplicate_select_field_is_reported() {
    let lowered = lower("s := { a = 1; }\nselect s { a; a; }");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, HirDiagnosticKind::DuplicateSelectField { name, .. } if name == "a")),
        "{:?}",
        lowered.diagnostics
    );
}

#[test]
fn type_level_select_lowers_with_field_order() {
    let lowered = lower(
        "Server :: type { host : Text; port : Int; }\nT :: type select Server { port; host; }\nT",
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let decl = &lowered.file.decl_arena[lowered.file.decls[1]];
    let HirDeclKind::TypeAlias { ty, .. } = decl.kind else {
        panic!("expected TypeAlias, got {:?}", decl.kind);
    };
    let HirTypeKind::Select { fields, .. } = type_kind(&lowered.file, ty) else {
        panic!("expected type-level Select");
    };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["port", "host"]);
}

#[test]
fn handle_distinguishes_value_and_operation_clauses() {
    let lowered = lower("d := 1\nhandle d with {\n  value = \\v. v;\n  fail = \\e. e;\n}");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Handle { clauses, .. } = &expr.kind else {
        panic!("expected Handle, got {:?}", expr.kind);
    };
    assert_eq!(clauses.len(), 2);
    assert!(matches!(clauses[0].op, HirHandleOp::Value));
    assert!(
        matches!(&clauses[1].op, HirHandleOp::Operation(path) if path == &vec!["fail".to_string()])
    );
}

#[test]
fn resume_inside_operation_clause_is_allowed() {
    let lowered = lower("d := 1\nhandle d with {\n  warn = \\x. resume x;\n}");
    assert!(
        !lowered
            .diagnostics
            .iter()
            .any(|d| matches!(d.kind, HirDiagnosticKind::ResumeOutsideHandler)),
        "resume in an operation clause must be accepted: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn resume_inside_value_clause_is_rejected() {
    let lowered = lower("d := 1\nhandle d with {\n  value = \\v. resume v;\n}");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(d.kind, HirDiagnosticKind::ResumeOutsideHandler)),
        "resume in a value clause must be rejected: {:?}",
        lowered.diagnostics
    );
}

#[test]
fn resume_at_top_level_is_rejected() {
    let lowered = lower("resume 1");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(d.kind, HirDiagnosticKind::ResumeOutsideHandler)),
        "{:?}",
        lowered.diagnostics
    );
}

#[test]
fn effect_row_lowers_to_effect_type() {
    let lowered = lower(
        "Config :: type Text\nParseError :: type Text\nparse :: Text -> Config ! { fail ParseError }\n  = text => text;\nparse",
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let decl = &lowered.file.decl_arena[lowered.file.decls[2]];
    let HirDeclKind::Function { sig: Some(sig), .. } = &decl.kind else {
        panic!("expected Function, got {:?}", decl.kind);
    };
    let HirTypeKind::Arrow { to, .. } = type_kind(&lowered.file, *sig) else {
        panic!("expected Arrow sig");
    };
    let HirTypeKind::Effect { row, .. } = type_kind(&lowered.file, *to) else {
        panic!("expected effectful result type");
    };
    assert_eq!(row.ops.len(), 1);
    assert_eq!(row.ops[0].path, vec!["fail".to_string()]);
    assert!(row.ops[0].payload.is_some());
}

#[test]
fn duplicate_explicit_field_in_open_record_is_reported() {
    let lowered = lower("T :: type { a : Int; a : Text; ...; }\nT");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, HirDiagnosticKind::DuplicateTypeRecordField { name, .. } if name == "a")),
        "{:?}",
        lowered.diagnostics
    );
}
