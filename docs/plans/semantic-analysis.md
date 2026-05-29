# Semantic analysis (`zutai-semantic`) — implementation guide

## Context

`crates/general/semantic/` (package `zutai-semantic`) is the semantic analysis layer for
Zutai general mode (`.zt`). It sits downstream of `zutai-syntax`: feed it the `SyntaxNode`
returned by `zutai_syntax::parse(src).syntax()`, get back an `AnalysisResult` with
diagnostics and a resolution map.

The CLI (`crates/cli/src/main.rs`) already calls `zutai_semantic::analyze` after `parse`
and merges the diagnostic vecs before rendering. Stub passes produce no output today; as
you implement each pass the CLI gains real errors.

Source of truth for every semantic rule: `docs/v0_spec/` — especially
`04-general-mode/`, `05-type-system/`, `06-polymorphism/`, and
`08-reference/error-model.md` §28 (the authoritative error-code list).

---

## Crate map

```
crates/general/semantic/src/
  lib.rs                 pub fn analyze(&SyntaxNode) -> AnalysisResult
  pass.rs                Pass trait + default_passes() registry
  context.rs             AnalysisContext { scopes, resolution, types, diagnostics }
                           + ctx.error / ctx.warning / ctx.error_with_label
  scope.rs               ScopeId, Scope, ScopeStack (arena), Symbol, SymbolKind
  resolution.rs          ResolutionMap (use-site TextRange -> def-site TextRange)
  ty.rs                  TyId, Ty { Unknown /* grow here */ }, TyInterner
  ast_ext.rs             LitClass + classify_literal() — the #1 CST-gap bridge
  passes/
    name_resolution.rs   NameResolution: Pass  (M1)
    type_check.rs        TypeCheck: Pass        (M2)
  tests/
    acceptance.rs        smoke tests over all valid + semantic_invalid fixtures
```

**Every pass** lives in `passes/`, implements `Pass`, and is registered in
`pass::default_passes()`. Passes run in order and share one `AnalysisContext`.

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

These are non-obvious mappings between the CST and semantic intent. Every pass is
affected by at least one of them.

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

`ast_ext::classify_literal` is already implemented — use it everywhere.

### 2. Type position is not in the tree

`List Int` is a `CALL_EXPR`; `(A, B)` in a type annotation is a `TUPLE_EXPR`. The
parser's internal `Ctx::Type` is not persisted. Reconstruct position from the parent:
type child of `ANNOTATED_BINDING`, `TYPE_FIELD`, `VARIANT_FIELD`, or the RHS of
`FUNCTION_TYPE` is in type position. Everything else is expression position.

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

**Goal:** Resolve every `NameRef` to a definition; populate `ctx.resolution`.  
**Emits:** `E0020 UnknownIdentifier`  
**Spec:** `04-general-mode/file-structure.md` §5.2–5.8

**Algorithm — two phases required:**

Top-level scope is *recursive* (§5.6 — forward refs and mutual recursion are legal).
Block-local `:=` is *sequential*. This means:

1. **Phase 1 — collect** all top-level `decl_name`s from `File::decls()`, calling
   `ctx.scopes.define(Symbol { ... })` for each. SymbolKinds:
   - `InferredBinding` / `AnnotatedBinding` → `Value`
   - `FuncDecl` with a `TYPE_FORM` child → `TypeDef`
   - `FuncDecl` otherwise → `Function`
   Emit `E0010 DuplicateBinding` with a secondary label on collision. (Decide whether to
   own E0010 here and remove it from `validation.rs`, or let both coexist.)

2. **Phase 2 — resolve** each body, walking the tree recursively:
   - `NameRef` in expression position → `ctx.scopes.resolve(name)`. If found, insert into
     `ctx.resolution`. If not found, emit `E0020`.
   - `Block` with local `:=` → `ctx.scopes.push_child()`; define locals *sequentially*
     (each local only sees prior locals + the outer scope); `ctx.scopes.pop()` on exit.
   - `LambdaExpr` → push scope, define the parameter, resolve body, pop.
   - `Clause` → push scope, define type-params from `TypeParamList`, then define
     *binding* patterns, resolve guard + body, pop.
   - `MatchCase` → push scope per case, define binding patterns, resolve guard + arm, pop.

**New fixture needed:** `crates/general/fixtures/semantic_invalid/unknown_identifier.zt`
(none of the existing semantic_invalid fixtures test E0020).

---

### M2 — Type checking

**Goal:** Verify every expression is well-typed; check closed-record conformance.  
**Emits:** `E0021 UnknownField`, `E0030 TypeMismatch`  
**Spec:** `docs/v0_spec/05-type-system/`, `06-polymorphism/polymorphism.md` §18  
**Prerequisite:** M1 complete (type checking reads `ctx.resolution`).

**Key rules:**
- Bidirectional: `check(expr, expected_ty)` vs `infer(expr) -> TyId`.
- Annotated binding `x : T = e` — elaborate `T`, then `check(e, T)`.
- Inferred binding `x := e` — `infer(e)`; HM let-generalise free vars.
- Closed record: extra/missing required fields → E0021/E0030. In type-check position
  against a known record type: every non-optional field must appear; no extra fields.
- `if` condition must be `Bool`; branches must unify.
- Optional chain typing: if `x : T?`, `x?.f : U?` where `T.f : U`; `(T?)?` flattens to `T?`.
- **Reject** `[A: Eq]` constrained type params as a v1 feature.

**Grow `ty.rs`:** add `Int`, `Float`, `Text`, `Bool`, `Atom(String)`, `Optional(TyId)`,
`List(TyId)`, `Record(...)`, `Union(Vec<TyId>)`, `Function { param, ret }`, `Var(u32)`.

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

### M4 — Structural lints

**Goal:** Reject `_tag` used explicitly by users.  
**Spec:** `docs/v0_spec/06-polymorphism/tagged-unions.md` §17.5

**Checks:**
- `_tag` as a top-level or local binding name → error.
- `_tag` as a value-record field name → error.
- Accessing `._tag` via field access → error.

This is a straightforward tree walk; it can go in `validation.rs` as a new lint function
or as its own `Pass` — your choice.

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
}

// Pass trait
pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&self, root: &SyntaxNode, ctx: &mut AnalysisContext);
}

// Context helpers
ctx.error(range, ErrorCode::UnknownIdentifier, "message");
ctx.warning(range, ErrorCode::CapitalizationConvention, "message");
ctx.error_with_label(range, code, msg, label_range, label_msg);

// Scope ops
ctx.scopes.push_child() -> ScopeId;
ctx.scopes.pop();
ctx.scopes.define(Symbol { name, kind, def, ty }) -> Result<(), TextRange>;
ctx.scopes.resolve("name") -> Option<&Symbol>;   // walks parent chain

// Resolution
ctx.resolution.insert(use_site: TextRange, def_site: TextRange);
ctx.resolution.get(use_site: TextRange) -> Option<TextRange>;

// Literal classification (ast_ext)
classify_literal(&SyntaxNode) -> Option<LitClass>
  // LitClass: NameRef | Wildcard | Int | Float | Str | Atom | Bool | NoneLit
```

---

## Milestones

- [x] **M0 Scaffold** — crate, Pass trait, AnalysisContext, ScopeStack, ResolutionMap, TyInterner stub, ast_ext classifier, stubbed passes, CLI wiring, smoke tests.
- [ ] **M1 Name resolution** — two-phase collect + resolve; E0020; new unknown_identifier fixture.
- [ ] **M2 Type checking** — bidirectional + HM let-gen; Ty variants; closed-record/union checks; E0021/E0030; flip closed_records + union_membership.
- [ ] **M3 Exhaustiveness** — finite-union coverage; guard fall-through; E0031; flip exhaustiveness.
- [ ] **M4 Structural lints** — `_tag` reserved check; flip reserved_tag.
- [ ] **M5 Imports + serialization boundary** — E0040; path resolution; cycle detection.
