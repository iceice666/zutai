//! Eager (call-by-value) TLC evaluator.
//!
//! Walks a `TlcModule` produced by `zutai-tlc::lower_thir`.  Because TLC has
//! fully elaborated all type abstractions, the evaluator skips `TyLam`/`TyApp`
//! (type-erasure semantics) and dispatches constraint methods via `GetField` on
//! the already-injected dict record — no witness-resolution needed at eval time.
//!
//! Phase 16 adds algebraic-effect execution with delimited continuations:
//! `perform` suspends the current TLC continuation, source `handle` clauses may
//! return directly or `resume`, and the host boundary handles residual
//! `io.print`. All produced values are wrapped in `Thunk::ready(…)`, so
//! `peek()` always returns `Some`; there are no deferred thunks in TLC
//! evaluation.

use rustc_hash::FxHashMap;
use std::rc::Rc;

use zutai_thir::ImportKey;
use zutai_tlc::{
    BuiltinOp, Literal, Row, TlcAlt, TlcDecl, TlcExpr, TlcExprId, TlcHandleClause, TlcModule,
    TlcPat, TlcPatItem, TlcTupleItem, TlcType, TlcTypeId, TlcTypeVar,
};

use crate::{
    EvalError,
    env::Env,
    thunk::Thunk,
    value::{
        BuiltinFn, ModuleId, TlcClosure, TupleField, Value, overlay_value, update_record_value,
    },
};

type EvalCont<'eval> = Rc<dyn Fn(Value) -> Result<EvalControl<'eval>, EvalError> + 'eval>;
type BindFn<'eval, 'module> =
    Rc<dyn Fn(Value, TlcEvaluator<'module>) -> Result<EvalControl<'eval>, EvalError> + 'eval>;
type Finalizers<'eval> = SmallVec<[EvalCont<'eval>; 1]>;
type FinishValues<'eval> = Rc<dyn Fn(Vec<Value>) -> Value + 'eval>;
pub type TlcModuleRegistry<'a> = Vec<&'a TlcModule>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TlcWrapperKind {
    Optional,
    Maybe,
}

enum EvalControl<'eval> {
    Value(Value),
    Perform {
        op: String,
        arg: Value,
        /// Enclosing `finally` teardowns this suspended effect sits inside,
        /// ordered inner-to-outer. If a handler aborts this effect (returns
        /// without `resume`), the continuation is discarded, so these teardowns
        /// must be unwound explicitly before the abort value escapes.
        finalizers: Finalizers<'eval>,
        cont: EvalCont<'eval>,
    },
    /// A pending tail call: evaluate `id` under `ev`/`env` in tail position.
    /// Returned by `eval_step` for every tail position and bounced by the
    /// `eval_control` driver loop, so a chain of tail calls runs in constant
    /// host-stack space instead of one Rust frame per call.
    Tail {
        ev: TlcEvaluator<'eval>,
        id: TlcExprId,
        env: Env,
        resume: Option<EvalCont<'eval>>,
    },
}

/// Bounce every `EvalControl::Tail` to a fixpoint, returning a settled control
/// (`Value` or `Perform`). This is the tail-call trampoline: each loop turn runs
/// exactly one `eval_step`, so a tail-recursive call chain costs one heap-light
/// `Tail` per step instead of one Rust stack frame. Every site that matches on
/// an `EvalControl` settles it first, so a `Tail` never escapes into a matcher.
fn settle<'eval>(mut ctrl: EvalControl<'eval>) -> Result<EvalControl<'eval>, EvalError> {
    while let EvalControl::Tail {
        ev,
        id,
        env,
        resume,
    } = ctrl
    {
        ctrl = ev.eval_step(id, &env, resume)?;
    }
    Ok(ctrl)
}

#[derive(Clone, Copy)]
pub struct TlcEvaluator<'a> {
    pub module: &'a TlcModule,
    registry: Option<&'a [&'a TlcModule]>,
    active_module: ModuleId,
    imports: Option<&'a FxHashMap<ImportKey, Value>>,
    operator_witnesses: Option<&'a FxHashMap<(String, String), Value>>,
    defer_aggregates: bool,
}

mod aggregate;
mod builtin;
mod effects;
mod expr;
mod force;
mod pattern;
mod top_env;
mod type_meta;
pub use builtin::eval_literal;
pub use force::tlc_force_deep;

use builtin::{lit_matches, tlc_module_can_defer_aggregates, value_cont, value_type_name};
pub(crate) use smallvec::SmallVec;

impl<'a> TlcEvaluator<'a> {
    pub fn new(module: &'a TlcModule) -> Self {
        Self {
            module,
            registry: None,
            active_module: ModuleId(0),
            imports: None,
            operator_witnesses: None,
            defer_aggregates: tlc_module_can_defer_aggregates(module),
        }
    }

    pub fn new_with_imports(
        module: &'a TlcModule,
        imports: &'a FxHashMap<ImportKey, Value>,
    ) -> Self {
        Self {
            module,
            registry: None,
            active_module: ModuleId(0),
            imports: Some(imports),
            operator_witnesses: None,
            defer_aggregates: tlc_module_can_defer_aggregates(module),
        }
    }

    pub fn new_in_registry(
        registry: &'a [&'a TlcModule],
        active_module: ModuleId,
        imports: &'a FxHashMap<ImportKey, Value>,
    ) -> Result<Self, EvalError> {
        let module = registry
            .get(active_module.0)
            .copied()
            .ok_or(EvalError::Internal("TLC module id out of registry bounds"))?;
        Ok(Self {
            module,
            registry: Some(registry),
            active_module,
            imports: Some(imports),
            operator_witnesses: None,
            defer_aggregates: tlc_module_can_defer_aggregates(module),
        })
    }

    pub fn new_in_registry_with_operator_witnesses(
        registry: &'a [&'a TlcModule],
        active_module: ModuleId,
        imports: &'a FxHashMap<ImportKey, Value>,
        operator_witnesses: &'a FxHashMap<(String, String), Value>,
    ) -> Result<Self, EvalError> {
        let module = registry
            .get(active_module.0)
            .copied()
            .ok_or(EvalError::Internal("TLC module id out of registry bounds"))?;
        Ok(Self {
            module,
            registry: Some(registry),
            active_module,
            imports: Some(imports),
            operator_witnesses: Some(operator_witnesses),
            defer_aggregates: tlc_module_can_defer_aggregates(module),
        })
    }
    pub(crate) fn for_module(&self, home: ModuleId) -> Result<Self, EvalError> {
        if home == self.active_module {
            return Ok(Self {
                module: self.module,
                registry: self.registry,
                active_module: self.active_module,
                imports: self.imports,
                operator_witnesses: self.operator_witnesses,
                defer_aggregates: self.defer_aggregates,
            });
        }
        let registry = self.registry.ok_or(EvalError::Internal(
            "TLC closure escaped without module registry",
        ))?;
        let module = registry
            .get(home.0)
            .copied()
            .ok_or(EvalError::Internal("TLC module id out of registry bounds"))?;
        Ok(Self {
            module,
            registry: self.registry,
            active_module: home,
            imports: self.imports,
            operator_witnesses: self.operator_witnesses,
            defer_aggregates: tlc_module_can_defer_aggregates(module),
        })
    }

    pub fn eval_expr(&self, id: TlcExprId, env: &Env) -> Result<Value, EvalError> {
        let control = (*self).eval_control(id, env, None)?;
        (*self).finish_top(control)
    }

    /// Apply a function value to one argument and drive it to a settled value.
    /// Used to instantiate an imported conditional witness by applying its
    /// curried dictionary function to recursively-resolved component dicts.
    pub fn apply_to_value(&self, fv: Value, arg: Value) -> Result<Value, EvalError> {
        let control = (*self).apply(fv, arg, None)?;
        (*self).finish_top(control)
    }
}
