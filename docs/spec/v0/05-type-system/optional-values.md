## Optional values

Postfix `?` is the optional marker:

```zt
Text?
Int?
Bool?
```

`T?` is shorthand for `Optional T`.

`Optional` is a builtin generic union convention for value optionality:

```zt
Optional :: <T> type {
  #none;
  #some (T);
};
```

The compiler recognizes `Optional` as the meaning of postfix `T?`; matching and construction use the tuple-payload constructors above.

There is no reserved `none` literal. `#none` and `#some` are atoms used by the `Optional` convention.

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
};
```

The `tls` field is required, but its value must be either `#none` or `#some (true)`.

Valid:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
  tls  = #some (true);
};
```

Also valid:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
  tls  = #none;
};
```

Invalid:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
};
```

because `tls` is required.

---
