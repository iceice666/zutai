## 12. Optional values

Postfix `?` is the optional marker:

```zt
Text?
Int?
Bool?
Server?
```

`T?` means:

```zt
type [
  T;
  none;
]
```

Here `none` is used as a singleton literal type.

So:

```zt
Bool?
```

desugars to:

```zt
type [
  Bool;
  none;
]
```

Example:

```zt
let Server: Type = type {
  host = Text;
  port = Int;
  tls = Bool?;
}
```

This means the `tls` field is required, but its value may be either `Bool` or `none`.

Valid:

```zt
let server: Server = {
  host = "localhost";
  port = 8080;
  tls = true;
}
```

Also valid:

```zt
let server: Server = {
  host = "localhost";
  port = 8080;
  tls = none;
}
```

Invalid:

```zt
let server: Server = {
  host = "localhost";
  port = 8080;
}
```

because `tls` is required.

---

