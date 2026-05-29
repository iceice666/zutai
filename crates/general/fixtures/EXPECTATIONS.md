# Fixture expectations

The `valid/` fixtures are exercised by both the `token_soup_round_trips` test and the
`m11_validation_no_false_positives_valid_fixtures` test in `lib.rs`, plus `assert_parses_clean`
in `tests/acceptance.rs`.
The `invalid/` fixtures are all exercised by `assert_parses_with_error` in `tests/acceptance.rs`
(lossless round-trip + ≥1 diagnostic + no panic).
The `semantic_invalid/` fixtures parse clean today (parser has no type/semantic pass); they are
exercised by `assert_parses_clean` in the `semantic_gap` module in `tests/acceptance.rs`. When
semantic analysis lands, move each to `invalid/` and flip to `assert_parses_with_error`.

| file                                        | expected              | spec ref                                                                              |
|---------------------------------------------|-----------------------|---------------------------------------------------------------------------------------|
| cursed.zt                                   | parse-clean           | — (existing, all-valid stress fixture)                                                |
| valid/deep_nesting.zt                       | parse-clean           | records/unions/tagged-unions, operator-precedence — nests legal forms deeply          |
| valid/optional_chains.zt                    | parse-clean           | optional-values, optional-fields, field-access, defaulting-operator — maximal chains  |
| valid/lexical_torture.zt                    | parse-clean           | conventions 3.1–3.8 — layout-insensitive, hyphenated fields, atom hyphens            |
| valid/bracket_disambiguation.zt             | parse-clean           | grammar-sketch §30 — `[A]` vs `[A;]`, brace triple, empty `{}`/`[]`, mixed unions   |
| valid/guards_and_blocks.zt                  | parse-clean           | functions §8, file-structure §5 — `::` clause guards, block local bindings           |
| valid/type_position_torture.zt              | parse-clean           | type-system §9–§15 — `T??`, `(T?)?`, optional fields, function types in types        |
| invalid/sigil_swaps.zt                      | parse-error           | conventions 3.8 — `:` annotates types, `=` binds values; never overlap               |
| invalid/separator_swaps.zt                  | parse-error           | values 6.5/6.6, records 10.2 — lists/records use `;`, tuples use `,`                |
| invalid/comparison_chaining.zt              | parse-error           | operator-precedence — comparisons are non-associative                                 |
| invalid/pipeline_ambiguity.zt               | parse-error           | operator-precedence — mixing `\|>` and `<\|` without parens is ambiguous             |
| invalid/keyword_misuse.zt                   | parse-error           | conventions 3.6, file-structure 5.2 — keywords and reserved words                    |
| invalid/no_unary_operator.zt                | parse-error           | conventions 3.8 — v0 has no unary `-`/`!`/`+`; negation is literal-only             |
| invalid/atom_and_comment_traps.zt           | parse-error           | conventions 3.4–3.5 — atom syntax; `//` and `/* */` are not valid comment forms      |
| invalid/string_number_lexical.zt            | parse-error           | conventions 3.2/3.3 — JSON-style strings/numbers; listed forms not in spec           |
| invalid/bracket_mismatch.zt                 | parse-error           | grammar-sketch §30 — mismatched closing brackets; missing `else`; incomplete lambda  |
| invalid/declaration_traps.zt                | parse-error           | file-structure §5 — `: =` (missing type), `::` with no clauses, clause missing body |
| invalid/dangling_operators.zt               | parse-error           | operator-precedence §27 — binary/pipeline operators with missing operand; `T??` in expr |
| invalid/duplicate_names.zt                  | parse-error           | file-structure §5.5 (E0010) — duplicate top-level binding; records §10 (E0011)       |
| semantic_invalid/closed_records.zt          | parse-clean (future)  | records §10.1 — value record with extra/missing fields for declared type              |
| semantic_invalid/exhaustiveness.zt          | parse-clean (future)  | pattern-matching §19.1 — non-exhaustive match over finite union                       |
| semantic_invalid/union_membership.zt        | parse-clean (future)  | unions §16 — atom value outside its declared union set                                |
| semantic_invalid/reserved_tag.zt            | parse-clean (future)  | tagged-unions §17.5 — `_tag` written directly (reserved for desugaring)              |

## Notes on sigil_swaps.zt

The canonical rule (conventions 3.8, records 10.1, tagged-unions 17):
- `:` appears **only** in type positions: type-record fields (`type { host : Text; }`), variant type
  fields (`(#circle, radius : Float)`), optional-field markers (`host? : Text`).
- `=` appears everywhere a field gets a value or is matched: value records (`{ host = "x"; }`),
  variant construction (`(#circle, radius = 5.0)`), all patterns.
The file exercises every swap: value records using `:`, type records using `=`, construction using `:`,
and variant type fields using `=`.

## Notes on separator_swaps.zt

- Lists `[...]` are `;`-terminated per element (spec values 6.6). `,` is for tuples/variants only.
- Records `{...}` are `;`-terminated per field (spec values 6.5). `,` is invalid.
- Tuples/variants `(...)` are `,`-separated. `;` is invalid.
- `record with { ... }` is not a v0 form (records 10.2); only v1 has record update.
- `[A, B;]` mixes type-param list separators (`,`) with union separators (`;`) illegally.

## Notes on comparison_chaining.zt

Comparisons (`==`, `!=`, `<`, `<=`, `>`, `>=`) are **non-associative** at level 11 in the Pratt
table (operator-precedence.md). Chaining two comparisons without parentheses is rejected by the
parser at M3.

## Notes on pipeline_ambiguity.zt

The spec (operator-precedence.md) states: "When `|>` and `<|` appear together without parentheses,
implementations should reject the expression as ambiguous." This is enforced at M3.

## Notes on no_unary_operator.zt

v0 has no unary operators. Negation is only valid as part of a numeric literal (`-5`, `-2.5e-3`).
`-x`, `!b`, `+n` are not valid expressions. The lexer tokenises `-` as MINUS so round-trip
works, but the parse / validation pass must reject these. Full detection requires M9+.
Note: `--x` is now a valid line comment (not a double-minus trap) and has been removed from this fixture.

## Notes on atom_and_comment_traps.zt

Atom grammar: `atom ::= "#" atom_body` where `atom_body ::= [A-Za-z_][A-Za-z0-9_-]*`.
- Bare `#` (no body): not an atom.
- `# foo` (space after `#`): `#` is not an atom prefix when followed by whitespace.
- `#1foo`, `#-foo`: atom bodies must start with a letter or `_`.
- `##foo`: `#` is not a valid atom-body character.
`//` and `/* */` are not valid comment forms in `.zt` (see §3.1.1 — valid comment forms
start with `--`). `#`-lines are atom-syntax traps, not shell-style comments.

## Notes on string_number_lexical.zt

Number grammar is JSON-style (conventions 3.3): `[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
Not in spec: leading zeros (`007`), lone `.` suffix (`3.`), leading dot (`.5`), incomplete
exponent (`1e`, `1e+`), double dot (`1.2.3`), hex (`0x...`), binary (`0b...`), underscores
(`1_000`).
Note: `--5` is now a valid line comment and has been removed from this fixture.
String grammar is JSON-style (conventions 3.2). Unterminated strings, bad escapes (`"\q"`),
and raw (unescaped) newlines inside string literals are invalid.

## Notes on bracket_mismatch.zt

Each line is a standalone snippet that independently triggers a diagnostic via `expect()` sites:
- `{ a = 1 ]` — value record closed with `]` instead of `}`.
- `[ 1; 2; )` — list closed with `)` instead of `]`.
- `( 1, 2 ]` — tuple closed with `]` instead of `)`.
- `match x { _ => 1` — unclosed match arm (missing `;`) and unclosed `{`.
- `if c then a` — missing `else` branch.
- `\x` — lambda with no `=>` or block body.

Note: function signatures starting with `{` at depth 0 cause `has_type_sig` to return false,
so the parser treats the `{...}` as a clause pattern. Avoid putting record types as the first
element of a function signature in a test fixture to prevent step-limit panics in `record_pattern`.

## Notes on declaration_traps.zt

Malformed declaration structure, distinct from keyword_misuse:
- `x : = 5` — annotated binding where `: =` (space between) is two tokens; `=` (EQ) is not a
  valid TypeExpr start, so "expected type expression after ':'" fires.
- `g ::` — function decl with nothing after `::` (EOF); "expected at least one clause" fires.
- `f :: 0` — single clause with pattern `0` but no `{` body; "expected '{' to start clause body" fires.
- `h :: n ->` — partial clause: pattern `n`, `->`, then EOF; err_recover fires for missing second pattern.

## Notes on dangling_operators.zt

Incomplete binary and pipeline expressions where the RHS is missing. Each triggers
"expected expression" at the `expr_bp` call site (exprs.rs). Also includes `v := Server??`
which is `Server ??` in expression position — `??` is a coalesce infix in Expr context,
so the missing RHS triggers the error. Contrast: `Server??` in TYPE position is the
double-optional shorthand (OPTIONAL_TYPE) and is silently accepted — only the expr form errors.

## Notes on duplicate_names.zt

Exercises the validation pass (validation.rs, not the syntactic parser):
- E0010 duplicate top-level binding: two `x := …` declarations at root level.
- E0011 duplicate record field: `{ a = 1; a = 2; }` — both top-level expression and inside a binding.
  E0011 walks `root.descendants()` so it fires at any nesting depth.
