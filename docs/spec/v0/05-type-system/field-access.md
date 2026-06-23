## Field access and optional chaining

### Required field access

Field access uses `.`:

```zt
server.host
server.port
```

If the left side is a record and the field exists, the field value is returned.

If the field is declared optional, direct field access returns a `Maybe` value:

```zt
raw.port
```

If `port` is absent, this evaluates to:

```zt
#absent
```

If `port` is present with value `8080`, this evaluates to:

```zt
#present (8080)
```

If the left side is `#none` or `#absent`, direct field access is an error:

```zt
maybeServer.port
```

where `maybeServer: Server?` is invalid unless `maybeServer` is known to be a `#some` value.

### Optional chaining

Optional chaining uses `?.` on either `Optional` or `Maybe` receivers:

```zt
maybeServer?.port
maybeField?.port
```

For an `Optional` receiver:

```zt
match x {
  | #none       => #none;
  | #some (r)   => #some (r.field);
}
```

For a `Maybe` receiver:

```zt
match x {
  | #absent      => #absent;
  | #present (r) => #present (r.field);
}
```

The projected `r.field` uses direct field-access semantics. If `field` is optional, `r.field` is `Maybe T`, so chaining preserves nested wrappers instead of flattening them.

Example:

```zt
raw :: import "app.zti"

port ::= raw.server?.port ?? #absent
```

If `raw.server` is absent, `raw.server` evaluates to `#absent`, then `?.port` also evaluates to `#absent`. If `raw.server` is present but `port` is absent, the result is `#present (#absent)`.

### Optional chaining type rule

If:

```zt
x: Optional T
```

and:

```zt
T.field: U
```

then:

```zt
x?.field: Optional U
```

If:

```zt
x: Maybe T
```

then:

```zt
x?.field: Maybe U
```

No flattening is applied. If `U` is already `Optional V` or `Maybe V`, the nested wrapper remains.

When the accessed field is declared optional (`field? : T?`), direct projection has type `Maybe (Optional T)`, so `x?.field` has type `Optional (Maybe (Optional T))` for `Optional` receivers and `Maybe (Maybe (Optional T))` for `Maybe` receivers.

---
