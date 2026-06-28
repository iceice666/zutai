//! THIR tree-walk evaluator — the THIR-specific reference oracle in this crate.
//!
//! The parallel `eval_tlc.rs` walker evaluates TLC modules for compiler-path
//! parity checks while reusing the same `value`, `thunk`, and `env` runtime
//! structures.
//!
//! The `Evaluator` holds a module registry (`&[ThirFile]`) plus an
//! `active_module: ModuleId` index.  Arena helpers route through the active
//! file.  When applying a cross-module closure, callers use `for_module(home)`
//! to obtain a copy with the correct active file before calling `eval`.

use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use zutai_hir::BindingId;
use zutai_syntax::ast::BinOp;
use zutai_thir::ir::UnionVariant;
use zutai_thir::{
    ImportKey, ThirClause, ThirDeclId, ThirDeclKind, ThirExprId, ThirExprKind, ThirFile, ThirPatId,
};
use zutai_thir::{
    RowTail, ThirPatKind, ThirTupleItem, ThirTuplePatItem, Type, TypeId, TypeKind, TypeRecordField,
    TypeTupleItem,
};

use crate::{
    EvalError,
    env::Env,
    force_deep,
    thunk::Thunk,
    value::{
        BuiltinFn, Closure, ModuleId, RuntimeType, TupleField, Value, eval_num_builtin_values,
        eval_text_builtin_values, overlay_value, update_record_value, values_equal,
    },
};

mod apply;
mod binary;
mod dispatch;
mod expr;
mod ops;
mod reflection;
mod top_env;
mod type_key;
use ops::{FloatBinOp, cmp_op, numeric_binop, value_type_name};
pub(crate) use smallvec::{SmallVec, smallvec};
use type_key::{key_is_ambiguous, resolve_alias_chain, type_key};

/// A slice of all evaluated modules for this run, keyed by position = `ModuleId`.
pub type ModuleRegistry = Vec<Arc<ThirFile>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWitness {
    pub module: ModuleId,
    pub constraint: String,
    pub target_key: String,
}

pub(crate) type AliasMap = FxHashMap<BindingId, (Vec<BindingId>, TypeId)>;

/// Per-run memoization shared across every `Evaluator` copy (the evaluator is
/// `Copy` and is recreated constantly via `for_module`, so the cache lives in a
/// longer-lived owner referenced by `&'a`). Keyed by `ModuleId` because each
/// module's THIR arenas are independent; a recompute is byte-identical, so this
/// is pure memoization with no behavioral effect.
#[derive(Default)]
pub(crate) struct EvalCaches {
    alias_maps: RefCell<FxHashMap<ModuleId, Rc<AliasMap>>>,
    type_keys: RefCell<FxHashMap<(ModuleId, TypeId), Rc<str>>>,
}

/// Holds read-only access to the THIR arenas while evaluating.
///
/// `Evaluator` is cheaply `Copy` — it's two references plus a `usize`.
/// Use `for_module(m)` to get a copy that operates in module `m`'s arenas.
#[derive(Clone, Copy)]
pub struct Evaluator<'a> {
    file: &'a ThirFile,
    registry: &'a [Arc<ThirFile>],
    active_module: ModuleId,
    imports: &'a FxHashMap<ImportKey, Value>,
    witnesses: &'a [RuntimeWitness],
    caches: &'a EvalCaches,
}

impl<'a> Evaluator<'a> {
    pub(crate) fn new(
        file: &'a ThirFile,
        registry: &'a [Arc<ThirFile>],
        active_module: ModuleId,
        imports: &'a FxHashMap<ImportKey, Value>,
        witnesses: &'a [RuntimeWitness],
        caches: &'a EvalCaches,
    ) -> Self {
        Self {
            file,
            registry,
            active_module,
            imports,
            witnesses,
            caches,
        }
    }

    /// Return a copy of this evaluator re-pointed at module `m`.
    ///
    /// Used by `apply_closure` and `Thunk::force` to switch arenas when
    /// evaluating a closure or thunk that was created in a different module.
    pub fn for_module(&self, m: ModuleId) -> Self {
        Self {
            file: &self.registry[m.0],
            active_module: m,
            registry: self.registry,
            imports: self.imports,
            witnesses: self.witnesses,
            caches: self.caches,
        }
    }

    /// Create a deferred thunk for `expr` in the current module.
    ///
    /// Stamps `home = self.active_module` so the thunk evaluates against the
    /// correct arena regardless of which module forces it later.
    pub fn defer(&self, expr: ThirExprId, env: Env) -> Thunk {
        Thunk::deferred(expr, env, self.active_module)
    }

    // ── arena helpers ────────────────────────────────────────────────────────

    fn expr(&self, id: ThirExprId) -> &'a zutai_thir::ThirExpr {
        &self.file.expr_arena[id]
    }

    fn pat(&self, id: ThirPatId) -> &'a zutai_thir::ThirPat {
        &self.file.pat_arena[id]
    }

    fn decl(&self, id: ThirDeclId) -> &'a zutai_thir::ThirDecl {
        &self.file.decl_arena[id]
    }
}
