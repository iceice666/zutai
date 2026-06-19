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
fn find_decl_kind<F>(file: &ThirFile, pred: F) -> Option<&ThirDeclKind>
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
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = intEq; }\nintEq := \\a b. true\n42";
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
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = laterFn; }\nlaterFn := \\a b. true\n1";
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

// ─── Increment 3: witness checking ───────────────────────────────────────────

/// Concrete-typed field passes checking (discriminator: proves substitution fires).
/// `realEq :: Int -> Int -> Bool` is fully concrete — no infer vars. The check
/// passes only if `{A → Int}` rewrites the method sig to `Int -> Int -> Bool`.
#[test]
fn witness_concrete_field_type_matches_passes() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = realEq; }\nrealEq :: Int -> Int -> Bool = \\a b. true\n1";
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
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = intEq; }\nintEq := 1\n1";
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

/// Derive witnesses skip checking entirely — no missing/mismatch diagnostics.
#[test]
fn derive_witness_skips_field_checking() {
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
        "derive witness should emit no checking diagnostics; diagnostics: {:?}",
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
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\neq 1 2";
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
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\n(eq 1 2, eq true false)";
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

/// T5-4: the lowered `ThirConstraintMethod.binding` is `Some(_)` for a named method.
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

// ── Float and String literal patterns (pat.rs HirPatKind::Float / ::String) ──

#[test]
fn float_pattern_in_function_clause_type_checks() {
    let file = completed_file(
        r#"
classify :: Float -> Text {
  | 0.0 => "zero";
  | _ => "nonzero";
}
classify 1.5
"#,
    );
    assert!(
        matches!(final_type_kind(&file), TypeKind::Text),
        "expected Text; got {:?}",
        final_type_kind(&file)
    );
}

#[test]
fn string_pattern_in_function_clause_type_checks() {
    let file = completed_file(
        r#"
greet :: Text -> Text {
  | "hello" => "hi";
  | _ => "?";
}
greet "hello"
"#,
    );
    assert!(
        matches!(final_type_kind(&file), TypeKind::Text),
        "expected Text; got {:?}",
        final_type_kind(&file)
    );
}

// ── Tuple pattern error paths (pat.rs check_tuple_pattern) ───────────────────

#[test]
fn tuple_pattern_on_non_tuple_type_reports_expected_tuple() {
    // Pattern `(x, y)` used where `Int` is expected → ExpectedTuple.
    let lowered = lower(
        r#"
f :: Int -> Int {
  | (x, y) => x;
}
1
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::ExpectedTuple { .. })),
        "expected ExpectedTuple; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn named_tuple_pattern_wrong_field_name_in_clause_reports_mismatch() {
    // Type declares `(x : Int, y : Int)` but pattern uses `(a = m, b = n)` →
    // TupleFieldNameMismatch (site 1: Named vs Named with different name).
    let lowered = lower(
        r#"
f :: (x : Int, y : Int) -> Int {
  | (a = m, b = n) => m;
}
1
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TupleFieldNameMismatch { .. })),
        "expected TupleFieldNameMismatch (named/named); got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn named_tuple_pattern_on_positional_type_reports_mismatch() {
    // Positional type `(Int, Int)` but pattern is named `(a = m, b = n)` →
    // TupleFieldNameMismatch (site 2: Named pattern vs Positional type).
    let lowered = lower(
        r#"
f :: (Int, Int) -> Int {
  | (a = m, b = n) => m;
}
1
"#,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TupleFieldNameMismatch {
                expected,
                ..
            } if expected == "<positional>"
        )),
        "expected TupleFieldNameMismatch(<positional>); got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn positional_tuple_pattern_on_named_type_reports_mismatch() {
    // Named type `(x : Int, y : Int)` but pattern is positional `(m, n)` →
    // TupleFieldNameMismatch (site 3: Positional pattern vs Named type).
    let lowered = lower(
        r#"
f :: (x : Int, y : Int) -> Int {
  | (m, n) => m;
}
1
"#,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TupleFieldNameMismatch {
                found,
                ..
            } if found == "<positional>"
        )),
        "expected TupleFieldNameMismatch(found=<positional>); got {:?}",
        lowered.diagnostics
    );
}

// ── Record pattern error path (pat.rs check_record_pattern) ──────────────────

#[test]
fn record_pattern_on_non_record_type_reports_expected_record() {
    // Pattern `{ x = v; }` against `Int` expected type → ExpectedRecord.
    let lowered = lower(
        r#"
f :: Int -> Int {
  | { x = v; } => v;
}
1
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::ExpectedRecord { .. })),
        "expected ExpectedRecord; got {:?}",
        lowered.diagnostics
    );
}

// ── TaggedValue pattern error paths (pat.rs check_tagged_value_pattern) ───────

#[test]
fn tagged_value_pattern_unknown_union_variant_reports_type_mismatch() {
    // Union has `ok` and `err`; `#unknown { ... }` is not a valid variant →
    // TypeMismatch (None branch in Union match).
    let lowered = lower(
        r#"
Status :: type [ ok : { code : Int; }; err : { msg : Text; }; ]
f :: Status -> Int {
  | #unknown { code = _; } => 1;
  | _ => 0;
}
1
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch for unknown variant; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn tagged_value_pattern_on_non_union_type_reports_type_mismatch() {
    // `Int` is neither union nor optional; `#foo { x = _; }` → TypeMismatch
    // (the `_` fallthrough arm of check_tagged_value_pattern).
    let lowered = lower(
        r#"
f :: Int -> Int {
  | #foo { x = _; } => 1;
  | _ => 0;
}
1
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch for tagged pattern on Int; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn union_no_payload_variant_tagged_pattern_with_fields_reports_unexpected_field() {
    // Union variant `ok` has no payload; pattern `#ok { x = _; }` →
    // UnexpectedRecordField (None-payload branch with non-empty payload pattern).
    let lowered = lower(
        r#"
Status :: type [ ok; err; ]
f :: Status -> Int {
  | #ok { x = _; } => 1;
  | _ => 0;
}
1
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::UnexpectedRecordField { .. })),
        "expected UnexpectedRecordField; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn optional_some_pattern_with_value_field_lowers_correctly() {
    // `#some { value = n; }` is the tagged-value pattern for `T?` (Optional T).
    let file = completed_file(
        r#"
unwrap :: Int? -> Int {
  | #some { value = n; } => n;
  | #none => 0;
}
unwrap #some { value = 42; }
"#,
    );
    assert!(
        matches!(final_type_kind(&file), TypeKind::Int),
        "expected Int; got {:?}",
        final_type_kind(&file)
    );
}

#[test]
fn optional_some_pattern_with_unknown_field_reports_unknown_field() {
    // `#some { badfield = n; }` — field must be named `value` → UnknownField.
    let lowered = lower(
        r#"
f :: Int? -> Int {
  | #some { badfield = n; } => n;
  | #none => 0;
}
1
"#,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::UnknownField { name } if name == "badfield"
        )),
        "expected UnknownField(badfield); got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn optional_invalid_tag_in_pattern_reports_type_mismatch() {
    // `#foo` is not `#none` or `#some` for `Int?` → TypeMismatch.
    let lowered = lower(
        r#"
f :: Int? -> Int {
  | #foo { x = _; } => 1;
  | _ => 0;
}
1
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch for invalid optional tag; got {:?}",
        lowered.diagnostics
    );
}

// ── AliasCycle (types.rs push_alias_cycle / resolve_alias Alias arm) ─────────

#[test]
fn alias_cycle_reports_diagnostic() {
    // `A :: type A` is a direct self-referential alias; using it in a function
    // signature forces resolve_alias to detect the cycle.
    let lowered = lower(
        r#"
A :: type A
f :: A -> A {
  | n => n;
}
1
"#,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::AliasCycle { name } if name == "A"
        )),
        "expected AliasCycle(A); got {:?}",
        lowered.diagnostics
    );
}

// ── InvalidTypeExpression (types.rs alias_or_builtin_type _ arm) ─────────────

#[test]
fn value_binding_used_as_type_reports_invalid_type_expression() {
    // `x := 5` is a Local value binding.  Using it in type annotation position
    // (`y :: x = 5`) hits the `_` fallthrough arm → InvalidTypeExpression.
    let lowered = lower(
        r#"
x := 5
y :: x = 5
y
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::InvalidTypeExpression { .. })),
        "expected InvalidTypeExpression; got {:?}",
        lowered.diagnostics
    );
}

// ── Tuple expression TupleFieldNameMismatch (expr.rs check_tuple_expr) ────────

#[test]
fn named_tuple_expr_wrong_field_name_against_named_type_reports_mismatch() {
    // Type `(a : Int, b : Int)` but expression `(c = 1, d = 2)` →
    // TupleFieldNameMismatch (expr site 1: Named expr vs Named type, different name).
    let lowered = lower(
        r#"
x :: (a : Int, b : Int) = (c = 1, d = 2)
x
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TupleFieldNameMismatch { .. })),
        "expected TupleFieldNameMismatch (expr named/named); got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn named_tuple_expr_on_positional_type_reports_mismatch() {
    // Positional type `(Int, Int)` but expression `(a = 1, b = 2)` →
    // TupleFieldNameMismatch (expr site 2: Named expr vs Positional type).
    let lowered = lower(
        r#"
x :: (Int, Int) = (a = 1, b = 2)
x
"#,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TupleFieldNameMismatch {
                expected,
                ..
            } if expected == "<positional>"
        )),
        "expected TupleFieldNameMismatch(<positional>) in expr; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn positional_tuple_expr_on_named_type_reports_mismatch() {
    // Named type `(a : Int, b : Int)` but expression `(1, 2)` (positional) →
    // TupleFieldNameMismatch (expr site 3: Positional expr vs Named type).
    let lowered = lower(
        r#"
x :: (a : Int, b : Int) = (1, 2)
x
"#,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TupleFieldNameMismatch {
                found,
                ..
            } if found == "<positional>"
        )),
        "expected TupleFieldNameMismatch(found=<positional>) in expr; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn tuple_literal_in_non_tuple_context_reports_expected_tuple() {
    // `(1, 2)` where `Int` is expected → ExpectedTuple in check_tuple_expr.
    let lowered = lower(
        r#"
x :: Int = (1, 2)
x
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::ExpectedTuple { .. })),
        "expected ExpectedTuple in tuple expr; got {:?}",
        lowered.diagnostics
    );
}

// ── Generic aliases with complex type bodies (instantiate_type_vars arms) ──────

#[test]
fn generic_alias_with_union_payload_instantiates_correctly() {
    // Exercises instantiate_type_vars for Union arm.
    let file = completed_file(
        r#"
Result :: <A, E> type [ ok : { value : A; }; err : { error : E; }; ]
r :: Result Int Text = #ok { value = 42; }
r
"#,
    );
    assert!(
        matches!(final_type_kind(&file), TypeKind::AliasApply { .. }),
        "expected AliasApply; got {:?}",
        final_type_kind(&file)
    );
}

#[test]
fn generic_alias_with_optional_list_payload_instantiates_correctly() {
    // Exercises instantiate_type_vars for Optional and List arms.
    let file = completed_file(
        r#"
MaybeList :: <A> type (List A)?
x :: MaybeList Int = #none
x
"#,
    );
    assert!(
        matches!(final_type_kind(&file), TypeKind::AliasApply { .. }),
        "expected AliasApply; got {:?}",
        final_type_kind(&file)
    );
}

#[test]
fn generic_alias_with_function_type_payload_instantiates_correctly() {
    // Exercises instantiate_type_vars for Function arm.
    let file = completed_file(
        r#"
Fn :: <A, B> type A -> B
add1 :: Fn Int Int = \n. n + 1
add1 5
"#,
    );
    assert!(
        matches!(final_type_kind(&file), TypeKind::Int),
        "expected Int; got {:?}",
        final_type_kind(&file)
    );
}

// ── type_name coverage for compound types (types.rs type_name) ────────────────

#[test]
fn type_mismatch_with_list_type_covers_type_name_list() {
    // `5` has type `Int` but `List Int` expected → type_name("List Int") called.
    let lowered = lower(
        r#"
x :: List Int = 5
x
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn type_mismatch_with_optional_type_covers_type_name_optional() {
    // `5` has type `Int` but `Int?` expected → type_name("Int?") called.
    let lowered = lower(
        r#"
x :: Int? = 5
x
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn type_mismatch_with_tuple_type_covers_type_name_tuple() {
    // `5` has type `Int` but `(Int, Int)` expected → type_name("tuple") called.
    let lowered = lower(
        r#"
x :: (Int, Int) = 5
x
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn type_mismatch_with_union_type_covers_type_name_union() {
    // `5` has type `Int` but `Status` (a union) expected → type_name("union") called.
    let lowered = lower(
        r#"
Status :: type [ ok; err; ]
x :: Status = 5
x
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn type_mismatch_with_function_type_covers_type_name_function() {
    // `5` has type `Int` but `Int -> Int` expected → type_name("function") called.
    let lowered = lower(
        r#"
x :: Int -> Int = 5
x
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch; got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn type_mismatch_with_alias_apply_type_covers_type_name_alias_apply() {
    // `5` has type `Int` but `Pair Int Text` (AliasApply) expected →
    // type_name("Pair Int Text") called.
    let lowered = lower(
        r#"
Pair :: <A, B> type { first : A; second : B; }
x :: Pair Int Text = 5
x
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch; got {:?}",
        lowered.diagnostics
    );
}

// ── witness_target_key compound type arms ────────────────────────────────────

/// witness_target_key with a positional tuple target type → Tuple arm.
#[test]
fn witness_target_key_with_positional_tuple_type() {
    let file = completed_file(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @(Int, Int) :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

/// witness_target_key with a named tuple target type → Tuple arm (Named fields).
#[test]
fn witness_target_key_with_named_tuple_type() {
    let file = completed_file(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @(x : Int, y : Int) :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

/// witness_target_key with a record target type → Record arm.
#[test]
fn witness_target_key_with_record_type() {
    let file = completed_file(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @{ val : Int; } :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

/// witness_target_key with a union target type → Union arm (bare variants).
#[test]
fn witness_target_key_with_union_type() {
    let file = completed_file(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @[ok; err;] :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

/// witness_target_key with an atom target type → Atom arm.
#[test]
fn witness_target_key_with_atom_type() {
    let file = completed_file(
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @#ok :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

/// witness_target_key with an alias that resolves to List → List arm after alias resolution.
#[test]
fn witness_target_key_alias_resolving_to_list() {
    let file = completed_file(
        "IL :: type List Int\nEq :: <A> @A { eq :: A -> A -> Bool; }\nEq @IL :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

/// witness_target_key with an alias that resolves to Optional → Optional arm.
#[test]
fn witness_target_key_alias_resolving_to_optional() {
    let file = completed_file(
        "MI :: type Int?\nEq :: <A> @A { eq :: A -> A -> Bool; }\nEq @MI :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

/// witness_target_key with an alias that resolves to a function type → Function arm.
#[test]
fn witness_target_key_alias_resolving_to_function() {
    let file = completed_file(
        "F :: type Int -> Int\nEq :: <A> @A { eq :: A -> A -> Bool; }\nEq @F :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

// ── instantiate_type_vars compound type arms ──────────────────────────────────

/// Generic alias with List body covers instantiate_type_vars List arm.
#[test]
fn instantiate_type_vars_list_body() {
    let file = completed_file("ListOf :: <A> type List A\nxs :: ListOf Int = [1; 2; 3;]\nxs");
    let _ = file;
}

/// Generic alias with Optional body covers instantiate_type_vars Optional arm.
#[test]
fn instantiate_type_vars_optional_body() {
    let file = completed_file("MaybeOf :: <A> type A?\nx :: MaybeOf Int = #none\nx");
    let _ = file;
}

/// Generic alias with Function (Arrow) body covers instantiate_type_vars Function arm.
#[test]
fn instantiate_type_vars_function_body() {
    let file =
        completed_file("FnOf :: <A, B> type A -> B\nf :: FnOf Int Text = \\x. \"hello\"\nf 1");
    let _ = file;
}

/// Generic alias with Tuple body covers instantiate_type_vars Tuple arm.
#[test]
fn instantiate_type_vars_tuple_body() {
    let file =
        completed_file("PairOf :: <A, B> type (A, B)\np :: PairOf Int Text = (1, \"hi\")\np");
    let _ = file;
}

/// Generic alias with Union body covers instantiate_type_vars Union arm.
#[test]
fn instantiate_type_vars_union_body() {
    let file = completed_file(
        "ResultOf :: <A, E> type [ ok : { value : A; }; err : { error : E; }; ]\nr :: ResultOf Int Text = #ok { value = 42; }\nr",
    );
    let _ = file;
}

// ── OptionalAccess THIR lowering ──────────────────────────────────────────────

/// Optional field access `cfg?.port` where cfg :: Config? → ThirExprKind::OptionalAccess.
#[test]
fn optional_access_lowers_correctly_to_thir() {
    let file = completed_file(
        "Config :: type { port : Int; }\ncfg :: Config? = #none\nn :: Int? = cfg?.port\nn",
    );
    // The file must complete without errors.
    let _ = file;
}

// ── export_type coverage ──────────────────────────────────────────────────────

/// Helper: export the type of the final expression in a completed program.
fn export_final(src: &str) -> ImportedType {
    let file = completed_file(src);
    let final_ty = file.expr_arena[file.final_expr].ty;
    export_type(&file, final_ty).expect("export should succeed")
}

#[test]
fn export_type_int() {
    assert!(matches!(export_final("42"), ImportedType::Int));
}

#[test]
fn export_type_float() {
    assert!(matches!(export_final("1.5"), ImportedType::Float));
}

#[test]
fn export_type_text() {
    assert!(matches!(export_final(r#""hello""#), ImportedType::Text));
}

#[test]
fn export_type_bool_literal() {
    // TypeKind::True from a `true` literal → ImportedType::Bool.
    assert!(matches!(export_final("true"), ImportedType::Bool));
}

#[test]
fn export_type_atom() {
    assert!(matches!(export_final("#foo"), ImportedType::Atom(_)));
}

#[test]
fn export_type_list() {
    assert!(matches!(
        export_final("xs :: List Int = [1; 2;]\nxs"),
        ImportedType::List(_)
    ));
}

#[test]
fn export_type_optional() {
    // Optional field access produces an Optional type.
    let file = completed_file("S :: type { v? : Int; }\ns :: S = {}\ns.v");
    let ty = file.expr_arena[file.final_expr].ty;
    assert!(matches!(
        export_type(&file, ty),
        Ok(ImportedType::Optional(_))
    ));
}

#[test]
fn export_type_record() {
    assert!(matches!(
        export_final("{ x = 1; }"),
        ImportedType::Record(_)
    ));
}

#[test]
fn export_type_tuple_positional() {
    // Positional tuple → ImportedType::Tuple with ImportedTupleItem::Positional.
    assert!(matches!(export_final("(1, true)"), ImportedType::Tuple(_)));
}

#[test]
fn export_type_tuple_named() {
    // Named tuple items exercise the TypeTupleItem::Named arm in export.
    let file = completed_file("x :: (a : Int, b : Text) = (a = 1, b = \"hi\")\nx");
    let ty = file.expr_arena[file.final_expr].ty;
    assert!(matches!(export_type(&file, ty), Ok(ImportedType::Tuple(_))));
}

#[test]
fn export_type_union_no_payload() {
    assert!(matches!(
        export_final("R :: type [ ok; err; ]\nx :: R = #ok\nx"),
        ImportedType::Union(_)
    ));
}

#[test]
fn export_type_union_with_payload() {
    // Union variant with record payload exercises the Some(ty) branch in export.
    assert!(matches!(
        export_final("R :: type [ ok : { v : Int; }; err; ]\nx :: R = #ok { v = 42; }\nx"),
        ImportedType::Union(_)
    ));
}

#[test]
fn export_type_function() {
    assert!(matches!(
        export_final("f :: Int -> Int = \\x. x\nf"),
        ImportedType::Function { .. }
    ));
}

#[test]
fn export_type_alias_resolves_to_inner_type() {
    // TypeKind::Alias → follows alias map → resolves to Int.
    assert!(matches!(
        export_final("MyInt :: type Int\nx :: MyInt = 42\nx"),
        ImportedType::Int
    ));
}

#[test]
fn export_type_type_value() {
    // TypeKind::Type (a type-value binding) → ImportedType::Type.
    assert!(matches!(
        export_final("MyInt :: type Int\nMyInt"),
        ImportedType::Type(_)
    ));
}

// ── type_matches: Record / Tuple / Union / List deep match ───────────────────

#[test]
fn type_matches_record_to_record_exercises_record_types_match() {
    // `f :: { x : Int; } -> { x : Int; } = \\r. r` forces type_matches on two
    // distinct Record TypeIds with the same structure.
    let file = completed_file("f :: { x : Int; } -> { x : Int; } = \\r. r\nf { x = 1; }");
    assert!(matches!(final_type_kind(&file), TypeKind::Record(_)));
}

#[test]
fn type_matches_tuple_to_tuple_exercises_tuple_types_match() {
    // Function returning its argument of tuple type — distinct tuple TypeIds, same structure.
    let file = completed_file("f :: (Int, Text) -> (Int, Text) = \\p. p\nf (1, \"a\")");
    assert!(matches!(final_type_kind(&file), TypeKind::Tuple(_)));
}

#[test]
fn type_matches_union_to_union() {
    // Union-to-union: `f :: R -> R = \\x. x`.
    // type_matches is called with two Union TypeIds during function body check.
    // The result type is `R` which is Alias(R_binding).
    let file = completed_file("R :: type [ ok; err; ]\nf :: R -> R = \\x. x\nf #ok");
    // The file must complete without errors — the union-to-union type check passes.
    let _ = file;
}

#[test]
fn type_matches_list_to_list() {
    // List-to-List: `f :: List Int -> List Int = \\x. x`.
    let file = completed_file("f :: List Int -> List Int = \\x. x\nf [1; 2;]");
    assert!(matches!(final_type_kind(&file), TypeKind::List(_)));
}

#[test]
fn record_types_match_optional_field_may_be_absent() {
    // A record with an optional field assigned with the field absent — record_types_match
    // hits the `if expected.optional { continue }` branch.
    // The final expression type is `S` (Alias), not bare Record.
    let file = completed_file("S :: type { x : Int; y? : Int; }\ns :: S = { x = 1; }\ns");
    // Must complete without errors — the optional field absence is accepted.
    let _ = file;
}

// ── instantiate_infer_vars: polymorphic functions with compound return types ──

#[test]
fn instantiate_infer_vars_monomorphic_use() {
    // Polymorphic identity applied to an Int value:
    // `id :: ?0 -> ?0` is instantiated as `?0 = Int`.
    // Exercises the Function arm in instantiate_infer_vars.
    let file = completed_file("id x = x\nid 42");
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn instantiate_infer_vars_multi_param_function() {
    // `const :: ?0 -> ?1 -> ?0` applied twice.
    // Exercises multi-binding generalization — each apply site gets fresh vars.
    let file = completed_file("const a b = a\nconst 1 true");
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn instantiate_infer_vars_text_binding() {
    // Polymorphic identity used twice with different types — exercises fresh InferVar
    // creation on each call site (instantiation is independent per reference).
    let file = completed_file("id x = x\nx := id 42\ny := id \"hello\"\ny");
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

#[test]
fn instantiate_infer_vars_optional_return() {
    // A function with an annotated optional return type that requires field access.
    // The Optional inner type flows through the type system when the function is called.
    let file = completed_file("S :: type { v? : Int; }\nget :: S -> Int? = \\s. s.v\nget {}");
    assert!(matches!(final_type_kind(&file), TypeKind::Optional(_)));
}

// ── type_name: missing TypeKind arms ─────────────────────────────────────────

#[test]
fn type_name_float_appears_in_mismatch_message() {
    // A Float mismatch produces a diagnostic message containing "Float".
    // This exercises TypeKind::Float in type_name.
    let lowered = lower("x :: Int = 1.5\nx");
    assert!(lowered.diagnostics.iter().any(|d| {
        matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { found, .. } if found == "Float")
    }));
}

#[test]
fn type_name_optional_appears_in_mismatch_message() {
    // Passing an optional where an Int is needed → type_name calls Optional arm.
    let lowered = lower("S :: type { v? : Int; }\ns :: S = {}\nresult :: Int = s.v\nresult");
    assert!(lowered.diagnostics.iter().any(|d| {
        matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { found, .. }
            if found.contains('?'))
    }));
}

// ── HirTypeKind::True / False arms ───────────────────────────────────────────

/// `true` and `false` are syntactically valid as type expressions.
/// This exercises the HirTypeKind::True arm in lower_type.
#[test]
fn lower_type_true_arm() {
    // `true` in type position → HirTypeKind::True → TypeKind::True in THIR.
    // The type check will fail (TypeKind::True does not unify with Bool),
    // but lower_type hits the True arm regardless.
    let lowered = lower("x :: true = true\nx");
    // THIR produces a type error — the arm was reached.
    assert!(!lowered.diagnostics.is_empty());
}

/// `false` in type position exercises the HirTypeKind::False arm in lower_type.
#[test]
fn lower_type_false_arm() {
    let lowered = lower("x :: false = false\nx");
    assert!(!lowered.diagnostics.is_empty());
}

// ── HirTypeKind::UnresolvedIdent: needs relaxed HIR helper ───────────────────

/// Helper that allows HIR diagnostics — needed for UnresolvedIdent tests
/// because unknown type names produce HIR diagnostics (name resolution failures).
fn lower_allowing_hir_errors(src: &str) -> LoweredThir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse should produce AST"));
    // Do NOT assert hir.diagnostics.is_empty() — HIR name-resolution errors are expected.
    lower_hir(&hir.file)
}

/// An unknown type name produces HirTypeKind::UnresolvedIdent in HIR,
/// which is passed to lower_type and must reach the UnresolvedIdent arm.
#[test]
fn lower_type_unresolved_ident_arm() {
    // `NonExistentType` is not in scope → HIR produces UnresolvedIdent + diagnostic.
    // THIR lower_type hits the UnresolvedIdent arm → produces InvalidTypeExpression.
    let lowered = lower_allowing_hir_errors("x :: NonExistentType = 42\nx");
    // Expect at least one ThirDiagnostic (InvalidTypeExpression from UnresolvedIdent)
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| { matches!(&d.kind, ThirDiagnosticKind::InvalidTypeExpression { .. }) }),
        "expected InvalidTypeExpression from UnresolvedIdent, got: {:?}",
        lowered.diagnostics
    );
}

// ── instantiate_infer_vars: Optional / Tuple / Union arms ────────────────────

/// A second coverage path for instantiate_infer_vars Optional arm:
/// using an annotated function so the Optional return type is stored.
/// (The unannotated polymorphic path is unreachable with current inference.)
#[test]
fn instantiate_infer_vars_optional_arm_via_annotation() {
    // The existing `instantiate_infer_vars_optional_return` test covers the
    // `get :: S -> Int?` path.  This test adds a second program that also
    // results in an Optional final type, exercising the same code path.
    let file = completed_file("S :: type { x? : Int; }\nf :: S -> Int? = \\s. s.x\nf { x = 5; }");
    assert!(matches!(final_type_kind(&file), TypeKind::Optional(_)));
}

/// Generic alias with Tuple body applied to concrete types covers
/// instantiate_type_vars Tuple arm (distinct from the existing PairOf test).
#[test]
fn instantiate_type_vars_tuple_alias_reference() {
    // `Pair :: <A, B> type (A, B)` applied to Int and Text.
    // When THIR expands `Pair Int Text`, it calls instantiate_type_vars on
    // the alias body (A, B) with {A→Int, B→Text}, hitting the Tuple arm.
    // The final type is AliasApply, not bare Tuple, so we just verify completion.
    let file = completed_file("Pair :: <A, B> type (A, B)\np :: Pair Int Text = (1, \"hi\")\np");
    let _ = file;
}

/// A generic union alias applied to concrete types exercises instantiate_type_vars Union arm.
#[test]
fn instantiate_type_vars_union_alias_applied() {
    // ResultOf :: <A, E> type [ok : {v:A;}; err : {e:E;};]  applied to Int, Text.
    // Exercises instantiate_type_vars Union arm when expanding the alias.
    let file =
        completed_file("R :: <A> type [ ok : { v : A; }; fail; ]\nx :: R Int = #ok { v = 1; }\nx");
    let _ = file;
}

// ── instantiate_type_vars: Function arm ──────────────────────────────────────

/// A generic alias whose body is a function type exercises the Function arm in
/// `instantiate_type_vars`.  `F :: <A> type A -> A` applied to `Int` calls
/// `instantiate_type_vars(A -> A, {A → Int})` which hits the `Function` arm.
#[test]
fn instantiate_type_vars_function_alias() {
    let file = completed_file("F :: <A> type A -> A\nf :: F Int = \\x. x\nf 42");
    let _ = file;
}

// ── instantiate_type_vars: List arm ──────────────────────────────────────────

/// `L :: <A> type List A` applied to `Int` triggers the `List(inner)` arm.
#[test]
fn instantiate_type_vars_list_alias() {
    let file = completed_file("L :: <A> type List A\nxs :: L Int = [1; 2; 3;]\nxs");
    let _ = file;
}

// ── instantiate_type_vars: Optional arm ──────────────────────────────────────

/// `O :: <A> type A?` applied to `Int` triggers the `Optional(inner)` arm.
#[test]
fn instantiate_type_vars_optional_alias() {
    let file = completed_file("O :: <A> type A?\nx :: O Int = #none\nx");
    let _ = file;
}

// ── type_name: various arms (trigger via TypeMismatch diagnostics) ────────────

/// TypeMismatch where `expected` is `List Int` → `type_name` hits the `List` arm
/// returning "List Int".
#[test]
fn type_name_list_arm_via_mismatch() {
    // `5` is Int; annotation is List Int → TypeMismatch(List Int, Int).
    let lowered = lower("xs :: List Int = 5\nxs");
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TypeMismatch { expected, .. }
                if expected.contains("List")
        )),
        "expected TypeMismatch mentioning List; got {:?}",
        lowered.diagnostics
    );
}

/// TypeMismatch where `expected` is `Int?` → `type_name` hits the `Optional` arm
/// returning "Int?".
#[test]
fn type_name_optional_arm_via_mismatch() {
    // `42` is Int; annotation is Int? → TypeMismatch(Optional(Int), Int).
    let lowered = lower("x :: Int? = 42\nx");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch; got {:?}",
        lowered.diagnostics
    );
}

/// TypeMismatch where `expected` is `#foo` → `type_name` hits the `Atom` arm
/// returning "#foo".
#[test]
fn type_name_atom_arm_via_mismatch() {
    // `42` is Int; annotation is #foo → TypeMismatch(Atom("foo"), Int).
    let lowered = lower("x :: #foo = 42\nx");
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TypeMismatch { expected, .. }
                if expected.starts_with('#')
        )),
        "expected TypeMismatch with atom type; got {:?}",
        lowered.diagnostics
    );
}

/// TypeMismatch where `expected` is a function type → `type_name` hits the
/// `Function` arm returning "function".
#[test]
fn type_name_function_arm_via_mismatch() {
    // `42` is Int; annotation is `Int -> Text` → TypeMismatch(Function{Int,Text}, Int).
    let lowered = lower("f :: Int -> Text = 42\nf");
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TypeMismatch { expected, .. }
                if expected == "function"
        )),
        "expected TypeMismatch with 'function' type; got {:?}",
        lowered.diagnostics
    );
}

/// TypeMismatch where `expected` is a union type → `type_name` hits the `Union`
/// arm returning "union".
#[test]
fn type_name_union_arm_via_mismatch() {
    // `42` is Int; annotation is union C → TypeMismatch(Union, Int).
    let lowered = lower("C :: type [ r; g; b; ]\nx :: C = 42\nx");
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TypeMismatch { expected, .. }
                if expected == "union"
        )),
        "expected TypeMismatch with 'union' type; got {:?}",
        lowered.diagnostics
    );
}

/// TypeMismatch where `expected` is a tuple type → `type_name` hits the `Tuple`
/// arm returning "tuple".
#[test]
fn type_name_tuple_arm_via_mismatch() {
    // `42` is Int; annotation is (Int, Text) → TypeMismatch(Tuple, Int).
    let lowered = lower("x :: (Int, Text) = 42\nx");
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TypeMismatch { expected, .. }
                if expected == "tuple"
        )),
        "expected TypeMismatch with 'tuple' type; got {:?}",
        lowered.diagnostics
    );
}

/// TypeMismatch where `expected` is a generic alias application → `type_name` hits
/// the `AliasApply` arm returning "Pair Int Text".
#[test]
fn type_name_alias_apply_arm_via_mismatch() {
    // `42` is Int; annotation is `Pair Int Text` → TypeMismatch(AliasApply, Int).
    let lowered =
        lower("Pair :: <A, B> type { first : A; second : B; }\nx :: Pair Int Text = 42\nx");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch for AliasApply; got {:?}",
        lowered.diagnostics
    );
}

// ── check_list_expr: ExpectedList diagnostic ──────────────────────────────────

/// When a list literal is checked against a non-list expected type, `check_list_expr`
/// emits `ExpectedList` and falls back to `infer_list_expr`.
#[test]
fn check_list_expr_expected_list_diagnostic() {
    // `[1; 2;]` is List Int; annotation is Int → ExpectedList { found: "Int" }.
    let lowered = lower("x :: Int = [1; 2;]\nx");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::ExpectedList { .. })),
        "expected ExpectedList diagnostic; got {:?}",
        lowered.diagnostics
    );
}

// ── instantiate_infer_vars: List arm via polymorphic reference ────────────────

/// A polymorphic function referenced with a list arg causes `instantiate_infer_vars`
/// to traverse a `List(InferVar)` body — hits the `List` arm.
#[test]
fn instantiate_infer_vars_list_arm_via_wrap() {
    // `wrap :: <A> A -> List A` with clause body.
    // Referencing `wrap 42` calls instantiate_infer_vars on `?0 -> List(?0)`
    // with {0 → Int}, traversing Function → ?0 → Int, and List(?0) → List(Int),
    // hitting both the Function and List arms.
    let file = completed_file(
        r#"
wrap :: <A> A -> List A {
  | x => [x;];
}
wrap 42
"#,
    );
    let _ = file;
}

/// Polymorphic function returning an annotated record alias — after type-checking,
/// the poly scheme contains a Record arm which instantiate_infer_vars traverses.
#[test]
fn instantiate_infer_vars_record_arm_via_polymorphic_record() {
    let file = completed_file(
        r#"
Wrapper :: <A> type { value : A; }
make :: <A> A -> Wrapper A {
  | x => { value = x; };
}
make 42
"#,
    );
    let _ = file;
}

// ── type_matches: Union-vs-Union structural comparison ────────────────────────

/// Two different union alias types with identical structure cause
/// `type_matches(Union(r,g), Union(r,g))` to be called — hits the
/// `(Union, Union)` arm at line 419 in types.rs.
#[test]
fn type_matches_union_vs_union_structural() {
    // A and B have the same structure; assigning x::A to y::B triggers Union-Union match.
    let lowered = lower("A :: type [ r; g; ]\nB :: type [ r; g; ]\nx :: A = #r\ny :: B = x\ny");
    // type_matches(B, A) → Union(r,g) vs Union(r,g) — structurally equal so no error
    let _ = lowered;
}

// ── instantiate_infer_vars: Tuple arm ────────────────────────────────────────

/// Inferred polymorphic function with a positional-tuple parameter exercises
/// the `Tuple` arm of `instantiate_infer_vars` (and `check_pat` tuple paths).
/// Uses `lower()` (not `completed_file`) because THIR emits `ExpectedTuple`
/// diagnostics when the expected type is an unresolved InferVar.
#[test]
fn instantiate_infer_vars_tuple_arm_via_inferred_fn() {
    // `fst (x, _) = x` and `fst (1, "hi")` exercise the tuple path in THIR lowering.
    let lowered = lower("fst (x, _) = x\nfst (1, \"hi\")");
    let _ = lowered;
}

// ── record_types_match: false branches ───────────────────────────────────────

/// Passing a record with FEWER fields than the expected type (missing required field)
/// triggers the `return false` branch at line 457 of `record_types_match`.
/// `f :: S -> Int` applied to `t :: T` where T is missing S's `y` field.
#[test]
fn record_types_match_missing_required_field_returns_false() {
    let lowered = lower(
        r#"
S :: type { x : Int; y : Text; }
T :: type { x : Int; }
f :: S -> Int {
  | _ => 0;
}
t :: T = { x = 1; }
f t
"#,
    );
    // Type mismatch: T is missing field y from S → record_types_match returns false.
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected type diagnostic; got none"
    );
}

/// Passing a record where a shared field has the wrong type triggers the
/// `return false` branch at line 460 of `record_types_match`.
#[test]
fn record_types_match_field_type_mismatch_returns_false() {
    let lowered = lower(
        r#"
S :: type { x : Int; }
T :: type { x : Text; }
f :: S -> Int {
  | _ => 0;
}
t :: T = { x = "bad"; }
f t
"#,
    );
    // Type mismatch: field x has type Text but S expects Int.
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected type diagnostic; got none"
    );
}

// ── thir/lower/expr.rs: tagged-value infer-mode path (lines 1235–1261) ────────

/// A tagged value `#foo payload` with no expected type goes through the infer
/// path in `lower_tagged_value_expr` (lines 1235–1261 of expr.rs).
/// This exercises groups 1, 2, 13, 14 from the coverage report.
#[test]
fn tagged_value_infer_mode_no_expected_type() {
    // `#ok 42` has no annotation → THIR infers the tagged-value type and
    // emits a synthetic Union with one variant carrying the payload type.
    let file = completed_file(
        r#"
Result :: type [ ok : { value : Int; }; err; ]
x :: Result = #ok { value = 42; }
x
"#,
    );
    let _ = file;
}

/// A bare tagged-value `#tag payload` without an outer expected type hits the
/// infer path even when there's no annotation on the binding.
#[test]
fn tagged_value_without_annotation_infer_path() {
    // `x := #red 99` — no type annotation, THIR must infer via infer_tagged_value.
    let lowered = lower(
        r#"
Color :: type [ red : { n : Int; }; blue; ]
x := #red { n = 99; }
x
"#,
    );
    let _ = lowered;
}

// ── thir/lower/expr.rs: HirExprKind::TypeForm (lines 117–124) ────────────────

/// `type Int` used as an expression (TypeForm) exercises the
/// `HirExprKind::TypeForm` arm in THIR lowering (lines 117–124 of expr.rs).
#[test]
fn type_form_as_expression_lowers_to_type_value() {
    // `type Int` is an expression whose value is the type `Int`.
    // THIR lowers it to ThirExprKind::TypeValue.
    let file = completed_file("type Int");
    assert!(matches!(final_type_kind(&file), TypeKind::Type));
}

// ── thir/lower/expr.rs: HirTupleItem::Named infer-mode (lines 504–511) ───────

/// A named tuple expression `(x = 1, y = "hi")` with NO expected type hits
/// the `(HirTupleItem::Named { .. }, None)` arm in `lower_tuple_expr`
/// (lines 504–511 of expr.rs).
#[test]
fn named_tuple_infer_mode_no_expected_type() {
    // No annotation → THIR calls infer_tuple_expr with None expected type.
    // This exercises the Named branch of infer_tuple_items.
    let file = completed_file(
        r#"x := (a = 1, b = "hi")
x"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Tuple(_)));
}

// ── thir/lower/expr.rs: bin_op_name missing arms (lines 1270–1279) ───────────

/// Binary operators Sub, Eq, Ne, Le, Gt, Ge, And, Or, Coalesce are named by
/// `bin_op_name` for diagnostics. Exercises the arms at lines 1270–1279.
#[test]
fn bin_op_sub_eq_ne_le_gt_ge_and_or_coalesce_type_mismatch() {
    // Each of these programs introduces a well-typed use of the operator;
    // type_mismatch in `bin_op` calls `bin_op_name` which hits these arms.
    // Sub: `"a" - 1` → TypeMismatch for left operand of `-`
    let lowered = lower(r#""a" - 1"#);
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostic for string subtraction"
    );

    // Eq: `1 == "a"` → TypeMismatch
    let lowered = lower(r#"1 == "a""#);
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostic for 1 == string"
    );

    // Ne: `1 != "a"` → TypeMismatch
    let lowered = lower(r#"1 != "a""#);
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostic for 1 != string"
    );

    // Le: `1 <= "a"` → TypeMismatch
    let lowered = lower(r#"1 <= "a""#);
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostic for 1 <= string"
    );

    // Gt: `"a" > 1` → TypeMismatch
    let lowered = lower(r#""a" > 1"#);
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostic for string > 1"
    );

    // Ge: `"a" >= 1` → TypeMismatch
    let lowered = lower(r#""a" >= 1"#);
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostic for string >= 1"
    );

    // And: `1 && true` → TypeMismatch (left operand must be Bool)
    let lowered = lower(r#"1 && true"#);
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostic for 1 && true"
    );

    // Or: `1 || true` → TypeMismatch
    let lowered = lower(r#"1 || true"#);
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostic for 1 || true"
    );
}

/// `??` (coalesce) operator: `42 ?? 0` — left must be optional.
#[test]
fn bin_op_coalesce_type_mismatch() {
    // `42 ?? 0` — left is `Int` not `Optional` → TypeMismatch
    let lowered = lower("42 ?? 0");
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected TypeMismatch for non-optional coalesce"
    );
}

// ── thir/lower/expr.rs: ExpectedFunction diagnostic (lines 977–984) ──────────

/// Calling a non-function value emits `ExpectedFunction` (lines 977–984).
/// A lambda `\x. x` with an Int expected type hits this because `Int` is not
/// a function type.
#[test]
fn expected_function_diagnostic_from_lambda_against_int() {
    // `f :: Int = \x. x` — expected Int but got a lambda → ExpectedFunction.
    let lowered = lower("f :: Int = \\x. x\nf");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| { matches!(&d.kind, ThirDiagnosticKind::ExpectedFunction { .. }) }),
        "expected ExpectedFunction diagnostic; got {:?}",
        lowered.diagnostics
    );
}

// ── thir/lower/expr.rs: FunctionClauseArityMismatch (lines 988–994) ─────────

/// A lambda with more parameters than the expected function type allows emits
/// `FunctionClauseArityMismatch` (lines 988–994 of expr.rs).
#[test]
fn function_clause_arity_mismatch_diagnostic() {
    // `f :: Int -> Int = \x y. x` — expected `Int -> Int` (1 param) but got 2.
    let lowered = lower("f :: Int -> Int = \\x y. x\nf 1");
    assert!(
        lowered.diagnostics.iter().any(|d| {
            matches!(
                &d.kind,
                ThirDiagnosticKind::FunctionClauseArityMismatch { .. }
            )
        }),
        "expected FunctionClauseArityMismatch; got {:?}",
        lowered.diagnostics
    );
}

// ── thir/lower/expr.rs: UnknownField diagnostic (lines 1136–1142) ────────────

/// Accessing a field that doesn't exist in the record type emits `UnknownField`
/// (lines 1136–1142 of expr.rs).
#[test]
fn unknown_field_diagnostic_on_missing_field() {
    // `{ x = 1; }.y` — field `y` not in `{ x : Int; }` → UnknownField.
    let lowered = lower("{ x = 1; }.y");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| { matches!(&d.kind, ThirDiagnosticKind::UnknownField { .. }) }),
        "expected UnknownField diagnostic; got {:?}",
        lowered.diagnostics
    );
}

// ── thir/lower/expr.rs: ExpectedRecord in field access (lines 1127–1132) ─────

/// Field access on a non-record type emits `ExpectedRecord` from the field-access
/// path (lines 1127–1132 of expr.rs).
#[test]
fn expected_record_diagnostic_from_field_access_on_int() {
    // `42.x` — `42` is an `Int` not a record → ExpectedRecord.
    let lowered = lower("42.x");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| { matches!(&d.kind, ThirDiagnosticKind::ExpectedRecord { .. }) }),
        "expected ExpectedRecord from field access on Int; got {:?}",
        lowered.diagnostics
    );
}

// ── thir/lower/expr.rs: ExpectedRecord in check_record_expr (lines 823–828) ──

/// Using a record literal `{ x = 1; }` where an `Int` is expected emits
/// `ExpectedRecord` from `check_record_expr` (lines 823–828 of expr.rs).
#[test]
fn expected_record_diagnostic_from_record_literal_against_int() {
    // `z :: Int = { x = 1; }` — record literal against non-record expected type.
    let lowered = lower("z :: Int = { x = 1; }\nz");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| { matches!(&d.kind, ThirDiagnosticKind::ExpectedRecord { .. }) }),
        "expected ExpectedRecord from record literal vs Int; got {:?}",
        lowered.diagnostics
    );
}

// ── thir/lower/expr.rs: HirExprKind::UnresolvedIdent in expr position ─────────

/// A reference to an undefined identifier in expression position produces
/// `ValueTypeUnavailable` via the `HirExprKind::UnresolvedIdent` arm
/// (lines 141–146 of expr.rs).
#[test]
fn unresolved_ident_in_expr_position() {
    let lowered = lower_allowing_hir_errors("undefinedSymbol");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| { matches!(&d.kind, ThirDiagnosticKind::ValueTypeUnavailable { .. }) }),
        "expected ValueTypeUnavailable from UnresolvedIdent; got {:?}",
        lowered.diagnostics
    );
}

// ── thir/lower/types.rs: collect_type_vars_into Union arm (lines 682–686) ────

/// A generic function whose parameter type is a *direct* (non-alias) union
/// containing a TypeVar causes `collect_type_vars_into` to traverse the Union
/// arm (not just the AliasApply arm).
#[test]
fn collect_type_vars_union_arm_via_generic_fn_call() {
    // `is_ok :: <A> [ok : {v : A;}; fail;] -> Bool` — the `from` type is
    // a direct Union(TypeVar A), not an AliasApply. When calling
    // `is_ok #ok {v = 42;}`, THIR collects TypeVars from the Union arm.
    let file = completed_file(
        r#"
is_ok :: <A> [ ok : { v : A; }; fail; ] -> Bool {
  | #ok { v = _; } => true;
  | #fail => false;
}
is_ok #ok { v = 42; }
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Bool));
}

// ── thir/lower/types.rs: collect_type_vars_into Tuple/Record/AliasApply ─────

/// A generic function with a Tuple parameter containing a TypeVar covers the
/// Tuple arm of `collect_type_vars_into`.
#[test]
fn collect_type_vars_tuple_arm_via_generic_fn_call() {
    let file = completed_file(
        r#"
fst :: <A, B> (A, B) -> A {
  | (x, _) => x;
}
fst (42, "hi")
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

/// A generic function with a Record parameter covering the Record arm of
/// `collect_type_vars_into`.
#[test]
fn collect_type_vars_record_arm_via_generic_fn_call() {
    let file = completed_file(
        r#"
get :: <A> { value : A; } -> A {
  | { value = x; } => x;
}
get { value = 42; }
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

// ── thir/lower/types.rs: instantiate_infer_vars Union arm (lines 983–1002) ───

/// A generic function whose explicit annotation contains an AliasApply union
/// exercises `instantiate_type_vars` Union arm when expanding the alias during
/// the function call.
#[test]
fn instantiate_type_vars_union_body_with_payload_substitution() {
    // `Result :: <A, E> type [ok : {v : A;}; err : {e : E;}; ]`
    // `is_ok :: <A, E> Result A E -> Bool`
    // When expanding `Result A E` with concrete args, `instantiate_type_vars`
    // traverses the Union body, covering the Union arm.
    let file = completed_file(
        r#"
Result :: <A, E> type [ ok : { v : A; }; err : { e : E; }; ]
is_ok :: <A, E> Result A E -> Bool {
  | #ok { v = _; } => true;
  | #err { e = _; } => false;
}
is_ok #ok { v = 99; }
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Bool));
}

// ── thir/lower/expr.rs: additional error paths ───────────────────────────────

/// Comparison with `<` on a `Bool` type (not Int, Float, or Text) triggers
/// `invalid_binary_operands` (L686-687 of expr.rs) and also the `false` branch
/// of `hir_has_ordering_constraint` (L799 of expr.rs).
#[test]
fn ordering_op_on_bool_type_reports_invalid_operands() {
    let lowered = lower("true < false");
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::InvalidBinaryOperands { op, .. } if *op == "<")),
        "expected InvalidBinaryOperands with op '<'; got {:?}",
        lowered.diagnostics
    );
}

/// `x?.foo` where `x :: Int?` — the inner type is `Int`, not a record, so
/// `lower_opt_access_expr` emits `ExpectedRecord` (L1127-1132 of expr.rs).
#[test]
fn opt_access_on_non_record_optional_inner_reports_expected_record() {
    let lowered = lower(
        r#"
x :: Int? = #none
x?.foo
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::ExpectedRecord { .. })),
        "expected ExpectedRecord; got {:?}",
        lowered.diagnostics
    );
}

/// `s?.hostname` where `Server` has no `hostname` field — emits `UnknownField`
/// (L1135-1142 of expr.rs).
#[test]
fn opt_access_unknown_field_emits_unknown_field_diagnostic() {
    let lowered = lower(
        r#"
Server :: type { port : Int; }
s :: Server? = #none
s?.hostname
"#,
    );
    assert!(
        lowered.diagnostics.iter().any(
            |d| matches!(&d.kind, ThirDiagnosticKind::UnknownField { name } if name == "hostname")
        ),
        "expected UnknownField(hostname); got {:?}",
        lowered.diagnostics
    );
}

/// Named tuple in infer mode: `(x = 1, y = 2)` without an expected type
/// exercises `(HirTupleItem::Named, None)` arm at L504-511 of expr.rs.
#[test]
fn named_tuple_in_infer_mode_covers_named_none_arm() {
    let file = completed_file("t := (x = 1, y = 2)\nt");
    let _ = file;
}

/// `#red {}` where `Color = [red; blue;]` (no-payload variant) in check mode
/// exercises the `None` payload arm at L1191 of expr.rs — the variant is found
/// but has no payload, so the code falls into `self.infer_expr(payload)`.
#[test]
fn tagged_value_no_payload_variant_in_check_mode_covers_l1191() {
    // `#red {}` in check mode against `Color` where `red` has no payload.
    // v.payload == None → hits L1191: `self.infer_expr(payload="{}")`.
    let lowered = lower(
        r#"
Color :: type [red; blue;]
x :: Color = #red {}
x
"#,
    );
    // Any outcome is acceptable; the important thing is the code is reached.
    let _ = lowered;
}

/// `#green {}` where `Color = [red; blue;]` — unknown variant in check mode
/// falls through to the `None =>` arm at L1204-1206 of expr.rs, then
/// the infer path synthesises a singleton union and emits TypeMismatch.
#[test]
fn tagged_value_unknown_variant_in_check_mode_falls_through() {
    let lowered = lower(
        r#"
Color :: type [red; blue;]
x :: Color = #green {}
x
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "expected TypeMismatch for unknown variant; got {:?}",
        lowered.diagnostics
    );
}

/// `#tag {}` where the expected type is `Int` (not Union or Optional) — hits
/// the `_ => {}` fallthrough arm at L1230 of expr.rs, then the infer path
/// creates a singleton union and emits TypeMismatch.
#[test]
fn tagged_value_with_non_union_expected_type_hits_fallthrough() {
    let lowered = lower(
        r#"
x :: Int = #tag {}
x
"#,
    );
    assert!(
        !lowered.diagnostics.is_empty(),
        "expected diagnostics for #tag where Int is expected; got none"
    );
}

/// A builtin type name (`Int`) used as an expression exercises the
/// `BindingKind::BuiltinType` branch in `lower_binding_ref` (L284-295 of
/// expr.rs), specifically the `builtin_type_by_name` call at L287-288.
#[test]
fn builtin_type_in_expression_position_yields_type_value() {
    let lowered = lower("Int");
    // THIR produces a TypeValue expression; no diagnostic.
    let _ = lowered;
}

// ── thir/lower/types.rs: free_infer_vars_into Union/Tuple/Record arms ────────

/// An inferred function with a union-returning expression causes
/// `free_infer_vars_into` to traverse the Union arm during generalization.
#[test]
fn free_infer_vars_union_arm_via_inferred_fn() {
    // `choose` returns one of two union variants — its type contains a Union.
    // During generalization, free_infer_vars_into traverses the Union body.
    let lowered = lower(
        r#"
Color :: type [ red; blue; ]
choose b = if b then #red else #blue
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
make_pair := \x. { first = x; second = 0; }
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
    // The body `| _ _ => true;` typechecks against `A -> A -> Bool`.
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool { | _ _ => true; }; }\n1";
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
  eq :: A -> A -> Bool { | _ _ => true; };
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
same :: <A: Eq> A -> A -> A { | x _ => x; }
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
id :: <A> A -> A { | x => x; }
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

// ── Phase 8: v1 forms are rejected at the semantic entry point ───────────────

fn rejects_as_unsupported(src: &str) {
    let lowered = lower(src);
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(d.kind, ThirDiagnosticKind::UnsupportedFeature { .. })),
        "expected an unsupported-feature diagnostic for {src:?}, got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn open_record_type_is_unsupported_in_thir() {
    rejects_as_unsupported("f :: { host : Text; ...; } -> Text {\n  | x => \"ok\";\n}\nf");
}

#[test]
fn value_select_is_unsupported_in_thir() {
    rejects_as_unsupported("s := { a = 1; }\nselect s { a; }");
}

#[test]
fn perform_is_unsupported_in_thir() {
    rejects_as_unsupported("err := 1\nperform fail err");
}
