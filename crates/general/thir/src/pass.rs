use crate::diagnostic::ThirDiagnostic;
use crate::ir::ThirFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirPassReport {
    pub name: &'static str,
}

pub trait ThirPass {
    fn name(&self) -> &'static str;
    fn run(&mut self, file: &mut ThirFile, diagnostics: &mut Vec<ThirDiagnostic>);
}

pub fn run_passes(
    file: &mut ThirFile,
    diagnostics: &mut Vec<ThirDiagnostic>,
    passes: &mut [&mut dyn ThirPass],
) -> Vec<ThirPassReport> {
    passes
        .iter_mut()
        .map(|pass| {
            pass.run(file, diagnostics);
            ThirPassReport { name: pass.name() }
        })
        .collect()
}

pub fn run_default_passes(
    file: &mut Option<ThirFile>,
    diagnostics: &mut Vec<ThirDiagnostic>,
) -> Vec<ThirPassReport> {
    let Some(file) = file.as_mut() else {
        return Vec::new();
    };
    let mut passes: [&mut dyn ThirPass; 0] = [];
    run_passes(file, diagnostics, &mut passes)
}
