use zutai_hir::ty::HirTyRef;
use zutai_semantic::analyze;
use zutai_semantic::ty::{BOOL_TY, FLOAT_TY, INT_TY, NONE_TY, TEXT_TY, Ty, TyInterner, UNKNOWN_TY};
use zutai_syntax::parse;

// ── TyInterner contract ───────────────────────────────────────────────────────

#[test]
fn pre_interned_primitives_at_correct_positions() {
    let interner = TyInterner::new();
    assert_eq!(interner.get(UNKNOWN_TY), &Ty::Unknown);
    assert_eq!(interner.get(INT_TY), &Ty::Int);
    assert_eq!(interner.get(FLOAT_TY), &Ty::Float);
    assert_eq!(interner.get(TEXT_TY), &Ty::Text);
    assert_eq!(interner.get(BOOL_TY), &Ty::Bool);
    assert_eq!(interner.get(NONE_TY), &Ty::None);
}

#[test]
fn intern_deduplicates_equal_types() {
    let mut interner = TyInterner::new();
    let a = interner.intern(Ty::Function {
        param: INT_TY,
        ret: TEXT_TY,
    });
    let b = interner.intern(Ty::Function {
        param: INT_TY,
        ret: TEXT_TY,
    });
    assert_eq!(a, b);
}

// ── Symbol::ty write-back ─────────────────────────────────────────────────────

fn first_decl_sym_ty(src: &str) -> Option<HirTyRef> {
    let parsed = parse(src);
    let result = analyze(&parsed.syntax());
    let decl_id = *result.hir.decls.first()?;
    let sym_id = match result.hir.decls_arena.get(decl_id) {
        zutai_hir::decl::HirDecl::Value { name, .. } => *name,
        zutai_hir::decl::HirDecl::Function { name, .. } => *name,
        zutai_hir::decl::HirDecl::TypeDef { name, .. } => *name,
    };
    result.hir.symbols.get(sym_id).ty
}

#[test]
fn annotated_int_binding_gets_int_ty() {
    let ty_ref = first_decl_sym_ty("x : Int = 42\nx");
    assert_eq!(
        ty_ref,
        Some(HirTyRef(INT_TY.0)),
        "x : Int should resolve to INT_TY"
    );
}

#[test]
fn annotated_text_binding_gets_text_ty() {
    let ty_ref = first_decl_sym_ty("s : Text = \"hello\"\ns");
    assert_eq!(
        ty_ref,
        Some(HirTyRef(TEXT_TY.0)),
        "s : Text should resolve to TEXT_TY"
    );
}

#[test]
fn annotated_bool_binding_gets_bool_ty() {
    let ty_ref = first_decl_sym_ty("b : Bool = true\nb");
    assert_eq!(
        ty_ref,
        Some(HirTyRef(BOOL_TY.0)),
        "b : Bool should resolve to BOOL_TY"
    );
}

#[test]
fn unknown_type_ref_gives_unknown_not_panic() {
    // `NotAType` is not defined — should elaborate to Unknown without panicking
    let ty_ref = first_decl_sym_ty("x : NotAType = 42\nx");
    assert_eq!(
        ty_ref,
        Some(HirTyRef(UNKNOWN_TY.0)),
        "unknown type ref should be Unknown"
    );
}

#[test]
fn unannotated_binding_has_no_ty() {
    let ty_ref = first_decl_sym_ty("x := 42\nx");
    assert_eq!(
        ty_ref,
        Option::None,
        "inferred binding with no annotation should have no ty"
    );
}
