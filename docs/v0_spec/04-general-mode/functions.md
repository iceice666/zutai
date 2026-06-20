## Functions

### Named function definitions

Named functions use `::` for the type signature followed by one or more `=` clauses.

```zt
add :: Int -> Int -> Int
  = a b => a + b;
```

Multi-clause definitions provide pattern matching directly in the function:

```zt
factorial :: Int -> Int
  = 0 => 1;
  = n => n * factorial (n - 1);

describe :: Bool -> Text
  = true  => "yes";
  = false => "no";
```

The type signature is required when using explicit clauses. Without a signature, write a single no-sig definition instead (see below).

Guard conditions use `if` between the pattern and `=>`:

```zt
classify :: Int -> Text
  = n if n > 0 => "positive";
  = n if n < 0 => "negative";
  = _          => "zero";
```

When a clause body requires local bindings, use a block expression `{ stmts; expr }` as the clause value:

```zt
normalizeServer :: RawServer -> Server
  = s => {
    host := s.host ?? "127.0.0.1";
    port := s.port ?? 8080;
    tls  := s.tls  ?? false;
    { host = host; port = port; tls = tls; }
  };
```

### No-sig single definitions

When the type is fully inferable and only one clause is needed, write a definition without a signature or `|`:

```zt
add a b = a + b
double x = x * 2
```

This form does not support multiple clauses or guards. For those, write a `::` signature and use `=` clauses.

### Typed constants

Constants (zero-argument bindings) use `::` for the type annotation:

```zt
port :: Int = 8080
host :: Text = "localhost"
```

For inferred constants, use `:=`:

```zt
port := 8080
```

### Curried functions

Functions are curried by default.

```zt
add :: Int -> Int -> Int
  = a b => a + b;
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

`f x y` means `(f x) y`.

### Function types

Function types use `->` and are right-associative:

```zt
Int -> Int -> Int
{ port : Int } -> Int
```

Polymorphic signatures put the type parameter list `<...>` immediately after `::`:

```zt
id :: <A> A -> A
  = x => x;
```

`A -> B -> C` means `A -> (B -> C)`.

In type-context positions, `{ field : Type; }` and `{ #tag; }` are parsed as record and union type literals without repeating the `type` keyword.

### Anonymous functions

Anonymous functions use `\` followed by space-separated patterns and `.` for the body:

```zt
\x. x * 2
\x y. x + y
```

When the body needs local bindings, use a block expression:

```zt
\acc x. {
  doubled := acc * 2;
  doubled + x
}
```

Examples:

```zt
map    (\x. x * 2) items
filter (\x. x > 0) items
fold   (\acc x. acc + x) 0 items
```

The lambda dot must be surrounded by whitespace from the last pattern. `\x.y` with no space is a parse error; write `\x. y`.

### Pipeline operators

General mode supports pipeline operators as syntax for ordinary function application.

Forward pipeline:

```zt
x |> f
```

means `f x`. Backward pipeline:

```zt
f <| x
```

means `f x`. Pipelines are useful for chaining transformations:

```zt
raw
  |> normalize
  |> validate
  |> render
```

which means `render (validate (normalize raw))`.

Because functions are curried, `x |> f a` means `(f a) x`. To place a value in a non-final position use an explicit lambda or `<|`:

```zt
x |> \v. f v a
f a <| x
```

### No method-call rewrite in v0

`x.f` is field or module access, not method-call syntax. Use ordinary function application or pipelines instead:

```zt
f x y
x |> f
```
