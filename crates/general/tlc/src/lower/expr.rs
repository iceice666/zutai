use rustc_hash::{FxHashMap, FxHashSet};

use zutai_hir::BindingId;
use zutai_syntax::{Span, ast::BinOp};
use zutai_thir::{
    ThirClause, ThirDeclKind, ThirExprId, ThirExprKind, ThirPatId, ThirPatKind, ThirRecordField,
    ThirRecordPatField, ThirTupleItem, ThirTuplePatItem, TypeId, TypeKind, TypeRecordField,
};

use crate::ir::{
    BuiltinOp, Literal, PrimTy, TlcAlt, TlcExpr, TlcExprId, TlcHandleClause, TlcPat, TlcPatItem,
    TlcTupleField, TlcTupleItem, TlcType, TlcTypeId,
};

use super::Lowerer;
type ForallLambdaDict = (BindingId, BindingId, TlcTypeId);
type ForallLambdaLayer = Vec<(BindingId, Vec<ForallLambdaDict>)>;
const CODE_EXPANSION_FUEL: u16 = 256;

#[derive(Clone)]
struct CodeExpansion {
    value: ThirExprId,
    frames: Vec<FxHashMap<BindingId, ThirExprId>>,
}

#[derive(Clone)]
struct CodeClosure {
    params: Vec<ThirPatId>,
    body: ThirExprId,
    frames: Vec<FxHashMap<BindingId, ThirExprId>>,
}

enum CompileTimeValue {
    Code(CodeExpansion),
    Closure(CodeClosure),
    Expr(CodeExpansion),
}

#[derive(Clone, Copy)]
enum WrapperKind {
    Optional,
    Maybe,
}

impl WrapperKind {
    fn absent_tag(self) -> &'static str {
        match self {
            WrapperKind::Optional => "none",
            WrapperKind::Maybe => "absent",
        }
    }

    fn present_tag(self) -> &'static str {
        match self {
            WrapperKind::Optional => "some",
            WrapperKind::Maybe => "present",
        }
    }
}

impl<'thir> Lowerer<'thir> {
    pub(super) fn lower_expr(&mut self, id: ThirExprId) -> TlcExprId {
        let expr = &self.thir.expr_arena[id];
        let span = expr.span;
        let thir_ty = expr.ty;
        let tlc_ty = self.lower_type(thir_ty);

        match expr.kind.clone() {
            ThirExprKind::Error => self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span),
            ThirExprKind::True => self.alloc_expr(TlcExpr::Lit(Literal::Bool(true)), tlc_ty, span),
            ThirExprKind::False => {
                self.alloc_expr(TlcExpr::Lit(Literal::Bool(false)), tlc_ty, span)
            }
            ThirExprKind::Integer(n) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Int(n)), tlc_ty, span)
            }
            ThirExprKind::Float(f) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Float(f)), tlc_ty, span)
            }
            ThirExprKind::Posit(literal) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Posit(literal)), tlc_ty, span)
            }
            ThirExprKind::String(s) => self.alloc_expr(TlcExpr::Lit(Literal::Str(s)), tlc_ty, span),
            ThirExprKind::Atom(s) => self.alloc_expr(TlcExpr::Lit(Literal::Atom(s)), tlc_ty, span),
            ThirExprKind::BindingRef {
                binding,
                instantiation,
                ..
            } => {
                if let Some(replacement) = self.code_substitution(binding) {
                    self.lower_expr(replacement)
                } else {
                    self.lower_binding_ref(binding, &instantiation, tlc_ty, thir_ty, span)
                }
            }
            ThirExprKind::Record(fields) => {
                let tlc_fields: Vec<(String, TlcExprId)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.lower_expr(f.value)))
                    .collect();
                self.alloc_expr(TlcExpr::Record(tlc_fields), tlc_ty, span)
            }
            ThirExprKind::RecordUpdate { receiver, fields } => {
                let receiver = self.lower_expr(receiver);
                let tlc_fields: Vec<(String, TlcExprId)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.lower_expr(f.value)))
                    .collect();
                self.alloc_expr(
                    TlcExpr::RecordUpdate {
                        receiver,
                        fields: tlc_fields,
                    },
                    tlc_ty,
                    span,
                )
            }
            ThirExprKind::Tuple(items) => {
                let tlc_items: Vec<TlcTupleItem> = items
                    .iter()
                    .map(|item| match item {
                        zutai_thir::ThirTupleItem::Named { name, value, .. } => {
                            TlcTupleItem::Named {
                                name: name.clone(),
                                value: self.lower_expr(*value),
                            }
                        }
                        zutai_thir::ThirTupleItem::Positional(v) => {
                            TlcTupleItem::Positional(self.lower_expr(*v))
                        }
                    })
                    .collect();
                self.alloc_expr(TlcExpr::Tuple(tlc_items), tlc_ty, span)
            }
            ThirExprKind::List(items) => {
                let tlc_items: Vec<TlcExprId> = items.iter().map(|&e| self.lower_expr(e)).collect();
                self.alloc_expr(TlcExpr::List(tlc_items), tlc_ty, span)
            }
            ThirExprKind::ListAppend { left, right } => {
                let left = self.lower_expr(left);
                let right = self.lower_expr(right);
                self.alloc_expr(TlcExpr::ListAppend(left, right), tlc_ty, span)
            }
            ThirExprKind::Access { receiver, field } => {
                let recv = self.lower_expr(receiver);
                self.alloc_expr(TlcExpr::GetField(recv, field), tlc_ty, span)
            }
            ThirExprKind::OptionalAccess { receiver, field } => {
                let recv = self.lower_expr(receiver);
                self.lower_optional_access(receiver, recv, field, tlc_ty, thir_ty, span)
            }
            ThirExprKind::Binary { op, lhs, rhs } => {
                let lhs_tlc = self.lower_expr(lhs);
                let rhs_tlc = self.lower_expr(rhs);
                let lhs_ty = self.thir.expr_arena[lhs].ty;

                if matches!(op, BinOp::And | BinOp::Or) {
                    return self.lower_logical_short_circuit(op, lhs_tlc, rhs_tlc, tlc_ty, span);
                }
                if op == BinOp::Coalesce {
                    return self.lower_coalesce(lhs_tlc, rhs_tlc, tlc_ty, span);
                }

                if op == BinOp::Ne {
                    if let Some(expr) = self
                        .lower_operator_method_call("!=", lhs_ty, lhs_tlc, rhs_tlc, tlc_ty, span)
                    {
                        return expr;
                    }
                    if let Some(eq_expr) = self
                        .lower_operator_method_call("==", lhs_ty, lhs_tlc, rhs_tlc, tlc_ty, span)
                    {
                        let bool_ty = self.alloc_type(TlcType::Prim(PrimTy::Bool));
                        let true_lit =
                            self.alloc_expr(TlcExpr::Lit(Literal::Bool(true)), bool_ty, span);
                        return self.alloc_expr(
                            TlcExpr::Builtin(BuiltinOp::Ne, eq_expr, true_lit),
                            tlc_ty,
                            span,
                        );
                    }
                } else if let Some(op_name) = binop_operator_method_name(op)
                    && let Some(expr) = self
                        .lower_operator_method_call(op_name, lhs_ty, lhs_tlc, rhs_tlc, tlc_ty, span)
                {
                    return expr;
                }

                let builtin = binop_to_builtin(op);
                self.alloc_expr(TlcExpr::Builtin(builtin, lhs_tlc, rhs_tlc), tlc_ty, span)
            }
            ThirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let scrutinee = self.lower_expr(cond);
                let then_expr = self.lower_expr(then_branch);
                let else_expr = self.lower_expr(else_branch);
                let alts = vec![
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(true)),
                        guard: None,
                        body: then_expr,
                    },
                    TlcAlt {
                        pat: TlcPat::Lit(Literal::Bool(false)),
                        guard: None,
                        body: else_expr,
                    },
                ];
                self.alloc_expr(TlcExpr::Case(scrutinee, alts), tlc_ty, span)
            }
            ThirExprKind::Block { bindings, result } => {
                let tail = self.lower_expr(result);
                bindings.iter().rev().fold(tail, |body, local| {
                    let value = self.lower_expr(local.value);
                    let ty = self.lower_type(local.ty);
                    self.alloc_expr(
                        TlcExpr::Let {
                            binding: local.binding,
                            ty,
                            value,
                            body,
                        },
                        tlc_ty,
                        local.span,
                    )
                })
            }
            ThirExprKind::Match { scrutinee, arms } => {
                let scrut = self.lower_expr(scrutinee);
                let alts: Vec<TlcAlt> = arms
                    .iter()
                    .map(|arm| self.lower_clause_as_alt(arm))
                    .collect();
                self.alloc_expr(TlcExpr::Case(scrut, alts), tlc_ty, span)
            }
            ThirExprKind::Lambda { params, body } => {
                self.lower_lambda(params, body, tlc_ty, thir_ty, span)
            }
            ThirExprKind::Apply {
                func,
                arg,
                instantiation,
                forall_instantiation,
            } => {
                if let Some(expr) = self.lower_overlay_full_apply(func, arg, thir_ty, span) {
                    return expr;
                }

                // Extract func binding info without holding a borrow while calling &mut self.
                let func_binding_info = {
                    let fe = &self.thir.expr_arena[func];
                    if let ThirExprKind::BindingRef { binding: b, .. } = fe.kind {
                        Some((b, fe.ty, fe.span))
                    } else {
                        None
                    }
                };

                if let Some((binding, func_thir_ty, func_span)) = func_binding_info {
                    if let Some(op) = self.builtin_effect_op(binding) {
                        let arg_tlc = self.lower_expr(arg);
                        return self.alloc_expr(
                            TlcExpr::Perform {
                                op: op.to_string(),
                                arg: arg_tlc,
                            },
                            tlc_ty,
                            span,
                        );
                    }

                    if self.thir.binding_names[binding.0 as usize] == "decode"
                        && let Some(&target) = instantiation.first()
                        && let Some(&sig) = self.decl_thir_types.get(&binding)
                        && let TypeKind::Function { from, to } =
                            self.thir.type_arena[sig.0 as usize].kind
                        && let Some((param, _)) = self
                            .fn_explicit_params
                            .get(&binding)
                            .and_then(|params| params.first())
                            .cloned()
                    {
                        let arg_tlc = self.lower_expr(arg);
                        let subst: FxHashMap<BindingId, TypeId> =
                            [(param, target)].into_iter().collect();
                        let data_ty = self.lower_expanded_type_with_subst(from, &subst);
                        let result_shape_ty = self.lower_expanded_type_with_subst(to, &subst);
                        self.expr_types.insert(arg_tlc, data_ty);
                        let target_ty = self.lower_type(target);
                        if let Some(decoded) = self.derive_from_data_value(
                            target,
                            target_ty,
                            arg_tlc,
                            data_ty,
                            result_shape_ty,
                            span,
                        ) {
                            self.expr_types.insert(decoded, tlc_ty);
                            return decoded;
                        }
                    }

                    if let Some(info) = self
                        .constraint_methods
                        .get(&binding)
                        .filter(|info| info.name == "fromData")
                        .cloned()
                        && let Some(&target) = instantiation.first()
                        && let Some(sig) = self.method_sig_for(info.constraint, "fromData")
                        && let TypeKind::Function { from, to } =
                            self.thir.type_arena[sig.0 as usize].kind
                    {
                        let arg_tlc = self.lower_expr(arg);
                        let subst: FxHashMap<BindingId, TypeId> =
                            [(info.constraint_param, target)].into_iter().collect();
                        let data_ty = self.lower_expanded_type_with_subst(from, &subst);
                        let result_shape_ty = self.lower_expanded_type_with_subst(to, &subst);
                        self.expr_types.insert(arg_tlc, data_ty);
                        let target_ty = self.lower_type(target);
                        if let Some(decoded) = self.derive_from_data_value(
                            target,
                            target_ty,
                            arg_tlc,
                            data_ty,
                            result_shape_ty,
                            span,
                        ) {
                            self.expr_types.insert(decoded, tlc_ty);
                            return decoded;
                        }
                    }

                    // Constraint-method / explicit-params dispatch: build the
                    // instantiated callee (TyApps + dict Apps) then apply the arg.
                    if let Some(callee) = self.lower_instantiated_callee(
                        binding,
                        func_thir_ty,
                        func_span,
                        &instantiation,
                        tlc_ty,
                        span,
                    ) {
                        let arg_tlc = self.lower_expr(arg);
                        return self.alloc_expr(TlcExpr::App(callee, arg_tlc), tlc_ty, span);
                    }
                }

                let mut func_tlc = self.lower_expr(func);
                if !forall_instantiation.is_empty() {
                    let func_thir_ty_id = self.thir.expr_arena[func].ty;
                    if let TypeKind::ForAll {
                        params,
                        param_bounds,
                        ..
                    } = self.thir.type_arena[func_thir_ty_id.0 as usize]
                        .kind
                        .clone()
                    {
                        for (i, (&_param, &inst_ty)) in
                            params.iter().zip(forall_instantiation.iter()).enumerate()
                        {
                            let ty_arg = self.lower_type(inst_ty);
                            let cur_ty = self.alloc_type(TlcType::Prim(PrimTy::Nothing));
                            func_tlc =
                                self.alloc_expr(TlcExpr::TyApp(func_tlc, ty_arg), cur_ty, span);
                            for &bound in &param_bounds[i] {
                                let dict = self.get_dict_expr(bound, inst_ty, span);
                                let after_dict_ty = self.alloc_type(TlcType::Prim(PrimTy::Nothing));
                                func_tlc = self.alloc_expr(
                                    TlcExpr::App(func_tlc, dict),
                                    after_dict_ty,
                                    span,
                                );
                            }
                        }
                    }
                }
                let arg_tlc = self.lower_expr(arg);
                self.alloc_expr(TlcExpr::App(func_tlc, arg_tlc), tlc_ty, span)
            }
            ThirExprKind::Perform { op, arg } => {
                let arg = self.lower_expr(arg);
                self.alloc_expr(TlcExpr::Perform { op, arg }, tlc_ty, span)
            }
            ThirExprKind::Resume { value } => {
                let value = self.lower_expr(value);
                self.alloc_expr(TlcExpr::Resume { value }, tlc_ty, span)
            }
            ThirExprKind::Handle {
                expr,
                value,
                finally,
                ops,
            } => {
                let expr = self.lower_expr(expr);
                let value = value.map(|value| self.lower_expr(value));
                let finally = finally.map(|finally| self.lower_expr(finally));
                let ops = ops
                    .into_iter()
                    .map(|clause| TlcHandleClause {
                        op: clause.op,
                        body: self.lower_expr(clause.body),
                    })
                    .collect();
                self.alloc_expr(
                    TlcExpr::Handle {
                        expr,
                        value,
                        finally,
                        ops,
                    },
                    tlc_ty,
                    span,
                )
            }
            ThirExprKind::Sequence(items) => {
                let items = items
                    .into_iter()
                    .map(|item| self.lower_expr(item))
                    .collect();
                self.alloc_expr(TlcExpr::Sequence(items), tlc_ty, span)
            }
            ThirExprKind::Import(source) => self.alloc_expr(TlcExpr::Import(source), tlc_ty, span),
            ThirExprKind::TypeValue(_) => {
                self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span)
            }
            ThirExprKind::WitnessReflect { constraint, target } => {
                let dict = constraint
                    .map(|constraint| self.get_dict_expr(constraint, target, span))
                    .unwrap_or_else(|| {
                        self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span)
                    });
                self.expr_types.insert(dict, tlc_ty);
                dict
            }
            ThirExprKind::Splice(code) => match self.resolve_code_expr(code) {
                Some(expansion) => self.lower_code_expansion(expansion),
                None => self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span),
            },
            // A standalone Code value is compile-time-only. Semantic validation
            // rejects escape; keep TLC construction total for error recovery.
            ThirExprKind::Quote(_) => self.alloc_expr(TlcExpr::Lit(Literal::Nothing), tlc_ty, span),
            ThirExprKind::TaggedValue { tag, payload } => {
                let payload_tlc = self.lower_expr(payload);
                self.alloc_expr(TlcExpr::Variant(tag, payload_tlc), tlc_ty, span)
            }
        }
    }

    fn code_substitution(&self, binding: BindingId) -> Option<ThirExprId> {
        self.code_frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(&binding).copied())
    }

    fn lower_code_expansion(&mut self, expansion: CodeExpansion) -> TlcExprId {
        let saved = std::mem::replace(&mut self.code_frames, expansion.frames);
        let value = self.lower_expr(expansion.value);
        self.code_frames = saved;
        value
    }

    fn resolve_code_expr(&self, id: ThirExprId) -> Option<CodeExpansion> {
        match self.eval_compile_time(id, self.code_frames.clone(), CODE_EXPANSION_FUEL)? {
            CompileTimeValue::Code(code) => Some(code),
            CompileTimeValue::Closure(_) | CompileTimeValue::Expr(_) => None,
        }
    }

    pub(super) fn lower_quoted_recipe_record(
        &mut self,
        constraint: BindingId,
        span: Span,
    ) -> Option<Vec<(String, TlcExprId)>> {
        let (body, definition) = self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding == constraint
                && let ThirDeclKind::Constraint {
                    recipe: Some(recipe),
                    ..
                } = &decl.kind
            {
                Some((recipe.body, decl.span))
            } else {
                None
            }
        })?;
        // Distinguish fuel exhaustion (a source diagnostic) from an ordinary
        // non-reducible recipe (silent fall-through to structural synthesizers).
        self.recipe_fuel_exhausted.set(false);
        let reduced = self.eval_compile_time(body, Vec::new(), CODE_EXPANSION_FUEL);
        if self.recipe_fuel_exhausted.replace(false) {
            let constraint_name = self.thir.binding_names[constraint.0 as usize].clone();
            self.diagnostics.push(zutai_thir::ThirDiagnostic {
                kind: zutai_thir::ThirDiagnosticKind::DeriveRecipeFuelExhausted {
                    constraint: constraint_name,
                    definition,
                },
                span,
            });
            return None;
        }
        // A `Code`-typed recipe body promises a witness record. If reduction
        // produced one, lower it. Otherwise, distinguish two cases by the body's
        // static type: a `Code`-typed body that failed to reduce (e.g. it stalls
        // on arithmetic or a comparison the pure reducer does not evaluate) is a
        // hard error — refuse rather than fall through to a structural witness the
        // recipe never described. A non-`Code` body (the `<T> => \x. x`
        // method-name form) legitimately falls through to the structural
        // synthesizers, so it returns `None` silently.
        let record = match reduced {
            Some(CompileTimeValue::Code(expansion)) => {
                match self.thir.expr_arena[expansion.value].kind.clone() {
                    ThirExprKind::Record(fields) => Some((fields, expansion.frames)),
                    _ => None,
                }
            }
            _ => None,
        };
        let Some((fields, frames)) = record else {
            let body_is_code = matches!(
                self.thir.type_arena[self.thir.expr_arena[body].ty.0 as usize].kind,
                TypeKind::Code(_)
            );
            if body_is_code {
                let constraint_name = self.thir.binding_names[constraint.0 as usize].clone();
                self.diagnostics.push(zutai_thir::ThirDiagnostic {
                    kind: zutai_thir::ThirDiagnosticKind::DeriveRecipeIrreducible {
                        constraint: constraint_name,
                        definition,
                    },
                    span,
                });
            }
            return None;
        };
        let saved = std::mem::replace(&mut self.code_frames, frames);
        let lowered = fields
            .into_iter()
            .map(|field| (field.name, self.lower_expr(field.value)))
            .collect();
        self.code_frames = saved;
        Some(lowered)
    }

    fn eval_compile_time(
        &self,
        id: ThirExprId,
        frames: Vec<FxHashMap<BindingId, ThirExprId>>,
        fuel: u16,
    ) -> Option<CompileTimeValue> {
        let Some(fuel) = fuel.checked_sub(1) else {
            self.recipe_fuel_exhausted.set(true);
            return None;
        };
        match self.thir.expr_arena[id].kind.clone() {
            ThirExprKind::Quote(value) => {
                Some(CompileTimeValue::Code(CodeExpansion { value, frames }))
            }
            ThirExprKind::BindingRef { binding, .. } => {
                if let Some(value) = frames
                    .iter()
                    .rev()
                    .find_map(|frame| frame.get(&binding).copied())
                {
                    return self.eval_compile_time(value, frames, fuel);
                }
                self.thir.decls.iter().find_map(|&decl_id| {
                    let decl = &self.thir.decl_arena[decl_id];
                    if decl.binding != binding {
                        return None;
                    }
                    match &decl.kind {
                        ThirDeclKind::Value { value, .. } => {
                            self.eval_compile_time(*value, frames.clone(), fuel)
                        }
                        ThirDeclKind::Function { clauses, .. } if clauses.len() == 1 => {
                            let clause = &clauses[0];
                            Some(CompileTimeValue::Closure(CodeClosure {
                                params: clause.patterns.clone(),
                                body: clause.body,
                                frames: frames.clone(),
                            }))
                        }
                        _ => None,
                    }
                })
            }
            ThirExprKind::Lambda { params, body } => Some(CompileTimeValue::Closure(CodeClosure {
                params,
                body,
                frames,
            })),
            ThirExprKind::Apply { func, arg, .. } => {
                let CompileTimeValue::Closure(mut closure) =
                    self.eval_compile_time(func, frames, fuel)?
                else {
                    return None;
                };
                let pattern = closure.params.first().copied()?;
                let mut frame = FxHashMap::default();
                if !self.bind_compile_time_pattern(pattern, arg, &mut frame) {
                    return None;
                }
                closure.frames.push(frame);
                closure.params.remove(0);
                if closure.params.is_empty() {
                    self.eval_compile_time(closure.body, closure.frames, fuel)
                } else {
                    Some(CompileTimeValue::Closure(closure))
                }
            }
            ThirExprKind::Block { bindings, result } => {
                let mut frame = FxHashMap::default();
                for binding in bindings {
                    frame.insert(binding.binding, binding.value);
                }
                let mut frames = frames;
                frames.push(frame);
                self.eval_compile_time(result, frames, fuel)
            }
            ThirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond = self.eval_compile_time(cond, frames.clone(), fuel)?;
                match self.compile_time_bool(cond) {
                    Some(true) => self.eval_compile_time(then_branch, frames, fuel),
                    Some(false) => self.eval_compile_time(else_branch, frames, fuel),
                    None => None,
                }
            }
            ThirExprKind::Match { scrutinee, arms } => {
                // Try each arm in order; the matcher reduces the scrutinee (and
                // its sub-expressions) structurally. A matched arm's body is
                // evaluated in the match's own frames extended with the pattern
                // bindings — pattern-leaf sub-exprs are bound raw and resolved
                // lazily, mirroring the `Apply` binding convention. Recipes match
                // on closed literal configs, so this frame flattening is exact.
                for arm in &arms {
                    if arm.guard.is_some() {
                        return None;
                    }
                    let pattern = *arm.patterns.first()?;
                    let mut frame = FxHashMap::default();
                    if self
                        .match_compile_time_pattern(pattern, scrutinee, &frames, &mut frame, fuel)?
                    {
                        let mut arm_frames = frames.clone();
                        arm_frames.push(frame);
                        return self.eval_compile_time(arm.body, arm_frames, fuel);
                    }
                }
                None
            }
            ThirExprKind::Perform { .. }
            | ThirExprKind::Handle { .. }
            | ThirExprKind::Resume { .. }
            | ThirExprKind::Import(_) => None,
            _ => Some(CompileTimeValue::Expr(CodeExpansion { value: id, frames })),
        }
    }

    fn bind_compile_time_pattern(
        &self,
        pattern: ThirPatId,
        value: ThirExprId,
        frame: &mut FxHashMap<BindingId, ThirExprId>,
    ) -> bool {
        match self.thir.pat_arena[pattern].kind {
            ThirPatKind::Bind(binding) => {
                frame.insert(binding, value);
                true
            }
            ThirPatKind::Wildcard => true,
            _ => false,
        }
    }

    /// Structurally match a THIR pattern against a compile-time expression for
    /// the recipe reducer. Returns `Some(true)` on a match (binding sub-exprs
    /// into `frame`), `Some(false)` on a decisive non-match (try the next arm),
    /// and `None` when the match is undecidable at compile time (abort the whole
    /// reduction and fall back to the structural synthesizers). Bound leaves
    /// capture the raw scrutinee sub-expr id, resolved lazily in the arm frames
    /// — exact for the closed literal configs recipes match on.
    fn match_compile_time_pattern(
        &self,
        pattern: ThirPatId,
        expr_id: ThirExprId,
        frames: &[FxHashMap<BindingId, ThirExprId>],
        frame: &mut FxHashMap<BindingId, ThirExprId>,
        fuel: u16,
    ) -> Option<bool> {
        let pat_kind = self.thir.pat_arena[pattern].kind.clone();
        // Irrefutable leaves need no scrutinee reduction.
        match &pat_kind {
            ThirPatKind::Error => return None,
            ThirPatKind::Wildcard => return Some(true),
            ThirPatKind::Bind(binding) => {
                frame.insert(*binding, expr_id);
                return Some(true);
            }
            _ => {}
        }
        // Refutable patterns require the scrutinee's structural head.
        let CompileTimeValue::Expr(scrut) =
            self.eval_compile_time(expr_id, frames.to_vec(), fuel)?
        else {
            return None;
        };
        let value = self.thir.expr_arena[scrut.value].kind.clone();
        let sub = scrut.frames.as_slice();
        match (pat_kind, value) {
            (ThirPatKind::True, ThirExprKind::True) => Some(true),
            (ThirPatKind::True, ThirExprKind::False) => Some(false),
            (ThirPatKind::False, ThirExprKind::False) => Some(true),
            (ThirPatKind::False, ThirExprKind::True) => Some(false),
            (ThirPatKind::Integer(p), ThirExprKind::Integer(v)) => Some(p == v),
            (ThirPatKind::Float(p), ThirExprKind::Float(v)) => Some(p.to_bits() == v.to_bits()),
            (ThirPatKind::Posit(p), ThirExprKind::Posit(v)) => Some(p == v),
            (ThirPatKind::String(p), ThirExprKind::String(v)) => Some(p == v),
            (ThirPatKind::Atom(p), ThirExprKind::Atom(v)) => Some(p == v),
            (ThirPatKind::ListNil, ThirExprKind::List(items)) => Some(items.is_empty()),
            (ThirPatKind::Record(pat_fields), ThirExprKind::Record(val_fields)) => {
                self.match_compile_time_record(&pat_fields, &val_fields, sub, frame, fuel)
            }
            (
                ThirPatKind::TaggedValue {
                    tag: ptag,
                    payload: pat_fields,
                },
                ThirExprKind::TaggedValue { tag: vtag, payload },
            ) => {
                if ptag != vtag {
                    return Some(false);
                }
                // A tagged value's payload is a single expr; recipes tag record
                // payloads, so resolve it as a record to match the field patterns.
                let CompileTimeValue::Expr(pl) =
                    self.eval_compile_time(payload, sub.to_vec(), fuel)?
                else {
                    return None;
                };
                let ThirExprKind::Record(val_fields) = self.thir.expr_arena[pl.value].kind.clone()
                else {
                    return None;
                };
                self.match_compile_time_record(&pat_fields, &val_fields, &pl.frames, frame, fuel)
            }
            (ThirPatKind::Tuple(pat_items), ThirExprKind::Tuple(val_items)) => {
                self.match_compile_time_tuple(&pat_items, &val_items, sub, frame, fuel)
            }
            // A nullary-variant (atom) pattern and a payload-carrying variant
            // value — or vice versa — are distinct constructors of the same
            // union. Both scrutinee and pattern have decided structural heads, so
            // this is a decisive non-match: try the next arm rather than stalling
            // the whole reduction (which would strand a structurally recursive
            // recipe and fall through to a broken witness).
            (ThirPatKind::Atom(_), ThirExprKind::TaggedValue { .. })
            | (ThirPatKind::TaggedValue { .. }, ThirExprKind::Atom(_)) => Some(false),
            _ => None,
        }
    }

    fn match_compile_time_record(
        &self,
        pat_fields: &[ThirRecordPatField],
        val_fields: &[ThirRecordField],
        frames: &[FxHashMap<BindingId, ThirExprId>],
        frame: &mut FxHashMap<BindingId, ThirExprId>,
        fuel: u16,
    ) -> Option<bool> {
        for pf in pat_fields {
            let vf = val_fields.iter().find(|f| f.name == pf.name)?;
            if !self.match_compile_time_pattern(pf.pattern, vf.value, frames, frame, fuel)? {
                return Some(false);
            }
        }
        Some(true)
    }

    fn match_compile_time_tuple(
        &self,
        pat_items: &[ThirTuplePatItem],
        val_items: &[ThirTupleItem],
        frames: &[FxHashMap<BindingId, ThirExprId>],
        frame: &mut FxHashMap<BindingId, ThirExprId>,
        fuel: u16,
    ) -> Option<bool> {
        if pat_items.len() != val_items.len() {
            return Some(false);
        }
        for (pi, vi) in pat_items.iter().zip(val_items) {
            let (sub_pat, sub_expr) = match (pi, vi) {
                (ThirTuplePatItem::Positional(p), ThirTupleItem::Positional(v)) => (*p, *v),
                (
                    ThirTuplePatItem::Named {
                        name: pn,
                        pattern: p,
                        ..
                    },
                    ThirTupleItem::Named {
                        name: vn, value: v, ..
                    },
                ) if pn == vn => (*p, *v),
                _ => return None,
            };
            if !self.match_compile_time_pattern(sub_pat, sub_expr, frames, frame, fuel)? {
                return Some(false);
            }
        }
        Some(true)
    }

    fn compile_time_bool(&self, value: CompileTimeValue) -> Option<bool> {
        let CompileTimeValue::Expr(value) = value else {
            return None;
        };
        match self.thir.expr_arena[value.value].kind {
            ThirExprKind::True => Some(true),
            ThirExprKind::False => Some(false),
            _ => None,
        }
    }

    fn lower_overlay_full_apply(
        &mut self,
        func: ThirExprId,
        base: ThirExprId,
        target: TypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        let (patch, deep) = self.overlay_full_apply_parts(func)?;
        let base = self.lower_expr(base);
        let patch_fields = self.record_literal_fields(patch, &mut FxHashSet::default())?;
        self.lower_overlay_record(
            base,
            patch_fields,
            target,
            &FxHashMap::default(),
            deep,
            span,
        )
    }

    fn overlay_full_apply_parts(&self, func: ThirExprId) -> Option<(ThirExprId, bool)> {
        let ThirExprKind::Apply {
            func: builtin,
            arg: patch,
            ..
        } = &self.thir.expr_arena[func].kind
        else {
            return None;
        };
        let ThirExprKind::BindingRef { binding, .. } = &self.thir.expr_arena[*builtin].kind else {
            return None;
        };
        match self.builtin_overlay_name(*binding)? {
            "overlay" => Some((*patch, false)),
            "overlayDeep" => Some((*patch, true)),
            _ => None,
        }
    }

    fn builtin_overlay_name(&self, binding: BindingId) -> Option<&str> {
        let name = self.thir.binding_names.get(binding.0 as usize)?.as_str();
        if name != "overlay" && name != "overlayDeep" {
            return None;
        }
        let first = self
            .thir
            .binding_names
            .iter()
            .position(|candidate| candidate == name)?;
        (first == binding.0 as usize).then_some(name)
    }

    fn record_literal_fields(
        &self,
        expr: ThirExprId,
        seen: &mut FxHashSet<BindingId>,
    ) -> Option<Vec<(String, ThirExprId)>> {
        match &self.thir.expr_arena[expr].kind {
            ThirExprKind::Record(fields) => Some(
                fields
                    .iter()
                    .map(|field| (field.name.clone(), field.value))
                    .collect(),
            ),
            ThirExprKind::BindingRef { binding, .. } => {
                if !seen.insert(*binding) {
                    return None;
                }
                let value = self.value_decl_expr(*binding)?;
                let fields = self.record_literal_fields(value, seen);
                seen.remove(binding);
                fields
            }
            _ => None,
        }
    }

    fn value_decl_expr(&self, binding: BindingId) -> Option<ThirExprId> {
        self.thir.decls.iter().find_map(|&decl_id| {
            let decl = &self.thir.decl_arena[decl_id];
            if decl.binding != binding {
                return None;
            }
            match &decl.kind {
                zutai_thir::ThirDeclKind::Value { value, .. } => Some(*value),
                _ => None,
            }
        })
    }

    fn lower_overlay_record(
        &mut self,
        base: TlcExprId,
        patch_fields: Vec<(String, ThirExprId)>,
        target: TypeId,
        subst: &FxHashMap<BindingId, TypeId>,
        deep: bool,
        span: Span,
    ) -> Option<TlcExprId> {
        let (target_fields, _tail, env) = self.record_shape_with_subst(target, subst)?;
        let result_ty = self.lower_type_with_subst(target, subst);
        let mut updates = Vec::with_capacity(patch_fields.len());
        for (name, patch_value) in patch_fields {
            let field = target_fields.iter().find(|field| field.name == name)?;
            let value = self.lower_overlay_field(base, patch_value, field, &env, deep, span)?;
            updates.push((name, value));
        }
        if updates.is_empty() {
            return Some(base);
        }
        Some(self.alloc_expr(
            TlcExpr::RecordUpdate {
                receiver: base,
                fields: updates,
            },
            result_ty,
            span,
        ))
    }

    fn lower_overlay_field(
        &mut self,
        base: TlcExprId,
        patch_value: ThirExprId,
        field: &TypeRecordField,
        subst: &FxHashMap<BindingId, TypeId>,
        deep: bool,
        span: Span,
    ) -> Option<TlcExprId> {
        if deep && self.record_shape_with_subst(field.ty, subst).is_some() {
            if field.optional {
                return None;
            }
            let field_ty = self.lower_type_with_subst(field.ty, subst);
            let base_field =
                self.alloc_expr(TlcExpr::GetField(base, field.name.clone()), field_ty, span);
            let patch_fields =
                self.record_literal_fields(patch_value, &mut FxHashSet::default())?;
            return self.lower_overlay_record(
                base_field,
                patch_fields,
                field.ty,
                subst,
                true,
                span,
            );
        }
        Some(self.lower_expr(patch_value))
    }

    /// Build the instantiated callee expression for a reference to `binding` at
    /// `instantiation` — the type-application / dictionary-passing prefix an
    /// `Apply` injects before the value argument. Returns `None` when `binding`
    /// needs no such dispatch (the caller then uses the plain `Var`, possibly with
    /// InferVar poly-scheme `TyApp`s). Shared by the `Apply` callee path and a
    /// standalone `BindingRef` (a polymorphic *value* used outside callee
    /// position, e.g. `empty :: <A> Stream A`).
    fn lower_instantiated_callee(
        &mut self,
        binding: BindingId,
        callee_thir_ty: TypeId,
        callee_span: Span,
        instantiation: &[TypeId],
        tlc_ty: TlcTypeId,
        span: Span,
    ) -> Option<TlcExprId> {
        if instantiation.is_empty() {
            return None;
        }

        // Constraint method: dispatch via GetField on the active dict param.
        if let Some(info) = self.constraint_methods.get(&binding).cloned() {
            // Recover the method sig's exact type-var order (deduped, sorted by
            // binding id) — reproduces THIR's `collect_type_vars`, positionally
            // aligned with `instantiation` even when the method omits a declared
            // param. Fall back to constraint-param + method-params if unavailable.
            let vars: Vec<BindingId> = self
                .method_sig_for(info.constraint, &info.name)
                .map(|sig| self.collect_thir_type_vars(sig))
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| {
                    let mut v: Vec<BindingId> = Vec::with_capacity(1 + info.method_params.len());
                    v.push(info.constraint_param);
                    v.extend(info.method_params.iter().copied());
                    v.sort_by_key(|b| b.0);
                    v.dedup();
                    v
                });
            let index_of = |b: BindingId| vars.iter().position(|v| *v == b);

            // The constraint param's instantiation selects the dict.
            let dict_inst = index_of(info.constraint_param)
                .and_then(|i| instantiation.get(i).copied())
                .unwrap_or(instantiation[0]);
            let dict_expr = self.get_dict_expr(info.constraint, dict_inst, callee_span);
            let method_ty = self.lower_type(callee_thir_ty);
            let method_name = info.name.clone();
            let mut acc = self.alloc_expr(
                TlcExpr::GetField(dict_expr, method_name.clone()),
                method_ty,
                span,
            );
            self.register_dict_field_slot(acc, info.constraint, &method_name);
            // Record the concrete dispatch key (the instantiated operand type) so
            // the interpreter can dispatch an imported witness method to the
            // instance whose target matches the operand. An abstract/unkeyable
            // operand yields "" (never matches a witness → dispatch refuses).
            let dispatch_key = self
                .structural_witness_key(dict_inst, &mut rustc_hash::FxHashSet::default())
                .unwrap_or_default();
            self.dict_dispatch_keys.insert(acc, dispatch_key);
            // Each method-level type param becomes a `TyApp`, in declaration order.
            for &mp in &info.method_params {
                if let Some(i) = index_of(mp)
                    && let Some(&inst_ty) = instantiation.get(i)
                {
                    let ty_arg = self.lower_type(inst_ty);
                    acc = self.alloc_expr(TlcExpr::TyApp(acc, ty_arg), method_ty, span);
                }
            }
            return Some(acc);
        }

        // Explicit-params function: inject TyApp + dict App over the plain Var.
        if let Some(explicit_params) = self.fn_explicit_params.get(&binding).cloned() {
            let fn_var_ty = self.lower_type(callee_thir_ty);
            let mut cur = self.alloc_expr(TlcExpr::Var(binding), fn_var_ty, callee_span);
            for (i, (_, constraint_bindings)) in explicit_params.iter().enumerate() {
                if i < instantiation.len() {
                    let inst_ty_id = instantiation[i];
                    let ty_arg = self.lower_type(inst_ty_id);
                    cur = self.alloc_expr(TlcExpr::TyApp(cur, ty_arg), tlc_ty, span);
                    for &cst_b in constraint_bindings.iter() {
                        let dict = self.get_dict_expr(cst_b, inst_ty_id, span);
                        cur = self.alloc_expr(TlcExpr::App(cur, dict), tlc_ty, span);
                    }
                }
            }
            return Some(cur);
        }

        None
    }

    fn lower_binding_ref(
        &mut self,
        binding: BindingId,
        instantiation: &[TypeId],
        tlc_ty: TlcTypeId,
        ref_thir_ty: TypeId,
        span: zutai_syntax::Span,
    ) -> TlcExprId {
        if let Some(op) = self.builtin_effect_op(binding)
            && let TlcType::Fun(arg_ty, result_ty, _) = self.type_arena[tlc_ty].clone()
        {
            let arg_binding = self.fresh_synth_binding();
            let arg_var = self.alloc_expr(TlcExpr::Var(arg_binding), arg_ty, span);
            let perform = self.alloc_expr(
                TlcExpr::Perform {
                    op: op.to_string(),
                    arg: arg_var,
                },
                result_ty,
                span,
            );
            return self.alloc_expr(TlcExpr::Lam(arg_binding, arg_ty, perform), tlc_ty, span);
        }

        // A polymorphic value used outside callee position carries a recorded
        // instantiation (THIR `lower_binding_ref` freshened its `<A>` TypeVars):
        // emit the same TyApp + dict-App prefix the Apply path would.
        if let Some(callee) =
            self.lower_instantiated_callee(binding, ref_thir_ty, span, instantiation, tlc_ty, span)
        {
            return callee;
        }

        let var_expr = self.alloc_expr(TlcExpr::Var(binding), tlc_ty, span);
        let scheme = self.thir.poly_schemes.get(&binding).cloned();
        let Some(vars) = scheme else {
            return var_expr;
        };
        if vars.is_empty() {
            return var_expr;
        }
        let Some(&decl_thir_ty) = self.decl_thir_types.get(&binding) else {
            return var_expr;
        };
        let instantiation = self.extract_instantiation(&vars, decl_thir_ty, ref_thir_ty);
        instantiation
            .into_iter()
            .fold(var_expr, |expr, (_, ty_arg)| {
                self.alloc_expr(TlcExpr::TyApp(expr, ty_arg), tlc_ty, span)
            })
    }

    fn builtin_effect_op(&self, binding: BindingId) -> Option<&'static str> {
        if !self
            .thir
            .binding_kinds
            .get(binding.0 as usize)
            .is_some_and(|kind| *kind == zutai_hir::BindingKind::BuiltinValue)
        {
            return None;
        }
        match self.thir.binding_names.get(binding.0 as usize)?.as_str() {
            "print" => Some("io.print"),
            "loadZti" => Some("load.zti"),
            "loadZt" => Some("load.zt"),
            _ => None,
        }
    }

    fn lower_clause_as_alt(&mut self, clause: &ThirClause) -> TlcAlt {
        let pat = if clause.patterns.is_empty() {
            TlcPat::Wildcard
        } else {
            self.lower_pat(clause.patterns[0])
        };
        let guard = clause.guard.map(|g| self.lower_expr(g));
        let body = self.lower_expr(clause.body);
        TlcAlt { pat, guard, body }
    }

    fn lower_logical_short_circuit(
        &mut self,
        op: BinOp,
        lhs: TlcExprId,
        rhs: TlcExprId,
        ty: TlcTypeId,
        span: Span,
    ) -> TlcExprId {
        let true_lit = self.alloc_expr(TlcExpr::Lit(Literal::Bool(true)), ty, span);
        let false_lit = self.alloc_expr(TlcExpr::Lit(Literal::Bool(false)), ty, span);
        let alts = match op {
            BinOp::And => vec![
                TlcAlt {
                    pat: TlcPat::Lit(Literal::Bool(true)),
                    guard: None,
                    body: rhs,
                },
                TlcAlt {
                    pat: TlcPat::Lit(Literal::Bool(false)),
                    guard: None,
                    body: false_lit,
                },
            ],
            BinOp::Or => vec![
                TlcAlt {
                    pat: TlcPat::Lit(Literal::Bool(true)),
                    guard: None,
                    body: true_lit,
                },
                TlcAlt {
                    pat: TlcPat::Lit(Literal::Bool(false)),
                    guard: None,
                    body: rhs,
                },
            ],
            _ => unreachable!("only logical operators short-circuit"),
        };
        self.alloc_expr(TlcExpr::Case(lhs, alts), ty, span)
    }

    fn lower_coalesce(
        &mut self,
        value: TlcExprId,
        fallback: TlcExprId,
        ty: TlcTypeId,
        span: Span,
    ) -> TlcExprId {
        let some_binding = self.fresh_synth_binding();
        let some_value = self.alloc_expr(TlcExpr::Var(some_binding), ty, span);
        let present_binding = self.fresh_synth_binding();
        let present_value = self.alloc_expr(TlcExpr::Var(present_binding), ty, span);
        self.alloc_expr(
            TlcExpr::Case(
                value,
                vec![
                    TlcAlt {
                        pat: TlcPat::Atom("none".to_string()),
                        guard: None,
                        body: fallback,
                    },
                    TlcAlt {
                        pat: TlcPat::Atom("absent".to_string()),
                        guard: None,
                        body: fallback,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant("none".to_string(), Box::new(TlcPat::Wildcard)),
                        guard: None,
                        body: fallback,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant("absent".to_string(), Box::new(TlcPat::Wildcard)),
                        guard: None,
                        body: fallback,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "some".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "0".to_string(),
                                TlcPat::Bind(some_binding),
                            )])),
                        ),
                        guard: None,
                        body: some_value,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            "present".to_string(),
                            Box::new(TlcPat::Record(vec![(
                                "0".to_string(),
                                TlcPat::Bind(present_binding),
                            )])),
                        ),
                        guard: None,
                        body: present_value,
                    },
                ],
            ),
            ty,
            span,
        )
    }

    fn lower_optional_access(
        &mut self,
        receiver: ThirExprId,
        recv: TlcExprId,
        field: String,
        ty: TlcTypeId,
        result_thir_ty: TypeId,
        span: Span,
    ) -> TlcExprId {
        let receiver_ty = self.thir.expr_arena[receiver].ty;
        let Some((wrapper, inner_thir_ty)) = self.thir_wrapper_inner(receiver_ty) else {
            return self.alloc_expr(TlcExpr::GetField(recv, field), ty, span);
        };
        let Some((_, result_inner_ty)) = self.thir_wrapper_inner(result_thir_ty) else {
            return self.alloc_expr(TlcExpr::GetField(recv, field), ty, span);
        };

        let inner_tlc_ty = self.lower_type(inner_thir_ty);
        let result_inner_tlc_ty = self.lower_type(result_inner_ty);
        let bind = self.fresh_synth_binding();
        let bound_record = self.alloc_expr(TlcExpr::Var(bind), inner_tlc_ty, span);
        let projected = self.alloc_expr(
            TlcExpr::GetField(bound_record, field),
            result_inner_tlc_ty,
            span,
        );
        let tuple_ty = self.alloc_type(TlcType::Tuple(vec![TlcTupleField::Positional(
            result_inner_tlc_ty,
        )]));
        let payload = self.alloc_expr(
            TlcExpr::Tuple(vec![TlcTupleItem::Positional(projected)]),
            tuple_ty,
            span,
        );
        let present_body = self.alloc_expr(
            TlcExpr::Variant(wrapper.present_tag().to_string(), payload),
            ty,
            span,
        );
        let absent_body = self.alloc_expr(
            TlcExpr::Lit(Literal::Atom(wrapper.absent_tag().to_string())),
            ty,
            span,
        );

        self.alloc_expr(
            TlcExpr::Case(
                recv,
                vec![
                    TlcAlt {
                        pat: TlcPat::Atom(wrapper.absent_tag().to_string()),
                        guard: None,
                        body: absent_body,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            wrapper.absent_tag().to_string(),
                            Box::new(TlcPat::Wildcard),
                        ),
                        guard: None,
                        body: absent_body,
                    },
                    TlcAlt {
                        pat: TlcPat::Variant(
                            wrapper.present_tag().to_string(),
                            Box::new(TlcPat::Record(vec![("0".to_string(), TlcPat::Bind(bind))])),
                        ),
                        guard: None,
                        body: present_body,
                    },
                ],
            ),
            ty,
            span,
        )
    }

    fn thir_wrapper_inner(&self, ty: TypeId) -> Option<(WrapperKind, TypeId)> {
        match self.thir.type_arena[ty.0 as usize].kind {
            TypeKind::Optional(inner) => Some((WrapperKind::Optional, inner)),
            TypeKind::Maybe(inner) => Some((WrapperKind::Maybe, inner)),
            TypeKind::Alias(binding) => self
                .type_alias_body(binding)
                .and_then(|body| self.thir_wrapper_inner(body)),
            _ => None,
        }
    }

    pub(super) fn lower_pat(&mut self, id: ThirPatId) -> TlcPat {
        match self.thir.pat_arena[id].kind.clone() {
            ThirPatKind::Error | ThirPatKind::Wildcard => TlcPat::Wildcard,
            ThirPatKind::Bind(b) => TlcPat::Bind(b),
            ThirPatKind::True => TlcPat::Lit(Literal::Bool(true)),
            ThirPatKind::False => TlcPat::Lit(Literal::Bool(false)),
            ThirPatKind::Integer(n) => TlcPat::Lit(Literal::Int(n)),
            ThirPatKind::Float(f) => TlcPat::Lit(Literal::Float(f)),
            ThirPatKind::Posit(literal) => TlcPat::Lit(Literal::Posit(literal)),
            ThirPatKind::String(s) => TlcPat::Lit(Literal::Str(s)),
            ThirPatKind::Atom(s) => TlcPat::Atom(s),
            ThirPatKind::Tuple(items) => {
                let tlc_items: Vec<TlcPatItem> = items
                    .iter()
                    .map(|item| match item {
                        zutai_thir::ThirTuplePatItem::Named { name, pattern, .. } => {
                            TlcPatItem::Named {
                                name: name.clone(),
                                pat: self.lower_pat(*pattern),
                            }
                        }
                        zutai_thir::ThirTuplePatItem::Positional(p) => {
                            TlcPatItem::Positional(self.lower_pat(*p))
                        }
                    })
                    .collect();
                TlcPat::Tuple(tlc_items)
            }
            ThirPatKind::ListNil => TlcPat::ListNil,
            ThirPatKind::ListCons { head, tail } => TlcPat::ListCons(
                Box::new(self.lower_pat(head)),
                Box::new(self.lower_pat(tail)),
            ),
            ThirPatKind::Record(fields) => {
                let tlc_fields: Vec<(String, TlcPat)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), self.lower_pat(f.pattern)))
                    .collect();
                TlcPat::Record(tlc_fields)
            }
            ThirPatKind::TaggedValue { tag, payload } => {
                let inner = if payload.is_empty() {
                    // Bare atom arm: `#dev` — no payload to bind.
                    Box::new(TlcPat::Wildcard)
                } else {
                    // Tagged-payload arm: `(#circle, radius: r)` — match as record.
                    let fields: Vec<(String, TlcPat)> = payload
                        .iter()
                        .map(|f| (f.name.clone(), self.lower_pat(f.pattern)))
                        .collect();
                    Box::new(TlcPat::Record(fields))
                };
                TlcPat::Variant(tag, inner)
            }
        }
    }

    fn binary_method_type(
        &mut self,
        operand_ty: TypeId,
        result_ty: TlcTypeId,
    ) -> (TlcTypeId, TlcTypeId) {
        use crate::ir::Row;

        let operand_tlc_ty = self.lower_type(operand_ty);
        let after_first = self.alloc_type(TlcType::Fun(operand_tlc_ty, result_ty, Row::REmpty));
        let full = self.alloc_type(TlcType::Fun(operand_tlc_ty, after_first, Row::REmpty));
        (full, after_first)
    }

    fn lower_operator_method_call(
        &mut self,
        op_name: &str,
        operand_ty: TypeId,
        lhs_tlc: TlcExprId,
        rhs_tlc: TlcExprId,
        result_ty: TlcTypeId,
        span: zutai_syntax::Span,
    ) -> Option<TlcExprId> {
        let guard = self.defining_op_witness.clone();
        for info in self.operator_methods.clone() {
            if info.name != op_name || !info.method_params.is_empty() {
                continue;
            }

            // Self-recursion guard: if we are lowering the body of this very
            // operator method and the call would dispatch back to it, use the
            // builtin instead. This makes `(==) = \a b. a == b` mean "delegate to
            // the primitive" rather than loop forever.
            if let Some((guard_binding, guard_op)) = &guard
                && guard_op == op_name
                && self.concrete_witness_binding(info.constraint, operand_ty)
                    == Some(*guard_binding)
            {
                continue;
            }

            let Some(dict) = self.try_get_dict_expr(info.constraint, operand_ty, span) else {
                continue;
            };
            let (method_ty, after_first_ty) = self.binary_method_type(operand_ty, result_ty);
            let method_name = info.name.clone();
            let method = self.alloc_expr(
                TlcExpr::GetField(dict, method_name.clone()),
                method_ty,
                span,
            );
            self.register_dict_field_slot(method, info.constraint, &method_name);
            let first = self.alloc_expr(TlcExpr::App(method, lhs_tlc), after_first_ty, span);
            return Some(self.alloc_expr(TlcExpr::App(first, rhs_tlc), result_ty, span));
        }

        None
    }

    pub(super) fn lower_lambda(
        &mut self,
        params: Vec<ThirPatId>,
        body: ThirExprId,
        outer_ty: TlcTypeId,
        thir_ty: TypeId,
        span: zutai_syntax::Span,
    ) -> TlcExprId {
        let forall_layers = self.prepare_forall_lambda_layers(thir_ty);
        let body_expr = self.lower_expr(body);
        for layer in &forall_layers {
            for &(param, ref dicts) in layer {
                for &(bound, _, _) in dicts {
                    self.active_dict_params.remove(&(bound.0, param.0));
                }
            }
        }

        // Peel `outer_ty` so each abstraction layer is typed with its own slice
        // (`∀a. rest`, `dict -> rest`, or `param -> rest`) instead of sharing the
        // full `outer_ty`. Sharing it gives an inner layer a type from the wrong
        // position, which the Dataflow structural validator rejects with an ICE.
        // This mirrors the per-layer wrapping in `lower/decl.rs`.
        //
        // `outer_ty` for a polymorphic lambda is `∀a…. dict… -> value-fun`: the
        // forall/dict prefix wraps the value-function type. Record each
        // forall-layer binder's own type while advancing `cur` past the prefix,
        // then peel the value arrows from the value-function type that remains.
        let mut cur = outer_ty;
        let mut forall_layer_tys: Vec<Vec<(TlcTypeId, Vec<TlcTypeId>)>> =
            Vec::with_capacity(forall_layers.len());
        for layer in &forall_layers {
            let mut layer_tys = Vec::with_capacity(layer.len());
            for (_param, dicts) in layer {
                let tylam_ty = cur;
                cur = match self.type_arena[cur].clone() {
                    TlcType::ForAll(_, _, body) => body,
                    _ => cur,
                };
                let mut dict_tys = Vec::with_capacity(dicts.len());
                for _ in dicts {
                    dict_tys.push(cur);
                    cur = match self.type_arena[cur].clone() {
                        TlcType::Fun(_, result, _) => result,
                        _ => cur,
                    };
                }
                layer_tys.push((tylam_ty, dict_tys));
            }
            forall_layer_tys.push(layer_tys);
        }

        // `cur` is now the value-function type; peel one arrow per value parameter
        // so each curried value lambda is typed `param -> rest`.
        let arity = params.len();
        let mut value_layer_tys = Vec::with_capacity(arity);
        for _ in 0..arity {
            value_layer_tys.push(cur);
            cur = match self.type_arena[cur].clone() {
                TlcType::Fun(_, result, _) => result,
                _ => cur,
            };
        }

        let mut expr = body_expr;
        for (i, &pat_id) in params.iter().enumerate().rev() {
            let pat = &self.thir.pat_arena[pat_id];
            let (param_binding, param_ty) = match pat.kind {
                ThirPatKind::Bind(b) => (b, self.lower_type(pat.ty)),
                _ => {
                    let fresh = self.fresh_synth_binding();
                    let ty = self.lower_type(pat.ty);
                    (fresh, ty)
                }
            };
            expr = self.alloc_expr(
                TlcExpr::Lam(param_binding, param_ty, expr),
                value_layer_tys[i],
                span,
            );
        }

        for (layer, layer_tys) in forall_layers.iter().zip(forall_layer_tys.iter()).rev() {
            for (&(param, ref dicts), (tylam_ty, dict_tys)) in
                layer.iter().zip(layer_tys.iter()).rev()
            {
                for (&(_, dict_param, dict_ty), &lam_ty) in dicts.iter().zip(dict_tys.iter()).rev()
                {
                    expr = self.alloc_expr(TlcExpr::Lam(dict_param, dict_ty, expr), lam_ty, span);
                }
                let tyvar = self.named_tyvar(param);
                let kind = self.kind_for_type_param(param);
                expr = self.alloc_expr(TlcExpr::TyLam(tyvar, kind, expr), *tylam_ty, span);
            }
        }
        expr
    }

    fn prepare_forall_lambda_layers(&mut self, thir_ty: TypeId) -> Vec<ForallLambdaLayer> {
        let mut layers = Vec::new();
        let mut current = thir_ty;
        loop {
            match self.thir.type_arena[current.0 as usize].kind.clone() {
                TypeKind::ForAll {
                    params,
                    param_bounds,
                    body,
                } => {
                    let mut layer = Vec::with_capacity(params.len());
                    for (param, bounds) in params.into_iter().zip(param_bounds) {
                        let mut dicts = Vec::with_capacity(bounds.len());
                        for bound in bounds {
                            let dict_param = self.fresh_synth_binding();
                            let dict_ty = self.alloc_type(TlcType::Record(crate::ir::Row::REmpty));
                            self.active_dict_params
                                .insert((bound.0, param.0), dict_param);
                            self.active_dict_types.insert(dict_param, dict_ty);
                            dicts.push((bound, dict_param, dict_ty));
                        }
                        layer.push((param, dicts));
                    }
                    layers.push(layer);
                    current = body;
                }
                _ => return layers,
            }
        }
    }
}

fn binop_operator_method_name(op: BinOp) -> Option<&'static str> {
    match op {
        BinOp::Eq => Some("=="),
        BinOp::Ne => Some("!="),
        BinOp::Lt => Some("<"),
        BinOp::Le => Some("<="),
        BinOp::Gt => Some(">"),
        BinOp::Ge => Some(">="),
        _ => None,
    }
}

fn binop_to_builtin(op: BinOp) -> BuiltinOp {
    match op {
        BinOp::Add => BuiltinOp::Add,
        BinOp::Sub => BuiltinOp::Sub,
        BinOp::Mul => BuiltinOp::Mul,
        BinOp::Div => BuiltinOp::Div,
        BinOp::Rem => BuiltinOp::Rem,
        BinOp::Eq => BuiltinOp::Eq,
        BinOp::Ne => BuiltinOp::Ne,
        BinOp::Lt => BuiltinOp::Lt,
        BinOp::Le => BuiltinOp::Le,
        BinOp::Gt => BuiltinOp::Gt,
        BinOp::Ge => BuiltinOp::Ge,
        BinOp::And => BuiltinOp::And,
        BinOp::Or => BuiltinOp::Or,
        BinOp::Coalesce => BuiltinOp::Coalesce,
    }
}
