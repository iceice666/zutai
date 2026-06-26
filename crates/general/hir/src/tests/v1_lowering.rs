use super::*;

#[test]
fn h13_anonymous_row_tail_type_lowers_to_open_record() {
    let lowered = lower("T :: type { host : Text; ...; };\nT");
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
fn effect_row_tail_resolves_to_type_param_as_var() {
    // An effect-row tail `...e` naming an in-scope type parameter lowers to a row
    // variable, exactly like a record/union row tail. This is the foundation for
    // effect-row-polymorphic annotations (e.g. an ergonomic effectful-stream type).
    let lowered = lower("f :: <e> (Int ! { ...e; }) -> Int\n  = x => x;\nf");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let decl = &lowered.file.decl_arena[lowered.file.decls[0]];
    let HirDeclKind::Function { sig: Some(sig), .. } = &decl.kind else {
        panic!("expected Function, got {:?}", decl.kind);
    };
    let HirTypeKind::Arrow { from, .. } = type_kind(&lowered.file, *sig) else {
        panic!("expected Arrow sig");
    };
    let HirTypeKind::Effect { row, .. } = type_kind(&lowered.file, *from) else {
        panic!("expected an effectful parameter type");
    };
    let tail = row.tail.as_ref().expect("expected an open effect-row tail");
    assert!(
        matches!(tail.kind, HirRowTailKind::Var(_)),
        "in-scope type param must lower to a row variable, got {:?}",
        tail.kind
    );
}

#[test]
fn anonymous_effect_row_tail_lowers_to_open() {
    let lowered = lower("f :: (Int ! { ...; }) -> Int\n  = x => x;\nf");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let decl = &lowered.file.decl_arena[lowered.file.decls[0]];
    let HirDeclKind::Function { sig: Some(sig), .. } = &decl.kind else {
        panic!("expected Function, got {:?}", decl.kind);
    };
    let HirTypeKind::Arrow { from, .. } = type_kind(&lowered.file, *sig) else {
        panic!("expected Arrow sig");
    };
    let HirTypeKind::Effect { row, .. } = type_kind(&lowered.file, *from) else {
        panic!("expected an effectful parameter type");
    };
    assert!(matches!(
        row.tail.as_ref().map(|t| &t.kind),
        Some(HirRowTailKind::Anonymous)
    ));
}

#[test]
fn named_union_row_tail_resolves_to_type_alias_as_spread() {
    let lowered =
        lower("Shape :: type { #dev; #test; };\nOpen :: type { ...Shape; #prod; };\nOpen");
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
    let lowered = lower("x ::= 1;\nT :: type { a : Int; ...x; };\nT");
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
    let lowered = lower("T :: type { a : Int; ...Unknown; };\nT");
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
    let lowered = lower("s ::= { a = 1; b = 2; c = 3; };\nselect s { c; a; }");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::Select { fields, .. } = &expr.kind else {
        panic!("expected Select, got {:?}", expr.kind);
    };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["c", "a"]);
}

#[test]
fn record_update_preserves_receiver_and_field_order() {
    let lowered = lower("s ::= { a = 1; b = 2; c = 3; };\ns with { c = 30; a = 10; }");
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    let expr = &lowered.file.expr_arena[lowered.file.final_expr];
    let HirExprKind::RecordUpdate { receiver, fields } = &expr.kind else {
        panic!("expected RecordUpdate, got {:?}", expr.kind);
    };
    assert!(matches!(
        lowered.file.expr_arena[*receiver].kind,
        HirExprKind::BindingRef(_)
    ));
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["c", "a"]);
}

#[test]
fn duplicate_record_update_field_is_reported() {
    let lowered = lower("s ::= { a = 1; };\ns with { a = 2; a = 3; }");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, HirDiagnosticKind::DuplicateRecordField { name, .. } if name == "a")),
        "{:?}",
        lowered.diagnostics
    );
}

#[test]
fn duplicate_select_field_is_reported() {
    let lowered = lower("s ::= { a = 1; };\nselect s { a; a; }");
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
        "Server :: type { host : Text; port : Int; };\nT :: type select Server { port; host; };\nT",
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
    let lowered = lower("d ::= 1;\nhandle d with {\n  value = \\v. v;\n  fail = \\e. e;\n}");
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
    let lowered = lower("d ::= 1;\nhandle d with {\n  warn = \\x. resume x;\n}");
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
    let lowered = lower("d ::= 1;\nhandle d with {\n  value = \\v. resume v;\n}");
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
        "Config :: type Text;\nParseError :: type Text;\nparse :: Text -> Config ! { fail ParseError; }\n  = text => text;\nparse",
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
    let lowered = lower("T :: type { a : Int; a : Text; ...; };\nT");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, HirDiagnosticKind::DuplicateTypeRecordField { name, .. } if name == "a")),
        "{:?}",
        lowered.diagnostics
    );
}
