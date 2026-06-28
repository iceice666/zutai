//! Module loader for imports.
//!
//! THIR lowering is pure, so all filesystem work happens here: walk the HIR for
//! import declarations (represented internally as `Import` expression nodes),
//! resolve each path relative to the importing file's directory, and produce
//! both a structural type (for THIR) and the data needed by the evaluator.
//!
//! - `.zti` (immediate data): parse the file and keep the parsed value; its type
//!   is derived structurally.
//! - `.zt` (module): recursively analyze the file, type the import by its final
//!   expression's exported type, and keep the analyzed sub-module so the
//!   evaluator can evaluate it.  Import cycles are detected and reported.
//!
//! Functions cross module boundaries via home-module handles stamped by the
//! evaluator.  Type-valued fields carry their denotation in `ImportedType::Type`
//! so annotation-position access (`x : serverLib.Server`) type-checks.

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use zutai_hir::{BindingId, HirExprKind, HirFile, HirImportSource};
use zutai_syntax::Span;
use zutai_thir::{
    ImportKey, ImportedField, ImportedTupleItem, ImportedType, ThirDeclKind, ThirExprKind,
    ThirFile, WitnessPattern, export_witness_pattern,
};

use crate::{Analysis, AnalysisOptions};

/// Recursion state shared across a single top-level analysis: the stack of
/// modules currently being analyzed (for cycle detection) and a cache of
/// already-analyzed `.zt` modules keyed by canonical path.
#[derive(Default)]
pub(crate) struct ImportContext {
    in_progress: Vec<PathBuf>,
    cache: FxHashMap<PathBuf, Rc<Analysis>>,
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
    pub types: FxHashMap<ImportKey, ImportedType>,
    /// Parsed `.zti` values, keyed by import source — consumed by the evaluator.
    pub values: FxHashMap<ImportKey, zutai_im::Value>,
    /// Analyzed `.zt` sub-modules, keyed by import source — evaluated recursively.
    pub modules: FxHashMap<ImportKey, Rc<Analysis>>,
    /// Witnesses exported by imported `.zt` modules, including re-exports.
    pub witnesses: Vec<WitnessExport>,
    pub diagnostics: Vec<ImportDiagnostic>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessExport {
    pub origin: PathBuf,
    pub constraint: String,
    pub target_key: String,
    pub target_display: String,
    /// HIR BindingId.0 of this witness instance's own binding.
    /// Used by the native backend to compute the dep-namespaced DC global name
    /// (`$dep{idx}${constraint}$w{binding_id}`) for cross-module witness dispatch.
    pub binding_id: u32,
    pub span: Span,
    /// For a parametric (conditional) witness such as `Eq @(List A) :: <A: Eq>`,
    /// the structural matcher plus per-parameter component-constraint names. `None`
    /// for a concrete witness (its `target_key` is `?`-free and matches directly).
    pub conditional: Option<ConditionalWitnessShape>,
}

/// Cross-module dispatch data for a conditional witness: the target shape with
/// parameter holes and, parallel to the holes, the component constraints each
/// hole's type must satisfy (`<A: Eq>` → `[["Eq"]]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalWitnessShape {
    pub pattern: WitnessPattern,
    pub param_bounds: Vec<Vec<String>>,
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
    /// `import stdlib.<name>` named a module the embedded standard library does
    /// not provide.
    UnknownStdlibModule {
        name: String,
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
    /// Two distinct imported witnesses claim the same `(Constraint, Type)` pair.
    ConflictingWitness {
        constraint: String,
        target: String,
    },
    /// An import path is absolute or escapes the importing file's directory
    /// subtree (e.g. `"/tmp/x.zti"` or `"../../../etc/foo.zti"`).
    PathTraversal {
        path: String,
    },
}

enum Kind {
    Zti,
    Zt,
}

struct Resolver<'a> {
    base: Option<&'a Path>,
    types: FxHashMap<ImportKey, ImportedType>,
    values: FxHashMap<ImportKey, zutai_im::Value>,
    modules: FxHashMap<ImportKey, Rc<Analysis>>,
    witnesses: Vec<WitnessExport>,
    witness_keys: FxHashMap<(String, String), PathBuf>,
    diagnostics: Vec<ImportDiagnostic>,
}

/// Resolve every distinct import declaration/internal import node in `hir` relative to `base`.
pub(crate) fn resolve_imports(
    hir: &HirFile,
    base: Option<&Path>,
    ctx: &mut ImportContext,
) -> ResolvedImports {
    let mut resolver = Resolver {
        base,
        types: FxHashMap::default(),
        values: FxHashMap::default(),
        modules: FxHashMap::default(),
        witnesses: Vec::new(),
        witness_keys: FxHashMap::default(),
        diagnostics: Vec::new(),
    };

    // Resolve each distinct source once, using the first span seen for diagnostics.
    let mut seen: FxHashSet<&HirImportSource> = FxHashSet::default();
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
        witnesses: resolver.witnesses,
        diagnostics: resolver.diagnostics,
    }
}

impl Resolver<'_> {
    fn resolve_one(&mut self, source: &HirImportSource, span: Span, ctx: &mut ImportContext) {
        // `import stdlib.<name>` resolves to an embedded module (no filesystem,
        // no install path, no subtree-confinement check). This is checked before
        // `relative_path` so `stdlib.stream` is not mistaken for `stem.ext`.
        if let HirImportSource::Path(parts) = source
            && parts.first().map(String::as_str) == Some("stdlib")
        {
            return self.resolve_stdlib(source, parts, span, ctx);
        }

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

        let base_dir = if base.as_os_str().is_empty() {
            Path::new(".")
        } else {
            base
        };
        let canonical_base = match std::fs::canonicalize(base_dir) {
            Ok(canonical_base) => canonical_base,
            Err(_) => return self.diag(ImportDiagnosticKind::FileNotFound { path: rel }, span),
        };

        // `canonicalize` requires the file to exist, which doubles as the
        // not-found check and dedupes symlinks to one resolved path.
        let canonical = match std::fs::canonicalize(base_dir.join(&rel)) {
            Ok(canonical) => canonical,
            Err(_) => return self.diag(ImportDiagnosticKind::FileNotFound { path: rel }, span),
        };

        // Confine the resolved path to the importing file's directory subtree.
        // `starts_with` is component-wise, so `/proj-evil` is not a prefix of `/proj`.
        // Symlinks are already resolved by `canonicalize` above, so symlink escapes
        // are also rejected.
        if !canonical.starts_with(&canonical_base) {
            return self.diag(ImportDiagnosticKind::PathTraversal { path: rel }, span);
        }

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
                match self.analyze_zt(canonical, canonical.parent(), &contents, rel, span, ctx) {
                    Some(module) => module,
                    None => return,
                }
            }
        };

        self.register_zt_module(source, module, rel, span);
    }

    /// `import stdlib.<name>` — resolve `<name>` against the embedded standard
    /// library and analyze it from in-binary source. Uses a synthetic cache key
    /// (`<stdlib>/<name>.zt`) so cycle detection and caching still apply without
    /// touching the filesystem.
    fn resolve_stdlib(
        &mut self,
        source: &HirImportSource,
        parts: &[String],
        span: Span,
        ctx: &mut ImportContext,
    ) {
        let name = match parts {
            [_, name] => name.as_str(),
            _ => {
                return self.diag(
                    ImportDiagnosticKind::UnsupportedImportForm {
                        path: parts.join("."),
                    },
                    span,
                );
            }
        };
        let Some(contents) = stdlib_source(name) else {
            return self.diag(
                ImportDiagnosticKind::UnknownStdlibModule {
                    name: name.to_string(),
                },
                span,
            );
        };

        let key = PathBuf::from("<stdlib>").join(format!("{name}.zt"));
        let rel = format!("stdlib.{name}");
        if ctx.in_progress.iter().any(|p| p == &key) {
            return self.diag(ImportDiagnosticKind::ImportCycle { path: rel }, span);
        }
        let module = match ctx.cache.get(&key) {
            Some(module) => module.clone(),
            None => match self.analyze_zt(&key, key.parent(), contents, &rel, span, ctx) {
                Some(module) => module,
                None => return,
            },
        };

        self.register_zt_module(source, module, &rel, span);
    }

    /// Recursively analyze a `.zt` module's source into a cached `Analysis`,
    /// pushing a diagnostic and returning `None` on cycle or module errors.
    /// `key` is the cache/cycle identity (a real canonical path or a synthetic
    /// stdlib key); `parent` is the directory used to resolve the module's own
    /// relative imports.
    fn analyze_zt(
        &mut self,
        key: &Path,
        parent: Option<&Path>,
        contents: &str,
        rel: &str,
        span: Span,
        ctx: &mut ImportContext,
    ) -> Option<Rc<Analysis>> {
        ctx.in_progress.push(key.to_path_buf());
        let analysis =
            crate::analyze_inner(contents, parent, Some(key), AnalysisOptions::default(), ctx);
        ctx.in_progress.pop();

        if analysis.blocking_diagnostics().next().is_some() || !analysis.is_thir_complete() {
            // A cycle is first detected on the back-edge, one module deeper;
            // propagate it so every level on the chain reports the cycle rather
            // than a vague "module has errors".
            let kind = if contains_cycle(&analysis) {
                ImportDiagnosticKind::ImportCycle {
                    path: rel.to_string(),
                }
            } else {
                ImportDiagnosticKind::ModuleHasErrors {
                    path: rel.to_string(),
                }
            };
            self.diag(kind, span);
            return None;
        }
        let module = Rc::new(analysis);
        ctx.cache.insert(key.to_path_buf(), module.clone());
        Some(module)
    }

    /// Type a resolved `.zt` module by its exported (final-expression) type and
    /// register it under `source` for THIR lowering and evaluation.
    fn register_zt_module(
        &mut self,
        source: &HirImportSource,
        module: Rc<Analysis>,
        rel: &str,
        span: Span,
    ) {
        // Type the import by exporting the module's final-expression type,
        // then enrich type-valued record fields with their denotations.
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
            zutai_thir::export_type(file, final_ty).map(|ty| enrich_with_type_denotations(ty, file))
        };

        match exported {
            Ok(ty) => {
                self.merge_witnesses(&module.witness_exports, span);
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

    fn merge_witnesses(&mut self, witnesses: &[WitnessExport], span: Span) {
        for witness in witnesses {
            let key = (witness.constraint.clone(), witness.target_key.clone());
            match self.witness_keys.get(&key) {
                Some(origin) if origin != &witness.origin => {
                    self.diag(
                        ImportDiagnosticKind::ConflictingWitness {
                            constraint: witness.constraint.clone(),
                            target: witness.target_display.clone(),
                        },
                        span,
                    );
                }
                Some(_) => {}
                None => {
                    self.witness_keys.insert(key, witness.origin.clone());
                    self.witnesses.push(witness.clone());
                }
            }
        }
    }
}

/// Embedded standard-library modules, addressed as `import stdlib.<name>`.
///
/// Resolved from in-binary source so there is no filesystem stdlib root or
/// install path. `stream` and `prelude` share their source with the ambient
/// prelude (`zutai_hir::STREAM_MODULE_SRC` / `PRELUDE_MODULE_SRC`), while
/// `optional`, `result`, `num`, `text`, and `cmp` are explicit-import-only.
fn stdlib_source(name: &str) -> Option<&'static str> {
    match name {
        "stream" => Some(zutai_hir::STREAM_MODULE_SRC),
        "prelude" => Some(zutai_hir::PRELUDE_MODULE_SRC),
        "optional" => Some(zutai_hir::OPTIONAL_MODULE_SRC),
        "result" => Some(zutai_hir::RESULT_MODULE_SRC),
        "num" => Some(zutai_hir::NUM_MODULE_SRC),
        "text" => Some(zutai_hir::TEXT_MODULE_SRC),
        "cmp" => Some(zutai_hir::CMP_MODULE_SRC),
        _ => None,
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

pub(crate) fn merge_witness_exports(
    imported: Vec<WitnessExport>,
    local: Vec<WitnessExport>,
) -> (Vec<WitnessExport>, Vec<ImportDiagnostic>) {
    let mut merged = Vec::new();
    let mut diagnostics = Vec::new();
    let mut keys: FxHashMap<(String, String), PathBuf> = FxHashMap::default();
    for witness in imported.into_iter().chain(local) {
        let key = (witness.constraint.clone(), witness.target_key.clone());
        match keys.get(&key) {
            Some(origin) if origin != &witness.origin => {
                diagnostics.push(ImportDiagnostic {
                    kind: ImportDiagnosticKind::ConflictingWitness {
                        constraint: witness.constraint.clone(),
                        target: witness.target_display.clone(),
                    },
                    span: witness.span,
                });
            }
            Some(_) => {}
            None => {
                keys.insert(key, witness.origin.clone());
                merged.push(witness);
            }
        }
    }
    (merged, diagnostics)
}

pub(crate) fn local_witness_exports(
    hir: &HirFile,
    file: &ThirFile,
    origin: &Path,
) -> Vec<WitnessExport> {
    let mut out = Vec::new();
    for (_, decl) in file.decl_arena.iter() {
        let ThirDeclKind::Witness {
            constraint: Some(constraint),
            target,
            params,
            param_bounds,
            ..
        } = &decl.kind
        else {
            continue;
        };
        let Ok(exported_target) = zutai_thir::export_type(file, *target) else {
            continue;
        };
        let constraint = binding_name(hir, *constraint).to_string();
        let target_key = imported_type_key(&exported_target);
        // A parametric witness carries its type params as holes; record the
        // structural matcher and per-param component constraints so an importer
        // can dispatch it at a concrete call site (Phase B). A concrete witness
        // (`params` empty) needs none — its `target_key` matches directly.
        let conditional = if params.is_empty() {
            None
        } else {
            export_witness_pattern(file, *target, params).map(|pattern| {
                let param_bounds = param_bounds
                    .iter()
                    .map(|bounds| {
                        bounds
                            .iter()
                            .map(|b| binding_name(hir, *b).to_string())
                            .collect()
                    })
                    .collect();
                ConditionalWitnessShape {
                    pattern,
                    param_bounds,
                }
            })
        };
        out.push(WitnessExport {
            origin: origin.to_path_buf(),
            constraint,
            target_display: target_key.clone(),
            target_key,
            binding_id: decl.binding.0,
            span: decl.span,
            conditional,
        });
    }
    out
}

fn binding_name(hir: &HirFile, binding: BindingId) -> &str {
    &hir.bindings[binding.0 as usize].name
}

fn imported_type_key(ty: &ImportedType) -> String {
    match ty {
        ImportedType::Bool => "Bool".to_string(),
        ImportedType::Int => "Int".to_string(),
        ImportedType::Float => "Float".to_string(),
        ImportedType::FixedNum(fw) => fw.name().to_string(),
        ImportedType::Posit(spec) => spec.type_name(),
        ImportedType::Text => "Text".to_string(),
        ImportedType::Atom(name) => format!("#{name}"),
        ImportedType::List(inner) => format!("[{}]", imported_type_key(inner)),
        ImportedType::Optional(inner) => format!("{}?", imported_type_key(inner)),
        ImportedType::Maybe(inner) => format!("Maybe[{}]", imported_type_key(inner)),
        ImportedType::Record(fields) => {
            let mut parts: Vec<String> = fields
                .iter()
                .map(|field| {
                    let marker = if field.optional { "?:" } else { ":" };
                    format!("{}{}{}", field.name, marker, imported_type_key(&field.ty))
                })
                .collect();
            parts.sort();
            format!("{{{}}}", parts.join(","))
        }
        ImportedType::Tuple(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| match item {
                    ImportedTupleItem::Named { name, ty } => {
                        format!("{}:{}", name, imported_type_key(ty))
                    }
                    ImportedTupleItem::Positional(ty) => imported_type_key(ty),
                })
                .collect();
            format!("({})", parts.join(","))
        }
        ImportedType::Union(variants) => {
            let parts: Vec<String> = variants
                .iter()
                .map(|variant| match &variant.payload {
                    Some(payload) => format!("{}({})", variant.name, imported_type_key(payload)),
                    None => variant.name.clone(),
                })
                .collect();
            format!("<{}>", parts.join("|"))
        }
        ImportedType::Function { from, to } => {
            format!("({}->{})", imported_type_key(from), imported_type_key(to))
        }
        ImportedType::Type(inner) => format!("Type({})", imported_type_key(inner)),
        ImportedType::TypeCon { params, body } => {
            let ps: Vec<String> = params.iter().map(|id| format!("'{id}")).collect();
            format!("\\<{}>{}", ps.join(","), imported_type_key(body))
        }
        ImportedType::ConApply { ctor, args } => {
            let parts: Vec<String> = args.iter().map(imported_type_key).collect();
            format!("{ctor}[{}]", parts.join(","))
        }
        ImportedType::TyVar(id) => format!("'{id}"),
        ImportedType::Unknown => "?".to_string(),
    }
}

/// Turn an import source into a relative path string.
fn relative_path(source: &HirImportSource) -> Result<String, ImportDiagnosticKind> {
    match source {
        HirImportSource::String(value) => {
            if Path::new(value).is_absolute() {
                return Err(ImportDiagnosticKind::PathTraversal {
                    path: value.clone(),
                });
            }
            Ok(value.clone())
        }
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
        // Heterogeneous arrays have no meaningful tag names for the variants,
        // so fall back to Unknown and let the consumer unify with what it needs.
        _ => ImportedType::Unknown,
    }
}

/// Enrich `ImportedType::Type` placeholders with their concrete denotations
/// recovered from the module's final expression.
///
/// `export_type` converts a bare `TypeKind::Type` slot (which is payload-less)
/// to `ImportedType::Type(Unknown)`. This function upgrades those placeholders
/// by walking the module's final-expression AST. For each record field whose
/// THIR value is a `TypeValue(tid)`, and for a direct type-valued final
/// expression, it calls `export_type(file, tid)` to obtain the real denotation.
///
/// Non-type final expressions (scalars, functions, …) are returned as-is.
fn enrich_with_type_denotations(ty: ImportedType, file: &ThirFile) -> ImportedType {
    let final_expr = &file.expr_arena[file.final_expr];
    match ty {
        ImportedType::Type(_) => {
            if let ThirExprKind::TypeValue(denotation_tid) = final_expr.kind
                && let Ok(denotation) = zutai_thir::export_type_value(file, denotation_tid)
            {
                ImportedType::Type(Box::new(denotation))
            } else {
                ImportedType::Type(Box::new(ImportedType::Unknown))
            }
        }
        ImportedType::Record(mut fields) => {
            let ThirExprKind::Record(thir_fields) = &final_expr.kind else {
                return ImportedType::Record(fields);
            };

            for thir_field in thir_fields {
                // Only enrich fields that are already `Type(Unknown)` placeholders.
                let Some(imp_field) = fields.iter_mut().find(|f| f.name == thir_field.name) else {
                    continue;
                };
                if !matches!(imp_field.ty, ImportedType::Type(_)) {
                    continue;
                }
                // The THIR field value must be a TypeValue to carry a denotation.
                let value_expr = &file.expr_arena[thir_field.value];
                if let ThirExprKind::TypeValue(denotation_tid) = value_expr.kind
                    && let Ok(denotation) = zutai_thir::export_type_value(file, denotation_tid)
                {
                    imp_field.ty = ImportedType::Type(Box::new(denotation));
                }
            }

            ImportedType::Record(fields)
        }
        other => other,
    }
}
