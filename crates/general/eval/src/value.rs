use std::fmt;

use rustc_hash::FxHashMap;
use zutai_hir::SymbolId;

/// Runtime value placeholder for the future lazy interpreter.
#[derive(Debug, Clone)]
pub enum EvalValue {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Atom(String),
    List(Vec<EvalValue>),
    Tuple(Vec<TupleElemValue>),
    Record(FxHashMap<String, EvalValue>),
    Closure(ClosureValue),
    Thunk(ThunkValue),
    Type,
}

impl EvalValue {
    pub fn kind_name(&self) -> &'static str {
        match self {
            EvalValue::None => "None",
            EvalValue::Bool(_) => "Bool",
            EvalValue::Int(_) => "Int",
            EvalValue::Float(_) => "Float",
            EvalValue::Text(_) => "Text",
            EvalValue::Atom(_) => "Atom",
            EvalValue::List(_) => "List",
            EvalValue::Tuple(_) => "Tuple",
            EvalValue::Record(_) => "Record",
            EvalValue::Closure(_) => "Closure",
            EvalValue::Thunk(_) => "Thunk",
            EvalValue::Type => "Type",
        }
    }
}

#[derive(Debug, Clone)]
pub enum TupleElemValue {
    Positional(EvalValue),
    Named(String, EvalValue),
}

/// Captured function value.
#[derive(Debug, Clone)]
pub struct ClosureValue {
    pub symbol: Option<SymbolId>,
    pub captures: FxHashMap<SymbolId, EvalValue>,
}

/// Suspended computation.
#[derive(Debug, Clone)]
pub struct ThunkValue {
    pub state: ThunkState,
}

/// Memoization state for a thunk.
#[derive(Debug, Clone)]
pub enum ThunkState {
    Unevaluated,
    Evaluating,
    Evaluated(Box<EvalValue>),
}

impl fmt::Display for EvalValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalValue::None => f.write_str("none"),
            EvalValue::Bool(value) => write!(f, "{value}"),
            EvalValue::Int(value) => write!(f, "{value}"),
            EvalValue::Float(value) => write!(f, "{value}"),
            EvalValue::Text(value) => write!(f, "{value:?}"),
            EvalValue::Atom(value) => write!(f, "#{value}"),
            EvalValue::List(values) => {
                f.write_str("[")?;
                for (idx, value) in values.iter().enumerate() {
                    if idx > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{value}")?;
                }
                f.write_str("]")
            }
            EvalValue::Tuple(values) => {
                f.write_str("(")?;
                for (idx, value) in values.iter().enumerate() {
                    if idx > 0 {
                        f.write_str(", ")?;
                    }
                    match value {
                        TupleElemValue::Positional(value) => write!(f, "{value}")?,
                        TupleElemValue::Named(name, value) => write!(f, "{name} = {value}")?,
                    }
                }
                f.write_str(")")
            }
            EvalValue::Record(fields) => {
                f.write_str("{ ")?;
                for (idx, (name, value)) in fields.iter().enumerate() {
                    if idx > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{name} = {value}")?;
                }
                f.write_str("; }")
            }
            EvalValue::Closure(_) => f.write_str("<closure>"),
            EvalValue::Thunk(_) => f.write_str("<thunk>"),
            EvalValue::Type => f.write_str("Type"),
        }
    }
}
