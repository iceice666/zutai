use std::error::Error;
use std::path::Path;

pub(super) struct FoldedAotReflection {
    pub(super) module: zutai_tlc::TlcModule,
    pub(super) hir_bindings: Vec<zutai_hir::Binding>,
}

pub(super) fn fold_aot_reflection(
    contents: &str,
    base: Option<&Path>,
) -> Result<FoldedAotReflection, Box<dyn Error>> {
    let source = fold_reflection_value_to_source(contents, base)?;
    let pure = zutai_semantic::analyze_with_base(
        &source,
        None,
        zutai_semantic::AnalysisOptions::default(),
    );
    if !pure.is_thir_complete() {
        return Err(std::io::Error::other("folded reflection value did not re-analyze").into());
    }
    let module = pure
        .tlc
        .ok_or_else(|| std::io::Error::other("folded reflection value produced no TLC"))?;
    let hir_bindings = pure
        .hir
        .ok_or_else(|| std::io::Error::other("folded reflection value produced no HIR"))?
        .file
        .bindings;
    Ok(FoldedAotReflection {
        module,
        hir_bindings,
    })
}

pub(super) fn fold_reflection_value_to_source(
    contents: &str,
    base: Option<&Path>,
) -> Result<String, Box<dyn Error>> {
    let contents = contents.to_owned();
    let base = base.map(Path::to_path_buf);
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || -> Result<String, String> {
            let value = zutai_eval::eval_with_base(&contents, base.as_deref())
                .map_err(|err| err.to_string())?;
            reflection_value_to_source(&value).ok_or_else(|| {
                if value_contains_type(&value) {
                    super::compile::UNSUPPORTED_TYPE_ENTRY_REASON.to_string()
                } else {
                    "reflection entry did not fold to a backend value".to_string()
                }
            })
        })?;
    match handle.join() {
        Ok(Ok(source)) => Ok(source),
        Ok(Err(err)) => Err(std::io::Error::other(err).into()),
        Err(_) => Err(std::io::Error::other("reflection fold worker panicked").into()),
    }
}

#[derive(Clone, Copy)]
pub(super) enum EmptyListType {
    SchemaFields,
    SchemaVariants,
}

pub(super) struct TypedEmptyList {
    name: String,
    ty: &'static str,
}

pub(super) fn reflection_value_to_source(value: &zutai_eval::Value) -> Option<String> {
    let mut empty_lists = Vec::new();
    let expr = reflection_value_to_source_in(value, None, &mut empty_lists)?;
    if empty_lists.is_empty() {
        return Some(expr);
    }

    let mut source = String::from("[");
    for empty in empty_lists {
        source.push_str(&empty.name);
        source.push_str(" : ");
        source.push_str(empty.ty);
        source.push_str(" = {;};\n");
    }
    source.push_str(&expr);
    source.push(']');
    Some(source)
}

pub(super) fn reflection_value_to_source_in(
    value: &zutai_eval::Value,
    empty_list_type: Option<EmptyListType>,
    empty_lists: &mut Vec<TypedEmptyList>,
) -> Option<String> {
    match value {
        zutai_eval::Value::List(items) if items.is_empty() => match empty_list_type {
            Some(kind) => {
                let name = format!("__zutai_fold_empty{}", empty_lists.len());
                let ty = match kind {
                    EmptyListType::SchemaFields => {
                        "List { name : Text; type : Text; optional : Bool; }"
                    }
                    EmptyListType::SchemaVariants => {
                        "List { name : Text; fields : List { name : Text; type : Text; optional : Bool; }; }"
                    }
                };
                empty_lists.push(TypedEmptyList {
                    name: name.clone(),
                    ty,
                });
                Some(name)
            }
            None => Some("{;}".to_string()),
        },
        zutai_eval::Value::List(items) => {
            let mut out = String::from("{");
            for item in items.iter() {
                out.push_str(&reflection_value_to_source_in(
                    &item.peek()?,
                    empty_list_type,
                    empty_lists,
                )?);
                out.push_str("; ");
            }
            out.push('}');
            Some(out)
        }
        zutai_eval::Value::Tuple(items) => {
            let mut out = String::from("(");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                if let Some(name) = &item.name {
                    out.push_str(name);
                    out.push_str(" = ");
                }
                out.push_str(&reflection_value_to_source_in(
                    &item.value.peek()?,
                    None,
                    empty_lists,
                )?);
            }
            out.push(')');
            Some(out)
        }
        zutai_eval::Value::Record(fields) => {
            let mut out = String::from("{");
            for (name, value) in fields.iter() {
                out.push_str(name);
                out.push_str(" = ");
                let list_type = match name.as_ref() {
                    "fields" => Some(EmptyListType::SchemaFields),
                    "variants" => Some(EmptyListType::SchemaVariants),
                    _ => None,
                };
                out.push_str(&reflection_value_to_source_in(
                    &value.peek()?,
                    list_type,
                    empty_lists,
                )?);
                out.push_str("; ");
            }
            out.push('}');
            Some(out)
        }
        zutai_eval::Value::TaggedValue { tag, payload } => {
            if payload.is_empty() {
                return Some(format!("#{tag}"));
            }
            let positional = payload
                .iter()
                .enumerate()
                .all(|(index, (name, _))| name.parse::<usize>() == Ok(index));
            if positional {
                let mut out = format!("#{tag} (");
                for (index, (_, value)) in payload.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&reflection_value_to_source_in(
                        &value.peek()?,
                        None,
                        empty_lists,
                    )?);
                }
                out.push(')');
                Some(out)
            } else {
                let mut out = format!("#{tag} {{");
                for (name, value) in payload.iter() {
                    out.push_str(name);
                    out.push_str(" = ");
                    out.push_str(&reflection_value_to_source_in(
                        &value.peek()?,
                        None,
                        empty_lists,
                    )?);
                    out.push_str("; ");
                }
                out.push('}');
                Some(out)
            }
        }
        _ => value_to_source(value),
    }
}

pub(super) fn value_to_source(value: &zutai_eval::Value) -> Option<String> {
    match value {
        zutai_eval::Value::Bool(value) => Some(value.to_string()),
        zutai_eval::Value::Int(value) => Some(value.to_string()),
        zutai_eval::Value::Float(value) => Some(float_source(*value)),
        zutai_eval::Value::Text(value) => Some(text_source(value)),
        zutai_eval::Value::Atom(value) => Some(format!("#{value}")),
        zutai_eval::Value::List(items) => {
            if items.is_empty() {
                return Some("{;}".to_string());
            }
            let mut out = String::from("{");
            for item in items.iter() {
                out.push_str(&value_to_source(&item.peek()?)?);
                out.push_str("; ");
            }
            out.push('}');
            Some(out)
        }
        zutai_eval::Value::Tuple(items) => {
            let mut out = String::from("(");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                if let Some(name) = &item.name {
                    out.push_str(name);
                    out.push_str(" = ");
                }
                out.push_str(&value_to_source(&item.value.peek()?)?);
            }
            out.push(')');
            Some(out)
        }
        zutai_eval::Value::Record(fields) => {
            let mut out = String::from("{");
            for (name, value) in fields.iter() {
                out.push_str(name);
                out.push_str(" = ");
                out.push_str(&value_to_source(&value.peek()?)?);
                out.push_str("; ");
            }
            out.push('}');
            Some(out)
        }
        zutai_eval::Value::TaggedValue { tag, payload } => {
            if payload.is_empty() {
                return Some(format!("#{tag}"));
            }
            let positional = payload
                .iter()
                .enumerate()
                .all(|(index, (name, _))| name.parse::<usize>() == Ok(index));
            if positional {
                let mut out = format!("#{tag} (");
                for (index, (_, value)) in payload.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&value_to_source(&value.peek()?)?);
                }
                out.push(')');
                Some(out)
            } else {
                let mut out = format!("#{tag} {{");
                for (name, value) in payload.iter() {
                    out.push_str(name);
                    out.push_str(" = ");
                    out.push_str(&value_to_source(&value.peek()?)?);
                    out.push_str("; ");
                }
                out.push('}');
                Some(out)
            }
        }
        zutai_eval::Value::Nothing => Some("#absent".to_string()),
        zutai_eval::Value::Posit(_)
        | zutai_eval::Value::Closure(_)
        | zutai_eval::Value::TypeValue(_)
        | zutai_eval::Value::WitnessDict(_)
        | zutai_eval::Value::TlcClosure(_)
        | zutai_eval::Value::HostHandle(_)
        | zutai_eval::Value::Builtin(_)
        | zutai_eval::Value::BuiltinPartial { .. } => None,
    }
}

pub(super) fn value_contains_type(value: &zutai_eval::Value) -> bool {
    match value {
        zutai_eval::Value::TypeValue(_) => true,
        zutai_eval::Value::List(items) => items
            .iter()
            .any(|item| item.peek().is_some_and(|value| value_contains_type(&value))),
        zutai_eval::Value::Tuple(items) => items.iter().any(|item| {
            item.value
                .peek()
                .is_some_and(|value| value_contains_type(&value))
        }),
        zutai_eval::Value::Record(fields) => fields.iter().any(|(_, value)| {
            value
                .peek()
                .is_some_and(|value| value_contains_type(&value))
        }),
        zutai_eval::Value::TaggedValue { payload, .. } => payload.iter().any(|(_, value)| {
            value
                .peek()
                .is_some_and(|value| value_contains_type(&value))
        }),
        zutai_eval::Value::Bool(_)
        | zutai_eval::Value::Int(_)
        | zutai_eval::Value::Float(_)
        | zutai_eval::Value::Text(_)
        | zutai_eval::Value::Atom(_)
        | zutai_eval::Value::Nothing
        | zutai_eval::Value::Posit(_)
        | zutai_eval::Value::Closure(_)
        | zutai_eval::Value::WitnessDict(_)
        | zutai_eval::Value::TlcClosure(_)
        | zutai_eval::Value::HostHandle(_)
        | zutai_eval::Value::Builtin(_)
        | zutai_eval::Value::BuiltinPartial { .. } => false,
    }
}

pub(super) fn text_source(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub(super) fn float_source(value: f64) -> String {
    let source = format!("{value:?}");
    if !value.is_finite() || source.contains('.') || source.contains('e') || source.contains('E') {
        source
    } else {
        format!("{source}.0")
    }
}
