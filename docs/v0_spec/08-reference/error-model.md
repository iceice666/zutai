## 28. Error model

Implementations should report deterministic, source-located errors.

Important error classes:

* lexical error
* parse error
* duplicate key error in `.zti`
* unknown identifier
* unknown field
* duplicate binding
* type mismatch
* non-exhaustive match
* invalid import path
* import cycle that cannot be resolved lazily
* type-level evaluation limit exceeded
* serialization boundary violation

Examples:

```text
error: duplicate key `port`
  --> config.zti:3:3
```

```text
error: expected `Int`, found `Text`
  --> app.zt:12:10
```

```text
error: type-level computation exceeded evaluation limit
  --> types.zt:8:17
```

---

