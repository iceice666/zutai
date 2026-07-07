use super::*;
use crate::posit::{posit_add, posit_cmp, posit_div, posit_mul, posit_sub};

pub(super) fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Posit(_) => "Posit",
        Value::Text(_) => "Text",
        Value::Atom(_) => "Atom",
        Value::List(_) => "List",
        Value::Tuple(_) => "Tuple",
        Value::Record(_) => "Record",
        Value::Closure(_) => "Function",
        Value::TlcClosure(_) => "Function",
        Value::Builtin(_) | Value::BuiltinPartial { .. } => "Function",
        Value::TypeValue(_) => "Type",
        Value::TaggedValue { .. } => "TaggedValue",
        Value::Nothing => "Nothing",
        Value::HostHandle(handle) => match handle.kind {
            HostHandleKind::Reader => "Reader",
            HostHandleKind::Writer => "Writer",
        },
        Value::WitnessDict(_) => "WitnessDict",
    }
}

/// Helper for arithmetic binary ops with overflow-checking for `Int` and
/// IEEE semantics for `Float`.
pub(super) fn numeric_binop(
    lv: Value,
    rv: Value,
    int_op: fn(i64, i64) -> Option<i64>,
    float_op: fn(f64, f64) -> f64,
    op_name: &'static str,
) -> Result<Value, EvalError> {
    match (lv, rv) {
        (Value::Int(a), Value::Int(b)) => int_op(a, b)
            .map(Value::Int)
            .ok_or(EvalError::IntOverflow(op_name)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(a, b))),
        (Value::Posit(a), Value::Posit(b)) => {
            let value = match op_name {
                "+" => posit_add(a, b)?,
                "-" => posit_sub(a, b)?,
                "*" => posit_mul(a, b)?,
                "/" => posit_div(a, b)?,
                _ => return Err(EvalError::Internal("unknown posit arithmetic operator")),
            };
            Ok(Value::Posit(value))
        }
        (a, b) => Err(EvalError::TypeMismatch {
            expected: "Int, Float, or Posit",
            found: if matches!(a, Value::Int(_) | Value::Float(_) | Value::Posit(_)) {
                value_type_name(&b)
            } else {
                value_type_name(&a)
            },
        }),
    }
}

/// Comparison operators for Int, Float, and Text.
pub(super) fn cmp_op(
    lv: Value,
    rv: Value,
    target: std::cmp::Ordering,
    or_equal: bool,
) -> Result<Value, EvalError> {
    let ord = match (&lv, &rv) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Float(a), Value::Float(b)) => match a.partial_cmp(b) {
            Some(o) => o,
            // IEEE 754: NaN is unordered, so `<`/`<=`/`>`/`>=` are all false.
            None => return Ok(Value::Bool(false)),
        },
        (Value::Posit(a), Value::Posit(b)) => posit_cmp(*a, *b)?,
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        _ => {
            return Err(EvalError::TypeMismatch {
                expected: "Int, Float, Posit, or Text",
                found: value_type_name(&lv),
            });
        }
    };
    let result = ord == target || (or_equal && ord == std::cmp::Ordering::Equal);
    Ok(Value::Bool(result))
}

// Float arithmetic via std ops.
pub(super) trait FloatBinOp {
    fn add(a: f64, b: f64) -> f64;
    fn sub(a: f64, b: f64) -> f64;
    fn mul(a: f64, b: f64) -> f64;
    fn div(a: f64, b: f64) -> f64;
}

impl FloatBinOp for f64 {
    fn add(a: f64, b: f64) -> f64 {
        a + b
    }
    fn sub(a: f64, b: f64) -> f64 {
        a - b
    }
    fn mul(a: f64, b: f64) -> f64 {
        a * b
    }
    fn div(a: f64, b: f64) -> f64 {
        a / b
    }
}
