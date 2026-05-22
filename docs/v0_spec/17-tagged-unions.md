## 17. Tagged unions

Record types and union types compose naturally.

```zt
let Shape: Type = type [
  {
    kind = #circle;
    radius = Float;
  };

  {
    kind = #rect;
    width = Float;
    height = Float;
  };
]
```

After `type [` each item is already a type-context expression: it is parsed as a type expression and checked to produce `Type`. Therefore record variants do not repeat `type`. The same rule applies to other type-context positions, such as annotations and operands of `->`.

Values:

```zt
let a: Shape = {
  kind = #circle;
  radius = 10.0;
}

let b: Shape = {
  kind = #rect;
  width = 20.0;
  height = 30.0;
}
```

Here:

```zt
kind = #circle;
```

inside a type record means the field `kind` must have the singleton atom type `#circle`.

This gives algebraic-data-type-like behavior without adding separate ADT syntax.

---

