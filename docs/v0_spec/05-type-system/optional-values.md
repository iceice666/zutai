## Optional values

Postfix `?` is the optional marker:

```zt
Text?
Int?
Bool?
Server?
```

`T?` is shorthand for `Optional T`.

The optional type is an ordinary generic union type:

```zt
Optional :: [T] type [
  #none;
  (#some, value : T);
]
```

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
  tls : Bool?;
}
```

This means the `tls` field is required, but its value must be either `#none` or a `#some` tuple carrying a `Bool`, such as `(#some, value = true)`.

Valid:

```zt
server : Server = {
  host = "localhost";
  port = 8080;
  tls = (#some, value = true);
}
```

Also valid:

```zt
server : Server = {
  host = "localhost";
  port = 8080;
  tls = #none;
}
```

Invalid:

```zt
server : Server = {
  host = "localhost";
  port = 8080;
}
```

because `tls` is required.

---
