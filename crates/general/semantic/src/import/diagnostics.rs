use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

use zutai_hir::HirFile;
use zutai_syntax::Span;
use zutai_thir::{
    ImportedRowTail, ImportedTupleItem, ImportedType, ThirDeclKind, ThirFile, WitnessPattern,
    export_witness_pattern, match_pattern_key,
};

use super::*;

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
    pub path: Option<PathBuf>,
    pub related: Vec<crate::SourceLocation>,
}

impl ImportDiagnostic {
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
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
    /// The configured filesystem standard library could not be loaded.
    StdlibSetup {
        message: String,
    },
    /// The nearest `zutai.zti` package manifest or its local dependency graph
    /// is malformed.
    PackageSetup {
        message: String,
    },
    /// A dotted package import could not resolve through the importing
    /// package's declared dependency aliases and public module map.
    PackageResolution {
        path: String,
        message: String,
    },
    /// `import stdlib.<name>` named a module the configured standard library
    /// does not provide.
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

impl ImportDiagnosticKind {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoBaseDirectory => "zutai::import::no_base_directory",
            Self::UnsupportedImportForm { .. } => "zutai::import::unsupported_form",
            Self::StdlibSetup { .. } => "zutai::import::stdlib_setup",
            Self::PackageSetup { .. } => "zutai::import::package_setup",
            Self::PackageResolution { .. } => "zutai::import::package_resolution",
            Self::UnknownStdlibModule { .. } => "zutai::import::unknown_stdlib_module",
            Self::FileNotFound { .. } => "zutai::import::file_not_found",
            Self::ReadError { .. } => "zutai::import::read_error",
            Self::ParseError { .. } => "zutai::import::parse_error",
            Self::ImportCycle { .. } => "zutai::import::cycle",
            Self::ModuleHasErrors { .. } => "zutai::import::module_has_errors",
            Self::UnsupportedExport { .. } => "zutai::import::unsupported_export",
            Self::ConflictingWitness { .. } => "zutai::import::conflicting_witness",
            Self::PathTraversal { .. } => "zutai::import::path_traversal",
        }
    }
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
                    path: Some(witness.origin.clone()),
                    related: Vec::new(),
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

/// Whether the merged witness registry `exports` provides a witness for
/// `constraint` at the concrete operand key `target_key`. Mirrors the
/// interpreter's runtime dispatch (`materialize_conditional_dict`) so the
/// compile-time S1 gate accepts exactly the calls the interpreter can run:
///
/// - a concrete export whose `constraint`/`target_key` match exactly, or
/// - a conditional export whose pattern matches `target_key`, recovering each
///   parameter's sub-key, where every one of that parameter's component
///   constraints is itself covered at its sub-key (recursively).
///
/// `depth` guards against a pathological conditional cycle, matching the runtime
/// depth bound.
pub(crate) fn witness_registry_covers(
    exports: &[WitnessExport],
    constraint: &str,
    target_key: &str,
    depth: u32,
) -> bool {
    if depth > 64 {
        return false;
    }
    // Concrete exact match. A bare higher-kinded constructor witness such as
    // `Functor @List` also covers each saturated application key (`List[Int]`).
    if exports.iter().any(|e| {
        e.constraint == constraint
            && e.conditional.is_none()
            && (e.target_key == target_key
                || target_key
                    .strip_prefix(e.target_key.as_str())
                    .is_some_and(|suffix| suffix.starts_with('[')))
    }) {
        return true;
    }
    // Conditional match: pattern matches the key and every component bound is
    // covered at its recovered sub-key.
    exports.iter().any(|e| {
        if e.constraint != constraint {
            return false;
        }
        let Some(cond) = &e.conditional else {
            return false;
        };
        let Some(sub_keys) = match_pattern_key(&cond.pattern, target_key, cond.param_bounds.len())
        else {
            return false;
        };
        cond.param_bounds.iter().enumerate().all(|(i, bounds)| {
            bounds
                .iter()
                .all(|bound| witness_registry_covers(exports, bound, &sub_keys[i], depth + 1))
        })
    })
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
        let Ok(exported_target) = zutai_thir::export_witness_target(file, *target) else {
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

pub(crate) fn imported_type_key(ty: &ImportedType) -> String {
    match ty {
        ImportedType::Bool => "Bool".to_string(),
        ImportedType::Int => "Int".to_string(),
        ImportedType::Float => "Float".to_string(),
        ImportedType::FixedNum(fw) => fw.name().to_string(),
        ImportedType::Posit(spec) => spec.type_name(),
        ImportedType::Text => "Text".to_string(),
        ImportedType::Opaque(name) => format!("opaque:{name}"),
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
        ImportedType::WithTypeExports { value, types } => {
            let mut parts: Vec<String> = types
                .iter()
                .map(|field| format!("{}:{}", field.name, imported_type_key(&field.ty)))
                .collect();
            parts.sort();
            format!("{}+types{{{}}}", imported_type_key(value), parts.join(","))
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
        ImportedType::Effect { base, ops, tail } => {
            let ops = ops
                .iter()
                .map(|op| {
                    format!(
                        "{}:{}->{}",
                        op.name,
                        imported_type_key(&op.param),
                        imported_type_key(&op.result)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            let tail = match tail {
                ImportedRowTail::Closed => String::new(),
                ImportedRowTail::Open => "...".to_string(),
                ImportedRowTail::Param(id) => format!("...#{id}"),
            };
            format!("{}!{{{}{tail}}}", imported_type_key(base), ops)
        }
        ImportedType::Type(inner) => format!("Type({})", imported_type_key(inner)),
        ImportedType::TypeCon { params, body } => {
            let ps: Vec<String> = params.iter().map(|id| format!("'{id}")).collect();
            format!("\\<{}>{}", ps.join(","), imported_type_key(body))
        }
        ImportedType::ConApply { ctor, args } => {
            if args.is_empty() {
                ctor.clone()
            } else {
                let parts: Vec<String> = args.iter().map(imported_type_key).collect();
                format!("{ctor}[{}]", parts.join(","))
            }
        }
        ImportedType::TyVar(id) => format!("'{id}"),
        ImportedType::Unknown => "?".to_string(),
    }
}
