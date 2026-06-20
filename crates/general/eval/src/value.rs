//! Runtime value model for the Zutai reference interpreter.
//!
//! All heap payloads use `Rc` so that `Value::clone()` is cheap regardless of
//! depth.  This module is deliberately IR-agnostic: nothing here imports THIR
//! directly; the THIR-specific eval walker lives in `eval.rs`.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use zutai_hir::BindingId;
use zutai_thir::{ThirClause, TypeId};
use zutai_tlc::TlcExprId;

use crate::{EvalError, env::Env};

/// Index into the module registry held by the evaluator.
///
/// Each evaluated `.zt` module is assigned a `ModuleId` so that closures and
/// thunks can record their home module and re-enter the correct arena when
/// forced or applied across a module boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub usize);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeType {
    pub module: ModuleId,
    pub ty: TypeId,
    pub subst: Rc<[(BindingId, RuntimeType)]>,
}

impl RuntimeType {
    pub fn new(module: ModuleId, ty: TypeId) -> Self {
        Self {
            module,
            ty,
            subst: Rc::from([]),
        }
    }

    pub fn with_subst(module: ModuleId, ty: TypeId, subst: Rc<[(BindingId, RuntimeType)]>) -> Self {
        Self { module, ty, subst }
    }

    pub fn with_ty(&self, ty: TypeId) -> Self {
        Self {
            module: self.module,
            ty,
            subst: self.subst.clone(),
        }
    }
}

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
    TypeValue(RuntimeType),
    /// A tagged union value: `#tag { field = value; ... }`.
    TaggedValue {
        tag: Rc<str>,
        payload: Rc<Vec<(Rc<str>, crate::thunk::Thunk)>>,
    },
    /// Absent optional field / left-hand side of `??` that is absent.
    Nothing,
    /// A resolved constraint witness dictionary mapping method/operator name to
    /// the evaluated closure for that field.  Injected into the environment at
    /// bounded call sites so that method dispatch inside the body can fall back
    /// to this dict when the type key is a TypeVar at the call site.
    WitnessDict(HashMap<String, Value>),
    /// A closure created by the TLC evaluator — stores a single-parameter lambda
    /// body and its captured environment.  Distinct from `Closure` (which is
    /// THIR-based) so the two evaluators never confuse each other's closures.
    TlcClosure(Rc<TlcClosure>),
    /// A compiler-provided builtin function value (the prelude). Seeded into the
    /// top-level environment by name; applied specially by both evaluators.
    Builtin(BuiltinFn),
}

/// A compiler-provided builtin function. `print` is re-pointed to the
/// `io.print` effect by the TLC evaluator; source handlers can intercept it and
/// the host run boundary handles residual `io.print`. `fields` and `schema`
/// reflect normalized type values through the THIR evaluator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinFn {
    Print,
    Fields,
    Schema,
}

impl BuiltinFn {
    /// Resolve a prelude builtin by its binding name. Mirrors
    /// `zutai_hir::BUILTIN_VALUE_NAMES`; returns `None` for any other name.
    pub fn from_name(name: &str) -> Option<BuiltinFn> {
        match name {
            "print" => Some(BuiltinFn::Print),
            "fields" => Some(BuiltinFn::Fields),
            "schema" => Some(BuiltinFn::Schema),
            _ => None,
        }
    }
}

/// A single-parameter closure produced by the TLC evaluator.
#[derive(Clone, Debug)]
pub struct TlcClosure {
    pub param: BindingId,
    pub body: TlcExprId,
    pub env: Env,
    pub home: ModuleId,
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
    /// The module in whose arena the clauses' `ThirExprId`s / `ThirPatId`s live.
    /// `apply_closure` switches the active module to this before evaluating any
    /// clause body or guard so arena look-ups hit the right file.
    pub home: ModuleId,
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
            (
                Value::TaggedValue {
                    tag: ta,
                    payload: pa,
                },
                Value::TaggedValue {
                    tag: tb,
                    payload: pb,
                },
            ) => {
                ta == tb
                    && pa.len() == pb.len()
                    && pa.iter().zip(pb.iter()).all(|((na, va), (nb, vb))| {
                        na == nb
                            && matches!((va.peek(), vb.peek()), (Some(xa), Some(xb)) if xa == xb)
                    })
            }
            (Value::Closure(a), Value::Closure(b)) => Rc::ptr_eq(a, b),
            (Value::TlcClosure(a), Value::TlcClosure(b)) => Rc::ptr_eq(a, b),
            (Value::Builtin(a), Value::Builtin(b)) => a == b,
            // WitnessDicts are opaque to user-level equality.
            (Value::WitnessDict(_), _) | (_, Value::WitnessDict(_)) => false,
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
        (
            Value::TaggedValue {
                tag: ta,
                payload: pa,
            },
            Value::TaggedValue {
                tag: tb,
                payload: pb,
            },
        ) => {
            if ta != tb {
                return Ok(false);
            }
            let mut fa: Vec<_> = pa.iter().collect();
            let mut fb: Vec<_> = pb.iter().collect();
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
        (Value::TlcClosure(_), _) | (_, Value::TlcClosure(_)) => Err(EvalError::Internal(
            "equality on TLC closure (unreachable in well-typed code)",
        )),
        (Value::TypeValue(_), _) | (_, Value::TypeValue(_)) => Err(EvalError::Internal(
            "equality on type value (unreachable in well-typed code)",
        )),
        // WitnessDicts are internal; comparing them is an internal error.
        (Value::WitnessDict(_), _) | (_, Value::WitnessDict(_)) => Err(EvalError::Internal(
            "equality on witness dict (unreachable in well-typed code)",
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
                // Non-finite floats (`inf`, `-inf`, `NaN`) have no integer
                // ambiguity, so emit the bare form rather than appending `.0`
                // (which would produce the malformed `inf.0` / `NaN.0`).
                let s = format!("{x:?}"); // Rust's shortest round-trip repr
                if !x.is_finite() || s.contains('.') || s.contains('e') || s.contains('E') {
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
            Value::TaggedValue { tag, payload } => {
                write!(f, "#{tag}")?;
                if !payload.is_empty() {
                    let positional = payload
                        .iter()
                        .enumerate()
                        .all(|(index, (name, _))| name.parse::<usize>() == Ok(index));
                    if positional {
                        write!(f, " (")?;
                        for (i, (_, t)) in payload.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            match t.peek() {
                                Some(v) => write!(f, "{v}")?,
                                None => write!(f, "<thunk>")?,
                            }
                        }
                        write!(f, ")")?;
                    } else {
                        write!(f, " {{")?;
                        for (i, (name, t)) in payload.iter().enumerate() {
                            if i > 0 {
                                write!(f, ";")?;
                            }
                            write!(f, " {name} = ")?;
                            match t.peek() {
                                Some(v) => write!(f, "{v}")?,
                                None => write!(f, "<thunk>")?,
                            }
                        }
                        write!(f, " }}")?;
                    }
                }
                Ok(())
            }
            Value::Closure(c) => {
                // The HIR binding name isn't stored here; use the arity.
                write!(f, "<function/{}>", c.arity - c.applied.len())
            }
            Value::TlcClosure(_) => write!(f, "<function/1>"),
            Value::TypeValue(_) => write!(f, "<type>"),
            Value::WitnessDict(_) => write!(f, "<witness>"),
            Value::Builtin(BuiltinFn::Print) => write!(f, "<builtin print>"),
            Value::Builtin(BuiltinFn::Fields) => write!(f, "<builtin fields>"),
            Value::Builtin(BuiltinFn::Schema) => write!(f, "<builtin schema>"),
        }
    }
}
