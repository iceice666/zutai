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
raw :: RawServer = {
  host = "localhost";
}
```

When accessed directly, an optional field evaluates to `Maybe T`, preserving physical field presence:

```zt
Maybe :: <T> type {
  #absent;
  #present (T);
}
```

If the field is absent, access returns `#absent`; if the field is present, access returns `#present (field_value)`.

```zt
raw.port ?? 8080
```

### Optional field versus optional value

These are different:

```zt
tls : Bool?;
```

means the field must exist, but must contain an explicit optional value such as `#none` or `#some (true)`.

```zt
tls? : Bool;
```

means the field may be absent. Direct access has type `Maybe Bool`.

```zt
tls? : Bool?;
```

means the field may be absent, and if present must contain an explicit optional `Bool`. Direct field access has type `Maybe (Optional Bool)`.

### Presence is not flattened

When a field is declared `field? : T?`, two layers exist: the field may be absent, and the value (if present) is itself optional. Field access keeps both layers:

- Field absent → `#absent`
- Field present, value is `#none` → `#present (#none)`
- Field present, value is `#some (v)` → `#present (#some (v))`

The result type is `Maybe (Optional T)`, never plain `Optional T`. See [Field access and optional chaining](field-access.md) for the chaining rule.

---
