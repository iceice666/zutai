//! Module loader for imports.
//!
//! THIR lowering is pure, so all filesystem work happens here: walk the HIR for
//! `import` expressions, resolve each path relative to the importing file's
//! directory, and produce both a structural type (for THIR) and the data needed
//! by the evaluator.
//!
//! - `.zti` (immediate data): parse the file and keep the parsed value; its type
//!   is derived structurally.
//! - `.zt` (module): recursively analyze the file, type the import by its final
//!   expression's exported type, and keep the analyzed sub-module so the
//!   evaluator can evaluate it.  Import cycles are detected and reported.
//!
//! Only self-contained *data* can cross a module boundary — the evaluator binds
//! closures and type values to file-relative arena indices — so a `.zt` module
//! whose final value is (or contains) a function or type is refused here rather
//! than mis-evaluated.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use zutai_hir::{HirExprKind, HirFile, HirImportSource};
use zutai_syntax::Span;
use zutai_thir::{ImportKey, ImportedField, ImportedType};

use crate::{Analysis, AnalysisOptions};

/// Recursion state shared across a single top-level analysis: the stack of
/// modules currently being analyzed (for cycle detection) and a cache of
/// already-analyzed `.zt` modules keyed by canonical path.
#[derive(Default)]
pub(crate) struct ImportContext {
    in_progress: Vec<PathBuf>,
    cache: HashMap<PathBuf, Rc<Analysis>>,
}

impl ImportContext {
    /// Seed the in-progress stack with the root file's canonical path so that a
    /// descendant importing the root is detected as a cycle.
    pub(crate) fn with_root(path: &Path) -> Self {
        let mut ctx = Self::default();
        if let Ok(canonical) = std::fs::canonicalize(path) {
            ctx.in_progress.push(canonical);
        }
        ctx
    }
}

/// Everything resolved for a single file's imports.
pub(crate) struct ResolvedImports {
    /// Structural types, keyed by import source — fed into THIR lowering.
    pub types: HashMap<ImportKey, ImportedType>,
    /// Parsed `.zti` values, keyed by import source — consumed by the evaluator.
    pub values: HashMap<ImportKey, zutai_im::Value>,
    /// Analyzed `.zt` sub-modules, keyed by import source — evaluated recursively.
    pub modules: HashMap<ImportKey, Rc<Analysis>>,
    pub diagnostics: Vec<ImportDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDiagnostic {
    pub kind: ImportDiagnosticKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportDiagnosticKind {
    /// An `import` appeared but the analysis has no base directory to resolve
    /// it against (e.g. `analyze(&str)` / REPL rather than `analyze_path`).
    NoBaseDirectory,
    /// Import path could not be turned into a supported file reference.
    UnsupportedImportForm {
        path: String,
    },
    FileNotFound {
        path: String,
    },
    ReadError {
        path: String,
        msg: String,
    },
    /// A `.zti` file failed to parse.
    ParseError {
        path: String,
        msg: String,
    },
    /// A `.zt` module imports (transitively) itself.
    ImportCycle {
        path: String,
    },
    /// A `.zt` module did not fully type-check, so it has no exportable value.
    ModuleHasErrors {
        path: String,
    },
    /// A `.zt` module's value cannot cross the import boundary (e.g. it is or
    /// contains a function or type value).
    UnsupportedExport {
        path: String,
        reason: &'static str,
    },
}

enum Kind {
    Zti,
    Zt,
}

struct Resolver<'a> {
    base: Option<&'a Path>,
    types: HashMap<ImportKey, ImportedType>,
    values: HashMap<ImportKey, zutai_im::Value>,
    modules: HashMap<ImportKey, Rc<Analysis>>,
    diagnostics: Vec<ImportDiagnostic>,
}

/// Resolve every distinct `import` in `hir` relative to `base`.
pub(crate) fn resolve_imports(
    hir: &HirFile,
    base: Option<&Path>,
    ctx: &mut ImportContext,
) -> ResolvedImports {
    let mut resolver = Resolver {
        base,
        types: HashMap::new(),
        values: HashMap::new(),
        modules: HashMap::new(),
        diagnostics: Vec::new(),
    };

    // Resolve each distinct source once, using the first span seen for diagnostics.
    let mut seen: HashSet<&HirImportSource> = HashSet::new();
    for (_, expr) in hir.expr_arena.iter() {
        let HirExprKind::Import(source) = &expr.kind else {
            continue;
        };
        if seen.insert(source) {
            resolver.resolve_one(source, expr.span, ctx);
        }
    }

    ResolvedImports {
        types: resolver.types,
        values: resolver.values,
        modules: resolver.modules,
        diagnostics: resolver.diagnostics,
    }
}

impl Resolver<'_> {
    fn resolve_one(&mut self, source: &HirImportSource, span: Span, ctx: &mut ImportContext) {
        let rel = match relative_path(source) {
            Ok(rel) => rel,
            Err(kind) => return self.diag(kind, span),
        };

        let kind = match Path::new(&rel)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("zti") => Kind::Zti,
            Some("zt") => Kind::Zt,
            _ => {
                return self.diag(
                    ImportDiagnosticKind::UnsupportedImportForm { path: rel },
                    span,
                );
            }
        };

        let Some(base) = self.base else {
            return self.diag(ImportDiagnosticKind::NoBaseDirectory, span);
        };

        // `canonicalize` requires the file to exist, which doubles as the
        // not-found check and dedupes symlinks to one resolved path.
        let canonical = match std::fs::canonicalize(base.join(&rel)) {
            Ok(canonical) => canonical,
            Err(_) => return self.diag(ImportDiagnosticKind::FileNotFound { path: rel }, span),
        };

        match kind {
            Kind::Zti => self.resolve_zti(source, &canonical, &rel, span),
            Kind::Zt => self.resolve_zt(source, &canonical, &rel, span, ctx),
        }
    }

    fn resolve_zti(&mut self, source: &HirImportSource, canonical: &Path, rel: &str, span: Span) {
        let contents = match std::fs::read_to_string(canonical) {
            Ok(contents) => contents,
            Err(err) => return self.read_error(rel, &err, span),
        };
        match zutai_im::parse(&contents) {
            Ok(block) => {
                let value = zutai_im::Value::Block(block);
                let ty = imported_type(&value);
                self.types.insert(source.clone(), ty);
                self.values.insert(source.clone(), value);
            }
            Err(err) => self.diag(
                ImportDiagnosticKind::ParseError {
                    path: rel.to_string(),
                    msg: err.to_string(),
                },
                span,
            ),
        }
    }

    fn resolve_zt(
        &mut self,
        source: &HirImportSource,
        canonical: &Path,
        rel: &str,
        span: Span,
        ctx: &mut ImportContext,
    ) {
        if ctx.in_progress.iter().any(|p| p == canonical) {
            return self.diag(
                ImportDiagnosticKind::ImportCycle {
                    path: rel.to_string(),
                },
                span,
            );
        }

        let module = match ctx.cache.get(canonical) {
            Some(module) => module.clone(),
            None => {
                let contents = match std::fs::read_to_string(canonical) {
                    Ok(contents) => contents,
                    Err(err) => return self.read_error(rel, &err, span),
                };
                ctx.in_progress.push(canonical.to_path_buf());
                let analysis = crate::analyze_inner(
                    &contents,
                    canonical.parent(),
                    AnalysisOptions::default(),
                    ctx,
                );
                ctx.in_progress.pop();

                if !analysis.is_thir_complete() {
                    // A cycle is first detected on the back-edge, one module
                    // deeper; propagate it so every level on the chain reports
                    // the cycle rather than a vague "module has errors".
                    let kind = if contains_cycle(&analysis) {
                        ImportDiagnosticKind::ImportCycle {
                            path: rel.to_string(),
                        }
                    } else {
                        ImportDiagnosticKind::ModuleHasErrors {
                            path: rel.to_string(),
                        }
                    };
                    return self.diag(kind, span);
                }
                let module = Rc::new(analysis);
                ctx.cache.insert(canonical.to_path_buf(), module.clone());
                module
            }
        };

        // Type the import by exporting the module's final-expression type.
        let exported = {
            let Some(file) = module.thir.as_ref().and_then(|thir| thir.file.as_ref()) else {
                return self.diag(
                    ImportDiagnosticKind::ModuleHasErrors {
                        path: rel.to_string(),
                    },
                    span,
                );
            };
            let final_ty = file.expr_arena[file.final_expr].ty;
            zutai_thir::export_type(file, final_ty)
        };

        match exported {
            Ok(ty) => {
                self.types.insert(source.clone(), ty);
                self.modules.insert(source.clone(), module);
            }
            Err(unsupported) => self.diag(
                ImportDiagnosticKind::UnsupportedExport {
                    path: rel.to_string(),
                    reason: unsupported.reason,
                },
                span,
            ),
        }
    }

    fn read_error(&mut self, rel: &str, err: &std::io::Error, span: Span) {
        self.diag(
            ImportDiagnosticKind::ReadError {
                path: rel.to_string(),
                msg: err.to_string(),
            },
            span,
        );
    }

    fn diag(&mut self, kind: ImportDiagnosticKind, span: Span) {
        self.diagnostics.push(ImportDiagnostic { kind, span });
    }
}

/// Whether `analysis` failed (at least in part) because of an import cycle.
fn contains_cycle(analysis: &Analysis) -> bool {
    analysis.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            crate::SemanticDiagnosticKind::Import(import)
                if matches!(import.kind, ImportDiagnosticKind::ImportCycle { .. })
        )
    })
}

/// Turn an import source into a relative path string.
fn relative_path(source: &HirImportSource) -> Result<String, ImportDiagnosticKind> {
    match source {
        HirImportSource::String(value) => Ok(value.clone()),
        // Bare shorthand `import config.zti` lexes to `["config", "zti"]`; only
        // the simple `stem.ext` form is resolved.  Anything else falls back to
        // the canonical quoted string form.
        HirImportSource::Path(parts) if parts.len() == 2 => {
            Ok(format!("{}.{}", parts[0], parts[1]))
        }
        HirImportSource::Path(parts) => Err(ImportDiagnosticKind::UnsupportedImportForm {
            path: parts.join("."),
        }),
    }
}

/// Derive the structural type of an imported `.zti` data value.
///
/// Blocks become records (all fields required), arrays become lists, atoms keep
/// their spelling.  A heterogeneous array yields a union of its distinct element
/// types; an empty array yields `Unknown` (a fresh inference variable in THIR).
fn imported_type(value: &zutai_im::Value) -> ImportedType {
    use zutai_im::Value;
    match value {
        Value::True | Value::False => ImportedType::Bool,
        Value::Integer(_) => ImportedType::Int,
        Value::Float(_) => ImportedType::Float,
        Value::String(_) => ImportedType::Text,
        Value::Atom(name) => ImportedType::Atom(name.clone()),
        Value::Block(block) => ImportedType::Record(
            block
                .iter()
                .map(|pair| ImportedField {
                    name: pair.field_name.clone(),
                    optional: false,
                    ty: imported_type(&pair.value),
                })
                .collect(),
        ),
        Value::Array(items) => ImportedType::List(Box::new(array_element_type(items))),
    }
}

fn array_element_type(items: &[zutai_im::Value]) -> ImportedType {
    let mut distinct: Vec<ImportedType> = Vec::new();
    for item in items {
        let ty = imported_type(item);
        if !distinct.contains(&ty) {
            distinct.push(ty);
        }
    }
    match distinct.len() {
        0 => ImportedType::Unknown,
        1 => distinct.pop().unwrap(),
        _ => ImportedType::Union(distinct),
    }
}
