## Optional values

Postfix `?` is the optional marker:

```zt
Text?
Int?
Bool?
```

`T?` is shorthand for `Optional T`.

`Optional` is an ordinary generic tagged union defined in the standard library — it is not a compiler primitive. Any user could write an equivalent definition:

```zt
Optional :: <T> type [
  none;
  some: { value: T; };
]
```

The compiler's only special knowledge of `Optional` is the `T?` → `Optional T` desugaring. All matching, construction, and type-checking behaviour follows from it being a normal generic tagged union.

There is no reserved `none` literal. `#none` and `#some` are ordinary atoms used by the `Optional` convention.

So:

```zt
Bool?
```

desugars to:

```zt
Optional Bool
```

Example:

```zt
Server :: type {
  host : Text;
  port : Int;
  tls  : Bool?;
}
```

The `tls` field is required, but its value must be either `#none` or `#some { value = true; }`.

Valid:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
  tls  = #some { value = true; };
}
```

Also valid:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
  tls  = #none;
}
```

Invalid:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
}
```

because `tls` is required.

---
