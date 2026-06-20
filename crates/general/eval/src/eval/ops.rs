use super::*;

pub(super) fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
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
        (a, b) => Err(EvalError::TypeMismatch {
            expected: "Int or Float",
            found: if matches!(a, Value::Int(_) | Value::Float(_)) {
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
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Less),
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        _ => {
            return Err(EvalError::TypeMismatch {
                expected: "Int, Float, or Text",
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
