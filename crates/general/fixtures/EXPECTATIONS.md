# Fixture expectations

The `valid/` fixtures are exercised by both the lexer-lossless test and the parser round-trip test.
The `invalid/` fixtures are exercised by the lexer-lossless test only — some invalid constructs
(e.g. `type { field = Type; }` with swapped sigils) trigger the parser's step-limit guard before
M9 error-recovery lands, so they cannot safely be passed through the round-trip harness yet.

Rows marked **(M3)** are detectable at the current parser state (M6 / M3 expression core).
Rows marked **(M7+)** require top-level declaration parsing (not yet implemented).
Rows marked **(M9+)** require the error-recovery and diagnostic pass.
Rows marked **(M11+)** require the typed-AST validation pass.

| file                              | expected     | spec ref                                                                         |
|-----------------------------------|--------------|----------------------------------------------------------------------------------|
| cursed.zt                         | parse-clean  | — (existing, all-valid stress fixture)                                           |
| valid/deep_nesting.zt             | parse-clean  | records/unions/tagged-unions, operator-precedence — nests legal forms deeply     |
| valid/optional_chains.zt          | parse-clean  | optional-values, optional-fields, field-access, defaulting-operator — maximal chains |
| valid/lexical_torture.zt          | parse-clean  | conventions 3.1–3.8 — layout-insensitive, hyphenated fields, atom hyphens       |
| invalid/sigil_swaps.zt            | parse-error (M7+) | conventions 3.8 — `:` annotates types, `=` binds values; never overlap     |
| invalid/separator_swaps.zt        | parse-error (M7+) | values 6.5/6.6, records 10.2 — lists/records use `;`, tuples use `,`       |
| invalid/comparison_chaining.zt    | parse-error **(M3)** | operator-precedence — comparisons are non-associative                    |
| invalid/pipeline_ambiguity.zt     | parse-error **(M3)** | operator-precedence — mixing `\|>` and `<\|` without parens is ambiguous |
| invalid/keyword_misuse.zt         | parse-error (M7+) | conventions 3.6, file-structure 5.2 — keywords and reserved words        |
| invalid/no_unary_operator.zt      | parse-error (M9+) | conventions 3.8 — v0 has no unary `-`/`!`/`+`; negation is literal-only  |
| invalid/atom_and_comment_traps.zt | parse-error (M9+) | conventions 3.4–3.5 — atom syntax; `//` and `/* */` are not valid comment forms in `.zt` |
| invalid/string_number_lexical.zt  | parse-error (M9+) | conventions 3.2/3.3 — JSON-style strings/numbers; listed forms not in spec|

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
