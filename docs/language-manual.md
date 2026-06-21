# Zutai Language Manual

This is the user-facing guide to current Zutai.

The [v0 language specification](v0_spec/00-index.md) is normative for stable syntax; implemented v1-adjacent features are summarized here with support levels and linked to the [v1 deferred feature specification](v1_spec/00-index.md) and [current implementation status](ARCHIVED.md#current-baseline).

## Quick start

Create `app.zti`:

```zti
{
  name = "demo";
  profile = #prod;

  server = {
    host = "localhost";
    port = 8080;
  };
}
```

Create `app.zt` in the same directory:

```zt
cfg :: import "app.zti"
cfg.server.port
```

`zutai-cli run app.zt` prints `8080` when both files are in the same directory.

## File modes

Zutai has two file modes.

- `.zti` is immediate mode: inert serialized data. It has no imports, functions, conditionals, arithmetic, name resolution, type computation, or evaluation.
- `.zt` is general mode: pure, lazy, typed computation over data. A `.zt` file contains zero or more declarations followed by one final expression, and the final expression is the file output.

When `.zti` data is imported into `.zt`, `.zti` blocks become `.zt` records and `.zti` arrays become `.zt` lists.

## Lexical basics

In `.zt`, whitespace separates tokens. Top-level declarations are separated by line boundaries at delimiter depth zero. A top-level declaration does not use a trailing semicolon.

`.zt` comments:

- Line comments begin with `--` and continue to the end of the line.
- Block comments begin with `--[` and end with `]--`; block comments may nest.
- Doc comments begin with `--|` and continue to the end of the line. In v0 they are lexically distinct but have no required semantic effect.

Canonical `.zti` v0 has no comments.

Strings are double-quoted and JSON-like. Immediate mode numbers use JSON-style syntax; general mode numbers use the same base syntax plus optional numeric postfixes such as `i8`, `u16`, `f32`, `f64`, `p32`, and `p64eN`. Without a postfix, integer-looking literals infer as `Int`, and literals with a decimal point or exponent infer as `Float`.

Atoms use `#` in both modes, for example `#prod` and `#x86_64-linux`. The reserved literals `true` and `false` are booleans, not atoms.

Binding identifiers and field names use letters, digits, and `_`, starting with a letter or `_`. Atoms also allow `-`, for example `#x86_64-linux`; fields do not, so `cfg.target-triple` is subtraction, not one field access.

Type-valued bindings are uppercase, and runtime value bindings are lowercase. This is statically enforced: `Server :: type { ... }` is a type binding, while `server := ...` is a runtime value binding.

Lambda-dot spacing is required: write `\x. y`, not `\x.y`.

Operator precedence, highest to lowest:

| Precedence | Operator / form                                                          | Associativity             |
| ---------: | ------------------------------------------------------------------------ | ------------------------- |
|          1 | field access `x.y`, optional chaining `x?.y`, postfix optional type `T?` | left / postfix            |
|          2 | function application `f x`                                               | left                      |
|          3 | `*`, `/`                                                                 | left                      |
|          4 | `+`, `-`                                                                 | left                      |
|          5 | comparison `==`, `!=`, `<`, `<=`, `>`, `>=`                              | non-associative           |
|          6 | `&&`                                                                     | left                      |
|          7 | `\|\|`                                                                 | left                      |
|          8 | defaulting `??`                                                          | right                     |
|          9 | pipeline `\|>`, `<\|`                                                   | `\|>` left, `<\|` right |
|         10 | function type `->`                                                       | right                     |
|         11 | `if`, `match`, `\` bodies                                               | syntax-delimited          |

`??` is right-associative. `|>` and `<|` must not mix without parentheses. v0 has no unary operators: negation is part of a numeric literal, such as `-10` or `x * -1`.

## Immediate mode `.zti`

A `.zti` file is pure data. The top-level form must be a block:

```zti
{
  name = "demo";
}
```

Each field uses `field = value;`. The trailing semicolon is required.

```zti
{
  host = "localhost";
  port = 8080;
}
```

Arrays contain semicolon-terminated items:

```text
[ value; ]
```

Inside a valid `.zti` file:

```zti
{
  features = [
    #logging;
    #metrics;
  ];
}
```

Duplicate keys in the same block are errors. There is no first-wins or last-wins rule.

Allowed values are `true`, `false`, atoms, strings, numbers, arrays, and blocks:

```zti
{
  enabled = true;
  disabled = false;
  mode = #prod;
  name = "demo";
  port = 8080;
  features = [
    #logging;
  ];
  server = {
    host = "localhost";
  };
}
```

Invalid examples:

```text
name = "demo";

[
  1;
]

{
  host = "localhost"
}

{
  port = 8080;
  port = 3000;
}
```

## General mode `.zt`

A `.zt` file has this shape:

```text
top_decl* final_expr
```

Top-level declaration forms:

| Form | Meaning |
| --- | --- |
| `name ::= expr` | Inferred top-level value binding. |
| `name :: TypeExpr = expr` | Typed top-level value binding. |
| `name :: TypeSignature`<br>`= pattern => body;` | Function signature followed by one or more clauses. |
| `Name :: type TypeExpr` | Type alias or named type expression. |

Top-level declarations are newline-separated at delimiter depth zero and do not use trailing semicolons. Function clauses use semicolons because they are clauses inside a declaration.

Zutai has one namespace. Types, functions, modules, and runtime values cannot reuse a name.

Top-level declarations are in one recursive scope, so functions may refer to themselves and mutually recursive top-level bindings are allowed subject to type checking and evaluation limits. Local block bindings use `name := expr;` for inferred immutable bindings and `name : TypeExpr = expr;` for typed immutable bindings; all bindings are immutable.

Brace syntax is disambiguated by the first item after `{`:

| Shape | Parses as |
| --- | --- |
| `{ field = value; ... }` | record value |
| `{ name := expr; final_expr }` | block with inferred local binding |
| `{ name : TypeExpr = expr; final_expr }` | block with typed local binding |
| `type { field : TypeExpr; ... }` | record type |

Examples:

```zt
{ x = 42; }
```

is a record with field `x`.

```zt
{ x := 42; x }
```

is a block that binds local `x` and returns `x`.

```zt
{ x : Int = 42; x }
```

is a block that binds typed local `x` and returns `x`.

## Values and expressions

Core value forms include booleans, text, numbers, atoms, records, tagged union values, tuples, and lists.

```zt
profile ::= #prod

{
  ok = true;
  name = "demo";
  port = 8080;
  profile = profile;
  tags = [#logging; #metrics;];
}
```

Records use semicolon-terminated fields. Lists use semicolon-terminated elements. Tuples use parentheses and comma-separated items; `(1, 2)` is a tuple, while `(x)` is grouping.

Tagged union values are atoms with optional payloads. A no-payload tag is a bare atom such as `#prod`. A record payload is written as an atom followed by a record, such as `#circle { radius = 5.0; }`.

Conditionals are expressions:

```text
if condition then expr else expr
```

The condition must have type `Bool`, and both branches must type-check to a compatible type.

Imports are pure, deterministic, path-relative, cached top-level declarations: `cfg :: import "config.zti"` creates one prefixed binding. Importing `.zti` parses data into `.zt` records and lists. Importing `.zt` evaluates the imported module and exposes its final expression as the binding; fields are accessed as `cfg.field` or `lib.Type`.

Function application uses whitespace and is left-associative: `f x y` means `(f x) y`. Functions are curried by default, so `add :: Int -> Int -> Int` takes one `Int` and returns a function `Int -> Int`. Lambdas use `\` and a spaced dot, for example `\x. x * 2`.

Pipelines are syntax for ordinary function application. `x |> f` means `f x`, and `f <| x` also means `f x`. Because functions are curried, `x |> f a` means `(f a) x`.

`x.f` is field or module access only; it is not method-call syntax.

General mode is pure and lazy. Unused bindings are not evaluated, and function arguments are lazy unless forced. External data enters through explicit static import declarations, not ambient `now`, `random`, filesystem, shell, environment primitives, or runtime `.zti` loading. Dynamic data loading belongs to a later explicit effect/capability design.

## Types

Zutai has static typing with inference, explicit parametric generics, and first-class compile-time `Type` values. A value of type `Type` describes a type; type values can be bound, passed to type-level functions, imported from `.zt` modules, and used in annotations, but they are not serializable final outputs.

Built-in type values include `Type`, `Text`, `Bool`, `Int`, `Float`, fixed-width integer and float names (`i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `f32`, `f64`), posit scalar names (`Posit32`, `Posit64`, `Posit32eN`, `Posit64eN`), and `List`.

Annotations use `::`:

```zt
port :: Int = 8080

port
```

Type aliases use `Name :: type TypeExpr`. Generic aliases use `<...>`, for example `Pair :: <A, B> type { first : A; second : B; }`. Type functions may also be ordinary functions returning `Type`.

Record types are closed in v0. A value of a closed record type must provide the declared fields and must not provide undeclared fields. List types use `List T`. Tagged union types use `type { ... }` with semicolon-terminated `#tag` members, optionally carrying record or tuple payloads. Every tagged union value exposes `.tag`, which returns the atom tag.

Optional value and optional field syntax are distinct:

- `field : T?` means the field is required and contains `Optional T`.
- `field? : T` means the field may be physically absent and direct access returns `Maybe T`.
- `field? : T?` means the field may be absent, and if present contains `Optional T`; direct access returns `Maybe (Optional T)`.

Nested wrappers are not flattened. `?.` works on `Optional` and `Maybe`, preserving the receiver wrapper. `??` unwraps exactly one `Optional` or `Maybe` layer.

Structural equality is defined for first-order comparable data: booleans, numbers, text, atoms, lists, records, tuples, and tagged union values whose payloads are comparable. Functions are not comparable. `Type` values are not comparable in user code. Tagged union values are comparable in `.zt` when their payloads are comparable, but they have no direct `.zti` or JSON representation in v0.

## Polymorphism and pattern matching

Polymorphic functions and types use a `<...>` type parameter list immediately after `::`. Type variables are capitalized.

```zt
id :: <A> A -> A
  = x => x;

id 1
```

Multiple type parameters are comma-separated, as in `<A, B>`. Polymorphic functions are implicitly instantiated at call sites. Explicit type application syntax is not part of v0.

Pattern matching uses `match`; each arm is introduced by `|` and uses `=>` for the body. For finite union types, `match` must be exhaustive. `_` is a wildcard pattern, and guards use `if` between the pattern and `=>`.

```zt
Profile :: type {#dev; #test; #prod;}

profile ::= #prod

match profile {
  | #dev  => false;
  | #test => false;
  | #prod => true;
}
```

Patterns can nest through records, tuples, tagged union payloads, and ordinary bindings. Function-clause pattern matching uses the same pattern language after a function signature:

```text
name :: TypeSignature
  = pattern if guard => body;
  = _                => fallback;
```

## Modules and serialization

A `.zt` module can return a record containing values, functions, and types. Imported `.zt` modules may contain non-serializable values such as functions and types; only rendering requires serializability.

Rendered `.zti` or JSON outputs must be serializable. Functions, `Type` values, function types, and tagged union values have no direct `.zti` or JSON representation in v0. Atoms keep their `#` spelling when rendered as `.zti`.

## Implemented extensions beyond v0

These features are not v0 core. Their syntax is specified in the linked v1 or post-v0 pages; current implementation support is summarized here.

| Feature | User syntax | Support level |
| --- | --- | --- |
| [Row tails/open records/open unions](v1_spec/01-row-polymorphism.md) | `...` and `...Rest` in record/union types | parser, HIR, THIR, and TLC support row variables; non-principal row inference requires explicit annotations |
| [Selective projection](v1_spec/01-row-polymorphism.md#selective-projection) | `select value { field; }` and `select TypeValue { field; }` | source-located checking; concrete value-level select lowers through Dataflow Core, ANF, SSA, and LLVM IR text |
| [Constraints/witnesses/derive](v1_spec/03-constraints.md) | `Constraint :: <A> @A { ... }`, `Constraint @Type :: { ... }`, and `derive` | THIR/TLC dictionary passing and the default evaluator support direct, bounded, conditional, imported, operator, method-level, and higher-kinded witnesses; do not claim full native-backend parity |
| [Reflection](v1_spec/04-metaprogramming.md) | `fields T` and `schema T` | THIR/type-value evaluator support; compile/dataflow reject reflection builtins until lowered to ordinary backend values |
| [Algebraic effects](v1_spec/05-effects.md) | `! { ... }`, `perform`, `handle`, `with`, and `resume` | TLC run supports handled effects and host run handles residual io.print; compile/dataflow reject residual effect markers and non-empty function effect rows |
| [Record update](v0_spec/05-type-system/records.md#record-update) / [config overlay](stdlib/config.md) | `record with { field = value; }` | post-v0 core expression for strict, non-extending, non-deleting replacement; config overlay remains a standard-library policy, not record syntax |
| [`print`](v1_spec/05-effects.md) | `print text` and handled operation `io.print` | prelude compatibility binding; source handlers can intercept io.print and the host run boundary handles residual io.print |

## Errors and diagnostics

Implementations should report deterministic, source-located errors. User-visible error classes include:

- lexical error
- parse error
- duplicate `.zti` key
- unknown identifier
- unknown field
- duplicate binding
- type mismatch
- non-exhaustive match
- invalid import path
- unresolvable import cycle
- type-level evaluation limit
- serialization boundary violation

Common parse-diagnostic mistakes should receive specific messages when the parser can identify them:

- chained comparison
- mixed pipeline directions
- lambda arrow used instead of lambda dot
- lambda dot without required whitespace before the body
- missing list item semicolon
- missing block result expression
- value-record field written with `:`
- top-level typed binding written with single `:`
- type-record field written with `=`

## Further reading

- [v0 language specification](v0_spec/00-index.md)
- [v1 deferred feature specification](v1_spec/00-index.md)
- [standard library](stdlib/00-index.md)
- [implementation status](ARCHIVED.md)
