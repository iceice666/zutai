## 7. Imports

Imports are expressions.

Canonical form:

```zt
let cfg = import "config.zti"
let lib = import "server.zt"
```

A shorthand unquoted import path may be accepted by implementations:

```zt
let cfg = import config.zti
```

However, the canonical syntax is the string form.

### 7.1 Importing `.zti`

```zt
import "file.zti"
```

parses the `.zti` file and returns inert data.

A `.zti` atom such as:

```zti
#prod
```

is represented in `.zt` with the same atom spelling:

```zt
#prod
```

### 7.2 Importing `.zt`

```zt
import "file.zt"
```

evaluates the imported `.zt` file and returns its final expression.

The returned value may contain records, lists, functions, or types.

### 7.3 Import purity

Imports are:

* pure
* deterministic
* path-relative
* cached

Re-importing the same resolved file path returns the same value.

---

