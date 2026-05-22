## 4. Immediate mode `.zti`

### 4.1 Purpose

`.zti` is Zutai's pure data literal format.

It is intended to be:

* deterministic
* non-evaluating
* easy to parse
* easy to validate
* suitable for SIMD structural scanning
* suitable for daemon-side lazy materialization

### 4.2 Grammar

```ebnf
document ::= block

block    ::= "{" pair* "}"
pair     ::= field_name "=" value ";"

array    ::= "[" item* "]"
item     ::= value ";"

value    ::= "true"
           | "false"
           | "none"
           | atom
           | string
           | number
           | array
           | block

atom       ::= "#" atom_body
atom_body  ::= [A-Za-z_][A-Za-z0-9_-]*
field_name ::= [A-Za-z_][A-Za-z0-9_-]*
string     ::= JSON-style-string
number     ::= JSON-style-number
```

### 4.3 Top-level form

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

### 4.4 Fields and semicolons

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

### 4.5 Arrays

Arrays contain semicolon-terminated items:

```zti
[
  #logging;
  #metrics;
  #tracing;
]
```

The trailing semicolon is required for every item.

### 4.6 Duplicate keys

Duplicate keys in the same block are invalid.

Invalid:

```zti
{
  port = 8080;
  port = 3000;
}
```

Rationale: `.zti` is deterministic data. There is no first-wins or last-wins rule.

### 4.7 Comments

Canonical `.zti` v0 has no comments.

Invalid:

```zti
{
  // comment
  port = 8080;
}
```

Tools may support non-canonical preprocessing, but the canonical fast path does not include comments.

### 4.8 Forbidden immediate-mode constructs

The following are invalid in `.zti`:

```zti
{
  x = 1 + 2;
  y = fn a => a;
  z = import "foo.zti";
  a = if cond then x else y;
}
```

Immediate mode has no expressions. It has only values.

### 4.9 Immediate-mode data model

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

---

