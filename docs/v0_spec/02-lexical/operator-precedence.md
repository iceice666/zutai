## Operator precedence

From highest to lowest:

| Precedence | Operator / form                                                          | Associativity             |
| ---------: | ------------------------------------------------------------------------ | ------------------------- |
|          1 | field access `x.y`, optional chaining `x?.y`, postfix optional type `T?` | left / postfix            |
|          2 | function application `f x`                                               | left                      |
|          3 | `*`, `/`                                                                 | left                      |
|          4 | `+`, `-`                                                                 | left                      |
|          5 | comparison `==`, `!=`, `<`, `<=`, `>`, `>=`                              | non-associative           |
|          6 | `&&`                                                                     | left                      |
|          7 | `\|\|`                                                                   | left                      |
|          8 | defaulting `??`                                                          | right                     |
|          9 | pipeline `|>`, `<|`                                                      | `|>` left, `<|` right     |
|         10 | function type `->`                                                       | right                     |
|         11 | `if`, `match`, `\` bodies                                                | syntax-delimited          |

v0 has no unary operators: negation is part of a numeric literal (e.g. `-10`, `x * -1`).

Examples:

```zt
A -> B -> C
```

means:

```zt
A -> (B -> C)
```

```zt
f x.y
```

means:

```zt
f (x.y)
```

```zt
raw.server?.port ?? 8080
```

means:

```zt
(raw.server?.port) ?? 8080
```

```zt
x |> f a
```

means:

```zt
x |> (f a)
```

which desugars to:

```zt
(f a) x
```

```zt
f <| x ?? y
```

means:

```zt
f <| (x ?? y)
```

which desugars to:

```zt
f (x ?? y)
```

When `|>` and `<|` appear together without parentheses, implementations should reject the expression as ambiguous.
