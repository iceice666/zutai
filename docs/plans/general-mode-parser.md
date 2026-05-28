# General-mode parser (`zutai-syntax`) — lossless CST with recovery

## Context

`crates/general/syntax/` (package `zutai-syntax`) is a bare stub: empty `[dependencies]`, a
doc-comment-only `lib.rs`, no lexer/AST/parser/tests. We need a parser for general mode (`.zt`)
that delivers **rich error reporting** and **error recovery** as first-class goals (not bolt-ons).

The grammar is substantially harder than the already-implemented immediate mode (`.zti`): a
9-level operator-precedence table with non-associative comparisons and a `|>`/`<|`-mixing ban,
function application by juxtaposition, contextual `TypeExpr` vs `Expr` (same brackets mean
different things), `::` clause families, and hyphen-in-field-names. The immediate-mode parsers
(winnow + a SIMD recursive-descent) bail on first error, track no spans, and have no shared
diagnostics — so there is little to reuse beyond scalar helpers.

**Acceptance target:** `crates/general/fixtures/cursed.zt` (206 lines, exercises ~every
construct) parses with **zero diagnostics** and round-trips losslessly.

### Decided approach (confirmed with user)

- **Parser engine:** hand-written recursive descent + Pratt expression core. No generator/winnow/chumsky.
- **Tree model:** lossless CST, rust-analyzer / `rowan`-style. Event-based parser that *never
  fails* and always emits a complete tree with `ERROR` nodes. Green tree (immutable, deduped),
  red tree (`SyntaxNode`) cursors, typed-AST accessor layer on top. All tokens incl. trivia preserved.
- **Diagnostics:** rustc-style `file:line:col`, rendered with `ariadne`, collected in a sink
  (collect-many, never bail). Spec format in `docs/v0_spec/08-reference/error-model.md`.

Source of truth: `docs/v0_spec/` — esp. `02-lexical/{grammar-sketch,operator-precedence,conventions}.md`,
`04-general-mode/*`, `05-type-system/*`, `06-polymorphism/*`, `08-reference/error-model.md`.

## Architecture (one paragraph)

**Lexer** → `Vec<Token>` (kind + byte length, trivia included), never fails. A **token-source**
view skips trivia and offers `nth(k)` lookahead. The **parser** is hand-written RD + Pratt that
emits a flat **event** stream (`Start`/`Token`/`Finish`/`Error`) via markers and never returns
`Result`. A **tree builder** replays events, re-attaching trivia, into a `rowan` green tree. The
**red tree** gives cursors; a **typed-AST** layer wraps nodes with accessors. **Diagnostics** are
plain data (`range`, `severity`, `code`, `message`, `labels`) decoupled from rendering; an
`ariadne`-backed renderer (behind a feature) produces the spec's output.

## Crate & module layout

Keep everything in the existing `zutai-syntax` crate (do not split speculative crates now).
Diagnostics live in a `diag` module with `ariadne` behind a `render` cargo feature; extract to a
shared crate later if/when a type-checker needs it.

```
crates/general/syntax/src/
  lib.rs            parse(&str) -> Parse { green, diagnostics }; re-export SyntaxNode/Kind/ast
  syntax_kind.rs    SyntaxKind enum (tokens + nodes), rowan::Language impl, T! macro
  lexer/
    mod.rs          &str -> Vec<Token>, maximal-munch dispatch, trivia, keywords, ERROR tokens
    cursor.rs       byte cursor (peek/bump/eat_while)
    classify.rs     char predicates (is_ident_*, is_atom_*), keyword lookup
    scalars.rs      string + number scanning (ported from immediate mode)
  token_set.rs      TokenSet (bitset over SyntaxKind) for first-sets / recovery sets
  parser/
    mod.rs          Parser, Marker/CompletedMarker (precede), bump/at/expect/err_recover, step guard
    input.rs        Tokens: trivia-skipping view, nth(k), raw-adjacency query
    event.rs        Event enum + process(events) -> green tree (trivia reattachment)
    grammar/
      mod.rs        file() entry + dispatch
      primary.rs    shared primary parser (ctx: Expr | Type) — hard case A
      exprs.rs      9-level Pratt loop, juxtaposition application, records/lists/tuples/lambda/match/if/import
      types.rs      parse_type via shared Pratt under ctx=Type; record/union/variant/optional/fn types
      patterns.rs   Pattern / TuplePattern / RecordPattern
      decls.rs      4 TopDecl forms, Clause (+ guard), Block — hard case C
      fieldname.rs  hyphenated FieldName reassembly — hard case D
  ast/
    mod.rs          AstNode trait + support::child/children/token helpers
    nodes.rs        typed node newtypes (File, ValueBinding, FuncDecl, Clause, Lambda, …)
    tokens.rs       typed token wrappers (int/string/atom/field-name decoding)
    operators.rs    BinaryOp etc. mapped from SyntaxKind
  diag/
    mod.rs          Diagnostic data model + DiagnosticSink (rendering-agnostic)
    render.rs       ariadne renderer (behind `render` feature)
  validation.rs     post-parse lints: capitalization convention, reserved-name use, dup binding/field
  tests/            integration + expect-test snapshots
```

### Dependencies (add to `crates/general/syntax/Cargo.toml`)

- `rowan = "0.16"` — green/red lossless tree (required by decision).
- `text-size = "1.1"` — `TextRange`/`TextSize` for spans (rowan re-exports; depend directly so `diag` spans are version-independent).
- `rustc-hash = "2.0.0"` — already in workspace; `FxHashMap`/`FxHashSet` for keyword table + dup detection.
- `ariadne = "0.5"` — diagnostic renderer, behind `[features] render = ["dep:ariadne"]`.
- `drop_bomb = "0.1"` (optional) — enforce every `Marker` is completed/abandoned; or hand-roll a `bool` flag.
- dev: `expect-test = "1.5"` — CST + diagnostic snapshot tests (`UPDATE_EXPECT=1` to regenerate).

Reuse from `crates/immediate/simd/src/{charclass,parser}.rs`: `is_name_start`/`is_name_continue`,
JSON string/escape/surrogate scanning, int/float/exponent number scanning. Port (copy + adapt to
emit kinds), don't add a cross-crate dep on immediate internals.

## Key design points

**`SyntaxKind`** — one `#[repr(u16)]` enum holding token kinds *and* node kinds (rowan stores one
`u16` per element). Trivia (`WHITESPACE`, `COMMENT`) are real leaf tokens → fully lossless. Standard
`rowan::Language` newtype + `transmute` impl; define `SyntaxNode/SyntaxToken/SyntaxElement` aliases
and a `T!` macro (`T![::]`, `T![:=]`, `T!['{']`, …).

**Lexer maximal-munch** (hard case F) — longest-match: `:`/`:=`/`::`, `?`/`?.`/`??`,
`=`/`==`/`=>`, `<`/`<=`/`<|`, `>`/`>=`, `|>`, `-`/`->`, `.`/`...`(reserved). `-` is *always* its own
`MINUS` token; `!` only as `!=`, `|` only as `|>`. Keywords re-tagged after scanning an ident
(`type match if then else import true false none`, reserved `forall select`). Lexer never fails
(unknown run → `ERROR` token + lexical diagnostic).

**Negative literals** — resolved in the *parser*, not lexer: when `MINUS` is immediately adjacent
(no trivia) to a number in primary position (e.g. `x * -1`, `make_adder -1`), fold into one negative
`LITERAL`. In infix position (`a - b`) it's subtraction. Adjacency is testable because trivia are tokens.

**Hyphenated field names** (hard case D) — lexer emits `IDENT MINUS IDENT` separately; the parser
reassembles `IDENT (MINUS IDENT)*` into a `FIELD_NAME` node **only where a field name is expected**
(after `.`/`?.`, and as record/type/pattern field keys), and **only when raw-adjacent** (no trivia
between). So `cfg.target-triple` → one field name; `(cfg.target) - triple` → subtraction. Atom
bodies and field-name lexemes allow `-` via `is_atom_continue`; only bare `IDENT` excludes it.

**Event/marker engine** — `start()`→`Marker`, `Marker::complete(p, kind)`/`abandon(p)`,
`CompletedMarker::precede(p)` for left-assoc folding (Pratt), `bump`/`eat`/`at`/`nth_at`/`expect`/
`err_recover`. A step-limit guard guarantees termination on adversarial input. `process(events)`
re-attaches trivia (simple v0 rule: attach trivia leftward except newline-leading trivia before a
`Start`, which goes inside the new node — aesthetics only, not correctness).

**Shared primary + contextual types** (hard case A) — `primary(p, ctx)` parses atoms common to both
positions; `ctx: Expr | Type` flips `{` (value record/block vs type record) and `[` (list vs union).
Literals/`none`/`true`/`false` are valid in both (singleton types). One Pratt driver
`expr_bp(p, min_bp, ctx)` encodes the precedence ladder; `parse_type` = `expr_bp(p, 0, Type)` with
`?` (optional type, prec 1 postfix) and `->` (fn type, prec 8 right) enabled.

**9-level Pratt** (`exprs.rs`) — `(left_bp, right_bp)` pairs descending by precedence:
prec1 postfix `.`/`?.`/`?`; prec2 application (juxtaposition, left); prec3 `*` `/`; prec4 `+` `-`;
prec5 comparisons (**non-assoc**); prec6 `??` (right); prec7 `|>`/`<|`; prec8 `->` (right); prec9
`if`/`match`/`\` start a primary. Specific rules:
- *Application by juxtaposition*: after `lhs` + postfix chain, if current token ∈ primary-FIRST-set
  (`IDENT ATOM INT FLOAT STRING TRUE FALSE NONE L_PAREN L_BRACK L_BRACE BACKSLASH IF MATCH IMPORT TYPE`,
  plus `MINUS`+number adjacency) and we're under the app ceiling, fold into `CALL_EXPR` left-assoc,
  respecting `min_bp` so `x |> f a` groups `f a`. Guard `{`/`[` args by ctx + brace-disambiguation.
- *Non-assoc comparison*: comparison bp has `r_bp == l_bp`; after folding one, if another comparison
  follows, emit "comparison operators are non-associative; parenthesize" and wrap RHS in an error
  node (tree stays complete).
- *Pipeline-mix ban*: thread a `PipelineState`; mixing `|>` and `<|` in one unparenthesized chain →
  diagnostic; a `PAREN_EXPR` resets the state. (Verify against `anon_in_pipeline` in the fixture.)

**Brace disambiguation** (hard case B, expr position) — bounded lookahead from `{`: `}` → empty
record; `FIELD_NAME EQ` → value record; `FIELD_NAME COLON` → type record (only valid in Type ctx,
else recovery diagnostic); otherwise → block (`(IDENT := Expr ;)* Expr`). A `{` after a clause
signature/`-> ReturnPattern` is structurally always a block body.

**TopDecl dispatch + hard case C** (`decls.rs`) — at top level, an `IDENT` whose `nth(1)` ∈
`{:=, :, ::}` starts a decl; otherwise the final `Expr` begins. Forms: `IDENT := Expr`;
`IDENT : TypeExpr = Expr`; `IDENT :: TypeParamList? TypeExpr (:: Clause)+`; `IDENT :: Clause+`.
After `::`, a `[` is a `TYPE_PARAM_LIST` iff its depth-1 items are `,`-separated with **no** `;`
(`[A]`, `[A,B]`); a depth-1 `;` makes it a `TypeUnion` (`[A;]`, `[#neg; #zero; #pos;]`). A `type …`
body is a type definition needing no clauses (`Server :: type {…}`). `Clause ::= Pattern (-> Pattern)*
Guard? { Block }` — **include the `if Expr` guard** (required by `pattern-matching.md`, missing from
the grammar sketch). `Block ::= (IDENT := Expr ;)* Expr`.

**Error recovery** (`parser/mod.rs`) — never returns `Result`; tree always complete. Recovery sets:
`STMT_RECOVERY = { ; } ] ) EOF }` plus the decl-start predicate; each construct adds its closers.
`err_recover(msg, set)` wraps stray tokens in `ERROR_NODE` until a recovery token (one diagnostic per
run, no spam). A delimiter stack handles balanced recovery + unclosed `{`/`[`/`(` (diagnostic with
the opener span as a secondary label; complete open markers so the tree balances). Top-level loop
resyncs to the next decl-start so one broken decl doesn't sink the file.

**Diagnostics** (`diag/`) — `Diagnostic { range, severity, code, message, labels }` + `DiagnosticSink`,
rendering-agnostic; `code` maps to error-model.md classes. `render::render(diags, name, src)` builds
`ariadne::Report`s (ariadne computes line:col from byte spans). Only `render.rs` imports ariadne.

**Validation pass** (`validation.rs`) — tree-walking lints emitted as diagnostics: capitalization
convention (warn), reserved-name-as-binding (error), duplicate top-level binding / duplicate record
field (error). Non-assoc-comparison and pipeline-mix stay in the parser (they affect tree shape).

**Typed AST** (`ast/`) — hand-written `AstNode` newtypes over `SyntaxNode` (rust-analyzer pattern):
`can_cast`/`cast`/`syntax`, `support::child/children/token`. Enums (`Expr`, `Type`, `Pattern`,
`TopDecl`) over kind sets. Typed tokens decode lazily: `IntLiteral::value()`, `StringLiteral::value()`,
`FieldName::text()` (concatenate `IDENT (MINUS IDENT)*`), `BinExpr::op()`.

## Milestones (each independently landable + tested)

- [x] **M0 Scaffolding** — add deps; `SyntaxKind` + `rowan::Language` + `T!`; `parse()` stub returns empty `FILE`. Test: empty input round-trips.
- [x] **M1 Lexer** — port scalar helpers; maximal-munch tokenizer + trivia + keywords + negative-number hook + `ERROR`. Lexer unit tests.
- [x] **M2 Event/marker/builder spine** — `Tokens` view, `Event`, markers + `precede`, `bump`/`at`/`expect`, `process` w/ trivia reattachment. Test: token-soup round-trip.
- [x] **M3 Expression Pratt core** — primary, full 9-level ladder, juxtaposition, non-assoc + pipeline-mix, negative-literal fold. Precedence snapshots (the operator-precedence.md examples).
- [x] **M4 Composite exprs** — records/lists/tuples/lambda/match/if/import + brace disambiguation.
- [x] **M5 Types** — `parse_type` via shared Pratt; record/union/variant/optional/fn types; contextual `{`/`[`. Snapshots of `Abyss`/`Shadows`/`Unholy`/`NightmareRecord`.
- [x] **M6 Patterns** — incl. nested; tested against `unholy_match` clause patterns.
- [ ] **M7 TopDecls/Clauses/Block** — 4 forms, hard case C, guards, blocks. Bulk of `cursed.zt` parses.
- [ ] **M8 Hyphenated field names** — `FIELD_NAME` reassembly everywhere; `target-triple` tests.
- [ ] **M9 Error recovery** — recovery sets, `err_recover`, delimiter stack, unclosed handling, top-level resync, multi-diagnostic tests.
- [ ] **M10 Diagnostics rendering** — ariadne behind `render`; wire into `crates/cli`; snapshot rendered output.
- [ ] **M11 Typed AST + validation** — `AstNode` wrappers, typed tokens, lints.
- [ ] **M12 Acceptance** — `cursed.zt`: zero diagnostics, lossless round-trip, structural snapshot; never-panic property test.

## Verification

- `cargo test --workspace` — unit (lexer maximal-munch, adjacency), construct-level `expect-test`
  CST snapshots, precedence-shape tests, recovery tests (broken input → complete tree **and**
  multiple expected diagnostics), and the golden `cursed.zt` test.
- **Losslessness invariant**: assert `parse(src).syntax().text().to_string() == src` for the fixture
  and all snapshot inputs; a small proptest that random bytes never panic and always round-trip.
- **Acceptance**: `parse(include_str!(".../fixtures/cursed.zt"))` yields zero diagnostics + lossless round-trip.
- Manual: a tiny CLI path (`crates/cli`) that parses a `.zt` file and prints ariadne diagnostics;
  eyeball rendered output against an intentionally broken file to confirm rich, recovered errors.
- `cargo fmt` and `cargo clippy --workspace --all-targets` clean before finishing.
