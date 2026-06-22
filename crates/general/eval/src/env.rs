//! Lexical environment for the Zutai reference interpreter.
//!
//! An `Env` is a singly-linked list of frames (parent-pointer tree), each
//! holding an `FxHashMap<BindingId, Thunk>`.  Each `Env` is an `Rc<Frame>` so
//! sharing the parent is cheap and closures can capture by cloning the `Env`.
//!
//! Interior mutability via `RefCell` lets the *top-level* letrec frame be
//! created first and then filled in with thunks that capture the frame
//! itself — allowing full mutual recursion without a second pass.
//!
//! This module is IR-agnostic.

use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;

use zutai_hir::BindingId;

use crate::{EvalError, thunk::Thunk};

/// A shared reference to a stack frame.
#[derive(Clone, Debug)]
pub struct Env(pub Rc<Frame>);

#[derive(Debug)]
pub struct Frame {
    pub bindings: RefCell<FxHashMap<BindingId, Thunk>>,
    pub parent: Option<Env>,
}

impl Env {
    /// Create a fresh empty environment with no parent.
    pub fn empty() -> Self {
        Env(Rc::new(Frame {
            bindings: RefCell::new(FxHashMap::default()),
            parent: None,
        }))
    }

    /// Create a child environment whose parent is `self`.
    pub fn push_frame(&self) -> Self {
        Env(Rc::new(Frame {
            bindings: RefCell::new(FxHashMap::default()),
            parent: Some(self.clone()),
        }))
    }

    /// Insert a binding into the *innermost* frame of this environment.
    pub fn insert(&self, id: BindingId, thunk: Thunk) {
        self.0.bindings.borrow_mut().insert(id, thunk);
    }

    /// Look up a binding, walking parent frames.  Returns `Err` if not found
    /// (should be unreachable for well-typed, fully gate-checked programs).
    pub fn lookup(&self, id: BindingId) -> Result<Thunk, EvalError> {
        let mut current = Some(self.clone());
        while let Some(env) = current {
            if let Some(t) = env.0.bindings.borrow().get(&id).cloned() {
                return Ok(t);
            }
            current = env.0.parent.clone();
        }
        Err(EvalError::UnboundBinding(id))
    }
}
