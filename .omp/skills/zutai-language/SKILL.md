---
name: zutai-language
description: Answer Zutai language questions by quickly routing to the current manual, v0 normative specification, implementation status, open roadmap, and relevant compiler docs. Use when the user asks about Zutai syntax, semantics, examples, support levels, compiler stages, parser/typechecker behavior, or where a language fact is documented.
---

# Zutai Language Specialist

Use this skill for Zutai language facts: syntax, examples, semantics, support levels, compiler-stage behavior, parser/typechecker behavior, and source-of-truth lookup.

## Fast source map

Read the smallest relevant set; do not answer from memory when a file can ground the claim.

1. `docs/language-manual.md` — first stop for user-facing current behavior, examples, implemented extensions, and common diagnostics.
2. `docs/spec/v0/00-index.md` — normative v0 index. Follow its chapter links for stable syntax and semantics.
3. `docs/spec/v0/02-lexical/grammar-reference.md` — compact implemented general-mode grammar; parser source remains `crates/general/syntax/src/parser/` when code behavior must be verified.
4. `docs/ARCHIVED.md` — current implementation baseline, validation notes, completed milestones, and precise backend/support caveats.
5. `docs/TBD.md` — open implementation order and unresolved work; top-to-bottom order is authoritative for roadmap questions.
6. `docs/spec/v1/00-index.md` — deferred v1 feature index. Use v1 pages only for features explicitly marked implemented or v1-adjacent in the manual/archive.
7. `docs/README.md` — compiler pipeline and crate responsibility map.
8. `docs/tlc-core.md`, `docs/dataflow-core.md`, `docs/anf.md`, and `docs/runtime-abi.md` — IR, lowering, scheduling, and runtime ABI details for compiler-stage questions.

## Answer workflow

1. Classify the question:
   - Syntax/example question → read `docs/language-manual.md`, then the relevant `docs/spec/v0/` chapter or `grammar-reference.md`.
   - Implementation/support question → read `docs/language-manual.md` implemented-extension table, then `docs/ARCHIVED.md` current baseline/validation notes, then `docs/TBD.md` if the feature is incomplete.
   - Compiler-stage question → read `docs/README.md` pipeline, then `docs/tlc-core.md` / `docs/dataflow-core.md` / `docs/anf.md` / `docs/runtime-abi.md` or the relevant crate source if exact behavior is needed.
   - Deferred/v1 question → read `docs/spec/v1/00-index.md` and the specific v1 page, then cross-check `docs/language-manual.md` and `docs/ARCHIVED.md` before claiming implementation support.
2. State support levels using this vocabulary when applicable: `syntax only`, `check-only`, `reference-interpreter support`, `backend rejection`, `LLVM/native support`, or `unimplemented/open`.
3. Cite exact paths and sections/line ranges for load-bearing claims.
4. For implementation work, read the relevant v0 spec page before editing parser, AST, HIR, THIR, TLC, evaluator, or backend code. Then inspect the crate named by `docs/README.md` for the pipeline layer.
5. Prefer existing syntax and examples from the manual/spec. Do not invent syntax, precedence, support level, fallback policy, or standard-library behavior.

## High-value facts to check first

- `.zti` immediate mode is inert data: no imports, functions, conditionals, arithmetic, name resolution, type computation, or evaluation. See `docs/language-manual.md` file modes and `docs/spec/v0/01-overview/file-modes.md`.
- `.zt` general mode is pure, lazy, typed computation over data; a file contains zero or more declarations followed by one final expression.
- Top-level declarations use `::=` for inferred values, `:: Type =` for typed values, function signatures followed by `= pattern => body;` clauses, and `Name :: type TypeExpr` for type aliases.
- Function application is whitespace and left-associative; functions are curried; lambdas use `\x. body` with required whitespace after the dot.
- Selective projection accepts keyword and postfix forms: `select value { field; }` / `value >>= { field; }` and `select TypeValue { field; }` / `TypeValue >>= { field; }`.
- Algebraic effects accept keyword and punctuation forms for operations: `perform op arg` / `! op arg` and `resume value` / `^ value`; handlers still use `handle expr with { ... }`.
- v0 stable behavior lives under `docs/spec/v0/`; v1 pages are deferred unless the manual/archive says a v1-adjacent feature is implemented.
- THIR is source-preserving and error-tolerant; TLC is produced only after successful type checking and is the clean input to Dataflow Core and later backend stages.

## Common traps

- Do not treat `.zti` as executable or import-capable.
- Do not use `type Server =`, `def`, or `class`; Zutai declarations use the documented forms.
- Do not claim tagged union payload values serialize directly to `.zti`; JSON rendering via `eval_path_to_json` uses the documented atom/envelope shapes.
- Do not claim full native-backend support for v1-adjacent features unless the current manual/archive says so.
- Do not bypass `docs/spec/v0/` when changing language syntax or semantics.
