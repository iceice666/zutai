## 3. Lexical conventions

### 3.1 Whitespace

Whitespace separates tokens but is otherwise insignificant outside strings.

### 3.1.1 Comments

Comments are a `.zt`-only feature. Immediate-mode `.zti` files are comment-free (see §4.7).

All comment forms share the `--` sigil. The character **immediately** after `--`
determines the form; there must be no space between the two dashes and the
disambiguator character.

| Form | Syntax | Description |
| ---- | ------ | ----------- |
| Line | `-- text` | Ignored from `--` to end of line. |
| Doc  | `--\| text` | Like a line comment, but attaches to the following declaration as documentation. Stacked `--\|` lines concatenate into one doc block. The body is a Markdown subset. |
| Block | `--{ … }--` | Spans multiple lines; fully nestable (`--{ … --{ … }-- … }--`). Ignored entirely. |
| Node comment | `--/` | Comments out the immediately following item (top-level declaration, record field, list item, or tuple item). The item is parsed but excluded from the semantic view of the file. |

Because the disambiguator is the character immediately after `--`, a plain line
comment whose text begins with `/`, `|`, or `{` requires a leading space:

```zt
-- /usr/local/bin    -- ok: plain line comment
--/                  -- error: node comment marker (expected item to follow)
```

Doc-comment blocks:

```zt
--| The HTTP port the server binds to.
--| Accepts values 1–65535.
port : Int = 8080
```

Block comment (nestable):

```zt
--{
  Disabled until the v1 type-class system lands.
  --{ nested inner note }--
}--
```

Node comment — comments out the following item:

```zt
--/ old-field = "deprecated";    -- inside a record
--/ x := 42                      -- top-level declaration
```

### 3.2 Strings

Strings are double-quoted and JSON-like:

```zti
"hello"
"/usr/local/bin"
"https://example.com"
```

Paths, URLs, and other complex textual values should be represented as strings, not atoms.

### 3.3 Numbers

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

### 3.4 Immediate-mode atoms

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
none
```

They are keyword literals. They may participate in singleton literal types, but they are not `Atom` values.

### 3.5 General-mode atoms

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

### 3.6 Identifiers

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
none
```

The following identifiers are reserved for future versions but not used in v0:

```zt
forall
select
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
server : Server = {
  host = "localhost";
  port = 8080;
}
```

This capitalization rule is a static convention. Implementations should warn or error when violated.

### 3.7 Field names

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

### 3.8 Symbols and operators

This is the canonical list of every symbol and operator in the language. Binding-precedence
and associativity for the infix/postfix forms are defined in
[Operator precedence](operator-precedence.md).

The two field sigils are kept strictly separate: **`:` annotates a type, `=` binds a value
or pattern**. They never overlap.

| Symbol           | Meaning                                                                    |
| ---------------- | -------------------------------------------------------------------------- |
| `--`             | line comment (to end of line)                                              |
| `--\|`           | doc-comment line (attaches to the following declaration)                   |
| `--{` … `}--`   | nestable block comment                                                     |
| `--/`            | node comment: excludes the immediately following item                     |
| `:=`             | inferred value binding (`name := expr`)                                    |
| `:`              | type annotation ("has type"): annotated bindings, type-record/tuple type fields, optional-field marker |
| `::`             | function/type definition: signature line and pattern-clause lines          |
| `=`              | value/pattern field binding: value records, tuple values, all patterns |
| `->`             | function type arrow; also separates clause parameter patterns              |
| `=>`             | anonymous-function body (short form) and `match` arm body                  |
| `\`              | anonymous function (lambda) introducer                                     |
| `.`              | field and module-member access                                             |
| `?.`             | optional chaining                                                          |
| `?`              | postfix optional type (`T?`) and optional-field marker (`field? : T`)      |
| `??`             | defaulting operator                                                        |
| `\|>` / `<\|`    | forward / backward pipeline                                                |
| `#`              | atom prefix                                                                |
| `+` `-` `*` `/`  | arithmetic operators                                                       |
| `==` `!=` `<` `<=` `>` `>=` | comparison operators                                            |
| `&&` `\|\|`      | logical AND / OR (short-circuit); operands and result are `Bool`           |
| `...`            | open row tail in record/union types — v1 feature, reserved |
| `;`              | terminator for fields, list items, clauses, and match arms                 |
| `,`              | separator between tuple fields                                             |
| `{` `}`          | value record, record type, or block body                                   |
| `[` `]`          | list value or union type                                                   |
| `(` `)`          | tuple, grouping, and the empty tuple                                       |
| `_`              | wildcard pattern                                                           |

There is no unary operator in v0: negation is part of a numeric literal (e.g. `-10`, `x * -1`).

---
