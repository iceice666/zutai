## Conditionals

General mode supports expression conditionals:

```zt
if condition then expr else expr
```

Example:

```zt
port :=
  if profile == #prod then 443 else 8080
```

The condition must have type `Bool`.

Both branches must type-check to a compatible type.

---
