use super::*;

// ─── errors ───────────────────────────────────────────────────────────────────

fn indent_msgs(msgs: &[String]) -> String {
    msgs.iter().map(|m| format!("\n  {m}")).collect()
}

#[derive(thiserror::Error, Debug, Clone, PartialEq)]
pub enum EvalError {
    /// Source program has parse or HIR errors.
    #[error("program has errors and cannot be evaluated:{}", indent_msgs(.0))]
    NotRunnable(Vec<String>),
    /// THIR type checking failed or is incomplete.
    #[error("type checking failed:{}", indent_msgs(.0))]
    TypeCheckFailed(Vec<String>),
    /// A `ThirExprKind::Error` node was reachable in a nominally-complete THIR.
    #[error("internal: reachable Error node in type-checked THIR")]
    ErrorNodeReachable,
    /// Program uses an effect form unsupported by the selected evaluator.
    #[error("cannot run: {0}")]
    EffectfulNotExecutable(String),
    /// Runtime reflection was asked to inspect a type outside the supported,
    /// serializable Phase 17 subset.
    #[error("reflection unsupported: {0}")]
    ReflectionUnsupported(String),
    /// An effect operation escaped all source handlers and the host boundary.
    #[error("runtime error: unhandled effect `{0}`")]
    UnhandledEffect(String),
    /// `resume` reached runtime without an operation continuation.
    #[error("runtime error: resume outside an operation handler")]
    ResumeOutsideHandler,
    /// Runtime black-hole: a non-productive recursive binding was forced.
    #[error("runtime error: non-productive recursive definition (black hole)")]
    BlackHole,
    /// Division by zero in integer division.
    #[error("runtime error: integer division by zero")]
    DivByZero,
    /// Integer overflow.
    #[error("runtime error: integer overflow in `{0}`")]
    IntOverflow(&'static str),
    /// No clause of a function matched the arguments.
    #[error("runtime error: no matching clause (non-exhaustive pattern match)")]
    NoMatchingClause,
    /// An unbound `BindingId` was looked up.
    ///
    /// Unreachable in fully-evaluated well-typed code **except** for constraint
    /// method calls with no matching witness in scope: dispatch is attempted at
    /// the `Apply` node using the instantiation's type key, but when no witness
    /// field matches the interpreter refuses rather than guessing a value.
    #[error("internal: unbound binding {0:?}")]
    UnboundBinding(BindingId),
    /// Runtime type mismatch (unreachable in well-typed code).
    #[error("internal: type mismatch — expected {expected}, found {found}")]
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },
    /// Internal invariant violated (always a bug in the interpreter).
    #[error("internal error: {0}")]
    Internal(&'static str),
    /// A constraint method was called inside a polymorphic function but no
    /// witness could be resolved — the function was likely called indirectly,
    /// where witness injection via env is not yet supported.
    ///
    /// This is a deliberate limitation of the oracle, not a bug in the user's
    /// program. Full dictionary-passing is deferred to the TLC elaboration layer.
    #[error(
        "eval limitation: cannot resolve witness for method `{method}` in indirect call (dictionary-passing deferred to TLC)"
    )]
    UnresolvedWitness { method: String },
}
