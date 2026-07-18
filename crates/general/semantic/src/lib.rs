//! Semantic analysis facade for Zutai general mode (`.zt`).
//!
//! This crate wires parser output into HIR, THIR, and TLC while keeping stage
//! results separate for callers that need partial output after a later phase
//! fails. It is not the home for ordinary single-IR passes: syntax/name and
//! structural validation live in `zutai-hir`, type checking and typed
//! elaboration live in `zutai-thir`, and fully elaborated type-lambda
//! lowering lives in `zutai-tlc`.
//!
//! `zutai-semantic` owns analysis that needs filesystem, module-graph, or
//! cross-stage context: path-relative `.zti`/`.zt` import loading, recursive
//! module analysis, import caching and cycle diagnostics, imported
//! value/module maps for the evaluator, witness export merging, and backend /
//! evaluator gate predicates over completed staged output.

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub use stdlib::{
    COMPILER_COMPATIBILITY as STDLIB_COMPILER_COMPATIBILITY, STDLIB_ROOT_ENV, StdlibError,
    StdlibSources,
};

pub fn configure_stdlib_root(root: impl Into<std::path::PathBuf>) -> Result<(), StdlibError> {
    stdlib::set_process_root(root.into())
}

mod cache;
mod import;
mod package;
mod reflect_detect;
mod stdlib;
#[cfg(test)]
mod tests;

use reflect_detect::*;

pub use cache::{AnalysisCache, AnalysisCacheStats};

pub use import::{ConditionalWitnessShape, ImportDiagnostic, ImportDiagnosticKind, WitnessExport};
pub use package::{PortablePackage, PortablePackageGraph};
pub use zutai_package::PortablePackageSource;

pub const BACKEND_IMPORT_WITNESS_CODE: &str = "zutai::backend::import_witness_non_matchable";
pub const BACKEND_ENTRY_TYPE_CODE: &str = "zutai::backend::entry_type_unsupported";
pub const BACKEND_REFLECTION_EFFECT_CODE: &str = "zutai::backend::reflection_effectful";
pub const BACKEND_REFLECTION_FOLD_CODE: &str = "zutai::backend::reflection_not_foldable";
pub const BACKEND_RESIDUAL_EFFECT_CODE: &str = "zutai::backend::residual_effect";
pub const BACKEND_DATAFLOW_CODE: &str = "zutai::backend::dataflow_lowering";
pub const BACKEND_CONFIG_OVERLAY_CODE: &str = "zutai::backend::config_overlay";

pub const IMPORT_WITNESS_REASON: &str = "native backend does not support importing higher-kinded or otherwise non-matchable typeclass instances yet. Use `zutai run` (interpreter)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub path: PathBuf,
    pub span: zutai_syntax::Span,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticMetadata {
    pub code: &'static str,
    pub severity: zutai_syntax::Severity,
    pub primary_span: zutai_syntax::Span,
    pub related: Option<(zutai_syntax::Span, &'static str)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendDiagnostic {
    pub code: &'static str,
    pub severity: zutai_syntax::Severity,
    pub message: String,
    pub span: zutai_syntax::Span,
    pub related: Vec<SourceLocation>,
}

#[derive(Debug)]
pub struct Analysis {
    pub ast: Option<zutai_syntax::File>,
    pub hir: Option<zutai_hir::LoweredHir>,
    pub thir: Option<zutai_thir::LoweredThir>,
    pub diagnostics: Vec<SemanticDiagnostic>,
    /// Canonical or synthetic identity of this analyzed source. Filesystem analyses
    /// use canonical paths; portable package and stdlib analyses use stable
    /// `<package>/...` and `<stdlib>/...` identities.
    pub source_path: Option<PathBuf>,
    pub pass_reports: Vec<SemanticPassReport>,
    /// Parsed `.zti` import values, keyed by import source, for the evaluator.
    pub import_values: FxHashMap<zutai_thir::ImportKey, zutai_im::Value>,
    /// Analyzed `.zt` sub-modules, keyed by import source, for the evaluator to
    /// evaluate recursively.
    pub import_modules: FxHashMap<zutai_thir::ImportKey, Rc<Analysis>>,
    /// Source spans for each import expression in this module.
    pub import_sites: FxHashMap<zutai_thir::ImportKey, zutai_syntax::Span>,
    /// TLC module produced by lowering THIR; `None` when THIR is incomplete.
    pub tlc: Option<zutai_tlc::TlcModule>,
    pub witness_exports: Vec<WitnessExport>,
}

/// An analysis plus the normalized transitive source graph read from disk.
///
/// Paths use `/` separators and are relative to the entry file's directory,
/// making the result suitable for deterministic browser/compiler bundles.
#[derive(Debug)]
pub struct RecordedAnalysis {
    pub entry: String,
    /// Maps stable analysis identities to filesystem source paths for editor
    /// navigation after replaying a recorded package graph.
    pub source_paths: BTreeMap<PathBuf, PathBuf>,
    pub sources: BTreeMap<String, String>,
    pub stdlib_compiler_compatibility: String,
    pub stdlib_sources: BTreeMap<String, String>,
    pub packages: PortablePackageGraph,
    pub analysis: Analysis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceMapError {
    InvalidPath { path: String, reason: &'static str },
    MissingEntry { path: String },
    Stdlib { message: String },
}

impl fmt::Display for SourceMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceMapError::InvalidPath { path, reason } => {
                write!(f, "invalid source path `{path}`: {reason}")
            }
            SourceMapError::MissingEntry { path } => {
                write!(f, "entry source `{path}` is not present in the source map")
            }
            SourceMapError::Stdlib { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for SourceMapError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticPassReport {
    pub stage: SemanticStage,
    pub name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDiagnostic {
    pub stage: SemanticStage,
    pub kind: SemanticDiagnosticKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticStage {
    Parse,
    Hir,
    Import,
    Thir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticDiagnosticKind {
    Parse(zutai_syntax::Diagnostic),
    Hir(zutai_hir::HirDiagnostic),
    Import(ImportDiagnostic),
    Thir(zutai_thir::ThirDiagnostic),
}

impl SemanticDiagnostic {
    pub fn metadata(&self) -> DiagnosticMetadata {
        match &self.kind {
            SemanticDiagnosticKind::Parse(parse) => DiagnosticMetadata {
                code: parse.code,
                severity: parse.severity,
                primary_span: parse.primary_span(),
                related: None,
            },
            SemanticDiagnosticKind::Hir(hir) => DiagnosticMetadata {
                code: hir.code(),
                severity: zutai_syntax::Severity::Error,
                primary_span: hir.span,
                related: hir.related_location(),
            },
            SemanticDiagnosticKind::Import(import) => DiagnosticMetadata {
                code: import.code(),
                severity: zutai_syntax::Severity::Error,
                primary_span: import.span,
                related: None,
            },
            SemanticDiagnosticKind::Thir(thir) => DiagnosticMetadata {
                code: thir.code(),
                severity: zutai_syntax::Severity::Error,
                primary_span: thir.span,
                related: None,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisOptions {
    pub run_hir_passes: bool,
    pub run_thir_passes: bool,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            run_hir_passes: true,
            run_thir_passes: true,
        }
    }
}

impl Analysis {
    pub fn has_parse_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.stage == SemanticStage::Parse)
    }

    pub fn has_hir_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.stage == SemanticStage::Hir)
    }

    pub fn is_thir_complete(&self) -> bool {
        self.thir
            .as_ref()
            .is_some_and(|lowered| lowered.file.is_some())
    }

    pub fn blocking_diagnostics(&self) -> impl Iterator<Item = &SemanticDiagnostic> {
        self.diagnostics.iter().filter(|diagnostic| {
            matches!(
                diagnostic.stage,
                SemanticStage::Parse | SemanticStage::Hir | SemanticStage::Import
            )
        })
    }

    /// Backend-only import refusals. These do not make semantic analysis or the
    /// reference interpreter fail; compile/dataflow render them as errors and
    /// editor diagnostics expose them as native-support warnings.
    pub fn native_import_diagnostics(&self) -> Vec<BackendDiagnostic> {
        let mut imports: Vec<_> = self.import_modules.iter().collect();
        imports.sort_by_key(|(source, _)| format!("{source:?}"));
        imports
            .into_iter()
            .filter_map(|(source, module)| {
                let span = self.import_sites.get(source).copied()?;
                let related = module.non_matchable_witness_chain()?;
                Some(BackendDiagnostic {
                    code: BACKEND_IMPORT_WITNESS_CODE,
                    severity: zutai_syntax::Severity::Warning,
                    message: IMPORT_WITNESS_REASON.to_owned(),
                    span,
                    related,
                })
            })
            .collect()
    }

    pub fn backend_diagnostics(&self) -> Vec<BackendDiagnostic> {
        let span = self
            .thir
            .as_ref()
            .and_then(|lowered| lowered.file.as_ref())
            .map(|file| file.expr_arena[file.final_expr].span)
            .unwrap_or_default();
        let mut diagnostics = self.native_import_diagnostics();
        if let Some(message) = self.effectful_program() {
            diagnostics.push(BackendDiagnostic {
                code: BACKEND_RESIDUAL_EFFECT_CODE,
                severity: zutai_syntax::Severity::Warning,
                message: message.to_owned(),
                span,
                related: Vec::new(),
            });
        }
        if let Some(message) = self.aot_reflection_program() {
            diagnostics.push(BackendDiagnostic {
                code: BACKEND_REFLECTION_FOLD_CODE,
                severity: zutai_syntax::Severity::Warning,
                message: message.to_owned(),
                span,
                related: Vec::new(),
            });
        }
        if let Some(message) = self.config_overlay_builtin_program() {
            diagnostics.push(BackendDiagnostic {
                code: BACKEND_CONFIG_OVERLAY_CODE,
                severity: zutai_syntax::Severity::Warning,
                message: message.to_owned(),
                span,
                related: Vec::new(),
            });
        }
        diagnostics
    }

    fn non_matchable_witness_chain(&self) -> Option<Vec<SourceLocation>> {
        if let Some(witness) = self.witness_exports.iter().find(|witness| {
            witness.conditional.is_none()
                && witness.target_key.contains('?')
                && self
                    .source_path
                    .as_ref()
                    .is_some_and(|path| path == &witness.origin)
        }) {
            return Some(vec![SourceLocation {
                path: witness.origin.clone(),
                span: witness.span,
                label: "non-matchable witness exported here".to_owned(),
            }]);
        }

        let mut imports: Vec<_> = self.import_modules.iter().collect();
        imports.sort_by_key(|(source, _)| format!("{source:?}"));
        for (source, module) in imports {
            let Some(mut related) = module.non_matchable_witness_chain() else {
                continue;
            };
            if let (Some(path), Some(span)) =
                (self.source_path.as_ref(), self.import_sites.get(source))
            {
                related.insert(
                    0,
                    SourceLocation {
                        path: path.clone(),
                        span: *span,
                        label: "import chain continues here".to_owned(),
                    },
                );
            }
            return Some(related);
        }

        self.witness_exports
            .iter()
            .find(|witness| witness.conditional.is_none() && witness.target_key.contains('?'))
            .map(|witness| {
                vec![SourceLocation {
                    path: witness.origin.clone(),
                    span: witness.span,
                    label: "non-matchable witness exported here".to_owned(),
                }]
            })
    }

    pub fn effectful_program(&self) -> Option<&'static str> {
        let file = self.thir.as_ref()?.file.as_ref()?;
        let has_effect_expr = file.expr_arena.iter().any(|(_, expr)| {
            matches!(
                expr.kind,
                zutai_thir::ThirExprKind::Perform { .. }
                    | zutai_thir::ThirExprKind::Handle { .. }
                    | zutai_thir::ThirExprKind::Resume { .. }
            )
        });
        fn type_has_effect(file: &zutai_thir::ThirFile, id: zutai_thir::TypeId) -> bool {
            match &file.type_arena[id.0 as usize].kind {
                zutai_thir::TypeKind::Effect { row, .. } => !row.is_pure(),
                zutai_thir::TypeKind::Function { from, to } => {
                    type_has_effect(file, *from) || type_has_effect(file, *to)
                }
                _ => false,
            }
        }

        let has_effect_type = file
            .expr_arena
            .iter()
            .any(|(_, expr)| type_has_effect(file, expr.ty));
        if has_effect_expr || has_effect_type {
            Some(
                "algebraic effects require the TLC effect evaluator or a backend residual-effect gate",
            )
        } else {
            None
        }
    }

    /// Reflection that the TLC-first evaluator cannot execute and that must be
    /// served by the THIR oracle instead: `fields`/`schema` build runtime
    /// `Type`-reflection values TLC has no representation for. `variants` and
    /// `witness` reflection are intentionally absent — the TLC evaluator
    /// evaluates both, so they stay on the default TLC path (see
    /// `aot_reflection_program` for the broader compile-time fold gate).
    pub fn reflection_builtin_program(&self) -> Option<&'static str> {
        if self.import_modules.iter().any(|(source, module)| {
            !is_stdlib_module(source, "reflect")
                && module.as_ref().reflection_builtin_program().is_some()
        }) {
            return Some(
                "reflection builtins are compile-time evaluator intrinsics and do not lower to pure backend IR yet",
            );
        }
        if self.uses_reflection_builtin(&["fields", "schema"], false)
            || self.uses_stdlib_reflect_call(&["fields", "schema"])
        {
            Some(
                "reflection builtins are compile-time evaluator intrinsics and do not lower to pure backend IR yet",
            )
        } else {
            None
        }
    }

    /// Compile-time reflection that must be AOT-folded to a backend value or
    /// rejected before Dataflow Core — the superset of [`Self::reflection_builtin_program`]
    /// that also covers `variants` and the `witness C @T` reflection expression.
    /// `fold_aot_reflection` evaluates the program through the default evaluator
    /// (TLC for `variants`/`witness`, THIR for `fields`/`schema`) and serializes
    /// the result; a non-serializable result (a raw witness dictionary, a
    /// function, a `Type`) is rejected rather than lowered. Used by the CLI
    /// `compile`/`dataflow` paths, never by the run-time evaluator routing, so
    /// `witness`/`variants` programs keep evaluating on the TLC path.
    pub fn aot_reflection_program(&self) -> Option<&'static str> {
        if self.import_modules.iter().any(|(source, module)| {
            !is_stdlib_module(source, "reflect")
                && module.as_ref().aot_reflection_program().is_some()
        }) {
            return Some(
                "reflection builtins are compile-time evaluator intrinsics and do not lower to pure backend IR yet",
            );
        }
        if self.uses_reflection_builtin(&["fields", "variants", "schema"], true)
            || self.uses_stdlib_reflect_call(&["fields", "variants", "schema"])
        {
            Some(
                "reflection builtins are compile-time evaluator intrinsics and do not lower to pure backend IR yet",
            )
        } else {
            None
        }
    }

    pub fn config_overlay_builtin_program(&self) -> Option<&'static str> {
        if self.import_modules.iter().any(|(source, module)| {
            !is_stdlib_module(source, "config")
                && module.as_ref().config_overlay_builtin_program().is_some()
        }) {
            return Some(
                "config overlay builtins could not be lowered to pure backend IR before Dataflow Core",
            );
        }
        let hir = &self.hir.as_ref()?.file;
        let module = self.tlc.as_ref()?;
        let binding_uses_overlay = |binding: zutai_hir::BindingId| {
            hir.bindings
                .get(binding.0 as usize)
                .is_some_and(|hir_binding| {
                    hir_binding.kind == zutai_hir::BindingKind::BuiltinValue
                        && (hir_binding.name == "overlay" || hir_binding.name == "overlayDeep")
                })
        };
        let expr_uses_overlay = |root| {
            let mut seen = rustc_hash::FxHashSet::default();
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                if matches!(module.expr_arena[id], zutai_tlc::TlcExpr::Var(binding) if binding_uses_overlay(binding))
                {
                    return true;
                }
                zutai_tlc::push_child_exprs(&module.expr_arena[id], &mut stack);
            }
            false
        };
        let uses_overlay = module.final_expr.is_some_and(expr_uses_overlay);
        if uses_overlay {
            Some(
                "config overlay builtins could not be lowered to pure backend IR before Dataflow Core",
            )
        } else {
            None
        }
    }

    fn uses_reflection_builtin(&self, fields: &[&str], include_witness: bool) -> bool {
        let Some(hir) = self.hir.as_ref().map(|lowered| &lowered.file) else {
            return false;
        };
        let Some(file) = self.thir.as_ref().and_then(|thir| thir.file.as_ref()) else {
            return false;
        };
        file.expr_arena.iter().any(|(_, expr)| match &expr.kind {
            zutai_thir::ThirExprKind::WitnessReflect { .. } => include_witness,
            zutai_thir::ThirExprKind::BindingRef { binding, .. } => hir
                .bindings
                .get(binding.0 as usize)
                .is_some_and(|hir_binding| {
                    hir_binding.kind == zutai_hir::BindingKind::BuiltinValue
                        && fields.contains(&hir_binding.name.as_str())
                }),
            _ => false,
        })
    }

    fn uses_stdlib_reflect_call(&self, fields: &[&str]) -> bool {
        let Some(hir) = self.hir.as_ref().map(|lowered| &lowered.file) else {
            return false;
        };
        let Some(file) = self.thir.as_ref().and_then(|thir| thir.file.as_ref()) else {
            return false;
        };
        file.expr_arena.iter().any(|(_, expr)| {
            let zutai_thir::ThirExprKind::Apply { func, .. } = expr.kind else {
                return false;
            };
            thir_expr_is_stdlib_reflect_alias(hir, file, func, fields, &mut FxHashSet::default())
        })
    }
}

pub fn analyze(input: &str) -> Analysis {
    analyze_with_options(input, AnalysisOptions::default())
}

pub fn analyze_with_options(input: &str, options: AnalysisOptions) -> Analysis {
    match StdlibSources::load_configured(None) {
        Ok(stdlib) => analyze_with_stdlib(input, options, &stdlib),
        Err(error) => stdlib_error_analysis(error),
    }
}

pub fn analyze_with_stdlib(
    input: &str,
    options: AnalysisOptions,
    stdlib: &StdlibSources,
) -> Analysis {
    analyze_with_base_and_stdlib(input, None, options, stdlib)
}

/// Analyze a `.zt` file on disk, resolving imports relative to its directory.
pub fn analyze_path(path: &Path) -> std::io::Result<Analysis> {
    let stdlib = configured_stdlib_io(None)?;
    analyze_path_with_stdlib(path, &stdlib)
}

pub fn analyze_path_with_stdlib(path: &Path, stdlib: &StdlibSources) -> std::io::Result<Analysis> {
    analyze_path_with_stdlib_and_cache(path, stdlib, None)
}

pub fn analyze_path_with_cache(path: &Path, cache: &AnalysisCache) -> std::io::Result<Analysis> {
    let stdlib = configured_stdlib_io(None)?;
    analyze_path_with_stdlib_and_cache(path, &stdlib, Some(cache))
}

fn analyze_path_with_stdlib_and_cache(
    path: &Path,
    stdlib: &StdlibSources,
    cache: Option<&AnalysisCache>,
) -> std::io::Result<Analysis> {
    let options = AnalysisOptions::default();
    let input = std::fs::read_to_string(path)?;
    let mut ctx = import::ImportContext::with_root(path, Rc::new(stdlib.clone()), cache, options);
    let current = std::fs::canonicalize(path).ok();
    Ok(analyze_inner(
        &input,
        path.parent(),
        current.as_deref(),
        options,
        &mut ctx,
    ))
}

/// Analyze a `.zt` file while recording every transitive `.zt`/`.zti` source
/// read through relative imports.
///
/// The returned entry and map keys are normalized, relative bundle paths. The
/// filesystem resolver otherwise has exactly the same canonicalization,
/// symlink-confinement, caching, and cycle behavior as [`analyze_path`].
pub fn analyze_path_recording(path: &Path) -> std::io::Result<RecordedAnalysis> {
    let stdlib = configured_stdlib_io(None)?;
    analyze_path_recording_with_stdlib(path, &stdlib)
}

pub fn analyze_path_recording_with_stdlib(
    path: &Path,
    stdlib: &StdlibSources,
) -> std::io::Result<RecordedAnalysis> {
    analyze_path_recording_with_stdlib_and_cache(path, stdlib, None)
}

pub fn analyze_path_recording_with_cache(
    path: &Path,
    cache: &AnalysisCache,
) -> std::io::Result<RecordedAnalysis> {
    let stdlib = configured_stdlib_io(None)?;
    analyze_path_recording_with_stdlib_and_cache(path, &stdlib, Some(cache))
}

fn analyze_path_recording_with_stdlib_and_cache(
    path: &Path,
    stdlib: &StdlibSources,
    cache: Option<&AnalysisCache>,
) -> std::io::Result<RecordedAnalysis> {
    let options = AnalysisOptions::default();
    let input = std::fs::read_to_string(path)?;
    let entry = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "entry path must have a UTF-8 file name",
            )
        })?
        .to_string();
    let mut ctx =
        import::ImportContext::with_recording_root(path, Rc::new(stdlib.clone()), cache, options);
    ctx.record_root_source(&entry, &input);
    let current = std::fs::canonicalize(path).ok();
    let analysis = analyze_inner(&input, path.parent(), current.as_deref(), options, &mut ctx);
    Ok(recorded_analysis(entry, analysis, &mut ctx))
}

/// Analyze and record a source graph relative to an explicit source root.
///
/// Unlike [`analyze_path_recording`], relative imports may traverse to sibling
/// directories as long as their canonical targets remain inside `source_root`.
/// This is the filesystem counterpart of [`analyze_sources`] and is intended
/// for building portable browser/compiler bundles.
pub fn analyze_path_recording_with_root(
    path: &Path,
    source_root: &Path,
) -> std::io::Result<RecordedAnalysis> {
    let stdlib = configured_stdlib_io(None)?;
    analyze_path_recording_with_root_and_stdlib(path, source_root, &stdlib)
}

pub fn analyze_path_recording_with_root_and_cache(
    path: &Path,
    source_root: &Path,
    cache: &AnalysisCache,
) -> std::io::Result<RecordedAnalysis> {
    let stdlib = configured_stdlib_io(None)?;
    analyze_path_recording_with_root_and_stdlib_and_cache(path, source_root, &stdlib, Some(cache))
}

pub fn analyze_path_recording_with_root_and_stdlib(
    path: &Path,
    source_root: &Path,
    stdlib: &StdlibSources,
) -> std::io::Result<RecordedAnalysis> {
    analyze_path_recording_with_root_and_stdlib_and_cache(path, source_root, stdlib, None)
}

fn analyze_path_recording_with_root_and_stdlib_and_cache(
    path: &Path,
    source_root: &Path,
    stdlib: &StdlibSources,
    cache: Option<&AnalysisCache>,
) -> std::io::Result<RecordedAnalysis> {
    let options = AnalysisOptions::default();
    let input = std::fs::read_to_string(path)?;
    let canonical = std::fs::canonicalize(path)?;
    let canonical_root = std::fs::canonicalize(if source_root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        source_root
    })?;
    let entry_path = canonical.strip_prefix(&canonical_root).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "entry path must be inside the source root",
        )
    })?;
    let entry = import::path_to_bundle_key(entry_path);
    validate_source_path(&entry).map_err(|error| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, error.to_string())
    })?;

    let mut ctx = import::ImportContext::with_explicit_recording_root(
        path,
        source_root,
        Rc::new(stdlib.clone()),
        cache,
        options,
    )?;
    ctx.record_root_source(&entry, &input);
    let analysis = analyze_inner(&input, path.parent(), Some(&canonical), options, &mut ctx);
    Ok(recorded_analysis(entry, analysis, &mut ctx))
}

/// Analyze a complete in-memory source graph.
///
/// `entry` and all source keys must be normalized, relative, `/`-separated
/// paths. Absolute paths, backslashes, NULs, empty segments, `.` and `..` are
/// rejected before parsing. Relative imports may use `..` to reach siblings in
/// the source graph, but cannot escape its virtual root. Standard-library
/// modules come from the configured filesystem root.
pub fn analyze_sources(
    entry: &str,
    sources: &BTreeMap<String, String>,
    options: AnalysisOptions,
) -> Result<Analysis, SourceMapError> {
    let stdlib = StdlibSources::load_configured(None).map_err(|error| SourceMapError::Stdlib {
        message: error.to_string(),
    })?;
    analyze_sources_with_stdlib(entry, sources, options, &stdlib)
}

pub fn analyze_sources_with_stdlib(
    entry: &str,
    sources: &BTreeMap<String, String>,
    options: AnalysisOptions,
    stdlib: &StdlibSources,
) -> Result<Analysis, SourceMapError> {
    analyze_sources_with_stdlib_and_packages(
        entry,
        sources,
        options,
        stdlib,
        PortablePackageGraph::default(),
    )
}

pub fn analyze_sources_with_stdlib_and_packages(
    entry: &str,
    sources: &BTreeMap<String, String>,
    options: AnalysisOptions,
    stdlib: &StdlibSources,
    packages: PortablePackageGraph,
) -> Result<Analysis, SourceMapError> {
    analyze_sources_with_stdlib_packages_and_cache(entry, sources, options, stdlib, packages, None)
}

pub fn analyze_sources_with_stdlib_packages_and_cache(
    entry: &str,
    sources: &BTreeMap<String, String>,
    options: AnalysisOptions,
    stdlib: &StdlibSources,
    packages: PortablePackageGraph,
    cache: Option<&AnalysisCache>,
) -> Result<Analysis, SourceMapError> {
    validate_source_path(entry)?;
    for path in sources.keys() {
        validate_source_path(path)?;
    }
    let input = sources
        .get(entry)
        .ok_or_else(|| SourceMapError::MissingEntry {
            path: entry.to_string(),
        })?;
    let entry_path = Path::new(entry);
    let base = entry_path.parent().unwrap_or_else(|| Path::new(""));
    let mut ctx = import::ImportContext::with_memory(
        sources,
        entry_path,
        Rc::new(stdlib.clone()),
        packages,
        cache,
        options,
    );
    Ok(analyze_inner(
        input,
        Some(base),
        Some(entry_path),
        options,
        &mut ctx,
    ))
}

fn validate_source_path(path: &str) -> Result<(), SourceMapError> {
    let invalid = |reason| SourceMapError::InvalidPath {
        path: path.to_string(),
        reason,
    };
    if path.is_empty() {
        return Err(invalid("path is empty"));
    }
    if path.contains('\0') {
        return Err(invalid("path contains NUL"));
    }
    if path.contains('\\') {
        return Err(invalid("use `/` separators"));
    }
    if path.starts_with('/')
        || Path::new(path).is_absolute()
        || import::has_windows_drive_prefix(path)
    {
        return Err(invalid("absolute paths are not allowed"));
    }
    for component in path.split('/') {
        match component {
            "" => return Err(invalid("empty path segments are not normalized")),
            "." => return Err(invalid("`.` path segments are not normalized")),
            ".." => return Err(invalid("`..` path segments are not allowed")),
            _ => {}
        }
    }
    Ok(())
}

/// Analyze `input`, resolving import declarations relative to `base`.
///
/// `base` is the directory of the importing file; `None` (string-only entry
/// points, REPL) means imports cannot be resolved and yield a diagnostic.
pub fn analyze_with_base(input: &str, base: Option<&Path>, options: AnalysisOptions) -> Analysis {
    match StdlibSources::load_configured(None) {
        Ok(stdlib) => analyze_with_base_and_stdlib(input, base, options, &stdlib),
        Err(error) => stdlib_error_analysis(error),
    }
}

pub fn analyze_with_base_and_stdlib(
    input: &str,
    base: Option<&Path>,
    options: AnalysisOptions,
    stdlib: &StdlibSources,
) -> Analysis {
    analyze_with_base_stdlib_and_cache(input, base, options, stdlib, None)
}

pub fn analyze_with_base_and_cache(
    input: &str,
    base: Option<&Path>,
    options: AnalysisOptions,
    cache: &AnalysisCache,
) -> Analysis {
    match StdlibSources::load_configured(None) {
        Ok(stdlib) => {
            analyze_with_base_stdlib_and_cache(input, base, options, &stdlib, Some(cache))
        }
        Err(error) => stdlib_error_analysis(error),
    }
}

fn analyze_with_base_stdlib_and_cache(
    input: &str,
    base: Option<&Path>,
    options: AnalysisOptions,
    stdlib: &StdlibSources,
    cache: Option<&AnalysisCache>,
) -> Analysis {
    let mut ctx = import::ImportContext::with_base(base, Rc::new(stdlib.clone()), cache, options);
    analyze_inner(input, base, None, options, &mut ctx)
}

fn configured_stdlib_io(explicit: Option<&Path>) -> std::io::Result<StdlibSources> {
    StdlibSources::load_configured(explicit).map_err(std::io::Error::other)
}

fn stdlib_error_analysis(error: StdlibError) -> Analysis {
    Analysis {
        ast: None,
        hir: None,
        thir: None,
        source_path: None,
        diagnostics: vec![SemanticDiagnostic {
            stage: SemanticStage::Import,
            kind: SemanticDiagnosticKind::Import(ImportDiagnostic {
                kind: ImportDiagnosticKind::StdlibSetup {
                    message: error.to_string(),
                },
                span: zutai_syntax::Span::default(),
                path: None,
                related: Vec::new(),
            }),
        }],
        pass_reports: Vec::new(),
        import_values: FxHashMap::default(),
        import_modules: FxHashMap::default(),
        import_sites: FxHashMap::default(),
        tlc: None,
        witness_exports: Vec::new(),
    }
}

fn recorded_analysis(
    entry: String,
    analysis: Analysis,
    ctx: &mut import::ImportContext<'_>,
) -> RecordedAnalysis {
    let compatibility = ctx.stdlib().compiler_compatibility().to_owned();
    let ambient: Vec<_> = ["stream", "prelude"]
        .into_iter()
        .filter_map(|name| {
            ctx.stdlib()
                .source(name)
                .map(|source| (name.to_owned(), source.to_owned()))
        })
        .collect();
    let mut stdlib_sources = ctx.take_recorded_stdlib();
    stdlib_sources.extend(ambient);
    RecordedAnalysis {
        entry,
        sources: ctx.take_recorded_sources(),
        stdlib_compiler_compatibility: compatibility,
        stdlib_sources,
        packages: ctx.take_recorded_packages(),
        source_paths: ctx.take_recorded_source_paths(),
        analysis,
    }
}

/// Analyze `input`, threading the recursive-import `ctx` (cycle stack + module
/// cache) so that `.zt` module imports can be resolved depth-first.
pub(crate) fn analyze_inner(
    input: &str,
    base: Option<&Path>,
    current: Option<&Path>,
    options: AnalysisOptions,
    ctx: &mut import::ImportContext<'_>,
) -> Analysis {
    let parsed = zutai_syntax::parse_ast_only(input);
    let parse_diagnostics: Vec<_> = parsed
        .diagnostics()
        .iter()
        .cloned()
        .map(|diagnostic| SemanticDiagnostic {
            stage: SemanticStage::Parse,
            kind: SemanticDiagnosticKind::Parse(diagnostic),
        })
        .collect();

    if parsed.has_errors() {
        return Analysis {
            ast: parsed.into_ast(),
            hir: None,
            thir: None,
            source_path: current.map(Path::to_path_buf),
            diagnostics: parse_diagnostics,
            pass_reports: Vec::new(),
            import_values: FxHashMap::default(),
            import_modules: FxHashMap::default(),
            import_sites: FxHashMap::default(),
            tlc: None,
            witness_exports: Vec::new(),
        };
    }

    let Some(ast) = parsed.into_ast() else {
        return Analysis {
            ast: None,
            hir: None,
            thir: None,
            source_path: current.map(Path::to_path_buf),
            diagnostics: parse_diagnostics,
            pass_reports: Vec::new(),
            import_values: FxHashMap::default(),
            import_modules: FxHashMap::default(),
            import_sites: FxHashMap::default(),
            tlc: None,
            witness_exports: Vec::new(),
        };
    };

    let hir = zutai_hir::lower_file_with_preludes(
        &ast,
        zutai_hir::HirLowerOptions {
            run_passes: options.run_hir_passes,
        },
        zutai_hir::SourcePreludes {
            stream: ctx.stdlib().source("stream"),
            prelude: ctx.stdlib().source("prelude"),
        },
    );
    let mut diagnostics = parse_diagnostics;
    diagnostics.extend(
        hir.diagnostics
            .iter()
            .cloned()
            .map(|diagnostic| SemanticDiagnostic {
                stage: SemanticStage::Hir,
                kind: SemanticDiagnosticKind::Hir(diagnostic),
            }),
    );

    let mut pass_reports: Vec<_> = hir
        .pass_reports
        .iter()
        .map(|report| SemanticPassReport {
            stage: SemanticStage::Hir,
            name: report.name,
        })
        .collect();

    let mut import_values = FxHashMap::default();
    let mut import_modules = FxHashMap::default();
    let mut import_sites = FxHashMap::default();
    let mut imported_witnesses = Vec::new();
    let thir =
        if hir.diagnostics.is_empty() {
            // Resolve imports before THIR lowering: the resolved types feed type
            // checking, the values/modules are kept for the evaluator, and any
            // failure is a diagnostic (the import then has no type, becoming a
            // THIR Error node).
            let resolved = import::resolve_imports(&hir.file, base, ctx);
            diagnostics.extend(resolved.diagnostics.into_iter().map(|diagnostic| {
                SemanticDiagnostic {
                    stage: SemanticStage::Import,
                    kind: SemanticDiagnosticKind::Import(diagnostic),
                }
            }));
            import_values = resolved.values;
            import_modules = resolved.modules;
            import_sites = resolved.sites;
            imported_witnesses = resolved.witnesses;

            let lowered = zutai_thir::lower_hir_with_options(
                &hir.file,
                zutai_thir::ThirLowerOptions {
                    run_passes: options.run_thir_passes,
                    imports: resolved.types,
                    import_provenance: resolved.provenance,
                    type_eval_fuel: None,
                },
            );
            pass_reports.extend(
                lowered
                    .pass_reports
                    .iter()
                    .map(|report| SemanticPassReport {
                        stage: SemanticStage::Thir,
                        name: report.name,
                    }),
            );
            diagnostics.extend(lowered.diagnostics.iter().cloned().map(|diagnostic| {
                SemanticDiagnostic {
                    stage: SemanticStage::Thir,
                    kind: SemanticDiagnosticKind::Thir(diagnostic),
                }
            }));
            Some(lowered)
        } else {
            None
        };

    let local_witnesses = thir
        .as_ref()
        .and_then(|t| t.file.as_ref())
        .map(|file| {
            let origin = current.unwrap_or_else(|| Path::new("<input>"));
            import::local_witness_exports(&hir.file, file, origin)
        })
        .unwrap_or_default();
    let (witness_exports, witness_diagnostics) =
        import::merge_witness_exports(imported_witnesses, local_witnesses);
    diagnostics.extend(
        witness_diagnostics
            .into_iter()
            .map(|diagnostic| SemanticDiagnostic {
                stage: SemanticStage::Import,
                kind: SemanticDiagnosticKind::Import(diagnostic),
            }),
    );

    let mut tlc = thir
        .as_ref()
        .and_then(|t| t.file.as_ref())
        .map(zutai_tlc::lower_thir);

    // S1 witness-existence gate. TLC lowering records every bare
    // constraint-method dispatch whose operand dict fell back to `Lit(Nothing)`;
    // it cannot see imports, so a genuinely-witnessed call can appear here too.
    // Resolve each against the merged (local + imported + derived) witness
    // registry — exactly as the interpreter does at runtime — and raise
    // `WitnessReflectNotInScope` for the ones no witness covers, so `check`,
    // `compile`, and the eval gate refuse the call instead of crashing on the
    // unbound synthetic dictionary.
    if let Some(module) = tlc.as_mut() {
        let mut gate_diagnostics = Vec::new();
        for dispatch in &module.unresolved_dispatches {
            if !import::witness_registry_covers(
                &witness_exports,
                &dispatch.constraint,
                &dispatch.target_key,
                0,
            ) {
                gate_diagnostics.push(zutai_thir::ThirDiagnostic {
                    kind: zutai_thir::ThirDiagnosticKind::WitnessReflectNotInScope {
                        constraint: dispatch.constraint.clone(),
                        target: dispatch.target_display.clone(),
                    },
                    span: dispatch.span,
                });
            }
        }
        module.diagnostics.extend(gate_diagnostics);
    }

    // Recipe-reduction diagnostics (e.g. fuel exhaustion) are produced during
    // TLC lowering but belong to the source program: surface them at the `Thir`
    // stage so CLI stage-filters and the LSP render them like any type error.
    // The S1 gate diagnostics pushed above ride the same channel.
    if let Some(module) = tlc.as_ref() {
        diagnostics.extend(module.diagnostics.iter().cloned().map(|diagnostic| {
            SemanticDiagnostic {
                stage: SemanticStage::Thir,
                kind: SemanticDiagnosticKind::Thir(diagnostic),
            }
        }));
    }

    Analysis {
        ast: Some(ast),
        hir: Some(hir),
        thir,
        source_path: current.map(Path::to_path_buf),
        diagnostics,
        pass_reports,
        import_values,
        import_modules,
        import_sites,
        tlc,
        witness_exports,
    }
}
