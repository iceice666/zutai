## General-mode values

Core value forms:

```zt
true
false
123
3.14
"hello"
#prod
[1; 2; 3;]
{
  host = "localhost";
  port = 8080;
}
(#circle, radius = 5.0)
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
type [
  true;
  false;
]
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

### Tuples

Tuples use parentheses and comma-separated items:

```zt
(#circle, radius = 5.0)
(1, 2)
()
```

Named tuple fields use `=` in value and pattern positions. A parenthesized single expression is a group, not a one-element tuple.

Tuples are general-mode values. They are used for structured union alternatives and intermediate computation, but v0 does not define a direct `.zti` serialization for tuple values.

### Lists

Lists use semicolon-terminated elements:

```zt
[
  "alpha";
  "beta";
  "gamma";
]
```

Inline form is also valid:

```zt
["alpha"; "beta"; "gamma";]
```

---
