# Archived Decisions

These closed stabilization items stay here so old risk decisions remain visible.
New unresolved work should become an open milestone in the [roadmap](roadmap.md).

- [x] **Compiler entry-type gate cleanup** — CLI `compile` and `dataflow`
  reject final runtime `Type` values before TLC→DC/LLVM lowering, including raw
  `type Int` entries and alias-value entries such as `MyInt :: type Int; MyInt`.
- [x] **v0 spec conformance sweep** — code fences from `docs/spec/` are
  extracted and routed through `check`/`run` for `.zt` survivors and the immediate
  parser for `.zti` survivors; stable survivors are promoted to acceptance tests.
- [x] **Diagnostic polish** — record-vs-record type mismatches render source-like
  record shapes, including optional fields and row tails; row-tail spread
  overlaps report the spread source and existing/incoming shapes.
- [x] **TLC-first evaluator cutover** — default evaluation runs through TLC for
  executable value programs; THIR remains the explicit regression oracle and
  runtime `Type`/reflection boundary.


Older milestone-specific decisions and superseded implementation choices remain
in the dated [implementation history](../history/README.md).
