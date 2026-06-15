use crate::*;

fn tlc_of(src: &str) -> TlcModule {
    let parsed = zutai_syntax::parse(src);
    assert!(
        !parsed.has_errors(),
        "parse errors: {:?}",
        parsed.diagnostics()
    );
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "hir errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "thir errors: {:?}",
        thir.diagnostics
    );
    lower_thir(thir.file.as_ref().expect("thir file should be complete"))
}

#[test]
fn tlc_module_is_constructible() {
    use la_arena::Arena;
    use std::collections::HashMap;
    let _m = TlcModule {
        decls: Vec::new(),
        decl_arena: Arena::new(),
        expr_arena: Arena::new(),
        type_arena: Arena::new(),
        expr_types: HashMap::new(),
        spans: HashMap::new(),
    };
}

#[test]
fn monomorphic_int_binding_translates_type() {
    let m = tlc_of("x := 42\nx");
    assert_eq!(m.decls.len(), 1);
    let decl = &m.decl_arena[m.decls[0]];
    let crate::TlcDecl::Value { ty, .. } = decl else {
        panic!("expected Value decl")
    };
    assert_eq!(m.type_arena[*ty], crate::TlcType::Prim(crate::PrimTy::Int));
}

#[test]
fn int_literal_final_expr_no_decls() {
    let m = tlc_of("42");
    assert_eq!(m.decls.len(), 0);
}

#[test]
fn annotated_value_decl_lowers_correctly() {
    let m = tlc_of("x :: Int = 42\nx");
    assert_eq!(m.decls.len(), 1);
    let decl = &m.decl_arena[m.decls[0]];
    let crate::TlcDecl::Value { ty, body, .. } = decl else { panic!("expected Value decl") };
    assert_eq!(m.type_arena[*ty], crate::TlcType::Prim(crate::PrimTy::Int));
    assert_eq!(m.expr_arena[*body], crate::TlcExpr::Lit(crate::Literal::Int(42)));
}

#[test]
fn type_alias_decl_lowers_correctly() {
    let m = tlc_of("Point :: type { x : Int; y : Int; }\nPoint");
    assert_eq!(m.decls.len(), 1);
    assert!(matches!(m.decl_arena[m.decls[0]], crate::TlcDecl::TypeAlias { .. }));
}

#[test]
fn bool_literal_no_crash() {
    let m = tlc_of("true");
    assert_eq!(m.decls.len(), 0);
}
