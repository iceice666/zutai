//! Name resolution pass (M1 — **stub**).
//!
//! ## What this pass does (implement me)
//!
//! Resolves every `NameRef`-classified `LITERAL` node in the tree to the
//! `Symbol` that defines it, populating `ctx.resolution` and emitting
//! `ErrorCode::UnknownIdentifier` (E0020) for references that resolve to nothing.
//!
//! ## Algorithm
//!
//! Name resolution must be **two-phase** because the top-level scope is
//! *recursive* — forward references and mutual recursion between top-level
//! declarations are legal (spec `file-structure.md` §5.6). Block-local
//! bindings, by contrast, are strictly sequential.
//!
//! ### Phase 1 — collect all top-level names
//!
//! Walk `File::decls()` and call `ctx.scopes.define(Symbol { ... })` for
//! each `TopDecl`. The `SymbolKind` depends on the decl form:
//! - `InferredBinding` / `AnnotatedBinding` → `SymbolKind::Value`
//! - `FuncDecl` whose first body clause contains a `TYPE_FORM` child
//!   → `SymbolKind::TypeDef`
//! - `FuncDecl` otherwise → `SymbolKind::Function`
//!
//! On a name collision in this phase, emit `ErrorCode::DuplicateBinding` (E0010)
//! with a secondary label pointing at the prior definition.
//! (Note: `validation.rs` already emits E0010 for top-level duplicates. You can
//! skip re-emitting it here and rely on that pass, or remove it from `validation.rs`
//! and own it fully in this pass. Decide before implementing M1.)
//!
//! ### Phase 2 — resolve bodies
//!
//! Walk each `TopDecl`'s body expression recursively:
//!
//! - **`NameRef` literal in expression position**: call `ctx.scopes.resolve(name)`.
//!   If `Some(sym)`, insert into `ctx.resolution` mapping the use-site
//!   `TextRange` to `sym.def`. If `None`, emit E0020 on the identifier token.
//!
//! - **Block / local binding** (`LocalBinding` inside a `Block`): push a new
//!   child scope with `ctx.scopes.push_child()`, define the local name into it,
//!   continue resolving the rest of the block body, then `ctx.scopes.pop()` when
//!   leaving the block. These are sequential — a local may only reference
//!   bindings defined *before* it in the same block.
//!
//! - **Lambda** (`LambdaExpr`): push a child scope, define the parameter binding,
//!   resolve the body, pop.
//!
//! - **Function clause** (`Clause`): push a child scope, define type-params
//!   from `TypeParamList`, then define *binding* patterns from each `Pattern`
//!   (the ones classified as `NameRef` in pattern position), resolve the guard
//!   and body, pop.
//!
//! - **`match` expression** (`MatchExpr`): resolve the scrutinee; for each
//!   `MatchCase`, push a child scope, define binding patterns, resolve the
//!   guard and body arm, pop.
//!
//! ## CST gotchas
//!
//! - Use `ast_ext::classify_literal` to distinguish a `NameRef` (IDENT child)
//!   from an integer/atom/bool/etc. inside a `LITERAL` node — all of these are
//!   the same CST node kind.
//! - In *pattern* position the same `NameRef`-classified literal *introduces* a
//!   binding; in *expression* position it *references* one. Determine position
//!   from the parent node kind (a `CLAUSE`, `MATCH_CASE`, `TUPLE_PATTERN`, or
//!   `RECORD_PATTERN` child is in pattern position).
//! - `_tag` is a reserved identifier (tagged-unions §17.5). Emit a structural
//!   error if a binding or record field uses that name (or handle in the M4 pass).
//! - `FuncDecl` with a `TYPE_FORM` child is a *type definition*, not a function.
//!   Type parameters (`[A, B]`) introduce `SymbolKind::TypeParam` into the
//!   function/type's scope, not the file scope.
//!
//! ## Spec refs
//!
//! - `docs/v0_spec/04-general-mode/file-structure.md` §5.2–5.8
//! - `docs/v0_spec/08-reference/error-model.md` §28 (E0020)
//!
//! ## Testing
//!
//! When this pass is implemented, add a new fixture
//! `crates/general/fixtures/semantic_invalid/unknown_identifier.zt` that
//! exercises E0020, and a `valid/` fixture that exercises mutual recursion at
//! top level. Add tests in `crates/general/semantic/tests/acceptance.rs`.

use zutai_syntax::SyntaxNode;

use crate::context::AnalysisContext;
use crate::pass::Pass;

pub struct NameResolution;

impl Pass for NameResolution {
    fn name(&self) -> &'static str {
        "name-resolution"
    }

    fn run(&self, _root: &SyntaxNode, _ctx: &mut AnalysisContext) {
        // TODO (M1): implement two-phase collection + resolution as described above.
    }
}
