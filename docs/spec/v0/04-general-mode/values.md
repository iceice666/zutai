## General-mode values

Core value forms:

```zt
true
false
123
3.14
"hello"
#prod
#circle { radius = 5.0; }
{ 1; 2; 3; }
{
  host = "localhost";
  port = 8080;
}
```

### Booleans

```zt
true
false
```

`true` and `false` are reserved literals, not atoms. In type positions they may be used as singleton literal types.

Conceptually:

```zt
Bool
```

is the finite type:

```zt
type {#true; #false;}
```

### Text

Text values are double-quoted strings:

```zt
"hello"
```

### Atoms

Atoms in `.zt` use `#`:

```zt
#dev
#test
#prod
```

Atoms are symbolic singleton-like values, useful for modes, tags, and enum cases.

Only `#`-prefixed values are atoms in `.zt`. The literals `true` and `false` are also singleton-capable literals, but they are not atoms.

The atom `#none` has no special lexical status. It is an ordinary atom that the optional type convention uses as its empty case.

### Records

Record values use `.zti`-style fields:

```zt
{
  host = "localhost";
  port = 8080;
}
```

Fields are semicolon-terminated.

When a field value is the identifier with the same name as the field, the value may be omitted, leaving the `=` in place. The field `name =;` is shorthand for `name = name;`:

```zt
{
  host =;
  port =;
}
```

is equivalent to:

```zt
{
  host = host;
  port = port;
}
```

The shorthand also applies to record-update fields, so `cfg with { port =; }` means `cfg with { port = port; }`.

### Tagged union values

A tag with no payload is a bare atom:

```zt
#north
#prod
#none
```

A tag with a record payload is an atom followed by a record:

```zt
#circle { radius = 5.0; }
#rect   { width = 4.0; height = 3.0; }
#some   { value = true; }
```

Tagged union values are general-mode values and are not part of `.zti` serialization in v0.

### Tuples

Tuples use parentheses and comma-separated items:

```zt
(1, 2)
()
```

A parenthesized single expression is a group, not a one-element tuple. Tuples are used for anonymous structured data and intermediate computation.

The empty tuple `()` is the **unit value**. Its type is the empty tuple type, also spelled `Unit` (see [Type system](../05-type-system/overview.md)). `()` is the only value tuple that does not require a comma; every other tuple needs at least one comma.

### Lists

Lists use a `{ … }` parallel container with semicolon-terminated bare elements (no `=`, which distinguishes a list from a record):

```zt
{
  "alpha";
  "beta";
  "gamma";
}
```

Inline form is also valid:

```zt
{ "alpha"; "beta"; "gamma"; }
```

---
