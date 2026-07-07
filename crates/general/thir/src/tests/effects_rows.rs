use super::*;

// ── thir/lower/types.rs: free_infer_vars_into Union/Tuple/Record arms ────────

/// An inferred function with a union-returning expression causes
/// `free_infer_vars_into` to traverse the Union arm during generalization.
#[test]
fn free_infer_vars_union_arm_via_inferred_fn() {
    // `choose` returns one of two union variants — its type contains a Union.
    // During generalization, free_infer_vars_into traverses the Union body.
    let lowered = lower(
        r#"
Color :: type { #red; #blue; };
choose b = if b then #red else #blue;
choose true
"#,
    );
    let _ = lowered;
}

/// An inferred lambda returning a record exercises `free_infer_vars_into`
/// Record arm during HM generalization, and `instantiate_infer_vars` Record
/// arm when instantiating the poly function at the call site.
/// Uses `:=` with a lambda body so the record is lowered in *infer* mode
/// (not check mode), avoiding the ExpectedRecord diagnostic that occurs when
/// THIR sees a record literal in check mode against an unresolved InferVar.
#[test]
fn free_infer_vars_record_arm_via_inferred_fn() {
    let file = completed_file(
        r#"
make_pair ::= \x. { first = x; second = 0; };
make_pair 42
"#,
    );
    let _ = file;
}

// ── D6: operator-method bindings + default bodies ────────────────────────────

/// D6/4b: an operator method in a constraint lowers to a `ThirConstraintMethod`
/// with `binding == Some(_)` (non-sentinel BindingId).
#[test]
fn operator_method_gets_binding_in_thir() {
    // Constraint with one operator method `(==)`.
    let src = "Eq :: <A> @A { (==) :: A -> A -> Bool; }\n1";
    let file = completed_file(src);
    let cst = find_decl_kind(&file, |k| matches!(k, ThirDeclKind::Constraint { .. }))
        .expect("expected ThirDeclKind::Constraint");
    let methods = match cst {
        ThirDeclKind::Constraint { methods, .. } => methods,
        _ => unreachable!(),
    };
    assert_eq!(methods.len(), 1, "expected one operator method");
    assert!(
        methods[0].is_operator,
        "method should be flagged as operator"
    );
    assert!(
        methods[0].binding.is_some(),
        "operator method must have Some(binding) after D6/4b, got None"
    );
}

/// D6/4a: a constraint method with a default body lowers to a
/// `ThirConstraintMethod` with `default == Some(clauses)` containing at least
/// one clause.
#[test]
fn constraint_method_default_body_lowered_to_thir() {
    // A non-optional method with a default clause body.
    // The body `= _ _ => true;` typechecks against `A -> A -> Bool`.
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool = _ _ => true; }\n1";
    let file = completed_file(src);
    let cst = find_decl_kind(&file, |k| matches!(k, ThirDeclKind::Constraint { .. }))
        .expect("expected ThirDeclKind::Constraint");
    let methods = match cst {
        ThirDeclKind::Constraint { methods, .. } => methods,
        _ => unreachable!(),
    };
    assert_eq!(methods.len(), 1, "expected one method");
    assert!(
        methods[0].default.is_some(),
        "method with default body must have Some(default) in THIR, got None"
    );
    let clauses = methods[0].default.as_ref().unwrap();
    assert!(
        !clauses.is_empty(),
        "default body must contain at least one clause"
    );
}

/// D6/4a: a witness that omits a non-optional method which has a default body
/// must NOT emit `MissingWitnessField`.  This is distinct from the `optional`
/// path: the method has no `?`, but the compiler-supplied default means the
/// witness is still valid when the field is absent.
#[test]
fn witness_omitting_method_with_default_body_no_missing_field_diagnostic() {
    // `eq` is non-optional but has a default body.  The witness omits `eq`.
    let src = r#"
Eq :: <A> @A {
  eq :: A -> A -> Bool
    = _ _ => true;
}
Eq @Int :: {}
1
"#;
    let lowered = lower(src);
    // File must be produced (no error should nullify it).
    assert!(
        lowered.file.is_some(),
        "witness omitting a method with a default body should not nullify the file; \
         diagnostics: {:?}",
        lowered.diagnostics
    );
    // Specifically, no MissingWitnessField for `eq`.
    assert!(
        !lowered.diagnostics.iter().any(
            |d| matches!(&d.kind, ThirDiagnosticKind::MissingWitnessField { name } if name == "eq")
        ),
        "MissingWitnessField for `eq` must not be emitted when the method has a default body; \
         diagnostics: {:?}",
        lowered.diagnostics
    );
}

/// Task 3: A function with a bounded type param records the bound's BindingId in
/// `param_bounds[0]`.
#[test]
fn function_type_param_bounds_are_recorded_in_thir() {
    let file = completed_file(
        r#"
Eq :: <A> @A {
  eq :: A -> A -> Bool;
}
same :: <A: Eq> A -> A -> A
  = x _ => x;
same
"#,
    );

    // Find the `same` function decl.
    let same_decl = file
        .decl_arena
        .iter()
        .map(|(_, d)| d)
        .find(|d| {
            matches!(
                &d.kind,
                ThirDeclKind::Function { params, .. } if !params.is_empty()
            )
        })
        .expect("same function decl should exist");

    let ThirDeclKind::Function { param_bounds, .. } = &same_decl.kind else {
        panic!("expected Function decl");
    };

    assert_eq!(
        param_bounds.len(),
        1,
        "one type param → one param_bounds entry"
    );
    assert!(
        !param_bounds[0].is_empty(),
        "type param A has bound Eq so param_bounds[0] must be non-empty"
    );
}

/// Task 3: An unconstrained type param produces an empty `param_bounds` entry.
#[test]
fn function_type_param_without_bounds_has_empty_param_bounds() {
    let file = completed_file(
        r#"
id :: <A> A -> A
  = x => x;
id
"#,
    );

    let id_decl = file
        .decl_arena
        .iter()
        .map(|(_, d)| d)
        .find(|d| {
            matches!(
                &d.kind,
                ThirDeclKind::Function { params, .. } if !params.is_empty()
            )
        })
        .expect("id function decl should exist");

    let ThirDeclKind::Function { param_bounds, .. } = &id_decl.kind else {
        panic!("expected Function decl");
    };

    assert_eq!(
        param_bounds.len(),
        1,
        "one type param → one param_bounds entry"
    );
    assert!(
        param_bounds[0].is_empty(),
        "unconstrained type param A should produce an empty bounds list"
    );
}

// ── Phase 15: effect typing (check-only) ─────────────────────────────────────

fn rejects_with(src: &str, pred: impl Fn(&ThirDiagnosticKind) -> bool) {
    let lowered = lower(src);
    assert!(
        lowered.diagnostics.iter().any(|d| pred(&d.kind)),
        "expected diagnostic for {src:?}, got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn effectful_function_perform_fail_checks() {
    let file = completed_file(
        r#"
Config :: type { value : Text; };
ParseError :: type Text;
parse :: Text -> Config ! { fail ParseError; }
  = text => perform fail text;
parse
"#,
    );
    let TypeKind::Function { to, .. } = final_type_kind(&file) else {
        panic!("expected parse to have function type");
    };
    let TypeKind::Effect { row, .. } = &file.type_arena[to.0 as usize].kind else {
        panic!(
            "expected effectful function result, got {:?}",
            file.type_arena[to.0 as usize].kind
        );
    };
    let fail = row.find("fail").expect("fail op in row");
    assert!(matches!(
        file.type_arena[fail.result.0 as usize].kind,
        TypeKind::Never
    ));
}

#[test]
fn effect_alias_return_is_discharged_at_call_site() {
    rejects_with(
        r#"
Config :: type { value : Text; };
EffConfig :: type Config ! { fail Text; };
parse :: Text -> EffConfig
  = text => perform fail text;
parse "bad"
"#,
        |kind| matches!(kind, ThirDiagnosticKind::EffectNotInRow { op } if op == "fail"),
    );
}

#[test]
fn if_inference_uses_else_type_when_then_branch_is_never() {
    completed_file(
        r#"
choose :: Bool -> Text ! { fail Text; }
  = b => [
    x := if b then perform fail "bad" else "ok";
    x
  ];
choose
"#,
    );
}

#[test]
fn match_inference_uses_later_type_when_first_arm_is_never() {
    completed_file(
        r#"
Status :: type { #bad; #good; };
choose :: Status -> Text ! { fail Text; }
  = s => [
    x := match s {
      | #bad => perform fail "bad";
      | #good => "ok";
    };
    x
  ];
choose
"#,
    );
}

#[test]
fn handler_forwards_unhandled_effects_and_resumes() {
    completed_file(
        r#"
Diagnostic :: type Text;
Config :: type { value : Text; };
check :: Config -> Config ! { warn Diagnostic; }
  = cfg => [ perform warn cfg.value; cfg ];
handleWarn :: Config -> Config ! { log Diagnostic; }
  = cfg => handle check cfg with { warn = \d. [ perform log d; resume () ]; };
handleWarn
"#,
    );
}

#[test]
fn nested_handler_resume_does_not_count_against_outer_handler() {
    completed_file(
        r#"
Diagnostic :: type Text;
Config :: type { value : Text; };
check :: Config -> Config ! { warn Diagnostic; }
  = cfg => [ perform warn cfg.value; cfg ];
nested :: Config -> Config
  = cfg => handle check cfg with { warn = \d. [ handle check cfg with { warn = \x. resume (); }; resume () ]; };
nested
"#,
    );
}

#[test]
fn explicit_standard_named_effect_signature_drives_resume_type() {
    completed_file(
        r#"
f :: Text -> Int ! { fail : Text -> Int; }
  = text => perform fail text;
handled :: Text -> Int
  = text => handle f text with { fail = \e. resume 1; };
handled
"#,
    );
}

#[test]
fn direct_standard_warn_handler_uses_unit_resume_type() {
    rejects_with(
        r#"
bad :: Text -> Text
  = d => handle [ perform warn d; "ok" ] with { warn = \x. resume 42; };
bad
"#,
        |kind| {
            matches!(kind, ThirDiagnosticKind::ResumeTypeMismatch { expected, found }
            if expected == "tuple" && found == "Int")
        },
    );
}

#[test]
fn dotted_capability_effect_checks() {
    completed_file(
        r#"
IOError :: type Text;
load :: FsRead -> Path -> Text ! { fs.read : Path -> Text; fail IOError; }
  = fs path => perform fs.read path;
load
"#,
    );
}

#[test]
fn standard_host_capability_types_and_ops_typecheck() {
    completed_file(
        r#"
WriteRequest :: type { path : Path; contents : Text; };
readConfig :: FsRead -> Path -> Text ! { fs.read : Path -> Text; }
  = fs path => perform fs.read path;
lookup :: Env -> Text -> Text? ! { env.get : Text -> Text?; }
  = env name => perform env.get name;
timestamp :: Clock -> Unit -> Instant ! { clock.now : Unit -> Instant; }
  = clock tick => perform clock.now tick;
randomInt :: Rng -> Unit -> Int ! { rng.next : Unit -> Int; }
  = rng tick => perform rng.next tick;
save :: FsWrite -> WriteRequest -> Unit ! { fs.write : WriteRequest -> Unit; }
  = fs req => perform fs.write req;
readConfig
"#,
    );
}

#[test]
fn scoped_reader_writer_host_ops_typecheck() {
    completed_file(
        r#"
WriteTextRequest :: type { contents : Text; writer : Writer; };
openRead :: FsRead -> Path -> Reader ! { fs.openRead : Path -> Reader; }
  = fs path => perform fs.openRead path;
readLine :: FsRead -> Reader -> Text? ! { fs.readLine : Reader -> Text?; }
  = fs reader => perform fs.readLine reader;
closeRead :: FsRead -> Reader -> Unit ! { fs.closeRead : Reader -> Unit; }
  = fs reader => perform fs.closeRead reader;
openWrite :: FsWrite -> Path -> Writer ! { fs.openWrite : Path -> Writer; }
  = fs path => perform fs.openWrite path;
writeText :: FsWrite -> WriteTextRequest -> Unit ! { fs.writeText : WriteTextRequest -> Unit; }
  = fs req => perform fs.writeText req;
flush :: FsWrite -> Writer -> Unit ! { fs.flush : Writer -> Unit; }
  = fs writer => perform fs.flush writer;
closeWrite :: FsWrite -> Writer -> Unit ! { fs.closeWrite : Writer -> Unit; }
  = fs writer => perform fs.closeWrite writer;
openRead
"#,
    );
}

#[test]
fn standard_host_ops_still_require_declared_function_effect_rows() {
    rejects_with(
        r#"
bad :: Path -> Text ! {}
  = path => perform fs.read path;
bad
"#,
        |kind| matches!(kind, ThirDiagnosticKind::EffectNotInRow { op } if op == "fs.read"),
    );
}

#[test]
fn scoped_host_ops_still_require_declared_function_effect_rows() {
    rejects_with(
        r#"
bad :: Path -> Reader ! {}
  = path => perform fs.openRead path;
bad
"#,
        |kind| matches!(kind, ThirDiagnosticKind::EffectNotInRow { op } if op == "fs.openRead"),
    );
}

#[test]
fn top_level_io_print_effect_is_allowed_at_host_boundary() {
    completed_file(r#"perform io.print "hello""#);
}

#[test]
fn effectful_non_function_top_level_value_is_rejected() {
    rejects_with(
        r#"
x :: Text ! { io.print : Text -> Text; } = print "hi";
1
"#,
        |kind| {
            matches!(kind, ThirDiagnosticKind::UnsupportedFeature { feature }
            if *feature == "effectful top-level value bindings")
        },
    );
}

#[test]
fn effectful_function_value_binding_remains_inert_until_called() {
    completed_file(
        r#"
f :: Text -> Text ! { io.print : Text -> Text; } = \text. print text;
f
"#,
    );
}

#[test]
fn perform_in_pure_function_reports_effect_not_in_row() {
    rejects_with(
        r#"
f :: Text -> Text
  = x => perform fail x;
f
"#,
        |kind| matches!(kind, ThirDiagnosticKind::EffectNotInRow { op } if op == "fail"),
    );
}

#[test]
fn perform_argument_mismatch_reports_type_mismatch() {
    rejects_with(
        r#"
Config :: type { value : Text; };
parse :: Text -> Config ! { fail Text; }
  = text => perform fail 1;
parse
"#,
        |kind| {
            matches!(kind, ThirDiagnosticKind::TypeMismatch { expected, found }
            if expected == "Text" && found == "Int")
        },
    );
}

#[test]
fn resume_argument_mismatch_reports_resume_type_mismatch() {
    rejects_with(
        r#"
Diagnostic :: type Text;
Config :: type { value : Text; };
check :: Config -> Config ! { warn Diagnostic; }
  = cfg => [ perform warn cfg.value; cfg ];
bad :: Config -> Config
  = cfg => handle check cfg with { warn = \d. resume 42; };
bad
"#,
        |kind| matches!(kind, ThirDiagnosticKind::ResumeTypeMismatch { found, .. } if found == "Int"),
    );
}

#[test]
fn handler_clause_wrong_arity_reports_diagnostic() {
    rejects_with(
        r#"
Diagnostic :: type Text;
Config :: type { value : Text; };
check :: Config -> Config ! { warn Diagnostic; }
  = cfg => [ perform warn cfg.value; cfg ];
bad :: Config -> Config
  = cfg => handle check cfg with { warn = \a b. resume (); };
bad
"#,
        |kind| {
            matches!(kind, ThirDiagnosticKind::HandlerClauseArityMismatch { op, expected, found }
            if op == "warn" && *expected == 1 && *found == 2)
        },
    );
}

#[test]
fn handler_clause_multiple_resume_reports_diagnostic() {
    rejects_with(
        r#"
Diagnostic :: type Text;
Config :: type { value : Text; };
check :: Config -> Config ! { warn Diagnostic; }
  = cfg => [ perform warn cfg.value; cfg ];
bad :: Config -> Config
  = cfg => handle check cfg with { warn = \d. [ resume (); resume () ]; };
bad
"#,
        |kind| matches!(kind, ThirDiagnosticKind::MultipleResume { op } if op == "warn"),
    );
}

#[test]
fn malformed_effect_op_reports_diagnostic() {
    rejects_with(
        r#"
bad :: Text -> Text ! { foo Text; }
  = x => x;
bad
"#,
        |kind| matches!(kind, ThirDiagnosticKind::MalformedEffectOp { op, .. } if op == "foo"),
    );
}

#[test]
fn resume_outside_handler_remains_a_hir_error() {
    let parsed = zutai_syntax::parse("resume 1");
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse should produce AST"));
    assert!(
        hir.diagnostics
            .iter()
            .any(|d| { matches!(d.kind, zutai_hir::HirDiagnosticKind::ResumeOutsideHandler) })
    );
}

#[test]
fn resume_in_nested_value_clause_remains_a_hir_error() {
    let parsed = zutai_syntax::parse(
        r#"
outer :: Text -> Text ! { warn Text; }
  = d => handle [ perform warn d; "ok" ] with { warn = \x. [ handle "ok" with { value = \v. resume (); }; resume () ]; };
outer
"#,
    );
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse should produce AST"));
    assert!(
        hir.diagnostics
            .iter()
            .any(|d| { matches!(d.kind, zutai_hir::HirDiagnosticKind::ResumeOutsideHandler) })
    );
}

// ── Phase 9: row-polymorphic THIR ────────────────────────────────────────────

#[test]
fn view_type_function_accepts_records_with_extra_fields() {
    completed_file(
        "getHost :: { host : Text; ...; } -> Text\n  = x => x.host;\ngetHost { host = \"h\"; port = 8080; }",
    );
}

#[test]
fn view_type_function_accepts_exact_record() {
    completed_file(
        "getHost :: { host : Text; ...; } -> Text\n  = x => x.host;\ngetHost { host = \"h\"; }",
    );
}

#[test]
fn view_type_function_rejects_record_missing_required_field() {
    let lowered = lower(
        "getHost :: { host : Text; ...; } -> Text\n  = x => x.host;\ngetHost { port = 8080; }",
    );
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::MissingRecordField { name } if name == "host"
    )));
}

#[test]
fn named_row_tail_identity_function_type_checks() {
    completed_file(
        "idHost :: <Rest> { host : Text; ...Rest; } -> { host : Text; ...Rest; }\n  = x => x;\nidHost",
    );
}

#[test]
fn named_row_tail_application_preserves_extra_fields() {
    let file = completed_file(
        "idHost :: <Rest> { host : Text; ...Rest; } -> { host : Text; ...Rest; }\n  = x => x;\nidHost { host = \"h\"; port = 8080; }",
    );
    let TypeKind::Record(fields, tail) = final_type_kind(&file) else {
        panic!("expected record result, got {:?}", final_type_kind(&file));
    };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert!(
        names.contains(&"host") && names.contains(&"port"),
        "named tail must preserve the extra `port`, got {names:?}"
    );
    assert_eq!(*tail, RowTail::Closed);
}

#[test]
fn value_select_builds_ordered_closed_record() {
    let file = completed_file(
        "s ::= { host = \"h\"; port = 8080; name = \"n\"; };\nselect s { port; host; }",
    );
    let TypeKind::Record(fields, tail) = final_type_kind(&file) else {
        panic!("expected record, got {:?}", final_type_kind(&file));
    };
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["port", "host"]);
    assert_eq!(*tail, RowTail::Closed);
}

#[test]
fn value_select_unknown_field_is_rejected() {
    let lowered = lower("s ::= { host = \"h\"; };\nselect s { missing; }");
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind, ThirDiagnosticKind::UnknownField { name } if name == "missing"
    )));
}

#[test]
fn type_select_projects_usable_closed_record_type() {
    completed_file(
        "Server :: type { host : Text; port : Int; };\nT :: type select Server { host; };\nx :: T = { host = \"h\"; };\nx",
    );
}

#[test]
fn type_select_unknown_field_is_rejected() {
    let lowered =
        lower("Server :: type { host : Text; };\nT :: type select Server { missing; };\nT");
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind, ThirDiagnosticKind::UnknownField { name } if name == "missing"
    )));
}

#[test]
fn diagnostic_polish_record_spread_overlap_shows_existing_and_incoming() {
    let lowered = lower(
        r#"
Base :: type { host : Text; port : Int; };
Bad :: type { host : Int; ...Base; };
Bad
"#,
    );
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::OverlappingRowField {
            item: RowOverlapItem::RecordField,
            source,
            name,
            existing,
            incoming,
        } if source == "Base"
            && name == "host"
            && existing == "host : Int"
            && incoming == "host : Text"
    )));
}

#[test]
fn open_union_match_with_wildcard_is_exhaustive() {
    completed_file(
        "classify :: { #dev; #test; ...; } -> Text\n  = #dev => \"d\";\n  = #test => \"t\";\n  = _ => \"o\";\nclassify #dev",
    );
}

#[test]
fn rest_tailed_union_match_typechecks() {
    // Phase D: a match over a `<Rest>`-tailed (rigid) open union type-checks — a
    // member pattern is a valid case of the rigid open union, and a wildcard
    // covers the tail.
    completed_file(
        "classify :: <Rest> { #dev; #test; ...Rest; } -> Text\n  = #dev => \"d\";\n  = #test => \"t\";\n  = _ => \"o\";\nclassify #dev",
    );
}

#[test]
fn open_union_match_without_wildcard_is_non_exhaustive() {
    let lowered = lower(
        "classify :: { #dev; #test; ...; } -> Text\n  = #dev => \"d\";\n  = #test => \"t\";\nclassify #dev",
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::NonExhaustiveMatch { .. }))
    );
}

#[test]
fn union_spread_merges_members_into_new_union() {
    // `Shape3D` spreads `Shape`; `#a` only type-checks against it if the spread
    // merged `Shape`'s members.
    completed_file(
        "Shape :: type { #a; #b; };\nShape3D :: type { ...Shape; #c; };\nx :: Shape3D = #a;\nx",
    );
}

#[test]
fn diagnostic_polish_union_spread_overlap_shows_existing_and_incoming() {
    let lowered = lower("Shape :: type { #a; #b; };\nBad :: type { #a; ...Shape; };\nBad");
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::OverlappingRowField {
            item: RowOverlapItem::UnionMember,
            source,
            name,
            existing,
            incoming,
        } if source == "Shape"
            && name == "a"
            && existing == "#a"
            && incoming == "#a"
    )));
}

#[test]
fn field_access_on_uninferred_value_requires_annotation() {
    let lowered = lower("f x = x.host;\nf");
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::RowAnnotationRequired { field: Some(field) }
            if field == "host"
    )));
}

#[test]
fn named_tail_result_field_is_accessible() {
    // The `port` preserved by the named tail must be visible to a later field
    // access, even before final zonking (record_fields flattens the solved tail).
    completed_file(
        "idHost :: <Rest> { host : Text; ...Rest; } -> { host : Text; ...Rest; }\n  = x => x;\n(idHost { host = \"h\"; port = 8080; }).port",
    );
}

#[test]
fn named_union_tail_application_captures_extra_member() {
    // `echo` forwards a named union tail; calling it with an extra member #prod
    // must succeed and the result must include #prod (captured by Rest).
    let file = completed_file(
        "echo :: <Rest> { #dev; ...Rest; } -> { #dev; ...Rest; }\n  = x => x;\necho #prod",
    );
    let TypeKind::Union(variants, tail) = final_type_kind(&file) else {
        panic!("expected union result, got {:?}", final_type_kind(&file));
    };
    let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
    assert!(
        names.contains(&"prod"),
        "named union tail must capture #prod, got {names:?}"
    );
    assert_eq!(*tail, RowTail::Closed);
}

#[test]
fn function_param_is_contravariant_for_open_records() {
    // `apply` invokes its callback with only `{ host }`; a callback that also
    // demands `port` must be rejected — function parameters are contravariant,
    // which is required for soundness once records have width subtyping.
    let lowered = lower(
        "apply :: ({ host : Text; ...; } -> Text) -> Text\n  = f => f { host = \"h\"; };\ng :: { host : Text; port : Int; } -> Text\n  = r => r.host;\napply g",
    );
    assert!(lowered.file.is_none());
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. }))
    );
}

#[test]
fn open_effect_row_tail_in_annotation_type_checks() {
    // A row-polymorphic effect signature: the row variable `...e` threads from the
    // argument thunk's effect row to the result thunk's. This is the check-only
    // foundation for effect-row-polymorphic types (e.g. an ergonomic effectful
    // stream). The rigid row variable lowers to `RowTail::Param` and threads via
    // exact-tail unification, exactly like a record/union row variable.
    let lowered = lower(
        r#"
forward :: <e> (Unit -> Int ! { ...e; }) -> Unit -> Int ! { ...e; }
  = f => f;
forward
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn call_site_pure_arg_against_open_effect_row_param() {
    // Call-site effect-row inference: applying a row-polymorphic function to a
    // *pure* thunk instantiates the open-row parameter `...e` and solves it to the
    // empty row. Previously exact-tail unification rejected `Closed` against the
    // instantiated `Infer` tail; now the flexible tail absorbs the (empty) residual.
    let lowered = lower(
        r#"
forward :: <e> (Unit -> Int ! { ...e; }) -> Unit -> Int ! { ...e; }
  = f => f;
forward (\_. 5)
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn call_site_explicit_closed_arg_against_open_effect_row_param() {
    // The load-bearing case for flexible effect-row tails: a binding *explicitly*
    // annotated with a closed effect row (`! {}`) passed to the row-polymorphic
    // parameter. This reaches effect-row *assignability* (function contravariance
    // over a `BindingRef` arg), where exact-tail matching previously rejected
    // `Closed` against the instantiated open tail. The flexible tail now absorbs
    // the empty residual, exactly as union/record rows do.
    let lowered = lower(
        r#"
forward :: <e> (Unit -> Int ! { ...e; }) -> Unit -> Int ! { ...e; }
  = f => f;
g :: Unit -> Int ! {} = \_. 9;
forward g
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn call_site_effectful_arg_threads_through_open_effect_row_param() {
    // The dual: an *effectful* thunk passed to the row-polymorphic parameter
    // solves the flexible tail to carry the `tick` operation through to the
    // result, where the surrounding handler discharges it.
    let lowered = lower(
        r#"
forward :: <e> (Unit -> Int ! { ...e; }) -> Unit -> Int ! { ...e; }
  = f => f;
handle (forward (\_. perform tick ()) ()) with { tick = \_. resume 5; }
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn effect_row_spread_of_named_effect_type_expands() {
    let file = completed_file(
        r#"
ReadPack :: type Unit ! { fs.read : Path -> Text; };
f :: Path -> Text ! { ...ReadPack; }
  = path => perform fs.read path;
f
"#,
    );
    let row = file.type_arena.iter().find_map(|ty| match &ty.kind {
        TypeKind::Effect { row, .. } if row.find("fs.read").is_some() => Some(row),
        _ => None,
    });
    assert!(row.is_some(), "expected expanded fs.read effect row");
}

#[test]
fn effect_row_spread_can_compose_with_row_variable_tail() {
    let lowered = lower(
        r#"
ReadPack :: type Unit ! { fs.read : Path -> Text; };
forward :: <e> (Unit -> Text ! { ...ReadPack; ...e; }) -> Unit -> Text ! { ...ReadPack; ...e; }
  = f => f;
forward
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn effect_row_spread_of_non_effect_type_is_refused() {
    let lowered = lower(
        r#"
Shape :: type { a : Int; };
f :: Int -> Int ! { ...Shape; }
  = x => x;
f
"#,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::InvalidTypeExpression { reason }
                if reason.contains("requires a named effect type")
        )),
        "{:?}",
        lowered.diagnostics
    );
}

#[test]
fn generic_effect_alias_substitutes_base_and_op_type() {
    let file = completed_file(
        r#"
Failing :: <A> type A ! { fail A; };
f :: Text -> Failing Int
  = _ => perform fail 1;
f
"#,
    );
    let effect = file
        .type_arena
        .iter()
        .find_map(|ty| match &ty.kind {
            TypeKind::Effect { base, row }
                if matches!(file.type_arena[base.0 as usize].kind, TypeKind::Int) =>
            {
                Some(row)
            }
            _ => None,
        })
        .expect("instantiated effect type");
    let fail = effect.find("fail").expect("fail effect op");
    assert!(matches!(
        file.type_arena[fail.param.0 as usize].kind,
        TypeKind::Int
    ));
    assert!(matches!(
        file.type_arena[fail.result.0 as usize].kind,
        TypeKind::Never
    ));
}

#[test]
fn union_payload_type_mismatch_reports_component_type() {
    let lowered = lower(
        r#"
Result :: type { #ok : { v : Int; }; ...; };
x :: Result = #ok { v = "bad"; };
x
"#,
    );
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected == "Int" && found == "Text"
        )
    }));
}
