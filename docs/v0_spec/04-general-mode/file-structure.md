## General mode `.zt`

### File structure

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

Top-level declarations are separated by line boundaries at delimiter depth zero. They do not use trailing semicolons.

### Declaration forms

There are three declaration forms.

**Inferred value binding** — type is inferred:

```zt
name := expr
```

**Typed value binding** — explicit type annotation:

```zt
name :: TypeExpr = expr
```

**Function or type definition** — uses `::` for the signature; `|` introduces each implementation clause:

```zt
name :: TypeSignature
     | pattern₁ pattern₂ => body
```

Type aliases use `:: type` and do not have implementation clauses:

```zt
Name :: type TypeExpr
```

Examples:

```zt
x := 42

port :: Int = 8080

add :: Int -> Int -> Int
    | a b => a + b

Server :: type { host : Text; port : Int; }
```

There is no separate syntax for:

```text
type Server = ...
def add ...
class Server ...
```

Everything is one of the three declaration forms.

### Function definitions

A named function consists of a type signature line followed by one or more `|` implementation clauses. The name appears once; clauses are indented under the signature:

```zt
factorial :: Int -> Int
          | 0 => 1
          | n => n * factorial (n - 1)
```

Multi-argument curried functions list all parameters in the clause:

```zt
add :: Int -> Int -> Int
    | a b => a + b
```

Pattern-matching multi-clause example:

```zt
unwrapOr :: <T> T? -> T -> T
         | #none              d => d
         | (#some, value = v) _ => v
```

The type signature is optional when the type can be inferred and only one clause is needed:

```zt
double x = x * 2
```

### Anonymous functions

Anonymous functions use `\` followed by space-separated patterns and `.` for the body:

```zt
map (\x. x * 2) items
fold (\acc x. acc + x) 0 items
```

Block form uses a block expression `{ stmts; expr }` when the body needs local bindings:

```zt
\acc x. {
  doubled := acc * 2;
  doubled + x
}
```

### One namespace

Zutai has one namespace.

The following is invalid:

```zt
Server :: type { host : Text; }

Server := 123
```

The name `Server` is already bound.

Types, functions, modules, and runtime values all share the same namespace.

### Binding scope

Top-level declarations in a `.zt` file are in one recursive scope.

This allows functions to refer to themselves:

```zt
factorial :: Int -> Int
          | 0 => 1
          | n => n * factorial (n - 1)

factorial 5
```

It also allows mutually recursive top-level bindings, subject to type-checking and evaluation limits.

### Local bindings

Inside function bodies, `:=` introduces a local immutable binding:

```zt
normalize :: RawServer -> Server
          | raw => {
            host := raw.host ?? "127.0.0.1";
            port := raw.port ?? 8080;
            tls  := raw.tls ?? false;
            { host = host; port = port; tls = tls; }
          }
```

A local binding is scoped to the remainder of the block.

### Immutability

All bindings are immutable.

There is no assignment statement and no mutation in the core language.

---
