use crate::*;

fn lower(src: &str) -> LoweredHir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    lower_file(parsed.ast().expect("parse should produce AST"))
}

fn lower_without_passes(src: &str) -> LoweredHir {
    let parsed = zutai_syntax::parse(src);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics());
    lower_file_with_options(
        parsed.ast().expect("parse should produce AST"),
        HirLowerOptions { run_passes: false },
    )
}

fn binding_name(file: &HirFile, id: BindingId) -> &str {
    &file.bindings[id.0 as usize].name
}
fn lower_no_diag(src: &str) -> LoweredHir {
    let lowered = lower(src);
    assert!(
        lowered.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        lowered.diagnostics
    );
    lowered
}

fn find_binding_by_name(file: &HirFile, name: &str) -> Option<BindingId> {
    file.bindings
        .iter()
        .enumerate()
        .find(|(_, b)| b.name == name)
        .map(|(i, _)| BindingId(i as u32))
}
fn contains_type_binding(file: &HirFile, ty: &HirTypeExpr, binding: BindingId) -> bool {
    match &ty.kind {
        HirTypeKind::BindingRef(id) => *id == binding,
        HirTypeKind::Arrow { from, to } => {
            contains_type_binding(file, &file.type_arena[*from], binding)
                || contains_type_binding(file, &file.type_arena[*to], binding)
        }
        HirTypeKind::ForAll { body, .. } => {
            contains_type_binding(file, &file.type_arena[*body], binding)
        }
        _ => false,
    }
}
fn type_kind(file: &HirFile, id: HirTypeId) -> &HirTypeKind {
    &file.type_arena[id].kind
}

mod constraints;
mod lowering;
mod v1_lowering;
