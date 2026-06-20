//! Call-by-need thunks for the Zutai reference interpreter.
//!
//! A `Thunk` is an `Rc<RefCell<ThunkState>>`.  All `BindingRef`s that refer to
//! the same binding share the same `Rc`, so the first `force()` memoises the
//! result and all subsequent forces return the cached value — call-by-need
//! sharing.
//!
//! Black-hole detection: when `force()` begins evaluation it sets the state to
//! `InProgress`.  If the same thunk is forced again before the first evaluation
//! finishes (non-productive recursion such as `x :: Int = x`) it returns
//! `Err(EvalError::BlackHole)` instead of overflowing the stack.
//!
//! This module is IR-agnostic.

use std::cell::RefCell;
use std::rc::Rc;

use zutai_thir::ThirExprId;
use zutai_tlc::TlcExprId;

use crate::{
    EvalError,
    env::Env,
    eval::Evaluator,
    value::{ModuleId, Value},
};

/// A single lazily-evaluated expression.
#[derive(Clone, Debug)]
pub struct Thunk(pub Rc<RefCell<ThunkState>>);

#[derive(Clone, Debug)]
pub enum ThunkState {
    Unforced {
        expr: ThirExprId,
        env: Env,
        /// The module in whose arena `expr` lives.  `force()` switches the
        /// evaluator to this module before evaluating.
        home: ModuleId,
    },
    TlcUnforced {
        expr: TlcExprId,
        env: Env,
        /// The module in whose TLC arena `expr` lives.
        home: ModuleId,
    },
    /// Set while the thunk is being evaluated; detected as a black-hole.
    InProgress,
    Forced(Value),
}

impl Thunk {
    /// Create a deferred thunk (not yet evaluated).
    pub fn deferred(expr: ThirExprId, env: Env, home: ModuleId) -> Self {
        Thunk(Rc::new(RefCell::new(ThunkState::Unforced {
            expr,
            env,
            home,
        })))
    }

    /// Create a deferred TLC thunk (not yet evaluated).
    pub fn tlc_deferred(expr: TlcExprId, env: Env, home: ModuleId) -> Self {
        Thunk(Rc::new(RefCell::new(ThunkState::TlcUnforced {
            expr,
            env,
            home,
        })))
    }

    /// Create an in-progress placeholder for recursive bindings.
    pub fn in_progress() -> Self {
        Thunk(Rc::new(RefCell::new(ThunkState::InProgress)))
    }

    /// Create an already-forced thunk wrapping a known value.
    pub fn ready(value: Value) -> Self {
        Thunk(Rc::new(RefCell::new(ThunkState::Forced(value))))
    }

    /// Evaluate the thunk (if not already done) and return the value.
    ///
    /// Memoises on first call.  Subsequent calls are free clones.
    /// Evaluates against the thunk's home module so arena look-ups are correct
    /// even when the thunk was created in a different module from the caller.
    pub fn force(&self, ev: &Evaluator<'_>) -> Result<Value, EvalError> {
        // Fast path: already forced.
        {
            let state = self.0.borrow();
            if let ThunkState::Forced(v) = &*state {
                return Ok(v.clone());
            }
            if matches!(*state, ThunkState::InProgress) {
                return Err(EvalError::BlackHole);
            }
            if matches!(*state, ThunkState::TlcUnforced { .. }) {
                return Err(EvalError::Internal("TLC thunk reached THIR force"));
            }
        }

        // Extract the (expr, env, home) tuple and mark as in-progress.
        let (expr, env, home) = {
            let mut state = self.0.borrow_mut();
            match std::mem::replace(&mut *state, ThunkState::InProgress) {
                ThunkState::Unforced { expr, env, home } => (expr, env, home),
                ThunkState::TlcUnforced { .. } => {
                    return Err(EvalError::Internal("TLC thunk reached THIR force"));
                }
                // Raced — shouldn't happen in single-threaded eval, but handle
                // defensively.
                ThunkState::InProgress => return Err(EvalError::BlackHole),
                ThunkState::Forced(v) => return Ok(v),
            }
        };
        // NOTE: borrow is dropped here before we call eval(), so there is no
        // outstanding borrow when eval() tries to force sub-expressions.

        // Evaluate in the home module's arena.
        let value = ev.for_module(home).eval(expr, &env)?;

        // Store the result.
        *self.0.borrow_mut() = ThunkState::Forced(value.clone());
        Ok(value)
    }

    pub fn replace_forced(&self, value: Value) {
        *self.0.borrow_mut() = ThunkState::Forced(value);
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(*self.0.borrow(), ThunkState::InProgress)
    }

    pub fn force_tlc(&self, ev: &crate::eval_tlc::TlcEvaluator<'_>) -> Result<Value, EvalError> {
        {
            let state = self.0.borrow();
            match &*state {
                ThunkState::Forced(v) => return Ok(v.clone()),
                ThunkState::InProgress => return Err(EvalError::BlackHole),
                ThunkState::Unforced { .. } => {
                    return Err(EvalError::Internal("THIR thunk reached TLC force"));
                }
                ThunkState::TlcUnforced { .. } => {}
            }
        }

        let (expr, env, home) = {
            let mut state = self.0.borrow_mut();
            match std::mem::replace(&mut *state, ThunkState::InProgress) {
                ThunkState::TlcUnforced { expr, env, home } => (expr, env, home),
                ThunkState::Unforced { .. } => {
                    return Err(EvalError::Internal("THIR thunk reached TLC force"));
                }
                ThunkState::InProgress => return Err(EvalError::BlackHole),
                ThunkState::Forced(v) => return Ok(v),
            }
        };

        let value = ev.for_module(home)?.eval_expr(expr, &env)?;
        *self.0.borrow_mut() = ThunkState::Forced(value.clone());
        Ok(value)
    }

    /// Peek at the current state without forcing.  Returns `Some(value)` if
    /// already forced, `None` otherwise.  Used only for Display.
    pub fn peek(&self) -> Option<Value> {
        match &*self.0.borrow() {
            ThunkState::Forced(v) => Some(v.clone()),
            _ => None,
        }
    }
}
