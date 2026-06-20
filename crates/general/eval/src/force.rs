use super::*;

/// Recursively force all lazy thunks inside a value so it can be displayed.
///
/// Only descends into finite structures.  Recursive lambdas can produce
/// infinite data (e.g. an infinite list); `force_deep` will loop on them.
/// This is acceptable for the reference interpreter — a non-terminating
/// program already diverges at eval time before reaching this function.
pub fn force_deep(v: Value, ev: &eval::Evaluator<'_>) -> Result<Value, EvalError> {
    match v {
        Value::List(thunks) => {
            let forced: Result<Vec<_>, _> = thunks
                .iter()
                .map(|t| {
                    let inner = t.force(ev)?;
                    let deep = force_deep(inner, ev)?;
                    Ok(thunk::Thunk::ready(deep))
                })
                .collect();
            Ok(Value::List(forced?.into()))
        }
        Value::Tuple(fields) => {
            let forced: Result<Vec<_>, _> = fields
                .iter()
                .map(|f| {
                    let inner = f.value.force(ev)?;
                    let deep = force_deep(inner, ev)?;
                    Ok(value::TupleField {
                        name: f.name.clone(),
                        value: thunk::Thunk::ready(deep),
                    })
                })
                .collect();
            Ok(Value::Tuple(forced?.into()))
        }
        Value::Record(fields) => {
            let forced: Result<Vec<_>, _> = fields
                .iter()
                .map(|(name, t)| {
                    let inner = t.force(ev)?;
                    let deep = force_deep(inner, ev)?;
                    Ok((name.clone(), thunk::Thunk::ready(deep)))
                })
                .collect();
            Ok(Value::Record(std::rc::Rc::new(forced?)))
        }
        Value::TaggedValue { tag, payload } => {
            let forced: Result<Vec<_>, _> = payload
                .iter()
                .map(|(name, t)| {
                    let inner = t.force(ev)?;
                    let deep = force_deep(inner, ev)?;
                    Ok((name.clone(), thunk::Thunk::ready(deep)))
                })
                .collect();
            Ok(Value::TaggedValue {
                tag,
                payload: std::rc::Rc::new(forced?),
            })
        }
        other => Ok(other),
    }
}
