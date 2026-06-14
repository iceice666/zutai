//! Runtime value model for the Zutai reference interpreter.
//!
//! All heap payloads use `Rc` so that `Value::clone()` is cheap regardless of
//! depth.  This module is deliberately IR-agnostic: nothing here imports THIR
//! directly; the THIR-specific eval walker lives in `eval.rs`.

use std::fmt;
use std::rc::Rc;

use zutai_hir::BindingId;
use zutai_thir::{ThirClause, TypeId};

use crate::{EvalError, env::Env};

/// A fully-evaluated or partially-applied Zutai runtime value.
#[derive(Clone, Debug)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(Rc<str>),
    Atom(Rc<str>),
    /// Lazy list — elements are thunks evaluated on demand.
    List(Rc<[crate::thunk::Thunk]>),
    /// Lazy tuple — items may be named.
    Tuple(Rc<[TupleField]>),
    /// Lazy record — only PRESENT fields are stored.
    Record(Rc<Vec<(Rc<str>, crate::thunk::Thunk)>>),
    Closure(Rc<Closure>),
    TypeValue(TypeId),
    /// Absent optional field / left-hand side of `??` that is absent.
    Nothing,
}

impl Value {
    /// Convert a parsed `.zti` immediate-mode value into a runtime value.
    ///
    /// Blocks become records and arrays become lists (per the import spec);
    /// every element is already fully evaluated, so its thunk is `ready`.
    pub fn from_immediate(value: &zutai_im::Value) -> Value {
        use zutai_im::Value as Im;
        match value {
            Im::True => Value::Bool(true),
            Im::False => Value::Bool(false),
            Im::Integer(n) => Value::Int(*n),
            Im::Float(f) => Value::Float(*f),
            Im::String(s) => Value::Text(Rc::from(s.as_str())),
            Im::Atom(s) => Value::Atom(Rc::from(s.as_str())),
            Im::Array(items) => Value::List(
                items
                    .iter()
                    .map(|item| crate::thunk::Thunk::ready(Value::from_immediate(item)))
                    .collect(),
            ),
            Im::Block(block) => Value::Record(Rc::new(
                block
                    .iter()
                    .map(|pair| {
                        (
                            Rc::from(pair.field_name.as_str()),
                            crate::thunk::Thunk::ready(Value::from_immediate(&pair.value)),
                        )
                    })
                    .collect(),
            )),
        }
    }
}

/// A named or positional tuple field carrying a lazy value.
#[derive(Clone, Debug)]
pub struct TupleField {
    pub name: Option<Rc<str>>,
    pub value: crate::thunk::Thunk,
}

/// A function (or partially-applied curried function).
#[derive(Clone, Debug)]
pub struct Closure {
    /// The `BindingId` of the top-level `Function` declaration this was built
    /// from, or `None` for an anonymous lambda.  Used only for display.
    pub binding: Option<BindingId>,
    /// Total number of value arguments the function expects (the number of
    /// `ThirPatId`s in `clauses[0].patterns`).
    pub arity: usize,
    /// All clauses of the function, shared across partial-application clones.
    pub clauses: Rc<[ThirClause]>,
    /// The environment captured at the point the closure was created.
    pub env: Env,
    /// Arguments already applied (thunks, in order).  Length < arity.
    pub applied: Vec<crate::thunk::Thunk>,
}

// ─── PartialEq ───────────────────────────────────────────────────────────────

/// Structural `PartialEq` for tests.  Container variants compare forced-thunk
/// contents (via `Thunk::peek`); after `eval_file`/`force_deep` all thunks are
/// in `Forced` state.  Closures compare by pointer identity.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Text(a), Value::Text(b)) => a == b,
            (Value::Atom(a), Value::Atom(b)) => a == b,
            (Value::Nothing, Value::Nothing) => true,
            (Value::TypeValue(a), Value::TypeValue(b)) => a == b,
            (Value::List(a), Value::List(b)) => a.len() == b.len()
                && a.iter().zip(b.iter()).all(
                    |(ta, tb)| matches!((ta.peek(), tb.peek()), (Some(va), Some(vb)) if va == vb),
                ),
            (Value::Tuple(a), Value::Tuple(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b.iter()).all(|(fa, fb)| {
                        fa.name == fb.name
                            && matches!((fa.value.peek(), fb.value.peek()),
                                (Some(va), Some(vb)) if va == vb)
                    })
            }
            (Value::Record(a), Value::Record(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                let mut fa: Vec<_> = a.iter().collect();
                let mut fb: Vec<_> = b.iter().collect();
                fa.sort_by_key(|(n, _)| n.as_ref() as *const str);
                fb.sort_by_key(|(n, _)| n.as_ref() as *const str);
                fa.iter().zip(fb.iter()).all(|((na, ta), (nb, tb))| {
                    na == nb && matches!((ta.peek(), tb.peek()), (Some(va), Some(vb)) if va == vb)
                })
            }
            (Value::Closure(a), Value::Closure(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

// ─── structural equality (runtime) ───────────────────────────────────────────

/// Structural equality for Zutai values.  Forces thunks as needed.
///
/// Returns `Err` only for non-comparable values (`Closure`, `TypeValue`),
/// which are unreachable for well-typed `==` expressions.
pub fn values_equal(
    a: &Value,
    b: &Value,
    ev: &crate::eval::Evaluator<'_>,
) -> Result<bool, EvalError> {
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => Ok(x == y),
        (Value::Int(x), Value::Int(y)) => Ok(x == y),
        (Value::Float(x), Value::Float(y)) => Ok(x == y),
        (Value::Text(x), Value::Text(y)) => Ok(x == y),
        (Value::Atom(x), Value::Atom(y)) => Ok(x == y),
        (Value::Nothing, Value::Nothing) => Ok(true),
        (Value::List(a), Value::List(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (ta, tb) in a.iter().zip(b.iter()) {
                let va = ta.force(ev)?;
                let vb = tb.force(ev)?;
                if !values_equal(&va, &vb, ev)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Tuple(a), Value::Tuple(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (fa, fb) in a.iter().zip(b.iter()) {
                if fa.name != fb.name {
                    return Ok(false);
                }
                let va = fa.value.force(ev)?;
                let vb = fb.value.force(ev)?;
                if !values_equal(&va, &vb, ev)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Record(a), Value::Record(b)) => {
            // Order-independent: sort by field name, then compare.
            let mut fa: Vec<_> = a.iter().collect();
            let mut fb: Vec<_> = b.iter().collect();
            fa.sort_by_key(|(n, _)| n.as_ref());
            fb.sort_by_key(|(n, _)| n.as_ref());
            if fa.len() != fb.len() {
                return Ok(false);
            }
            for ((na, ta), (nb, tb)) in fa.iter().zip(fb.iter()) {
                if na != nb {
                    return Ok(false);
                }
                let va = ta.force(ev)?;
                let vb = tb.force(ev)?;
                if !values_equal(&va, &vb, ev)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        // Closures and TypeValues are not comparable under `==` in well-typed
        // programs; this branch is an internal error if ever reached.
        (Value::Closure(_), _) | (_, Value::Closure(_)) => Err(EvalError::Internal(
            "equality on closure (unreachable in well-typed code)",
        )),
        (Value::TypeValue(_), _) | (_, Value::TypeValue(_)) => Err(EvalError::Internal(
            "equality on type value (unreachable in well-typed code)",
        )),
        _ => Ok(false),
    }
}

// ─── Display ─────────────────────────────────────────────────────────────────

/// Display a fully `force_deep`'d value.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(x) => {
                // Always include a decimal point for clarity.
                let s = format!("{x:?}"); // uses Rust's shortest round-trip repr
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    write!(f, "{s}")
                } else {
                    write!(f, "{s}.0")
                }
            }
            Value::Text(s) => {
                write!(f, "\"")?;
                for ch in s.chars() {
                    match ch {
                        '"' => write!(f, "\\\"")?,
                        '\\' => write!(f, "\\\\")?,
                        '\n' => write!(f, "\\n")?,
                        '\r' => write!(f, "\\r")?,
                        '\t' => write!(f, "\\t")?,
                        c => write!(f, "{c}")?,
                    }
                }
                write!(f, "\"")
            }
            Value::Atom(a) => write!(f, "#{a}"),
            Value::Nothing => write!(f, "#none"),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, t) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    // By the time Display is called the value should be
                    // force_deep'd; display whatever we have.
                    match t.peek() {
                        Some(v) => write!(f, "{v}")?,
                        None => write!(f, "<thunk>")?,
                    }
                }
                write!(f, "]")
            }
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, field) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if let Some(name) = &field.name {
                        write!(f, "{name} = ")?;
                    }
                    match field.value.peek() {
                        Some(v) => write!(f, "{v}")?,
                        None => write!(f, "<thunk>")?,
                    }
                }
                write!(f, ")")
            }
            Value::Record(fields) => {
                write!(f, "{{")?;
                for (i, (name, t)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, " {name} = ")?;
                    match t.peek() {
                        Some(v) => write!(f, "{v}")?,
                        None => write!(f, "<thunk>")?,
                    }
                }
                write!(f, " }}")
            }
            Value::Closure(c) => {
                // The HIR binding name isn't stored here; use the arity.
                write!(f, "<function/{}>", c.arity - c.applied.len())
            }
            Value::TypeValue(_) => write!(f, "<type>"),
        }
    }
}
