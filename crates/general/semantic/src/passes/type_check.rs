//! Type checking pass (M2 — **stub**).
//!
//! ## What this pass does (implement me)
//!
//! Verifies that every expression and declaration in the tree is well-typed
//! according to the v0 type system (bidirectional type checking + HM-style
//! let generalisation). Emits `ErrorCode::TypeMismatch` (E0030) and
//! `ErrorCode::UnknownField` (E0021) for violations.
//!
//! **Prerequisite:** the name-resolution pass (M1) must have run first.
//! Type checking uses `ctx.resolution` to look up what a `NameRef` refers to,
//! and `ctx.scopes` to retrieve the `Symbol::ty` that was assigned during
//! or after phase 1.
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
//! **Annotated binding** `x : T = expr`:
//! - Infer/elaborate `T` as a type expression (it's parsed as an `Expr` node
//!   in type position — see CST gap note below).
//! - Check `expr` against the elaborated type.
//! - Assign the type to the symbol in `ctx.scopes`.
//!
//! **Inferred binding** `x := expr`:
//! - Infer a type for `expr`.
//! - Assign the inferred type to the symbol in `ctx.scopes`.
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
//! ## CST gap: type position
//!
//! The type annotation `T` in `x : T = expr` is parsed as an `Expr` node
//! stored as the `ty()` child of `AnnotatedBinding`. You must *elaborate*
//! this expression as a type:
//! - `LITERAL(IDENT)` with an uppercase-initial name → type constructor
//!   (look it up in `ctx.scopes` as `SymbolKind::TypeDef` or `TypeParam`).
//! - `CALL_EXPR` → type application (`List Int`, `Pair A B`).
//! - `TUPLE_EXPR` with a leading `ATOM` → variant type.
//! - `RECORD_EXPR` in type-position (actually `TYPE_RECORD` node) → record type.
//! - etc.
//!
//! ## Fixtures to flip when done
//!
//! - `crates/general/fixtures/semantic_invalid/closed_records.zt` → `invalid/`
//! - `crates/general/fixtures/semantic_invalid/union_membership.zt` → `invalid/`
//!
//! ## Spec refs
//!
//! - `docs/v0_spec/05-type-system/` (all files)
//! - `docs/v0_spec/06-polymorphism/polymorphism.md` §18
//! - `docs/v0_spec/08-reference/error-model.md` §28 (E0021, E0030)

use zutai_syntax::SyntaxNode;

use crate::context::AnalysisContext;
use crate::pass::Pass;

pub struct TypeCheck;

impl Pass for TypeCheck {
    fn name(&self) -> &'static str {
        "type-check"
    }

    fn run(&self, _root: &SyntaxNode, _ctx: &mut AnalysisContext) {
        // TODO (M2): implement bidirectional type checking as described above.
        // Prerequisite: NameResolution (M1) must be complete.
    }
}
