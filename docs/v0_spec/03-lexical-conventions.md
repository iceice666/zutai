## 3. Lexical conventions

### 3.1 Whitespace

Whitespace separates tokens but is otherwise insignificant outside strings.

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
let
fn
type
forall
match
if
then
else
import
select
true
false
none
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
let Server: Type = type {
  host = Text;
  port = Int;
}
```

Runtime value bindings should be lowercase:

```zt
let server: Server = {
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

---

