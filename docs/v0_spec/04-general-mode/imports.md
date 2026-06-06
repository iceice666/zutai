## Imports

Imports are expressions.

Canonical form:

```zt
cfg := import "config.zti"
lib := import "server.zt"
```

A shorthand unquoted import path may be accepted by implementations:

```zt
cfg := import config.zti
```

However, the canonical syntax is the string form.

### Importing `.zti`

```zt
import "file.zti"
```

parses the `.zti` file and returns the corresponding `.zt` data value. Blocks become records and arrays become lists. No `.zti` expression is evaluated, because immediate mode has only values.

A `.zti` atom such as:

```zti
#prod
```

is represented in `.zt` with the same atom spelling:

```zt
#prod
```

### Importing `.zt`

```zt
import "file.zt"
```

evaluates the imported `.zt` file and returns its final expression.

The returned value may contain records, lists, functions, or types.

### Import purity

Imports are:

* pure
* deterministic
* path-relative
* cached

Re-importing the same resolved file path returns the same value.

---
