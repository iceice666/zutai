## Immediate mode `.zti`

### Purpose

`.zti` is Zutai's pure data literal format.

It is intended to be:

* deterministic
* non-evaluating
* easy to parse
* easy to validate
* suitable for SIMD structural scanning
* suitable for daemon-side lazy materialization

### Grammar

```ebnf
document ::= block

block    ::= "{" pair* "}"
pair     ::= field_name "=" value ";"

array    ::= "[" item* "]"
item     ::= value ";"

value    ::= "true"
           | "false"
           | atom
           | string
           | number
           | array
           | block

atom       ::= "#" atom_body
atom_body  ::= [A-Za-z_][A-Za-z0-9_-]*
field_name ::= [A-Za-z_][A-Za-z0-9_]*
string     ::= JSON-style-string
number     ::= JSON-style-number-without-type-postfix
```

Numeric type postfixes such as `1u8` and `3.14f64` are general-mode syntax only; `.zti` numbers remain JSON-style so immediate-mode data stays directly serializable.

### Top-level form

The top-level form of a `.zti` file must be a block:

```zti
{
  name = "demo";
}
```

This is invalid:

```zti
name = "demo";
```

This is also invalid:

```zti
[
  1;
  2;
]
```

### Fields and semicolons

Every block field is a pair:

```zti
name = value;
```

The trailing semicolon is required.

Valid:

```zti
{
  host = "localhost";
  port = 8080;
}
```

Invalid:

```zti
{
  host = "localhost"
  port = 8080
}
```

### Arrays

Arrays contain semicolon-terminated items:

```zti
[
  #logging;
  #metrics;
  #tracing;
]
```

The trailing semicolon is required for every item.

### Duplicate keys

Duplicate keys in the same block are invalid.

Invalid:

```zti
{
  port = 8080;
  port = 3000;
}
```

Rationale: `.zti` is deterministic data. There is no first-wins or last-wins rule.

### Comments

Canonical `.zti` v0 has no comments.

Invalid:

```zti
{
  // comment
  port = 8080;
}
```

Tools may support non-canonical preprocessing, but the canonical fast path does not include comments.

### Forbidden immediate-mode constructs

The following are invalid in `.zti`:

```zti
{
  x = 1 + 2;
  y = \a. a;
  z = readFile "foo.zti";
  a = if cond then x else y;
}
```

Immediate mode has no expressions. It has only values.

### Immediate-mode data model

The `.zti` data model contains:

```text
None
Bool
Int
Float
Text
Atom
Array
Block
```

A block is an ordered set of unique field names mapped to values.

Implementations may preserve source order for formatting and diagnostics, but semantic equality of blocks is structural and key-based.

When `.zti` data is imported into `.zt`, blocks become record values and arrays become list values. `.zti` has no tuple literal; general-mode tuple values are not part of the immediate-mode data model.

---
