# Zutai v0 Implementation Roadmap

This roadmap tracks the path from the current parser/HIR/THIR workspace to a complete v0 implementation with an interpreter and REPL. The v0 spec under `docs/v0_spec/` remains the source of truth; this document is an implementation plan, not a language-design override.

## Current Baseline

- Immediate mode parses `.zti` data through selectable parser backends.
- General mode parses `.zt`, lowers to HIR, and partially lowers/checks THIR.
- THIR currently supports scalar literals, records, tuples, lists, closed record checking, field access, optional-field defaulting, `if`, scalar binary operators, type aliases, block locals, and explicitly typed monomorphic function declarations/application.
- The CLI currently analyzes files and prints ASTs; it does not execute `.zt` output or provide a REPL.

## Phase 1: Complete v0 Frontend

Goal: every v0 syntax form should parse, lower through HIR, and either complete THIR or produce a source-located v0 diagnostic.

- Parser
  - Keep declaration disambiguation aligned with `docs/v0_spec/02-lexical/grammar-sketch.md`.
  - Cover type application such as `List Int` and `Optional T`.
  - Preserve existing rejection of ambiguous mixed pipelines and chained comparisons.
- HIR
  - Keep name resolution, top-level namespace rules, local scopes, and syntax-only desugarings here.
  - Add module/import name resolution once module loading is introduced.
- THIR
  - Finish optional access lowering (`?.`) and defaulting flattening rules.
  - Add tuple and record pattern checking.
  - Add `match` exhaustiveness and unreachable-arm diagnostics.
  - Add no-signature function inference.
  - Add lambdas and closure types.
  - Add predicative polymorphism and call-site instantiation.
  - Add type-level expression normalization with deterministic limits.
  - Add import typing for `.zti` and `.zt` modules.

Verification gate: `cargo test --workspace` includes spec-shaped parser, HIR, THIR, and semantic facade tests for every v0 chapter.

## Phase 2: Runtime Value Model

Goal: define a stable interpreter-facing value representation without exposing parser AST internals.

- Add a runtime crate, likely `crates/general/runtime`, depending on `zutai-thir` and immediate-mode parsing.
- Represent v0 values: bool, int, float, text, atom, record, tuple, list, function closure, type value, and an internal error marker.
- Preserve laziness with thunks for top-level bindings, local bindings, function arguments, branches, and imports.
- Define structural equality for comparable v0 values.
- Define serialization eligibility: records, lists, scalars, atoms, and booleans are serializable; tuple and type values are `.zt` runtime values but not direct `.zti` outputs.

Verification gate: unit tests evaluate THIR snippets into runtime values and assert laziness for unused branches/bindings.

## Phase 3: Interpreter

Goal: evaluate complete THIR according to v0 purity and laziness rules.

- Evaluate declarations into a module environment with recursive top-level thunks.
- Evaluate final file expression as the module output.
- Implement scalar operators, equality, field access, optional access, `??`, `if`, blocks, function application, lambdas, and pattern matching.
- Implement imports:
  - `.zti` import parses inert data and converts blocks/arrays into `.zt` records/lists.
  - `.zt` import analyzes and evaluates the imported module final expression.
- Enforce deterministic limits for evaluation, recursion, import cycles, and type-level normalization.

Verification gate: semantic analysis plus interpreter tests cover `docs/v0_spec/08-reference/complete-example.md` and targeted error cases from `docs/v0_spec/08-reference/error-model.md`.

## Phase 4: CLI Execution

Goal: make `zutai-cli` useful for both inspection and execution.

- Replace the single positional mode with subcommands:
  - `parse <path>` prints AST or parse diagnostics.
  - `check <path>` runs parse, HIR, THIR, and semantic diagnostics.
  - `run <path>` evaluates the final expression.
  - `repl` starts interactive general mode.
- Add output rendering for runtime values and `.zti`-serializable output.
- Keep diagnostics source-located through the semantic facade.

Verification gate: CLI integration tests cover successful `.zt` run, `.zti` parse, parse errors, semantic errors, and runtime errors.

## Phase 5: REPL

Goal: provide an interactive general-mode session backed by the same semantic and runtime pipeline.

- Maintain session state as accumulated declarations plus the current expression.
- Support evaluating expressions, adding declarations, inspecting types, and resetting state.
- Use the same parser, HIR, THIR, and interpreter as file execution.
- Print diagnostics without corrupting the session environment when a line fails.

Verification gate: REPL tests drive scripted input/output for declaration persistence, expression evaluation, diagnostics, and reset behavior.

## Near-Term Implementation Order

1. Finish THIR for remaining non-polymorphic v0 expression forms: optional access, tuple/record patterns, and `match`.
2. Add the runtime crate with values, thunks, environments, and scalar/data evaluation.
3. Route `zutai-cli run` through `zutai_semantic::analyze` and the runtime crate.
4. Add imports and module environments.
5. Add lambdas, no-signature inference, and polymorphism.
6. Add the REPL once file execution uses the final runtime path.
