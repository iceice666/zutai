use super::*;

// ─── HM let-generalization ────────────────────────────────────────────────────

#[test]
fn polymorphic_identity_runs_at_two_types() {
    let v = eval_file("id x = x;\n(id 42, id \"hello\")").unwrap();
    let expected = Value::Tuple(
        vec![
            value::TupleField {
                name: None,
                value: thunk::Thunk::ready(Value::Int(42)),
            },
            value::TupleField {
                name: None,
                value: thunk::Thunk::ready(Value::Text("hello".into())),
            },
        ]
        .into(),
    );
    assert_eq!(v, expected);
}

#[test]
fn monomorphic_value_binding_still_runs() {
    assert_eq!(eval_file("answer ::= 42;\nanswer").unwrap(), Value::Int(42));
}

// ─── generic type aliases ─────────────────────────────────────────────────────

#[test]
fn generic_alias_value_evaluates() {
    // A value typed with a generic alias must evaluate to the underlying record,
    // and field access must return the correctly typed value.
    let decl = r#"
Pair :: <A, B> type { first : A; second : B; };
p :: Pair Text Int = { first = "x"; second = 1; };
"#;
    assert_eq!(run(&format!("{decl}\np.first")), Value::Text("x".into()));
    assert_eq!(run(&format!("{decl}\np.second")), Value::Int(1));
}

// ─── T-INV: v1 constraint/witness does not break THIR completeness ────────────

/// T-INV: a file with well-formed constraint + witness + normal binding produces
/// a complete THIR (LoweredThir.file.is_some()) and still evaluates.
/// This guards the semantics-oracle invariant: constraint/witness decls must
/// emit zero HIR+THIR diagnostics so they don't null out LoweredThir.file.
#[test]
fn t_inv_constraint_witness_does_not_break_eval() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = intEq; }\nintEq ::= \\a b. true;\n42";
    assert_eq!(run(src), Value::Int(42));
}

/// Derive witness also must not break THIR completeness.
#[test]
fn t_inv_derive_witness_does_not_break_eval() {
    // Use builtin type `Int` so target resolves without error
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: derive\n1";
    assert_eq!(run(src), Value::Int(1));
}

#[test]
fn tlc_derive_int_eq_dispatches() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: derive\neq 1 1";
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
}

#[test]
fn tlc_derive_record_eq_compares_fields() {
    let src = r#"
Point :: type { x : Int; y : Int; };
p1 :: Point = { x = 1; y = 2; };
p2 :: Point = { x = 1; y = 3; };
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Point :: derive
eq p1 p2
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(false));
}

#[test]
fn tlc_derive_neq_is_true_negation() {
    let src = r#"
Point :: type { x : Int; y : Int; };
p1 :: Point = { x = 1; y = 2; };
p2 :: Point = { x = 1; y = 3; };
Eq :: <A> @A { eq :: A -> A -> Bool; neq :: A -> A -> Bool; } derive
Eq @Point :: derive
neq p1 p2
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
}

#[test]
fn tlc_derive_union_eq_compares_shape_and_payload() {
    let src = r#"
Status :: type { #ok: { code : Int; }; #err: { msg : Text; }; };
ok :: Status = #ok { code = 200; };
err :: Status = #err { msg = "no"; };
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Status :: derive
eq ok err
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(false));
}

#[test]
fn explicit_witness_reflection_dispatches_dictionary() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
(witness Eq @Int).eq 1 1
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
}

#[test]
fn witness_reflection_accepts_conditional_dictionary_resolution() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Eq @(List A) :: <A: Eq> { eq = \xs ys. true; }
(witness Eq @(List Int)).eq {1;} {2;}
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
}

#[test]
fn variants_builtin_reflects_union_payload_types() {
    let value = eval_file(
        r#"
Result :: type { #ok: { value : Int; }; #err; };
variants (type Result)
"#,
    )
    .unwrap();
    let ok = list_item(&value, 0);
    assert_eq!(record_field_value(&ok, "name"), Value::Text("ok".into()));
    let fields = record_field_value(&ok, "fields");
    let field = list_item(&fields, 0);
    assert_eq!(record_field_value(&field, "Type").to_string(), "<type>");
}

#[test]
fn recipe_show_derives_record_witness() {
    let src = r#"
Point :: type { x : Int; y : Int; };
p :: Point = { x = 1; y = 2; };
Show :: <A> @A { show :: A -> Text; } derive = <T> => \x. x
Show @Point :: derive
show p
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Text("{x, y}".into()));
}

#[test]
fn recipe_witness_reflection_dispatches_derived_dictionary() {
    let src = r#"
Point :: type { x : Int; y : Int; };
p :: Point = { x = 1; y = 2; };
Show :: <A> @A { show :: A -> Text; } derive = <T> => \x. x
Show @Point :: derive
(witness Show @Point).show p
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Text("{x, y}".into()));
}

#[test]
fn recipe_ord_derives_lexicographic_record_witness() {
    let src = r#"
Ordering :: type { #lt; #eq; #gt; };
Point :: type { x : Int; y : Int; };
p1 :: Point = { x = 1; y = 2; };
p2 :: Point = { x = 1; y = 3; };
Ord :: <A> @A { compare :: A -> A -> Ordering; } derive = <T> => \x. x
Ord @Point :: derive
compare p1 p2
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Atom("lt".into()));
}

#[test]
fn recipe_show_and_ord_derive_union_witnesses() {
    let show_src = r#"
Status :: type { #ok; #err; };
s :: Status = #err;
Show :: <A> @A { show :: A -> Text; } derive = <T> => \x. x
Show @Status :: derive
show s
"#;
    assert_eq!(eval_tlc_file(show_src).unwrap(), Value::Text("#err".into()));

    let ord_src = r#"
Ordering :: type { #lt; #eq; #gt; };
Status :: type { #ok; #err; };
ok :: Status = #ok;
err :: Status = #err;
Ord :: <A> @A { compare :: A -> A -> Ordering; } derive = <T> => \x. x
Ord @Status :: derive
compare ok err
"#;
    assert_eq!(eval_tlc_file(ord_src).unwrap(), Value::Atom("lt".into()));

    let payload_ord_src = r#"
Ordering :: type { #lt; #eq; #gt; };
Result :: type { #ok: { value : Int; }; #err; };
lhs :: Result = #ok { value = 1; };
rhs :: Result = #ok { value = 2; };
Ord :: <A> @A { compare :: A -> A -> Ordering; } derive = <T> => \x. x
Ord @Result :: derive
compare lhs rhs
"#;
    assert_eq!(
        eval_tlc_file(payload_ord_src).unwrap(),
        Value::Atom("lt".into())
    );
}

// ─── Phase 13: conditional (parametric) witnesses ─────────────────────────────

/// Direct call site: `eq` on two `List Int` resolves the conditional witness
/// `Eq @(List A) :: <A: Eq>` by structurally matching `List A` against `List Int`
/// and applying it to the recursively resolved `Eq @Int` component dict.
#[test]
fn tlc_conditional_list_witness_direct() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Eq @(List A) :: <A: Eq> { eq = \xs ys. true; }
eq {1; 2;} {1; 2;}
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
}

/// Indirect / polymorphic call site: inside a `<A: Eq>`-bounded function, the
/// `eq` call on `List A` resolves the conditional witness against the abstract
/// `A`, threading the function's own component dict into the list witness. At the
/// outer call the dict is the concrete `Eq @Int` witness.
#[test]
fn tlc_conditional_list_witness_indirect() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Eq @(List A) :: <A: Eq> { eq = \xs ys. true; }
useEq :: <A: Eq> List A -> List A -> Bool
  = xs ys => eq xs ys;
useEq {1;} {1;}
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
}
/// End-to-end component dispatch through a parametric record alias: the
/// `Eq @(Pair A)` witness compares `fst` fields with the element `eq`. Because
/// witness fields are checked against the (instantiated) method signature, the
/// lambda's params are typed `Pair A`, so `p.fst : A` and the element `eq`
/// dispatches through the witness's own component dict — proving the `Eq @Int`
/// dict is genuinely threaded in (the result depends on it).
#[test]
fn tlc_conditional_record_alias_witness_uses_component() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Pair :: <A> type { fst : A; snd : A; };
Eq @(Pair A) :: <A: Eq> { eq = \p q. eq p.fst q.fst; }
samePair :: <A: Eq> Pair A -> Pair A -> Bool
  = p q => eq p q;
p1 :: Pair Int = { fst = 1; snd = 2; };
p2 :: Pair Int = { fst = 1; snd = 9; };
samePair p1 p2
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
}

/// Negative direction of the component-dispatch test: differing `fst` fields make
/// the element `eq` (and thus the derived `Pair` equality) return false.
#[test]
fn tlc_conditional_record_alias_witness_component_false() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Pair :: <A> type { fst : A; snd : A; };
Eq @(Pair A) :: <A: Eq> { eq = \p q. eq p.fst q.fst; }
samePair :: <A: Eq> Pair A -> Pair A -> Bool
  = p q => eq p q;
p1 :: Pair Int = { fst = 1; snd = 2; };
p2 :: Pair Int = { fst = 7; snd = 2; };
samePair p1 p2
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(false));
}
/// `type_key` expands parametric `AliasApply` targets: a concrete witness on a
/// `Pair Int`-typed operand dispatches through the THIR interpreter (`run`)
/// instead of being flagged ambiguous. The custom `(==)` returns false even for
/// structurally-equal pairs, proving the witness — not structural equality — ran.
#[test]
fn run_operator_dispatch_on_alias_apply_operand() {
    let src = r#"
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Pair :: <A> type { fst : A; snd : A; };
Eq @(Pair Int) :: { (==) = \a b. false; }
p1 :: Pair Int = { fst = 1; snd = 2; };
p2 :: Pair Int = { fst = 1; snd = 2; };
p1 == p2
"#;
    assert_eq!(run(src), Value::Bool(false));
}
/// Nested parametric alias (`Pair (Pair Int)`): resolving `Eq @(Pair A)` binds
/// the witness param `A` to the inner `Pair Int`. The binding must keep its
/// `AliasApply` shape so the recursive `get_dict_expr(Eq, Pair Int)` re-resolves
/// it through `Eq @(Pair A)` again down to `Eq @Int` — alias-expanding the bound
/// type here would strand the inner `Int`, yielding a `Nothing` component dict
/// and a refused evaluation. The inner `fst` fields are equal, so the result is
/// `true` and the genuine threaded component dict ran.
#[test]
fn tlc_conditional_nested_alias_witness_threads_inner_component() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Pair :: <A> type { fst : A; snd : A; };
Eq @(Pair A) :: <A: Eq> { eq = \p q. eq p.fst q.fst; }
p1 :: Pair (Pair Int) = { fst = { fst = 1; snd = 2; }; snd = { fst = 3; snd = 4; }; };
p2 :: Pair (Pair Int) = { fst = { fst = 1; snd = 8; }; snd = { fst = 9; snd = 4; }; };
eq p1 p2
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
}

/// Negative direction of the nested-alias test: the inner `fst` fields differ, so
/// the recursively threaded element `eq` discriminates and the whole comparison
/// is `false` — proving the inner component dict actually ran rather than a
/// vacuous match.
#[test]
fn tlc_conditional_nested_alias_witness_component_false() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Pair :: <A> type { fst : A; snd : A; };
Eq @(Pair A) :: <A: Eq> { eq = \p q. eq p.fst q.fst; }
p1 :: Pair (Pair Int) = { fst = { fst = 1; snd = 2; }; snd = { fst = 3; snd = 4; }; };
p2 :: Pair (Pair Int) = { fst = { fst = 7; snd = 8; }; snd = { fst = 9; snd = 4; }; };
eq p1 p2
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(false));
}

/// Conditional witness over a parametric *union* alias (`Box A`): matching the
/// witness target `Box A` against a concrete `Box Int` must recurse into the
/// variant payload to bind `A` (the `Union` arm of `unify_env`). Without it the
/// two normalized union bodies compare equal without pinning `A`, the candidate
/// is skipped, and the witness refuses. The body threads the element `eq`; equal
/// payloads give `true`.
#[test]
fn tlc_conditional_union_alias_witness_threads_component() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Box :: <A> type { #box: { value: A; }; };
Eq @(Box A) :: <A: Eq> { eq = \x y. match x { | #box { value = a; } => match y { | #box { value = b; } => eq a b; }; }; }
b1 :: Box Int = #box { value = 1; };
b2 :: Box Int = #box { value = 1; };
eq b1 b2
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
}

/// Negative direction of the union-alias test: differing payloads make the
/// threaded element `eq` return `false`.
#[test]
fn tlc_conditional_union_alias_witness_component_false() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Box :: <A> type { #box: { value: A; }; };
Eq @(Box A) :: <A: Eq> { eq = \x y. match x { | #box { value = a; } => match y { | #box { value = b; } => eq a b; }; }; }
b1 :: Box Int = #box { value = 1; };
b2 :: Box Int = #box { value = 2; };
eq b1 b2
"#;
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(false));
}

// Increment 5: method-name resolution — eval invariant tests
// ---------------------------------------------------------------------------

/// T-INV-5: `eq 1 2` type-checks (THIR is complete) but has no runtime value yet
/// (no dictionary-passing).  The interpreter must refuse with `UnboundBinding`
/// rather than guessing a value — the oracle must not invent semantics.
#[test]
fn t_inv5_method_call_type_checks_but_refuses_eval() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\neq 1 2";
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::UnboundBinding(_)),
        "expected EvalError::UnboundBinding for un-dispatched method call, got {err:?}"
    );
}

// ─── Increment 6: dictionary-passing / instance resolution ────────────────────

/// Basic dispatch: `eq 1 2` resolves to the `Eq @Int` witness body.
#[test]
fn dispatch_basic_method_call() {
    let src = "
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \\a b. true; }
eq 1 2
";
    assert_eq!(run(src), Value::Bool(true));
}

#[test]
fn dispatch_imported_named_witness() {
    let src = r#"
w :: import "witness_eq_int_a.zt";
Eq :: <A> @A { eq :: A -> A -> Bool; }
eq 1 2
"#;
    assert_eq!(run_in_imports(src), Value::Bool(true));
}

/// Cross-module type-directed selection: a dep exports TWO concrete witnesses for
/// the same constraint method (`Eq @Int` and `Eq @Bool`). Each call site must
/// dispatch to the instance whose target matches the operand type. Before the
/// fix the interpreter resolved imported methods by NAME only, so two same-named
/// instances were ambiguous and the call refused (`UnboundBinding`). The `Eq @Bool`
/// witness returns a constant `false` (≠ structural equality of `true`/`true`),
/// so the result discriminates a correct dispatch from a wrong one.
#[test]
fn dispatch_imported_type_directed_witness_selection() {
    let src = r#"
w :: import "witness_eq_int_bool.zt";
Eq :: <A> @A { eq :: A -> A -> Bool; }
(eq 1 1, eq true true)
"#;
    let v = run_import(src);
    match v {
        Value::Tuple(fields) => {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].value.peek(), Some(Value::Bool(true)));
            assert_eq!(fields[1].value.peek(), Some(Value::Bool(false)));
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

/// Type-directed selection: two witnesses for the same constraint, each with a
/// different target type — the dispatch must pick the right one per call site.
#[test]
fn dispatch_type_directed_witness_selection() {
    let src = "
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \\a b. true; }
Eq @Bool :: { eq = \\a b. false; }
(eq 1 2, eq true false)
";
    let v = run(src);
    match v {
        Value::Tuple(fields) => {
            assert_eq!(fields.len(), 2);
            // force_deep (called inside eval_file) ensures all thunk fields are forced.
            assert_eq!(fields[0].value.peek(), Some(Value::Bool(true)));
            assert_eq!(fields[1].value.peek(), Some(Value::Bool(false)));
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

/// Refusal: method with constraint but NO witness → still `UnboundBinding`.
/// The oracle must decline rather than invent a value.
#[test]
fn dispatch_refusal_no_witness() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\neq 1 2";
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::UnboundBinding(_)),
        "expected UnboundBinding when no witness is in scope, got {err:?}"
    );
}

// ─── Increment 7: operator-method dispatch ────────────────────────────────────

/// Custom `(==)` on a scalar overrides builtin structural equality.
/// `1 == 1` is builtin-`true` but the witness returns `false`.
#[test]
fn op_dispatch_eq_overrides_builtin() {
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \\a b. false; }
1 == 1
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn op_dispatch_imported_witness_overrides_builtin() {
    let src = r#"
w :: import "witness_eq_int_operator.zt";
1 == 1
"#;
    assert_eq!(run_in_imports(src), Value::Bool(false));
}

#[test]
fn op_dispatch_imported_bounded_operator_uses_witness() {
    let src = r#"
w :: import "witness_eq_int_operator_bounded.zt";
w
"#;
    assert_eq!(run_in_imports(src), Value::Bool(false));
}

/// `!=` negates the `(==)` field when no `(!=)` field is present.
#[test]
fn op_dispatch_ne_negates_eq() {
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \\a b. false; }
1 != 1
";
    // Custom (==) says false → ne returns true.
    assert_eq!(run(src), Value::Bool(true));
}

/// Custom `(<)` on a scalar overrides builtin ordering.
/// `2 < 1` is builtin-`false` but the witness returns `true`.
#[test]
fn op_dispatch_lt_overrides_builtin() {
    let src = "
Ord :: <A> @A { (<) :: A -> A -> Bool; }
Ord @Int :: { (<) = \\a b. true; }
2 < 1
";
    assert_eq!(run(src), Value::Bool(true));
}

/// All six comparison operators dispatch to the appropriate witness field.
#[test]
fn op_dispatch_all_six_operators() {
    let src = "
Cmp :: <A> @A {
  (==) :: A -> A -> Bool;
  (!=) :: A -> A -> Bool;
  (<)  :: A -> A -> Bool;
  (<=) :: A -> A -> Bool;
  (>)  :: A -> A -> Bool;
  (>=) :: A -> A -> Bool;
}
Cmp @Int :: {
  (==)  = \\a b. false;
  (!=)  = \\a b. false;
  (<)   = \\a b. false;
  (<=)  = \\a b. false;
  (>)   = \\a b. false;
  (>=)  = \\a b. false;
}
(1 == 2, 1 != 2, 1 < 2, 1 <= 2, 1 > 2, 1 >= 2)
";
    let v = run(src);
    match v {
        Value::Tuple(fields) => {
            assert_eq!(fields.len(), 6);
            for f in fields.iter() {
                assert_eq!(f.value.peek(), Some(Value::Bool(false)));
            }
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

/// Alias-resolved key match: a witness whose target is a named type alias
/// must still dispatch when the operand's inferred type is the structural record.
/// This verifies the D4 alias-resolution fix in `type_key`.
#[test]
fn op_dispatch_alias_resolved_key() {
    let src = "
Point :: type { x : Int; y : Int; };
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Point :: { (==) = \\a b. false; }
{ x = 1; y = 2; } == { x = 1; y = 2; }
";
    // Without alias resolution in type_key, dispatch would miss and builtin
    // values_equal would return true (structural equality). With it: false.
    assert_eq!(run(src), Value::Bool(false));
}

/// Builtin fallback: with no witness, `1 == 1` uses structural equality.
#[test]
fn op_dispatch_eq_builtin_fallback() {
    assert_eq!(run("1 == 1"), Value::Bool(true));
    assert_eq!(run("1 == 2"), Value::Bool(false));
}

/// Ordering on a non-scalar type-checks (D6 relaxation) when an ordering
/// constraint exists, but eval refuses via `cmp_op` when no witness matches.
#[test]
fn op_dispatch_ordering_non_scalar_no_witness_refuses() {
    let src = "
Ord :: <A> @A { (<) :: A -> A -> Bool; }
{ x = 1; } < { x = 2; }
";
    // Type-checks (no THIR error) because Ord constraint declares (<).
    // Eval refuses: no Ord @{...} witness → cmp_op returns TypeMismatch.
    let err = run_err(src);
    assert!(
        matches!(err, EvalError::TypeMismatch { .. }),
        "expected TypeMismatch for non-scalar < with no witness, got {err:?}"
    );
}

/// Ordering on a non-scalar WITH a witness dispatches correctly.
#[test]
fn op_dispatch_ordering_non_scalar_with_witness() {
    let src = "
Point :: type { x : Int; y : Int; };
Ord :: <A> @A { (<) :: A -> A -> Bool; }
Ord @Point :: { (<) = \\a b. true; }
{ x = 2; y = 0; } < { x = 1; y = 0; }
";
    // Custom (<) always returns true even though 2 > 1.
    assert_eq!(run(src), Value::Bool(true));
}

#[test]
fn op_dispatch_bounded_eq_uses_witness_dict() {
    let src = r#"
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. false; }
same :: <A: Eq> A -> A -> Bool
  = x y => x == y;
same 1 1
"#;
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn op_dispatch_bounded_ne_negates_eq_witness() {
    let src = r#"
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. false; }
same :: <A: Eq> A -> A -> Bool
  = x y => x != y;
same 1 1
"#;
    assert_eq!(run(src), Value::Bool(true));
}

#[test]
fn op_dispatch_bounded_lt_uses_witness_dict() {
    let src = r#"
Ord :: <A> @A { (<) :: A -> A -> Bool; }
Ord @Int :: { (<) = \a b. true; }
less :: <A: Ord> A -> A -> Bool
  = x y => x < y;
less 2 1
"#;
    assert_eq!(run(src), Value::Bool(true));
}

// ─── Increment 8: polymorphic constraint dispatch ─────────────────────────────

/// Headline test: `same 1 1` evaluates to `Bool true` via witness-dict injection.
/// The `eq` method inside `same` dispatches through the `Eq @Int` witness because
/// the injected WitnessDict resolves the ambiguous TypeVar at the call site.
#[test]
fn dispatch_polymorphic_method_inside_bounded_fn() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
same :: <A: Eq> A -> A -> Bool
  = x y => eq x y;
same 1 1
"#;
    assert_eq!(run(src), Value::Bool(true));
}

/// Default-body fallback: a witness that omits the method uses the default body
/// defined in the constraint.  Witness `Eq @Int :: {}` is valid (method has a
/// default), and calling `eq 1 2` uses the default clause `= _ _ => true;`.
#[test]
fn dispatch_default_method_used_when_field_absent() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool = _ _ => true; }
Eq @Int :: {}
eq 1 2
"#;
    assert_eq!(run(src), Value::Bool(true));
}

/// Regression: an unbounded wrapper calls a bounded function indirectly. The
/// TLC-first default runs it via dictionary passing; the THIR oracle still
/// refuses with its known `UnresolvedWitness` limitation.
#[test]
fn tlc_first_default_runs_indirect_bounded_call() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
same :: <A: Eq> A -> A -> Bool
  = x y => eq x y;
wrapper :: Int -> Bool
  = n => same n n;
wrapper 1
"#;
    assert_eq!(eval_file(src).unwrap(), Value::Bool(true));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(true));
    let err = run_thir_err(src);
    assert!(
        matches!(err, EvalError::UnresolvedWitness { .. }),
        "expected UnresolvedWitness for THIR indirect bounded-fn call, got {err:?}"
    );
}

/// Regression: the default-body fallback must NOT fire for ambiguous type keys
/// even when the method has a default body. The TLC-first default uses the real
/// witness and returns `false`; the THIR oracle still refuses with
/// `UnresolvedWitness` rather than silently returning the default `Bool(true)`.
#[test]
fn dispatch_default_not_used_when_witness_exists_but_indirect() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool = _ _ => true; }
Eq @Int :: { eq = \a b. a == b; }
same :: <A: Eq> A -> A -> Bool
  = x y => eq x y;
useit :: Int -> Bool
  = _ => same 1 2;
useit 0
"#;
    assert_eq!(eval_file(src).unwrap(), Value::Bool(false));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Bool(false));
    let err = run_thir_err(src);
    assert!(
        matches!(err, EvalError::UnresolvedWitness { .. }),
        "expected UnresolvedWitness (not Bool(true) wrong answer), got {err:?}"
    );
}
// ─── type_key dispatch arms ───────────────────────────────────────────────────

#[test]
fn type_key_float_witness() {
    // type_key hits TypeKind::Float arm when witness target is @Float
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Float :: { (==) = \\a b. false; }
1.0 == 1.0
";
    // Without witness: builtin says true; with Float witness: false.
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn type_key_text_witness() {
    // type_key hits TypeKind::Text arm
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Text :: { (==) = \\a b. false; }
\"hi\" == \"hi\"
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn type_key_atom_witness() {
    // type_key hits TypeKind::Atom arm for singleton atom type @#hello
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @#hello :: { (==) = \\a b. false; }
#hello == #hello
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn type_key_tuple_witness() {
    // type_key hits TypeKind::Tuple arm for (Int, Int) witness target
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @(Int, Int) :: { (==) = \\a b. false; }
(1, 2) == (1, 2)
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn type_key_fixed_width_witness_overrides_builtin() {
    let src = r#"
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @u8 :: { (==) = \a b. false; }
x :: u8 = 1u8;
y :: u8 = 1u8;
x == y
"#;
    assert_eq!(eval_thir_file(src).unwrap(), Value::Bool(false));
}

#[test]
fn type_key_optional_method_dispatch() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @(Int?) :: { eq = \a b. false; }
x :: Int? = #some (1);
y :: Int? = #some (1);
eq x y
"#;
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn type_key_maybe_method_dispatch() {
    let src = r#"
S :: type { p? : Int; };
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @(Maybe Int) :: { eq = \a b. false; }
a :: S = { p = 1; };
b :: S = { p = 1; };
eq a.p b.p
"#;
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn type_key_patch_witness_dispatches_for_patch_values() {
    let src = r#"
Config :: type { port : Int; host : Text; };
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @(Patch Config) :: { (==) = \a b. false; }
p1 :: Patch Config = { port = 8080; };
p2 :: Patch Config = { port = 8080; };
p1 == p2
"#;
    assert_eq!(eval_thir_file(src).unwrap(), Value::Bool(false));
}

#[test]
fn type_key_function_method_dispatch() {
    let src = r#"
Show :: <A> @A { show :: A -> Text; }
Show @(Int -> Int) :: { show = \f. "function"; }
inc ::= \n. n + 1;
show inc
"#;
    assert_eq!(run(src), Value::Text("function".into()));
}

// ─── derive operator / default-method regressions ─────────────────────────────

/// Regression (operator self-recursion): an operator witness whose body
/// delegates to the same operator on the same primitive — `(==) = \a b. a == b`
/// — must lower the inner `==` to the builtin, not a call back into the witness
/// being defined. Otherwise `1 == 1` recurses until the stack overflows.
#[test]
fn op_dispatch_operator_witness_delegates_to_builtin() {
    let src = r#"
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. a == b; }
1 == 1
"#;
    assert_eq!(run(src), Value::Bool(true));
}

/// Regression (derived record operator + delegating component witness): a record
/// derives `(==)`/`(!=)`; its `Int` fields dispatch to an `Eq @Int` operator
/// witness defined as `\a b. a == b`. The delegating body must reach the builtin
/// so field comparison terminates. `p1 == p2` (equal fields) and `p1 != p3`
/// (differing) are both `true`.
#[test]
fn op_dispatch_derived_record_operator_no_self_recursion() {
    let src = r#"
Eq :: <A> @A {
  (==) :: A -> A -> Bool;
  (!=) :: A -> A -> Bool;
} derive
Eq @Int :: {
  (==) = \a b. a == b;
  (!=) = \a b. a != b;
}
Point :: type { x : Int; y : Int; };
Eq @Point :: derive
p1 :: Point = { x = 5; y = 5; };
p2 :: Point = { x = 5; y = 5; };
p3 :: Point = { x = 1; y = 2; };
(p1 == p2, p1 != p3)
"#;
    match run(src) {
        Value::Tuple(fields) => {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].value.peek(), Some(Value::Bool(true)));
            assert_eq!(fields[1].value.peek(), Some(Value::Bool(true)));
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

/// Regression (default-method dict threading): a constraint declares a defaulted
/// method `neq?` whose default body calls the sibling `eq`. A witness that omits
/// `neq` gets the default, and the default's `eq` must dispatch through the
/// witness's own dict (threaded as the constraint's self-dict) instead of a
/// `Nothing` placeholder. `checkNeq 1 1` -> false (equal), `checkNeq 1 2` -> true.
#[test]
fn dispatch_default_method_body_references_sibling() {
    let src = r#"
MyEq :: <A> @A {
  eq    :: A -> A -> Bool;
  neq?  :: A -> A -> Bool
    = a b => if eq a b then false else true;
} derive
MyEq @Int :: { eq = \a b. a == b; }
checkNeq :: <A: MyEq> A -> A -> Bool
  = a b => neq a b;
(checkNeq 1 1, checkNeq 1 2)
"#;
    match run(src) {
        Value::Tuple(fields) => {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].value.peek(), Some(Value::Bool(false)));
            assert_eq!(fields[1].value.peek(), Some(Value::Bool(true)));
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

#[test]
fn tlc_derive_nullary_union_eq_same_and_different_variants() {
    let same = r#"
Color :: type { #red; #blue; };
r1 :: Color = #red;
r2 :: Color = #red;
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Color :: derive
eq r1 r2
"#;
    assert_eq!(eval_tlc_file(same).unwrap(), Value::Bool(true));

    let different = r#"
Color :: type { #red; #blue; };
r1 :: Color = #red;
r2 :: Color = #blue;
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Color :: derive
eq r1 r2
"#;
    assert_eq!(eval_tlc_file(different).unwrap(), Value::Bool(false));
}

#[test]
fn tlc_higher_rank_apply_id_runs() {
    let src = r#"
id :: <A> A -> A
  = x => x;
applyId :: (<A> A -> A) -> { i : Int; t : Text; }
  = f => { i = f 1; t = f "x"; };
applyId id
"#;
    assert_eq!(
        eval_tlc_file(src).unwrap().to_string(),
        r#"{ i = 1;  t = "x" }"#
    );
}

#[test]
fn tlc_higher_rank_show_both_runs() {
    let src = r#"
Show :: <A> @A { show :: A -> Text; }
Show @Int :: { show = \n. "int"; }
Show @Bool :: { show = \b. "bool"; }
showBoth :: (<A: Show> A -> Text) -> { left : Text; right : Text; } =
  \render. { left = render 1; right = render true; };
showBoth (\x. show x)
"#;
    assert_eq!(
        eval_tlc_file(src).unwrap().to_string(),
        r#"{ left = "int";  right = "bool" }"#
    );
}
