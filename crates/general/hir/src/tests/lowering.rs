use super::*;

#[test]
fn reports_duplicate_top_level_binding_in_one_namespace() {
    let lowered = lower("Server :: type Text;\nServer ::= 1;\nServer");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateBinding { name, .. }) if name == "Server"
    ));
}

#[test]
fn normalizes_no_signature_function_to_function_decl() {
    let lowered = lower("double x = x * 2;\ndouble 2");
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
fn import_binding_lowers_to_value_with_import_expr() {
    // `import` is an expression: `lib ::= import "lib.zt"` is an ordinary
    // inferred value binding whose value is an `Import` expr.
    let lowered = lower("lib ::= import \"lib.zt\";\nlib");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let binding = find_binding_by_name(&lowered.file, "lib").expect("lib binding");
    assert_eq!(
        lowered.file.bindings[binding.0 as usize].kind,
        BindingKind::TopValue
    );

    let decl = &lowered.file.decl_arena[lowered.file.decls[0]];
    assert_eq!(decl.binding, binding);
    match &decl.kind {
        HirDeclKind::Value { annotation, value } => {
            assert!(annotation.is_none());
            assert!(matches!(
                &lowered.file.expr_arena[*value].kind,
                HirExprKind::Import(HirImportSource::String(source)) if source == "lib.zt"
            ));
        }
        other => panic!("expected value decl, got {other:?}"),
    }
}

#[test]
fn desugars_forward_pipeline_to_application() {
    let lowered = lower("f x = x;\n1 |> f");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Apply { func, arg } = expr.kind else {
        panic!("expected pipeline to become apply, got {:?}", expr.kind);
    };
    let func = &lowered.file.expr_arena[func];
    let arg = &lowered.file.expr_arena[arg];
    assert!(matches!(func.kind, HirExprKind::BindingRef(_)));
    assert!(matches!(arg.kind, HirExprKind::Integer(1, None)));
}

#[test]
fn desugars_backward_pipeline_to_application() {
    let lowered = lower("f x = x;\nf <| 1");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Apply { func, arg } = expr.kind else {
        panic!("expected pipeline to become apply, got {:?}", expr.kind);
    };
    let func = &lowered.file.expr_arena[func];
    let arg = &lowered.file.expr_arena[arg];
    assert!(matches!(func.kind, HirExprKind::BindingRef(_)));
    assert!(matches!(arg.kind, HirExprKind::Integer(1, None)));
}

#[test]
fn resolves_local_binding_only_after_its_value() {
    let lowered = lower("x ::= 1;\n[ x := x; x ]");
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
fn lowers_typed_local_binding_annotation() {
    let lowered = lower("[ x : Int = 1; x ]");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let block = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Block { bindings, .. } = &block.kind else {
        panic!("expected final block, got {:?}", block.kind);
    };
    let annotation = bindings[0]
        .annotation
        .expect("typed local should lower annotation");
    assert!(matches!(
        type_kind(&lowered.file, annotation),
        HirTypeKind::BindingRef(id) if binding_name(&lowered.file, *id) == "Int"
    ));
}

#[test]
fn lowers_tagged_value_ascription_to_block_with_type_annotation() {
    let lowered = lower("Status :: type { #err; };\n#err as Status");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Block { bindings, result } = &expr.kind else {
        panic!(
            "expected final expression to be a block, got {:?}",
            expr.kind
        );
    };
    assert_eq!(bindings.len(), 1);

    let annotation = bindings[0]
        .annotation
        .expect("tagged value ascription should lower annotated binding");
    assert!(matches!(
        type_kind(&lowered.file, annotation),
        HirTypeKind::BindingRef(id) if binding_name(&lowered.file, *id) == "Status"
    ));

    let value = &lowered.file.expr_arena[bindings[0].value];
    let HirExprKind::TaggedValue { tag, payload } = &value.kind else {
        panic!("expected tagged value payload in lowered ascription");
    };
    assert_eq!(tag, "err");
    assert!(matches!(
        &lowered.file.expr_arena[*payload].kind,
        HirExprKind::Tuple(items) if items.is_empty()
    ));

    assert!(matches!(
        &lowered.file.expr_arena[*result].kind,
        HirExprKind::BindingRef(id) if *id == bindings[0].binding
    ));
}

#[test]
fn lowers_tagged_value_ascription_with_record_payload() {
    let lowered = lower(
        r#"
Status :: type {
  #err: { code : Text; };
};
#err { code = "x"; } as Status
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Block { bindings, result } = &expr.kind else {
        panic!(
            "expected final expression to be a block, got {:?}",
            expr.kind
        );
    };
    assert_eq!(bindings.len(), 1);

    let annotation = bindings[0]
        .annotation
        .expect("tagged value ascription should lower annotated binding");
    assert!(matches!(
        type_kind(&lowered.file, annotation),
        HirTypeKind::BindingRef(id) if binding_name(&lowered.file, *id) == "Status"
    ));

    let value = &lowered.file.expr_arena[bindings[0].value];
    let HirExprKind::TaggedValue { tag, payload } = &value.kind else {
        panic!("expected tagged value payload in lowered ascription");
    };
    assert_eq!(tag, "err");
    assert!(matches!(
        &lowered.file.expr_arena[*payload].kind,
        HirExprKind::Record(_)
    ));

    assert!(matches!(
        &lowered.file.expr_arena[*result].kind,
        HirExprKind::BindingRef(id) if *id == bindings[0].binding
    ));
}

#[test]
fn lowers_use_identifier_binding() {
    let lowered = lower("use ::= 1;\nuse");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let binding = find_binding_by_name(&lowered.file, "use").expect("use binding");
    assert_eq!(lowered.file.decls.len(), 1);

    let decl = &lowered.file.decl_arena[lowered.file.decls[0]];
    assert_eq!(decl.binding, binding);
    match &decl.kind {
        HirDeclKind::Value { value, .. } => assert!(matches!(
            &lowered.file.expr_arena[*value].kind,
            HirExprKind::Integer(1, None)
        )),
        other => panic!("expected value decl, got {other:?}"),
    }
}

#[test]
fn lowers_interleaved_do_block_as_sequence_then_nested_block() {
    let lowered = lower("x ::= 1;\n[ x; y := x + 1; y ]");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);

    let block = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Block { bindings, result } = &block.kind else {
        panic!("expected final block, got {:?}", block.kind);
    };
    assert!(bindings.is_empty());

    let sequence = &lowered.file.expr_arena[*result];
    let HirExprKind::Sequence(items) = &sequence.kind else {
        panic!("expected sequence result, got {:?}", sequence.kind);
    };
    assert_eq!(items.len(), 2);
    let first_ref = match lowered.file.expr_arena[items[0]].kind {
        HirExprKind::BindingRef(id) => id,
        ref other => panic!("expected first statement ref, got {other:?}"),
    };
    assert_eq!(binding_name(&lowered.file, first_ref), "x");

    let nested = &lowered.file.expr_arena[items[1]];
    let HirExprKind::Block {
        bindings: nested_bindings,
        result: nested_result,
    } = &nested.kind
    else {
        panic!("expected nested block, got {:?}", nested.kind);
    };
    assert_eq!(nested_bindings.len(), 1);
    let nested_ref = match lowered.file.expr_arena[*nested_result].kind {
        HirExprKind::BindingRef(id) => id,
        ref other => panic!("expected nested result ref, got {other:?}"),
    };
    assert_eq!(nested_ref, nested_bindings[0].binding);
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
fn allows_duplicate_value_record_fields_for_later_wins() {
    let lowered = lower("{ a = 1; a = 2; }");

    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Record(items) = &expr.kind else {
        panic!("expected record, got {:?}", expr.kind);
    };
    assert_eq!(
        items
            .iter()
            .filter(|item| matches!(item, HirRecordItem::Field(_)))
            .count(),
        2
    );
}

#[test]
fn reports_duplicate_type_record_fields() {
    let lowered = lower("T :: type { a : Int; a : Text; };\nT");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateTypeRecordField { name, .. }) if name == "a"
    ));
}

#[test]
fn reports_duplicate_record_pattern_fields() {
    let lowered = lower("f x = match x { | { a = one; a = two; } => one; };\nf");

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
    let lowered = lower("T :: type (#point, x : Int, x : Float);\nT");

    assert!(matches!(
        lowered.diagnostics.first().map(|diagnostic| &diagnostic.kind),
        Some(HirDiagnosticKind::DuplicateTypeTupleField { name, .. }) if name == "x"
    ));
}

#[test]
fn reports_duplicate_named_tuple_pattern_fields() {
    let lowered = lower("f (#point, x = one, x = two) = one;\nf");

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
