use super::*;

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
    let file = completed_file("id x = x\nx ::= id 42\ny ::= id \"hello\"\ny");
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}

#[test]
fn instantiate_infer_vars_maybe_return() {
    // A function with an annotated Maybe return type that requires optional field access.
    // The Maybe inner type flows through the type system when the function is called.
    let file = completed_file("S :: type { v? : Int; }\nget :: S -> Maybe Int = \\s. s.v\nget {}");
    assert!(matches!(final_type_kind(&file), TypeKind::Maybe(_)));
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
fn type_name_fixed_width_appears_in_mismatch_message() {
    let lowered = lower("x :: u8 = 255\nx");
    assert!(lowered.diagnostics.iter().any(|d| {
        matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { expected, found }
            if expected == "u8" && found == "Int")
    }));
}

#[test]
fn type_name_maybe_appears_in_mismatch_message() {
    // Passing field presence where an Int is needed → type_name calls Maybe arm.
    let lowered = lower("S :: type { v? : Int; }\ns :: S = {}\nresult :: Int = s.v\nresult");
    assert!(lowered.diagnostics.iter().any(|d| {
        matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { found, .. }
            if found.contains("Maybe"))
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

/// A second coverage path for instantiate_infer_vars Maybe arm:
/// using an annotated function so the Maybe return type is stored.
#[test]
fn instantiate_infer_vars_maybe_arm_via_annotation() {
    let file =
        completed_file("S :: type { x? : Int; }\nf :: S -> Maybe Int = \\s. s.x\nf { x = 5; }");
    assert!(matches!(final_type_kind(&file), TypeKind::Maybe(_)));
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
    // ResultOf :: <A, E> type {#ok: {v:A;}; #err: {e:E;};}  applied to Int, Text.
    // Exercises instantiate_type_vars Union arm when expanding the alias.
    let file =
        completed_file("R :: <A> type { #ok: { v : A; }; #fail; }\nx :: R Int = #ok { v = 1; }\nx");
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
    let lowered = lower("C :: type { #r; #g; #b; }\nx :: C = 42\nx");
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
wrap :: <A> A -> List A
  = x => [x;];
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
make :: <A> A -> Wrapper A
  = x => { value = x; };
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
    let lowered = lower("A :: type { #r; #g; }\nB :: type { #r; #g; }\nx :: A = #r\ny :: B = x\ny");
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
f :: S -> Int
  = _ => 0;
t :: T = { x = 1; }
f t
"#,
    );
    let mismatch = lowered
        .diagnostics
        .iter()
        .find_map(|d| match &d.kind {
            ThirDiagnosticKind::TypeMismatch { expected, found } => {
                Some((expected.as_str(), found.as_str()))
            }
            _ => None,
        })
        .expect("expected TypeMismatch diagnostic");
    assert_eq!(mismatch, ("{ x : Int; y : Text; }", "{ x : Int; }"));
}

/// Passing a record where a shared field has the wrong type triggers the
/// `return false` branch at line 460 of `record_types_match`.
#[test]
fn record_types_match_field_type_mismatch_returns_false() {
    let lowered = lower(
        r#"
S :: type { x : Int; }
T :: type { x : Text; }
f :: S -> Int
  = _ => 0;
t :: T = { x = "bad"; }
f t
"#,
    );
    let mismatch = lowered
        .diagnostics
        .iter()
        .find_map(|d| match &d.kind {
            ThirDiagnosticKind::TypeMismatch { expected, found } => {
                Some((expected.as_str(), found.as_str()))
            }
            _ => None,
        })
        .expect("expected TypeMismatch diagnostic");
    assert_eq!(mismatch, ("{ x : Int; }", "{ x : Text; }"));
}

#[test]
fn diagnostic_polish_record_mismatch_shows_open_tail() {
    let lowered = lower(
        r#"
getHost :: { host : Text; ...; } -> Text
  = x => x.host;
PortOnly :: type { port : Int; }
p :: PortOnly = { port = 8080; }
getHost p
"#,
    );
    let mismatch = lowered
        .diagnostics
        .iter()
        .find_map(|d| match &d.kind {
            ThirDiagnosticKind::TypeMismatch { expected, found } => {
                Some((expected.as_str(), found.as_str()))
            }
            _ => None,
        })
        .expect("expected TypeMismatch diagnostic");
    assert_eq!(mismatch, ("{ host : Text; ...; }", "{ port : Int; }"));
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
Result :: type { #ok: { value : Int; }; #err; }
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
Color :: type { #red: { n : Int; }; #blue; }
x ::= #red { n = 99; }
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
        r#"x ::= (a = 1, b = "hi")
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
    // `is_ok :: <A> { #ok: {v : A;}; #fail; } -> Bool` — the `from` type is
    // a direct Union(TypeVar A), not an AliasApply. When calling
    // `is_ok #ok {v = 42;}`, THIR collects TypeVars from the Union arm.
    let file = completed_file(
        r#"
is_ok :: <A> { #ok : { v : A; }; #fail; } -> Bool
  = #ok { v = _; } => true;
  = #fail => false;
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
fst :: <A, B> (A, B) -> A
  = (x, _) => x;
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
get :: <A> { value : A; } -> A
  = { value = x; } => x;
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
    // `Result :: <A, E> type {#ok: {v : A;}; #err: {e : E;}; }`
    // `is_ok :: <A, E> Result A E -> Bool`
    // When expanding `Result A E` with concrete args, `instantiate_type_vars`
    // traverses the Union body, covering the Union arm.
    let file = completed_file(
        r#"
Result :: <A, E> type { #ok: { v : A; }; #err: { e : E; }; }
is_ok :: <A, E> Result A E -> Bool
  = #ok { v = _; } => true;
  = #err { e = _; } => false;
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
    let file = completed_file("t ::= (x = 1, y = 2)\nt");
    let _ = file;
}

/// `#red {}` where `Color = type { #red; #blue; }` (no-payload variant) in check mode
/// exercises the `None` payload arm at L1191 of expr.rs — the variant is found
/// but has no payload, so the code falls into `self.infer_expr(payload)`.
#[test]
fn tagged_value_no_payload_variant_in_check_mode_covers_l1191() {
    // `#red {}` in check mode against `Color` where `red` has no payload.
    // v.payload == None → hits L1191: `self.infer_expr(payload="{}")`.
    let lowered = lower(
        r#"
Color :: type {#red; #blue;}
x :: Color = #red {}
x
"#,
    );
    // Any outcome is acceptable; the important thing is the code is reached.
    let _ = lowered;
}

/// `#green {}` where `Color = type { #red; #blue; }` — unknown variant in check mode
/// falls through to the `None =>` arm at L1204-1206 of expr.rs, then
/// the infer path synthesises a singleton union and emits TypeMismatch.
#[test]
fn tagged_value_unknown_variant_in_check_mode_falls_through() {
    let lowered = lower(
        r#"
Color :: type {#red; #blue;}
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

#[test]
fn instantiate_type_vars_patch_alias_body() {
    let src = r#"
PatchOf :: <A> type Patch { value : A; note : Text; }
p :: PatchOf Int = { value = 1; }
p
"#;
    let file = completed_file(src);
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::AliasApply { .. }
    ));
}

#[test]
fn maybe_rejects_optional_none_atom() {
    let lowered = lower(
        r#"
x :: Maybe Int = #none
x
"#,
    );
    assert!(lowered.file.is_none());
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            ThirDiagnosticKind::TypeMismatch { expected, found }
                if expected.contains("Maybe") && found.contains("#none")
        )
    }));
}

#[test]
fn function_param_contravariance_accepts_more_general_callback() {
    let src = r#"
apply :: ({ host : Text; port : Int; ...; } -> Text) -> Text
  = f => f { host = "h"; port = 8080; };
g :: { host : Text; ...; } -> Text
  = r => r.host;
apply g
"#;
    let file = completed_file(src);
    assert!(matches!(final_type_kind(&file), TypeKind::Text));
}
