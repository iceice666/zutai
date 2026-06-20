## Lexical conventions

### Whitespace

Whitespace separates tokens but is otherwise insignificant outside strings.

In `.zt`, top-level declarations are separated by line boundaries at delimiter depth zero. A top-level declaration does not use a trailing semicolon.

### General-mode comments

In `.zt`, comments are treated as whitespace outside strings.

Line comments begin with `--` and continue to the end of the line:

```zt
-- this is a comment
answer := 42
```

Block comments begin with `--[` and end with `]--`. Block comments may nest:

```zt
--[
  outer comment
  --[ nested comment ]--
]--
answer := 42
```

Doc comments begin with `--|` and continue to the end of the line:

```zt
--| Documentation for answer.
answer := 42
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

Numbers use JSON-style syntax.

Examples:

```zti
0
123
-10
3.14
1e9
-2.5e-3
```

The core numeric types are:

```zt
Int
Float
```

`Int` is an integer type.

`Float` is an IEEE-754 binary64 floating-point type in v0.

A host implementation may expose `Number` as a convenience supertype or alias, but the v0 core distinguishes `Int` and `Float`.

### Immediate-mode atoms

In `.zti`, atom literals use the same `#` prefix as `.zt`:

```ebnf
atom_body ::= [A-Za-z_][A-Za-z0-9_-]*
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
atom_body ::= [A-Za-z_][A-Za-z0-9_-]*
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

Binding identifiers use:

```ebnf
ident ::= [A-Za-z_][A-Za-z0-9_]*
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

This capitalization rule is enforced statically. Parsers and elaborators must error when a type-valued binding starts with a lowercase letter, or when a runtime value binding starts with an uppercase letter.

### Field names

Field names are bare labels and use the atom-body shape:

```ebnf
field_name ::= [A-Za-z_][A-Za-z0-9_-]*
```

This allows fields such as:

```zt
{
  host = "localhost";
  target-triple = "x86_64-linux";
}
```

Field access uses `.`:

```zt
cfg.host
cfg.target-triple
```

After `.`, the parser consumes a `field_name` as a field token. Therefore:

```zt
cfg.target-triple
```

means field access to `target-triple`, not subtraction.

If subtraction is intended, write:

```zt
(cfg.target) - triple
```

### Symbols and operators

This is the canonical list of every symbol and operator in the language. Binding-precedence
and associativity for the infix/postfix forms are defined in
[Operator precedence](operator-precedence.md).

The two field sigils are kept strictly separate: **`:` annotates a type, `=` binds a value
or pattern**. They never overlap.

| Symbol           | Meaning                                                                    |
| ---------------- | -------------------------------------------------------------------------- |
| `:=`             | inferred value binding (`name := expr`)                                    |
| `:`              | type annotation in type positions: type-record fields, tuple type fields, optional-field marker |
| `::`             | typed binding, function signature, and type definition                     |
| `\|`             | clause introducer inside `{ }` blocks — both function bodies and `match` bodies |
| `=`              | value/pattern field binding: value records, named tuple fields, and all patterns |
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
| `{` `}`          | value record, record type, or block body                                   |
| `[` `]`          | list value or union type                                                   |
| `(` `)`          | tuple, grouping, and the empty tuple                                       |
| `_`              | wildcard pattern                                                           |

There is no unary operator in v0: negation is part of a numeric literal (e.g. `-10`, `x * -1`).

**Lambda-dot disambiguation**: the `.` in `\params. body` is the lambda body separator. Whitespace must separate the final pattern from `.` and `.` from the start of the body. `\x.y` (no space before the dot) is a parse error; write `\x. y`.

---
