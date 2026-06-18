// ── lower_pat: literal patterns + record/tuple + HM polymorphism ──────────────

use super::{make_module, tlc_of};
use crate::*;

#[test]
fn inferred_tyvar_as_normalize_binder() {
    // TyApp(TyLamK(Inferred(0), _, TyVar(Inferred(0))), Int) β-reduces to Int.
    // Exercises the TyLamK path with an Inferred TlcTypeVar binder.
    use la_arena::Arena;
    let mut ta: Arena<TlcType> = Arena::new();
    let var = TlcTypeVar::Inferred(0);
    let kind = Kind::ground();
    let tvar = ta.alloc(TlcType::TyVar(var, kind.clone()));
    let lam = ta.alloc(TlcType::TyLamK(var, kind, tvar));
    let int_ty = ta.alloc(TlcType::Prim(PrimTy::Int));
    let app = ta.alloc(TlcType::TyApp(lam, int_ty));
    let mut m = make_module(ta);
    let norm = m.normalize(app).expect("normalize");
    assert!(
        matches!(m.type_arena[norm], TlcType::Prim(PrimTy::Int)),
        "TyLamK with Inferred var β-reduces to Int; got {:?}",
        m.type_arena[norm]
    );
}

#[test]
fn true_false_literal_patterns_lower_to_lit_bool() {
    // A function matching on `true` and `false` — exercises TlcPat::Lit(Bool(true/false)).
    let m = tlc_of(
        r#"toInt :: Bool -> Int {
  | true => 1;
  | false => 0;
}
toInt true"#,
    );
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Case(_, alts) if alts.iter().any(|a| matches!(&a.pat, TlcPat::Lit(Literal::Bool(true)))))),
        "expected TlcPat::Lit(Bool(true)) in a Case alt"
    );
}

#[test]
fn record_pattern_lowers_to_tlc_record_pat() {
    // Record pattern in a match arm exercises ThirPatKind::Record → TlcPat::Record.
    let m = tlc_of(
        r#"Point :: type { x : Int; y : Int; }
getX :: Point -> Int {
  | { x = v; y = _; } => v;
}
getX { x = 42; y = 0; }"#,
    );
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Case(_, alts) if alts.iter().any(|a| matches!(&a.pat, TlcPat::Record(_))))),
        "expected TlcPat::Record in a Case alt"
    );
}

/// An inferred-polymorphic identity function applied to an `Int` value forces
/// TLC to call `lower_binding_ref` → `extract_instantiation` → `match_types`
/// (Function and InferVar arms). This covers lines 152-163 in tlc/lower/expr.rs
/// and lines 158-192 in tlc/lower/types.rs.
///
/// Note: TLC only lowers *declarations*, not the final expression. The call to
/// `id` must appear inside a declaration (here `result := id 42`) so that TLC
/// traverses the Apply and hits `lower_binding_ref` for the poly scheme.
#[test]
fn polymorphic_identity_lowers_with_ty_app() {
    // `id x = x` is inferred as `?0 -> ?0`; THIR puts it in poly_schemes.
    // `result := id 42` is a declaration whose value TLC lowers. When TLC
    // processes the BindingRef to `id`, lower_binding_ref sees the poly scheme
    // and calls extract_instantiation(scheme=[?0], stored=?0->?0, ref=Int->Int).
    let m = tlc_of("id x = x\nresult := id 42\nresult");
    // id + result = 2 declarations
    assert_eq!(m.decls.len(), 2, "expected id and result decls");
    // The module should contain a TyApp expression (type application for id)
    let has_ty_app = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyApp(_, _)));
    assert!(
        has_ty_app,
        "expected TyApp expression for polymorphic id application"
    );
}

/// `wrap := \x. [x;]` has type `?0 -> List(?0)`. Using `wrap` inside a declaration
/// triggers `match_types(List(?0), List(Int))` → the `List(ti)` arm at L194-197.
///
/// Uses `:=` with a lambda so the list body is inferred (not checked against an
/// unresolved InferVar), avoiding the ExpectedList diagnostic from function-decl syntax.
/// Note: TLC only lowers declarations; the call must be inside a decl.
#[test]
fn polymorphic_list_wrapper_covers_match_types_list_arm() {
    let m = tlc_of("wrap := \\x. [x;]\nresult := wrap 42\nresult");
    assert_eq!(m.decls.len(), 2, "expected wrap and result decls");
    let has_ty_app = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::TyApp(_, _)));
    assert!(
        has_ty_app,
        "expected TyApp for polymorphic wrap application"
    );
}

/// `pass x = x` applied to an optional value forces
/// `match_types(Optional(?0), Optional(Int?))` → the `Optional(ti)` arm at L199-202.
///
/// Note: TLC only lowers declarations; the call must be inside a decl.
#[test]
fn polymorphic_identity_with_optional_covers_match_types_optional_arm() {
    let m = tlc_of("pass x = x\nv :: Int? = #none\nresult := pass v\nresult");
    // The poly scheme for `pass` is instantiated with `Int?` → match_types Optional arm hit.
    let _ = m;
}

#[test]
fn tuple_pattern_lowers_to_tlc_tuple_pat() {
    // Positional tuple pattern in a function clause exercises TlcPat::Tuple.
    let m = tlc_of(
        r#"fst :: (Int, Int) -> Int {
  | (x, _) => x;
}
fst (1, 2)"#,
    );
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Case(_, alts) if alts.iter().any(|a| matches!(&a.pat, TlcPat::Tuple(_))))),
        "expected TlcPat::Tuple in a Case alt"
    );
}
