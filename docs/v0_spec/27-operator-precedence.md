## 27. Operator precedence

From highest to lowest:

| Precedence | Operator / form                                                          | Associativity             |
| ---------: | ------------------------------------------------------------------------ | ------------------------- |
|          1 | field access `x.y`, optional chaining `x?.y`, postfix optional type `T?` | left / postfix            |
|          2 | function application `f x`                                               | left                      |
|          3 | unary operators                                                          | right                     |
|          4 | `*`, `/`                                                                 | left                      |
|          5 | `+`, `-`                                                                 | left                      |
|          6 | comparison `==`, `!=`, `<`, `<=`, `>`, `>=`                              | non-associative           |
|          7 | defaulting `??`                                                          | right                     |
|          8 | pipeline `|>`, `<|`                                                      | `|>` left, `<|` right     |
|          9 | function type `->`                                                       | right                     |
|         10 | `if`, `match`, `fn`, `select` bodies                                     | syntax-delimited          |

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
