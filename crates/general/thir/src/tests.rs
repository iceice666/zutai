use crate::*;

fn lower(src: &str) -> LoweredThir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse should produce AST"));
    assert!(hir.diagnostics.is_empty(), "{:?}", hir.diagnostics);
    lower_hir(&hir.file)
}

fn completed_file(src: &str) -> ThirFile {
    let lowered = lower(src);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    lowered.file.expect("valid THIR should be produced")
}

fn final_type_kind(file: &ThirFile) -> &TypeKind {
    let final_expr = &file.expr_arena[file.final_expr.0 as usize];
    &file.type_arena[final_expr.ty.0 as usize].kind
}

#[test]
fn inferred_integer_binding_completes_thir() {
    let file = completed_file("x := 1\nx");

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
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
Profile :: type [
  #dev;
  #prod;
]

profile :: Profile = #prod
profile
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Alias(_)));
}

#[test]
fn no_signature_function_declarations_remain_unsupported() {
    let lowered = lower("id x = x\n1");

    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind,
            ThirDiagnosticKind::UnsupportedFeature {
                feature: "no-signature function declarations"
            }
        )
    }));
}

#[test]
fn monomorphic_function_application_yields_return_type() {
    let file = completed_file(
        r#"
id :: Int -> Int {
  | x => x;
}

id 41
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn curried_function_application_yields_final_return_type() {
    let file = completed_file(
        r#"
first :: Int -> Text -> Int {
  | x _ => x;
}

first 1 "ignored"
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn function_can_reference_later_function_signature() {
    let file = completed_file(
        r#"
useLater :: Int -> Int {
  | x => later x;
}

later :: Int -> Int {
  | y => y;
}

useLater 3
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn function_return_mismatch_reports_type_error() {
    let lowered = lower(
        r#"
bad :: Int -> Text {
  | x => x;
}

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
id :: Int -> Int {
  | x => x;
}

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
    let lowered = lower("x := 1\nx 2");

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
            ThirDiagnosticKind::ExpectedOptional { found } if found == "Int"
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

make :: Text -> Server {
  | host => {
    host = host;
    port = 8080;
  };
}

make "localhost"
"#,
    );

    assert!(matches!(final_type_kind(&file), TypeKind::Alias(_)));
}

#[test]
fn function_clause_arity_mismatch_is_reported() {
    let lowered = lower(
        r#"
bad :: Int -> Int {
  | x y => x;
}

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
Profile :: type [
  #dev;
  #prod;
]

isProd :: Profile -> Bool {
  | #prod => true;
}

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

    let mut file = ThirFile {
        decls: Vec::new(),
        final_expr: ThirExprId(0),
        decl_arena: Vec::new(),
        expr_arena: Vec::new(),
        pat_arena: Vec::new(),
        type_arena: Vec::new(),
    };
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
