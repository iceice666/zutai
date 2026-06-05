//! Type checking pass (M2).
//!
//! ## What this pass does
//!
//! Verifies that every expression and declaration in HIR is well-typed
//! according to the v0 type system (bidirectional type checking + HM-style
//! let generalisation). Emits `ErrorCode::TypeMismatch` (E0030) and
//! `ErrorCode::UnknownField` (E0021) for violations.
//!
//! **Prerequisite:** CST-to-HIR lowering has run. Name references are already
//! resolved to `HirExprKind::Var(SymbolId)`, and symbols live in the HIR
//! `SymbolTable`.
//!
//! ## Algorithm overview
//!
//! Bidirectional type checking alternates between two modes:
//!
//! - **Check mode** (`check(expr, expected_ty)`) — you know what type the
//!   expression *should* have (e.g. from an annotation or context). Recurse
//!   into sub-expressions propagating the expectation.
//!
//! - **Infer mode** (`infer(expr) -> TyId`) — synthesise a type bottom-up.
//!   Returns the inferred type to the caller.
//!
//! Bidirectional is more powerful than pure inference: it lets you check
//! branches of `if`/`match` against a known type, and avoid annotation
//! requirements on lambdas when the context already knows the function type.
//!
//! ### Key rules
//!
//! **Annotated binding** `HirDecl::Value { ty: Some(T), body, .. }`:
//! - Elaborate `T` into a semantic `TyId`.
//! - Check `body` against the elaborated type.
//! - Assign the type to the symbol in `hir.symbols`.
//!
//! **Inferred binding** `HirDecl::Value { ty: None, body, .. }`:
//! - Infer a type for `body`.
//! - Assign the inferred type to the symbol in `hir.symbols`.
//! - HM let-generalisation: if the inferred type contains free type variables,
//!   generalise them into `∀` quantifiers at the binding boundary (spec §18.3).
//!
//! **Function declaration** `f :: [A, B] Clause+`:
//! - Bring `A, B` into scope as `SymbolKind::TypeParam` with kind `Ty::Var(...)`.
//! - For each `Clause`, check patterns against the expected input types, then
//!   check/infer the body.
//! - All clauses must produce the same return type.
//!
//! **Record expression** `{ host = "x"; port = 8080; }`:
//! - In check mode against a record type: verify every field declared in the
//!   type is present (no missing required fields) and no extra fields are given.
//!   Emit E0021 for unknown fields, E0030 for missing required fields.
//! - In infer mode: synthesise a closed record type from the present fields.
//!
//! **`if` expression**:
//! - Check the condition against `Bool` (E0030 if not).
//! - Both branches must check/infer to a compatible type.
//!
//! **Optional chaining** `x?.field`:
//! - If `x : T?`, the chain yields `U?` where `T.field : U`.
//! - `(T?)?` normalises to `T?` (no double-optional).
//!
//! **Reject constrained type params** `[A: Eq]`:
//! - The `[A: Eq]` syntax is v1 only (spec §18.1). If the parser lets it
//!   through (it currently does not — the grammar rejects `:`-bounded params),
//!   this pass must emit an error.
//!
//! ## HIR shape
//!
//! Type-position reconstruction already happened during lowering. Type checking
//! consumes `HirTypeId`, `HirExprId`, `HirDecl`, and `SymbolId` directly.
//!
//! ## Spec refs
//!
//! - `docs/v0_spec/05-type-system/` (all files)
//! - `docs/v0_spec/06-polymorphism/polymorphism.md` §18
//! - `docs/v0_spec/08-reference/error-model.md` §28 (E0021, E0030)

use text_size::TextRange;
use zutai_hir::HirFile;
use zutai_hir::decl::{HirDecl, HirDeclId};
use zutai_hir::expr::{HirArm, HirExprId, HirExprKind};
use zutai_hir::pat::{HirPatId, HirPatKind, HirTuplePatElem};
use zutai_hir::ty::{HirTyRef, HirTypeId, LitVal};
use zutai_syntax::diag::ErrorCode;

use crate::context::AnalysisContext;
use crate::elab::ty_of_hir;
use crate::pass::Pass;
use crate::ty::*;

pub struct TypeCheck;

impl Pass for TypeCheck {
    fn name(&self) -> &'static str {
        "type-check"
    }

    fn run(&self, hir: &mut HirFile, ctx: &mut AnalysisContext) {
        let mut checker = TypeChecker { hir, ctx };
        checker.check_file();
    }
}

struct TypeChecker<'a> {
    hir: &'a mut HirFile,
    ctx: &'a mut AnalysisContext,
}

impl<'a> TypeChecker<'a> {
    pub fn infer_expr(&mut self, expr_id: HirExprId) -> TyId {
        let expr = self.hir.exprs.get(expr_id);
        let kind = expr.kind.clone();
        let range = expr.range;

        match kind {
            // Literal
            HirExprKind::Lit(lit_val) => match lit_val {
                LitVal::None => NONE_TY,
                LitVal::Bool(_) => BOOL_TY,
                LitVal::Int(_) => INT_TY,
                LitVal::Float(_) => FLOAT_TY,
                LitVal::Text(_) => TEXT_TY,
                LitVal::Atom(name) => self.ctx.types.intern(Ty::Atom(name.to_string())),
            },

            // Variable — look up the symbol's already-elaborated type
            HirExprKind::Var(idx) => {
                if idx.is_error() {
                    UNKNOWN_TY
                } else {
                    self.hir
                        .symbols
                        .get(idx)
                        .ty
                        .map(|ty_ref| TyId(ty_ref.0))
                        .unwrap_or(UNKNOWN_TY)
                }
            }

            // Record expression (infer mode -> synthesise closed record type)
            HirExprKind::Record { fields } => {
                let record_fields: Vec<RecordField> = fields
                    .iter()
                    .map(|(name, expr_id)| {
                        let ty = self.infer_expr(*expr_id);
                        RecordField {
                            name: norm_name(name),
                            ty,
                            kind: crate::ty::FieldKind::Required,
                        }
                    })
                    .collect();
                self.ctx.types.intern(Ty::Record(record_fields))
            }

            HirExprKind::Tuple { items } => {
                let tuple_items: Vec<TupleElem> = items
                    .iter()
                    .map(|item| match item {
                        zutai_hir::expr::HirTupleExprElem::Positional(expr_id) => {
                            TupleElem::Positional(self.infer_expr(*expr_id))
                        }
                        zutai_hir::expr::HirTupleExprElem::Named(name, expr_id) => {
                            TupleElem::Named(norm_name(name), self.infer_expr(*expr_id))
                        }
                    })
                    .collect();
                self.ctx.types.intern(Ty::Tuple(tuple_items))
            }

            // List
            HirExprKind::List { items } => {
                let elem_ty = items
                    .first()
                    .map(|&e| self.infer_expr(e))
                    .unwrap_or(UNKNOWN_TY);
                for &item in items.get(1..).unwrap_or(&[]) {
                    let t = self.infer_expr(item);
                    self.unify(elem_ty, t, range);
                }
                self.ctx.types.intern(Ty::List(elem_ty))
            }

            // Lambda: synthesise param type as Unknown unless check mode has
            // an expected function type to push into the parameter pattern.
            HirExprKind::Lambda { body, .. } => {
                let param_ty = UNKNOWN_TY;
                let ret_ty = self.infer_expr(body);
                self.ctx.types.intern(Ty::Function {
                    param: param_ty,
                    ret: ret_ty,
                })
            }

            // Curried application
            HirExprKind::Apply { fun, arg } => {
                let fun_ty = self.infer_expr(fun);
                match self.ctx.types.get(fun_ty).clone() {
                    Ty::Function { param, ret } => {
                        self.check_expr(arg, param);
                        ret
                    }
                    Ty::Unknown => UNKNOWN_TY,
                    _ => {
                        self.type_mismatch(range, fun_ty, UNKNOWN_TY);
                        UNKNOWN_TY
                    }
                }
            }
            HirExprKind::BinOp { op, lhs, rhs } => {
                use zutai_hir::expr::BinaryOp;
                match op {
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                        let lhs_ty = self.infer_expr(lhs);
                        let rhs_ty = self.infer_expr(rhs);
                        self.unify(lhs_ty, rhs_ty, range)
                    }
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::And
                    | BinaryOp::Or => {
                        let lhs_ty = self.infer_expr(lhs);
                        let rhs_ty = self.infer_expr(rhs);
                        self.unify(lhs_ty, rhs_ty, range);
                        BOOL_TY
                    }
                }
            }
            HirExprKind::Let {
                name,
                ty,
                value,
                body,
            } => {
                let value_ty = match ty {
                    Some(hir_ty) => {
                        let expected = self.ty_of_hir(hir_ty);
                        self.check_expr(value, expected);
                        expected
                    }
                    None => self.infer_expr(value),
                };
                if !name.is_error() {
                    self.hir.symbols.get_mut(name).ty = Some(HirTyRef(value_ty.0));
                }
                self.infer_expr(body)
            }

            // If — condition must be Bool; branches must unify
            HirExprKind::If { cond, then_, else_ } => {
                self.check_expr(cond, BOOL_TY);
                let t = self.infer_expr(then_);
                let e = self.infer_expr(else_);
                self.unify(t, e, range)
            }

            // Match: infer scrutinee type, unify all arm body types
            HirExprKind::Match { scrutinee, arms } => {
                let scr_ty = self.infer_expr(scrutinee);
                let mut result_ty = UNKNOWN_TY;
                let unwrap_binds = self.match_has_none_arm(&arms);
                for arm in &arms {
                    self.check_pat(arm.pat, scr_ty, unwrap_binds);
                    if let Some(guard) = arm.guard {
                        self.check_expr(guard, BOOL_TY);
                    }
                    let arm_ty = self.infer_expr(arm.body);
                    if result_ty == UNKNOWN_TY {
                        result_ty = arm_ty;
                    } else {
                        result_ty = self.unify(result_ty, arm_ty, range);
                    }
                }
                result_ty
            }

            // Field access
            HirExprKind::Field { value, ref label } => {
                let label = label.clone();
                let obj_ty = self.infer_expr(value);
                self.infer_field_access(obj_ty, &label, range)
            }

            HirExprKind::Import { .. } => UNKNOWN_TY,

            // Annotation: `(expr : T)` — check expr against T, return T
            HirExprKind::Annot { expr, ty } => {
                let expected = self.ty_of_hir(ty);
                self.check_expr(expr, expected);
                expected
            }
            HirExprKind::Error => UNKNOWN_TY,
        }
    }

    pub fn check_expr(&mut self, expr_id: HirExprId, expected: TyId) {
        if expected == UNKNOWN_TY {
            self.infer_expr(expr_id);
            return;
        }

        let expr = self.hir.exprs.get(expr_id);
        let kind = expr.kind.clone();
        let range = expr.range;

        match kind {
            HirExprKind::Record { fields } => {
                self.check_record_expr(&fields, expected, range);
            }
            HirExprKind::Lambda { params, body } => {
                self.check_lambda_expr(&params, body, expected, range);
            }
            HirExprKind::If { cond, then_, else_ } => {
                self.check_expr(cond, BOOL_TY);
                self.check_expr(then_, expected);
                self.check_expr(else_, expected);
            }
            HirExprKind::Match { scrutinee, arms } => {
                let scr_ty = self.infer_expr(scrutinee);
                let unwrap_binds = self.match_has_none_arm(&arms);
                for arm in &arms {
                    self.check_pat(arm.pat, scr_ty, unwrap_binds);
                    if let Some(guard) = arm.guard {
                        self.check_expr(guard, BOOL_TY);
                    }
                    self.check_expr(arm.body, expected);
                }
            }
            HirExprKind::Annot { expr, ty } => {
                let annotated = self.ty_of_hir(ty);
                self.check_expr(expr, annotated);
                if !self.compatible(annotated, expected) {
                    self.type_mismatch(range, annotated, expected);
                }
            }
            _ => {
                let actual = self.infer_expr(expr_id);
                if !self.compatible(actual, expected) {
                    self.type_mismatch(range, actual, expected);
                }
            }
        }
    }

    // The entrypoint of type checking
    pub fn check_file(&mut self) {
        let decls = self.hir.decls.clone();
        decls.into_iter().for_each(|id| self.check_decl(id));

        self.infer_expr(self.hir.final_expr);
    }

    fn check_decl(&mut self, decl_id: HirDeclId) {
        match self.hir.decls_arena.get(decl_id).clone() {
            // x :Int = 42 => elaborate annotation, check body against it
            HirDecl::Value {
                name,
                ty: Some(hir_ty),
                body,
            } => {
                let expected = self.ty_of_hir(hir_ty);
                self.check_expr(body, expected);
                self.hir.symbols.get_mut(name).ty = Some(HirTyRef(expected.0))
            }

            // x := 42 => infer body, write back (HM generalise later)
            HirDecl::Value {
                name,
                ty: None,
                body,
            } => {
                let inferred = self.infer_expr(body);
                if !name.is_error() {
                    self.hir.symbols.get_mut(name).ty = Some(HirTyRef(inferred.0));
                }
            }
            // f :: [A] Clause+ => already lowered to Lambda+Match; infer/check sig
            HirDecl::Function {
                name, sig, body, ..
            } => match sig {
                Some(hir_ty) => {
                    let expected = self.ty_of_hir(hir_ty);
                    self.check_expr(body, expected);
                    self.hir.symbols.get_mut(name).ty = Some(HirTyRef(expected.0))
                }
                None => {
                    let inferred = self.infer_expr(body);
                    self.hir.symbols.get_mut(name).ty = Some(HirTyRef(inferred.0));
                }
            },
            // Nothing to check
            HirDecl::TypeDef { .. } => {}
        }
    }

    /// Infer the type of `value.label` or `value?.label`.
    /// - `obj_ty` is the already-inferred type of the left-hand side.
    /// - Returns the field's type, or UNKNOWN_TY if the field doesn't exist.
    fn infer_field_access(&mut self, obj_ty: TyId, label: &str, range: TextRange) -> TyId {
        let label = norm_name(label);
        match self.ctx.types.get(obj_ty).clone() {
            // Happy path: closed record type — look up the field
            Ty::Record(fields) => match fields.iter().find(|f| f.name == label) {
                Some(field) => match field.kind {
                    FieldKind::Required => field.ty,
                    FieldKind::Optional => self.ctx.types.intern(Ty::Optional(field.ty)),
                },
                None => {
                    self.ctx.error(
                        range,
                        ErrorCode::UnknownField,
                        format!("unknown field `{label}`"),
                    );
                    UNKNOWN_TY
                }
            },

            // Optional record: `x?.field` was desugared to Match in HIR,
            // so plain `.field` on a `T?` is a type error per spec §14.1.
            Ty::Optional(_) => UNKNOWN_TY,

            // Unknown propagates silently (avoids cascading after E0020)
            Ty::Unknown => UNKNOWN_TY,

            // Anything else can't have fields
            _ => UNKNOWN_TY,
        }
    }

    fn ty_of_hir(&mut self, hir_ty: HirTypeId) -> TyId {
        ty_of_hir(
            hir_ty,
            &self.hir.types,
            &self.hir.symbols,
            &mut self.ctx.types,
        )
    }

    fn unify(&mut self, a: TyId, b: TyId, range: TextRange) -> TyId {
        if a == b {
            return a;
        }
        if a == UNKNOWN_TY {
            return b;
        }
        if b == UNKNOWN_TY {
            return a;
        }
        if a == NONE_TY {
            return self.optional_of(b);
        }
        if b == NONE_TY {
            return self.optional_of(a);
        }
        if matches!(self.ctx.types.get(a), Ty::Atom(_))
            && matches!(self.ctx.types.get(b), Ty::Atom(_))
        {
            return self.union_of([a, b]);
        }
        if let Ty::Optional(inner) = self.ctx.types.get(a).clone()
            && (self.compatible(b, inner) || b == NONE_TY)
        {
            return a;
        }
        if let Ty::Optional(inner) = self.ctx.types.get(b).clone()
            && (self.compatible(a, inner) || a == NONE_TY)
        {
            return b;
        }
        if self.compatible(a, b) {
            return b;
        }
        if self.compatible(b, a) {
            return a;
        }
        self.type_mismatch(range, b, a);
        a // return left on mismatch to continue checking
    }

    fn check_lambda_expr(
        &mut self,
        params: &[HirPatId],
        body: HirExprId,
        expected: TyId,
        range: TextRange,
    ) {
        let Ty::Function { param, ret } = self.ctx.types.get(expected).clone() else {
            let actual = self.infer_expr(body);
            self.type_mismatch(range, actual, expected);
            return;
        };

        let Some((first, rest)) = params.split_first() else {
            self.check_expr(body, ret);
            return;
        };
        self.check_param_pat(*first, param);

        if rest.is_empty() {
            self.check_expr(body, ret);
            return;
        }

        let mut nested = body;
        for pat in rest.iter().rev() {
            nested = self.hir.exprs.alloc(zutai_hir::expr::HirExpr {
                kind: HirExprKind::Lambda {
                    params: vec![*pat],
                    body: nested,
                },
                range,
            });
        }
        self.check_expr(nested, ret);
    }

    fn check_record_expr(
        &mut self,
        fields: &[(String, HirExprId)],
        expected: TyId,
        range: TextRange,
    ) {
        let Ty::Record(expected_fields) = self.ctx.types.get(expected).clone() else {
            let actual = self.infer_expr_from_record_fields(fields);
            if !self.compatible(actual, expected) {
                self.type_mismatch(range, actual, expected);
            }
            return;
        };

        for (name, expr_id) in fields {
            let name = norm_name(name);
            match expected_fields.iter().find(|f| f.name == name) {
                Some(field) => {
                    let expected_ty = match field.kind {
                        FieldKind::Required => field.ty,
                        FieldKind::Optional => self.optional_of(field.ty),
                    };
                    self.check_expr(*expr_id, expected_ty);
                }
                None => self.ctx.error(
                    range,
                    ErrorCode::UnknownField,
                    format!("unknown field `{name}`"),
                ),
            }
        }

        for field in expected_fields
            .iter()
            .filter(|f| matches!(f.kind, FieldKind::Required))
        {
            if !fields.iter().any(|(name, _)| norm_name(name) == field.name) {
                self.ctx.error(
                    range,
                    ErrorCode::TypeMismatch,
                    format!("missing required field `{}`", field.name),
                );
            }
        }
    }

    fn infer_expr_from_record_fields(&mut self, fields: &[(String, HirExprId)]) -> TyId {
        let record_fields: Vec<RecordField> = fields
            .iter()
            .map(|(name, expr_id)| RecordField {
                name: norm_name(name),
                ty: self.infer_expr(*expr_id),
                kind: FieldKind::Required,
            })
            .collect();
        self.ctx.types.intern(Ty::Record(record_fields))
    }

    fn check_param_pat(&mut self, pat_id: HirPatId, expected: TyId) {
        self.check_pat(pat_id, expected, false);
    }

    fn check_pat(&mut self, pat_id: HirPatId, expected: TyId, unwrap_optional_bind: bool) {
        let pat = self.hir.pats.get(pat_id);
        let kind = pat.kind.clone();
        let range = pat.range;

        match kind {
            HirPatKind::Wildcard => {}
            HirPatKind::Bind(sym) => {
                if !sym.is_error() {
                    let bind_ty = if unwrap_optional_bind {
                        self.binding_type_for_pattern(expected)
                    } else {
                        expected
                    };
                    self.hir.symbols.get_mut(sym).ty = Some(HirTyRef(bind_ty.0));
                }
            }
            HirPatKind::Literal(lit) => {
                let actual = self.ty_of_lit(&lit);
                if !self.compatible(actual, expected) {
                    self.type_mismatch(range, actual, expected);
                }
            }
            HirPatKind::Paren(inner) => self.check_pat(inner, expected, unwrap_optional_bind),
            HirPatKind::Record { fields } => self.check_record_pat(&fields, expected, range),
            HirPatKind::Tuple { items } => self.check_tuple_pat(&items, expected, range),
            HirPatKind::Error => {}
        }
    }

    fn check_record_pat(
        &mut self,
        fields: &[(String, HirPatId)],
        expected: TyId,
        range: TextRange,
    ) {
        let Ty::Record(expected_fields) = self.ctx.types.get(expected).clone() else {
            if expected != UNKNOWN_TY {
                self.type_mismatch(range, UNKNOWN_TY, expected);
            }
            return;
        };
        for (name, pat_id) in fields {
            let name = norm_name(name);
            match expected_fields.iter().find(|f| f.name == name) {
                Some(field) => self.check_pat(*pat_id, field.ty, true),
                None => self.ctx.error(
                    range,
                    ErrorCode::UnknownField,
                    format!("unknown field `{name}`"),
                ),
            }
        }
    }

    fn check_tuple_pat(&mut self, items: &[HirTuplePatElem], expected: TyId, range: TextRange) {
        let expected = self
            .variant_for_tuple_pat(items, expected)
            .unwrap_or(expected);
        let Ty::Tuple(expected_items) = self.ctx.types.get(expected).clone() else {
            if expected != UNKNOWN_TY {
                self.type_mismatch(range, UNKNOWN_TY, expected);
            }
            return;
        };

        for (idx, item) in items.iter().enumerate() {
            match item {
                HirTuplePatElem::Positional(pat_id) => {
                    if let Some(TupleElem::Positional(ty)) = expected_items.get(idx) {
                        self.check_pat(*pat_id, *ty, true);
                    }
                }
                HirTuplePatElem::Named(name, pat_id) => {
                    let name = norm_name(name);
                    match expected_items.iter().find_map(|item| match item {
                        TupleElem::Named(field, ty) if *field == name => Some(*ty),
                        _ => None,
                    }) {
                        Some(ty) => self.check_pat(*pat_id, ty, true),
                        None => self.ctx.error(
                            range,
                            ErrorCode::UnknownField,
                            format!("unknown field `{name}`"),
                        ),
                    }
                }
            }
        }
    }

    fn variant_for_tuple_pat(&self, items: &[HirTuplePatElem], expected: TyId) -> Option<TyId> {
        let Ty::Union(variants) = self.ctx.types.get(expected) else {
            return None;
        };
        let Some(HirTuplePatElem::Positional(first_pat)) = items.first() else {
            return None;
        };
        let HirPatKind::Literal(LitVal::Atom(tag)) = &self.hir.pats.get(*first_pat).kind else {
            return None;
        };
        variants.iter().copied().find(|variant| {
            let Ty::Tuple(items) = self.ctx.types.get(*variant) else {
                return false;
            };
            let Some(TupleElem::Positional(first_ty)) = items.first() else {
                return false;
            };
            matches!(self.ctx.types.get(*first_ty), Ty::Atom(atom) if atom == tag)
        })
    }

    fn binding_type_for_pattern(&mut self, expected: TyId) -> TyId {
        match self.ctx.types.get(expected).clone() {
            Ty::Optional(inner) => inner,
            Ty::Union(variants) => {
                let non_none: Vec<TyId> =
                    variants.into_iter().filter(|ty| *ty != NONE_TY).collect();
                match non_none.as_slice() {
                    [single] => *single,
                    [] => expected,
                    _ => self.ctx.types.intern(Ty::Union(non_none)),
                }
            }
            _ => expected,
        }
    }

    fn match_has_none_arm(&self, arms: &[HirArm]) -> bool {
        arms.iter().any(|arm| {
            matches!(
                self.hir.pats.get(arm.pat).kind,
                HirPatKind::Literal(LitVal::None)
            )
        })
    }

    fn compatible(&self, actual: TyId, expected: TyId) -> bool {
        if actual == expected || actual == UNKNOWN_TY || expected == UNKNOWN_TY {
            return true;
        }

        match (self.ctx.types.get(actual), self.ctx.types.get(expected)) {
            (Ty::Param(_), _) | (_, Ty::Param(_)) => true,
            (Ty::List(actual), Ty::List(expected)) => self.compatible(*actual, *expected),
            (_, Ty::Optional(inner)) => actual == NONE_TY || self.compatible(actual, *inner),
            (Ty::Optional(inner), _) => self.compatible(*inner, expected),
            (_, Ty::Union(variants)) => variants
                .iter()
                .any(|variant| self.compatible(actual, *variant)),
            (Ty::Union(variants), _) => variants
                .iter()
                .all(|variant| self.compatible(*variant, expected)),
            (Ty::Record(actual_fields), Ty::Record(expected_fields)) => {
                self.record_shapes_compatible(actual_fields, expected_fields)
            }
            (Ty::Tuple(actual_items), Ty::Tuple(expected_items)) => {
                self.tuple_shapes_compatible(actual_items, expected_items)
            }
            (Ty::Function { param: ap, ret: ar }, Ty::Function { param: ep, ret: er }) => {
                self.compatible(*ep, *ap) && self.compatible(*ar, *er)
            }
            _ => false,
        }
    }

    fn record_shapes_compatible(&self, actual: &[RecordField], expected: &[RecordField]) -> bool {
        expected.iter().all(|expected_field| {
            match actual
                .iter()
                .find(|actual_field| actual_field.name == expected_field.name)
            {
                Some(actual_field) => self.compatible(actual_field.ty, expected_field.ty),
                None => matches!(expected_field.kind, FieldKind::Optional),
            }
        }) && actual.iter().all(|actual_field| {
            expected
                .iter()
                .any(|expected_field| expected_field.name == actual_field.name)
        })
    }

    fn tuple_shapes_compatible(&self, actual: &[TupleElem], expected: &[TupleElem]) -> bool {
        actual.len() == expected.len()
            && actual
                .iter()
                .zip(expected)
                .all(|(actual, expected)| match (actual, expected) {
                    (TupleElem::Positional(a), TupleElem::Positional(e)) => self.compatible(*a, *e),
                    (TupleElem::Named(an, a), TupleElem::Named(en, e)) if an == en => {
                        self.compatible(*a, *e)
                    }
                    _ => false,
                })
    }

    fn ty_of_lit(&mut self, lit: &LitVal) -> TyId {
        match lit {
            LitVal::None => NONE_TY,
            LitVal::Bool(_) => BOOL_TY,
            LitVal::Int(_) => INT_TY,
            LitVal::Float(_) => FLOAT_TY,
            LitVal::Text(_) => TEXT_TY,
            LitVal::Atom(name) => self.ctx.types.intern(Ty::Atom(name.clone())),
        }
    }

    fn optional_of(&mut self, inner: TyId) -> TyId {
        match self.ctx.types.get(inner).clone() {
            Ty::Optional(_) => inner,
            _ => self.ctx.types.intern(Ty::Optional(inner)),
        }
    }

    fn type_mismatch(&mut self, range: TextRange, actual: TyId, expected: TyId) {
        if actual == UNKNOWN_TY || expected == UNKNOWN_TY {
            return;
        }
        self.ctx.error(
            range,
            ErrorCode::TypeMismatch,
            format!(
                "type mismatch: expected {}, found {}",
                self.describe_ty(expected),
                self.describe_ty(actual)
            ),
        );
    }

    fn union_of<const N: usize>(&mut self, tys: [TyId; N]) -> TyId {
        let mut variants = Vec::new();
        for ty in tys {
            match self.ctx.types.get(ty).clone() {
                Ty::Union(inner) => {
                    for variant in inner {
                        if !variants.contains(&variant) {
                            variants.push(variant);
                        }
                    }
                }
                _ if !variants.contains(&ty) => variants.push(ty),
                _ => {}
            }
        }
        self.ctx.types.intern(Ty::Union(variants))
    }

    fn describe_ty(&self, ty: TyId) -> String {
        match self.ctx.types.get(ty) {
            Ty::Unknown => "unknown".to_string(),
            Ty::Int => "Int".to_string(),
            Ty::Float => "Float".to_string(),
            Ty::Text => "Text".to_string(),
            Ty::Bool => "Bool".to_string(),
            Ty::None => "None".to_string(),
            Ty::Atom(atom) => format!("#{atom}"),
            Ty::Optional(inner) => format!("{}?", self.describe_ty(*inner)),
            Ty::List(inner) => format!("List {}", self.describe_ty(*inner)),
            Ty::Record(_) => "record".to_string(),
            Ty::Union(_) => "union".to_string(),
            Ty::Tuple(_) => "tuple".to_string(),
            Ty::Function { .. } => "function".to_string(),
            Ty::Apply { .. } => "applied type".to_string(),
            Ty::Param(id) => format!("T{id}"),
        }
    }
}

fn norm_name(name: &str) -> String {
    name.trim().to_string()
}
