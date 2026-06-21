// ── Phase 5 tests: dictionary-passing elaboration ────────────────────────────

use super::tlc_of;
use crate::*;

fn has_get_field(m: &TlcModule, field_name: &str) -> bool {
    m.expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::GetField(_, field) if field == field_name))
}

fn has_nested_binary_method_app(m: &TlcModule, field_name: &str) -> bool {
    m.expr_arena.iter().any(|(_, e)| {
        let TlcExpr::App(first, _) = e else {
            return false;
        };
        let TlcExpr::App(method, _) = &m.expr_arena[*first] else {
            return false;
        };
        matches!(&m.expr_arena[*method], TlcExpr::GetField(_, field) if field == field_name)
    })
}

/// A constraint witness decl lowers to a `TlcDecl::Value` whose body is a `Record`.
#[test]
fn witness_decl_lowers_to_record_value() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
1
"#,
    );
    let has_record = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Record(_)));
    assert!(has_record, "expected TlcExpr::Record for witness dict body");
}

/// A bounded function gets TyLam + dict Lam wrapping; its type starts with ForAll.
#[test]
fn bounded_function_gets_forall_and_dict_lam() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool
  = x y => eq x y;
same 1 1
"#,
    );
    let has_forall = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::ForAll(_, _, _)));
    assert!(
        has_forall,
        "expected ForAll type for bounded function `same`"
    );
    let has_tylam = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyLam(_, _, _)));
    assert!(has_tylam, "expected TyLam body for bounded function `same`");
}

/// Inside a bounded function body, a constraint-method call becomes GetField.
#[test]
fn constraint_method_call_lowers_to_get_field() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool
  = x y => eq x y;
same 1 1
"#,
    );
    let has_get_field = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::GetField(_, _)));
    assert!(
        has_get_field,
        "expected TlcExpr::GetField for constraint method call inside bounded function"
    );
}

#[test]
fn operator_witness_binary_lowers_to_get_field() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. false; }
result :: Bool = 1 == 1
result
"#,
    );

    assert!(
        has_get_field(&m, "=="),
        "expected TlcExpr::GetField(_, \"==\") for witnessed equality"
    );
    assert!(
        has_nested_binary_method_app(&m, "=="),
        "expected nested App(App(GetField(_, \"==\"), _), _) for witnessed equality"
    );
}

#[test]
fn bounded_operator_witness_binary_lowers_to_get_field() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. false; }
same :: <A: Eq> A -> A -> Bool
  = x y => x == y;
same 1 1
"#,
    );

    assert!(
        has_get_field(&m, "=="),
        "expected TlcExpr::GetField(_, \"==\") in bounded witnessed equality"
    );
    assert!(
        has_nested_binary_method_app(&m, "=="),
        "expected nested App(App(GetField(_, \"==\"), _), _) in bounded witnessed equality"
    );
}

/// A call to a bounded function injects TyApp for the type argument.
/// The call must be inside a declaration (not the final expression) so that
/// TLC lowers it.
#[test]
fn call_to_bounded_function_injects_tyapp() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool
  = x y => eq x y;
result :: Bool = same 1 1
result
"#,
    );
    let has_tyapp = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyApp(_, _)));
    assert!(
        has_tyapp,
        "expected TlcExpr::TyApp at call site of bounded function `same`"
    );
}

/// The bounded function body wraps with at least one extra Lam for the dict param.
#[test]
fn bounded_function_body_contains_dict_lam() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool
  = x y => eq x y;
same 1 1
"#,
    );
    // dict Lam + 2 value param Lams (x and y) = at least 3
    let lam_count = m
        .expr_arena
        .iter()
        .filter(|(_, e)| matches!(e, TlcExpr::Lam(_, _, _)))
        .count();
    assert!(
        lam_count >= 3,
        "expected at least 3 Lam nodes (dict + value params), got {}",
        lam_count
    );
}

/// A constraint with two methods produces a witness Record with two fields.
#[test]
fn witness_with_two_methods_has_two_record_fields() {
    let m = tlc_of(
        r#"
Ord :: <A> @A { lt :: A -> A -> Bool; gt :: A -> A -> Bool; }
Ord @Int :: { lt = \a b. a < b; gt = \a b. a > b; }
1
"#,
    );
    let two_field_record = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Record(fields) = e {
            fields.len() == 2
        } else {
            false
        }
    });
    assert!(
        two_field_record,
        "expected a Record with exactly 2 fields for Ord @Int witness"
    );
}

#[test]
fn derive_witness_synthesizes_method_field() {
    let m = tlc_of(
        r#"
Point :: type { x : Int; y : Int; }
p1 :: Point = { x = 1; y = 2; }
p2 :: Point = { x = 1; y = 2; }
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Point :: derive
eq p1 p2
"#,
    );
    let has_eq_field = m.expr_arena.iter().any(|(_, e)| {
        matches!(e, TlcExpr::Record(fields) if fields.iter().any(|(name, _)| name == "eq"))
    });
    assert!(
        has_eq_field,
        "derive witness should synthesize an `eq` field"
    );
}

/// Every expression in the module still has a type entry after Phase 5 elaboration.
#[test]
fn every_expr_has_type_after_phase5() {
    let m = tlc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
same :: <A: Eq> A -> A -> Bool
  = x y => eq x y;
same 1 1
"#,
    );
    for (id, _) in m.expr_arena.iter() {
        assert!(
            m.expr_types.contains_key(&id),
            "expr {:?} missing from expr_types after Phase 5 elaboration",
            id
        );
    }
}

/// A higher-kinded witness method (`Functor @List { map = \f xs. xs; }`) wraps
/// the field body in a `TyLam` per method-level param. The witness is not
/// conditional and there is no bounded function, so the only `TyLam` source is
/// the method-polymorphism wrapping — its presence verifies that path.
#[test]
fn hkt_witness_method_wrapped_in_tylam() {
    let m = tlc_of(
        r#"
Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }
Functor @List :: { map = \f xs. xs; }
1
"#,
    );
    let has_tylam = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyLam(_, _, _)));
    assert!(
        has_tylam,
        "expected a TyLam wrapping the polymorphic `map` witness field"
    );
}

/// A polymorphic constraint-method call (`map f …` inside `mapTwice`) elaborates
/// to `TyApp` on the dict-fetched method, instantiating its method-level params.
#[test]
fn hkt_method_call_elaborates_tyapp() {
    let m = tlc_of(
        r#"
Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }
Functor @List :: { map = \f xs. xs; }
mapTwice :: <F: Functor, A> (A -> A) -> F A -> F A
  = f xs => map f (map f xs);
1
"#,
    );
    let has_tyapp = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyApp(_, _)));
    assert!(
        has_tyapp,
        "expected TyApp at the polymorphic `map` call site"
    );
}

/// A partial-application witness target (`Functor @(Result E)`) elaborates to a
/// TLC module without error — every expr gets a type and the witness method is
/// `TyLam`-wrapped.
#[test]
fn hkt_partial_application_witness_elaborates() {
    let m = tlc_of(
        r#"
Result :: <E, A> type { ok : A; err : E; }
Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }
Functor @(Result E) :: <E> { map = \f r. r; }
1
"#,
    );
    let has_tylam = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyLam(_, _, _)));
    assert!(has_tylam, "expected TyLam for the polymorphic `map` field");
    for (id, _) in m.expr_arena.iter() {
        assert!(m.expr_types.contains_key(&id), "expr {id:?} missing a type");
    }
}

/// A constraint method may declare a type param it never uses in its signature
/// (`phantom :: <A, B> F A -> F A` — `B` is unused). The witness field must be
/// wrapped in one `TyLam` per param that actually appears in the signature (here
/// just `A`), matching the call site's `TyApp` count. Wrapping per *declared*
/// param instead leaves a residual type abstraction the call site never
/// instantiates — an ill-typed TLC term.
#[test]
fn hkt_unused_method_param_wraps_only_signature_params() {
    let m = tlc_of(
        r#"
Functor :: <F :: Type -> Type> @F { phantom :: <A, B> F A -> F A; }
Functor @List :: { phantom = \xs. xs; }
useIt :: <F: Functor, A> F A -> F A
  = xs => phantom xs;
1
"#,
    );
    fn count_leading_tylam(m: &TlcModule, mut id: TlcExprId) -> usize {
        let mut n = 0;
        while let TlcExpr::TyLam(_, _, body) = &m.expr_arena[id] {
            n += 1;
            id = *body;
        }
        n
    }
    let field = m
        .expr_arena
        .iter()
        .find_map(|(_, e)| match e {
            TlcExpr::Record(fields) => fields
                .iter()
                .find(|(name, _)| name == "phantom")
                .map(|(_, eid)| *eid),
            _ => None,
        })
        .expect("witness dict must carry a `phantom` field");
    assert_eq!(
        count_leading_tylam(&m, field),
        1,
        "`phantom` declares <A, B> but uses only A; its witness field must be \
         wrapped in exactly 1 TyLam (signature-present params), not 2 (declared)"
    );
}

fn has_witness_record_field(m: &TlcModule, field: &str) -> bool {
    m.expr_arena
        .iter()
        .any(|(_, expr)| matches!(expr, TlcExpr::Record(fields) if fields.iter().any(|(name, _)| name == field)))
}

fn has_nested_eq_application(m: &TlcModule) -> bool {
    m.expr_arena.iter().any(|(_, expr)| {
        let TlcExpr::App(left, _) = expr else {
            return false;
        };
        let TlcExpr::App(func, _) = &m.expr_arena[*left] else {
            return false;
        };
        matches!(&m.expr_arena[*func], TlcExpr::GetField(_, field) if field == "eq")
    })
}

#[test]
fn derive_named_tuple_eq_uses_tuple_case_with_named_patterns() {
    let m = tlc_of(
        r#"
Pair :: type (x : Int, y : Text)
p1 :: Pair = (x = 1, y = "a")
p2 :: Pair = (x = 1, y = "a")
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Pair :: derive
eq p1 p2
"#,
    );
    assert!(has_witness_record_field(&m, "eq"));
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, expr)| matches!(expr, TlcExpr::Case(_, _)))
    );
    assert!(m.expr_arena.iter().any(|(_, expr)| {
        matches!(
            expr,
            TlcExpr::Case(_, alts)
                if alts.iter().any(|alt| matches!(alt.pat, TlcPat::Tuple(_)))
        )
    }));
}

#[test]
fn derive_record_component_uses_existing_witness_call() {
    let m = tlc_of(
        r#"
Name :: type Text
Person :: type { name : Name; age : Int; }
p1 :: Person = { name = "a"; age = 1; }
p2 :: Person = { name = "a"; age = 1; }
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Name :: { eq = \a b. a == b; }
Eq @Person :: derive
eq p1 p2
"#,
    );
    assert!(has_get_field(&m, "eq"));
    assert!(has_nested_eq_application(&m));
}

#[test]
fn derive_operator_neq_synthesizes_bang_equal_field() {
    let m = tlc_of(
        r#"
Point :: type { x : Int; y : Int; }
p1 :: Point = { x = 1; y = 2; }
p2 :: Point = { x = 1; y = 3; }
Eq :: <A> @A { (==) :: A -> A -> Bool; (!=) :: A -> A -> Bool; } derive
Eq @Point :: derive
p1 != p2
"#,
    );
    assert!(has_witness_record_field(&m, "=="));
    assert!(has_witness_record_field(&m, "!="));
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, expr)| { matches!(expr, TlcExpr::Builtin(BuiltinOp::Ne, _, _)) })
    );
}
