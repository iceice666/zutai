## Field access and optional chaining

### Required field access

Field access uses `.`:

```zt
server.host
server.port
```

If the left side is a record and the field exists, the field value is returned.

If the field is declared optional, direct field access returns an optional value:

```zt
raw.port
```

If `port` is absent, this evaluates to:

```zt
#none
```

If `port` is present with value `8080`, this evaluates to:

```zt
#some { value = 8080; }
```

If the left side is `#none`, direct field access is an error:

```zt
maybeServer.port
```

where `maybeServer: Server?` is invalid unless `maybeServer` is known to be a `#some` value.

### Optional chaining

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
  | #none                    => #none;
  | #some { value = value; } => optionalWrap(value.field);
}
```

`optionalWrap` is a specification helper, not a user-visible function. It returns an already-optional value unchanged; otherwise it wraps the value as `#some { value = result; }`.

Example:

```zt
raw : {
  server? : {
    port? : Int;
  };
} = import "app.zti"

port := raw.server?.port ?? 8080
```

If `raw.server` is absent, `raw.server` evaluates to `#none`, then `?.port` also evaluates to `#none`, and `?? 8080` supplies the default.

### Optional chaining type rule

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
(Server?)?
```

normalizes to:

```zt
Server?
```

Note: `??` is always the defaulting token, so double-postfix optional must be parenthesized as `(T?)?`.

When the accessed field is declared optional (`field? : T?`), this same flattening applies to collapse the field-absence layer and the value-optional layer into a single `T?`. See [Optional fields](optional-fields.md) for the concrete absent/present semantics.

---
