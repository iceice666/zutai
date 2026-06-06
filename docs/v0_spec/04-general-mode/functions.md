## Functions

### Named function definitions

Named functions use `::` for both the type signature and implementation clauses. The type signature line gives the full type; each clause line gives patterns and a body block.

```zt
add :: Int -> Int -> Int
    :: a -> b { a + b }
```

The `->` between clause patterns corresponds to the `->` in the type signature — one pattern per arrow.

Function clauses are introduced by `::` and are not semicolon-terminated.

Multi-clause definitions provide pattern matching directly in the function:

```zt
factorial :: Int -> Int
          :: 0 { 1 }
          :: n { n * factorial (n - 1) }

describe :: Bool -> Text
         :: true  { "yes" }
         :: false { "no" }
```

The type signature is optional when the type can be inferred:

```zt
negate :: x { x * -1 }
```

### Curried functions

Functions are curried by default.

```zt
add :: Int -> Int -> Int
    :: a -> b { a + b }
```

`add` takes one `Int` and returns a function `Int -> Int`. Partial application:

```zt
add5 := add 5
```

### Function application

Function application uses whitespace and is left-associative:

```zt
add 1 2
normalizeServer raw.server
Pair Text Int
```

```zt
f x y
```

means:

```zt
(f x) y
```

### Function types

Function types use `->` and are right-associative:

```zt
Int -> Int -> Int
{ port : Int } -> Int
```

Polymorphic signatures put the type parameter list immediately after `::`:

```zt
id :: [A] A -> A
```

```zt
A -> B -> C
```

means:

```zt
A -> (B -> C)
```

In type-context positions, `{ ... }` and `[ ... ]` are parsed as record and union type literals without repeating the `type` keyword.

Function expressions use `=>` or `{}`.
Function types use `->`.

### Anonymous functions

Anonymous functions use `\` followed by space-separated patterns and `=>` for the body:

```zt
\x => x * 2
\x y => x + y
```

Block form with `{}` when the body needs local bindings:

```zt
\x { x * 2 }
\acc x {
  doubled := acc * 2;
  doubled + x
}
```

Examples:

```zt
map    (\x => x * 2) items
filter (\x => x > 0) items
fold   (\acc x => acc + x) 0 items
```

### Pipeline operators

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
x |> \v => f v a
```

or choose argument order so that ordinary currying and `<|` compose naturally:

```zt
f a <| x
```

means:

```zt
f a x
```

### No method-call rewrite in v0

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
