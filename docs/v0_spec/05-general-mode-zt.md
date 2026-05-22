## 5. General mode `.zt`

### 5.1 File structure

A `.zt` file is:

```ebnf
file ::= let_binding* expr
```

Example:

```zt
let cfg = import "app.zti"
let name = cfg.name

{
  name = name;
  profile = cfg.profile;
}
```

The final expression is the file output.

### 5.2 One declaration form

There is exactly one declaration form:

```zt
let name = expr
let name: TypeExpr = expr
```

Examples:

```zt
let x = 42

let port: Int = 8080

let add: Int -> Int -> Int =
  fn x y => x + y

let Server: Type = type {
  host = Text;
  port = Int;
}
```

There is no separate syntax for:

```zt
type Server = ...
fn add ...
def add ...
class Server ...
```

Everything is a `let` binding.

### 5.3 One namespace

Zutai has one namespace.

The following is invalid:

```zt
let Server: Type = type {
  host = Text;
}

let Server = 123
```

The name `Server` is already bound.

Types, functions, modules, and runtime values all share the same namespace.

### 5.4 Binding scope

Top-level `let` bindings in a `.zt` file are in one recursive scope.

This allows functions to refer to themselves:

```zt
let factorial: Int -> Int =
  fn n =>
    if n <= 1 then 1 else n * factorial (n - 1)

factorial 5
```

It also allows mutually recursive top-level bindings, subject to type-checking and evaluation limits.

### 5.5 Immutability

Bindings are immutable.

There is no assignment statement and no mutation in the core language.

---

