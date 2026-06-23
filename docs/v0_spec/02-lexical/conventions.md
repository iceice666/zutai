## Lexical conventions

### Whitespace

Whitespace separates tokens but is otherwise insignificant outside strings.

In `.zt`, top-level declarations are separated by line boundaries at delimiter depth zero. A top-level declaration does not use a trailing semicolon.

### General-mode comments

In `.zt`, comments are treated as whitespace outside strings.

Line comments begin with `--` and continue to the end of the line:

```zt
-- this is a comment
answer ::= 42
```

Block comments begin with `--[` and end with `]--`. Block comments may nest:

```zt
--[
  outer comment
  --[ nested comment ]--
]--
answer ::= 42
```

Doc comments begin with `--|` and continue to the end of the line:

```zt
--| Documentation for answer.
answer ::= 42
```

In v0, doc comments are lexically distinct from ordinary line comments, but they have no required semantic effect.

### Strings

Strings are double-quoted and JSON-like:

```zti
"hello"
"/usr/local/bin"
"https://example.com"
```

Paths, URLs, and other complex textual values should be represented as strings, not atoms.

### Numbers

Immediate mode numbers use JSON-style syntax. General mode numbers use the same base syntax plus optional numeric type postfixes.

Examples:

```zti
0
123
-10
3.14
1e9
-2.5e-3
```

General mode also allows numeric type postfixes:

```zt
255u8
-128i8
8080u16
42i64
1f32
3.14f64
1e9f64
1p32
1.5p32e3
2p64
4.0p64e5
```

The allowed number type postfixes — the `NumberTypePostfix` production in the grammar reference — are: `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `f32`, `f64`, `p32`, `p64`, `p32eN`, and `p64eN`.

Semantic rules for number type postfixes:

- A number postfix is part of the numeric literal token and must appear immediately after the numeric body with no whitespace.
- Without a postfix, existing v0 inference is unchanged: an integer-looking body has type `Int`; a body with a `.` or an exponent has type `Float`.
- `i8`, `i16`, `i32`, `i64` require an integer body with no fractional part and no exponent; they may use a leading `-`.
- `u8`, `u16`, `u32`, `u64` require an integer body with no fractional part, no exponent, and no leading `-`.
- `f32` and `f64` may use an integer-looking body, a fractional body, or an exponent body; `1f32`, `3.14f64`, and `1e9f64` are valid.
- `p32`, `p64`, `p32eN`, and `p64eN` are experimental posit literal postfixes. They may use integer-looking, fractional, or exponent numeric bodies. `p32` and `p64` mean exponent size `e2`; explicit `eN` requires `0 <= N < 32` for `p32eN` and `0 <= N < 64` for `p64eN`.
- A letter, digit, or `_` immediately following a numeric body is treated as a candidate postfix; if it is not exactly one of the allowed postfixes, the literal is a lexical error. This makes `1foo`, `1i128`, `1_u8`, and `1ms` invalid rather than parsing as two adjacent tokens.
- A `.` followed by a digit remains part of the numeric body; a `.` followed by a non-digit remains field access, so `1.foo` is field access on `1`, while `1.0foo` is an invalid numeric postfix.

The core numeric types are:

```zt
Int
Float
i8   i16  i32  i64
u8   u16  u32  u64
f32  f64
Posit32  Posit64  Posit32eN  Posit64eN
```

`Int` is the default signed integer type and is an alias of `i64` in v0.

`Float` is the default floating-point type and is an alias of `f64` in v0.

`i8`, `i16`, `i32`, `i64` are signed two's-complement integer types of exactly 8, 16, 32, and 64 bits.

`u8`, `u16`, `u32`, `u64` are unsigned integer types of exactly 8, 16, 32, and 64 bits.

`f32` and `f64` are IEEE-754 binary32 and binary64 floating-point types.

`Posit32`, `Posit64`, `Posit32eN`, and `Posit64eN` are experimental implementation-provided posit scalar types for supported exponent sizes. `Posit32` and `Posit64` are aliases for `e2`.

`Number` remains an optional host/prelude convenience and is not required for the v0 core.

These postfixed literals are invalid:

- `-1u8` — unsigned postfixes reject a leading `-`.
- `1.0i64` — integer postfixes reject a fractional body.
- `1e3u32` — integer and unsigned postfixes reject an exponent body.
- `1i128` — `i128` is not in the fixed postfix set (unknown suffix).
- `1ms` — unit suffixes are not in the fixed postfix set (unknown suffix).
- `1p16`, `1p32e32`, `1p64e64`, `1p32e01`, and `1p32e` — invalid posit postfix candidates.
- `1foo` — an arbitrary identifier run is not a valid postfix (unknown suffix).

### Immediate-mode atoms

In `.zti`, atom literals use the same `#` prefix as `.zt`:

```ebnf
atom_body ::= ("_" | XID_Start) ("_" | XID_Continue | "-")*
atom      ::= "#" atom_body
```

Examples:

```zti
#prod
#x86_64-linux
#logging
```

These are valid:

```zti
{
  profile = #prod;
  target = #x86_64-linux;
}
```

These are not atoms and should be strings:

```zti
{
  path = "/usr/local/bin";
  url = "https://example.com";
  dotted = "foo.bar";
}
```

The following are reserved literals, not atoms:

```zti
true
false
```

They are keyword literals. They may participate in singleton literal types, but they are not `Atom` values.

### General-mode atoms

In `.zt`, atom literals also use a `#` prefix:

```zt
#dev
#test
#prod
#x86_64-linux
```

The atom body uses the same simple atom shape as immediate mode:

```ebnf
atom_body ::= ("_" | XID_Start) ("_" | XID_Continue | "-")*
atom      ::= "#" atom_body
```

The `#` prefix avoids ambiguity with identifiers and field access and keeps atom syntax consistent across `.zti` and `.zt`.

When `.zti` data is imported into `.zt`, atoms preserve their `#` spelling.

Example:

```zti
{
  profile = #prod;
}
```

Imported into `.zt`, the value is equivalent to:

```zt
{
  profile = #prod;
}
```

### Identifiers

Binding identifiers follow Unicode UAX #31 identifier classes:

```ebnf
ident ::= ("_" | XID_Start) ("_" | XID_Continue)*
```

Syntactic keywords and reserved literals cannot be used as identifiers. This includes:

```zt
type
match
if
then
else
import
true
false
```

The following identifiers are reserved for future versions but not used in v0:

```zt
select
perform
handle
with
resume
```

Examples:

```zt
server
normalizeServer
RawServer
Config
```

Unicode identifiers such as `café` and `名前` are valid. Identifiers are compared by Unicode scalar sequence; no normalization is applied.

Type-valued bindings should be capitalized:

```zt
Server :: type {
  host : Text;
  port : Int;
}
```

Runtime value bindings should be lowercase:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
}
```

This capitalization rule is enforced statically for scripts with case. Parsers and elaborators must error when a cased type-valued binding starts with a lowercase letter, or when a cased runtime value binding starts with an uppercase letter; identifiers whose first scalar is caseless are unaffected.

### Field names

Field names are bare labels and use the identifier-like Unicode shape:

```ebnf
field_name ::= ("_" | XID_Start) ("_" | XID_Continue)*
```

Atoms may contain `-`; field names may not. Use `_` or camel case for multiword
fields:

```zt
{
  host = "localhost";
  target_triple = "x86_64-linux";
}
```

Field access uses `.`:

```zt
cfg.host
cfg.target_triple
```

After `.`, the parser consumes a `field_name`. Therefore:

```zt
cfg.target-triple
```

means `(cfg.target) - triple`, not field access to `target-triple`.

### Symbols and operators

This is the canonical list of every symbol and operator in the language. Binding-precedence
and associativity for the infix/postfix forms are defined in
[Operator precedence](operator-precedence.md).

The two field sigils are kept strictly separate: **`:` annotates a type, `=` binds a value
or pattern**. They never overlap.

| Symbol           | Meaning                                                                    |
| ---------------- | -------------------------------------------------------------------------- |
| `:=`             | inferred local binding (`name := expr;`)                                  |
| `::=`            | inferred top-level value binding (`name ::= expr`)                        |
| `:`              | type annotation in type positions: type-record fields, tuple type fields, optional-field marker |
| `::`             | typed binding, function signature, import declaration, and type definition |
| `\|`             | match arm introducer inside `match` bodies                                |
| `=`              | value/pattern field binding and top-level function clause introducer       |
| `->`             | function type arrow                                                        |
| `=>`             | function clause and `match` arm body separator                             |
| `\`              | anonymous function (lambda) introducer                                     |
| `.`              | lambda body separator (after `\params`); also field and module-member access |
| `?.`             | optional chaining                                                          |
| `?`              | postfix optional type (`T?`) and optional-field marker (`field? : T`)      |
| `??`             | defaulting operator                                                        |
| `\|>` / `<\|`    | forward / backward pipeline                                                |
| `#`              | atom prefix                                                                |
| `+` `-` `*` `/`  | arithmetic operators                                                       |
| `==` `!=` `<` `<=` `>` `>=` | comparison operators                                            |
| `&&` `\|\|`      | logical AND / OR (short-circuit); operands and result are `Bool`           |
| `...`            | row tail / union spread in record or union types — v1 feature, reserved |
| `;`              | terminator for fields, list items, local bindings, and match arms          |
| `,`              | separator between tuple fields                                             |
| `{` `}`          | value record, record type, tagged union type, or block body                |
| `[` `]`          | list value                                                                 |
| `(` `)`          | tuple, grouping, and the empty tuple                                       |
| `_`              | wildcard pattern                                                           |

There is no unary operator in v0: negation is part of a numeric literal (e.g. `-10`, `x * -1`).

**Lambda-dot disambiguation**: the `.` in `\params. body` is the lambda body separator. Whitespace must separate the final pattern from `.` and `.` from the start of the body. `\x.y` (no space before the dot) is a parse error; write `\x. y`.

---
