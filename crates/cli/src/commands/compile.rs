use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EmitMode {
    Llvm,
    Obj,
    Bin,
    Lib,
}

pub(crate) fn analyze_with_cli_diagnostics(
    path: &str,
    contents: &str,
    base: Option<&Path>,
    cache: &zutai_semantic::AnalysisCache,
) -> zutai_semantic::Analysis {
    let analysis = zutai_semantic::analyze_with_base_and_cache(
        contents,
        base,
        zutai_semantic::AnalysisOptions::default(),
        cache,
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
        print_semantic_errors(path, contents, &semantic_errors);
        std::process::exit(1);
    }
    analysis
}

pub(crate) fn unsupported_cli_entry_type_reason(
    analysis: &zutai_semantic::Analysis,
) -> Option<&'static str> {
    unsupported_thir_entry_type_reason(analysis.thir.as_ref()?.file.as_ref()?)
}

pub(crate) fn fold_aot_reflection_for_cli(
    contents: &str,
    base: Option<&Path>,
) -> Result<FoldedAotReflection, String> {
    fold_aot_reflection(contents, base).map_err(|err| err.to_string())
}

pub(crate) fn backend_entry_span(analysis: &zutai_semantic::Analysis) -> zutai_syntax::Span {
    analysis
        .thir
        .as_ref()
        .and_then(|lowered| lowered.file.as_ref())
        .map(|file| file.expr_arena[file.final_expr].span)
        .unwrap_or_default()
}

pub(crate) const UNSUPPORTED_TYPE_ENTRY_REASON: &str =
    "compiled entry point returns Type, which cannot be shown by the runtime ABI";
pub(crate) const UNSUPPORTED_OPAQUE_ENTRY_REASON: &str =
    "compiled entry point returns an opaque host handle, which cannot be shown by the runtime ABI";

pub(crate) fn unsupported_thir_entry_type_reason(
    thir: &zutai_thir::ThirFile,
) -> Option<&'static str> {
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

// ── Module-import program assembly ─────────────────────────────────────────────

/// Collect the transitive `.zt` dependency analyses of `analysis` in topological
/// order (post-order DFS), so a dependency always precedes the modules that
/// import it (`deps[i]` may only import `deps[j]` with `j < i`). Dependencies that
/// failed to lower (no `tlc`) are omitted; the lowering gate rejects any import
/// still referencing them. Diamond imports are deduplicated by `Rc` pointer
/// identity, and the front end rejects import cycles, so the walk terminates.
pub(crate) fn collect_dep_analyses(
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
pub(crate) fn build_module_imports<'a>(
    module_analysis: &'a zutai_semantic::Analysis,
    ptr_to_idx: &std::collections::HashMap<*const zutai_semantic::Analysis, usize>,
) -> rustc_hash::FxHashMap<zutai_hir::HirImportSource, zutai_dataflow::ImportTarget<'a>> {
    let mut map = rustc_hash::FxHashMap::default();
    for (source, value) in &module_analysis.import_values {
        map.insert(source.clone(), zutai_dataflow::ImportTarget::Zti(value));
    }
    for (source, dep) in &module_analysis.import_modules {
        let ptr = Rc::as_ptr(dep);
        let idx = *ptr_to_idx
            .get(&ptr)
            .expect("all transitive import modules must be collected");
        map.insert(source.clone(), zutai_dataflow::ImportTarget::Zt(idx));
    }
    map
}

/// Assemble the dependency [`zutai_dataflow::ModuleInput`]s for a program, borrowing
/// each dependency's TLC and HIR bindings from `dep_analyses`. The returned vector
/// is index-aligned with `dep_analyses` (and therefore with the `Zt` targets in
/// every import map).
pub(crate) fn dep_module_inputs<'a>(
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
pub(crate) fn backend_dep_modules(
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
pub(crate) type ConcreteExternWitness = (String, String, String);

/// Build the concrete and conditional extern-witness tables for the root's TLC
/// lowering from the transitive dependency analyses. Each dep's witness export
/// maps to a dep-namespaced DC global name (`$dep{idx}${constraint}$w{binding_id}`).
///
/// Returns `Err` when a dependency exports a parametric witness whose target
/// cannot be matched structurally (no conditional shape) — e.g. a higher-kinded
/// instance — so the caller falls back to the interpreter rather than miscompile.
pub(crate) fn extern_witness_tables(
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

/// Compile a `.zt` file for an explicit validated target.
pub(crate) fn run_compile(
    path: &str,
    output_path: Option<&str>,
    emit: EmitMode,
    target: zutai_codegen::NativeTarget,
    metadata_path: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let base = Path::new(path).parent();
    let cache = zutai_semantic::AnalysisCache::default();
    if metadata_path.is_some() {
        let recorded = zutai_semantic::analyze_path_recording_with_cache(Path::new(path), &cache)?;
        if recorded.analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.stage,
                zutai_semantic::SemanticStage::Parse
                    | zutai_semantic::SemanticStage::Hir
                    | zutai_semantic::SemanticStage::Import
                    | zutai_semantic::SemanticStage::Thir
            )
        }) {
            analyze_with_cli_diagnostics(path, &contents, base, &cache);
        }
        run_compile_analysis(
            CompileRequest {
                path,
                output_path,
                emit,
                target,
                metadata_path,
                contents: &contents,
                base,
            },
            &recorded.analysis,
            Some(&recorded),
        )
    } else {
        let analysis = analyze_with_cli_diagnostics(path, &contents, base, &cache);
        run_compile_analysis(
            CompileRequest {
                path,
                output_path,
                emit,
                target,
                metadata_path,
                contents: &contents,
                base,
            },
            &analysis,
            None,
        )
    }
}

pub(crate) struct CompileRequest<'a> {
    path: &'a str,
    output_path: Option<&'a str>,
    emit: EmitMode,
    target: zutai_codegen::NativeTarget,
    metadata_path: Option<&'a Path>,
    contents: &'a str,
    base: Option<&'a Path>,
}

pub(crate) fn run_compile_analysis(
    request: CompileRequest<'_>,
    analysis: &zutai_semantic::Analysis,
    recorded: Option<&zutai_semantic::RecordedAnalysis>,
) -> Result<(), Box<dyn Error>> {
    let CompileRequest {
        path,
        output_path,
        emit,
        target,
        metadata_path,
        contents,
        base,
    } = request;
    if !analysis.is_thir_complete() {
        eprintln!("compile error: THIR incomplete");
        std::process::exit(1);
    }
    if let Some(reason) = unsupported_cli_entry_type_reason(analysis) {
        print_backend_error(
            path,
            contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_ENTRY_TYPE_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: backend_entry_span(analysis),
                related: Vec::new(),
            },
        );
        std::process::exit(1);
    }
    let thir = analysis.thir.as_ref().unwrap().file.as_ref().unwrap();
    let original_hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;
    let uses_reflection = analysis.aot_reflection_program().is_some();

    if let Some(reason) = analysis.config_overlay_builtin_program() {
        print_backend_error(
            path,
            contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_CONFIG_OVERLAY_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: backend_entry_span(analysis),
                related: Vec::new(),
            },
        );
        std::process::exit(1);
    }

    if let Some(mut diagnostic) = analysis.native_import_diagnostics().into_iter().next() {
        diagnostic.severity = zutai_syntax::Severity::Error;
        print_backend_error(path, contents, &diagnostic);
        std::process::exit(1);
    }
    let (dep_analyses, ptr_to_idx) = collect_dep_analyses(analysis);
    let (extern_witnesses, extern_conditionals) = extern_witness_tables(&dep_analyses)
        .expect("native import diagnostics reject every non-matchable witness export");
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
        print_backend_error(
            path,
            contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_REFLECTION_EFFECT_CODE,
                severity: zutai_syntax::Severity::Error,
                message: "reflection builtins cannot be AOT-folded with effectful code yet"
                    .to_owned(),
                span: backend_entry_span(analysis),
                related: Vec::new(),
            },
        );
        std::process::exit(1);
    }
    if uses_reflection {
        match fold_aot_reflection_for_cli(contents, base) {
            Ok(folded) => {
                module = folded.module;
                folded_bindings = Some(folded.hir_bindings);
            }
            Err(err) => {
                print_backend_error(
                    path,
                    contents,
                    &zutai_semantic::BackendDiagnostic {
                        code: zutai_semantic::BACKEND_REFLECTION_FOLD_CODE,
                        severity: zutai_syntax::Severity::Error,
                        message: err.to_string(),
                        span: backend_entry_span(analysis),
                        related: Vec::new(),
                    },
                );
                std::process::exit(1);
            }
        }
    } else if let Some(reason) = residual_effect_reason {
        print_backend_error(
            path,
            contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_RESIDUAL_EFFECT_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: backend_entry_span(analysis),
                related: Vec::new(),
            },
        );
        std::process::exit(1);
    }
    if let Some(reason) =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants)
    {
        print_backend_error(
            path,
            contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_RESIDUAL_EFFECT_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: backend_entry_span(analysis),
                related: Vec::new(),
            },
        );
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
            imports: build_module_imports(analysis, &ptr_to_idx),
        },
        deps: dep_module_inputs(&dep_analyses, &dep_modules, &ptr_to_idx),
    };
    let graph =
        match zutai_dataflow::try_lower_program_with_host_grants(&program, boundary_host_grants) {
            Ok(g) => g,
            Err(reason) => {
                print_backend_error(
                    path,
                    contents,
                    &zutai_semantic::BackendDiagnostic {
                        code: zutai_semantic::BACKEND_DATAFLOW_CODE,
                        severity: zutai_syntax::Severity::Error,
                        message: reason.to_owned(),
                        span: backend_entry_span(analysis),
                        related: Vec::new(),
                    },
                );
                std::process::exit(1);
            }
        };
    let anf = zutai_anf::lower_dc(&graph);
    let ssa = zutai_ssa::lower_anf(&anf);
    if let Some(reason) = zutai_codegen::unsupported_entry_type_reason(&ssa) {
        print_backend_error(
            path,
            contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_ENTRY_TYPE_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: backend_entry_span(analysis),
                related: Vec::new(),
            },
        );
        std::process::exit(1);
    }
    let native_preflight = if matches!(emit, EmitMode::Obj | EmitMode::Bin | EmitMode::Lib) {
        Some(preflight_native(emit, target)?)
    } else {
        None
    };
    let llvm_ir = match emit {
        EmitMode::Lib => zutai_codegen::emit_llvm_library(&ssa, target),
        EmitMode::Llvm | EmitMode::Obj | EmitMode::Bin => zutai_codegen::emit_llvm(&ssa, target),
    };

    match emit {
        EmitMode::Llvm => match output_path {
            Some(out) => fs::write(out, &llvm_ir)?,
            None => println!("{llvm_ir}"),
        },
        EmitMode::Obj => {
            let out = output_path_for(path, output_path, EmitMode::Obj, target);
            let intermediates = NativeIntermediates::new(&out)?;
            fs::write(&intermediates.llvm, &llvm_ir)?;
            assemble_object(&intermediates.llvm, &intermediates.object, target)?;
            fs::rename(&intermediates.object, &out)?;
        }
        EmitMode::Bin => {
            let out = output_path_for(path, output_path, EmitMode::Bin, target);
            let preflight = native_preflight.as_ref().unwrap();
            let intermediates = NativeIntermediates::new(&out)?;
            let pending = &intermediates.pending;
            fs::write(&intermediates.llvm, &llvm_ir)?;
            assemble_object(&intermediates.llvm, &intermediates.object, target)?;
            link_binary(
                &intermediates.object,
                preflight.runtime.as_ref().unwrap(),
                pending,
                target,
            )?;
            fs::rename(pending, out)?;
        }
        EmitMode::Lib => {
            let out = output_path_for(path, output_path, EmitMode::Lib, target);
            let preflight = native_preflight.as_ref().unwrap();
            let intermediates = NativeIntermediates::new(&out)?;
            let pending = &intermediates.pending;
            fs::write(&intermediates.llvm, &llvm_ir)?;
            assemble_object(&intermediates.llvm, &intermediates.object, target)?;
            link_shared_library(
                &intermediates.object,
                preflight.runtime.as_ref().unwrap(),
                pending,
                target,
            )?;
            fs::rename(pending, out)?;
        }
    }
    if let Some(metadata_path) = metadata_path {
        write_build_metadata(
            metadata_path,
            emit,
            target,
            recorded.expect("metadata compilation records its analysis inputs"),
        )?;
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
        print_backend_error(
            path,
            &contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_ENTRY_TYPE_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: backend_entry_span(&analysis),
                related: Vec::new(),
            },
        );
        std::process::exit(1);
    }
    let original_hir_bindings = &analysis.hir.as_ref().unwrap().file.bindings;

    let uses_reflection = analysis.aot_reflection_program().is_some();
    if let Some(reason) = analysis.config_overlay_builtin_program() {
        print_backend_error(
            path,
            &contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_CONFIG_OVERLAY_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: backend_entry_span(&analysis),
                related: Vec::new(),
            },
        );
        std::process::exit(1);
    }

    if let Some(mut diagnostic) = analysis.native_import_diagnostics().into_iter().next() {
        diagnostic.severity = zutai_syntax::Severity::Error;
        print_backend_error(path, &contents, &diagnostic);
        std::process::exit(1);
    }
    let (dep_analyses, ptr_to_idx) = collect_dep_analyses(&analysis);
    let (extern_witnesses_df, extern_conditionals_df) = extern_witness_tables(&dep_analyses)
        .expect("native import diagnostics reject every non-matchable witness export");
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
        print_backend_error(
            path,
            &contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_REFLECTION_EFFECT_CODE,
                severity: zutai_syntax::Severity::Error,
                message: "reflection builtins cannot be AOT-folded with effectful code yet"
                    .to_owned(),
                span: backend_entry_span(&analysis),
                related: Vec::new(),
            },
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
                print_backend_error(
                    path,
                    &contents,
                    &zutai_semantic::BackendDiagnostic {
                        code: zutai_semantic::BACKEND_REFLECTION_FOLD_CODE,
                        severity: zutai_syntax::Severity::Error,
                        message: err.to_string(),
                        span: backend_entry_span(&analysis),
                        related: Vec::new(),
                    },
                );
                std::process::exit(1);
            }
        }
    } else if let Some(reason) = residual_effect_reason {
        print_backend_error(
            path,
            &contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_RESIDUAL_EFFECT_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: backend_entry_span(&analysis),
                related: Vec::new(),
            },
        );
        std::process::exit(1);
    }
    if let Some(reason) =
        zutai_tlc::residual_effect_reason_with_grants(&module, boundary_host_grants)
    {
        print_backend_error(
            path,
            &contents,
            &zutai_semantic::BackendDiagnostic {
                code: zutai_semantic::BACKEND_RESIDUAL_EFFECT_CODE,
                severity: zutai_syntax::Severity::Error,
                message: reason.to_owned(),
                span: backend_entry_span(&analysis),
                related: Vec::new(),
            },
        );
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
                print_backend_error(
                    path,
                    &contents,
                    &zutai_semantic::BackendDiagnostic {
                        code: zutai_semantic::BACKEND_DATAFLOW_CODE,
                        severity: zutai_syntax::Severity::Error,
                        message: reason.to_owned(),
                        span: backend_entry_span(&analysis),
                        related: Vec::new(),
                    },
                );
                std::process::exit(1);
            }
        };
    println!("{graph:#?}");
    Ok(())
}
