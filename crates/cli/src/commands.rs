use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
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
}

const UNSUPPORTED_TYPE_ENTRY_REASON: &str =
    "compiled entry point returns Type, which cannot be shown by the v0 runtime ABI";

fn unsupported_thir_entry_type_reason(thir: &zutai_thir::ThirFile) -> Option<&'static str> {
    let final_ty = thir.expr_arena[thir.final_expr].ty;
    let kind = &thir.type_arena.get(final_ty.0 as usize)?.kind;
    matches!(kind, zutai_thir::TypeKind::Type).then_some(UNSUPPORTED_TYPE_ENTRY_REASON)
}

pub(crate) fn run_bare_path(path: &str) -> Result<(), Box<dyn Error>> {
    match extension_or_error(path)?.as_str() {
        "zt" => run_file(path),
        "zti" => run_parse_zti(path),
        other => Err(format!("Unsupported file extension: .{other}").into()),
    }
}

// ─── subcommand implementations ───────────────────────────────────────────────

/// Evaluate a `.zt` file and print the result.
pub(crate) fn run_file(path: &str) -> Result<(), Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    // Resolve imports relative to the file's directory.
    let base = Path::new(path).parent();
    match eval_to_string(&contents, base) {
        Ok(rendered) => println!("{rendered}"),
        Err(zutai_eval::EvalError::NotRunnable(msgs)) => {
            // These are parse/HIR/import errors — render with miette if possible,
            // otherwise fall back to the semantic analyzer for pretty output.
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
        Err(zutai_eval::EvalError::TypeCheckFailed(msgs)) => {
            for m in msgs {
                eprintln!("type error: {m}");
            }
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("runtime error: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Evaluate `contents` on a worker thread with a large stack.
///
/// The reference evaluator can still use deep native recursion on both the TLC
/// path and the THIR reflection boundary. A 256 MiB worker stack lets realistic
/// recursion complete. The forced `Value` holds `Rc`s and is not `Send`, so it is
/// rendered to its `Display` string inside the worker; only the `String` (or the
/// `Send` `EvalError`) crosses the join boundary.
fn eval_to_string(contents: &str, base: Option<&Path>) -> Result<String, zutai_eval::EvalError> {
    const EVAL_STACK_SIZE: usize = 256 * 1024 * 1024;
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(EVAL_STACK_SIZE)
            .spawn_scoped(scope, || {
                zutai_eval::eval_with_base(contents, base).map(|value| value.to_string())
            })
            .expect("failed to spawn evaluation thread")
            .join()
            .expect("evaluation thread panicked")
    })
}

/// Parse a `.zt` file and print the AST (the old default behavior).
pub(crate) fn run_parse(path: &str) -> Result<(), Box<dyn Error>> {
    let ext = extension_or_error(path)?;
    let contents = fs::read_to_string(path)?;
    match ext.as_str() {
        "zt" => {
            let analysis = zutai_semantic::analyze(&contents);
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
                        zutai_semantic::SemanticStage::Hir | zutai_semantic::SemanticStage::Thir
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

/// Compile a `.zt` file. LLVM emits text; object/binary modes invoke the host LLVM toolchain.
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
    let uses_reflection = analysis.reflection_builtin_program().is_some();

    if let Some(reason) = analysis.config_overlay_builtin_program() {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }

    // TLC lowering. Effectful closed executables are folded through the TLC
    // semantics oracle before DC; residual effectful functions still fail here.
    let mut module = zutai_tlc::lower_thir(thir);
    let mut folded_bindings = None;
    let mut host_prints = Vec::new();
    let has_residual_effects = zutai_tlc::residual_effect_reason(&module).is_some();
    if uses_reflection && has_residual_effects {
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
                host_prints = folded.host_prints;
            }
            Err(err) => {
                eprintln!("compile error: {err}");
                std::process::exit(1);
            }
        }
    } else if has_residual_effects {
        match fold_aot_effects(&contents, base) {
            Ok(folded) => {
                module = folded.module;
                folded_bindings = Some(folded.hir_bindings);
                host_prints = folded.host_prints;
            }
            Err(err) => {
                eprintln!("compile error: {err}");
                std::process::exit(1);
            }
        }
    }
    if let Some(reason) = zutai_tlc::residual_effect_reason(&module) {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    let hir_bindings = folded_bindings
        .as_deref()
        .unwrap_or(original_hir_bindings.as_slice());

    // DC → ANF → SSA → LLVM IR pipeline.
    let graph = zutai_dataflow::lower_tlc(&module, hir_bindings);
    let anf = zutai_anf::lower_dc(&graph);
    let ssa = zutai_ssa::lower_anf(&anf);
    if let Some(reason) = zutai_codegen::unsupported_entry_type_reason(&ssa) {
        eprintln!("compile error: {reason}");
        std::process::exit(1);
    }
    let llvm_ir = zutai_codegen::emit_llvm_with_host_prints(&ssa, &host_prints);

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
            link_binary(&obj, &rt, &out)?;
        }
    }
    Ok(())
}

struct FoldedAotEffects {
    module: zutai_tlc::TlcModule,
    hir_bindings: Vec<zutai_hir::Binding>,
    host_prints: Vec<String>,
}

fn fold_aot_effects(
    contents: &str,
    base: Option<&Path>,
) -> Result<FoldedAotEffects, Box<dyn Error>> {
    let (source, host_prints) = fold_effect_value_to_source(contents, base)?;
    let pure = zutai_semantic::analyze_with_base(
        &source,
        None,
        zutai_semantic::AnalysisOptions::default(),
    );
    if !pure.is_thir_complete() {
        return Err(std::io::Error::other("folded effect value did not re-analyze").into());
    }
    let module = pure
        .tlc
        .ok_or_else(|| std::io::Error::other("folded effect value produced no TLC"))?;
    let hir_bindings = pure
        .hir
        .ok_or_else(|| std::io::Error::other("folded effect value produced no HIR"))?
        .file
        .bindings;
    Ok(FoldedAotEffects {
        module,
        hir_bindings,
        host_prints,
    })
}

fn fold_aot_reflection(
    contents: &str,
    base: Option<&Path>,
) -> Result<FoldedAotEffects, Box<dyn Error>> {
    let source = fold_reflection_value_to_source(contents, base)?;
    let pure = zutai_semantic::analyze_with_base(
        &source,
        None,
        zutai_semantic::AnalysisOptions::default(),
    );
    if !pure.is_thir_complete() {
        return Err(std::io::Error::other("folded reflection value did not re-analyze").into());
    }
    let module = pure
        .tlc
        .ok_or_else(|| std::io::Error::other("folded reflection value produced no TLC"))?;
    let hir_bindings = pure
        .hir
        .ok_or_else(|| std::io::Error::other("folded reflection value produced no HIR"))?
        .file
        .bindings;
    Ok(FoldedAotEffects {
        module,
        hir_bindings,
        host_prints: Vec::new(),
    })
}

fn fold_reflection_value_to_source(
    contents: &str,
    base: Option<&Path>,
) -> Result<String, Box<dyn Error>> {
    let contents = contents.to_owned();
    let base = base.map(Path::to_path_buf);
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || -> Result<String, String> {
            let value = zutai_eval::eval_with_base(&contents, base.as_deref())
                .map_err(|err| err.to_string())?;
            reflection_value_to_source(&value).ok_or_else(|| {
                if value_contains_type(&value) {
                    UNSUPPORTED_TYPE_ENTRY_REASON.to_string()
                } else {
                    "reflection entry did not fold to a backend value".to_string()
                }
            })
        })?;
    match handle.join() {
        Ok(Ok(source)) => Ok(source),
        Ok(Err(err)) => Err(std::io::Error::other(err).into()),
        Err(_) => Err(std::io::Error::other("reflection fold worker panicked").into()),
    }
}

fn fold_effect_value_to_source(
    contents: &str,
    base: Option<&Path>,
) -> Result<(String, Vec<String>), Box<dyn Error>> {
    let contents = contents.to_owned();
    let base = base.map(Path::to_path_buf);
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || -> Result<(String, Vec<String>), String> {
            let analysis = zutai_semantic::analyze_with_base(
                &contents,
                base.as_deref(),
                zutai_semantic::AnalysisOptions::default(),
            );
            let (value, host_prints) = zutai_eval::eval_tlc_analysis_capture_io(&analysis)
                .map_err(|err| err.to_string())?;
            let source = value_to_source(&value)
                .ok_or_else(|| "effectful entry did not fold to a backend value".to_string())?;
            Ok((source, host_prints))
        })?;
    match handle.join() {
        Ok(Ok(folded)) => Ok(folded),
        Ok(Err(err)) => Err(std::io::Error::other(err).into()),
        Err(_) => Err(std::io::Error::other("effect fold worker panicked").into()),
    }
}
#[derive(Clone, Copy)]
enum EmptyListType {
    SchemaFields,
    SchemaVariants,
}

struct TypedEmptyList {
    name: String,
    ty: &'static str,
}

fn reflection_value_to_source(value: &zutai_eval::Value) -> Option<String> {
    let mut empty_lists = Vec::new();
    let expr = reflection_value_to_source_in(value, None, &mut empty_lists)?;
    if empty_lists.is_empty() {
        return Some(expr);
    }

    let mut source = String::from("{");
    for empty in empty_lists {
        source.push_str(&empty.name);
        source.push_str(" : ");
        source.push_str(empty.ty);
        source.push_str(" = [];\n");
    }
    source.push_str(&expr);
    source.push('}');
    Some(source)
}

fn reflection_value_to_source_in(
    value: &zutai_eval::Value,
    empty_list_type: Option<EmptyListType>,
    empty_lists: &mut Vec<TypedEmptyList>,
) -> Option<String> {
    match value {
        zutai_eval::Value::List(items) if items.is_empty() => match empty_list_type {
            Some(kind) => {
                let name = format!("__zutai_fold_empty{}", empty_lists.len());
                let ty = match kind {
                    EmptyListType::SchemaFields => {
                        "List { name : Text; type : Text; optional : Bool; }"
                    }
                    EmptyListType::SchemaVariants => {
                        "List { name : Text; fields : List { name : Text; type : Text; optional : Bool; }; }"
                    }
                };
                empty_lists.push(TypedEmptyList {
                    name: name.clone(),
                    ty,
                });
                Some(name)
            }
            None => Some("[]".to_string()),
        },
        zutai_eval::Value::List(items) => {
            let mut out = String::from("[");
            for item in items.iter() {
                out.push_str(&reflection_value_to_source_in(
                    &item.peek()?,
                    empty_list_type,
                    empty_lists,
                )?);
                out.push_str("; ");
            }
            out.push(']');
            Some(out)
        }
        zutai_eval::Value::Tuple(items) => {
            let mut out = String::from("(");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                if let Some(name) = &item.name {
                    out.push_str(name);
                    out.push_str(" = ");
                }
                out.push_str(&reflection_value_to_source_in(
                    &item.value.peek()?,
                    None,
                    empty_lists,
                )?);
            }
            out.push(')');
            Some(out)
        }
        zutai_eval::Value::Record(fields) => {
            let mut out = String::from("{");
            for (name, value) in fields.iter() {
                out.push_str(name);
                out.push_str(" = ");
                let list_type = match name.as_ref() {
                    "fields" => Some(EmptyListType::SchemaFields),
                    "variants" => Some(EmptyListType::SchemaVariants),
                    _ => None,
                };
                out.push_str(&reflection_value_to_source_in(
                    &value.peek()?,
                    list_type,
                    empty_lists,
                )?);
                out.push_str("; ");
            }
            out.push('}');
            Some(out)
        }
        zutai_eval::Value::TaggedValue { tag, payload } => {
            if payload.is_empty() {
                return Some(format!("#{tag}"));
            }
            let positional = payload
                .iter()
                .enumerate()
                .all(|(index, (name, _))| name.parse::<usize>() == Ok(index));
            if positional {
                let mut out = format!("#{tag} (");
                for (index, (_, value)) in payload.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&reflection_value_to_source_in(
                        &value.peek()?,
                        None,
                        empty_lists,
                    )?);
                }
                out.push(')');
                Some(out)
            } else {
                let mut out = format!("#{tag} {{");
                for (name, value) in payload.iter() {
                    out.push_str(name);
                    out.push_str(" = ");
                    out.push_str(&reflection_value_to_source_in(
                        &value.peek()?,
                        None,
                        empty_lists,
                    )?);
                    out.push_str("; ");
                }
                out.push('}');
                Some(out)
            }
        }
        _ => value_to_source(value),
    }
}

fn value_to_source(value: &zutai_eval::Value) -> Option<String> {
    match value {
        zutai_eval::Value::Bool(value) => Some(value.to_string()),
        zutai_eval::Value::Int(value) => Some(value.to_string()),
        zutai_eval::Value::Float(value) => Some(float_source(*value)),
        zutai_eval::Value::Text(value) => Some(text_source(value)),
        zutai_eval::Value::Atom(value) => Some(format!("#{value}")),
        zutai_eval::Value::List(items) => {
            let mut out = String::from("[");
            for item in items.iter() {
                out.push_str(&value_to_source(&item.peek()?)?);
                out.push_str("; ");
            }
            out.push(']');
            Some(out)
        }
        zutai_eval::Value::Tuple(items) => {
            let mut out = String::from("(");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                if let Some(name) = &item.name {
                    out.push_str(name);
                    out.push_str(" = ");
                }
                out.push_str(&value_to_source(&item.value.peek()?)?);
            }
            out.push(')');
            Some(out)
        }
        zutai_eval::Value::Record(fields) => {
            let mut out = String::from("{");
            for (name, value) in fields.iter() {
                out.push_str(name);
                out.push_str(" = ");
                out.push_str(&value_to_source(&value.peek()?)?);
                out.push_str("; ");
            }
            out.push('}');
            Some(out)
        }
        zutai_eval::Value::TaggedValue { tag, payload } => {
            if payload.is_empty() {
                return Some(format!("#{tag}"));
            }
            let positional = payload
                .iter()
                .enumerate()
                .all(|(index, (name, _))| name.parse::<usize>() == Ok(index));
            if positional {
                let mut out = format!("#{tag} (");
                for (index, (_, value)) in payload.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&value_to_source(&value.peek()?)?);
                }
                out.push(')');
                Some(out)
            } else {
                let mut out = format!("#{tag} {{");
                for (name, value) in payload.iter() {
                    out.push_str(name);
                    out.push_str(" = ");
                    out.push_str(&value_to_source(&value.peek()?)?);
                    out.push_str("; ");
                }
                out.push('}');
                Some(out)
            }
        }
        zutai_eval::Value::Nothing => Some("#absent".to_string()),
        zutai_eval::Value::Posit(_)
        | zutai_eval::Value::Closure(_)
        | zutai_eval::Value::TypeValue(_)
        | zutai_eval::Value::WitnessDict(_)
        | zutai_eval::Value::TlcClosure(_)
        | zutai_eval::Value::Builtin(_)
        | zutai_eval::Value::BuiltinPartial { .. } => None,
    }
}

fn value_contains_type(value: &zutai_eval::Value) -> bool {
    match value {
        zutai_eval::Value::TypeValue(_) => true,
        zutai_eval::Value::List(items) => items
            .iter()
            .any(|item| item.peek().is_some_and(|value| value_contains_type(&value))),
        zutai_eval::Value::Tuple(items) => items.iter().any(|item| {
            item.value
                .peek()
                .is_some_and(|value| value_contains_type(&value))
        }),
        zutai_eval::Value::Record(fields) => fields.iter().any(|(_, value)| {
            value
                .peek()
                .is_some_and(|value| value_contains_type(&value))
        }),
        zutai_eval::Value::TaggedValue { payload, .. } => payload.iter().any(|(_, value)| {
            value
                .peek()
                .is_some_and(|value| value_contains_type(&value))
        }),
        zutai_eval::Value::Bool(_)
        | zutai_eval::Value::Int(_)
        | zutai_eval::Value::Float(_)
        | zutai_eval::Value::Text(_)
        | zutai_eval::Value::Atom(_)
        | zutai_eval::Value::Nothing
        | zutai_eval::Value::Posit(_)
        | zutai_eval::Value::Closure(_)
        | zutai_eval::Value::WitnessDict(_)
        | zutai_eval::Value::TlcClosure(_)
        | zutai_eval::Value::Builtin(_)
        | zutai_eval::Value::BuiltinPartial { .. } => false,
    }
}

fn text_source(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn float_source(value: f64) -> String {
    let source = format!("{value:?}");
    if !value.is_finite() || source.contains('.') || source.contains('e') || source.contains('E') {
        source
    } else {
        format!("{source}.0")
    }
}

fn output_path_for(input: &str, output_path: Option<&str>, emit: EmitMode) -> PathBuf {
    if let Some(out) = output_path {
        return PathBuf::from(out);
    }
    let mut out = PathBuf::from(input);
    match emit {
        EmitMode::Llvm => out.set_extension("ll"),
        EmitMode::Obj => out.set_extension("o"),
        EmitMode::Bin => out.set_extension(""),
    };
    out
}

fn tool_name(env_name: &str, fallback_env: &str, default: &'static str) -> String {
    std::env::var(env_name)
        .or_else(|_| std::env::var(fallback_env))
        .unwrap_or_else(|_| default.to_string())
}

fn run_tool(command: &mut Command, tool: &str, purpose: &str) -> Result<(), Box<dyn Error>> {
    let status = command.status().map_err(|err| {
        format!(
            "compile error: required tool `{tool}` failed to start for {purpose}: {err}; install it or set ZUTAI_{}",
            tool.to_ascii_uppercase()
        )
    })?;
    if !status.success() {
        return Err(format!("compile error: `{tool}` failed while {purpose}").into());
    }
    Ok(())
}

fn assemble_object(ll: &Path, out: &Path) -> Result<(), Box<dyn Error>> {
    let llc = tool_name("ZUTAI_LLC", "LLC", "llc");
    let mut command = Command::new(&llc);
    command
        .arg("-filetype=obj")
        .arg("-relocation-model=pic")
        .arg("-o")
        .arg(out)
        .arg(ll);
    run_tool(&mut command, &llc, "assembling LLVM IR")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("cli crate lives under crates/cli")
        .to_path_buf()
}

fn cargo_target_dir(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => {
            let path = PathBuf::from(dir);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        }
        None => root.join("target"),
    }
}

fn append_rustflag(flag: &str) -> String {
    match std::env::var("RUSTFLAGS") {
        Ok(existing) if !existing.trim().is_empty() => format!("{existing} {flag}"),
        _ => flag.to_string(),
    }
}

fn build_runtime_archive() -> Result<PathBuf, Box<dyn Error>> {
    let root = workspace_root();
    let target_dir = cargo_target_dir(&root);
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("-p")
        .arg("zutai-rt")
        .current_dir(&root);
    if std::env::consts::OS == "linux" {
        command.env("RUSTFLAGS", append_rustflag("-C relocation-model=pic"));
    }
    run_tool(&mut command, "cargo", "building zutai-rt")?;
    Ok(target_dir.join("debug").join("libzutai_rt.a"))
}

fn runtime_link_flags() -> &'static [&'static str] {
    match std::env::consts::OS {
        "linux" => &["-pie", "-lpthread", "-ldl", "-lm"],
        "macos" => &[],
        _ => &[],
    }
}

fn link_binary(obj: &Path, runtime: &Path, out: &Path) -> Result<(), Box<dyn Error>> {
    let clang = tool_name("ZUTAI_CLANG", "CLANG", "clang");
    let mut command = Command::new(&clang);
    command.arg(obj).arg(runtime);
    for flag in runtime_link_flags() {
        command.arg(flag);
    }
    command.arg("-o").arg(out);
    run_tool(&mut command, &clang, "linking native binary")
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

    let uses_reflection = analysis.reflection_builtin_program().is_some();
    if let Some(reason) = analysis.config_overlay_builtin_program() {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }

    let mut module = zutai_tlc::lower_thir(thir);
    let mut folded_bindings = None;
    let has_residual_effects = zutai_tlc::residual_effect_reason(&module).is_some();
    if uses_reflection && has_residual_effects {
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
    } else if has_residual_effects {
        match fold_aot_effects(&contents, base) {
            Ok(folded) => {
                module = folded.module;
                folded_bindings = Some(folded.hir_bindings);
            }
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
    }
    if let Some(reason) = zutai_tlc::residual_effect_reason(&module) {
        eprintln!("error: {reason}");
        std::process::exit(1);
    }
    let hir_bindings = folded_bindings
        .as_deref()
        .unwrap_or(original_hir_bindings.as_slice());
    let graph = zutai_dataflow::lower_tlc(&module, hir_bindings);
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
                match zutai_eval::eval_file(&eval_src) {
                    Ok(value) => println!("{value}"),
                    Err(zutai_eval::EvalError::NotRunnable(msgs)) => {
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
                    Err(zutai_eval::EvalError::TypeCheckFailed(msgs)) => {
                        for m in msgs {
                            eprintln!("type error: {m}");
                        }
                    }
                    Err(e) => eprintln!("error: {e}"),
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
    let analysis = zutai_semantic::analyze(src);
    match analysis.hir.as_ref() {
        Some(h) => h.file.decls.len(),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use zutai_eval::thunk::Thunk;
    use zutai_eval::value::{BuiltinFn, TupleField};

    #[test]
    fn count_decls_in_returns_zero_for_unparseable() {
        assert_eq!(count_decls_in(""), 0);
    }

    #[test]
    fn count_decls_in_returns_one_for_single_decl() {
        assert_eq!(count_decls_in("x ::= 1\nx\n"), 1);
    }

    #[test]
    fn count_decls_in_returns_two_for_two_decls() {
        assert_eq!(count_decls_in("x ::= 1\ny ::= 2\nx\n"), 2);
    }

    #[test]
    fn runtime_link_flags_never_request_non_pie() {
        assert!(!runtime_link_flags().contains(&"-no-pie"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_runtime_link_flags_request_pie() {
        assert!(runtime_link_flags().contains(&"-pie"));
    }

    #[test]
    fn value_to_source_covers_scalar_escapes_and_float_suffix() {
        assert_eq!(
            value_to_source(&zutai_eval::Value::Bool(true)),
            Some("true".to_string())
        );
        assert_eq!(
            value_to_source(&zutai_eval::Value::Float(2.0)),
            Some("2.0".to_string())
        );
        assert_eq!(
            value_to_source(&zutai_eval::Value::Float(f64::INFINITY)),
            Some("inf".to_string())
        );
        assert_eq!(
            value_to_source(&zutai_eval::Value::Atom("prod".into())),
            Some("#prod".to_string())
        );
        let text = "quote\" slash\\ line\n cr\r tab\t";
        assert_eq!(
            value_to_source(&zutai_eval::Value::Text(text.into())),
            Some("\"quote\\\" slash\\\\ line\\n cr\\r tab\\t\"".to_string())
        );
    }

    #[test]
    fn value_to_source_covers_tuple_tagged_absent_and_none() {
        let one = Thunk::ready(zutai_eval::Value::Int(1));
        let tuple = zutai_eval::Value::Tuple(Rc::from([
            TupleField {
                name: Some("x".into()),
                value: one.clone(),
            },
            TupleField {
                name: None,
                value: Thunk::ready(zutai_eval::Value::Text("y".into())),
            },
        ]));
        assert_eq!(value_to_source(&tuple), Some("(x = 1, \"y\")".to_string()));

        assert_eq!(
            value_to_source(&zutai_eval::Value::TaggedValue {
                tag: "none".into(),
                payload: Rc::new(vec![]),
            }),
            Some("#none".to_string())
        );
        assert_eq!(
            value_to_source(&zutai_eval::Value::TaggedValue {
                tag: "some".into(),
                payload: Rc::new(vec![
                    ("0".into(), Thunk::ready(zutai_eval::Value::Int(1))),
                    (
                        "1".into(),
                        Thunk::ready(zutai_eval::Value::Text("x".into())),
                    ),
                ]),
            }),
            Some("#some (1, \"x\")".to_string())
        );
        assert_eq!(
            value_to_source(&zutai_eval::Value::TaggedValue {
                tag: "point".into(),
                payload: Rc::new(vec![
                    ("x".into(), Thunk::ready(zutai_eval::Value::Int(1))),
                    ("y".into(), Thunk::ready(zutai_eval::Value::Int(2))),
                ]),
            }),
            Some("#point {x = 1; y = 2; }".to_string())
        );
        assert_eq!(
            value_to_source(&zutai_eval::Value::Nothing),
            Some("#absent".to_string())
        );
        assert_eq!(
            value_to_source(&zutai_eval::Value::Builtin(BuiltinFn::Print)),
            None
        );
    }

    #[test]
    fn output_path_for_derives_default_paths() {
        assert_eq!(
            output_path_for("main.zt", None, EmitMode::Llvm),
            PathBuf::from("main.ll")
        );
        assert_eq!(
            output_path_for("main.zt", None, EmitMode::Obj),
            PathBuf::from("main.o")
        );
        assert_eq!(
            output_path_for("main.zt", None, EmitMode::Bin),
            PathBuf::from("main")
        );
        assert_eq!(
            output_path_for("main.zt", Some("custom.out"), EmitMode::Bin),
            PathBuf::from("custom.out")
        );
    }
}
