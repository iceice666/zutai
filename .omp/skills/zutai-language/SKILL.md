---
name: zutai-language
description: Answer Zutai language questions by quickly routing to the current manual, stable specification, implementation status, open roadmap, and relevant compiler docs. Use when the user asks about Zutai syntax, semantics, examples, support levels, compiler stages, parser/typechecker behavior, or where a language fact is documented.
---

# Zutai Language Specialist

Use this skill for Zutai language facts: syntax, examples, semantics, support levels, compiler-stage behavior, parser/typechecker behavior, and source-of-truth lookup.

## Fast source map

Read the smallest relevant set; do not answer from memory when a file can ground the claim.

1. `docs/language-manual.md` — first stop for user-facing current behavior, examples, implemented extensions, and common diagnostics.
2. `docs/spec/00-index.md` — normative stable-language index. Follow its chapter links for syntax and semantics.
3. `docs/spec/02-lexical/grammar-reference.md` — compact implemented general-mode grammar; parser source remains `crates/general/syntax/src/parser/` when code behavior must be verified.
4. `docs/stdlib/00-index.md` — module-by-module standard-library surface with per-module support levels; follow its links to the module page (`stream.md`, `list.md`, `data.md`, `validate.md`, `config.md`, `reflect.md`, `result.md`, `collection.md`, `prelude.md`, `fs.md`, `net.md`, `capabilities.md`, `html.md`, `css.md`) before describing any stdlib behavior.
5. `docs/project/status.md` — current implementation baseline, validation notes, and precise backend/support caveats.
6. `docs/project/roadmap.md` — open implementation order and unresolved work; top-to-bottom order is authoritative for roadmap questions.
7. `docs/project/decisions.md` — closed stabilization decisions; check here before calling a question open.
8. `docs/history/README.md` and the half-year files (`docs/history/2026-h1.md`, `docs/history/2026-h2.md`) — completed milestones, newest first; the fastest way to date a landed capability.
9. `docs/design/reserved-language-boundaries.md` — explicit non-goals and
   demand-gated future design boundaries; these are not speculative versions.
10. `docs/README.md` — compiler pipeline and crate responsibility map.
11. `docs/compiler/tlc.md`, `docs/compiler/dataflow-core.md`, `docs/compiler/anf.md`, and `docs/compiler/runtime-abi.md` — IR, lowering, scheduling, and runtime ABI details for compiler-stage questions.

## Answer workflow

1. Classify the question:
   - Syntax/example question → read `docs/language-manual.md`, then the relevant `docs/spec/` chapter or `grammar-reference.md`.
   - Implementation/support question → read `docs/language-manual.md` implemented-extension table, then `docs/project/status.md` current baseline/validation notes, then `docs/project/roadmap.md` if the feature is incomplete.
   - Compiler-stage question → read `docs/README.md` pipeline, then `docs/compiler/tlc.md` / `docs/compiler/dataflow-core.md` / `docs/compiler/anf.md` / `docs/compiler/runtime-abi.md` or the relevant crate source if exact behavior is needed.
   - Standard-library question → read `docs/stdlib/00-index.md`, then the specific module page. Never describe a stdlib function, alias, or support level from memory.
   - Reserved-boundary question → read
     `docs/design/reserved-language-boundaries.md`, then cross-check
     `docs/language-manual.md` and `docs/project/status.md` before claiming support.
2. State support levels using this vocabulary when applicable: `syntax only`, `check-only`, `reference-interpreter support`, `backend rejection`, `LLVM/native support`, or `unimplemented/open`.
3. Cite exact paths and sections/line ranges for load-bearing claims.
4. For implementation work, read the relevant stable spec page before editing parser, AST, HIR, THIR, TLC, evaluator, or backend code. Then inspect the crate named by `docs/README.md` for the pipeline layer.
5. Prefer existing syntax and examples from the manual/spec. Do not invent syntax, precedence, support level, fallback policy, or standard-library behavior.
6. When editing `docs/language-manual.md` or any `docs/spec/` page, remember that `crates/general/syntax/tests/format_corpus.rs` extracts every parseable ```zt / ```zti fence from those files and asserts it formats idempotently. A new or edited fence must parse and already be in canonical form, or that test fails.

## High-value facts to check first

- `.zti` immediate mode is inert data: no imports, functions, conditionals, arithmetic, name resolution, type computation, or evaluation. See `docs/language-manual.md` file modes and `docs/spec/01-overview/file-modes.md`.
- `.zt` general mode is pure, lazy, typed computation over data; a file contains zero or more declarations followed by one final expression.
- Top-level declarations use `::=` for inferred values, `:: Type =` for typed values, function signatures followed by `= pattern => body;` clauses, and `Name :: type TypeExpr` for type aliases.
- Function application is whitespace and left-associative; functions are curried; lambdas use `\x. body` with required whitespace after the dot.
- Selective projection accepts keyword and postfix forms: `select value { field; }` / `value >>= { field; }` and `select TypeValue { field; }` / `TypeValue >>= { field; }`.
- `cond { ... }` is the canonical expression conditional; it desugars to the core `if`/`else` form, the older `if/then/else` spelling is still accepted, and `then` is still a keyword (which is why `stdlib.cmp` backs its exported `then` with an internal `thenCmp` binding).
- Field-pun shorthand `{ field =; }` means `{ field = field; }` and works in both record values and record patterns. Field sections `_.field` / `_?.field` are the terse projection form used in list/stream folds.
- A tagged value may pin its union type with `as`: `#move { x = 1; y = 2; } as Action`. The `as` keyword is scoped to tagged-value contexts, so `as` is still usable as an ordinary identifier elsewhere.
- Algebraic effects accept keyword and punctuation forms for operations: `perform op arg` / `! op arg` and `resume value` / `^ value`; handlers still use `handle expr with { ... }`.
- Stable behavior lives under `docs/spec/`; implementation support is stated
  per feature and is never inferred from a language-version label.
- THIR is source-preserving and error-tolerant; TLC is produced only after successful type checking and is the clean input to Dataflow Core and later backend stages.

## Common traps

- Do not treat `.zti` as executable or import-capable.
- Do not use `type Server =`, `def`, or `class`; Zutai declarations use the documented forms.
- Do not claim tagged union payload values serialize directly to `.zti`; JSON rendering via `eval_path_to_json` uses the documented atom/envelope shapes.
- Do not say the interpreter is more permissive than the backend about program *results*. `zutai-cli run` and `zutai-cli json` refuse a final expression that is not first-order serializable data (function, runtime `Type`, constraint witness, opaque host handle, including nested), reporting the same runtime-ABI reason `zutai-cli compile` reports. Only the interactive REPL stays permissive.
- Do not answer CLI-surface questions from memory. The authoritative list is the `Commands` enum in `crates/cli/src/main.rs`: `run`, `parse`, `json`, `format` (`--check`), `check`, `compile` (`--emit llvm|obj|bin|lib`, `--target`, `--metadata`), `dataflow`, `web`, `package`, `repl`, `lsp`, and `model-check` (`--max-states`).
- Do not overlook the bounded explicit-state model checker: `zutai-cli model-check` over a pure `.zt` transition system, implemented in `crates/general/model/` (`zutai-model`). It adds no language syntax, no SMT solving, and no refinement types; an exhausted state budget is inconclusive, not safe. See the manual's "Model checking" section.
- Do not claim full native-backend support unless the current manual/archive says so.
- Do not bypass `docs/spec/` when changing language syntax or semantics.
