use super::*;

// ── Float and String literal patterns (pat.rs HirPatKind::Float / ::String) ──

#[test]
fn float_pattern_in_function_clause_type_checks() {
    let file = completed_file(
        r#"
classify :: Float -> Text
  = 0.0 => "zero";
  = _ => "nonzero";
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
fn posit_pattern_in_function_clause_type_checks() {
    let file = completed_file(
        r#"
classify :: Posit32e3 -> Text
  = 0p32e3 => "zero";
  = _ => "nonzero";
classify 1p32e3
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
greet :: Text -> Text
  = "hello" => "hi";
  = _ => "?";
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
f :: Int -> Int
  = (x, y) => x;
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
f :: (x : Int, y : Int) -> Int
  = (a = m, b = n) => m;
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
f :: (Int, Int) -> Int
  = (a = m, b = n) => m;
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
f :: (x : Int, y : Int) -> Int
  = (m, n) => m;
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
f :: Int -> Int
  = { x = v; } => v;
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
Status :: type { #ok: { code : Int; }; #err: { msg : Text; }; };
f :: Status -> Int
  = #unknown { code = _; } => 1;
  = _ => 0;
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
f :: Int -> Int
  = #foo { x = _; } => 1;
  = _ => 0;
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
Status :: type { #ok; #err; };
f :: Status -> Int
  = #ok { x = _; } => 1;
  = _ => 0;
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
fn optional_some_tuple_pattern_lowers_correctly() {
    // `#some (n)` is the tagged-value pattern for `T?` (Optional T).
    let file = completed_file(
        r#"
unwrap :: Int? -> Int
  = #some (n) => n;
  = #none => 0;
unwrap #some (42)
"#,
    );
    assert!(
        matches!(final_type_kind(&file), TypeKind::Int),
        "expected Int; got {:?}",
        final_type_kind(&file)
    );
}

#[test]
fn optional_some_pattern_with_record_field_reports_tuple_field_mismatch() {
    // Builtin Optional uses tuple slot `0`; record payload fields are rejected.
    let lowered = lower(
        r#"
f :: Int? -> Int
  = #some { badfield = n; } => n;
  = #none => 0;
1
"#,
    );
    assert!(
        lowered.diagnostics.iter().any(|d| matches!(
            &d.kind,
            ThirDiagnosticKind::TupleFieldNameMismatch { expected, found }
                if expected == "<positional>" && found == "badfield"
        )),
        "expected TupleFieldNameMismatch(badfield); got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn maybe_absent_and_present_patterns_lower_correctly() {
    let file = completed_file(
        r#"
S :: type { p? : Int; };
s :: S = { p = 42; };
unwrap :: Maybe Int -> Int
  = #present (n) => n;
  = #absent => 0;
unwrap s.p
"#,
    );
    assert!(
        matches!(final_type_kind(&file), TypeKind::Int),
        "expected Int; got {:?}",
        final_type_kind(&file)
    );
}

#[test]
fn optional_invalid_tag_in_pattern_reports_type_mismatch() {
    // `#foo` is not `#none` or `#some` for `Int?` → TypeMismatch.
    let lowered = lower(
        r#"
f :: Int? -> Int
  = #foo { x = _; } => 1;
  = _ => 0;
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
A :: type A;
f :: A -> A
  = n => n;
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
x ::= 5;
y :: x = 5;
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
x :: (a : Int, b : Int) = (c = 1, d = 2);
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
x :: (Int, Int) = (a = 1, b = 2);
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
x :: (a : Int, b : Int) = (1, 2);
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
x :: Int = (1, 2);
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
Result :: <A, E> type { #ok: { value : A; }; #err: { error : E; }; };
r :: Result Int Text = #ok { value = 42; };
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
MaybeList :: <A> type (List A)?;
x :: MaybeList Int = #none;
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
Fn :: <A, B> type A -> B;
add1 :: Fn Int Int = \n. n + 1;
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
x :: List Int = 5;
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
x :: Int? = 5;
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
x :: (Int, Int) = 5;
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
Status :: type { #ok; #err; };
x :: Status = 5;
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
x :: Int -> Int = 5;
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
Pair :: <A, B> type { first : A; second : B; };
x :: Pair Int Text = 5;
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
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @{#ok; #err;} :: { eq = \\a b. true; }\n1",
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
        "IL :: type List Int;\nEq :: <A> @A { eq :: A -> A -> Bool; }\nEq @IL :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

/// witness_target_key with an alias that resolves to Optional → Optional arm.
#[test]
fn witness_target_key_alias_resolving_to_optional() {
    let file = completed_file(
        "MI :: type Int?;\nEq :: <A> @A { eq :: A -> A -> Bool; }\nEq @MI :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

/// witness_target_key with an alias that resolves to a function type → Function arm.
#[test]
fn witness_target_key_alias_resolving_to_function() {
    let file = completed_file(
        "F :: type Int -> Int;\nEq :: <A> @A { eq :: A -> A -> Bool; }\nEq @F :: { eq = \\a b. true; }\n1",
    );
    let _ = file;
}

// ── instantiate_type_vars compound type arms ──────────────────────────────────

/// Generic alias with List body covers instantiate_type_vars List arm.
#[test]
fn instantiate_type_vars_list_body() {
    let file = completed_file("ListOf :: <A> type List A;\nxs :: ListOf Int = {1; 2; 3;};\nxs");
    let _ = file;
}

/// Generic alias with Optional body covers instantiate_type_vars Optional arm.
#[test]
fn instantiate_type_vars_optional_body() {
    let file = completed_file("MaybeOf :: <A> type A?;\nx :: MaybeOf Int = #none;\nx");
    let _ = file;
}

/// Generic alias with Function (Arrow) body covers instantiate_type_vars Function arm.
#[test]
fn instantiate_type_vars_function_body() {
    let file =
        completed_file("FnOf :: <A, B> type A -> B;\nf :: FnOf Int Text = \\x. \"hello\";\nf 1");
    let _ = file;
}

/// Generic alias with Tuple body covers instantiate_type_vars Tuple arm.
#[test]
fn instantiate_type_vars_tuple_body() {
    let file =
        completed_file("PairOf :: <A, B> type (A, B);\np :: PairOf Int Text = (1, \"hi\");\np");
    let _ = file;
}

/// Generic alias with Union body covers instantiate_type_vars Union arm.
#[test]
fn instantiate_type_vars_union_body() {
    let file = completed_file(
        "ResultOf :: <A, E> type { #ok: { value : A; }; #err: { error : E; }; };\nr :: ResultOf Int Text = #ok { value = 42; };\nr",
    );
    let _ = file;
}

// ── OptionalAccess THIR lowering ──────────────────────────────────────────────

/// Optional field access `cfg?.port` where cfg :: Config? → ThirExprKind::OptionalAccess.
#[test]
fn optional_access_lowers_correctly_to_thir() {
    let file = completed_file(
        "Config :: type { port : Int; };\ncfg :: Config? = #none;\nn :: Int? = cfg?.port;\nn",
    );
    // The file must complete without errors.
    let _ = file;
}
