## Optional fields

Optional fields use `?` on the field name:

```zt
RawServer :: type {
  host? : Text;
  port? : Int;
  tls? : Bool;
}
```

This means the field may be absent.

Valid:

```zt
raw : RawServer = {
  host = "localhost";
}
```

When accessed directly, an optional field evaluates to an optional value. If the field is absent, access returns `#none`; if the field is present, access returns `(#some, value = field_value)`, unless the field value is already optional and the access rule flattens it.

```zt
raw.port ?? 8080
```

### Optional field versus optional value

These are different:

```zt
tls : Bool?;
```

means the field must exist, but must contain an explicit optional value such as `#none` or `(#some, value = true)`.

```zt
tls? : Bool;
```

means the field may be absent.

```zt
tls? : Bool?;
```

means the field may be absent, and if present must contain an explicit optional `Bool`. Direct field access flattens the result to `Bool?`.

---
