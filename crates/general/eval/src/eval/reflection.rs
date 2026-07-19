use super::*;

use zutai_thir::reflect::{
    ReflectError, ReflectedType, ReflectedVariant, ReflectedView, SchemaData, open_row_error,
    reflected_view, schema_data,
};

impl<'a> Evaluator<'a> {
    pub(super) fn reflect_fields_value(&self, value: Value) -> Result<Value, EvalError> {
        match value {
            Value::TypeValue(ty) => self.reflect_fields(&ty),
            other => Err(EvalError::TypeMismatch {
                expected: "Type",
                found: value_type_name(&other),
            }),
        }
    }

    pub(super) fn reflect_schema_value(&self, value: Value) -> Result<Value, EvalError> {
        match value {
            Value::TypeValue(ty) => self.reflect_schema(&ty),
            other => Err(EvalError::TypeMismatch {
                expected: "Type",
                found: value_type_name(&other),
            }),
        }
    }

    pub(super) fn reflect_variants_value(&self, value: Value) -> Result<Value, EvalError> {
        match value {
            Value::TypeValue(ty) => self.reflect_variants(&ty),
            other => Err(EvalError::TypeMismatch {
                expected: "Type",
                found: value_type_name(&other),
            }),
        }
    }

    /// Shared THIR files backing this evaluator's module registry, indexed by
    /// `ModuleId` — the resolver `zutai_thir::reflect` walks types through.
    fn reflect_files(&self) -> Vec<&ThirFile> {
        self.registry.iter().map(Arc::as_ref).collect()
    }

    fn reflect_fields(&self, ty: &RuntimeType) -> Result<Value, EvalError> {
        let files = self.reflect_files();
        match reflected_view(&files, &to_reflected_type(ty)).map_err(reflect_error_to_eval)? {
            ReflectedView::Record(fields, RowTail::Closed) => Ok(list_value(
                fields
                    .into_iter()
                    .map(|field| {
                        record_value(vec![
                            ("name", Value::Text(Rc::from(field.name.as_str()))),
                            ("Type", Value::TypeValue(from_reflected_type(&field.ty))),
                            ("optional", Value::Bool(field.optional)),
                        ])
                    })
                    .collect(),
            )),
            ReflectedView::Record(_, _) => Err(reflect_error_to_eval(open_row_error("record"))),
            ReflectedView::Union(_, _) => Err(EvalError::ReflectionUnsupported(
                "`fields` reflects record fields; use `schema` for union variants".to_string(),
            )),
            ReflectedView::Other => Err(EvalError::ReflectionUnsupported(
                "`fields` expects a record type".to_string(),
            )),
        }
    }

    fn reflect_variants(&self, ty: &RuntimeType) -> Result<Value, EvalError> {
        let files = self.reflect_files();
        match reflected_view(&files, &to_reflected_type(ty)).map_err(reflect_error_to_eval)? {
            ReflectedView::Union(variants, RowTail::Closed) => variants
                .into_iter()
                .map(|variant| self.reflect_variant(&files, variant))
                .collect::<Result<Vec<_>, _>>()
                .map(list_value),
            ReflectedView::Union(_, _) => Err(reflect_error_to_eval(open_row_error("union"))),
            _ => Err(EvalError::ReflectionUnsupported(
                "`variants` expects a union type".to_string(),
            )),
        }
    }

    fn reflect_variant(
        &self,
        files: &[&ThirFile],
        variant: ReflectedVariant,
    ) -> Result<Value, EvalError> {
        let fields = match variant.payload {
            Some(payload) => {
                match reflected_view(files, &payload).map_err(reflect_error_to_eval)? {
                    ReflectedView::Record(fields, RowTail::Closed) => list_value(
                        fields
                            .into_iter()
                            .map(|field| {
                                record_value(vec![
                                    ("name", Value::Text(Rc::from(field.name.as_str()))),
                                    ("Type", Value::TypeValue(from_reflected_type(&field.ty))),
                                    ("optional", Value::Bool(field.optional)),
                                ])
                            })
                            .collect(),
                    ),
                    ReflectedView::Record(_, _) => {
                        return Err(reflect_error_to_eval(open_row_error("record")));
                    }
                    _ => {
                        return Err(EvalError::ReflectionUnsupported(
                            "union variant payload reflection expects a record payload".to_string(),
                        ));
                    }
                }
            }
            None => list_value(Vec::new()),
        };
        Ok(record_value(vec![
            ("name", Value::Text(Rc::from(variant.name.as_str()))),
            ("fields", fields),
        ]))
    }

    /// `schema` delegates to the shared THIR reflection implementation
    /// (`zutai_thir::reflect`) — the same computation the THIR→TLC fold uses —
    /// so folded schema literals equal the oracle's values by construction.
    fn reflect_schema(&self, ty: &RuntimeType) -> Result<Value, EvalError> {
        let files = self.reflect_files();
        let data = schema_data(&files, &to_reflected_type(ty)).map_err(reflect_error_to_eval)?;
        Ok(schema_data_value(&data))
    }
}

fn to_reflected_type(ty: &RuntimeType) -> ReflectedType {
    ReflectedType::with_subst(
        ty.module.0,
        ty.ty,
        ty.subst
            .iter()
            .map(|(binding, replacement)| (*binding, to_reflected_type(replacement)))
            .collect::<Vec<_>>()
            .into(),
    )
}

fn from_reflected_type(ty: &ReflectedType) -> RuntimeType {
    RuntimeType::with_subst(
        ModuleId(ty.module),
        ty.ty,
        ty.subst
            .iter()
            .map(|(binding, replacement)| (*binding, from_reflected_type(replacement)))
            .collect::<Vec<_>>()
            .into(),
    )
}

fn schema_data_value(data: &SchemaData) -> Value {
    match data {
        SchemaData::Record(fields) => record_value(
            fields
                .iter()
                .map(|(name, value)| (*name, schema_data_value(value)))
                .collect(),
        ),
        SchemaData::List(items) => list_value(items.iter().map(schema_data_value).collect()),
        SchemaData::Text(text) => Value::Text(Rc::from(text.as_str())),
        SchemaData::Bool(value) => Value::Bool(*value),
        SchemaData::Atom(name) => Value::Atom(Rc::from(*name)),
    }
}

fn reflect_error_to_eval(error: ReflectError) -> EvalError {
    match error {
        ReflectError::Unsupported(message) => EvalError::ReflectionUnsupported(message),
        ReflectError::Internal(message) => EvalError::Internal(message),
    }
}

fn record_value(fields: Vec<(&'static str, Value)>) -> Value {
    Value::Record(Rc::new(
        fields
            .into_iter()
            .map(|(name, value)| (Rc::from(name), Thunk::ready(value)))
            .collect(),
    ))
}

fn list_value(values: Vec<Value>) -> Value {
    Value::List(
        values
            .into_iter()
            .map(Thunk::ready)
            .collect::<Vec<_>>()
            .into(),
    )
}
