## General mode `.zt`

### File structure

A `.zt` file is:

```ebnf
file ::= top_decl* expr
```

Example:

```zt
cfg ::= import "app.zti";
name ::= cfg.name;

{
  name = name;
  profile = cfg.profile;
}
```

The final expression is the file output.

`;` is the universal terminator/separator: every value-like top-level declaration ends in `;`. The trailing file-output expression takes no `;`; a trailing `;` (or no tail at all) makes the file's value `()`. (Clause-functions, constraint definitions, and witness definitions are the exceptions: they end at the final clause `;` or the closing `}`/`derive`.)

### Declaration forms

There are six core declaration forms.

**Inferred value binding** — type is inferred:

```zt
name ::= expr;
```

**Typed value binding** — explicit type annotation:

```zt
name :: TypeExpr = expr;
```

**Import binding** — `import` is an expression with a literal source, so a static module/data import is an ordinary inferred binding:

```zt
name ::= import "path.zti";
```

This creates one prefixed binding; imported fields are accessed through it, for example `name.field` or `name.Type`. Members can also be destructured directly: `{ field; } ::= import "path.zt";`.

**Grouped static imports** — `use` expands to ordinary inferred import bindings:

```zt
use stdlib {
  stream as s;
  num as n;
  text as t;
}
```

is equivalent to:

```zt
s ::= import stdlib.stream;
n ::= import stdlib.num;
t ::= import stdlib.text;
```

**Function definition** — uses `::` for the signature, followed by one or more `=` clauses:

```zt
name :: TypeSignature
  = pattern₁ pattern₂ => body;
```

Type aliases use `:: type` and do not have implementation clauses:

```zt
Name :: type TypeExpr;
```

Examples:

```zt
x ::= 42;

port :: Int = 8080;

cfg ::= import "app.zti";

add :: Int -> Int -> Int
  = a b => a + b;

Server :: type { host : Text; port : Int; };
```

There is no separate syntax for:

```text
type Server = ...
def add ...
class Server ...
```

Every top-level binding is one of these forms. `import` itself is an expression (with a literal source), so it appears as the right-hand side of a binding rather than as a dedicated declaration form.

### Function definitions

A named function consists of a type signature followed by one or more `=` clauses:

```zt
factorial :: Int -> Int
  = 0 => 1;
  = n => n * factorial (n - 1);
```

Multi-argument curried functions list all parameters in the clause:

```zt
add :: Int -> Int -> Int
  = a b => a + b;
```

Pattern-matching multi-clause example:

```zt
unwrapOr :: <T> T? -> T -> T
  = #none       d => d;
  = #some (v)   _ => v;
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

Do-block form uses a `[ stmts; expr ]` block when the body needs local bindings:

```zt
\acc x. [
  doubled := acc * 2;
  doubled + x
]
```

### One namespace

Zutai has one namespace.

The following is invalid:

```zt
Server :: type { host : Text; };

Server ::= 123;
```

The name `Server` is already bound.

Types, functions, modules, and runtime values all share the same namespace.

### Binding scope

Top-level declarations in a `.zt` file are in one recursive scope.

This allows functions to refer to themselves:

```zt
factorial :: Int -> Int
  = 0 => 1;
  = n => n * factorial (n - 1);

factorial 5
```

It also allows mutually recursive top-level bindings, subject to type-checking and evaluation limits.

### Local bindings

Inside function bodies, `name := expr;` introduces an inferred local immutable
binding, and `name : TypeExpr = expr;` introduces a typed local immutable binding:

```zt
normalize :: RawServer -> Server
  = raw => [
    host : Text = raw.host ?? "127.0.0.1";
    port : Int = raw.port ?? 8080;
    tls  := raw.tls ?? false;
    { host = host; port = port; tls = tls; }
  ];
```

A local binding is scoped to the remainder of the do-block.

### Immutability

All bindings are immutable.

There is no assignment statement and no mutation in the core language.

---
