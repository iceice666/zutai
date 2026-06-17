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
    let final_expr = &file.expr_arena[file.final_expr];
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
  dev;
  prod;
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
  dev;
  prod;
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
Status :: type [ok; err;]

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
Profile :: type [dev; prod;]
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
Profile :: type [dev; prod;]
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
Profile :: type [dev; prod;]
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
Profile :: type [dev; prod;]
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
Profile :: type [dev; prod;]
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
Profile :: type [dev; prod;]
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
  circle: { radius: Int; };
  square: { side: Int; };
]
area :: Shape -> Int {
  | #circle { radius = r; } => r;
  | #square { side = s; } => s;
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
  circle: { radius: Int; };
  square: { side: Int; };
]
area :: Shape -> Int {
  | #circle { radius = r; } => r;
}
area
"#,
    );
    assert_eq!(
        nonexhaustive_witness(&lowered).as_deref(),
        Some("#square { ... }")
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
        Some("#some { ... }")
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

// ── Generic type aliases (parametric type constructors) ──────────────────────

#[test]
fn generic_alias_application_resolves_to_record() {
    // `Pair :: <A, B> type { first: A; second: B; }` then `p : Pair Text Int`.
    // After THIR type-checks the record, the final_expr (p) has type
    // `AliasApply { binding: Pair, args: [Text, Int] }`.  The alias is
    // transparent at the use site: the record literal `{ first: "x"; second: 1 }`
    // must match — so we assert the whole program completes with no diagnostics.
    let file = completed_file(
        r#"
Pair :: <A, B> type { first : A; second : B; }
p :: Pair Text Int = { first = "x"; second = 1; }
p
"#,
    );
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::AliasApply { .. }
    ));
}

#[test]
fn generic_alias_used_in_function_signature() {
    // A function that takes a `Pair Int Int` and returns the first field.
    let file = completed_file(
        r#"
Pair :: <A, B> type { first : A; second : B; }
fst :: Pair Int Int -> Int {
  | p => p.first;
}
fst { first = 1; second = 2; }
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn generic_alias_wrong_arity_reports_error() {
    // `Pair` needs 2 args; giving 1 must emit TypeConstructorArityMismatch.
    let lowered = lower(
        r#"
Pair :: <A, B> type { first : A; second : B; }
x :: Pair Text = x
x
"#,
    );
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::TypeConstructorArityMismatch { name, expected, found }
            if name == "Pair" && *expected == 2 && *found == 1
    )));
}

#[test]
fn generic_alias_bare_reference_reports_error() {
    // A bare `Pair` (zero args) in type position must emit TypeConstructorArityMismatch.
    let lowered = lower(
        r#"
Pair :: <A, B> type { first : A; second : B; }
x :: Pair = x
x
"#,
    );
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::TypeConstructorArityMismatch { expected, found, .. }
            if *expected == 2 && *found == 0
    )));
}

// ── Type-level evaluation fuel limit ────────────────────────────────────────

/// Lower `src` with a reduced type-evaluation fuel budget, returned as a
/// `LoweredThir` so callers can inspect diagnostics.
fn lower_with_type_eval_fuel(src: &str, fuel: u32) -> LoweredThir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse should produce AST"));
    assert!(hir.diagnostics.is_empty(), "{:?}", hir.diagnostics);
    lower_hir_with_options(
        &hir.file,
        ThirLowerOptions {
            run_passes: true,
            type_eval_fuel: Some(fuel),
            ..ThirLowerOptions::default()
        },
    )
}

#[test]
fn type_level_expansion_exceeding_fuel_reports_limit() {
    // D1 → D2 = Pair D1 D1 → D3 = Pair D2 D2: resolving D3 requires multiple
    // Pair expansions. With a budget of 1 the second expansion is denied.
    let src = r#"
Pair :: <A, B> type { first : A; second : B; }
D1 :: type Int
D2 :: type Pair D1 D1
D3 :: type Pair D2 D2
x :: D3 = x
x
"#;
    let lowered = lower_with_type_eval_fuel(src, 1);
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeLevelEvalLimitExceeded)),
        "expected TypeLevelEvalLimitExceeded in {:?}",
        lowered.diagnostics
    );
}

#[test]
fn poly_schemes_populated_for_inferred_identity() {
    // `id x = x` is polymorphic — poly_schemes[id] should be non-empty.
    let file = completed_file("id x = x\nid 42");
    assert!(
        !file.poly_schemes.is_empty(),
        "expected poly_schemes to be non-empty for polymorphic `id`"
    );
}

// ── Higher-order functions via record callback ────────────────────────────────

#[test]
fn function_field_in_record_called_correctly() {
    // A record holding an `Int -> Int` field; the function stored inside
    // is called on an argument.  Tests that field access yields a callable type.
    let file = completed_file(
        r#"
Callback :: type { fn : Int -> Int; }

runCallback :: Callback -> Int -> Int {
  | cb x => cb.fn x;
}

runCallback { fn = \n. n * 2; } 5
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn two_function_fields_composed_via_pipeline() {
    // Two `Int -> Int` fields stored in records; pipeline chains them.
    let file = completed_file(
        r#"
Fns :: type { first : Int -> Int; second : Int -> Int; }

applyBoth :: Fns -> Int -> Int {
  | fns x => x |> fns.first |> fns.second;
}

applyBoth { first = \n. n + 1; second = \n. n * 2; } 4
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn function_stored_in_let_binding_is_callable() {
    let file = completed_file(
        r#"
inc :: Int -> Int {
  | n => n + 1;
}

{
  fn := inc;
  fn 10
}
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn calling_non_function_field_reports_error() {
    // `x.val 5` where `val : Int` should raise ExpectedFunction.
    let lowered = lower(
        r#"
Rec :: type { val : Int; }

apply :: Rec -> Int -> Int {
  | r x => r.val x;
}

apply { val = 1; } 2
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::ExpectedFunction { .. }))
    );
}

// ── Pipeline desugaring and typing ───────────────────────────────────────────

#[test]
fn forward_pipeline_chain_yields_correct_type() {
    let file = completed_file(
        r#"
inc :: Int -> Int {
  | n => n + 1;
}

double :: Int -> Int {
  | n => n * 2;
}

3 |> inc |> double
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn backward_pipeline_single_step_yields_correct_type() {
    // Single `<|` step: `double <| 3` desugars to `double 3`.
    let file = completed_file(
        r#"
double :: Int -> Int {
  | n => n * 2;
}

double <| 3
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn backward_pipeline_chained_via_application_yields_int() {
    // Chain using function application then `<|`: `double <| inc 3`.
    // Application binds tighter, so this is `double <| (inc 3)`.
    let file = completed_file(
        r#"
inc :: Int -> Int {
  | n => n + 1;
}

double :: Int -> Int {
  | n => n * 2;
}

double <| inc 3
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

// ── Block expressions with locals ─────────────────────────────────────────────

#[test]
fn block_with_local_bindings_in_function_body() {
    let file = completed_file(
        r#"
compute :: Int -> Int {
  | n => {
    doubled := n * 2;
    incremented := doubled + 1;
    incremented
  };
}

compute 5
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn block_result_type_propagates_to_caller() {
    let file = completed_file(
        r#"
makeLabel :: Int -> Text {
  | n => {
    prefix := "value-";
    _ := n;
    prefix
  };
}

makeLabel 42
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

// ── If-else expressions ───────────────────────────────────────────────────────

#[test]
fn if_else_with_matching_branches_yields_correct_type() {
    let file = completed_file(r#"if true then 1 else 2"#);
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn nested_if_else_yields_correct_type() {
    let file = completed_file(r#"if true then (if false then 1 else 2) else 3"#);
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn if_else_branch_type_mismatch_reports_error() {
    let lowered = lower(r#"if true then 1 else "text""#);
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. }))
    );
}

// ── Arithmetic and boolean expressions ───────────────────────────────────────

#[test]
fn boolean_and_or_chain_yields_bool_type() {
    let file = completed_file(r#"(1 > 0) && (2 > 1) || false"#);
    assert!(matches!(final_type_kind(&file), TypeKind::Bool));
}

#[test]
fn integer_arithmetic_chain_yields_int_type() {
    let file = completed_file(r#"(1 + 2 * 3 - 4) / 1"#);
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn mixed_type_arithmetic_reports_error() {
    // `true + false` is already tested; `1 + true` produces a type-level error.
    let lowered = lower(r#"1 + true"#);
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostics for mixed-type arithmetic"
    );
}

// ── Multi-field record access chains ─────────────────────────────────────────

#[test]
fn nested_record_field_access_yields_correct_type() {
    let file = completed_file(
        r#"
Inner :: type { value : Int; }
Outer :: type { inner : Inner; }

o :: Outer = { inner = { value = 42; }; }

o.inner.value
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn access_text_field_on_nested_record() {
    let file = completed_file(
        r#"
Meta :: type { label : Text; count : Int; }
Config :: type { meta : Meta; enabled : Bool; }

cfg :: Config = {
  meta = { label = "prod"; count = 3; };
  enabled = true;
}

cfg.meta.label
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

// ── True higher-order functions (Fix A + Fix B) ───────────────────────────────

#[test]
fn hof_apply_with_signature_returns_int() {
    // `apply :: (Int -> Int) -> Int -> Int` — exercises Fix A (grouped type).
    // Before Fix A the `(Int -> Int)` parameter was a 1-element Tuple, making
    // the body's `f x` fail with ExpectedFunction.
    let file = completed_file(
        r#"
apply :: (Int -> Int) -> Int -> Int {
  | f x => f x;
}

apply (\n. n * 3) 4
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn hof_apply_signatureless_returns_int() {
    // `apply f x = f x` with no type annotation — exercises Fix B (infer
    // function type for unknown callee).  The solver must mint a fresh arrow
    // for `f` and confirm the result is Int from the concrete call.
    let file = completed_file(
        r#"
apply f x = f x

apply (\n. n + 1) 7
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn hof_apply_twice_with_signature_returns_int() {
    // `applyTwice :: (Int -> Int) -> Int -> Int` — exercises Fix A.
    let file = completed_file(
        r#"
applyTwice :: (Int -> Int) -> Int -> Int {
  | f x => f (f x);
}

applyTwice (\n. n + 1) 5
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn hof_compose_with_generic_signature_returns_int() {
    // `compose :: <A,B,C> (B -> C) -> (A -> B) -> A -> C` — exercises Fix A
    // for grouped types inside a polymorphic signature.
    let file = completed_file(
        r#"
compose :: <A, B, C> (B -> C) -> (A -> B) -> A -> C {
  | f g x => f (g x);
}

inc :: Int -> Int { | n => n + 1; }
double :: Int -> Int { | n => n * 2; }

compose double inc 3
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn hof_wrong_argument_type_reports_type_mismatch() {
    // Passing `Text` where `(Int -> Int)` is expected must produce TypeMismatch.
    let lowered = lower(
        r#"
apply :: (Int -> Int) -> Int -> Int {
  | f x => f x;
}

apply "not-a-function" 5
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch, got {:?}",
        lowered.diagnostics
    );
}

// ── Coalescing and optional access ────────────────────────────────────────────

#[test]
fn null_coalescing_on_optional_yields_unwrapped_type() {
    let file = completed_file(
        r#"
x :: Int? = #none

x ?? 0
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn opt_access_chained_with_coalesce() {
    let file = completed_file(
        r#"
Server :: type { port : Int; }

get_port :: Server? -> Int {
  | s => s?.port ?? 80;
}

get_port #none
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

// ── v1 Constraint / Witness THIR representation (Increment 2) ────────────────

/// Helper: find the first decl in `file.decls` whose kind matches the predicate.
fn find_decl_kind<'a, F>(file: &'a ThirFile, pred: F) -> Option<&'a ThirDeclKind>
where
    F: Fn(&ThirDeclKind) -> bool,
{
    file.decls
        .iter()
        .find(|&&id| pred(&file.decl_arena[id].kind))
        .map(|&id| &file.decl_arena[id].kind)
}

/// A constraint def + witness + normal binding all lower to a complete THIR file
/// with the expected structural presence.
#[test]
fn constraint_and_witness_produce_thir_decls() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = intEq; }\nintEq := 1\n42";
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
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. a; }\n1";
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
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = laterFn; }\nlaterFn := \\x. x\n1";
    let lowered = lower(src);
    assert!(
        lowered.file.is_some(),
        "forward-reference in witness field should not null LoweredThir.file; diagnostics: {:?}",
        lowered.diagnostics
    );
}

/// A `derive` witness lowers to `ThirDeclKind::Witness { derive: true, fields: [] }`.
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
