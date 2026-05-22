## 23. Type-level computation

Type-level computation uses the same pure expression language.

Example:

```zt
let Response: Type -> Type =
  fn Body => type {
    status = Int;
    body = Body?;
  }

let User: Type = type {
  id = Text;
  name = Text;
}

let UserResponse: Type = Response User
```

Zutai does not require the type-level language to be total.

Instead, it uses pragmatic compile-time evaluation with deterministic evaluator limits.

This is syntactically allowed:

```zt
let Loop: Type -> Type =
  fn T => Loop T

let Bad: Type = Loop Int
```

but it fails during type evaluation:

```text
error: type-level computation exceeded evaluation limit
```

Type-level programming is powerful and pure, but successful type checking requires type-level evaluation to terminate within compiler limits.

---

