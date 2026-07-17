//! LLVM IR text emission from Zutai SSA modules.
//!
//! Produces `.ll` files for an explicit validated native target.
//! All Zutai values are represented as `i64` for v0 simplicity:
//! integers are stored directly, booleans as 0/1, and compound
//! values (records, tuples, lists, closures, text) are heap-allocated
//! with their pointer cast to `i64`.

use zutai_ssa::*;
mod descriptors;
mod format;
mod instr;
mod preamble;
pub mod target;
#[cfg(test)]
mod tests;

use crate::descriptors::*;
use crate::instr::*;
use crate::preamble::*;
pub use target::{NativeArch, NativeOs, NativeTarget, NativeTargetError};

// ── Public API ─────────────────────────────────────────────────────────────────
/// Emit a complete LLVM IR `.ll` file for a validated native target.
pub fn emit_llvm(module: &SsaModule, target: NativeTarget) -> String {
    emit_llvm_artifact(module, target, ArtifactKind::Executable)
}

/// Emit LLVM IR for a native library artifact.
///
/// The library form intentionally omits `main` and instead exports:
///
/// - `zutai_entry() -> i64` — raw v0 ABI value,
/// - `zutai_entry_descriptor() -> i64` — static descriptor pointer as an `i64`,
/// - `zutai_entry_json() -> i64` — JSON text value serialized through `serde_json`.
pub fn emit_llvm_library(module: &SsaModule, target: NativeTarget) -> String {
    emit_llvm_artifact(module, target, ArtifactKind::Library)
}

#[derive(Clone, Copy)]
enum ArtifactKind {
    Executable,
    Library,
}

fn emit_llvm_artifact(module: &SsaModule, target: NativeTarget, artifact: ArtifactKind) -> String {
    let mut out = String::with_capacity(8192);
    emit_preamble(&mut out, target);
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

    match artifact {
        ArtifactKind::Executable => emit_main(&mut out, &module.entry.name, &entry_desc),
        ArtifactKind::Library => emit_library_exports(&mut out, &module.entry.name, &entry_desc),
    }
    out
}

/// Return why the module's entry value cannot be rendered by the native ABI.
pub fn unsupported_entry_type_reason(module: &SsaModule) -> Option<&'static str> {
    fn contains_opaque(
        types: &DfTypes,
        ty: &DfTy,
        seen: &mut rustc_hash::FxHashSet<DfTyId>,
    ) -> bool {
        match ty {
            DfTy::Opaque(_) => true,
            DfTy::List(inner) | DfTy::Optional(inner) | DfTy::Maybe(inner) => {
                seen.insert(*inner) && contains_opaque(types, &types[*inner], seen)
            }
            DfTy::Record(fields) => fields.iter().any(|field| {
                seen.insert(field.ty) && contains_opaque(types, &types[field.ty], seen)
            }),
            DfTy::Union(variants) => variants.iter().any(|variant| {
                seen.insert(variant.ty) && contains_opaque(types, &types[variant.ty], seen)
            }),
            DfTy::Tuple(fields) => fields.iter().any(|field| {
                let ty = match field {
                    DfTupleField::Named { ty, .. } | DfTupleField::Positional(ty) => *ty,
                };
                seen.insert(ty) && contains_opaque(types, &types[ty], seen)
            }),
            DfTy::Fun(from, to) => {
                (seen.insert(*from) && contains_opaque(types, &types[*from], seen))
                    || (seen.insert(*to) && contains_opaque(types, &types[*to], seen))
            }
            DfTy::TyFun(_, body) => {
                seen.insert(*body) && contains_opaque(types, &types[*body], seen)
            }
            DfTy::TyApp(func, args) => {
                (seen.insert(*func) && contains_opaque(types, &types[*func], seen))
                    || args
                        .iter()
                        .any(|arg| seen.insert(*arg) && contains_opaque(types, &types[*arg], seen))
            }
            _ => false,
        }
    }

    match &module.entry_ty {
        DfTy::Fun(_, _) => Some(
            "compiled entry point returns a function, which cannot be shown by the runtime ABI",
        ),
        DfTy::Type => {
            Some("compiled entry point returns Type, which cannot be shown by the runtime ABI")
        }
        ty if contains_opaque(&module.types, ty, &mut rustc_hash::FxHashSet::default()) => Some(
            "compiled entry point returns an opaque host handle, which cannot be shown by the runtime ABI",
        ),
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
