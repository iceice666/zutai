# Semantic analysis (`zutai-semantic`) — implementation guide

## Context

`crates/general/semantic/` (package `zutai-semantic`) is the semantic analysis layer for
Zutai general mode (`.zt`). It sits downstream of `zutai-syntax` and `zutai-hir`: feed it
the `SyntaxNode` returned by `zutai_syntax::parse(src).syntax()`, it lowers CST to HIR
with `zutai_hir::lower_file(root)`, then runs semantic checks and returns an
`AnalysisResult`.

The CLI (`crates/cli/src/main.rs`) already calls `zutai_semantic::analyze` after `parse`
and merges the diagnostic vecs before rendering. HIR lowering already emits name-resolution
diagnostics; semantic passes are still mostly stubbed, so each implemented pass gives the
CLI more real errors.

Source of truth for every semantic rule: `docs/v0_spec/` — especially
`04-general-mode/`, `05-type-system/`, `06-polymorphism/`, and
`08-reference/error-model.md` §28 (the authoritative error-code list).

---

## Crate map

```
crates/general/hir/src/
  lower/                CST -> HIR lowering, including M1 name resolution
    decl.rs             two-phase top-level declaration collection + body lowering
    expr.rs             expression lowering; local `:=` is sequential
    pat.rs              pattern lowering; identifier patterns introduce symbols
    ty.rs               type-position lowering into HirType
    ctx.rs              LowerCtx, diagnostics, symbol/scopes arenas
  expr.rs               HirExpr/HirExprKind, HirArm
  decl.rs               HirDecl
  ty.rs                 HirType/HirTypeKind, LitVal, HirTyRef
  symbol.rs             SymbolId, SymbolKind, SymbolTable

crates/general/semantic/src/
  lib.rs                 pub fn analyze(&SyntaxNode) -> AnalysisResult
  pass.rs                HIR-only Pass trait + default_passes() registry
  context.rs             AnalysisContext { scopes, resolution, types, diagnostics }
                           + ctx.error / ctx.warning / ctx.error_with_label
  scope.rs               legacy CST-scope support; HIR lowering owns active name scopes
  resolution.rs          legacy/use-site TextRange -> def-site TextRange map
                          (currently not populated by HIR lowering)
  ty.rs                  TyId, Ty, TyInterner for semantic types
  ast_ext.rs             LitClass + classify_literal() — the #1 CST-gap bridge
  surface_checks.rs      CST-only semantic/surface checks run before HIR passes
  passes/
    type_check.rs        TypeCheck: Pass        (M2)
    exhaustiveness.rs    Exhaustiveness: Pass   (M3)
  tests/
    acceptance.rs        smoke tests over all valid + semantic_invalid fixtures
```

**Current pipeline:** `semantic::analyze` lowers CST to `HirFile`, merges lowering
diagnostics into `AnalysisContext`, elaborates HIR type annotations, then runs the
semantic pass registry. Semantic passes are HIR-only:

```rust
pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, hir: &mut HirFile, ctx: &mut AnalysisContext);
}
```

Use HIR as the semantic-pass contract. Checks that need exact user-written syntax or
CST-only source ranges should run in a separate `surface_checks` phase before HIR passes
(or in `zutai-syntax::validation` if they are purely syntactic). M2 should be implemented
against `HirFile` and its `SymbolTable`, not by re-resolving names from CST.

The diagnostic model (types, error codes, builder helpers) is in `zutai_syntax::diag` —
do not fork it. `ErrorCode` already reserves all v0 semantic codes:

| Code  | Variant                | Meaning                          |
|-------|------------------------|----------------------------------|
| E0020 | `UnknownIdentifier`    | Name not in scope                |
| E0021 | `UnknownField`         | Field not declared in record/union |
| E0030 | `TypeMismatch`         | Expression type disagrees        |
| E0031 | `NonExhaustiveMatch`   | `match`/clauses don't cover all cases |
| E0040 | `InvalidImportPath`    | Import can't be resolved         |

---

## The five CST gotchas

These are non-obvious mappings between the CST and semantic intent. HIR lowering already
absorbs most of these gaps; use this section when changing lowering or any pass that still
walks CST directly.

### 1. There is no `NameRef` node

A variable reference, `_`, `42`, `"x"`, `true`, `none`, and `#ok` are all `LITERAL`
nodes. Distinguish them by the inner token kind:

```
classify_literal(node) -> Option<LitClass>
  IDENT       → NameRef       (variable reference or pattern binding)
  UNDERSCORE  → Wildcard
  INT/FLOAT   → Int/Float     (also MINUS INT/FLOAT for negatives)
  STRING      → Str
  ATOM        → Atom (#name)
  KW_TRUE/FALSE → Bool
  KW_NONE     → NoneLit
```

`ast_ext::classify_literal` exists in `zutai-semantic`; `zutai-hir::lower` also keeps its
own copy to avoid a dependency cycle. Use the local classifier for whichever crate you are
editing.

### 2. Type position is not in the tree

`List Int` is a `CALL_EXPR`; `(A, B)` in a type annotation is a `TUPLE_EXPR`. The parser's
internal `Ctx::Type` is not persisted in CST. `zutai_hir::lower::ty::lower_type` is now the
central place that reconstructs type position from declaration/type-field context and
produces `HirType`.

### 3. Patterns overlap `LITERAL`

In a `CLAUSE` or `MATCH_CASE`, a `NameRef`-classified `LITERAL` *introduces* a binding.
In expression position the same kind *references* one. Determine position from the
parent node kind.

Only `WILDCARD_PATTERN`, `TUPLE_PATTERN`, and `RECORD_PATTERN` are distinct node kinds;
all other pattern forms (binding pattern `n`, atom pattern `#ok`, literal pattern `0`)
are `LITERAL` nodes.

### 4. Type definitions are `FUNC_DECL` nodes

`Server :: type { host : Text; port : Int; }` produces a `FUNC_DECL` with a `TYPE_FORM`
child — there is no `TypeDecl`. Check:

```rust
let is_type_def = decl.children().any(|c| c.kind() == SyntaxKind::TYPE_FORM);
```

### 5. `_tag` is reserved and implicit

Tagged-union desugaring (`tagged-unions.md` §17.5):
`(#circle, radius : Float)` → `{ _tag : #circle; radius : Float; }`.

`_tag` is never written by users. The M4 structural-check pass must reject it if it
appears as an explicit binding name, record field name, or `_tag` access.

---

## Pass order (implementation milestones)

Each pass depends on the output of all prior passes. Do not skip ahead.

### M1 — Name resolution

**Goal:** Resolve every `NameRef` to a definition during HIR lowering; lower references
to `HirExprKind::Var(SymbolId)`.  
**Emits:** `E0020 UnknownIdentifier`  
**Spec:** `04-general-mode/file-structure.md` §5.2–5.8

**Algorithm — two phases required:**

Top-level scope is *recursive* (§5.6 — forward refs and mutual recursion are legal).
Block-local `:=` is *sequential*. This is implemented in `zutai-hir` lowering:

1. **Phase 1 — collect** all top-level `decl_name`s from `File::decls()`, calling
   `LowerCtx::define_sym_in(file_scope, ...)` for each. `SymbolKind`s:
   - `InferredBinding` / `AnnotatedBinding` → `Value`
   - `FuncDecl` with a `TYPE_FORM` child → `TypeDef`
   - `FuncDecl` otherwise → `Function`
   `E0010 DuplicateBinding` currently belongs to `zutai-syntax::validation`; lowering does
   not re-emit it.

2. **Phase 2 — lower/resolve** each body:
   - `NameRef` in expression position → `LowerCtx::resolve_name(name, range)`. If found,
     lower to `HirExprKind::Var(SymbolId)`. If not found, emit E0020 and use `ERROR_SYM`.
   - `Block` with local `:=` → push a child scope; lower each RHS before defining that
     local name; lower the remaining block under the extended scope; pop on exit.
   - `LambdaExpr` → push scope, define the parameter, resolve body, pop.
   - `Clause` → push scope, define type-params from `TypeParamList`, then define
     *binding* patterns, resolve guard + body, pop.
   - `MatchCase` → push scope per case, define binding patterns, resolve guard + arm, pop.

**Status:** implemented in `crates/general/hir/src/lower/`. The remaining M1 work is
test coverage and removal/retirement of the stale `semantic::passes::NameResolution` stub.

**Tests still needed:**
- unknown expression identifier emits E0020.
- top-level forward reference/mutual recursion is accepted.
- block-local forward reference emits E0020 because locals are sequential.
- type annotation references to built-ins (`Int`, `Text`, `Bool`, `List`) are accepted.

---

### M2 — Type checking

**Goal:** Verify every expression is well-typed; check closed-record conformance.  
**Emits:** `E0021 UnknownField`, `E0030 TypeMismatch`  
**Spec:** `docs/v0_spec/05-type-system/`, `06-polymorphism/polymorphism.md` §18  
**Prerequisite:** HIR lowering/M1 complete (type checking reads `HirExprKind::Var(SymbolId)`,
`HirDecl`, `HirType`, and `SymbolTable`, not `ctx.resolution`).

**Implementation shape:**
- Update `Pass::run` to receive `(&mut HirFile, &mut AnalysisContext)`.
- Register `TypeCheck` in `pass::default_passes()` after any HIR-only checks that do not
  depend on types.
- Implement `TypeCheck::run` as a thin wrapper around internal HIR helpers such as
  `check_file(hir, ctx)`.
- Elaborate `HirTypeId` into semantic `TyId` with `ctx.types`.
- Write inferred/checked semantic types back into `hir.symbols.get_mut(sym).ty` as `HirTyRef`.
- Skip or propagate `ERROR_SYM`/`HirExprKind::Error` to avoid cascaded diagnostics after E0020.
- Keep source ranges from HIR nodes for E0021/E0030.

**Key rules:**
- Bidirectional: `check(expr, expected_ty)` vs `infer(expr) -> TyId`.
- Annotated binding `HirDecl::Value { ty: Some(T), body, .. }` — elaborate `T`, then
  `check(body, T)`.
- Inferred binding `HirDecl::Value { ty: None, body, .. }` — `infer(body)`; HM
  let-generalise free vars.
- Functions are already lowered to `HirExprKind::Lambda` plus nested `Match` arms; use
  the optional `HirDecl::Function::sig` as the expected function type when present.
- Closed record: extra/missing required fields → E0021/E0030. In type-check position
  against a known record type: every non-optional field must appear; no extra fields.
- `if` condition must be `Bool`; branches must unify.
- Optional chain was lowered to `Match` in HIR; either type-check that desugared form or
  preserve enough metadata later if diagnostics need to mention optional access directly.
- `??` was lowered to `Match` in HIR; same caveat as optional chain.
- **Reject** `[A: Eq]` constrained type params as a v1 feature.

**Grow `semantic/src/ty.rs`:** align semantic `Ty` with HIR's existing shape:
`Int`, `Float`, `Text`, `Bool`, `None`, `Atom(String)`, `Optional(TyId)`,
`List(TyId)`, `Record(...)`, `Variant { tag, fields }`, `Union(Vec<TyId>)`,
`Function { param, ret }`, `Var(u32)`, plus `Unknown/Error` for recovery.

**Fixtures to flip:**
- `semantic_invalid/closed_records.zt` → `invalid/`
- `semantic_invalid/union_membership.zt` → `invalid/`

---

### M3 — Exhaustiveness

**Goal:** Check that `match` expressions and function clause sets cover all cases of a
finite union.  
**Emits:** `E0031 NonExhaustiveMatch`  
**Spec:** `docs/v0_spec/06-polymorphism/pattern-matching.md` §19  
**Prerequisite:** M2 complete (need the scrutinee's type to know what "all cases" are).

**Rules:**
- A union `[#ok; #err; #pending;]` has a finite, known case set.
- A `match` must cover every case. A wildcard `_` or an unconstrained `NameRef` pattern
  counts as a catch-all.
- Guards (`if cond`) do *not* count as coverage — a guarded arm may fall through.
- Function clause sets: same coverage check per union-typed first argument.

**Fixture to flip:** `semantic_invalid/exhaustiveness.zt` → `invalid/`

---

### M4 — Surface structural checks

**Goal:** Reject `_tag` used explicitly by users.  
**Spec:** `docs/v0_spec/06-polymorphism/tagged-unions.md` §17.5

**Checks:**
- `_tag` as a top-level or local binding name → error.
- `_tag` as a value-record field name → error.
- Accessing `._tag` via field access → error.

This is a surface/CST check because the rule is about what the user explicitly wrote. Put
it in `surface_checks.rs` before HIR passes, or in `validation.rs` if it is purely
syntactic. Do not make this a semantic `Pass`; the pass registry is HIR-only.

**Fixture to flip:** `semantic_invalid/reserved_tag.zt` → `invalid/`

---

### M5 — Import resolution

**Goal:** Resolve `import "path"` expressions; detect cycles.  
**Emits:** `E0040 InvalidImportPath`  
**Spec:** `docs/v0_spec/04-general-mode/imports.md` §7

**Rules:**
- Paths are relative to the importing file's directory.
- `.zti` imports return inert data; `.zt` imports evaluate and return the file's output expression.
- Imports are cached: re-importing the same resolved path returns the same value.
- Import cycles that cannot be resolved lazily → error.

**After M5:** add serialization-boundary checks (`docs/v0_spec/07-modules/serialization-boundary.md` §26)
— verify that values crossing the `.zt` → `.zti` boundary are serializable (no functions, no
type values, no unresolved imports).

---

## Testing convention

1. **Smoke tests (already in `tests/acceptance.rs`):** every valid fixture runs through
   `analyze` with zero diagnostics. These must stay green as passes are implemented.

2. **Semantic-error tests:** when a pass is implemented, move its fixture from
   `crates/general/fixtures/semantic_invalid/` to `invalid/`, update
   `crates/general/fixtures/EXPECTATIONS.md`, and flip the `semantic_gap_*` test in
   `tests/acceptance.rs` to use an `assert_has_semantic_error` helper.

3. **New fixtures for new error codes:** e.g. `unknown_identifier.zt` for M1's E0020
   (no existing fixture tests that code).

4. Use `expect-test` (not insta) for any diagnostic snapshot tests, matching the
   `zutai-syntax` crate's convention. `UPDATE_EXPECT=1 cargo test` to regenerate.

---

## Spine API recap (for cold-start sessions)

```rust
// Entry point
pub fn analyze(root: &SyntaxNode) -> AnalysisResult

// AnalysisResult
pub struct AnalysisResult {
    pub diagnostics: Vec<Diagnostic>,   // zutai_syntax::diag::Diagnostic
    pub resolution: ResolutionMap,
    pub hir: HirFile,
}

// HIR-only Pass trait
pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, hir: &mut HirFile, ctx: &mut AnalysisContext);
}

// Context helpers
ctx.error(range, ErrorCode::UnknownIdentifier, "message");
ctx.warning(range, ErrorCode::CapitalizationConvention, "message");
ctx.error_with_label(range, code, msg, label_range, label_msg);

// HIR name references
HirExprKind::Var(SymbolId);
hir.symbols.get(sym_id) -> &Symbol;
hir.symbols.get_mut(sym_id) -> &mut Symbol;

// Literal classification (ast_ext)
classify_literal(&SyntaxNode) -> Option<LitClass>
  // LitClass: NameRef | Wildcard | Int | Float | Str | Atom | Bool | NoneLit
```

---

## Milestones

- [x] **M0 Scaffold** — crate, Pass trait, AnalysisContext, ScopeStack, ResolutionMap, TyInterner stub, ast_ext classifier, stubbed passes, CLI wiring, smoke tests.
- [x] **M1 Name resolution implementation** — implemented during HIR lowering; two-phase top-level collect + sequential locals; E0020.
- [ ] **M1 test/API cleanup pass** — add unknown_identifier/forward-reference tests; update `Pass::run` to HIR-only; remove or retire the stale CST `NameResolution` pass.
- [ ] **M2 Type checking pass** — bidirectional + HM let-gen; Ty variants; closed-record/union checks; E0021/E0030; flip closed_records + union_membership.
- [ ] **M3 Exhaustiveness pass** — finite-union coverage; guard fall-through; E0031; flip exhaustiveness.
- [ ] **M4 Surface structural checks** — `_tag` reserved check outside the HIR pass registry; flip reserved_tag.
- [ ] **M5 Imports + serialization boundary** — E0040; path resolution; cycle detection.
