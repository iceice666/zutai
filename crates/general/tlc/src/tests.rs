use crate::*;

fn tlc_of(src: &str) -> TlcModule {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "parse errors: {:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(hir.diagnostics.is_empty(), "hir errors: {:?}", hir.diagnostics);
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
    use std::collections::HashMap;
    use la_arena::Arena;
    let _m = TlcModule {
        decls: Vec::new(),
        decl_arena: Arena::new(),
        expr_arena: Arena::new(),
        type_arena: Arena::new(),
        expr_types: HashMap::new(),
        spans: HashMap::new(),
    };
}
