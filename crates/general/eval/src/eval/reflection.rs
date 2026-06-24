use super::*;

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

    fn reflect_fields(&self, ty: &RuntimeType) -> Result<Value, EvalError> {
        match self.runtime_type_view(ty, 0)? {
            RuntimeTypeView::Record(fields, RowTail::Closed) => fields
                .into_iter()
                .map(|field| {
                    Ok(record_value(vec![
                        ("name", Value::Text(Rc::from(field.name.as_str()))),
                        ("Type", Value::TypeValue(field.ty)),
                        ("optional", Value::Bool(field.optional)),
                    ]))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(list_value),
            RuntimeTypeView::Record(_, _) => Err(open_row_reflection_error("record")),
            RuntimeTypeView::Union(_, _) => Err(EvalError::ReflectionUnsupported(
                "`fields` reflects record fields; use `schema` for union variants".to_string(),
            )),
            _ => Err(EvalError::ReflectionUnsupported(
                "`fields` expects a record type".to_string(),
            )),
        }
    }

    fn reflect_variants(&self, ty: &RuntimeType) -> Result<Value, EvalError> {
        match self.runtime_type_view(ty, 0)? {
            RuntimeTypeView::Union(variants, RowTail::Closed) => variants
                .into_iter()
                .map(|variant| self.reflect_variant(variant))
                .collect::<Result<Vec<_>, _>>()
                .map(list_value),
            RuntimeTypeView::Union(_, _) => Err(open_row_reflection_error("union")),
            _ => Err(EvalError::ReflectionUnsupported(
                "`variants` expects a union type".to_string(),
            )),
        }
    }

    fn reflect_variant(&self, variant: ReflectedUnionVariant) -> Result<Value, EvalError> {
        let fields = match variant.payload {
            Some(payload) => match self.runtime_type_view(&payload, 0)? {
                RuntimeTypeView::Record(fields, RowTail::Closed) => fields
                    .into_iter()
                    .map(|field| {
                        Ok(record_value(vec![
                            ("name", Value::Text(Rc::from(field.name.as_str()))),
                            ("Type", Value::TypeValue(field.ty)),
                            ("optional", Value::Bool(field.optional)),
                        ]))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(list_value)?,
                RuntimeTypeView::Record(_, _) => return Err(open_row_reflection_error("record")),
                _ => {
                    return Err(EvalError::ReflectionUnsupported(
                        "union variant payload reflection expects a record payload".to_string(),
                    ));
                }
            },
            None => list_value(Vec::new()),
        };
        Ok(record_value(vec![
            ("name", Value::Text(Rc::from(variant.name.as_str()))),
            ("fields", fields),
        ]))
    }

    fn reflect_schema(&self, ty: &RuntimeType) -> Result<Value, EvalError> {
        match self.runtime_type_view(ty, 0)? {
            RuntimeTypeView::Record(fields, RowTail::Closed) => Ok(record_value(vec![
                ("kind", Value::Atom(Rc::from("record"))),
                ("fields", self.schema_fields(fields)?),
            ])),
            RuntimeTypeView::Record(_, _) => Err(open_row_reflection_error("record")),
            RuntimeTypeView::Union(variants, RowTail::Closed) => {
                let variants = variants
                    .into_iter()
                    .map(|variant| self.schema_variant(variant))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(record_value(vec![
                    ("kind", Value::Atom(Rc::from("union"))),
                    ("variants", list_value(variants)),
                ]))
            }
            RuntimeTypeView::Union(_, _) => Err(open_row_reflection_error("union")),
            _ => Err(EvalError::ReflectionUnsupported(
                "`schema` expects a record or union type".to_string(),
            )),
        }
    }

    fn schema_fields(&self, fields: Vec<ReflectedRecordField>) -> Result<Value, EvalError> {
        fields
            .into_iter()
            .map(|field| self.schema_field(field))
            .collect::<Result<Vec<_>, _>>()
            .map(list_value)
    }

    fn schema_field(&self, field: ReflectedRecordField) -> Result<Value, EvalError> {
        Ok(record_value(vec![
            ("name", Value::Text(Rc::from(field.name.as_str()))),
            (
                "type",
                Value::Text(Rc::from(self.type_label(&field.ty)?.as_str())),
            ),
            ("optional", Value::Bool(field.optional)),
        ]))
    }

    fn schema_variant(&self, variant: ReflectedUnionVariant) -> Result<Value, EvalError> {
        let fields = match variant.payload {
            Some(payload) => match self.runtime_type_view(&payload, 0)? {
                RuntimeTypeView::Record(fields, RowTail::Closed) => self.schema_fields(fields)?,
                RuntimeTypeView::Record(_, _) => return Err(open_row_reflection_error("record")),
                _ => {
                    return Err(EvalError::ReflectionUnsupported(
                        "union variant payload reflection expects a record payload".to_string(),
                    ));
                }
            },
            None => list_value(Vec::new()),
        };
        Ok(record_value(vec![
            ("name", Value::Text(Rc::from(variant.name.as_str()))),
            ("fields", fields),
        ]))
    }

    fn type_label(&self, ty: &RuntimeType) -> Result<String, EvalError> {
        match self.runtime_type_view(ty, 0)? {
            RuntimeTypeView::Type => Ok("Type".to_string()),
            RuntimeTypeView::Bool => Ok("Bool".to_string()),
            RuntimeTypeView::Text => Ok("Text".to_string()),
            RuntimeTypeView::Int => Ok("Int".to_string()),
            RuntimeTypeView::Float => Ok("Float".to_string()),
            RuntimeTypeView::FixedNum(fw) => Ok(fw.name().to_string()),
            RuntimeTypeView::Posit(spec) => Ok(spec.type_name()),
            RuntimeTypeView::Atom(name) => Ok(format!("#{name}")),
            RuntimeTypeView::True => Ok("true".to_string()),
            RuntimeTypeView::False => Ok("false".to_string()),
            RuntimeTypeView::Never => Ok("Never".to_string()),
            RuntimeTypeView::List(inner) => Ok(format!("[{}]", self.type_label(&inner)?)),
            RuntimeTypeView::Optional(inner) => Ok(format!("{}?", self.type_label(&inner)?)),
            RuntimeTypeView::Maybe(inner) => Ok(format!("Maybe {}", self.type_label(&inner)?)),
            RuntimeTypeView::Record(_, RowTail::Closed) => Ok("record".to_string()),
            RuntimeTypeView::Record(_, _) => Err(open_row_reflection_error("record")),
            RuntimeTypeView::Opaque(name) => Ok(name),
            RuntimeTypeView::Union(_, RowTail::Closed) => Ok("union".to_string()),
            RuntimeTypeView::Union(_, _) => Err(open_row_reflection_error("union")),
            RuntimeTypeView::Tuple(items) => {
                let parts = items
                    .into_iter()
                    .map(|item| match item {
                        ReflectedTupleItem::Named { name, ty } => {
                            Ok(format!("{name}: {}", self.type_label(&ty)?))
                        }
                        ReflectedTupleItem::Positional(ty) => self.type_label(&ty),
                    })
                    .collect::<Result<Vec<_>, EvalError>>()?;
                Ok(format!("({})", parts.join(", ")))
            }
            RuntimeTypeView::Function { from, to } => Ok(format!(
                "{} -> {}",
                self.type_label(&from)?,
                self.type_label(&to)?
            )),
            RuntimeTypeView::Effect { base } => Ok(format!("{} ! effect", self.type_label(&base)?)),
        }
    }

    fn runtime_type_view(
        &self,
        ty: &RuntimeType,
        depth: u16,
    ) -> Result<RuntimeTypeView, EvalError> {
        if depth > 256 {
            return Err(EvalError::ReflectionUnsupported(
                "type alias expansion exceeded reflection fuel".to_string(),
            ));
        }
        let file = self.file_for_module(ty.module)?;
        let Some(type_node) = file.type_arena.get(ty.ty.0 as usize) else {
            return Err(EvalError::Internal(
                "type value points outside its module arena",
            ));
        };
        match type_node.kind.clone() {
            TypeKind::Type(_) => Ok(RuntimeTypeView::Type),
            TypeKind::Bool => Ok(RuntimeTypeView::Bool),
            TypeKind::Text => Ok(RuntimeTypeView::Text),
            TypeKind::Int => Ok(RuntimeTypeView::Int),
            TypeKind::Float => Ok(RuntimeTypeView::Float),
            TypeKind::FixedNum(fw) => Ok(RuntimeTypeView::FixedNum(fw)),
            TypeKind::Posit(spec) => Ok(RuntimeTypeView::Posit(spec)),
            TypeKind::Opaque(name) => Ok(RuntimeTypeView::Opaque(name)),
            TypeKind::Atom(name) => Ok(RuntimeTypeView::Atom(name)),
            TypeKind::True => Ok(RuntimeTypeView::True),
            TypeKind::False => Ok(RuntimeTypeView::False),
            TypeKind::Never => Ok(RuntimeTypeView::Never),
            TypeKind::List(inner) => Ok(RuntimeTypeView::List(ty.with_ty(inner))),
            TypeKind::Optional(inner) => Ok(RuntimeTypeView::Optional(ty.with_ty(inner))),
            TypeKind::Maybe(inner) => Ok(RuntimeTypeView::Maybe(ty.with_ty(inner))),
            TypeKind::Patch { .. } => Err(EvalError::ReflectionUnsupported(
                "patch types cannot be reflected in this phase".to_string(),
            )),
            TypeKind::Record(fields, tail) => Ok(RuntimeTypeView::Record(
                reflect_record_fields(ty, fields),
                tail,
            )),
            TypeKind::Union(variants, tail) => Ok(RuntimeTypeView::Union(
                reflect_union_variants(ty, variants),
                tail,
            )),
            TypeKind::Tuple(items) => Ok(RuntimeTypeView::Tuple(reflect_tuple_items(ty, items))),
            TypeKind::Function { from, to } => Ok(RuntimeTypeView::Function {
                from: ty.with_ty(from),
                to: ty.with_ty(to),
            }),
            TypeKind::Effect { base, .. } => Ok(RuntimeTypeView::Effect {
                base: ty.with_ty(base),
            }),
            TypeKind::TypeVar(binding) => {
                match ty.subst.iter().rev().find(|(b, _)| *b == binding) {
                    Some((_, replacement)) => self.runtime_type_view(replacement, depth + 1),
                    None => Err(EvalError::ReflectionUnsupported(format!(
                        "unsubstituted type parameter `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    ))),
                }
            }
            TypeKind::Alias(binding) => {
                let Some((params, body)) = alias_decl(file, binding) else {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "unknown type alias `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    )));
                };
                if !params.is_empty() {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "unapplied type constructor `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    )));
                }
                self.runtime_type_view(
                    &RuntimeType::with_subst(ty.module, body, ty.subst.clone()),
                    depth + 1,
                )
            }
            TypeKind::AliasApply { binding, args } => {
                let Some((params, body)) = alias_decl(file, binding) else {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "unknown type alias `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    )));
                };
                if params.len() != args.len() {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "type constructor `{}` has arity {}, got {}",
                        binding_name_in_file(file, binding),
                        params.len(),
                        args.len()
                    )));
                }
                let subst = extend_subst(ty, &params, &args);
                self.runtime_type_view(&RuntimeType::with_subst(ty.module, body, subst), depth + 1)
            }
            TypeKind::Apply { .. } => self.runtime_apply_view(ty, depth),
            TypeKind::Con(binding) => Err(EvalError::ReflectionUnsupported(format!(
                "unapplied builtin type constructor `{}` cannot be reflected",
                binding_name_in_file(file, binding)
            ))),
            TypeKind::ForAll { .. } => Err(EvalError::ReflectionUnsupported(
                "higher-rank polymorphic types cannot be reflected in this phase".to_string(),
            )),
            TypeKind::InferVar(_) | TypeKind::Error => Err(EvalError::ReflectionUnsupported(
                "incomplete or erroneous types cannot be reflected".to_string(),
            )),
        }
    }

    fn runtime_apply_view(
        &self,
        ty: &RuntimeType,
        depth: u16,
    ) -> Result<RuntimeTypeView, EvalError> {
        let file = self.file_for_module(ty.module)?;
        let mut args = Vec::new();
        let mut head = ty.ty;
        while let TypeKind::Apply { func, arg } = file.type_arena[head.0 as usize].kind.clone() {
            args.push(arg);
            head = func;
        }
        args.reverse();
        match file.type_arena[head.0 as usize].kind.clone() {
            TypeKind::Alias(binding) => {
                let Some((params, body)) = alias_decl(file, binding) else {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "unknown type alias `{}` cannot be reflected",
                        binding_name_in_file(file, binding)
                    )));
                };
                if params.len() != args.len() {
                    return Err(EvalError::ReflectionUnsupported(format!(
                        "type constructor `{}` has arity {}, got {}",
                        binding_name_in_file(file, binding),
                        params.len(),
                        args.len()
                    )));
                }
                let subst = extend_subst(ty, &params, &args);
                self.runtime_type_view(&RuntimeType::with_subst(ty.module, body, subst), depth + 1)
            }
            TypeKind::Con(binding)
                if binding_name_in_file(file, binding) == "List" && args.len() == 1 =>
            {
                Ok(RuntimeTypeView::List(ty.with_ty(args[0])))
            }
            TypeKind::Con(binding)
                if binding_name_in_file(file, binding) == "Optional" && args.len() == 1 =>
            {
                Ok(RuntimeTypeView::Optional(ty.with_ty(args[0])))
            }
            TypeKind::Con(binding)
                if binding_name_in_file(file, binding) == "Maybe" && args.len() == 1 =>
            {
                Ok(RuntimeTypeView::Maybe(ty.with_ty(args[0])))
            }
            _ => Err(EvalError::ReflectionUnsupported(
                "higher-kinded or partial type application cannot be reflected".to_string(),
            )),
        }
    }

    fn file_for_module(&self, module: ModuleId) -> Result<&ThirFile, EvalError> {
        self.registry
            .get(module.0)
            .map(Arc::as_ref)
            .ok_or(EvalError::Internal("type value module is not registered"))
    }
}

#[derive(Clone)]
struct ReflectedRecordField {
    name: String,
    optional: bool,
    ty: RuntimeType,
}

#[derive(Clone)]
struct ReflectedUnionVariant {
    name: String,
    payload: Option<RuntimeType>,
}

enum ReflectedTupleItem {
    Named { name: String, ty: RuntimeType },
    Positional(RuntimeType),
}

enum RuntimeTypeView {
    Type,
    Bool,
    Text,
    Int,
    Float,
    FixedNum(zutai_thir::FixedWidth),
    Posit(zutai_syntax::posit::PositSpec),
    Atom(String),
    True,
    False,
    Never,
    Opaque(String),
    List(RuntimeType),
    Optional(RuntimeType),
    Maybe(RuntimeType),
    Record(Vec<ReflectedRecordField>, RowTail),
    Union(Vec<ReflectedUnionVariant>, RowTail),
    Tuple(Vec<ReflectedTupleItem>),
    Function { from: RuntimeType, to: RuntimeType },
    Effect { base: RuntimeType },
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

fn reflect_record_fields(
    owner: &RuntimeType,
    fields: Vec<TypeRecordField>,
) -> Vec<ReflectedRecordField> {
    fields
        .into_iter()
        .map(|field| ReflectedRecordField {
            name: field.name,
            optional: field.optional,
            ty: owner.with_ty(field.ty),
        })
        .collect()
}

fn reflect_union_variants(
    owner: &RuntimeType,
    variants: Vec<UnionVariant>,
) -> Vec<ReflectedUnionVariant> {
    variants
        .into_iter()
        .map(|variant| ReflectedUnionVariant {
            name: variant.name,
            payload: variant.payload.map(|payload| owner.with_ty(payload)),
        })
        .collect()
}

fn reflect_tuple_items(owner: &RuntimeType, items: Vec<TypeTupleItem>) -> Vec<ReflectedTupleItem> {
    items
        .into_iter()
        .map(|item| match item {
            TypeTupleItem::Named { name, ty, .. } => ReflectedTupleItem::Named {
                name,
                ty: owner.with_ty(ty),
            },
            TypeTupleItem::Positional(ty) => ReflectedTupleItem::Positional(owner.with_ty(ty)),
        })
        .collect()
}

fn alias_decl(file: &ThirFile, binding: BindingId) -> Option<(Vec<BindingId>, TypeId)> {
    file.decls.iter().find_map(|decl_id| {
        let decl = &file.decl_arena[*decl_id];
        match &decl.kind {
            ThirDeclKind::TypeAlias { params, ty } if decl.binding == binding => {
                Some((params.clone(), *ty))
            }
            _ => None,
        }
    })
}

fn binding_name_in_file(file: &ThirFile, binding: BindingId) -> &str {
    file.binding_names
        .get(binding.0 as usize)
        .map_or("<unknown>", String::as_str)
}

fn extend_subst(
    owner: &RuntimeType,
    params: &[BindingId],
    args: &[TypeId],
) -> Rc<[(BindingId, RuntimeType)]> {
    let mut subst: Vec<(BindingId, RuntimeType)> = owner.subst.iter().cloned().collect();
    subst.extend(
        params
            .iter()
            .zip(args.iter())
            .map(|(param, arg)| (*param, owner.with_ty(*arg))),
    );
    Rc::from(subst.into_boxed_slice())
}

fn open_row_reflection_error(kind: &str) -> EvalError {
    EvalError::ReflectionUnsupported(format!(
        "reflection rejects open {kind} rows; close the row before calling `fields` or `schema`"
    ))
}
