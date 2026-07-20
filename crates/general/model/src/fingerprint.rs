//! First-order state/action validation and canonical fingerprinting.
//!
//! A state or action is fingerprinted by its canonical [`Value::Display`]
//! string, which name-sorts records and record-style tagged payloads and keeps
//! lists/tuples positional. That single deterministic formatter is the
//! visited-set key and the diagnostic rendering; this module never maintains a
//! second formatter. Every value is deep-forced and recursively validated
//! against the accepted first-order grammar before `to_string()` is called, so
//! a closure, builtin, type value, witness, host handle, internal `Nothing`,
//! `Float`, or `Posit` anywhere inside is rejected rather than rendered.

use zutai_eval::{TlcSession, Value};

use crate::ModelError;

/// Deep-force `value`, reject any non-first-order content, and return the
/// canonical display string used as both the visited-set key and the witness
/// rendering. `in_action` selects the `state`/`action` error wording.
pub(crate) fn fingerprint(
    session: &TlcSession,
    value: Value,
    in_action: bool,
) -> Result<(Value, String), ModelError> {
    let value = session.force(value)?;
    validate(&value, in_action)?;
    let rendered = value.to_string();
    Ok((value, rendered))
}

/// Recursively check that a deep-forced value uses only `Bool`, `Int`, `Text`,
/// `Atom`, `List`, `Tuple`, `Record`, and `TaggedValue`.
fn validate(value: &Value, in_action: bool) -> Result<(), ModelError> {
    match value {
        Value::Bool(_) | Value::Int(_) | Value::Text(_) | Value::Atom(_) => Ok(()),
        Value::List(items) => {
            for item in items.iter() {
                validate(&peek(item, in_action)?, in_action)?;
            }
            Ok(())
        }
        Value::Tuple(items) => {
            for field in items.iter() {
                validate(&peek(&field.value, in_action)?, in_action)?;
            }
            Ok(())
        }
        Value::Record(fields) => {
            for (_, thunk) in fields.iter() {
                validate(&peek(thunk, in_action)?, in_action)?;
            }
            Ok(())
        }
        Value::TaggedValue { payload, .. } => {
            for (_, thunk) in payload.iter() {
                validate(&peek(thunk, in_action)?, in_action)?;
            }
            Ok(())
        }
        other => Err(reject(value_kind(other), in_action)),
    }
}

/// Peek an already deep-forced thunk. A deep force settles every reachable
/// thunk, so an unforced peek here means an internal invariant broke.
fn peek(thunk: &zutai_eval::Thunk, in_action: bool) -> Result<Value, ModelError> {
    thunk
        .peek()
        .ok_or_else(|| reject("<unforced thunk>", in_action))
}

fn reject(kind: &'static str, in_action: bool) -> ModelError {
    if in_action {
        ModelError::NonFirstOrderAction(kind)
    } else {
        ModelError::NonFirstOrderState(kind)
    }
}

/// Human-readable runtime kind name for diagnostics.
pub(crate) fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Posit(_) => "Posit",
        Value::Text(_) => "Text",
        Value::Atom(_) => "Atom",
        Value::List(_) => "List",
        Value::Tuple(_) => "Tuple",
        Value::Record(_) => "Record",
        Value::Closure(_) | Value::TlcClosure(_) => "Function",
        Value::TypeValue(_) => "Type",
        Value::TaggedValue { .. } => "TaggedValue",
        Value::Nothing => "Nothing",
        Value::HostHandle(_) => "HostHandle",
        Value::WitnessDict(_) => "WitnessDict",
        Value::Builtin(_) | Value::BuiltinPartial { .. } => "Builtin",
    }
}
