//! LLVM IR text emission from Zutai SSA modules.
//!
//! Produces `.ll` files for the host LLVM target.
//! All Zutai values are represented as `i64` for v0 simplicity:
//! integers are stored directly, booleans as 0/1, and compound
//! values (records, tuples, lists, closures, text) are heap-allocated
//! with their pointer cast to `i64`.

use zutai_ssa::*;
mod descriptors;
mod format;
mod instr;
mod preamble;
#[cfg(test)]
mod tests;

use crate::descriptors::*;
use crate::instr::*;
use crate::preamble::*;

// ── Public API ─────────────────────────────────────────────────────────────────

/// Emit a complete LLVM IR `.ll` file from an SSA module.
pub fn emit_llvm(module: &SsaModule) -> String {
    let mut out = String::with_capacity(8192);
    emit_preamble(&mut out);
    emit_type_decls(&mut out);
    emit_runtime_decls(&mut out);
    emit_posit_runtime_decls(module, &mut out);
    collect_and_emit_constants(module, &mut out);
    let entry_desc = emit_descriptors(module, &mut out);
    emit_static_closures(&mut out, module);

    let all_funcs = collect_functions(module);
    for func in &all_funcs {
        emit_func_def(&mut out, func);
    }

    emit_main(&mut out, &module.entry.name, &entry_desc);
    out
}

/// Return why the module's entry value cannot be rendered by the native ABI.
pub fn unsupported_entry_type_reason(module: &SsaModule) -> Option<&'static str> {
    match &module.entry_ty {
        DfTy::Fun(_, _) => Some(
            "compiled entry point returns a function, which cannot be shown by the v0 runtime ABI",
        ),
        DfTy::Type => {
            Some("compiled entry point returns Type, which cannot be shown by the v0 runtime ABI")
        }
        _ => None,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// All functions reachable from the module (entry + every decl).
fn collect_functions(module: &SsaModule) -> Vec<&SsaFunc> {
    let mut funcs = Vec::new();
    funcs.push(&module.entry);
    for decl in &module.decls {
        match decl {
            SsaDecl::Func(f) => funcs.push(f),
            SsaDecl::RecGroup(group) => funcs.extend(group),
        }
    }
    funcs
}
