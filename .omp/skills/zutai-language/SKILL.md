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
4. `docs/project/status.md` — current implementation baseline, validation notes, and precise backend/support caveats.
5. `docs/project/roadmap.md` — open implementation order and unresolved work; top-to-bottom order is authoritative for roadmap questions.
6. `docs/design/reserved-language-boundaries.md` — explicit non-goals and
   demand-gated future design boundaries; these are not speculative versions.
7. `docs/README.md` — compiler pipeline and crate responsibility map.
8. `docs/compiler/tlc.md`, `docs/compiler/dataflow-core.md`, `docs/compiler/anf.md`, and `docs/compiler/runtime-abi.md` — IR, lowering, scheduling, and runtime ABI details for compiler-stage questions.

## Answer workflow

1. Classify the question:
   - Syntax/example question → read `docs/language-manual.md`, then the relevant `docs/spec/` chapter or `grammar-reference.md`.
   - Implementation/support question → read `docs/language-manual.md` implemented-extension table, then `docs/project/status.md` current baseline/validation notes, then `docs/project/roadmap.md` if the feature is incomplete.
   - Compiler-stage question → read `docs/README.md` pipeline, then `docs/compiler/tlc.md` / `docs/compiler/dataflow-core.md` / `docs/compiler/anf.md` / `docs/compiler/runtime-abi.md` or the relevant crate source if exact behavior is needed.
   - Reserved-boundary question → read
     `docs/design/reserved-language-boundaries.md`, then cross-check
     `docs/language-manual.md` and `docs/project/status.md` before claiming support.
2. State support levels using this vocabulary when applicable: `syntax only`, `check-only`, `reference-interpreter support`, `backend rejection`, `LLVM/native support`, or `unimplemented/open`.
3. Cite exact paths and sections/line ranges for load-bearing claims.
4. For implementation work, read the relevant stable spec page before editing parser, AST, HIR, THIR, TLC, evaluator, or backend code. Then inspect the crate named by `docs/README.md` for the pipeline layer.
5. Prefer existing syntax and examples from the manual/spec. Do not invent syntax, precedence, support level, fallback policy, or standard-library behavior.

## High-value facts to check first

- `.zti` immediate mode is inert data: no imports, functions, conditionals, arithmetic, name resolution, type computation, or evaluation. See `docs/language-manual.md` file modes and `docs/spec/01-overview/file-modes.md`.
- `.zt` general mode is pure, lazy, typed computation over data; a file contains zero or more declarations followed by one final expression.
- Top-level declarations use `::=` for inferred values, `:: Type =` for typed values, function signatures followed by `= pattern => body;` clauses, and `Name :: type TypeExpr` for type aliases.
- Function application is whitespace and left-associative; functions are curried; lambdas use `\x. body` with required whitespace after the dot.
- Selective projection accepts keyword and postfix forms: `select value { field; }` / `value >>= { field; }` and `select TypeValue { field; }` / `TypeValue >>= { field; }`.
- Algebraic effects accept keyword and punctuation forms for operations: `perform op arg` / `! op arg` and `resume value` / `^ value`; handlers still use `handle expr with { ... }`.
- Stable behavior lives under `docs/spec/`; implementation support is stated
  per feature and is never inferred from a language-version label.
- THIR is source-preserving and error-tolerant; TLC is produced only after successful type checking and is the clean input to Dataflow Core and later backend stages.

## Common traps

- Do not treat `.zti` as executable or import-capable.
- Do not use `type Server =`, `def`, or `class`; Zutai declarations use the documented forms.
- Do not claim tagged union payload values serialize directly to `.zti`; JSON rendering via `eval_path_to_json` uses the documented atom/envelope shapes.
- Do not claim full native-backend support unless the current manual/archive says so.
- Do not bypass `docs/spec/` when changing language syntax or semantics.
