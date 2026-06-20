use super::*;

/// Recursively force all thunks in a TLC value.
pub fn tlc_force_deep(v: Value, ev: &TlcEvaluator<'_>) -> Result<Value, EvalError> {
    match v {
        Value::List(thunks) => {
            let forced: Result<Vec<_>, _> = thunks
                .iter()
                .map(|t| {
                    let inner = t.force_tlc(ev)?;
                    Ok(Thunk::ready(tlc_force_deep(inner, ev)?))
                })
                .collect();
            Ok(Value::List(forced?.into()))
        }
        Value::Tuple(fields) => {
            let forced: Result<Vec<_>, _> = fields
                .iter()
                .map(|f| {
                    let inner = f.value.force_tlc(ev)?;
                    Ok(TupleField {
                        name: f.name.clone(),
                        value: Thunk::ready(tlc_force_deep(inner, ev)?),
                    })
                })
                .collect();
            Ok(Value::Tuple(forced?.into()))
        }
        Value::Record(fields) => {
            let forced: Result<Vec<_>, _> = fields
                .iter()
                .map(|(name, t)| {
                    let inner = t.force_tlc(ev)?;
                    Ok((name.clone(), Thunk::ready(tlc_force_deep(inner, ev)?)))
                })
                .collect();
            Ok(Value::Record(Rc::new(forced?)))
        }
        Value::TaggedValue { tag, payload } => {
            let forced: Result<Vec<_>, _> = payload
                .iter()
                .map(|(name, t)| {
                    let inner = t.force_tlc(ev)?;
                    Ok((name.clone(), Thunk::ready(tlc_force_deep(inner, ev)?)))
                })
                .collect();
            Ok(Value::TaggedValue {
                tag,
                payload: Rc::new(forced?),
            })
        }
        other => Ok(other),
    }
}
