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
mod stdlib;

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

fn is_stdlib_module(source: &zutai_thir::ImportKey, module: &str) -> bool {
    matches!(source, zutai_hir::HirImportSource::Path(parts)
        if matches!(parts.as_slice(), [root, name] if root == "stdlib" && name == module))
}

fn stdlib_module_field<'a>(
    hir: &zutai_hir::HirFile,
    expr: zutai_hir::HirExprId,
    module: &str,
    fields: &'a [&'a str],
    seen: &mut rustc_hash::FxHashSet<zutai_hir::BindingId>,
) -> Option<&'a str> {
    match &hir.expr_arena[expr].kind {
        zutai_hir::HirExprKind::Access { receiver, field } if fields.contains(&field.as_str()) => {
            expr_is_stdlib_import(hir, *receiver, module, seen)
                .then(|| {
                    fields
                        .iter()
                        .copied()
                        .find(|candidate| *candidate == field.as_str())
                })
                .flatten()
        }
        zutai_hir::HirExprKind::BindingRef(binding) => {
            if !seen.insert(*binding) {
                return None;
            }
            value_decl_expr(hir, *binding)
                .and_then(|value| stdlib_module_field(hir, value, module, fields, seen))
        }
        _ => None,
    }
}

fn expr_is_stdlib_import(
    hir: &zutai_hir::HirFile,
    expr: zutai_hir::HirExprId,
    module: &str,
    seen: &mut rustc_hash::FxHashSet<zutai_hir::BindingId>,
) -> bool {
    match &hir.expr_arena[expr].kind {
        zutai_hir::HirExprKind::Import(zutai_hir::HirImportSource::Path(parts)) => {
            matches!(parts.as_slice(), [root, name] if root == "stdlib" && name == module)
        }
        zutai_hir::HirExprKind::BindingRef(binding) => {
            if !seen.insert(*binding) {
                return false;
            }
            value_decl_expr(hir, *binding)
                .is_some_and(|value| expr_is_stdlib_import(hir, value, module, seen))
        }
        _ => false,
    }
}

fn value_decl_expr(
    hir: &zutai_hir::HirFile,
    binding: zutai_hir::BindingId,
) -> Option<zutai_hir::HirExprId> {
    hir.decls.iter().find_map(|decl_id| {
        let decl = &hir.decl_arena[*decl_id];
        if decl.binding != binding {
            return None;
        }
        let zutai_hir::HirDeclKind::Value { value, .. } = decl.kind else {
            return None;
        };
        Some(value)
    })
}

fn thir_decl_exprs(
    file: &zutai_thir::ThirFile,
    binding: zutai_hir::BindingId,
) -> Vec<zutai_thir::ThirExprId> {
    file.decls
        .iter()
        .find_map(|decl_id| {
            let decl = &file.decl_arena[*decl_id];
            if decl.binding != binding {
                return None;
            }
            match &decl.kind {
                zutai_thir::ThirDeclKind::Value { value, .. } => Some(vec![*value]),
                zutai_thir::ThirDeclKind::Function { clauses, .. } => Some(
                    clauses
                        .iter()
                        .flat_map(|clause| {
                            clause.guard.into_iter().chain(std::iter::once(clause.body))
                        })
                        .collect(),
                ),
                _ => Some(Vec::new()),
            }
        })
        .unwrap_or_default()
}

fn thir_expr_is_stdlib_reflect_alias(
    hir: &zutai_hir::HirFile,
    file: &zutai_thir::ThirFile,
    expr: zutai_thir::ThirExprId,
    fields: &[&str],
    seen_bindings: &mut FxHashSet<zutai_hir::BindingId>,
) -> bool {
    if stdlib_module_field(
        hir,
        file.expr_arena[expr].source,
        "reflect",
        fields,
        &mut FxHashSet::default(),
    )
    .is_some()
    {
        return true;
    }
    match &file.expr_arena[expr].kind {
        zutai_thir::ThirExprKind::BindingRef { binding, .. } => {
            if seen_bindings.insert(*binding) {
                for body in thir_decl_exprs(file, *binding) {
                    if thir_expr_is_stdlib_reflect_alias(hir, file, body, fields, seen_bindings) {
                        return true;
                    }
                }
            }
            false
        }
        _ => false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_stops_before_hir() {
        let analysis = analyze("{1; 2}");

        assert!(analysis.has_parse_errors());
        assert!(analysis.hir.is_none());
        assert!(analysis.thir.is_none());
    }

    #[test]
    fn hir_error_stops_before_thir() {
        let analysis = analyze("missing");

        assert!(!analysis.has_parse_errors());
        assert!(analysis.has_hir_errors());
        assert!(analysis.hir.is_some());
        assert!(analysis.thir.is_none());
    }

    #[test]
    fn structural_key_hir_error_stops_before_thir() {
        let analysis = analyze("T :: type { a : Int; a : Text; };\nT");

        assert!(!analysis.has_parse_errors());
        assert!(analysis.has_hir_errors());
        assert!(analysis.hir.is_some());
        assert!(analysis.thir.is_none());
        assert!(analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind,
                SemanticDiagnosticKind::Hir(zutai_hir::HirDiagnostic {
                    kind: zutai_hir::HirDiagnosticKind::DuplicateTypeRecordField { .. },
                    ..
                })
            )
        }));
    }

    #[test]
    fn valid_parse_and_hir_reaches_thir_stage() {
        let analysis = analyze("x ::= 1;\nx");

        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors());
        assert!(analysis.ast.is_some());
        assert!(analysis.hir.is_some());
        assert!(analysis.thir.is_some());
        assert!(analysis.is_thir_complete());
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn thir_type_error_is_reported_by_semantic_analysis() {
        let analysis = analyze("x :: Int = \"bad\";\nx");

        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors());
        assert!(analysis.hir.is_some());
        assert!(analysis.thir.is_some());
        assert!(!analysis.is_thir_complete());
        assert!(analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                &diagnostic.kind,
                SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                    kind: zutai_thir::ThirDiagnosticKind::TypeMismatch { expected, found },
                    ..
                }) if expected == "Int" && found == "Text"
            )
        }));
    }

    /// Whether `analysis` carries a `WitnessReflectNotInScope` diagnostic whose
    /// constraint (and, when given, target) match. Mirrors the removed THIR
    /// bare-dispatch gate, now enforced during TLC lowering against the merged
    /// witness registry.
    fn has_witness_not_in_scope(
        analysis: &Analysis,
        constraint: &str,
        target: Option<&str>,
    ) -> bool {
        analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                &diagnostic.kind,
                SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                    kind: zutai_thir::ThirDiagnosticKind::WitnessReflectNotInScope { constraint: c, target: t },
                    ..
                }) if c == constraint && target.is_none_or(|want| t == want)
            )
        })
    }

    /// A bare constraint-method dispatch to a concrete target with no witness in
    /// scope is refused during analysis (relocated from the THIR gate): THIR is
    /// complete, but the S1 witness-existence gate raises
    /// `WitnessReflectNotInScope` so `check`/`compile`/eval refuse the call.
    #[test]
    fn bare_method_call_missing_witness_rejected_by_analysis() {
        let analysis = analyze("Show :: <A> @A { show :: A -> Text; }\nshow 42");

        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors());
        assert!(analysis.is_thir_complete());
        assert!(
            has_witness_not_in_scope(&analysis, "Show", Some("Int")),
            "expected WitnessReflectNotInScope for Show @Int; diagnostics: {:?}",
            analysis.diagnostics
        );
    }

    /// A bare dispatch on a structural union value (`show (#err)`) whose only
    /// witness targets the nominal union does not match the operand key, so the
    /// gate refuses it — the structural-target rejection the THIR gate covered.
    #[test]
    fn bare_method_call_structural_target_without_witness_rejected() {
        let analysis = analyze(
            "Status :: type { #ok; #err; };\nShow :: <A> @A { show :: A -> Text; }\nShow @Status :: { show = \\s. \"shown\"; }\nshow (#err)",
        );

        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors());
        assert!(
            has_witness_not_in_scope(&analysis, "Show", None),
            "expected WitnessReflectNotInScope for `Show`; diagnostics: {:?}",
            analysis.diagnostics
        );
    }

    /// A concrete named target with a witness in scope stays valid: the gate
    /// resolves it against the registry and emits nothing.
    #[test]
    fn bare_method_call_with_witness_is_accepted() {
        let analysis = analyze(
            "Show :: <A> @A { show :: A -> Text; }\nShow @Text :: { show = \\s. s; }\nshow \"x\"",
        );

        assert!(analysis.is_thir_complete());
        assert!(
            !has_witness_not_in_scope(&analysis, "Show", None),
            "unexpected witness-not-in-scope; diagnostics: {:?}",
            analysis.diagnostics
        );
    }

    /// Q4 regression: a bare record-literal argument to a derived/constraint
    /// method is inferred structurally and unified into the parameter's inference
    /// variable, so the witness gate names a concrete record target
    /// (`Show @{ x : Int }`) instead of leaking the raw `?N` metavariable that
    /// the old `ExpectedRecord` refusal produced. The call still has no witness,
    /// so it is refused — but with an actionable target name.
    #[test]
    fn q4_record_literal_dispatch_names_concrete_target_not_metavar() {
        let analysis = analyze(
            "Show :: <A> @A { show :: A -> Text; }\nShow @Int :: { show = \\n. \"N\"; }\nshow { x = 1; }",
        );

        assert!(analysis.is_thir_complete());
        let target =
            analysis
                .diagnostics
                .iter()
                .find_map(|diagnostic| match &diagnostic.kind {
                    SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                        kind:
                            zutai_thir::ThirDiagnosticKind::WitnessReflectNotInScope {
                                constraint,
                                target,
                            },
                        ..
                    }) if constraint == "Show" => Some(target.clone()),
                    _ => None,
                });
        let target = target.expect("expected a Show witness-not-in-scope diagnostic");
        assert!(
            target.contains("x") && !target.contains('?'),
            "expected a concrete record target with no leaked metavariable, got {target:?}"
        );
    }

    // ── `.zti` imports ────────────────────────────────────────────────────────

    fn imports_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports")
    }

    fn analyze_in_imports(input: &str) -> Analysis {
        analyze_with_base(input, Some(&imports_dir()), AnalysisOptions::default())
    }

    fn has_import_diagnostic(analysis: &Analysis, expected: &ImportDiagnosticKind) -> bool {
        analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                &diagnostic.kind,
                SemanticDiagnosticKind::Import(import) if &import.kind == expected
            )
        })
    }

    #[test]
    fn import_with_base_completes() {
        let analysis = analyze_in_imports("cfg ::= import \"config.zti\";\ncfg.port");
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    // ── filesystem stdlib + destructuring imports ──────────────────────────────

    #[test]
    fn stdlib_stream_import_resolves_without_base() {
        // Dotted stdlib imports need no user-module base directory.
        let analysis =
            analyze("s ::= import stdlib.stream;\ns.fold (\\acc x. acc + x) 0 (s.singleton 5)");
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_stream_exports_encode_without_exporting_constraint_values() {
        let analysis = analyze(
            "s ::= import stdlib.stream;\nencoded :: s.Data = s.encode {1; 2; 3;};\nencoded",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn unknown_stdlib_module_is_diagnosed() {
        let analysis = analyze("s ::= import stdlib.nope;\ns");
        assert!(has_import_diagnostic(
            &analysis,
            &ImportDiagnosticKind::UnknownStdlibModule {
                name: "nope".to_string(),
            },
        ));
    }

    #[test]
    fn destructured_stdlib_members_bind_unqualified() {
        let analysis = analyze(
            "s ::= import stdlib.stream;\n{ map; fold; singleton; } ::= s;\nfold (\\a x. a + x) 0 (map (\\n. n + 1) (singleton 4))",
        );
        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn destructuring_an_unknown_field_is_diagnosed() {
        let analysis = analyze("s ::= import stdlib.stream;\n{ nope; } ::= s;\nnope");
        assert!(!analysis.is_thir_complete());
    }

    // ── ambient function prelude (stdlib slice B) ──────────────────────────────

    #[test]
    fn function_prelude_ambient_resolves_without_import() {
        // `id`/`const`/`compose`/`flip` are ambient (no import); a higher-order
        // use type-checks end-to-end through the semantic facade.
        let analysis = analyze("compose (\\x. x + 1) (\\x. x * 2) 3");
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_prelude_import_resolves_without_base() {
        let analysis =
            analyze("p ::= import stdlib.prelude;\np.compose (\\x. x + 1) (\\x. x * 2) 3");
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_optional_import_resolves_without_base() {
        let analysis = analyze(
            "o ::= import stdlib.optional;\no.withDefault 0 (o.map (\\x. x + 1) (#some (41)))",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_result_import_resolves_without_base() {
        let analysis = analyze(
            "r ::= import stdlib.result;\n\
             res :: r.Result Text Int = r.ok 41;\n\
             r.withDefault 0 (r.map (\\x. x + 1) res)",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn destructured_stdlib_result_members_bind_unqualified() {
        let analysis = analyze(
            "{ ok; err; map; withDefault; } ::= import stdlib.result;\n\
             withDefault 0 (map (\\n. n + 1) (if true then ok 4 else err \"x\"))",
        );
        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_num_import_resolves_without_base() {
        let analysis = analyze(
            "n ::= import stdlib.num;\n\
             n.clamp 0 10 (n.max (n.min 99 7) (n.abs (0 - 3)))",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn destructured_stdlib_num_members_bind_unqualified() {
        let analysis = analyze(
            "{ gcd; rem; pow; toFloat; round; truncate; } ::= import stdlib.num;\n\
             gcd 54 24 + rem 17 5 + pow 2 3 + round (toFloat 4) + truncate 3.9",
        );
        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_text_import_resolves_without_base() {
        let analysis = analyze(
            "t ::= import stdlib.text;\n\
             t.length (t.replace \"a\" \"o\" (t.trim \" cat \")) + t.length (t.show \"x\")",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_cmp_import_resolves_without_base() {
        let analysis = analyze(
            "c ::= import stdlib.cmp;\n\
             c.then (c.compareInt 1 2) (c.reverse c.gt)",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_list_import_resolves_without_base() {
        let analysis = analyze(
            "l ::= import stdlib.list;\n\
             c ::= import stdlib.cmp;\n\
             l.sum (l.take 3 (l.sortBy c.compareInt {3; 1; 2; 4;})) + l.product {2; 3;}",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_data_import_resolves_without_base() {
        let analysis = analyze(
            "d ::= import stdlib.data;\n\
             value ::= d.record { d.fieldOf \"port\" (d.int 8080); };\n\
             match d.field \"port\" value { | #ok { value = found; } => d.asInt found; | #err { error = error; } => #err { error = error; }; }",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_validate_import_resolves_without_base() {
        let analysis = analyze(
            "v ::= import stdlib.validate;\n\
             length (v.errors (v.map3 (\\a b c. a + b + c) (v.valid 1) (v.intRange \"x\" 0 10 20) (v.required \"name\" (#none))))",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_net_import_resolves_without_base() {
        let analysis = analyze(
            "net ::= import stdlib.net;\n\
             main :: Net -> net.Server Text\n\
               = cap => [\n\
                 listener := net.listen cap 7777;\n\
                 net.withConnection cap listener (\\conn. [\n\
                   line := net.read cap conn;\n\
                   net.write cap line;\n\
                   line\n\
                 ])\n\
               ];\n\
             main",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn browser_stdlib_modules_resolve_together() {
        let analysis = analyze(
            "css ::= import stdlib.css;\n\
             html ::= import stdlib.html;\n\
             browser ::= import stdlib.browser;\n\
             (css, html, browser)",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_config_import_resolves_and_overlay_alias_typechecks() {
        let analysis = analyze(
            "cfg ::= import stdlib.config;\n\
             Server :: type { host : Text; port : Int; };\n\
             base :: Server = { host = \"127.0.0.1\"; port = 8080; };\n\
             (cfg.overlay { port = 9090; } base).port",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn destructured_stdlib_config_overlay_shadows_builtin_and_typechecks() {
        let analysis = analyze(
            "{ overlay; } ::= import stdlib.config;\n\
             Server :: type { host : Text; port : Int; };\n\
             base :: Server = { host = \"127.0.0.1\"; port = 8080; };\n\
             (overlay { port = 9090; } base).port",
        );
        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn stdlib_reflect_import_resolves_without_base() {
        let analysis = analyze(
            "refl ::= import stdlib.reflect;\n\
             Server :: type { host : Text; port : Int; };\n\
             length ((refl.schema Server).fields ?? {;})",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        assert!(analysis.reflection_builtin_program().is_some());
        assert!(analysis.aot_reflection_program().is_some());
    }

    #[test]
    fn destructured_stdlib_reflect_alias_use_triggers_reflection_gate() {
        let analysis = analyze(
            "{ schema; } ::= import stdlib.reflect;\n\
             Server :: type { host : Text; port : Int; };\n\
             length ((schema Server).fields ?? {;})",
        );
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(analysis.reflection_builtin_program().is_some());
        assert!(analysis.aot_reflection_program().is_some());
    }

    #[test]
    fn unused_destructured_stdlib_reflect_import_does_not_trip_backend_gate() {
        let analysis = analyze("{ schema; } ::= import stdlib.reflect;\n1");
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(analysis.reflection_builtin_program().is_none());
        assert!(analysis.aot_reflection_program().is_none());
    }

    #[test]
    fn unused_stdlib_config_and_reflect_imports_do_not_trip_backend_gates() {
        let analysis = analyze("cfg ::= import stdlib.config;\nrefl ::= import stdlib.reflect;\n1");
        assert!(!analysis.has_parse_errors());
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(analysis.config_overlay_builtin_program().is_none());
        assert!(analysis.reflection_builtin_program().is_none());
        assert!(analysis.aot_reflection_program().is_none());
    }

    #[test]
    fn destructured_stdlib_optional_members_bind_unqualified() {
        let analysis = analyze(
            "{ map; withDefault; } ::= import stdlib.optional;\nwithDefault 0 (map (\\n. n + 1) (#some (4)))",
        );
        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn user_binding_shadows_prelude_without_duplicate_diagnostic() {
        // The prelude is a fallback: a user `id` of the same name wins and raises
        // no duplicate-binding diagnostic.
        let analysis = analyze("id :: Int -> Int = x => x + 1;\nid 5");
        assert!(!analysis.has_parse_errors());
        assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    }

    #[test]
    fn analyze_path_resolves_relative_import() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/importer.zt");
        let analysis = analyze_path(&path).expect("read importer.zt");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn analyze_path_resolves_relative_zt_import() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/zt_importer.zt");
        let analysis = analyze_path(&path).expect("read zt_importer.zt");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn analyze_sources_resolves_transitive_zt_and_zti_imports() {
        let sources = BTreeMap::from([
            (
                "main.zt".to_string(),
                "lib ::= import \"modules/site.zt\";\nlib.port".to_string(),
            ),
            (
                "modules/site.zt".to_string(),
                "cfg ::= import \"../config.zti\";\n{ port = cfg.port; }".to_string(),
            ),
            ("config.zti".to_string(), "{ port = 8787; }".to_string()),
        ]);

        let analysis = analyze_sources("main.zt", &sources, AnalysisOptions::default()).unwrap();
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        assert_eq!(analysis.import_modules.len(), 1);
    }

    #[test]
    fn imported_data_type_mismatch_retains_zti_value_span() {
        let a = "{\n  host = \"localhost\";\n  port = \"wrong\";\n}\n";
        let sources = BTreeMap::from([
            (
                "C.zt".to_string(),
                "b ::= import \"B.zt\";\na ::= import \"A.zti\";\nchecked :: b.Config = a;\nchecked\n"
                    .to_string(),
            ),
            (
                "B.zt".to_string(),
                "Config :: type { host : Text; port : Int; };\n{ Config = Config; }\n"
                    .to_string(),
            ),
            ("A.zti".to_string(), a.to_string()),
        ]);

        let analysis = analyze_sources("C.zt", &sources, AnalysisOptions::default()).unwrap();
        let origin = analysis
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                    kind:
                        zutai_thir::ThirDiagnosticKind::ImportedDataTypeMismatch {
                            expected,
                            found,
                            origin,
                        },
                    ..
                }) if expected == "Int" && found == "Text" => Some(origin),
                _ => None,
            })
            .expect("expected imported-data mismatch");
        assert_eq!(
            &a[origin.span.start as usize..origin.span.end as usize],
            "\"wrong\""
        );
        assert!(matches!(
            &origin.source,
            zutai_hir::HirImportSource::String(path) if path == "A.zti"
        ));
    }

    #[test]
    fn heterogeneous_imported_list_checks_each_item_against_context() {
        let a = "{ ports = [1; \"wrong\";]; }";
        let sources = BTreeMap::from([
            (
                "C.zt".to_string(),
                "b ::= import \"B.zt\";\na ::= import \"A.zti\";\nchecked :: b.Config = a;\nchecked\n"
                    .to_string(),
            ),
            (
                "B.zt".to_string(),
                "Config :: type { ports : List Int; };\n{ Config = Config; }\n".to_string(),
            ),
            ("A.zti".to_string(), a.to_string()),
        ]);

        let analysis = analyze_sources("C.zt", &sources, AnalysisOptions::default()).unwrap();
        let origin = analysis
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                    kind: zutai_thir::ThirDiagnosticKind::ImportedDataTypeMismatch { origin, .. },
                    ..
                }) => Some(origin),
                _ => None,
            })
            .expect("expected heterogeneous-list mismatch");
        assert_eq!(
            &a[origin.span.start as usize..origin.span.end as usize],
            "\"wrong\""
        );
    }

    #[test]
    fn analyze_sources_rejects_non_normalized_or_missing_entries() {
        let sources = BTreeMap::from([("main.zt".to_string(), "1".to_string())]);
        assert!(matches!(
            analyze_sources("../main.zt", &sources, AnalysisOptions::default()),
            Err(SourceMapError::InvalidPath { .. })
        ));
        assert!(matches!(
            analyze_sources("C:/main.zt", &sources, AnalysisOptions::default()),
            Err(SourceMapError::InvalidPath { .. })
        ));
        assert!(matches!(
            analyze_sources("missing.zt", &sources, AnalysisOptions::default()),
            Err(SourceMapError::MissingEntry { .. })
        ));

        let invalid = BTreeMap::from([("dir\\main.zt".to_string(), "1".to_string())]);
        assert!(matches!(
            analyze_sources("dir/main.zt", &invalid, AnalysisOptions::default()),
            Err(SourceMapError::InvalidPath { .. })
        ));
    }

    #[test]
    fn analyze_sources_confines_imports_to_virtual_root() {
        let sources = BTreeMap::from([
            (
                "main.zt".to_string(),
                "secret ::= import \"../secret.zti\";\nsecret".to_string(),
            ),
            ("secret.zti".to_string(), "value = 1".to_string()),
        ]);
        let analysis = analyze_sources("main.zt", &sources, AnalysisOptions::default()).unwrap();
        assert!(has_import_diagnostic(
            &analysis,
            &ImportDiagnosticKind::PathTraversal {
                path: "../secret.zti".to_string(),
            },
        ));
    }

    #[test]
    fn analyze_path_recording_captures_transitive_sources() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/chain_top.zt");
        let recorded = analyze_path_recording(&path).expect("record source graph");
        assert_eq!(recorded.entry, "chain_top.zt");
        assert_eq!(
            recorded.sources.keys().cloned().collect::<Vec<_>>(),
            vec![
                "chain_mid.zt".to_string(),
                "chain_top.zt".to_string(),
                "config.zti".to_string(),
            ]
        );
        assert_eq!(
            recorded.stdlib_sources.keys().cloned().collect::<Vec<_>>(),
            vec!["prelude".to_string(), "stream".to_string()]
        );
        assert_eq!(
            recorded.stdlib_compiler_compatibility,
            STDLIB_COMPILER_COMPATIBILITY
        );
        assert!(recorded.analysis.is_thir_complete());
    }

    #[test]
    fn recording_bundles_only_used_stdlib_modules_plus_ambient_preludes() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-semantic-stdlib-recording-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let entry = root.join("main.zt");
        std::fs::write(&entry, "n ::= import stdlib.num;\nn.abs -1\n").unwrap();

        let recorded = analyze_path_recording(&entry).expect("record stdlib source graph");
        assert_eq!(
            recorded.stdlib_sources.keys().cloned().collect::<Vec<_>>(),
            vec![
                "num".to_string(),
                "prelude".to_string(),
                "stream".to_string(),
            ]
        );
        assert!(recorded.analysis.is_thir_complete());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_recording_root_allows_sibling_imports_inside_root() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-semantic-source-root-{}-{unique}",
            std::process::id()
        ));
        let app = root.join("app");
        let shared = root.join("shared");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        let entry = app.join("main.zt");
        std::fs::write(&entry, "cfg ::= import \"../shared/config.zti\";\ncfg.port").unwrap();
        std::fs::write(shared.join("config.zti"), "{ port = 8787; }").unwrap();

        let recorded = analyze_path_recording_with_root(&entry, &root).unwrap();
        assert_eq!(recorded.entry, "app/main.zt");
        assert_eq!(
            recorded.sources.keys().cloned().collect::<Vec<_>>(),
            vec!["app/main.zt".to_string(), "shared/config.zti".to_string()]
        );
        assert!(
            recorded.analysis.is_thir_complete(),
            "{:?}",
            recorded.analysis.diagnostics
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn analysis_cache_reuses_unchanged_import_graph() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-semantic-cache-reuse-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let entry = root.join("main.zt");
        std::fs::write(&entry, "mid ::= import \"mid.zt\";\nmid\n").unwrap();
        std::fs::write(root.join("mid.zt"), "leaf ::= import \"leaf.zt\";\nleaf\n").unwrap();
        std::fs::write(root.join("leaf.zt"), "42\n").unwrap();

        let cache = AnalysisCache::default();
        let first = analyze_path_with_cache(&entry, &cache).unwrap();
        assert!(first.is_thir_complete(), "{:?}", first.diagnostics);
        assert_eq!(
            cache.stats(),
            AnalysisCacheStats {
                module_hits: 0,
                module_misses: 2,
            }
        );

        let second = analyze_path_with_cache(&entry, &cache).unwrap();
        assert!(second.is_thir_complete(), "{:?}", second.diagnostics);
        assert_eq!(
            cache.stats(),
            AnalysisCacheStats {
                module_hits: 1,
                module_misses: 2,
            }
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn analysis_cache_invalidates_changed_module_and_dependents_only() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-semantic-cache-invalidation-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let entry = root.join("main.zt");
        std::fs::write(
            &entry,
            "left ::= import \"left.zt\";\nright ::= import \"right.zt\";\nleft + right\n",
        )
        .unwrap();
        std::fs::write(root.join("left.zt"), "leaf ::= import \"leaf.zt\";\nleaf\n").unwrap();
        std::fs::write(root.join("right.zt"), "2\n").unwrap();
        std::fs::write(root.join("leaf.zt"), "1\n").unwrap();

        let cache = AnalysisCache::default();
        let first = analyze_path_with_cache(&entry, &cache).unwrap();
        assert!(first.is_thir_complete(), "{:?}", first.diagnostics);
        assert_eq!(cache.stats().module_misses, 3);

        std::fs::write(root.join("leaf.zt"), "3\n").unwrap();
        let second = analyze_path_with_cache(&entry, &cache).unwrap();
        assert!(second.is_thir_complete(), "{:?}", second.diagnostics);
        assert_eq!(
            cache.stats(),
            AnalysisCacheStats {
                module_hits: 1,
                module_misses: 5,
            },
            "the independent right module should hit while leaf and left miss"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn analysis_cache_invalidates_data_import_dependents() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-semantic-cache-data-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let entry = root.join("main.zt");
        std::fs::write(&entry, "m ::= import \"module.zt\";\nm.port\n").unwrap();
        std::fs::write(
            root.join("module.zt"),
            "cfg ::= import \"config.zti\";\ncfg\n",
        )
        .unwrap();
        std::fs::write(root.join("config.zti"), "{ port = 1; }\n").unwrap();

        let cache = AnalysisCache::default();
        let first = analyze_path_with_cache(&entry, &cache).unwrap();
        assert!(first.is_thir_complete(), "{:?}", first.diagnostics);
        std::fs::write(root.join("config.zti"), "{ port = 2; }\n").unwrap();
        let second = analyze_path_with_cache(&entry, &cache).unwrap();
        assert!(second.is_thir_complete(), "{:?}", second.diagnostics);
        assert_eq!(
            cache.stats(),
            AnalysisCacheStats {
                module_hits: 0,
                module_misses: 2,
            }
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn analysis_recording_captures_all_public_package_modules() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-semantic-recorded-public-modules-{}-{unique}",
            std::process::id()
        ));
        let app = root.join("app");
        let dep = root.join("dep");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(dep.join("src")).unwrap();
        std::fs::write(
            app.join("zutai.zti"),
            format!(
                "{{ formatVersion = 1; name = \"app\"; compilerCompatibility = \"{}\"; modules = []; dependencies = [{{ alias = \"dep\"; path = \"../dep\"; }};]; }}",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();
        std::fs::write(
            dep.join("zutai.zti"),
            format!(
                "{{ formatVersion = 1; name = \"dep\"; compilerCompatibility = \"{}\"; modules = [{{ name = \"api\"; path = \"src/api.zt\"; }}; {{ name = \"unused\"; path = \"src/unused.zt\"; }};]; dependencies = []; }}",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();
        let entry = app.join("src/main.zt");
        let api = dep.join("src/api.zt");
        let unused = dep.join("src/unused.zt");
        std::fs::write(&entry, "api ::= import dep.api;\napi.answer\n").unwrap();
        std::fs::write(&api, "answer ::= 1;\n{ answer = answer; }\n").unwrap();
        std::fs::write(&unused, "unused ::= 2;\n{ unused = unused; }\n").unwrap();

        let recorded = analyze_path_recording(&entry).unwrap();
        let dep_package = recorded
            .packages
            .packages
            .values()
            .find(|package| package.name == "dep")
            .unwrap();
        assert_eq!(
            dep_package.sources.keys().cloned().collect::<Vec<_>>(),
            vec!["src/api.zt", "src/unused.zt"]
        );
        assert_eq!(
            recorded
                .source_paths
                .values()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                std::fs::canonicalize(api).unwrap(),
                std::fs::canonicalize(unused).unwrap(),
            ])
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn analysis_cache_hits_preserve_recorded_source_graph() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-semantic-cache-recording-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let entry = root.join("main.zt");
        std::fs::write(&entry, "mid ::= import \"mid.zt\";\nmid\n").unwrap();
        std::fs::write(
            root.join("mid.zt"),
            "cfg ::= import \"config.zti\";\ncfg.port\n",
        )
        .unwrap();
        std::fs::write(root.join("config.zti"), "{ port = 1; }\n").unwrap();

        let cache = AnalysisCache::default();
        let first = analyze_path_recording_with_root_and_cache(&entry, &root, &cache).unwrap();
        let second = analyze_path_recording_with_root_and_cache(&entry, &root, &cache).unwrap();
        assert!(
            second.analysis.is_thir_complete(),
            "{:?}",
            second.analysis.diagnostics
        );
        assert_eq!(second.sources, first.sources);
        assert_eq!(second.source_paths, first.source_paths);
        assert!(cache.stats().module_hits >= 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn analysis_cache_hits_preserve_explicit_stdlib_sources() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "zutai-semantic-cache-stdlib-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let entry = root.join("main.zt");
        std::fs::write(&entry, "mid ::= import \"mid.zt\";\nmid\n").unwrap();
        std::fs::write(
            root.join("mid.zt"),
            "num ::= import stdlib.num;\nnum.abs -1\n",
        )
        .unwrap();

        let cache = AnalysisCache::default();
        let first = analyze_path_recording_with_root_and_cache(&entry, &root, &cache).unwrap();
        let second = analyze_path_recording_with_root_and_cache(&entry, &root, &cache).unwrap();
        assert!(
            second.analysis.is_thir_complete(),
            "{:?}",
            second.analysis.diagnostics
        );
        assert_eq!(second.stdlib_sources, first.stdlib_sources);
        assert!(second.stdlib_sources.contains_key("num"));
        assert!(cache.stats().module_hits >= 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn import_without_base_reports_no_base_directory() {
        let analysis = analyze("cfg ::= import \"config.zti\";\ncfg.port");
        assert!(!analysis.is_thir_complete());
        assert!(has_import_diagnostic(
            &analysis,
            &ImportDiagnosticKind::NoBaseDirectory
        ));
    }

    #[test]
    fn import_missing_file_reports_file_not_found() {
        let analysis = analyze_in_imports("cfg ::= import \"nope.zti\";\ncfg");
        assert!(!analysis.is_thir_complete());
        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Import(import)
                if matches!(import.kind, ImportDiagnosticKind::FileNotFound { .. })
        )));
    }

    #[test]
    fn zt_import_data_module_completes() {
        let analysis = analyze_in_imports("m ::= import \"data_module.zt\";\nm.doubled");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn zt_import_transitive_completes() {
        // chain_top.zt -> chain_mid.zt -> config.zti.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/chain_top.zt");
        let analysis = analyze_path(&path).expect("read chain_top.zt");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    }

    #[test]
    fn zt_import_function_module_completes() {
        // func_module.zt exports a function; the import must now succeed and
        // produce a complete THIR (no UnsupportedExport diagnostic).
        let analysis = analyze_in_imports("f ::= import \"func_module.zt\";\nf");
        assert!(
            analysis.is_thir_complete(),
            "expected complete THIR, got diagnostics: {:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn imported_higher_kinded_witness_is_native_matchable() {
        let source = "m ::= import \"hkt_witness_list.zt\";\nm\n";
        let analysis = analyze_in_imports(source);
        assert!(
            analysis.native_import_diagnostics().is_empty(),
            "bare first-order constructor witness should cross imports"
        );
        assert!(analysis.witness_exports.iter().any(|witness| {
            witness.constraint == "Functor"
                && witness.target_key == "List"
                && witness.conditional.is_none()
        }));
    }

    #[test]
    fn imported_partial_alias_witness_is_runtime_matchable() {
        let source = "m ::= import \"hkt_witness_result.zt\";\nm\n";
        let analysis = analyze_in_imports(source);
        assert!(analysis.native_import_diagnostics().is_empty());
        let witness = analysis
            .witness_exports
            .iter()
            .find(|witness| witness.constraint == "Functor")
            .expect("Functor witness export");
        assert!(witness.target_key.starts_with("Result['"));
        let conditional = witness
            .conditional
            .as_ref()
            .expect("partial alias witness should export a matcher");
        assert_eq!(conditional.param_bounds.len(), 1);
        assert!(
            zutai_thir::match_pattern_key(
                &conditional.pattern,
                "Result[Text][Int]",
                conditional.param_bounds.len(),
            )
            .is_some()
        );
        assert!(
            zutai_thir::match_pattern_key(
                &conditional.pattern,
                "<ok({value:Int})|err({error:Text})>",
                conditional.param_bounds.len(),
            )
            .is_none()
        );
    }

    #[test]
    fn genuinely_nonmatchable_witness_keeps_source_located_refusal() {
        let source = "m ::= import \"nonmatchable_witness.zt\";\nm\n";
        let analysis = analyze_in_imports(source);
        let diagnostics = analysis.native_import_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(
            &source[diagnostic.span.start as usize..diagnostic.span.end as usize],
            "import \"nonmatchable_witness.zt\""
        );
        assert_eq!(diagnostic.related.len(), 1);
        assert!(
            diagnostic.related[0]
                .path
                .ends_with("nonmatchable_witness.zt")
        );
        assert_eq!(
            diagnostic.related[0].label,
            "non-matchable witness exported here"
        );
    }
    #[test]
    fn cross_module_conflicting_witnesses_report_import_error() {
        let analysis = analyze_in_imports(
            r#"
a ::= import "witness_eq_int_a.zt";
b ::= import "witness_eq_int_b.zt";
(a, b)
"#,
        );

        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Import(import)
                if matches!(
                    &import.kind,
                    ImportDiagnosticKind::ConflictingWitness { constraint, target }
                        if constraint == "Eq" && target == "Int"
                )
        )));
        assert!(analysis.blocking_diagnostics().next().is_some());
    }

    #[test]
    fn imported_witness_conflicts_with_local_witness() {
        let analysis = analyze_in_imports(
            r#"
a ::= import "witness_eq_int_a.zt";
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
a
"#,
        );

        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Import(import)
                if matches!(
                    &import.kind,
                    ImportDiagnosticKind::ConflictingWitness { constraint, target }
                        if constraint == "Eq" && target == "Int"
                )
        )));
        assert!(analysis.blocking_diagnostics().next().is_some());
    }

    #[test]
    fn cross_module_distinct_witness_targets_complete() {
        let analysis = analyze_in_imports(
            r#"
a ::= import "witness_eq_int_a.zt";
b ::= import "witness_eq_bool.zt";
(a, b)
"#,
        );

        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn cross_module_same_witness_reexport_is_deduped() {
        let analysis = analyze_in_imports(
            r#"
a ::= import "witness_reexport_a.zt";
b ::= import "witness_reexport_b.zt";
(a, b)
"#,
        );

        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn zt_import_cycle_is_reported() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/cycle_a.zt");
        let analysis = analyze_path(&path).expect("read cycle_a.zt");
        assert!(!analysis.is_thir_complete());
        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Import(import)
                if matches!(import.kind, ImportDiagnosticKind::ImportCycle { .. })
        )));
        let cycle = analysis
            .diagnostics
            .iter()
            .find_map(|diagnostic| match &diagnostic.kind {
                SemanticDiagnosticKind::Import(import)
                    if matches!(import.kind, ImportDiagnosticKind::ImportCycle { .. }) =>
                {
                    Some(import)
                }
                _ => None,
            })
            .expect("cycle diagnostic");
        assert!(!cycle.related.is_empty());
        assert!(cycle.related.iter().all(|location| {
            location
                .path
                .extension()
                .is_some_and(|extension| extension == "zt")
        }));
    }

    #[test]
    fn bad_zti_reports_parse_error() {
        let analysis = analyze_in_imports("cfg ::= import \"bad.zti\";\ncfg");
        assert!(!analysis.is_thir_complete());
        assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Import(import)
                if matches!(import.kind, ImportDiagnosticKind::ParseError { .. })
        )));
    }

    #[test]
    fn empty_imported_list_still_completes() {
        // Empty `.zti` array → `List(InferVar)`; a free inference variable is
        // allowed in completed THIR.
        let analysis = analyze_in_imports("cfg ::= import \"empty_list.zti\";\ncfg");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    }

    #[test]
    fn mixed_imported_list_still_completes() {
        // Heterogeneous `.zti` array → `List(Union(...))`.
        let analysis = analyze_in_imports("cfg ::= import \"mixed_list.zti\";\ncfg");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    }

    #[test]
    fn import_absolute_path_is_rejected() {
        // Absolute paths are rejected before any filesystem access, so no fixture
        // file is needed; the guard fires in `relative_path`.
        let analysis = analyze_in_imports("cfg ::= import \"/etc/hosts.zti\";\ncfg");
        assert!(
            !analysis.is_thir_complete(),
            "expected incomplete THIR for absolute import"
        );
        assert!(
            has_import_diagnostic(
                &analysis,
                &ImportDiagnosticKind::PathTraversal {
                    path: "/etc/hosts.zti".to_string()
                }
            ),
            "expected PathTraversal diagnostic, got: {:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn import_parent_traversal_is_rejected() {
        // `../expr_core.zt` exists one directory above imports/, but resolving it
        // would escape the imports/ base directory and must be rejected.
        let analysis = analyze_in_imports("cfg ::= import \"../expr_core.zt\";\ncfg");
        assert!(
            !analysis.is_thir_complete(),
            "expected incomplete THIR for parent-traversal import"
        );
        assert!(
            has_import_diagnostic(
                &analysis,
                &ImportDiagnosticKind::PathTraversal {
                    path: "../expr_core.zt".to_string()
                }
            ),
            "expected PathTraversal diagnostic, got: {:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn import_dotdot_within_base_is_allowed() {
        // `../imports/config.zti` canonicalizes back to the imports/ directory
        // itself, so it stays within the base and must succeed.
        let analysis = analyze_in_imports("cfg ::= import \"../imports/config.zti\";\ncfg.port");
        assert!(
            analysis.is_thir_complete(),
            "expected complete THIR for in-base dotdot import, got: {:?}",
            analysis.diagnostics
        );
        assert!(
            analysis.diagnostics.is_empty(),
            "expected no diagnostics, got: {:?}",
            analysis.diagnostics
        );
    }

    #[test]
    fn tlc_is_some_for_complete_thir() {
        let analysis = analyze("x ::= 42;\nx");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.tlc.is_some(),
            "expected TLC module for complete program"
        );
    }

    #[test]
    fn tlc_is_none_for_type_error() {
        let analysis = analyze("x :: Int = \"bad\";\nx");
        assert!(!analysis.is_thir_complete());
        assert!(
            analysis.tlc.is_none(),
            "expected no TLC module for type-error program"
        );
    }

    #[test]
    fn effectful_program_predicate_detects_phase15_effects() {
        let analysis = analyze(
            r#"
Config :: type { value : Text; };
ParseError :: type Text;
parse :: Text -> Config ! { fail ParseError; }
  = text => perform fail text;
parse
"#,
        );
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(analysis.tlc.is_some(), "check path still builds TLC");
        assert!(analysis.effectful_program().is_some());
    }

    #[test]
    fn effectful_program_predicate_ignores_pure_programs() {
        let analysis = analyze("x ::= 1;\nx");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert_eq!(analysis.effectful_program(), None);
    }

    #[test]
    fn tlc_function_row_keeps_parametric_effect_alias() {
        let analysis = analyze(
            r#"
Config :: type { value : Text; };
Eff :: <A> type A ! { fail Text; };
parse :: Text -> Eff Config
  = text => perform fail text;
parse
"#,
        );
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        let tlc = analysis.tlc.expect("check path should build TLC");
        let has_fail_row = tlc.type_arena.iter().any(|(_, ty)| {
            matches!(ty, zutai_tlc::TlcType::Fun(_, _, zutai_tlc::Row::RExtend { label, .. })
                if label == "fail")
        });
        assert!(
            has_fail_row,
            "expected TLC function effect row to include fail"
        );
    }

    /// Walk to the tail of a TLC row, returning its row variable if open.
    fn row_tail_var(row: &zutai_tlc::Row) -> Option<zutai_tlc::TlcTypeVar> {
        match row {
            zutai_tlc::Row::RVar(v) => Some(*v),
            zutai_tlc::Row::RExtend { tail, .. } => row_tail_var(tail),
            zutai_tlc::Row::REmpty => None,
        }
    }

    #[test]
    fn tlc_emits_row_variable_for_named_row_tail() {
        let analysis = analyze(
            "idHost :: <Rest> { host : Text; ...Rest; } -> { host : Text; ...Rest; }\n  = x => x;\nidHost",
        );
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        let tlc = analysis
            .tlc
            .expect("expected TLC module for a complete row program");
        // The row-polymorphic function quantifies its `<Rest>` tail with Kind::Row.
        let has_row_forall = tlc
            .type_arena
            .iter()
            .any(|(_, t)| matches!(t, zutai_tlc::TlcType::ForAll(_, zutai_tlc::Kind::Row(_), _)));
        assert!(
            has_row_forall,
            "expected a ForAll quantifying a row variable with Kind::Row"
        );
        // A record row ends in a named row variable (`...Rest`).
        let has_named_rvar = tlc.type_arena.iter().any(|(_, t)| {
            matches!(t, zutai_tlc::TlcType::Record(row)
                if matches!(row_tail_var(row), Some(zutai_tlc::TlcTypeVar::Named(_))))
        });
        assert!(
            has_named_rvar,
            "expected a record row ending in RVar(Named)"
        );
    }

    #[test]
    fn tlc_closed_record_has_no_free_row_variable() {
        let analysis =
            analyze("s :: { host : Text; port : Int; } = { host = \"h\"; port = 1; };\ns");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        let tlc = analysis.tlc.expect("expected TLC module");
        for (_, t) in tlc.type_arena.iter() {
            if let zutai_tlc::TlcType::Record(row) = t {
                assert!(
                    row_tail_var(row).is_none(),
                    "closed record must contain no row variable: {row:?}"
                );
            }
        }
    }

    #[test]
    fn witness_reflection_is_aot_only_not_run_routing() {
        // `witness C @T` reflection must trigger the compile-time AOT-fold gate
        // but NOT the run-time THIR-routing gate: the TLC evaluator handles it,
        // and routing it to the THIR oracle would regress witness dispatch.
        let analysis = analyze(
            "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: derive\nwitness Eq @Int",
        );
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.aot_reflection_program().is_some(),
            "witness reflection must be detected for AOT folding"
        );
        assert!(
            analysis.reflection_builtin_program().is_none(),
            "witness reflection must NOT trigger THIR run routing"
        );
    }

    #[test]
    fn variants_reflection_is_aot_only_not_run_routing() {
        // `variants` folds/evaluates on the TLC path, so it belongs to the AOT
        // gate but not the THIR-routing gate.
        let analysis = analyze("Color :: type { #red: {}; #green: {}; };\nvariants (Color)");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(
            analysis.aot_reflection_program().is_some(),
            "variants reflection must be detected for AOT folding"
        );
        assert!(
            analysis.reflection_builtin_program().is_none(),
            "variants reflection must NOT trigger THIR run routing"
        );
    }

    #[test]
    fn schema_reflection_triggers_both_gates() {
        // `schema`/`fields` need the THIR oracle, so they trigger both gates.
        let analysis = analyze("Server :: type { host : Text; };\nschema Server");
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(analysis.reflection_builtin_program().is_some());
        assert!(analysis.aot_reflection_program().is_some());
    }

    #[test]
    fn implicit_witness_dispatch_is_not_reflection() {
        // Implicit method dispatch (`lt 1 2`) is not the `witness` reflection
        // expression and must lower natively — neither gate fires.
        let analysis = analyze(
            "Ord :: <A> @A { lt :: A -> A -> Bool; }\nOrd @Int :: { lt = \\a b. a < b; }\nlt 1 2",
        );
        assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
        assert!(analysis.aot_reflection_program().is_none());
        assert!(analysis.reflection_builtin_program().is_none());
    }
}
