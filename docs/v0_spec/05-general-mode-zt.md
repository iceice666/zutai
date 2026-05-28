## 5. General mode `.zt`

### 5.1 File structure

A `.zt` file is:

```ebnf
file ::= top_decl* expr
```

Example:

```zt
cfg := import "app.zti"
name := cfg.name

{
  name = name;
  profile = cfg.profile;
}
```

The final expression is the file output.

### 5.2 Declaration forms

There are three declaration forms.

**Inferred value binding** — type is inferred:

```zt
name := expr
```

**Annotated value binding** — explicit type annotation, `:=` is shorthand for `: <type> =` with the type omitted:

```zt
name : TypeExpr = expr
```

**Function or type definition** — uses `::` for the type signature and one or more `::` clauses for implementation:

```zt
name :: TypeSignature
     :: pattern₁ -> pattern₂ { body }
```

Examples:

```zt
x := 42

port : Int = 8080

add :: Int -> Int -> Int
    :: a -> b { a + b }

Server :: type { host : Text; port : Int; }
```

There is no separate syntax for:

```zt
type Server = ...
def add ...
class Server ...
```

Everything is one of the three declaration forms.

### 5.3 Function definitions

A named function consists of a type signature line followed by one or more implementation clauses. Both use `::`. The `->` between clause patterns mirrors the `->` in the type signature — one pattern per arrow:

```zt
factorial :: Int -> Int
          :: 0 { 1 }
          :: n { n * factorial (n - 1) }
```

Multi-argument curried functions:

```zt
add :: Int -> Int -> Int
    :: a -> b { a + b }
```

Pattern-matching multi-clause example:

```zt
unwrap_or_default :: forall T. T? -> T -> T
                  :: (#some, v) -> _ { v }
                  :: #none -> d      { d }
```

The type signature is optional when the type can be inferred:

```zt
double :: a { a * 2 }
```

### 5.4 Anonymous functions

Anonymous functions use `\` followed by space-separated patterns and `=>` for the body:

```zt
map (\x => x * 2) items
fold (\acc x => acc + x) 0 items
```

Block form uses `{}` when the body needs local bindings:

```zt
\x { x * 2 }
\acc x {
  doubled := acc * 2;
  doubled + x
}
```

### 5.5 One namespace

Zutai has one namespace.

The following is invalid:

```zt
Server :: type { host : Text; }

Server := 123
```

The name `Server` is already bound.

Types, functions, modules, and runtime values all share the same namespace.

### 5.6 Binding scope

Top-level declarations in a `.zt` file are in one recursive scope.

This allows functions to refer to themselves:

```zt
factorial :: Int -> Int
          :: 0 { 1 }
          :: n { n * factorial (n - 1) }

factorial 5
```

It also allows mutually recursive top-level bindings, subject to type-checking and evaluation limits.

### 5.7 Local bindings

Inside function bodies, `:=` introduces a local immutable binding:

```zt
normalize :: RawServer -> Server
          :: raw {
            host := raw.host ?? "127.0.0.1";
            port := raw.port ?? 8080;
            tls  := raw.tls ?? false;
            { host = host; port = port; tls = tls; }
          }
```

A local binding is scoped to the remainder of the block.

### 5.8 Immutability

All bindings are immutable.

There is no assignment statement and no mutation in the core language.

---
