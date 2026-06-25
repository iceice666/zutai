use super::*;

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
Pair :: <A, B> type { first : A; second : B; };
p :: Pair Text Int = { first = "x"; second = 1; };
p
"#,
    );
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::AliasApply { .. }
    ));
}

#[test]
fn pair_int_type_kind_checks_at_higher_level() {
    let file = completed_file(
        r#"
Pair :: <A, B> type { first : A; second : B; };
MetaPair :: type Pair Int Type;
MetaPair
"#,
    );
    let meta_pair = file
        .decls
        .iter()
        .map(|&id| &file.decl_arena[id])
        .find(|decl| file.binding_names[decl.binding.0 as usize] == "MetaPair")
        .expect("MetaPair declaration");
    let ty = match meta_pair.kind {
        ThirDeclKind::TypeAlias { ty, .. } => ty,
        ref other => panic!("expected MetaPair type alias, got {other:?}"),
    };
    assert_eq!(file.type_universes[ty.0 as usize], 1);
}

#[test]
fn pair_int_text_defaults_to_ground_level() {
    let file = completed_file(
        r#"
Pair :: <A, B> type { first : A; second : B; };
TextIntPair :: type Pair Text Int;
TextIntPair
"#,
    );
    let text_int_pair = file
        .decls
        .iter()
        .map(|&id| &file.decl_arena[id])
        .find(|decl| file.binding_names[decl.binding.0 as usize] == "TextIntPair")
        .expect("TextIntPair declaration");
    let ty = match text_int_pair.kind {
        ThirDeclKind::TypeAlias { ty, .. } => ty,
        ref other => panic!("expected TextIntPair type alias, got {other:?}"),
    };
    assert_eq!(file.type_universes[ty.0 as usize], 0);
}

#[test]
fn unused_higher_universe_alias_arg_does_not_raise_result_level() {
    let file = completed_file(
        r#"
Const :: <A, B> type A;
ConstIntType :: type Const Int Type;
ConstIntType
"#,
    );
    let const_int_type = file
        .decls
        .iter()
        .map(|&id| &file.decl_arena[id])
        .find(|decl| file.binding_names[decl.binding.0 as usize] == "ConstIntType")
        .expect("ConstIntType declaration");
    let ty = match const_int_type.kind {
        ThirDeclKind::TypeAlias { ty, .. } => ty,
        ref other => panic!("expected ConstIntType type alias, got {other:?}"),
    };
    assert_eq!(file.type_universes[ty.0 as usize], 0);
    assert_eq!(file.type_universes.len(), file.type_arena.len());
}

#[test]
fn recursive_union_alias_elaborates_without_expanding() {
    let file = completed_file(
        r#"
Tree :: type {
  #leaf;
  #node : { value : Int; left : Tree; right : Tree; };
};

example :: Tree =
  #node {
    value = 1;
    left  = #leaf;
    right = #node { value = 2; left = #leaf; right = #leaf; };
  };

example
"#,
    );

    let tree_decl = file
        .decls
        .iter()
        .map(|&id| &file.decl_arena[id])
        .find(|decl| file.binding_names[decl.binding.0 as usize] == "Tree")
        .expect("Tree declaration");
    let tree_binding = tree_decl.binding;

    let tree_ty = match &tree_decl.kind {
        ThirDeclKind::TypeAlias { params, ty } => {
            assert!(params.is_empty());
            *ty
        }
        other => panic!("expected Tree type alias, got {other:?}"),
    };

    let variants = match &file.type_arena[tree_ty.0 as usize].kind {
        TypeKind::Union(variants, RowTail::Closed) => variants,
        other => panic!("expected closed union alias body, got {other:?}"),
    };

    let leaf = variants
        .iter()
        .find(|variant| variant.name == "leaf")
        .expect("leaf variant");
    assert!(leaf.payload.is_none());

    let node = variants
        .iter()
        .find(|variant| variant.name == "node")
        .expect("node variant");
    let node_payload = node.payload.expect("node payload");
    let fields = match &file.type_arena[node_payload.0 as usize].kind {
        TypeKind::Record(fields, RowTail::Closed) => fields,
        other => panic!("expected closed node record payload, got {other:?}"),
    };

    let left = fields
        .iter()
        .find(|field| field.name == "left")
        .expect("left field");
    let right = fields
        .iter()
        .find(|field| field.name == "right")
        .expect("right field");

    assert!(matches!(
        &file.type_arena[left.ty.0 as usize].kind,
        TypeKind::Alias(binding) if *binding == tree_binding
    ));
    assert!(matches!(
        &file.type_arena[right.ty.0 as usize].kind,
        TypeKind::Alias(binding) if *binding == tree_binding
    ));
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::Alias(binding) if *binding == tree_binding
    ));
}

#[test]
fn generic_alias_used_in_function_signature() {
    // A function that takes a `Pair Int Int` and returns the first field.
    let file = completed_file(
        r#"
Pair :: <A, B> type { first : A; second : B; };
fst :: Pair Int Int -> Int
  = p => p.first;
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
Pair :: <A, B> type { first : A; second : B; };
x :: Pair Text = x;
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
Pair :: <A, B> type { first : A; second : B; };
x :: Pair = x;
x
"#,
    );
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::TypeConstructorArityMismatch { expected, found, .. }
            if *expected == 2 && *found == 0
    )));
}
#[test]
fn partial_application_in_alias_body_reports_error() {
    // An under-applied constructor buried in a type-alias body (a field typed
    // `Pair Int`) must still emit TypeConstructorArityMismatch — partial
    // application is only legal in witness targets.
    let lowered = lower(
        r#"
Pair :: <A, B> type { first : A; second : B; };
Bad :: type { x : Pair Int; };
1
"#,
    );
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::TypeConstructorArityMismatch { name, expected, found }
            if name == "Pair" && *expected == 2 && *found == 1
    )));
}

#[test]
fn partial_application_in_method_signature_reports_error() {
    // An under-applied constructor in a constraint method signature must emit
    // TypeConstructorArityMismatch.
    let lowered = lower(
        r#"
Pair :: <A, B> type { first : A; second : B; };
C :: <T> @T { bad :: T -> Pair Int; }
1
"#,
    );
    assert!(lowered.diagnostics.iter().any(|d| matches!(
        &d.kind,
        ThirDiagnosticKind::TypeConstructorArityMismatch { name, expected, found }
            if name == "Pair" && *expected == 2 && *found == 1
    )));
}

// ── Type-level evaluation fuel limit ────────────────────────────────────────

#[test]
fn type_level_expansion_exceeding_fuel_reports_limit() {
    // D1 → D2 = Pair D1 D1 → D3 = Pair D2 D2: resolving D3 requires multiple
    // Pair expansions. With a budget of 1 the second expansion is denied.
    let src = r#"
Pair :: <A, B> type { first : A; second : B; };
D1 :: type Int;
D2 :: type Pair D1 D1;
D3 :: type Pair D2 D2;
x :: D3 = x;
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
    let file = completed_file("id x = x;\nid 42");
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
Callback :: type { fn : Int -> Int; };

runCallback :: Callback -> Int -> Int
  = cb x => cb.fn x;

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
Fns :: type { first : Int -> Int; second : Int -> Int; };

applyBoth :: Fns -> Int -> Int
  = fns x => x |> fns.first |> fns.second;

applyBoth { first = \n. n + 1; second = \n. n * 2; } 4
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn function_stored_in_let_binding_is_callable() {
    let file = completed_file(
        r#"
inc :: Int -> Int
  = n => n + 1;

[
  fn := inc;
  fn 10
]
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn calling_non_function_field_reports_error() {
    // `x.val 5` where `val : Int` should raise ExpectedFunction.
    let lowered = lower(
        r#"
Rec :: type { val : Int; };

apply :: Rec -> Int -> Int
  = r x => r.val x;

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
inc :: Int -> Int
  = n => n + 1;

double :: Int -> Int
  = n => n * 2;

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
double :: Int -> Int
  = n => n * 2;

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
inc :: Int -> Int
  = n => n + 1;

double :: Int -> Int
  = n => n * 2;

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
compute :: Int -> Int
  = n => [
    doubled := n * 2;
    incremented := doubled + 1;
    incremented
  ];

compute 5
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn block_result_type_propagates_to_caller() {
    let file = completed_file(
        r#"
makeLabel :: Int -> Text
  = n => [
    prefix := "value-";
    _ := n;
    prefix
  ];

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
Inner :: type { value : Int; };
Outer :: type { inner : Inner; };

o :: Outer = { inner = { value = 42; }; };

o.inner.value
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn access_text_field_on_nested_record() {
    let file = completed_file(
        r#"
Meta :: type { label : Text; count : Int; };
Config :: type { meta : Meta; enabled : Bool; };

cfg :: Config = {
  meta = { label = "prod"; count = 3; };
  enabled = true;
};

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
apply :: (Int -> Int) -> Int -> Int
  = f x => f x;

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
apply f x = f x;

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
applyTwice :: (Int -> Int) -> Int -> Int
  = f x => f (f x);

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
compose :: <A, B, C> (B -> C) -> (A -> B) -> A -> C
  = f g x => f (g x);

inc :: Int -> Int
  = n => n + 1;
double :: Int -> Int
  = n => n * 2;

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
apply :: (Int -> Int) -> Int -> Int
  = f x => f x;

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
x :: Int? = #none;

x ?? 0
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn opt_access_chained_with_coalesce() {
    let file = completed_file(
        r#"
Server :: type { port : Int; };

get_port :: Server? -> Int
  = s => s?.port ?? 80;

get_port #none
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Int));
}

#[test]
fn generic_recursive_union_alias_checks_values() {
    let file = completed_file(
        r#"
Tree :: <A> type {
  #leaf;
  #node : { value : A; left : Tree A; right : Tree A; };
};
example :: Tree Int =
  #node { value = 1; left = #leaf; right = #leaf; };
example == example
"#,
    );
    assert!(matches!(final_type_kind(&file), TypeKind::Bool));
}

#[test]
fn repeated_recursive_alias_mismatch_does_not_reuse_stale_fixpoint() {
    let lowered = lower(
        r#"
Tree :: <A> type {
  #leaf;
  #node : { value : A; left : Tree A; right : Tree A; };
};
f :: Tree Bool -> Int = t => 0;
y :: Tree Int = #leaf;
a :: Int = f y;
b :: Int = f y;
b
"#,
    );
    let mismatches = lowered
        .diagnostics
        .iter()
        .filter(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. }))
        .count();
    assert!(
        mismatches >= 2,
        "both repeated bad calls should report TypeMismatch, got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn alias_application_fast_path_does_not_ignore_variance() {
    let lowered = lower(
        r#"
Fn :: <A> type A -> Int;
g :: Fn true = x => 0;
h :: Fn Bool = g;
h
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "contravariant alias parameter mismatch should be rejected, got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn covariant_alias_arg_accepted_at_expected_type() {
    // Pair :: <A, B> type { first: A; second: B } is covariant in A.
    // Pair true Int should be assignable to Pair Bool Int (true is a subtype of Bool).
    let lowered = lower(
        r#"
TrueT :: type true;
Pair :: <A, B> type { first : A; second : B; };
f :: Pair Bool Int -> Int = x => 0;
g :: Pair TrueT Int -> Int = f;
g
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .all(|d| !matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "Pair true Int should be assignable to Pair Bool Int (covariant), got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn deep_mismatch_behind_recursion_not_masked_by_coinductive_guard() {
    let lowered = lower(
        r#"
TreeA :: type { #leaf; #node : { value : Int; child : TreeA; }; };
TreeB :: type { #leaf; #node : { value : Text; child : TreeB; }; };
f :: TreeA -> Int = t => 0;
y :: TreeB = #leaf;
z :: Int = f y;
z
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "deep value mismatch (Int vs Text) must surface despite the recursive child back-edge, got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn mutual_generic_aliases_elaborate_cleanly_in_thir() {
    let file = completed_file(
        r#"
Odd :: <A> type { #nil; #node : { v : A; e : Even A; }; };
Even :: <A> type { #nil; #node : { v : A; e : Odd A; }; };
x :: Odd Int = #nil;
x
"#,
    );
    assert!(matches!(
        final_type_kind(&file),
        TypeKind::AliasApply { .. }
    ));
}

#[test]
fn higher_rank_apply_id_checks() {
    let src = r#"
applyId :: (<A> A -> A) -> { i : Int; t : Text; }
  = f => { i = f 1; t = f "x"; };
applyId
"#;
    let lowered = lower(src);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn higher_rank_show_both_checks() {
    let src = r#"
Show :: <A> @A { show :: A -> Text; }
Show @Int :: { show = \n. "int"; }
Show @Bool :: { show = \b. "bool"; }
showBoth :: (<A: Show> A -> Text) -> { left : Text; right : Text; } =
  \render. { left = render 1; right = render true; };
showBoth
"#;
    let lowered = lower(src);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
}

#[test]
fn insufficient_higher_rank_annotation_reports_type_mismatch() {
    let lowered = lower(
        r#"
bad :: (Int -> Int) -> { i : Int; t : Text; }
  = f => { i = f 1; t = f "x"; };
bad
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::TypeMismatch { .. })),
        "conflicting monomorphic uses should produce TypeMismatch, got {:?}",
        lowered.diagnostics
    );
}

#[test]
fn impredicative_list_rejected() {
    let lowered = lower(
        r#"
xs :: List (<A> A -> A) = {;};
xs
"#,
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .any(|d| matches!(&d.kind, ThirDiagnosticKind::UnsupportedFeature { .. })),
        "List of ForAll must reject as impredicative, got {:?}",
        lowered.diagnostics
    );
}
