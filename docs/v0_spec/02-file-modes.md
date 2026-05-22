## 2. File modes

### 2.1 Immediate mode: `.zti`

A `.zti` file contains only pure data:

```zti
{
  name = "demo";
  profile = #prod;

  server = {
    host = "localhost";
    port = 8080;
    tls = false;
  };

  features = [
    #logging;
    #metrics;
  ];
}
```

A `.zti` file has:

* no imports
* no functions
* no conditionals
* no arithmetic
* no name resolution
* no type computation
* no evaluation

It is an inert serialized data tree.

### 2.2 General mode: `.zt`

A `.zt` file contains pure lazy computation:

```zt
let cfg = import "app.zti"

{
  name = cfg.name;
  profile = cfg.profile;
}
```

A `.zt` file consists of zero or more `let` bindings followed by one final expression:

```zt
let binding = expr
let binding: TypeExpr = expr

final_expr
```

The final expression is the file output.

If a `.zt` file is imported by another `.zt` file, its final value may contain records, lists, functions, or types.

If a `.zt` file is rendered as `.zti`, JSON, or another data format, its final value must be serializable.

---

