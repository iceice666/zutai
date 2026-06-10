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

use crate::{EvalError, env::Env, eval::Evaluator, value::Value};

/// A single lazily-evaluated expression.
#[derive(Clone, Debug)]
pub struct Thunk(pub Rc<RefCell<ThunkState>>);

#[derive(Clone, Debug)]
pub enum ThunkState {
    Unforced {
        expr: ThirExprId,
        env: Env,
    },
    /// Set while the thunk is being evaluated; detected as a black-hole.
    InProgress,
    Forced(Value),
}

impl Thunk {
    /// Create a deferred thunk (not yet evaluated).
    pub fn deferred(expr: ThirExprId, env: Env) -> Self {
        Thunk(Rc::new(RefCell::new(ThunkState::Unforced { expr, env })))
    }

    /// Create an already-forced thunk wrapping a known value.
    pub fn ready(value: Value) -> Self {
        Thunk(Rc::new(RefCell::new(ThunkState::Forced(value))))
    }

    /// Evaluate the thunk (if not already done) and return the value.
    ///
    /// Memoises on first call.  Subsequent calls are free clones.
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
        }

        // Extract the (expr, env) pair and mark as in-progress.
        let (expr, env) = {
            let mut state = self.0.borrow_mut();
            match std::mem::replace(&mut *state, ThunkState::InProgress) {
                ThunkState::Unforced { expr, env } => (expr, env),
                // Raced — shouldn't happen in single-threaded eval, but handle
                // defensively.
                ThunkState::InProgress => return Err(EvalError::BlackHole),
                ThunkState::Forced(v) => return Ok(v),
            }
        };
        // NOTE: borrow is dropped here before we call eval(), so there is no
        // outstanding borrow when eval() tries to force sub-expressions.

        let value = ev.eval(expr, &env)?;

        // Store the result.
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
