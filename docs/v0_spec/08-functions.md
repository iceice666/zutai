## 8. Functions

### 8.1 Function expressions

Function values use `fn` and `=>`:

```zt
let add =
  fn x y => x + y
```

Annotated:

```zt
let add: Int -> Int -> Int =
  fn x y => x + y
```

### 8.2 Curried functions

Functions are curried by default.

```zt
let add: Int -> Int -> Int =
  fn x y => x + y
```

means `add` takes one `Int` and returns a function that takes another `Int`.

### 8.3 Function application

Function application uses whitespace:

```zt
add 1 2
normalizeServer raw.server
Pair Text Int
```

Application is left-associative:

```zt
f x y
```

means:

```zt
(f x) y
```

### 8.4 Function types

Function types use `->`:

```zt
Int -> Int -> Int
```

The operands of `->` are type-context expressions, so record and union type literals can be written without a repeated `type` prefix:

```zt
{ port = Int; ...; } -> Int
```

Function types are right-associative:

```zt
A -> B -> C
```

means:

```zt
A -> (B -> C)
```

Function expressions use `=>`.

Function types use `->`.

### 8.5 Pipeline operators

General mode supports pipeline operators as syntax for ordinary function application.

Forward pipeline:

```zt
x |> f
```

means:

```zt
f x
```

Backward pipeline:

```zt
f <| x
```

means:

```zt
f x
```

Pipelines are useful for chaining transformations:

```zt
raw
  |> normalize
  |> validate
  |> render
```

which means:

```zt
render (validate (normalize raw))
```

Pipelines do not perform implicit argument reordering beyond the single desugaring above.

Because functions are curried:

```zt
x |> f a
```

means:

```zt
(f a) x
```

not:

```zt
f x a
```

To place a value in a non-final position, use an explicit function:

```zt
x |> fn v => f v a
```

or choose argument order so that ordinary currying and `<|` compose naturally:

```zt
f a <| x
```

means:

```zt
f a x
```

### 8.6 No method-call rewrite in v0

The expression:

```zt
x.f
```

is field or module access, not method-call syntax.

There is no v0 rewrite from:

```zt
x.f y
```

to:

```zt
f x y
```

Use ordinary function application or pipelines instead:

```zt
f x y
x |> f
```
