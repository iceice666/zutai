use std::error::Error;
use std::fs;
use std::path::Path;

mod reflect;
#[cfg(test)]
mod tests;
mod toolchain;

use self::reflect::*;
use self::toolchain::*;
use std::process::Command;

use crate::diagnostics::{
    ZtParseDiagnostic, extension_or_error, format_import_diagnostic, print_ast,
    print_semantic_errors, print_zt_errors,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EmitMode {
    Llvm,
    Obj,
    Bin,
    Lib,
}

const UNSUPPORTED_TYPE_ENTRY_REASON: &str =
    "compiled entry point returns Type, which cannot be shown by the v0 runtime ABI";
const UNSUPPORTED_OPAQUE_ENTRY_REASON: &str = "compiled entry point returns an opaque host handle, which cannot be shown by the v0 runtime ABI";

fn unsupported_thir_entry_type_reason(thir: &zutai_thir::ThirFile) -> Option<&'static str> {
    fn alias_body(
        thir: &zutai_thir::ThirFile,
        binding: zutai_hir::BindingId,
    ) -> Option<zutai_thir::TypeId> {
        thir.decl_arena.iter().find_map(|(_, decl)| {
            if decl.binding == binding
                && let zutai_thir::ThirDeclKind::TypeAlias { ty, .. } = decl.kind
            {
                Some(ty)
            } else {
                None
            }
        })
    }

    fn resolve_alias(
        thir: &zutai_thir::ThirFile,
        mut ty: zutai_thir::TypeId,
    ) -> zutai_thir::TypeId {
        let mut seen = rustc_hash::FxHashSet::default();
        loop {
            if !seen.insert(ty) {
                return ty;
            }
            match thir.type_arena[ty.0 as usize].kind {
                zutai_thir::TypeKind::Alias(binding) => match alias_body(thir, binding) {
                    Some(body) => ty = body,
                    None => return ty,
                },
                _ => return ty,
            }
        }
    }

    fn is_capability_type(thir: &zutai_thir::ThirFile, ty: zutai_thir::TypeId) -> bool {
        let ty = resolve_alias(thir, ty);
        matches!(
            &thir.type_arena[ty.0 as usize].kind,
            zutai_thir::TypeKind::Opaque(name)
                if zutai_hir::ir::HOST_CAPABILITY_TYPE_NAMES.contains(&name.as_str())
        )
    }

    fn is_capability_record(thir: &zutai_thir::ThirFile, ty: zutai_thir::TypeId) -> bool {
        let ty = resolve_alias(thir, ty);
        match &thir.type_arena[ty.0 as usize].kind {
            zutai_thir::TypeKind::Record(fields, zutai_thir::RowTail::Closed) => {
                !fields.is_empty()
                    && fields
                        .iter()
                        .all(|field| is_capability_type(thir, field.ty))
            }
            _ => false,
        }
    }

    fn rendered_entry_type(
        thir: &zutai_thir::ThirFile,
        mut ty: zutai_thir::TypeId,
    ) -> zutai_thir::TypeId {
        loop {
            let resolved = resolve_alias(thir, ty);
            match thir.type_arena[resolved.0 as usize].kind {
                zutai_thir::TypeKind::Function { from, to }
                    if is_capability_type(thir, from) || is_capability_record(thir, from) =>
                {
                    ty = to;
                }
                _ => return resolved,
            }
        }
    }

    fn contains_opaque(
        thir: &zutai_thir::ThirFile,
        ty: zutai_thir::TypeId,
        seen: &mut rustc_hash::FxHashSet<zutai_thir::TypeId>,
    ) -> bool {
        let ty = resolve_alias(thir, ty);
        if !seen.insert(ty) {
            return false;
        }
        match &thir.type_arena[ty.0 as usize].kind {
            zutai_thir::TypeKind::Opaque(_) => true,
            zutai_thir::TypeKind::List(inner)
            | zutai_thir::TypeKind::Optional(inner)
            | zutai_thir::TypeKind::Maybe(inner)
            | zutai_thir::TypeKind::Patch { target: inner, .. } => {
                contains_opaque(thir, *inner, seen)
            }
            zutai_thir::TypeKind::Record(fields, _) => fields
                .iter()
                .any(|field| contains_opaque(thir, field.ty, seen)),
            zutai_thir::TypeKind::Tuple(items) => items.iter().any(|item| {
                let ty = match item {
                    zutai_thir::TypeTupleItem::Named { ty, .. }
                    | zutai_thir::TypeTupleItem::Positional(ty) => *ty,
                };
                contains_opaque(thir, ty, seen)
            }),
            zutai_thir::TypeKind::Union(variants, _) => variants.iter().any(|variant| {
                variant
                    .payload
                    .is_some_and(|ty| contains_opaque(thir, ty, seen))
            }),
            zutai_thir::TypeKind::Effect { base, .. } => contains_opaque(thir, *base, seen),
            zutai_thir::TypeKind::Function { .. } => false,
            zutai_thir::TypeKind::Alias(_)
            | zutai_thir::TypeKind::AliasApply { .. }
            | zutai_thir::TypeKind::Apply { .. }
            | zutai_thir::TypeKind::Con(_)
            | zutai_thir::TypeKind::ForAll { .. }
            | zutai_thir::TypeKind::TypeVar(_)
            | zutai_thir::TypeKind::InferVar(_) => false,
            _ => false,
        }
    }

    let final_ty = rendered_entry_type(thir, thir.expr_arena[thir.final_expr].ty);
    let kind = &thir.type_arena.get(final_ty.0 as usize)?.kind;
    if matches!(kind, zutai_thir::TypeKind::Type(_)) {
        Some(UNSUPPORTED_TYPE_ENTRY_REASON)
    } else if contains_opaque(thir, final_ty, &mut rustc_hash::FxHashSet::default()) {
        Some(UNSUPPORTED_OPAQUE_ENTRY_REASON)
    } else {
        None
    }
}

pub(crate) fn run_bare_path(path: &str) -> Result<(), Box<dyn Error>> {
    match extension_or_error(path)?.as_str() {
        "zt" => run_file(path),
        "zti" => run_parse_zti(path),
        other => Err(format!("Unsupported file extension: .{other}").into()),
    }
}

// ─── eval isolation ───────────────────────────────────────────────────────────

/// Outcome of an isolated evaluation: successful rendered value, a structured
/// error, or a panic absorbed from the evaluator worker.
#[derive(Debug, PartialEq)]
pub(crate) enum EvalOutcome {
    Ok(String),
    Err(zutai_eval::EvalError),
    Panicked(String),
}

/// Extract a human-readable message from a `catch_unwind` panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "evaluator panicked (run with RUST_BACKTRACE=1 for details)".to_string()
    }
}

/// Run `f` on a large-stack (256 MiB) worker thread with full panic isolation.
///
/// Returns [`EvalOutcome::Panicked`] when the worker panics instead of
/// re-panicking the caller; the main thread is never unwound.
pub(crate) fn run_isolated<F>(f: F) -> EvalOutcome
where
    F: FnOnce() -> Result<String, zutai_eval::EvalError> + Send,
{
    const EVAL_STACK_SIZE: usize = 256 * 1024 * 1024;
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .stack_size(EVAL_STACK_SIZE)
            .spawn_scoped(scope, move || {
                // Absorb panics inside the worker so the thread itself never
                // unwinds past this closure; join() always returns Ok(_).
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
            })
            .expect("failed to spawn evaluation thread");
        match handle.join() {
            Ok(catch_result) => match catch_result {
                Ok(Ok(s)) => EvalOutcome::Ok(s),
                Ok(Err(e)) => EvalOutcome::Err(e),
                Err(payload) => EvalOutcome::Panicked(panic_message(&*payload)),
            },
            Err(payload) => EvalOutcome::Panicked(panic_message(&*payload)),
        }
    })
}

/// Evaluate `contents` on an isolated large-stack worker thread.
///
/// The forced `Value` holds `Rc`s and is not `Send`, so it is rendered to its
/// `Display` string inside the worker; only the `String` (or the `Send`
/// `EvalError`, or the caught panic message) crosses the join boundary.
pub(crate) fn eval_isolated(contents: &str, base: Option<&Path>) -> EvalOutcome {
    let contents = contents.to_owned();
    let base = base.map(Path::to_path_buf);
    run_isolated(move || {
        reject_unsupported_render_entry(&contents, base.as_deref())?;
        zutai_eval::eval_with_base(&contents, base.as_deref()).map(|v| v.to_string())
    })
}

fn reject_unsupported_render_entry(
    contents: &str,
    base: Option<&Path>,
) -> Result<(), zutai_eval::EvalError> {
    let analysis = zutai_semantic::analyze_with_base(
        contents,
        base,
        zutai_semantic::AnalysisOptions::default(),
    );
    let Some(thir) = analysis.thir.as_ref().and_then(|thir| thir.file.as_ref()) else {
        return Ok(());
    };
    if let Some(reason) = unsupported_thir_entry_type_reason(thir) {
        Err(zutai_eval::EvalError::NotRunnable(vec![reason.to_string()]))
    } else {
        Ok(())
    }
}

// ─── subcommand implementations ───────────────────────────────────────────────

/// Evaluate a `.zt` file and print the result.
pub(crate) fn run_file(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    // Resolve imports relative to the file's directory.
    let base = Path::new(path).parent();
    match eval_isolated(&contents, base) {
        EvalOutcome::Ok(rendered) => println!("{rendered}"),
        outcome => exit_for_eval_failure(path, &contents, base, outcome),
    }
    Ok(())
}

/// Render a non-`Ok` `.zt` evaluation outcome to stderr and exit nonzero.
///
/// Shared by `run_file` and `run_json` so `.zt` parse/import/type/runtime/panic
/// diagnostics and exit codes stay identical across both commands.
fn exit_for_eval_failure(
    path: &str,
    contents: &str,
    base: Option<&Path>,
    outcome: EvalOutcome,
) -> ! {
    match outcome {
        EvalOutcome::Ok(_) => unreachable!("exit_for_eval_failure called with EvalOutcome::Ok"),
        EvalOutcome::Err(zutai_eval::EvalError::NotRunnable(msgs)) => {
            // These are parse/HIR/import errors — render with miette if possible,
            // otherwise fall back to the semantic analyzer for pretty output.
            let analysis = zutai_semantic::analyze_with_base(
                contents,
                base,
                zutai_semantic::AnalysisOptions::default(),
            );
            let parse_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter_map(|d| match &d.kind {
                    zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
                    _ => None,
                })
                .collect();
            if !parse_errors.is_empty() {
                print_zt_errors(path, contents, &parse_errors);
                std::process::exit(1);
            }
            let import_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter_map(|d| match &d.kind {
                    zutai_semantic::SemanticDiagnosticKind::Import(i) => Some(i.clone()),
                    _ => None,
                })
                .collect();
            if !import_errors.is_empty() {
                for err in &import_errors {
                    eprintln!("import error: {}", format_import_diagnostic(err));
                }
                std::process::exit(1);
            }
            for m in msgs {
                eprintln!("error: {m}");
            }
            std::process::exit(1);
        }
        EvalOutcome::Err(zutai_eval::EvalError::TypeCheckFailed(msgs)) => {
            for m in msgs {
                eprintln!("type error: {m}");
            }
            std::process::exit(1);
        }
        EvalOutcome::Err(e) => {
            eprintln!("runtime error: {e}");
            std::process::exit(1);
        }
        EvalOutcome::Panicked(msg) => {
            eprintln!("internal evaluator error: {msg}");
            std::process::exit(1);
        }
    }
}

/// Parse a `.zt` file and print the AST (the old default behavior).
pub(crate) fn run_parse(path: &str) -> Result<(), Box<dyn Error>> {
    let ext = extension_or_error(path)?;
    let contents = fs::read_to_string(path)?;
    match ext.as_str() {
        "zt" => {
            let analysis = zutai_semantic::analyze_with_base(
                &contents,
                Path::new(path).parent(),
                zutai_semantic::AnalysisOptions::default(),
            );
            let parse_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter_map(|d| match &d.kind {
                    zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
                    _ => None,
                })
                .collect();
            if !parse_errors.is_empty() {
                print_zt_errors(path, &contents, &parse_errors);
                std::process::exit(1);
            }
            let semantic_errors: Vec<_> = analysis
                .diagnostics
                .iter()
                .filter(|d| {
                    matches!(
                        d.stage,
                        zutai_semantic::SemanticStage::Import
                            | zutai_semantic::SemanticStage::Hir
                            | zutai_semantic::SemanticStage::Thir
                    )
                })
                .collect();
            if !semantic_errors.is_empty() {
                print_semantic_errors(path, &contents, &semantic_errors);
                std::process::exit(1);
            }
            if let Some(ast) = analysis.ast.as_ref() {
                print_ast("zt", ast);
            } else {
                eprintln!("parse produced no AST");
                std::process::exit(1);
            }
        }
        "zti" => run_parse_zti(path)?,
        other => return Err(format!("Unsupported extension: {other}").into()),
    }
    Ok(())
}

/// Parse and print a `.zti` immediate-format file.
pub(crate) fn run_parse_zti(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let ast = zutai_im::parse(&contents).map_err(|e| format!("Failed to parse .zti: {e}"))?;
    print_ast("zti", &ast);
    Ok(())
}

/// Parse a `.zti` or evaluate a `.zt` file and print the final result as
/// natural JSON.
///
/// `.zti` documents are parsed and their inert data serialized directly; `.zt`
/// programs are fully evaluated (under the same isolated large-stack worker as
/// `run`) and the forced final value serialized. Encoding is natural JSON:
/// atoms become `#`-prefixed strings and tagged values become
/// `{ "tag", "payload" }` objects.
pub(crate) fn run_json(path: &str) -> Result<(), Box<dyn Error>> {
    match extension_or_error(path)?.as_str() {
        "zti" => {
            let contents = fs::read_to_string(path)?;
            let block =
                zutai_im::parse(&contents).map_err(|e| format!("Failed to parse .zti: {e}"))?;
            let json = zti_block_to_json(&block);
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        "zt" => {
            let contents = fs::read_to_string(path)?;
            let base = Path::new(path).parent();
            let owned_contents = contents.clone();
            let owned_base = base.map(Path::to_path_buf);
            let outcome = run_isolated(move || {
                reject_unsupported_render_entry(&owned_contents, owned_base.as_deref())?;
                zutai_eval::eval_with_base(&owned_contents, owned_base.as_deref())
                    .and_then(|value| value.to_json())
                    .map(|json| {
                        serde_json::to_string_pretty(&json)
                            .expect("serializing serde_json::Value cannot fail")
                    })
            });
            match outcome {
                EvalOutcome::Ok(rendered) => println!("{rendered}"),
                outcome => exit_for_eval_failure(path, &contents, base, outcome),
            }
        }
        other => return Err(format!("Unsupported extension: {other}").into()),
    }
    Ok(())
}

/// Convert a parsed `.zti` block into a natural JSON object.
fn zti_block_to_json(block: &zutai_im::Block) -> serde_json::Value {
    let mut map = serde_json::Map::with_capacity(block.len());
    for pair in block.iter() {
        map.insert(pair.field_name.clone(), zti_value_to_json(&pair.value));
    }
    serde_json::Value::Object(map)
}

/// Convert a parsed `.zti` value into natural JSON.
fn zti_value_to_json(value: &zutai_im::Value) -> serde_json::Value {
    use serde_json::Value as J;
    use zutai_im::Value as Im;
    match value {
        Im::True => J::Bool(true),
        Im::False => J::Bool(false),
        Im::Integer(n) => serde_json::json!(n),
        Im::Float(f) => serde_json::json!(f),
        Im::String(s) => J::String(s.clone()),
        Im::Atom(s) => J::String(format!("#{s}")),
        Im::Array(items) => J::Array(items.iter().map(zti_value_to_json).collect()),
        Im::Block(block) => zti_block_to_json(block),
    }
}

/// Run the type-checker on a `.zt` file and print diagnostics.
pub(crate) fn run_check(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let base = Path::new(path).parent();
    let analysis = zutai_semantic::analyze_with_base(
        &contents,
        base,
        zutai_semantic::AnalysisOptions::default(),
    );
    let parse_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter_map(|d| match &d.kind {
            zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    if !parse_errors.is_empty() {
        print_zt_errors(path, &contents, &parse_errors);
        std::process::exit(1);
    }
    let semantic_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.stage,
                zutai_semantic::SemanticStage::Hir
                    | zutai_semantic::SemanticStage::Thir
                    | zutai_semantic::SemanticStage::Import
            )
        })
        .collect();
    if !semantic_errors.is_empty() {
        print_semantic_errors(path, &contents, &semantic_errors);
        std::process::exit(1);
    }
    if analysis.is_thir_complete() {
        println!("check passed: {path}");
    } else {
        eprintln!("check incomplete: THIR has errors");
        std::process::exit(1);
    }
    Ok(())
}

// ── Module-import program assembly ─────────────────────────────────────────────

/// Reject reason when native lowering cannot safely link an imported witness
/// export: concrete and structurally matchable conditional witnesses lower
/// through extern tables, but higher-kinded / non-matchable shapes still have no
/// static dispatch key the backend can preserve.
const IMPORT_WITNESS_REASON: &str = "native backend does not support importing higher-kinded or \
    otherwise non-matchable typeclass instances yet. Use `zutai run` (interpreter)";

/// Collect the transitive `.zt` dependency analyses of `analysis` in topological
/// order (post-order DFS), so a dependency always precedes the modules that
/// import it (`deps[i]` may only import `deps[j]` with `j < i`). Dependencies that
/// failed to lower (no `tlc`) are omitted; the lowering gate rejects any import
/// still referencing them. Diamond imports are deduplicated by `Rc` pointer
/// identity, and the front end rejects import cycles, so the walk terminates.
fn collect_dep_analyses(
    analysis: &zutai_semantic::Analysis,
) -> (
    Vec<std::rc::Rc<zutai_semantic::Analysis>>,
    std::collections::HashMap<*const zutai_semantic::Analysis, usize>,
) {
    use std::collections::HashMap;
    use std::rc::Rc;

    fn recurse(
        dep: &Rc<zutai_semantic::Analysis>,
        analyses: &mut Vec<Rc<zutai_semantic::Analysis>>,
        ptr_to_idx: &mut HashMap<*const zutai_semantic::Analysis, usize>,
    ) {
        let ptr = Rc::as_ptr(dep);
        if ptr_to_idx.contains_key(&ptr) || dep.tlc.is_none() {
            return;
        }
        for child in dep.import_modules.values() {
            recurse(child, analyses, ptr_to_idx);
        }
        let idx = analyses.len();
        analyses.push(Rc::clone(dep));
        ptr_to_idx.insert(ptr, idx);
    }

    let mut analyses = Vec::new();
    let mut ptr_to_idx = HashMap::new();
    for dep in analysis.import_modules.values() {
        recurse(dep, &mut analyses, &mut ptr_to_idx);
    }
    (analyses, ptr_to_idx)
}

/// Build one module's import-resolution map: `.zti` data imports resolve to inline
/// constants; `.zt` module imports resolve to their dependency index. Import
/// sources are raw, module-local strings (the same string can name different
/// files from different directories), so `.zt` targets are keyed by the imported
/// analysis's `Rc` pointer, never the source string.
fn build_module_imports<'a>(
    module_analysis: &'a zutai_semantic::Analysis,
    ptr_to_idx: &std::collections::HashMap<*const zutai_semantic::Analysis, usize>,
) -> rustc_hash::FxHashMap<zutai_hir::HirImportSource, zutai_dataflow::ImportTarget<'a>> {
    let mut map = rustc_hash::FxHashMap::default();
    for (source, value) in &module_analysis.import_values {
        map.insert(source.clone(), zutai_dataflow::ImportTarget::Zti(value));
    }
    for (source, dep) in &module_analysis.import_modules {
        if let Some(&idx) = ptr_to_idx.get(&std::rc::Rc::as_ptr(dep)) {
            map.insert(source.clone(), zutai_dataflow::ImportTarget::Zt(idx));
        }
    }
    map
}

/// Assemble the dependency [`zutai_dataflow::ModuleInput`]s for a program, borrowing
/// each dependency's TLC and HIR bindings from `dep_analyses`. The returned vector
/// is index-aligned with `dep_analyses` (and therefore with the `Zt` targets in
/// every import map).
fn dep_module_inputs<'a>(
    dep_analyses: &'a [std::rc::Rc<zutai_semantic::Analysis>],
    dep_modules: &'a [zutai_tlc::TlcModule],
    ptr_to_idx: &std::collections::HashMap<*const zutai_semantic::Analysis, usize>,
) -> Vec<zutai_dataflow::ModuleInput<'a>> {
    dep_analyses
        .iter()
        .zip(dep_modules.iter())
        .map(|(dep, module)| {
            let hir_bindings = dep
                .hir
                .as_ref()
                .map(|hir| hir.file.bindings.as_slice())
                .unwrap_or(&[]);
            let imports = build_module_imports(dep, ptr_to_idx);
            zutai_dataflow::ModuleInput {
                module,
                hir_bindings,
                imports,
            }
        })
        .collect()
}

/// Clone dependency TLC modules and run the backend effect-lowering pass on each
/// one. Semantic analysis keeps interpreter-oriented TLC; native lowering needs
/// the same `finally`/residual-effect rewrite that the root module receives.
fn backend_dep_modules(
    dep_analyses: &[std::rc::Rc<zutai_semantic::Analysis>],
) -> Vec<zutai_tlc::TlcModule> {
    dep_analyses
        .iter()
        .map(|dep| {
            let mut module = dep
                .tlc
                .as_ref()
                .expect("dependency with no TLC must be filtered by collect_dep_analyses")
                .clone();
            zutai_tlc::lower_effects_for_backend(&mut module);
            module
        })
        .collect()
}

/// Concrete extern-witness triple: `(constraint_name, target_key, dc_global_name)`.
type ConcreteExternWitness = (String, String, String);

/// Build the concrete and conditional extern-witness tables for the root's TLC
/// lowering from the transitive dependency analyses. Each dep's witness export
/// maps to a dep-namespaced DC global name (`$dep{idx}${constraint}$w{binding_id}`).
///
/// Returns `Err` when a dependency exports a parametric witness whose target
/// cannot be matched structurally (no conditional shape) — e.g. a higher-kinded
/// instance — so the caller falls back to the interpreter rather than miscompile.
fn extern_witness_tables(
    dep_analyses: &[std::rc::Rc<zutai_semantic::Analysis>],
) -> Result<
    (
        Vec<ConcreteExternWitness>,
        Vec<zutai_tlc::ExternConditionalWitness>,
    ),
    (),
> {
    let mut concrete = Vec::new();
    let mut conditional = Vec::new();
    for (idx, dep) in dep_analyses.iter().enumerate() {
        for w in &dep.witness_exports {
            let global = format!("$dep{idx}${}$w{}", w.constraint, w.binding_id);
            match &w.conditional {
                Some(shape) => conditional.push(zutai_tlc::ExternConditionalWitness {
                    constraint: w.constraint.clone(),
                    pattern: shape.pattern.clone(),
                    param_bounds: shape.param_bounds.clone(),
                    global,
                }),
                None if w.target_key.contains('?') => return Err(()),
                None => concrete.push((w.constraint.clone(), w.target_key.clone(), global)),
            }
        }
    }
    Ok((concrete, conditional))
}

/// Compile a `.zt` file. LLVM emits text; native artifact modes invoke the host LLVM toolchain.
pub(crate) fn run_compile(
    path: &str,
    output_path: Option<&str>,
    emit: EmitMode,
) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let base = Path::new(path).parent();
    let analysis = zutai_semantic::analyze_with_base(
        &contents,
        base,
        zutai_semantic::AnalysisOptions::default(),
    );
    // Gate on parse and semantic errors.
    let parse_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter_map(|d| match &d.kind {
            zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    if !parse_errors.is_empty() {
        print_zt_errors(path, &contents, &parse_errors);
        std::process::exit(1);
    }
    let semantic_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.stage,
                zutai_semantic::SemanticStage::Hir
                    | zutai_semantic::SemanticStage::Thir
                    | zutai_semantic::SemanticStage::Import
            )
        })
        .collect();
    if !semantic_errors.is_empty() {
        print_semantic_errors(path, &contents, &semantic_errors);
        std::process::exit(1);
    }
    if !analysis.is_thir_complete() {
        eprintln!("compile error: THIR incomplete");
        std::process::exit(1);
    }
    let thir = analysis.thir.as_ref().unwrap().file.as_ref().unwrap();
    if let Some(reason) = unsupported_thir_entry_type_reason(thir) {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    let original_hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;
    let uses_reflection = analysis.aot_reflection_program().is_some();

    if let Some(reason) = analysis.config_overlay_builtin_program() {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }

    // Collect deps early (needed to build extern witness list for TLC lowering).
    let (dep_analyses, ptr_to_idx) = collect_dep_analyses(&analysis);
    // Build the concrete + conditional extern-witness tables. A parametric
    // witness with no dispatchable shape (e.g. higher-kinded) still gates to the
    // interpreter; concrete and matchable-conditional witnesses lower natively.
    let (extern_witnesses, extern_conditionals) = match extern_witness_tables(&dep_analyses) {
        Ok(tables) => tables,
        Err(()) => {
            eprintln!("compile error: {IMPORT_WITNESS_REASON}");
            std::process::exit(1);
        }
    };
    let dep_modules = backend_dep_modules(&dep_analyses);

    // TLC lowering. Effectful programs enter DC only when TLC lowering has
    // eliminated effect markers or mapped ambient `io.print` to the runtime
    // HostPrint path.
    let mut module = if extern_witnesses.is_empty() && extern_conditionals.is_empty() {
        zutai_tlc::lower_thir_for_backend(thir)
    } else {
        zutai_tlc::lower_thir_with_extern_witnesses_for_backend(
            thir,
            extern_witnesses,
            extern_conditionals,
        )
    };
    // Backend-only: lower handled effects (finally, recursive/higher-order, …)
    // that `lower_thir` leaves residual for the interpreter oracle.
    zutai_tlc::lower_effects_for_backend(&mut module);
    let mut folded_bindings = None;
    let boundary_host_grants = zutai_tlc::HostEffectSet::ALL;
    let has_host_io_print = zutai_tlc::contains_host_io_print(&module);
    let has_unfolded_effects = zutai_tlc::residual_effect_reason(&module).is_some();
    let residual_effect_reason =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants);
    if uses_reflection && (has_host_io_print || has_unfolded_effects) {
        eprintln!(
            "compile error: reflection builtins cannot be AOT-folded with effectful code yet"
        );
        std::process::exit(1);
    }
    if uses_reflection {
        match fold_aot_reflection(&contents, base) {
            Ok(folded) => {
                module = folded.module;
                folded_bindings = Some(folded.hir_bindings);
            }
            Err(err) => {
                eprintln!("compile error: {err}");
                std::process::exit(1);
            }
        }
    } else if let Some(reason) = residual_effect_reason {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    if let Some(reason) =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants)
    {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    // Row-erased monomorphization: inline open-row field selects at concrete call
    // sites so the slot-based backend can lower them (Phase C).
    zutai_tlc::monomorphize_open_row_selects(&mut module);
    let hir_bindings = folded_bindings
        .as_deref()
        .unwrap_or(original_hir_bindings.as_slice());
    let program = zutai_dataflow::ProgramInput {
        root: zutai_dataflow::ModuleInput {
            module: &module,
            hir_bindings,
            imports: build_module_imports(&analysis, &ptr_to_idx),
        },
        deps: dep_module_inputs(&dep_analyses, &dep_modules, &ptr_to_idx),
    };
    let graph =
        match zutai_dataflow::try_lower_program_with_host_grants(&program, boundary_host_grants) {
            Ok(g) => g,
            Err(reason) => {
                eprintln!("compile error: {reason}");
                std::process::exit(1);
            }
        };
    let anf = zutai_anf::lower_dc(&graph);
    let ssa = zutai_ssa::lower_anf(&anf);
    if let Some(reason) = zutai_codegen::unsupported_entry_type_reason(&ssa) {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    let llvm_ir = match emit {
        EmitMode::Lib => zutai_codegen::emit_llvm_library(&ssa),
        EmitMode::Llvm | EmitMode::Obj | EmitMode::Bin => zutai_codegen::emit_llvm(&ssa),
    };

    match emit {
        EmitMode::Llvm => match output_path {
            Some(out) => fs::write(out, &llvm_ir)?,
            None => println!("{llvm_ir}"),
        },
        EmitMode::Obj => {
            let out = output_path_for(path, output_path, EmitMode::Obj);
            let ll = out.with_extension("ll");
            fs::write(&ll, &llvm_ir)?;
            assemble_object(&ll, &out)?;
        }
        EmitMode::Bin => {
            let out = output_path_for(path, output_path, EmitMode::Bin);
            let ll = out.with_extension("ll");
            let obj = out.with_extension("o");
            fs::write(&ll, &llvm_ir)?;
            assemble_object(&ll, &obj)?;
            let rt = build_runtime_archive()?;
            link_binary(&obj, rt.path(), &out)?;
        }
        EmitMode::Lib => {
            let out = output_path_for(path, output_path, EmitMode::Lib);
            let ll = out.with_extension("ll");
            let obj = out.with_extension("o");
            fs::write(&ll, &llvm_ir)?;
            assemble_object(&ll, &obj)?;
            let rt = build_runtime_archive()?;
            link_shared_library(&obj, rt.path(), &out)?;
        }
    }
    Ok(())
}

/// Print the Dataflow Core graph for a `.zt` file.
pub(crate) fn run_dataflow(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let base = Path::new(path).parent();
    let analysis = zutai_semantic::analyze_with_base(
        &contents,
        base,
        zutai_semantic::AnalysisOptions::default(),
    );
    let parse_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter_map(|d| match &d.kind {
            zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    if !parse_errors.is_empty() {
        print_zt_errors(path, &contents, &parse_errors);
        std::process::exit(1);
    }
    let semantic_errors: Vec<_> = analysis
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.stage,
                zutai_semantic::SemanticStage::Hir
                    | zutai_semantic::SemanticStage::Thir
                    | zutai_semantic::SemanticStage::Import
            )
        })
        .collect();
    if !semantic_errors.is_empty() {
        print_semantic_errors(path, &contents, &semantic_errors);
        std::process::exit(1);
    }
    if !analysis.is_thir_complete() {
        eprintln!("error: cannot lower incomplete THIR");
        std::process::exit(1);
    }
    let thir = analysis.thir.as_ref().unwrap().file.as_ref().unwrap();
    if let Some(reason) = unsupported_thir_entry_type_reason(thir) {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }
    let original_hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;

    let uses_reflection = analysis.aot_reflection_program().is_some();
    if let Some(reason) = analysis.config_overlay_builtin_program() {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }

    let (dep_analyses, ptr_to_idx) = collect_dep_analyses(&analysis);
    let (extern_witnesses_df, extern_conditionals_df) = match extern_witness_tables(&dep_analyses) {
        Ok(tables) => tables,
        Err(()) => {
            eprintln!("error: {IMPORT_WITNESS_REASON}");
            std::process::exit(1);
        }
    };
    let dep_modules = backend_dep_modules(&dep_analyses);

    let mut module = if extern_witnesses_df.is_empty() && extern_conditionals_df.is_empty() {
        zutai_tlc::lower_thir_for_backend(thir)
    } else {
        zutai_tlc::lower_thir_with_extern_witnesses_for_backend(
            thir,
            extern_witnesses_df,
            extern_conditionals_df,
        )
    };
    // Backend-only: lower handled effects that `lower_thir` leaves residual.
    zutai_tlc::lower_effects_for_backend(&mut module);
    let mut folded_bindings = None;
    let boundary_host_grants = zutai_tlc::HostEffectSet::ALL;
    let has_host_io_print = zutai_tlc::contains_host_io_print(&module);
    let has_unfolded_effects = zutai_tlc::residual_effect_reason(&module).is_some();
    let residual_effect_reason =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants);
    if uses_reflection && (has_host_io_print || has_unfolded_effects) {
        eprintln!("error: reflection builtins cannot be AOT-folded with effectful code yet");
        std::process::exit(1);
    }
    if uses_reflection {
        match fold_aot_reflection(&contents, base) {
            Ok(folded) => {
                module = folded.module;
                folded_bindings = Some(folded.hir_bindings);
            }
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
    } else if let Some(reason) = residual_effect_reason {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }
    if let Some(reason) =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants)
    {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }
    zutai_tlc::monomorphize_open_row_selects(&mut module);
    let hir_bindings = folded_bindings
        .as_deref()
        .unwrap_or(original_hir_bindings.as_slice());
    let program = zutai_dataflow::ProgramInput {
        root: zutai_dataflow::ModuleInput {
            module: &module,
            hir_bindings,
            imports: build_module_imports(&analysis, &ptr_to_idx),
        },
        deps: dep_module_inputs(&dep_analyses, &dep_modules, &ptr_to_idx),
    };
    let graph =
        match zutai_dataflow::try_lower_program_with_host_grants(&program, boundary_host_grants) {
            Ok(g) => g,
            Err(reason) => {
                eprintln!("error: {reason}");
                std::process::exit(1);
            }
        };
    println!("{graph:#?}");
    Ok(())
}

/// Run an interactive REPL session.
///
/// ## Session model
///
/// We keep a running `decls_buf: String` that accumulates all declarations the
/// user has entered.  Each input line is classified as:
///
/// - A **declaration** if `analyze(decls_buf + input + "\n0")` (with a throwaway
///   final expression appended so THIR has something to resolve) adds a new
///   declaration, parses cleanly, and passes the THIR gate.  If so, we append
///   the line to `decls_buf` and acknowledge.
///
/// - An **expression** otherwise: we try `eval_file(decls_buf + input)` through
///   the default TLC-first evaluator and print the result (or the diagnostic).
///
/// `BindingId`s are NOT stable across separate `analyze()` calls, so we NEVER
/// cache thunks across turns — the env is rebuilt from scratch for each
/// evaluation.
pub(crate) fn run_repl() -> Result<(), Box<dyn Error>> {
    use rustyline::DefaultEditor;
    use rustyline::error::ReadlineError;

    let mut rl = DefaultEditor::new()?;
    let mut decls_buf = String::new();

    println!("Zutai REPL (type `:quit` to exit, `:reset` to clear bindings)");

    loop {
        let prompt = if decls_buf.is_empty() { "zt> " } else { "... " };
        match rl.readline(prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(trimmed);

                match trimmed {
                    ":quit" | ":q" => break,
                    ":reset" => {
                        decls_buf.clear();
                        println!("(bindings cleared)");
                        continue;
                    }
                    _ => {}
                }

                // Try treating the input as a declaration by appending it and
                // a throwaway final expression `0`.
                let probe_src = format!("{decls_buf}{trimmed}\n0\n");
                let probe = zutai_semantic::analyze(&probe_src);
                let decl_count_before = count_decls_in(&decls_buf);
                let decl_count_after = count_decls_in(&probe_src);

                let is_new_decl = !probe.has_parse_errors()
                    && !probe.has_hir_errors()
                    && probe.is_thir_complete()
                    && decl_count_after > decl_count_before;

                if is_new_decl {
                    decls_buf.push_str(trimmed);
                    decls_buf.push('\n');
                    println!("ok");
                    continue;
                }

                // Otherwise treat as an expression to evaluate.
                let eval_src = format!("{decls_buf}{trimmed}\n");
                // Suppress the worker's default panic-hook line so only the
                // clean error message reaches stderr; the hook is always
                // restored before the next prompt so no global state leaks.
                let prev_hook = std::panic::take_hook();
                std::panic::set_hook(Box::new(|_| {}));
                let outcome = eval_isolated(&eval_src, None);
                std::panic::set_hook(prev_hook);
                match outcome {
                    EvalOutcome::Ok(rendered) => println!("{rendered}"),
                    EvalOutcome::Err(zutai_eval::EvalError::NotRunnable(msgs)) => {
                        // Try pretty parse error output.
                        let analysis = zutai_semantic::analyze(&eval_src);
                        let parse_errors: Vec<_> = analysis
                            .diagnostics
                            .iter()
                            .filter_map(|d| match &d.kind {
                                zutai_semantic::SemanticDiagnosticKind::Parse(p) => Some(p.clone()),
                                _ => None,
                            })
                            .collect();
                        if !parse_errors.is_empty() {
                            for err in &parse_errors {
                                eprintln!(
                                    "{:?}",
                                    miette::Report::new(ZtParseDiagnostic::new(
                                        "<repl>",
                                        &eval_src,
                                        err.clone()
                                    ))
                                );
                            }
                        } else {
                            for m in msgs {
                                eprintln!("error: {m}");
                            }
                        }
                    }
                    EvalOutcome::Err(zutai_eval::EvalError::TypeCheckFailed(msgs)) => {
                        for m in msgs {
                            eprintln!("type error: {m}");
                        }
                    }
                    EvalOutcome::Err(e) => eprintln!("error: {e}"),
                    EvalOutcome::Panicked(msg) => {
                        eprintln!(
                            "internal evaluator error: {msg}\n(input not applied; session preserved)"
                        );
                    }
                }
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())
}

/// Estimate the number of top-level declarations in `src` from parsed HIR
/// declarations (used only to classify REPL input).
pub(crate) fn count_decls_in(src: &str) -> usize {
    // Count user declarations from the parsed AST — the HIR additionally carries
    // the builtin prelude (e.g. the `Stream` codata type), which is not user code.
    let analysis = zutai_semantic::analyze(src);
    match analysis.ast.as_ref() {
        Some(ast) => ast.decls.len(),
        None => 0,
    }
}
