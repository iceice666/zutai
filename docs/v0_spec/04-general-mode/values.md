## 6. General-mode values

Core value forms:

```zt
none
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
```

### 6.1 None

`none` represents the absence of a value.

It is used by optional values and optional fields.

`none` is a reserved literal, not an atom. In type positions it may be used as a singleton literal type whose only value is `none`.

### 6.2 Booleans

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

### 6.3 Text

Text values are double-quoted strings:

```zt
"hello"
```

### 6.4 Atoms

Atoms in `.zt` use `#`:

```zt
#dev
#test
#prod
```

Atoms are symbolic singleton-like values, useful for modes, tags, enum cases, and discriminants.

Only `#`-prefixed values are atoms in `.zt`. The literals `true`, `false`, and `none` are also singleton-capable literals, but they are not atoms.

### 6.5 Records

Record values use `.zti`-style fields:

```zt
{
  host = "localhost";
  port = 8080;
}
```

Fields are semicolon-terminated.

### 6.6 Lists

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
