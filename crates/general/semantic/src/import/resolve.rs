use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use zutai_hir::{BindingId, HirExprKind, HirFile, HirImportSource};
use zutai_syntax::Span;
use zutai_thir::{ImportKey, ImportedProvenance, ImportedType};

use crate::Analysis;
use crate::cache::{
    CacheDependency, CacheDependencyKind, CacheDependencySource, Fingerprint, fingerprint_text,
};

use super::*;

/// Everything resolved for a single file's imports.
pub(crate) struct ResolvedImports {
    /// Structural types, keyed by import source — fed into THIR lowering.
    pub types: FxHashMap<ImportKey, ImportedType>,
    pub provenance: FxHashMap<ImportKey, ImportedProvenance>,
    /// Parsed `.zti` values, keyed by import source — consumed by the evaluator.
    pub values: FxHashMap<ImportKey, zutai_im::Value>,
    /// Analyzed `.zt` sub-modules, keyed by import source — evaluated recursively.
    pub modules: FxHashMap<ImportKey, Rc<Analysis>>,
    /// Witnesses exported by imported `.zt` modules, including re-exports.
    pub witnesses: Vec<WitnessExport>,
    /// Source spans for each distinct import expression.
    pub sites: FxHashMap<ImportKey, Span>,
    pub diagnostics: Vec<ImportDiagnostic>,
}

pub(crate) enum Kind {
    Zti,
    Zt,
}

pub(crate) struct Resolver<'a> {
    base: Option<&'a Path>,
    types: FxHashMap<ImportKey, ImportedType>,
    provenance: FxHashMap<ImportKey, ImportedProvenance>,
    values: FxHashMap<ImportKey, zutai_im::Value>,
    modules: FxHashMap<ImportKey, Rc<Analysis>>,
    witnesses: Vec<WitnessExport>,
    witness_keys: FxHashMap<(String, String), PathBuf>,
    sites: FxHashMap<ImportKey, Span>,
    diagnostics: Vec<ImportDiagnostic>,
}

/// Resolve every distinct import declaration/internal import node in `hir` relative to `base`.
pub(crate) fn resolve_imports(
    hir: &HirFile,
    base: Option<&Path>,
    ctx: &mut ImportContext<'_>,
) -> ResolvedImports {
    let mut resolver = Resolver {
        base,
        types: FxHashMap::default(),
        provenance: FxHashMap::default(),
        values: FxHashMap::default(),
        modules: FxHashMap::default(),
        witnesses: Vec::new(),
        witness_keys: FxHashMap::default(),
        sites: FxHashMap::default(),
        diagnostics: Vec::new(),
    };

    if let Some(error) = ctx.take_package_setup_error() {
        let span = error.span.unwrap_or_else(|| {
            hir.expr_arena
                .iter()
                .find_map(|(_, expr)| {
                    matches!(expr.kind, HirExprKind::Import(_)).then_some(expr.span)
                })
                .unwrap_or_default()
        });
        resolver.diagnostics.push(ImportDiagnostic {
            kind: ImportDiagnosticKind::PackageSetup {
                message: error.message,
            },
            span,
            path: error.path,
            related: error.related,
        });
    }

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
        provenance: resolver.provenance,
        values: resolver.values,
        modules: resolver.modules,
        witnesses: resolver.witnesses,
        sites: resolver.sites,
        diagnostics: resolver.diagnostics,
    }
}

impl Resolver<'_> {
    fn resolve_one(&mut self, source: &HirImportSource, span: Span, ctx: &mut ImportContext) {
        self.sites.entry(source.clone()).or_insert(span);
        // `import stdlib.<name>` resolves from the validated stdlib source set
        // (outside quoted-import subtree confinement). This is checked before
        // `relative_path` so `stdlib.stream` is not mistaken for `stem.ext`.
        if let HirImportSource::Path(parts) = source
            && parts.first().map(String::as_str) == Some("stdlib")
        {
            return self.resolve_stdlib(source, parts, span, ctx);
        }

        if ctx.packages.invalid_error().is_some() {
            return;
        }

        if let HirImportSource::Path(parts) = source
            && parts.len() >= 2
            && !matches!(parts.as_slice(), [_, extension] if matches!(extension.as_str(), "zt" | "zti"))
        {
            match ctx.packages.resolve(self.base, parts) {
                Ok(Some(loaded)) => {
                    ctx.packages.record_source(
                        &mut ctx.recorded_packages,
                        &loaded.key,
                        &loaded.contents,
                    );
                    if let Some(path) = loaded.filesystem_path.as_ref() {
                        let stable = ctx
                            .packages
                            .stable_source_key(path)
                            .unwrap_or_else(|| loaded.key.clone());
                        ctx.recorded_source_paths.insert(stable, path.clone());
                    }
                    let rel = loaded.display.clone();
                    return self.resolve_zt(
                        source,
                        LoadedSource {
                            key: loaded.key,
                            contents: loaded.contents,
                        },
                        &rel,
                        span,
                        CacheDependencySource::Package {
                            importer: self.base.map(Path::to_path_buf),
                            parts: parts.to_vec(),
                        },
                        ctx,
                    );
                }
                Ok(None) => {}
                Err(error) => {
                    return self.diag(
                        ImportDiagnosticKind::PackageResolution {
                            path: parts.join("."),
                            message: error.to_string(),
                        },
                        span,
                    );
                }
            }
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

        let loaded = match ctx.load(base, &rel) {
            Ok(loaded) => loaded,
            Err(LoadError::NotFound) => {
                return self.diag(ImportDiagnosticKind::FileNotFound { path: rel }, span);
            }
            Err(LoadError::Traversal) => {
                return self.diag(ImportDiagnosticKind::PathTraversal { path: rel }, span);
            }
            Err(LoadError::Read(err)) => return self.read_error(&rel, &err, span),
        };
        if let Ok(path) = std::fs::canonicalize(&loaded.key) {
            ctx.recorded_source_paths.insert(loaded.key.clone(), path);
        }

        match kind {
            Kind::Zti => self.resolve_zti(
                source,
                &loaded.contents,
                &rel,
                span,
                CacheDependencySource::Relative {
                    base: base.to_path_buf(),
                    path: rel.clone(),
                },
                ctx,
            ),
            Kind::Zt => self.resolve_zt(
                source,
                loaded,
                &rel,
                span,
                CacheDependencySource::Relative {
                    base: base.to_path_buf(),
                    path: rel.clone(),
                },
                ctx,
            ),
        }
    }

    fn resolve_zti(
        &mut self,
        source: &HirImportSource,
        contents: &str,
        rel: &str,
        span: Span,
        dependency_source: CacheDependencySource,
        ctx: &mut ImportContext<'_>,
    ) {
        match zutai_im::parse_located(contents) {
            Ok(block) => {
                ctx.record_dependency(CacheDependency {
                    source: dependency_source,
                    kind: CacheDependencyKind::Data,
                    fingerprint: fingerprint_text(contents),
                });
                let provenance = block_provenance(&block);
                let value = zutai_im::Value::Block(block.value);
                let ty = imported_type(&value);
                self.types.insert(source.clone(), ty);
                self.provenance.insert(source.clone(), provenance);
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
        loaded: LoadedSource,
        rel: &str,
        span: Span,
        dependency_source: CacheDependencySource,
        ctx: &mut ImportContext,
    ) {
        if ctx.in_progress.iter().any(|p| p == &loaded.key) {
            let related = ctx
                .in_progress
                .iter()
                .skip_while(|path| *path != &loaded.key)
                .map(|path| crate::SourceLocation {
                    path: path.clone(),
                    span: Span::default(),
                    label: "module in this import cycle".to_owned(),
                })
                .chain(std::iter::once(crate::SourceLocation {
                    path: loaded.key.clone(),
                    span: Span::default(),
                    label: "cycle returns to this module".to_owned(),
                }))
                .collect();
            return self.diag_with_related(
                ImportDiagnosticKind::ImportCycle {
                    path: rel.to_string(),
                },
                span,
                related,
            );
        }

        let (module, fingerprint) = match ctx.cache.get(&loaded.key) {
            Some(module) => {
                let fingerprint = ctx
                    .cached_fingerprint(&loaded.key)
                    .expect("session cache entries carry fingerprints");
                (module.clone(), fingerprint)
            }
            None => match ctx.try_cached_module(&loaded.key, &loaded.contents) {
                Some(cached) => cached,
                None => match self.analyze_zt(
                    &loaded.key,
                    loaded.key.parent(),
                    &loaded.contents,
                    rel,
                    span,
                    ctx,
                ) {
                    Some(cached) => cached,
                    None => return,
                },
            },
        };
        ctx.record_dependency(CacheDependency {
            source: dependency_source,
            kind: CacheDependencyKind::Module,
            fingerprint,
        });

        self.register_zt_module(source, module, rel, span);
    }

    /// `import stdlib.<name>` — resolve `<name>` against the configured standard
    /// library source set. Uses a synthetic cache key
    /// (`<stdlib>/<name>.zt`) so cycle detection and caching still apply without
    /// touching the filesystem.
    fn resolve_stdlib(
        &mut self,
        source: &HirImportSource,
        parts: &[String],
        span: Span,
        ctx: &mut ImportContext<'_>,
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
        let Some(contents) = ctx.stdlib.source(name).map(str::to_owned) else {
            return self.diag(
                ImportDiagnosticKind::UnknownStdlibModule {
                    name: name.to_string(),
                },
                span,
            );
        };

        ctx.recorded_stdlib
            .insert(name.to_owned(), contents.clone());
        let key = PathBuf::from("<stdlib>").join(format!("{name}.zt"));
        let rel = format!("stdlib.{name}");
        if ctx.in_progress.iter().any(|p| p == &key) {
            return self.diag(ImportDiagnosticKind::ImportCycle { path: rel }, span);
        }
        let (module, fingerprint) = match ctx.cache.get(&key) {
            Some(module) => {
                let fingerprint = ctx
                    .cached_fingerprint(&key)
                    .expect("session cache entries carry fingerprints");
                (module.clone(), fingerprint)
            }
            None => match ctx.try_cached_module(&key, &contents) {
                Some(cached) => cached,
                None => match self.analyze_zt(&key, key.parent(), &contents, &rel, span, ctx) {
                    Some(cached) => cached,
                    None => return,
                },
            },
        };
        ctx.record_dependency(CacheDependency {
            source: CacheDependencySource::Stdlib {
                name: name.to_owned(),
            },
            kind: CacheDependencyKind::Module,
            fingerprint,
        });

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
        ctx: &mut ImportContext<'_>,
    ) -> Option<(Rc<Analysis>, Fingerprint)> {
        ctx.begin_module();
        ctx.in_progress.push(key.to_path_buf());
        let analysis = crate::analyze_inner(contents, parent, Some(key), ctx.options, ctx);
        ctx.in_progress.pop();
        let dependencies = ctx.finish_module();

        if analysis.blocking_diagnostics().next().is_some() || !analysis.is_thir_complete() {
            let nested =
                analysis
                    .diagnostics
                    .iter()
                    .find_map(|diagnostic| match &diagnostic.kind {
                        crate::SemanticDiagnosticKind::Import(import)
                            if matches!(import.kind, ImportDiagnosticKind::ImportCycle { .. }) =>
                        {
                            Some(import)
                        }
                        _ => None,
                    });
            let kind = if nested.is_some() {
                ImportDiagnosticKind::ImportCycle {
                    path: rel.to_string(),
                }
            } else {
                ImportDiagnosticKind::ModuleHasErrors {
                    path: rel.to_string(),
                }
            };
            let related = nested
                .map(|nested| {
                    let mut related = vec![crate::SourceLocation {
                        path: key.to_path_buf(),
                        span: nested.span,
                        label: "import cycle continues here".to_owned(),
                    }];
                    related.extend(nested.related.clone());
                    related
                })
                .unwrap_or_default();
            self.diag_with_related(kind, span, related);
            return None;
        }
        let module = Rc::new(analysis);
        let fingerprint = ctx.store_cached_module(key, contents, dependencies, module.clone());
        Some((module, fingerprint))
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
            zutai_thir::export_type(file, final_ty)
                .map(|ty| enrich_with_type_denotations(ty, file))
                .map(|ty| attach_type_only_exports(ty, file))
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
        self.diagnostics.push(ImportDiagnostic {
            kind,
            span,
            path: None,
            related: Vec::new(),
        });
    }

    fn diag_with_related(
        &mut self,
        kind: ImportDiagnosticKind,
        span: Span,
        related: Vec<crate::SourceLocation>,
    ) {
        self.diagnostics.push(ImportDiagnostic {
            kind,
            span,
            path: None,
            related,
        });
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

pub(crate) fn binding_name(hir: &HirFile, binding: BindingId) -> &str {
    &hir.bindings[binding.0 as usize].name
}

/// Turn an import source into a relative path string.
pub(crate) fn relative_path(source: &HirImportSource) -> Result<String, ImportDiagnosticKind> {
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
