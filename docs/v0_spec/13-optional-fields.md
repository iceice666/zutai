## 13. Optional fields

Optional fields use `?` on the field name:

```zt
let RawServer: Type = type {
  host? = Text;
  port? = Int;
  tls? = Bool;
}
```

This means the field may be absent.

Valid:

```zt
let raw: RawServer = {
  host = "localhost";
}
```

When accessed directly, an absent optional field evaluates to `none`:

```zt
raw.port ?? 8080
```

### 13.1 Optional field versus optional value

These are different:

```zt
tls = Bool?;
```

means the field must exist, but may contain `none`.

```zt
tls? = Bool;
```

means the field may be absent.

```zt
tls? = Bool?;
```

means the field may be absent, and if present may be `Bool` or `none`.

---

