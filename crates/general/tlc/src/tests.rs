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
