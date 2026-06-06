## Pattern matching

Pattern matching uses `match` and `=>`. Each arm is introduced with `|`:

```zt
match profile {
  | #dev  => false;
  | #test => false;
  | #prod => true;
}
```

Optional matching:

```zt
match raw.port {
  | #none              => 8080;
  | (#some, value = port) => port;
}
```

Tuple union matching:

```zt
match shape {
  | (#circle, radius = r)          => r * r * 3.14159;
  | (#rect, width = w, height = h) => w * h;
}
```

### Exhaustiveness

For finite union types, `match` must be exhaustive.

Given:

```zt
Profile :: type [
  #dev;
  #test;
  #prod;
]
```

This is exhaustive:

```zt
match profile {
  | #dev  => false;
  | #test => false;
  | #prod => true;
}
```

This is invalid:

```zt
match profile {
  | #prod => true;
}
```

because `#dev` and `#test` are not handled.

A wildcard pattern `_` or a catch-all binding may be used:

```zt
match value {
  | #none              => fallback;
  | (#some, value = x) => x;
}
```

### Guard clauses

A pattern may include a guard condition using `if`:

```zt
match n {
  | x if x > 0 => #positive;
  | x if x < 0 => #negative;
  | _           => #zero;
}
```

The guard is evaluated only if the pattern matches. If the guard is false, the next clause is tried.

Guards also apply to tuple patterns:

```zt
match shape {
  | (#circle, radius = r) if r > 0.0 => r * r * 3.14159;
  | (#circle, radius = _)             => 0.0;
  | (#square, length = l)             => l * l;
}
```

### Nested patterns

Patterns may be nested to destructure composite values:

```zt
Response :: type [
  (#ok, body : Shape);
  (#err, message : Text);
]

match response {
  | (#ok, body = (#circle, radius = r)) => r * r * 3.14159;
  | (#ok, body = _)                     => 0.0;
  | (#err, message = _)                 => 0.0;
}
```

Nesting works for both tuple and atom patterns:

```zt
match config {
  | { profile = #prod; } => "production";
  | { profile = _;    }  => "non-production";
}
```

### Pattern matching in function clauses

Multi-clause function definitions use the same pattern language in `|` clauses:

```zt
describe_shape :: Shape -> Text
              | (#circle, radius = _)          => "circle"
              | (#square, length = _)          => "square"
              | (#rect, width = _, height = _) => "rect"
```

Guards in clauses:

```zt
classify :: Int -> Text
         | n if n > 0 => "positive"
         | n if n < 0 => "negative"
         | _          => "zero"
```

---
