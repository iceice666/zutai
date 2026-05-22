## 19. Pattern matching

Pattern matching uses `match` and `=>`:

```zt
match profile {
  #dev => false;
  #test => false;
  #prod => true;
}
```

Optional matching:

```zt
match raw.port {
  none => 8080;
  port => port;
}
```

Tagged matching:

```zt
match shape.kind {
  #circle => shape.radius * shape.radius * 3.14159;
  #rect => shape.width * shape.height;
}
```

### 19.1 Exhaustiveness

For finite union types, `match` must be exhaustive.

Given:

```zt
let Profile: Type = type [
  #dev;
  #test;
  #prod;
]
```

This is exhaustive:

```zt
match profile {
  #dev => false;
  #test => false;
  #prod => true;
}
```

This is invalid:

```zt
match profile {
  #prod => true;
}
```

because `#dev` and `#test` are not handled.

A catch-all pattern may be used where appropriate:

```zt
match value {
  none => fallback;
  x => x;
}
```

---

