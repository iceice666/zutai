use super::*;

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
Profile :: type {#dev; #prod;}
isProd :: Profile -> Bool
  = #dev => false;
  = #prod => true;
isProd #dev
"#,
    );
}

#[test]
fn non_exhaustive_atom_union_reports_witness() {
    let lowered = lower(
        r#"
Profile :: type {#dev; #prod;}
isProd :: Profile -> Bool
  = #prod => true;
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
Profile :: type {#dev; #prod;}
isProd :: Profile -> Bool
  = #prod => true;
  = _ => false;
isProd #dev
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn redundant_arm_after_catch_all_is_unreachable() {
    let lowered = lower(
        r#"
Profile :: type {#dev; #prod;}
classify :: Profile -> Bool
  = _ => false;
  = #prod => true;
classify #dev
"#,
    );
    assert!(has_unreachable(&lowered));
}

#[test]
fn bool_match_exhaustive_passes() {
    completed_file(
        r#"
negate :: Bool -> Bool
  = true => false;
  = false => true;
negate true
"#,
    );
}

#[test]
fn bool_match_non_exhaustive_reports_false() {
    let lowered = lower(
        r#"
f :: Bool -> Bool
  = true => false;
f true
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("false"));
}

#[test]
fn int_match_requires_wildcard() {
    let lowered = lower(
        r#"
f :: Int -> Int
  = 1 => 10;
  = 2 => 20;
f 1
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("_"));
}

#[test]
fn int_match_with_wildcard_is_exhaustive() {
    completed_file(
        r#"
f :: Int -> Int
  = 1 => 10;
  = _ => 0;
f 1
"#,
    );
}

#[test]
fn guarded_arm_does_not_cover() {
    let lowered = lower(
        r#"
Profile :: type {#dev; #prod;}
f :: Profile -> Bool
  = #dev => false;
  = #prod if true => true;
f #dev
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("#prod"));
}

#[test]
fn plain_arm_after_guarded_same_pattern_is_reachable() {
    let lowered = lower(
        r#"
Profile :: type {#dev; #prod;}
f :: Profile -> Bool
  = #dev => false;
  = #prod if true => true;
  = #prod => false;
f #dev
"#,
    );
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn multi_clause_function_exhaustive_passes() {
    completed_file(
        r#"
pick :: Bool -> Bool -> Text
  = true true => "tt";
  = true false => "tf";
  = false _ => "f";
pick true false
"#,
    );
}

#[test]
fn multi_clause_function_non_exhaustive_reports() {
    let lowered = lower(
        r#"
pick :: Bool -> Bool -> Text
  = true true => "tt";
  = true false => "tf";
pick true false
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("false _"));
}

#[test]
fn tagged_tuple_union_exhaustive_passes() {
    completed_file(
        r#"
Shape :: type {
  #circle: { radius: Int; };
  #square: { side: Int; };
}
area :: Shape -> Int
  = #circle { radius = r; } => r;
  = #square { side = s; } => s;
area
"#,
    );
}

#[test]
fn tagged_tuple_union_non_exhaustive_reports_witness() {
    let lowered = lower(
        r#"
Shape :: type {
  #circle: { radius: Int; };
  #square: { side: Int; };
}
area :: Shape -> Int
  = #circle { radius = r; } => r;
area
"#,
    );
    assert_eq!(
        nonexhaustive_witness(&lowered).as_deref(),
        Some("#square { ... }")
    );
}

#[test]
fn positional_payload_union_exhaustive_passes() {
    completed_file(
        r#"
Pair :: type {
  #pair: (Int, Int);
  #empty;
}
sum :: Pair -> Int
  = #pair (x, y) => x + y;
  = #empty => 0;
sum
"#,
    );
}

#[test]
fn positional_payload_union_non_exhaustive_reports_tuple_witness() {
    let lowered = lower(
        r#"
Pair :: type {
  #pair: (Int, Int);
  #empty;
}
sum :: Pair -> Int
  = #empty => 0;
sum
"#,
    );
    assert_eq!(
        nonexhaustive_witness(&lowered).as_deref(),
        Some("#pair (...)")
    );
}

#[test]
fn optional_match_exhaustive_passes() {
    completed_file(
        r#"
unwrap :: Int? -> Int
  = #none => 0;
  = #some (x) => x;
unwrap #none
"#,
    );
}

#[test]
fn optional_match_missing_none_reports_witness() {
    let lowered = lower(
        r#"
unwrap :: Int? -> Int
  = #some (x) => x;
unwrap #none
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("#none"));
}

#[test]
fn optional_match_missing_some_reports_witness() {
    let lowered = lower(
        r#"
unwrap :: Int? -> Int
  = #none => 0;
unwrap #none
"#,
    );
    assert_eq!(
        nonexhaustive_witness(&lowered).as_deref(),
        Some("#some (...)")
    );
}

#[test]
fn maybe_match_missing_absent_reports_witness() {
    let lowered = lower(
        r#"
unwrap :: Maybe Int -> Int
  = #present (x) => x;
unwrap #present (1)
"#,
    );
    assert_eq!(nonexhaustive_witness(&lowered).as_deref(), Some("#absent"));
}

#[test]
fn maybe_match_missing_present_reports_witness() {
    let lowered = lower(
        r#"
unwrap :: Maybe Int -> Int
  = #absent => 0;
unwrap #absent
"#,
    );
    assert_eq!(
        nonexhaustive_witness(&lowered).as_deref(),
        Some("#present (...)")
    );
}

// ── Optional access (`?.`) ───────────────────────────────────────────────────

#[test]
fn opt_access_on_optional_record() {
    let file = completed_file(
        r#"
Server :: type { port : Int; }

get_port :: Server? -> Int?
  = s => s?.port;

get_port #none
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Optional(_)));
}

#[test]
fn opt_access_optional_field_preserves_presence() {
    let file = completed_file(
        r#"
Server :: type { port? : Int; }

get_port :: Server? -> Optional (Maybe Int)
  = s => s?.port;

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

get_port :: Server -> Int?
  = s => s?.port;

get_port { port = 80; }
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::ExpectedOptionalOrMaybe { .. }))
    );
}
