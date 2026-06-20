use crate::*;

fn lower(src: &str) -> LoweredThir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse should produce AST"));
    assert!(hir.diagnostics.is_empty(), "{:?}", hir.diagnostics);
    lower_hir(&hir.file)
}

fn completed_file(src: &str) -> ThirFile {
    let lowered = lower(src);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    lowered.file.expect("valid THIR should be produced")
}

fn final_type_kind(file: &ThirFile) -> &TypeKind {
    let final_expr = &file.expr_arena[file.final_expr];
    &file.type_arena[final_expr.ty.0 as usize].kind
}
/// Lower `src` with a reduced type-evaluation fuel budget, returned as a
/// `LoweredThir` so callers can inspect diagnostics.
fn lower_with_type_eval_fuel(src: &str, fuel: u32) -> LoweredThir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse should produce AST"));
    assert!(hir.diagnostics.is_empty(), "{:?}", hir.diagnostics);
    lower_hir_with_options(
        &hir.file,
        ThirLowerOptions {
            run_passes: true,
            type_eval_fuel: Some(fuel),
            ..ThirLowerOptions::default()
        },
    )
}
/// Helper: find the first decl in `file.decls` whose kind matches the predicate.
fn find_decl_kind<F>(file: &ThirFile, pred: F) -> Option<&ThirDeclKind>
where
    F: Fn(&ThirDeclKind) -> bool,
{
    file.decls
        .iter()
        .find(|&&id| pred(&file.decl_arena[id].kind))
        .map(|&id| &file.decl_arena[id].kind)
}
/// Helper that allows HIR diagnostics — needed for UnresolvedIdent tests
/// because unknown type names produce HIR diagnostics (name resolution failures).
fn lower_allowing_hir_errors(src: &str) -> LoweredThir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse should produce AST"));
    // Do NOT assert hir.diagnostics.is_empty() — HIR name-resolution errors are expected.
    lower_hir(&hir.file)
}

mod constraints_witnesses;
mod core_lowering;
mod effects_rows;
mod exhaustiveness;
mod export_types;
mod generics_aliases;
mod patterns_and_diagnostics;
mod type_internals;
