pub mod structural_key_validation;

use crate::diagnostic::HirDiagnostic;
use crate::ir::HirFile;

pub use structural_key_validation::StructuralKeyValidationPass;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirPassReport {
    pub name: &'static str,
}

pub trait HirPass {
    fn name(&self) -> &'static str;
    fn run(&mut self, file: &mut HirFile, diagnostics: &mut Vec<HirDiagnostic>);
}

pub fn run_passes(
    file: &mut HirFile,
    diagnostics: &mut Vec<HirDiagnostic>,
    passes: &mut [&mut dyn HirPass],
) -> Vec<HirPassReport> {
    passes
        .iter_mut()
        .map(|pass| {
            pass.run(file, diagnostics);
            HirPassReport { name: pass.name() }
        })
        .collect()
}

pub fn run_default_passes(
    file: &mut HirFile,
    diagnostics: &mut Vec<HirDiagnostic>,
) -> Vec<HirPassReport> {
    let mut structural_keys = StructuralKeyValidationPass;
    let mut passes: [&mut dyn HirPass; 1] = [&mut structural_keys];
    run_passes(file, diagnostics, &mut passes)
}
