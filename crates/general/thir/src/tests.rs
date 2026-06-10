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
fn no_signature_identity_function_completes_thir() {
    // `id x = x` — polymorphic identity; no annotation needed.
    let file = completed_file("id x = x\nid 42");
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
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
id :: <A> A -> A {
  | x => x;
}

id 99
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn generic_identity_function_applied_to_text() {
    let file = completed_file(
        r#"
id :: <A> A -> A {
  | x => x;
}

id "hello"
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

#[test]
fn generic_const_function_returns_first_arg() {
    let file = completed_file(
        r#"
const :: <A, B> A -> B -> A {
  | x _ => x;
}

const 42 "ignored"
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
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
  | #dev => false;
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

// ── Tuple patterns ──────────────────────────────────────────────────────────

#[test]
fn tuple_pattern_in_function_clause() {
    let file = completed_file(
        r#"
pair_first :: (#tag, Int) -> Int {
  | (#tag, x) => x;
}

pair_first (#tag, 42)
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn positional_tuple_pattern_in_function_clause() {
    let file = completed_file(
        r#"
add_pair :: (Int, Int) -> Int {
  | (a, b) => a + b;
}

add_pair (1, 2)
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn tuple_pattern_arity_mismatch_reports_error() {
    let lowered = lower(
        r#"
fst :: (Int, Int) -> Int {
  | (a, b, c) => a;
}

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

get_x :: Point -> Int {
  | { x = v; y = _; } => v;
}

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

get_x :: Point -> Int {
  | { x = v; z = _; } => v;
}

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
Status :: type [ #ok; #err; ]

describe :: Status -> Text {
  | s => match s {
    | #ok => "ok";
    | #err => "error";
  };
}

describe #ok
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

#[test]
fn match_with_guard_lowers_correctly() {
    let file = completed_file(
        r#"
classify :: Int -> Text {
  | n => match n {
    | x if x > 0 => "positive";
    | x if x < 0 => "negative";
    | _ => "zero";
  };
}

classify 5
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

#[test]
fn match_with_tuple_arm_lowers_correctly() {
    let file = completed_file(
        r#"
extract :: (#tag, Int) -> Int {
  | pair => match pair {
    | (#tag, n) => n;
  };
}

extract (#tag, 7)
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

// ── Exhaustiveness & reachability ────────────────────────────────────────────

fn nonexhaustive_witness(lowered: &LoweredThir) -> Option<String> {
    lowered.diagnostics.iter().find_map(|d| match &d.kind {
        ThirDiagnosticKind::NonExhaustiveMatch { witness } => Some(witness.clone()),
        _ => None,
    })
}

fn has_unreachable(lowered: &LoweredThir) -> bool {
    lowered
        .diagnostics
        .iter()
        .any(|d| matches!(d.kind, ThirDiagnosticKind::UnreachableMatchArm))
}

#[test]
fn exhaustive_atom_union_passes() {
    completed_file(
        r#"
Profile :: type [ #dev; #prod; ]
isProd :: Profile -> Bool {
  | #dev => false;
  | #prod => true;
}
isProd #dev
"#,
    );
}

#[test]
fn non_exhaustive_atom_union_reports_witness() {
    let lowered = lower(
        r#"
Profile :: type [ #dev; #prod; ]
isProd :: Profile -> Bool {
  | #prod => true;
}
isProd #prod
"#,
    );
    assert!(lowered.file.is_none());
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("#dev"));
}

#[test]
fn wildcard_catch_all_is_exhaustive() {
    let lowered = lower(
        r#"
Profile :: type [ #dev; #prod; ]
isProd :: Profile -> Bool {
  | #prod => true;
  | _ => false;
}
isProd #dev
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn redundant_arm_after_catch_all_is_unreachable() {
    let lowered = lower(
        r#"
Profile :: type [ #dev; #prod; ]
classify :: Profile -> Bool {
  | _ => false;
  | #prod => true;
}
classify #dev
"#,
    );
    assert!(has_unreachable(&lowered));
}

#[test]
fn bool_match_exhaustive_passes() {
    completed_file(
        r#"
negate :: Bool -> Bool {
  | true => false;
  | false => true;
}
negate true
"#,
    );
}

#[test]
fn bool_match_non_exhaustive_reports_false() {
    let lowered = lower(
        r#"
f :: Bool -> Bool {
  | true => false;
}
f true
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("false"));
}

#[test]
fn int_match_requires_wildcard() {
    let lowered = lower(
        r#"
f :: Int -> Int {
  | 1 => 10;
  | 2 => 20;
}
f 1
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("_"));
}

#[test]
fn int_match_with_wildcard_is_exhaustive() {
    completed_file(
        r#"
f :: Int -> Int {
  | 1 => 10;
  | _ => 0;
}
f 1
"#,
    );
}

#[test]
fn guarded_arm_does_not_cover() {
    let lowered = lower(
        r#"
Profile :: type [ #dev; #prod; ]
f :: Profile -> Bool {
  | #dev => false;
  | #prod if true => true;
}
f #dev
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("#prod"));
}

#[test]
fn plain_arm_after_guarded_same_pattern_is_reachable() {
    let lowered = lower(
        r#"
Profile :: type [ #dev; #prod; ]
f :: Profile -> Bool {
  | #dev => false;
  | #prod if true => true;
  | #prod => false;
}
f #dev
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn multi_clause_function_exhaustive_passes() {
    completed_file(
        r#"
pick :: Bool -> Bool -> Text {
  | true true => "tt";
  | true false => "tf";
  | false _ => "f";
}
pick true false
"#,
    );
}

#[test]
fn multi_clause_function_non_exhaustive_reports() {
    let lowered = lower(
        r#"
pick :: Bool -> Bool -> Text {
  | true true => "tt";
  | true false => "tf";
}
pick true false
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("false _"));
}

#[test]
fn tagged_tuple_union_exhaustive_passes() {
    completed_file(
        r#"
Shape :: type [
  (#circle, radius : Int);
  (#square, side : Int);
]
area :: Shape -> Int {
  | (#circle, radius = r) => r;
  | (#square, side = s) => s;
}
area
"#,
    );
}

#[test]
fn tagged_tuple_union_non_exhaustive_reports_witness() {
    let lowered = lower(
        r#"
Shape :: type [
  (#circle, radius : Int);
  (#square, side : Int);
]
area :: Shape -> Int {
  | (#circle, radius = r) => r;
}
area
"#,
    );
    assert_eq!(
        nonexhaustive_witness(&lowered).as_deref(),
        Some("(#square, ...)")
    );
}

#[test]
fn optional_match_exhaustive_passes() {
    completed_file(
        r#"
unwrap :: Int? -> Int {
  | #none => 0;
  | (#some, value = x) => x;
}
unwrap #none
"#,
    );
}

#[test]
fn optional_match_missing_none_reports_witness() {
    let lowered = lower(
        r#"
unwrap :: Int? -> Int {
  | (#some, value = x) => x;
}
unwrap #none
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("#none"));
}

#[test]
fn optional_match_missing_some_reports_witness() {
    let lowered = lower(
        r#"
unwrap :: Int? -> Int {
  | #none => 0;
}
unwrap #none
"#,
    );
    assert_eq!(
        nonexhaustive_witness(&lowered).as_deref(),
        Some("(#some, ...)")
    );
}

// ── Optional access (`?.`) ───────────────────────────────────────────────────

#[test]
fn opt_access_on_optional_record() {
    let file = completed_file(
        r#"
Server :: type { port : Int; }

get_port :: Server? -> Int? {
  | s => s?.port;
}

get_port #none
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Optional(_)));
}

#[test]
fn opt_access_optional_field_flattens() {
    let file = completed_file(
        r#"
Server :: type { port? : Int; }

get_port :: Server? -> Int? {
  | s => s?.port;
}

get_port #none
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Optional(_)));
}

#[test]
fn opt_access_on_non_optional_reports_error() {
    let lowered = lower(
        r#"
Server :: type { port : Int; }

get_port :: Server -> Int? {
  | s => s?.port;
}

get_port { port = 80; }
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::ExpectedOptional { .. }))
    );
}
