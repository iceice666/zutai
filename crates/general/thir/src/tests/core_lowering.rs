use super::*;
use zutai_syntax::posit::PositSpec;

#[test]
fn inferred_integer_binding_completes_thir() {
    let file = completed_file("x ::= 1\nx");

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn numeric_postfix_literals_have_fixed_types() {
    let file = completed_file("255u8");
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::FixedNum(FixedWidth::U8)
    ));

    let file = completed_file("-128i8");
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::FixedNum(FixedWidth::I8)
    ));

    let file = completed_file("42i64");
    assert!(matches!(final_type_kind(&file), TypeKind::Int));

    let file = completed_file("3.14f64");
    assert!(matches!(final_type_kind(&file), TypeKind::Float));

    let file = completed_file("1f32");
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::FixedNum(FixedWidth::F32)
    ));
}

#[test]
fn posit_literals_have_posit_types() {
    let file = completed_file("x :: Posit32 = 1.5p32\nx");
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::Posit(spec) if *spec == (PositSpec { nbits: 32, es: 2 })
    ));

    let file = completed_file("x :: Posit64e5 = 1.5p64e5\nx");
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::Posit(spec) if *spec == (PositSpec { nbits: 64, es: 5 })
    ));
}

#[test]
fn posit_annotations_require_matching_literals() {
    let lowered = lower("x :: Float = 1p32\nx");
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected == "Float" && found == "Posit32"
        )
    }));
}

#[test]
fn posit_arithmetic_requires_matching_posit_types() {
    let lowered = lower("1p32 + 2p32e3");
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected == "Posit32" && found == "Posit32e3"
        )
    }));
}

#[test]
fn fixed_width_arithmetic_is_rejected() {
    let lowered = lower("255u8 + 1u8");
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::InvalidBinaryOperands { op, lhs, rhs }
                if *op == "+" && lhs == "u8" && rhs == "u8"
        )
    }));
}

#[test]
fn fixed_width_integer_literals_are_range_checked() {
    let lowered = lower("256u8");
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::NumericLiteralOutOfRange { value, ty }
                if *value == 256 && ty == "u8"
        )
    }));
}

#[test]
fn fixed_width_annotations_require_matching_literals() {
    let file = completed_file("x :: u8 = 255u8\nx");
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::FixedNum(FixedWidth::U8)
    ));

    let lowered = lower("x :: u8 = 255\nx");
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected == "u8" && found == "Int"
        )
    }));
}

#[test]
fn fixed_width_literal_patterns_type_check() {
    completed_file(
        r#"
classify :: u8 -> Text
  = 255u8 => "max";
  = _ => "other";
classify 255u8
"#,
    );
}

#[test]
fn typed_integer_mismatch_reports_type_error() {
    let lowered = lower("x :: Int = \"bad\"\nx");

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected == "Int" && found == "Text"
        )
    }));
}

#[test]
fn non_generic_record_alias_accepts_matching_record() {
    let file = completed_file(
        r#"
Server :: type {
  host : Text;
  port : Int;
}

server :: Server = {
  host = "localhost";
  port = 8080;
}

server
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Alias(_)));
}

#[test]
fn record_literal_reports_missing_required_field() {
    let lowered = lower(
        r#"
Server :: type {
  host : Text;
  port : Int;
}

server :: Server = {
  host = "localhost";
}

server
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::MissingRecordField { name } if name == "port"
        )
    }));
}

#[test]
fn record_literal_reports_unexpected_field() {
    let lowered = lower(
        r#"
Server :: type {
  host : Text;
}

server :: Server = {
  host = "localhost";
  port = 8080;
}

server
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::UnexpectedRecordField { name } if name == "port"
        )
    }));
}

#[test]
fn optional_record_field_may_be_omitted() {
    let file = completed_file(
        r#"
RawServer :: type {
  host? : Text;
  port : Int;
}

server :: RawServer = {
  port = 8080;
}

server
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Alias(_)));
}

#[test]
fn record_update_required_field_type_checks() {
    let file = completed_file(
        r#"
Server :: type {
  host : Text;
  port : Int;
}

server :: Server = {
  host = "localhost";
  port = 8080;
}

server with { port = 9090; }
"#,
    );

    let update = &file.expr_arena[file.final_expr];
    let ThirExprKind::RecordUpdate { receiver, fields } = &update.kind else {
        panic!("expected RecordUpdate, got {:?}", update.kind);
    };
    assert_eq!(fields[0].name, "port");
    assert_eq!(update.ty, file.expr_arena[*receiver].ty);
    assert!(matches!(final_type_kind(&file), TypeKind::Alias(_)));
}

#[test]
fn record_update_field_type_mismatch_is_reported() {
    let lowered = lower(
        r#"
Server :: type {
  port : Int;
}

server :: Server = {
  port = 8080;
}

server with { port = "bad"; }
"#,
    );

    assert!(lowered.file.is_none());
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(&diagnostic.kind, ThirDiagnosticKind::TypeMismatch { .. }))
    );
}

#[test]
fn record_update_unknown_field_is_reported() {
    let lowered = lower(
        r#"
Server :: type {
  port : Int;
}

server :: Server = {
  port = 8080;
}

server with { missing = 1; }
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::UnknownField { name } if name == "missing"
        )
    }));
}

#[test]
fn record_update_uninferred_receiver_requires_row_annotation() {
    let lowered = lower("f x = x with { host = \"localhost\"; }\nf");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic.kind, ThirDiagnosticKind::RowAnnotationRequired))
    );
}

#[test]
fn record_update_duplicate_field_is_hir_diagnostic() {
    let parsed = zutai_syntax::parse("s ::= { a = 1; }\ns with { a = 2; a = 3; }");
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse should produce AST"));
    assert!(hir.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            zutai_hir::HirDiagnosticKind::DuplicateRecordField { name, .. } if name == "a"
        )
    }));
}

#[test]
fn record_update_optional_field_accepts_payload_type() {
    let file = completed_file(
        r#"
Server :: type {
  host : Text;
  port? : Int;
}

server :: Server = {
  host = "localhost";
}

server with { port = 8080; }
"#,
    );

    let update = &file.expr_arena[file.final_expr];
    let ThirExprKind::RecordUpdate { fields, .. } = &update.kind else {
        panic!("expected RecordUpdate, got {:?}", update.kind);
    };
    assert_eq!(fields[0].name, "port");
    assert!(matches!(
        file.type_arena[file.expr_arena[fields[0].value].ty.0 as usize].kind,
        TypeKind::Int
    ));
}

#[test]
fn record_update_empty_block_is_rejected() {
    let lowered = lower(
        r#"
Server :: type {
  port : Int;
}

server :: Server = {
  port = 8080;
}

server with {}
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::UnsupportedFeature { feature } if *feature == "empty record update"
        )
    }));
}

#[test]
fn patch_record_accepts_subset_fields() {
    let file = completed_file(
        r#"
Server :: type {
  host : Text;
  port : Int;
}

patch :: Patch Server = {
  port = 8080;
}

patch
"#,
    );

    assert!(matches!(
        final_type_kind(&file),
        TypeKind::Patch { deep: false, .. }
    ));
}

#[test]
fn patch_record_rejects_unknown_closed_field() {
    let lowered = lower(
        r#"
Server :: type {
  port : Int;
}

patch :: Patch Server = {
  missing = 1;
}

patch
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::UnexpectedRecordField { name } if name == "missing"
        )
    }));
}

#[test]
fn deep_patch_record_accepts_nested_record_patch() {
    let file = completed_file(
        r#"
Server :: type {
  host : Text;
  port : Int;
}

Config :: type {
  server : Server;
  name : Text;
}

patch :: DeepPatch Config = {
  server = {
    port = 8080;
  };
}

patch
"#,
    );

    assert!(matches!(
        final_type_kind(&file),
        TypeKind::Patch { deep: true, .. }
    ));
}

#[test]
fn patch_requires_record_target() {
    let lowered = lower(
        r#"
patch :: Patch Int = {}
patch
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::InvalidTypeExpression { reason }
                if *reason == "Patch requires a record type"
        )
    }));
}

#[test]
fn deep_patch_requires_record_target() {
    let lowered = lower(
        r#"
patch :: DeepPatch Int = {}
patch
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::InvalidTypeExpression { reason }
                if *reason == "DeepPatch requires a record type"
        )
    }));
}

#[test]
fn overlay_accepts_inline_base_and_patch() {
    let file = completed_file(
        r#"
overlay {
  port = 8080;
} {
  host = "localhost";
  port = 80;
}
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Record(_, _)));
}

#[test]
fn overlay_rejects_unknown_patch_field() {
    let lowered = lower(
        r#"
overlay {
  missing = 1;
} {
  port = 80;
}
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::UnexpectedRecordField { name } if name == "missing"
        )
    }));
}

#[test]
fn overlay_deep_accepts_nested_patch() {
    let file = completed_file(
        r#"
overlayDeep {
  server = {
    port = 8080;
  };
} {
  server = {
    host = "localhost";
    port = 80;
  };
  name = "dev";
}
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Record(_, _)));
}

#[test]
fn overlay_deep_checks_nested_patch_field_type() {
    let lowered = lower(
        r#"
overlayDeep {
  server = {
    port = "bad";
  };
} {
  server = {
    port = 80;
  };
}
"#,
    );

    assert!(lowered.file.is_none());
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(&diagnostic.kind, ThirDiagnosticKind::TypeMismatch { .. }))
    );
}
#[test]
fn required_field_access_yields_field_type() {
    let file = completed_file(
        r#"
Server :: type {
  host : Text;
  port : Int;
}

server :: Server = {
  host = "localhost";
  port = 8080;
}

server.host
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

#[test]
fn atom_union_alias_accepts_matching_atom() {
    let file = completed_file(
        r#"
Profile :: type {
  #dev;
  #prod;
}

profile :: Profile = #prod
profile
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Alias(_)));
}

#[test]
fn no_signature_identity_function_completes_thir() {
    // `id x = x` — polymorphic identity; no annotation needed.
    let file = completed_file("id x = x\nid 42");
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn no_signature_identity_used_at_two_types_completes() {
    // `id x = x` generalizes; each use instantiates fresh InferVars.
    let file = completed_file("id x = x\n(id 42, id \"hello\")");
    assert!(matches!(final_type_kind(&file), TypeKind::Tuple(_)));
}

#[test]
fn no_signature_identity_single_type_still_int() {
    // Single-type use is unaffected by generalization.
    let file = completed_file("id x = x\nid 42");
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn recursive_function_stays_monomorphic() {
    // Self-references read the un-generalized signature so recursion stays monomorphic.
    let file = completed_file("count n = count n\ncount 5");
    let _ = file;
}

#[test]
fn no_signature_arithmetic_function_infers_int_type() {
    let file = completed_file("double x = x + x\ndouble 5");
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn no_signature_multi_param_function_completes_thir() {
    let file = completed_file("add x y = x + y\nadd 3 4");
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

// ── Generic (explicit TypeVar) functions ────────────────────────────────────

#[test]
fn generic_identity_function_applied_to_int() {
    let file = completed_file(
        r#"
id :: <A> A -> A
  = x => x;

id 99
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn generic_identity_function_applied_to_text() {
    let file = completed_file(
        r#"
id :: <A> A -> A
  = x => x;

id "hello"
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

#[test]
fn generic_const_function_returns_first_arg() {
    let file = completed_file(
        r#"
const :: <A, B> A -> B -> A
  = x _ => x;

const 42 "ignored"
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn monomorphic_function_application_yields_return_type() {
    let file = completed_file(
        r#"
id :: Int -> Int
  = x => x;

id 41
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn curried_function_application_yields_final_return_type() {
    let file = completed_file(
        r#"
first :: Int -> Text -> Int
  = x _ => x;

first 1 "ignored"
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn function_can_reference_later_function_signature() {
    let file = completed_file(
        r#"
useLater :: Int -> Int
  = x => later x;

later :: Int -> Int
  = y => y;

useLater 3
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn function_return_mismatch_reports_type_error() {
    let lowered = lower(
        r#"
bad :: Int -> Text
  = x => x;

bad 1
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected == "Text" && found == "Int"
        )
    }));
}

#[test]
fn function_argument_mismatch_reports_type_error() {
    let lowered = lower(
        r#"
id :: Int -> Int
  = x => x;

id "bad"
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

#[test]
fn applying_non_function_reports_expected_function() {
    let lowered = lower("x ::= 1\nx 2");

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::ExpectedFunction { found } if found == "Int"
        )
    }));
}

#[test]
fn block_local_binding_yields_result_type() {
    let file = completed_file("{ x := 1; x }");

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn typed_block_local_binding_checks_annotation() {
    let file = completed_file("{ x : Int = 1; x }");

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn typed_block_local_binding_reports_type_error() {
    let lowered = lower("{ x : Int = \"bad\"; x }");

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected == "Int" && found == "Text"
        )
    }));
}

#[test]
fn typed_block_local_binding_allows_type_params() {
    let file = completed_file(
        r#"
id :: <A> A -> A
  = x => {
    y : A = x;
    y
  };
id 42
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn list_literal_infers_homogeneous_list_type() {
    let file = completed_file("[1; 2; 3;]");

    assert!(matches!(final_type_kind(&file), TypeKind::List(_)));
}

#[test]
fn typed_empty_list_completes_thir() {
    let file = completed_file(
        r#"
items :: List Int = []
items
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::List(_)));
}

#[test]
fn untyped_empty_list_reports_inference_error() {
    let lowered = lower("[]");

    assert!(lowered.file.is_none());
    assert!(
        lowered.diagnostics.iter().any(|diagnostic| {
            matches!(diagnostic.kind, ThirDiagnosticKind::EmptyListNeedsType)
        })
    );
}

#[test]
fn list_literal_reports_item_type_mismatch() {
    let lowered = lower("[1; \"bad\";]");

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected == "Int" && found == "Text"
        )
    }));
}

#[test]
fn tuple_literal_infers_tuple_type() {
    let file = completed_file("(#circle, radius = 5.0)");

    assert!(matches!(final_type_kind(&file), TypeKind::Tuple(_)));
}

#[test]
fn conditional_requires_bool_condition_and_compatible_branches() {
    let file = completed_file("if true then 1 else 2");

    assert!(matches!(final_type_kind(&file), TypeKind::Int));

    let lowered = lower("if 1 then 1 else 2");
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected == "Bool" && found == "Int"
        )
    }));
}

#[test]
fn scalar_binary_expressions_complete_thir() {
    for src in ["1 + 2", "1 < 2", "true && false", "1 == 1"] {
        let file = completed_file(src);
        assert!(
            matches!(final_type_kind(&file), TypeKind::Int | TypeKind::Bool),
            "{src}"
        );
    }
}

#[test]
fn invalid_arithmetic_operands_are_reported() {
    let lowered = lower("true + false");

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::InvalidBinaryOperands { op, lhs, rhs }
                if *op == "+" && lhs == "Bool" && rhs == "Bool"
        )
    }));
}

#[test]
fn defaulting_operator_requires_optional_lhs() {
    let file = completed_file(
        r#"
RawServer :: type {
  port? : Int;
}

server :: RawServer = {}
server.port ?? 8080
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Int));

    let lowered = lower("1 ?? 2");
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::ExpectedOptionalOrMaybe { found } if found == "Int"
        )
    }));
}

#[test]
fn function_body_can_return_checked_record_literal() {
    let file = completed_file(
        r#"
Server :: type {
  host : Text;
  port : Int;
}

make :: Text -> Server
  = host => {
    host = host;
    port = 8080;
  };

make "localhost"
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Alias(_)));
}

#[test]
fn function_clause_arity_mismatch_is_reported() {
    let lowered = lower(
        r#"
bad :: Int -> Int
  = x y => x;

1
"#,
    );

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind,
            ThirDiagnosticKind::FunctionClauseArityMismatch {
                expected: 1,
                found: 2
            }
        )
    }));
}

#[test]
fn atom_literal_pattern_accepts_union_member() {
    let file = completed_file(
        r#"
Profile :: type {
  #dev;
  #prod;
}

isProd :: Profile -> Bool
  = #prod => true;
  = #dev => false;

isProd #prod
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Bool));
}

#[test]
fn runs_thir_passes_in_order() {
    struct MarkerPass(&'static str);

    impl ThirPass for MarkerPass {
        fn name(&self) -> &'static str {
            self.0
        }

        fn run(&mut self, file: &mut ThirFile, _diagnostics: &mut Vec<ThirDiagnostic>) {
            file.decls.clear();
        }
    }

    let mut file = completed_file("1");
    let mut diagnostics = Vec::new();
    let mut first = MarkerPass("first");
    let mut second = MarkerPass("second");
    let mut passes: [&mut dyn ThirPass; 2] = [&mut first, &mut second];

    let reports = run_passes(&mut file, &mut diagnostics, &mut passes);

    assert_eq!(
        reports,
        vec![
            ThirPassReport { name: "first" },
            ThirPassReport { name: "second" }
        ]
    );
    assert!(diagnostics.is_empty());
}

// ── Tuple patterns ──────────────────────────────────────────────────────────

#[test]
fn tuple_pattern_in_function_clause() {
    let file = completed_file(
        r#"
pair_first :: (#tag, Int) -> Int
  = (#tag, x) => x;

pair_first (#tag, 42)
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn positional_tuple_pattern_in_function_clause() {
    let file = completed_file(
        r#"
add_pair :: (Int, Int) -> Int
  = (a, b) => a + b;

add_pair (1, 2)
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn tuple_pattern_arity_mismatch_reports_error() {
    let lowered = lower(
        r#"
fst :: (Int, Int) -> Int
  = (a, b, c) => a;

fst (1, 2)
"#,
    );
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::TupleArityMismatch {
            expected: 2,
            found: 3
        }
    )));
}

// ── Record patterns ─────────────────────────────────────────────────────────

#[test]
fn record_pattern_in_function_clause() {
    let file = completed_file(
        r#"
Point :: type { x : Int; y : Int; }

get_x :: Point -> Int
  = { x = v; y = _; } => v;

get_x { x = 10; y = 20; }
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn record_pattern_unknown_field_reports_error() {
    let lowered = lower(
        r#"
Point :: type { x : Int; y : Int; }

get_x :: Point -> Int
  = { x = v; z = _; } => v;

get_x { x = 1; y = 2; }
"#,
    );
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::UnknownField { name } if name == "z"
    )));
}

// ── Lambda expressions ───────────────────────────────────────────────────────

#[test]
fn lambda_in_checked_position_lowers_correctly() {
    let file = completed_file(
        r#"
double :: Int -> Int = \n. n * 2

double 5
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn lambda_multi_param_in_checked_position() {
    let file = completed_file(
        r#"
add :: Int -> Int -> Int = \a b. a + b

add 3 4
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn lambda_without_type_context_infers_polymorphic_type() {
    // `\x. x` is a polymorphic identity; inference now succeeds without a
    // type annotation.
    let file = completed_file(r#"(\x. x) 42"#);
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn lambda_without_annotation_applied_to_text_yields_text_type() {
    let file = completed_file(r#"(\x. x) "hello""#);
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

// ── Match expressions ────────────────────────────────────────────────────────

#[test]
fn match_on_atom_union_lowers_correctly() {
    let file = completed_file(
        r#"
Status :: type {#ok; #err;}

describe :: Status -> Text
  = s => match s {
    | #ok => "ok";
    | #err => "error";
  };

describe #ok
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

#[test]
fn match_with_guard_lowers_correctly() {
    let file = completed_file(
        r#"
classify :: Int -> Text
  = n => match n {
    | x if x > 0 => "positive";
    | x if x < 0 => "negative";
    | _ => "zero";
  };

classify 5
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

#[test]
fn match_with_tuple_arm_lowers_correctly() {
    let file = completed_file(
        r#"
extract :: (#tag, Int) -> Int
  = pair => match pair {
    | (#tag, n) => n;
  };

extract (#tag, 7)
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}
