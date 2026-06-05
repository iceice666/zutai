use std::fs;
use std::path::{Path, PathBuf};

use zutai_hir::{HirFile, SymbolId};
use zutai_syntax::diag::Diagnostic;

use crate::error::{EvalError, EvalErrorKind, EvalResult};
use crate::import::ImportCache;
use crate::value::EvalValue;

/// Tunables for the evaluator and REPL display boundary.
#[derive(Debug, Clone)]
pub struct EvalConfig {
    pub max_force_depth: usize,
    pub max_eval_steps: usize,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            max_force_depth: 128,
            max_eval_steps: 100_000,
        }
    }
}

/// Persistent evaluator session.
///
/// The general-mode REPL should keep one session alive across inputs. Future
/// shell mode can reuse the same session and add its own effect runner above it.
#[derive(Debug, Default)]
pub struct EvalSession {
    config: EvalConfig,
    imports: ImportCache,
    bindings: rustc_hash::FxHashMap<SymbolId, EvalValue>,
}

impl EvalSession {
    pub fn new() -> Self {
        Self::with_config(EvalConfig::default())
    }

    pub fn with_config(config: EvalConfig) -> Self {
        Self {
            config,
            imports: ImportCache::new(),
            bindings: rustc_hash::FxHashMap::default(),
        }
    }

    pub fn config(&self) -> &EvalConfig {
        &self.config
    }

    pub fn imports(&self) -> &ImportCache {
        &self.imports
    }

    pub fn imports_mut(&mut self) -> &mut ImportCache {
        &mut self.imports
    }

    pub fn reset(&mut self) {
        self.imports.clear();
        self.bindings.clear();
    }

    /// Parse, analyze, and eventually evaluate a source string as one `.zt`
    /// file. Currently stops after successful analysis.
    pub fn eval_source(
        &mut self,
        source_name: impl Into<String>,
        source: &str,
    ) -> EvalResult<EvalValue> {
        let unit = self.prepare_source(source_name, source)?;
        self.eval_hir(&unit.hir)
    }

    /// Read and evaluate a `.zt` file.
    pub fn eval_file(&mut self, path: impl AsRef<Path>) -> EvalResult<EvalValue> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|source| {
            EvalError::new(EvalErrorKind::Io {
                path: path.to_path_buf(),
                source,
            })
        })?;
        self.eval_source(path.display().to_string(), &source)
    }

    /// Prepare source for evaluation, returning HIR plus diagnostics metadata.
    pub fn prepare_source(
        &self,
        source_name: impl Into<String>,
        source: &str,
    ) -> EvalResult<PreparedUnit> {
        let parsed = zutai_syntax::parse(source);
        let semantic = zutai_semantic::analyze(&parsed.syntax());
        let mut diagnostics = parsed.diagnostics;
        diagnostics.extend(semantic.diagnostics);

        if !diagnostics.is_empty() {
            return Err(EvalError::new(EvalErrorKind::Diagnostics { diagnostics }));
        }

        Ok(PreparedUnit {
            source_name: source_name.into(),
            hir: semantic.hir,
        })
    }

    /// Evaluate already-analyzed HIR.
    pub fn eval_hir(&mut self, _hir: &HirFile) -> EvalResult<EvalValue> {
        Err(EvalError::not_implemented("general-mode HIR evaluation"))
    }
}

/// Parsed and analyzed source ready for evaluation.
pub struct PreparedUnit {
    pub source_name: String,
    pub hir: HirFile,
}

impl PreparedUnit {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &[]
    }

    pub fn source_path_hint(&self) -> Option<PathBuf> {
        let path = PathBuf::from(&self.source_name);
        if path.as_os_str().is_empty() {
            None
        } else {
            Some(path)
        }
    }
}
