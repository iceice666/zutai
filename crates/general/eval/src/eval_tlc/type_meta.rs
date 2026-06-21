use super::*;

impl<'a> TlcEvaluator<'a> {
    pub(super) fn tlc_expr_target_key(&self, expr_id: TlcExprId) -> Option<String> {
        let ty_id = self
            .module
            .expr_types
            .get(&expr_id)
            .copied()
            .map(|ty_id| self.resolve_tlc_alias_chain(ty_id))?;
        self.tlc_type_target_key(ty_id)
    }

    pub(super) fn tlc_type_target_key(&self, ty_id: TlcTypeId) -> Option<String> {
        match &self.module.type_arena[ty_id] {
            TlcType::Prim(prim) => match prim {
                zutai_tlc::PrimTy::Int => Some("Int".to_string()),
                zutai_tlc::PrimTy::Float => Some("Float".to_string()),
                zutai_tlc::PrimTy::FixedNum(fw) => Some(fw.name().to_string()),
                zutai_tlc::PrimTy::Bool => Some("Bool".to_string()),
                zutai_tlc::PrimTy::Str => Some("Text".to_string()),
                zutai_tlc::PrimTy::Atom => None,
                zutai_tlc::PrimTy::Nothing => None,
            },
            TlcType::Singleton(Literal::Int(_)) => Some("Int".to_string()),
            TlcType::Singleton(Literal::Float(_)) => Some("Float".to_string()),
            TlcType::Singleton(Literal::Bool(_)) => Some("Bool".to_string()),
            TlcType::Singleton(Literal::Str(_)) => Some("Text".to_string()),
            TlcType::Singleton(Literal::Atom(atom)) => Some(format!("#{atom}")),
            TlcType::Singleton(Literal::Nothing) => None,
            _ => None,
        }
    }

    pub(super) fn resolve_tlc_alias_chain(&self, mut ty_id: TlcTypeId) -> TlcTypeId {
        let mut fuel = 64u8;
        while fuel > 0 {
            match &self.module.type_arena[ty_id] {
                TlcType::TyVar(TlcTypeVar::Named(binding), _) => {
                    let Some(next) = self.type_alias_body(*binding) else {
                        break;
                    };
                    ty_id = next;
                    fuel -= 1;
                }
                _ => break,
            }
        }
        ty_id
    }

    pub(super) fn type_alias_body(&self, binding: u32) -> Option<TlcTypeId> {
        self.module
            .decls
            .iter()
            .find_map(|&decl_id| match &self.module.decl_arena[decl_id] {
                TlcDecl::TypeAlias {
                    binding: alias,
                    params,
                    body,
                } if alias.0 == binding && params.is_empty() => Some(*body),
                _ => None,
            })
    }

    pub(super) fn tlc_field_meta(
        &self,
        ty_id: TlcTypeId,
        field: &str,
    ) -> Option<(bool, TlcTypeId)> {
        let ty_id = self.resolve_tlc_alias_chain(ty_id);
        match &self.module.type_arena[ty_id] {
            TlcType::Record(row) => {
                let mut current = row;
                loop {
                    match current {
                        Row::REmpty | Row::RVar(_) => return None,
                        Row::RExtend {
                            label,
                            ty,
                            optional,
                            tail,
                        } => {
                            if label == field {
                                return Some((*optional, *ty));
                            }
                            current = tail;
                        }
                    }
                }
            }
            _ => None,
        }
    }

    pub(super) fn tlc_record_field_order(&self, ty_id: TlcTypeId) -> Option<Vec<(String, bool)>> {
        let ty_id = self.resolve_tlc_alias_chain(ty_id);
        match &self.module.type_arena[ty_id] {
            TlcType::Record(row) => {
                let mut fields = Vec::new();
                let mut current = row;
                loop {
                    match current {
                        Row::REmpty | Row::RVar(_) => return Some(fields),
                        Row::RExtend {
                            label,
                            optional,
                            tail,
                            ..
                        } => {
                            fields.push((label.clone(), *optional));
                            current = tail;
                        }
                    }
                }
            }
            _ => None,
        }
    }

    pub(super) fn tlc_type_wrapper_kind(&self, ty_id: TlcTypeId) -> Option<TlcWrapperKind> {
        let ty_id = self.resolve_tlc_alias_chain(ty_id);
        match &self.module.type_arena[ty_id] {
            TlcType::Optional(_) => Some(TlcWrapperKind::Optional),
            TlcType::Maybe(_) => Some(TlcWrapperKind::Maybe),
            _ => None,
        }
    }

    pub(super) fn project_maybe_field(
        &self,
        fields: &Rc<Vec<(Rc<str>, Thunk)>>,
        field: &str,
    ) -> Value {
        match fields.iter().find(|(name, _)| name.as_ref() == field) {
            None => Value::Atom(Rc::from("absent")),
            Some((_, thunk)) => Value::TaggedValue {
                tag: Rc::from("present"),
                payload: Rc::new(vec![(Rc::from("0"), thunk.clone())]),
            },
        }
    }

    pub(super) fn project_record_field(
        &self,
        ty_id: TlcTypeId,
        fields: &Rc<Vec<(Rc<str>, Thunk)>>,
        field: &str,
    ) -> Result<Value, EvalError> {
        if let Some((true, _)) = self.tlc_field_meta(ty_id, field) {
            return Ok(self.project_maybe_field(fields, field));
        }
        match fields.iter().find(|(name, _)| name.as_ref() == field) {
            Some((_, thunk)) => thunk.force_tlc(self),
            None => Ok(Value::Nothing),
        }
    }
}
