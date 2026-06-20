use super::*;

/// Normalized target-type key for constraint witness lookup.
///
/// After THIR zonking every witness target is one of these; the key is used to
/// match a call-site concrete type against the registered witness dictionaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum WitnessTargetKey {
    Int,
    Float,
    Bool,
    Str,
    Atom,
    /// Named type alias or generic alias application, keyed by BindingId.0.
    Named(u32),
    Structural(String),
}

/// A parametric (conditional) witness such as `Eq @(List A) :: <A: Eq>`.
///
/// Its TLC value is a function `forall params. (component dicts) -> Record`,
/// so resolving it at a concrete call site means structurally matching `target`
/// against the call-site type, then applying the witness to the recursively
/// resolved component dictionaries.
#[derive(Clone)]
pub(crate) struct ConditionalWitness {
    /// Witness decl binding (the TLC value to apply).
    pub(crate) binding: BindingId,
    /// Constraint BindingId.0 this witness satisfies.
    pub(crate) constraint: u32,
    /// Witness target type, containing the witness params as `TypeVar` holes.
    pub(crate) target: TypeId,
    /// Witness type params, in declaration order; each gets a `TyApp`.
    pub(crate) params: Vec<BindingId>,
    /// Per-param constraint bounds, parallel to `params`; each bound gets an
    /// `App` of the recursively resolved component dict.
    pub(crate) param_bounds: Vec<Vec<BindingId>>,
}
/// Per-constraint-method dispatch info, keyed by the method's `BindingId`.
/// Lets the Apply arm split a call site's `instantiation` vector into the
/// constraint-param entry (selects the dict) and the method-level params
/// (each becomes a `TyApp` on the fetched method).
#[derive(Clone)]
pub(crate) struct ConstraintMethodInfo {
    /// The constraint's own `BindingId` (for dict lookup).
    pub(crate) constraint: BindingId,
    /// Method name (the dict field to `GetField`).
    pub(crate) name: String,
    /// The constraint's type parameter (`@F`); its instantiation selects the dict.
    pub(crate) constraint_param: BindingId,
    /// The method's own type parameters (`<A, B>`); each becomes a `TyApp`.
    pub(crate) method_params: Vec<BindingId>,
}

impl<'thir> Lowerer<'thir> {
    /// Map a THIR TypeId to a `WitnessTargetKey` for witness lookup.
    /// Returns `None` for types that cannot serve as witness targets (TypeVar, InferVar, etc.).
    pub(crate) fn thir_type_to_witness_key(&self, ty: TypeId) -> Option<WitnessTargetKey> {
        match &self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::Int => Some(WitnessTargetKey::Int),
            TypeKind::Float => Some(WitnessTargetKey::Float),
            TypeKind::Bool | TypeKind::True | TypeKind::False => Some(WitnessTargetKey::Bool),
            TypeKind::Text => Some(WitnessTargetKey::Str),
            TypeKind::Atom(_) => Some(WitnessTargetKey::Atom),
            TypeKind::Alias(b) => Some(WitnessTargetKey::Named(b.0)),
            TypeKind::AliasApply { binding, .. } => Some(WitnessTargetKey::Named(binding.0)),
            TypeKind::Record(_, _)
            | TypeKind::Tuple(_)
            | TypeKind::Union(_, _)
            | TypeKind::List(_)
            | TypeKind::Optional(_)
            | TypeKind::Function { .. } => self.thir_type_to_resolved_witness_key(ty),
            TypeKind::Con(b) => Some(WitnessTargetKey::Named(b.0)),
            TypeKind::Apply { .. } => {
                let (head, _) = self.thir_app_spine(ty);
                match &self.thir.type_arena[head.0 as usize].kind {
                    TypeKind::Alias(b) | TypeKind::Con(b) => Some(WitnessTargetKey::Named(b.0)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Try to build a TLC expression for passing the witness dict at a call site.
    ///
    /// If `inst_type_id` is an abstract `TypeVar`, threads the active dict param.
    /// If it is a concrete type, looks up the registered witness decl or a
    /// matching conditional witness. Returns `None` when no dictionary resolves.
    pub(crate) fn try_get_dict_expr(
        &mut self,
        cst_binding: BindingId,
        inst_type_id: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        use crate::ir::Row;

        let thir_kind = self.thir.type_arena[inst_type_id.0 as usize].kind.clone();
        match thir_kind {
            TypeKind::TypeVar(tv_binding) => {
                let dp = *self
                    .active_dict_params
                    .get(&(cst_binding.0, tv_binding.0))?;
                let dp_ty = self
                    .active_dict_types
                    .get(&dp)
                    .copied()
                    .unwrap_or_else(|| self.alloc_type(TlcType::Record(Row::REmpty)));
                Some(self.alloc_expr(TlcExpr::Var(dp), dp_ty, span))
            }
            _ => {
                if let Some(key) = self.thir_type_to_witness_key(inst_type_id)
                    && let Some(&wb) = self.witness_bindings.get(&(cst_binding.0, key))
                {
                    let wb_ty = self
                        .decl_thir_types
                        .get(&wb)
                        .copied()
                        .map(|thir_ty| self.lower_type(thir_ty))
                        .unwrap_or_else(|| self.alloc_type(TlcType::Record(Row::REmpty)));
                    return Some(self.alloc_expr(TlcExpr::Var(wb), wb_ty, span));
                }
                self.resolve_conditional_witness(cst_binding, inst_type_id, span)
            }
        }
    }

    /// Build a TLC expression for passing the witness dict at a call site.
    ///
    /// Returns a `Lit(Nothing)` placeholder on failure (undefined witness) to
    /// preserve existing named-method and bounded-call lowering behavior.
    pub(crate) fn get_dict_expr(
        &mut self,
        cst_binding: BindingId,
        inst_type_id: TypeId,
        span: Span,
    ) -> TlcExprId {
        self.try_get_dict_expr(cst_binding, inst_type_id, span)
            .unwrap_or_else(|| {
                use crate::ir::{Literal, PrimTy};

                let ty = self.alloc_type(TlcType::Prim(PrimTy::Nothing));
                self.alloc_expr(TlcExpr::Lit(Literal::Nothing), ty, span)
            })
    }
}

pub(super) fn row_tail_key(tail: RowTail) -> String {
    match tail {
        RowTail::Closed => String::new(),
        RowTail::Open => "...".to_string(),
        RowTail::Param(b) => format!("...#{}", b.0),
        RowTail::Infer(v) => format!("...?{v}"),
    }
}
