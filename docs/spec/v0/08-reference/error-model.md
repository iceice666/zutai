## Error model

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
* import path escapes the project directory (absolute path or path traversal outside the importing file's directory subtree)
* import cycle that cannot be resolved lazily
* type-level evaluation limit exceeded
* serialization boundary violation

General-mode parse diagnostics should prefer specific syntax errors when the
parser can identify a common mistake:

* chained comparison
* mixed pipeline directions
* lambda arrow used instead of lambda dot
* lambda dot without required whitespace before the body
* missing list item semicolon
* missing block result expression
* value-record field written with `:`
* top-level typed binding written with single `:`
* type-record field written with `=`

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
