## 14. Field access and optional chaining

### 14.1 Required field access

Field access uses `.`:

```zt
server.host
server.port
```

If the left side is a record and the field exists, the field value is returned.

If the field is declared optional and is absent, direct field access returns `none`:

```zt
raw.port
```

If `port` is absent, this evaluates to:

```zt
none
```

If the left side is `none`, direct field access is an error:

```zt
maybeServer.port
```

where `maybeServer: Server?` is invalid unless `maybeServer` is known not to be `none`.

### 14.2 Optional chaining

Optional chaining uses `?.`:

```zt
maybeServer?.port
```

Semantics:

```zt
x?.field
```

means:

```zt
match x {
  none => none;
  value => value.field;
}
```

Example:

```zt
let raw: {
  server? = {
    port? = Int;
  };
} = import "app.zti"

let port = raw.server?.port ?? 8080
```

If `raw.server` is absent, `raw.server` evaluates to `none`, then `?.port` also evaluates to `none`, and `?? 8080` supplies the default.

### 14.3 Optional chaining type rule

If:

```zt
x: T?
```

and:

```zt
T.field: U
```

then:

```zt
x?.field: U?
```

If `U` is already optional, `U?` is flattened to `U`.

So:

```zt
Server??
```

normalizes to:

```zt
Server?
```

---

