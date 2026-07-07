# Zutai Language Manual

This is the user-facing guide to current Zutai.

The [v0 language specification](spec/v0/00-index.md) is normative for stable syntax; implemented v1-adjacent features are summarized here with support levels and linked to the [v1 deferred feature specification](spec/v1/00-index.md) and [current implementation status](ARCHIVED.md#current-baseline).

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
cfg ::= import "app.zti";
cfg.server.port
```

`zutai-cli run app.zt` prints `8080` when both files are in the same directory.

## File modes

Zutai has two file modes.

- `.zti` is immediate mode: inert serialized data. It has no imports, functions, conditionals, arithmetic, name resolution, type computation, or evaluation.
- `.zt` is general mode: pure, lazy, typed computation over data. A `.zt` file contains zero or more declarations followed by one final expression, and the final expression is the file output.

When `.zti` data is imported into `.zt`, `.zti` blocks become `.zt` records and `.zti` arrays become `.zt` lists.

## Real-program style

Larger programs usually pair inert `.zti` data with a typed `.zt`
transformation. Import `.zti` at a typed boundary, then keep the rest of the
program in named helpers with explicit input and output types. This gives row
checking enough information for record projections and keeps diagnostics close
to the helper that needs an annotation.

Short module aliases keep pipelines readable in example-sized programs:
`s ::= import stdlib.stream;`, `n ::= import stdlib.num;`,
`t ::= import stdlib.text;`, `r ::= import stdlib.result;`, and
`c ::= import stdlib.cmp;` are common local names. Larger opt-in surfaces often
use `l ::= import stdlib.list;`, `d ::= import stdlib.data;`,
`v ::= import stdlib.validate;`, `cfg ::= import stdlib.config;`, and
`refl ::= import stdlib.reflect;`. Prefer typed projection or
predicate helpers such as `isEnabled :: Service -> Bool = svc => svc.enabled;`
before using them in `map`, `filter`, or stream/list folds.

For nested conditionals inside expressions, use parentheses when the nesting
would otherwise be visually ambiguous:

```zt
else (if score >= 50 then #elevated else #steady)
```

## Lexical basics

In `.zt`, Unicode whitespace separates tokens. `;` is the universal terminator/separator: every value-like top-level declaration ends in `;`, and a trailing `;` on an expression makes it a `()` statement. (Clause-functions, constraint definitions, and witness definitions instead end at the final clause `;` or the closing `}`/`derive`.)

`.zt` comments are treated as whitespace and may contain UTF-8 text:

- Line comments begin with `--` and continue to the end of the line.
- Block comments begin with `--[` and end with `]--`; block comments may nest.
- Doc comments begin with `--|` and continue to the end of the line. In v0 they are lexically distinct but have no required semantic effect.

Canonical `.zti` v0 has no comments.

Strings are double-quoted and JSON-like. Immediate mode numbers use JSON-style syntax; general mode numbers use the same base syntax plus optional numeric postfixes such as `i8`, `u16`, `f32`, `f64`, `p32`, and `p64eN`. Without a postfix, integer-looking literals infer as `Int`, and literals with a decimal point or exponent infer as `Float`.

Atoms use `#` in both modes, for example `#prod` and `#x86_64-linux`. The reserved literals `true` and `false` are booleans, not atoms.

Binding identifiers and field names use Unicode UAX #31 XID letters/digits plus `_`, starting with an XID start scalar or `_`. Atoms use the same Unicode body shape and also allow `-`, for example `#x86_64-linux`; fields do not, so `cfg.target-triple` is subtraction, not one field access. Identifiers are compared by Unicode scalar sequence; no normalization is applied.

Type-valued bindings are uppercase, and runtime value bindings are lowercase. This is statically enforced: `Server :: type { ... };` is a type binding, while `server ::= ...;` is a runtime value binding.

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

A `.zti` file is pure data. Unicode whitespace may separate tokens outside
strings. The top-level form must be a block:

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
| `name ::= expr;` | Inferred top-level value binding. |
| `name :: TypeExpr = expr;` | Typed top-level value binding. |
| `name :: TypeSignature`<br>`= pattern => body;` | Function signature followed by one or more clauses. |
| `Name :: type TypeExpr;` | Type alias or named type expression. |

`;` is the universal terminator/separator, so each value-like top-level declaration ends in `;`; the trailing file-output expression takes no `;`. Clause-functions end at the final clause `;`.

Zutai has one namespace. Types, functions, modules, and runtime values cannot reuse a name.

Top-level declarations are in one recursive scope, so functions may refer to themselves and mutually recursive top-level bindings are allowed subject to type checking and evaluation limits. Local bindings appear inside a `[ … ]` do-block: `name := expr;` introduces an inferred immutable binding and `name : TypeExpr = expr;` a typed immutable binding; all bindings are immutable.

The container glyph picks the shape, and the scope picks the binding operator. A `{ … }` is a parallel container — a record when its first item is `field =`, otherwise a list of bare `;`-terminated expressions — while a `[ … ]` is a serial do-block:

| Shape | Parses as |
| --- | --- |
| `{}` | empty record value |
| `{ field = value; ... }` | record value |
| `{ field =; ... }` | record value with field-pun shorthand (`field = field;`) |
| `{ value; ... }` | list value (bare `;`-terminated items) |
| `{;}` | empty list value |
| `[ name := expr; final_expr ]` | do-block with inferred local binding |
| `[ name : TypeExpr = expr; final_expr ]` | do-block with typed local binding |
| `[]` | empty do-block |
| `type { field : TypeExpr; ... }` | record type (fields, not tags) |
| `type { #tag; #tag: TypeExpr; ... }` | tagged union type (members start with `#`) |

Examples:

```zt
{ x = 42; }
```

is a record with field `x`.

```zt
[ x := 42; x ]
```

is a do-block that binds local `x` and returns `x`.

```zt
[ x : Int = 42; x ]
```

is a do-block that binds typed local `x` and returns `x`.

## Values and expressions

Core value forms include booleans, text, numbers, atoms, records, tagged union values, tuples, and lists.

```zt
profile ::= #prod;

{
  ok = true;
  name = "demo";
  port = 8080;
  profile = profile;
  tags = { #logging; #metrics; };
}
```

Records use semicolon-terminated `field = value;` items. A field whose value is the identifier with the same name may omit the value: `{ host =; port =; }` is shorthand for `{ host = host; port = port; }`, and the same form works in record updates such as `cfg with { port =; }`. Lists are a `{ … }` parallel container of bare `;`-terminated items, distinguished from a record because the items have no `=`. Tuples use parentheses and comma-separated items; `(1, 2)` is a tuple, while `(x)` is grouping.

Tagged union values are atoms with optional payloads. A no-payload tag is a bare atom such as `#prod`. A record payload is written as an atom followed by a record, such as `#circle { radius = 5.0; }`.

Conditionals are expressions:

```text
if condition then expr else expr
```

The condition must have type `Bool`, and both branches must type-check to a compatible type.

`import` is a pure, deterministic, static, cached expression whose source is a
literal import source: either a quoted path (`cfg ::= import "config.zti"`) or a
dotted stdlib path (`{ map; fold; } ::= import stdlib.stream`). Importing `.zti`
parses data into `.zt` records and lists. Importing `.zt` evaluates the imported
module and exposes its final expression as the binding; fields are accessed as
`cfg.field` or `lib.Type`.

Function application uses whitespace and is left-associative: `f x y` means `(f x) y`. Functions are curried by default, so `add :: Int -> Int -> Int` takes one `Int` and returns a function `Int -> Int`. Lambdas use `\` and a spaced dot, for example `\x. x * 2`.

Pipelines are syntax for ordinary function application. `x |> f` means `f x`, and `f <| x` also means `f x`. Because functions are curried, `x |> f a` means `(f a) x`.

`x.f` is field or module access only; it is not method-call syntax.

General mode is pure and lazy. Unused bindings are not evaluated, and function arguments are lazy unless forced. Static external data enters through explicit `import` declarations. Runtime-selected `.zti` / `.zt` loading is not ambient: it is an explicit host effect via `loadZti path` / `loadZt path`, returning the first-order `Data` envelope and requiring the `load.zti` / `load.zt` operation in the surrounding effect row unless handled.

Effect rows can be factored through named effect type aliases. A closed
operation pack is usually written with `Unit` as its carrier, then spread inside
larger rows or wrapped in a result-position alias:

```zt
FsReadEffects :: type Unit ! { fs.read : Path -> Text; };
FsReadResult :: <A> type A ! { ...FsReadEffects; };

load :: Path -> FsReadResult Text
  = path => perform fs.read path;
```

## Types

Zutai has static typing with inference, explicit parametric generics, and first-class compile-time `Type` values. A value of type `Type` describes a type; type values can be bound, passed to type-level functions, imported from `.zt` modules, and used in annotations, but they are not serializable final outputs.

Ambient type values include `Type`, `Unit`, `Text`, `Bool`, `Int`, `Float`,
fixed-width integer and float names (`i8`, `i16`, `i32`, `i64`, `u8`, `u16`,
`u32`, `u64`, `f32`, `f64`), posit scalar names (`Posit32`, `Posit64`,
`Posit32eN`, `Posit64eN`), and the standard data/container type constructors
`List`, `Optional`, `Maybe`, `Patch`, and `DeepPatch`. The source prelude also
provides `Data`, `DataField`, `Stream`, `StreamEff`, and `Step`; `stdlib.data`
also exports its first-order `Data`/`DataField` envelope and structured
decoders explicitly. `Unit` is the empty-tuple type `()`, whose only value is
`()`.

`Data` is a first-order tagged envelope with scalar, list, record, and
tagged-value cases; `load.zt` rejects functions, `Type` values, witnesses, and
other non-serializable final values.

Annotations use `::`:

```zt
port :: Int = 8080;

port
```

Type aliases use `Name :: type TypeExpr;`. Generic aliases use `<...>`, for example `Pair :: <A, B> type { first : A; second : B; };`. Type functions may also be ordinary functions returning `Type`.

Record types are closed in v0. A value of a closed record type must provide the declared fields and must not provide undeclared fields. List types use `List T`.

Tagged union types use `type { ... }` with semicolon-terminated `#tag` members. The `:` colon after a tag name introduces a payload type; bare `#tag;` means no payload. Payload types may be record types or tuple types:

```zt
Action :: type {
  #quit;                             -- no payload
  #spawn: { command : Text; };       -- record payload
  #move: (Int, Int);                 -- tuple payload
};
```

The colon in the type declaration (`#tag: PayloadType`) is distinct from value construction, which uses no colon (`#spawn { command = "ghostty"; }`).

Every tagged union value exposes `.tag`, which returns the atom tag.

An empty list literal `{;}` cannot infer its element type; always provide an annotation: `items :: List T = {;};`.

Optional value and optional field syntax are distinct:

- `field : T?` means the field is required and contains `Optional T`.
- `field? : T` means the field may be physically absent and direct access returns `Maybe T`.
- `field? : T?` means the field may be absent, and if present contains `Optional T`; direct access returns `Maybe (Optional T)`.

Nested wrappers are not flattened. `?.` works on `Optional` and `Maybe`, preserving the receiver wrapper. `??` unwraps exactly one `Optional` or `Maybe` layer.

Structural equality is defined for first-order comparable data: booleans, numbers, text, atoms, lists, records, tuples, and tagged union values whose payloads are comparable. Functions are not comparable. `Type` values are not comparable in user code. Tagged union values with record payloads that are comparable are structurally equal when their tags and payloads are equal.

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
Profile :: type {#dev; #test; #prod;};

profile ::= #prod;

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

Rendered `.zti` outputs must be serializable. Functions and `Type` values have no `.zti` representation. Atoms keep their `#` spelling in `.zti`.

### JSON rendering

`eval_path_to_json` (the Rust evaluator API) serializes tagged union values as follows:

- A bare atom `#tag` with no payload renders as the JSON string `"#tag"` (the `#` prefix is preserved).
- A tagged value with a record payload `#tag { field = value; ... }` renders as a JSON object `{"tag": "tag", "payload": {...}}`. Note: the `tag` key holds the bare name without `#`.

```zt
Action :: type { #quit; #spawn: { command : Text; }; };

-- #quit renders as: "#quit"
-- #spawn { command = "ghostty"; } renders as: {"tag": "spawn", "payload": {"command": "ghostty"}}
```

Consumers of the JSON API must handle both shapes. Tagged union values do not have a direct `.zti` representation.

### Import paths

Quoted-string imports are relative to the importing file's directory. Absolute
paths and `..`-traversals that escape the importing file's directory are rejected
as a security boundary. This means a user config file cannot
`import "/etc/app/defaults.zt"`; layering must be managed at the host application
level, not by importing absolute paths. Dotted stdlib imports such as
`import stdlib.stream` are static literal import sources too, but they resolve to
embedded in-binary modules rather than the filesystem and are not subject to the
quoted-path subtree check.

## Implemented extensions beyond v0

These features are not v0 core. Their syntax is specified in the linked v1 or post-v0 pages; current implementation support is summarized here.

| Feature | User syntax | Support level |
| --- | --- | --- |
| [Row tails/open records/open unions](spec/v1/01-row-polymorphism.md) | `...` and `...Rest` in record/union types | parser, HIR, THIR, and TLC support row variables; non-principal row inference requires explicit annotations |
| [Selective projection](spec/v1/01-row-polymorphism.md#selective-projection) | `select value { field; }`, `value >>= { field; }`, `select TypeValue { field; }`, and `TypeValue >>= { field; }` | source-located checking; concrete value-level select lowers through Dataflow Core, ANF, SSA, and LLVM IR text |
| [Constraints/witnesses/derive](spec/v1/03-constraints.md) | `Constraint :: <A> @A { ... }`, `Constraint @Type :: { ... }`, and `derive` | THIR/TLC dictionary passing and the default evaluator support direct, bounded, conditional, imported, operator, method-level, and higher-kinded witnesses; native support covers direct/bounded/conditional/operator/method-level witnesses plus imported concrete and structurally matchable conditional witnesses. Higher-kinded execution and non-matchable cross-module witness exports remain check-only/native-gated |
| [Reflection](spec/v1/04-metaprogramming.md) | `fields T`, `variants T`, `schema T`, `witness C @T`, or explicit `refl.fields T` / `refl.schema T` through `import stdlib.reflect` | THIR/TLC/evaluator support; compile/dataflow fold supported reflection to backend values or reject residual reflection before lowering; importing `stdlib.reflect` alone does not trigger the backend gate |
| [Algebraic effects](spec/v1/05-effects.md) | `! { ... }`, named and qualified row spreads `...Effects` / `...m.Effects`, effect result aliases, `perform`/`!`, `handle`, `with`, and `resume`/`^` | TLC run supports handled effects; compile/dataflow lower supported handled effects and ambient `io.print` through runtime codegen while rejecting unsupported residual effects |
| [Record update](spec/v0/05-type-system/records.md#record-update) / [config overlay](stdlib/config.md) | `record with { field = value; }`; `defaults |> overlay patch`; `cfg.overlay patch base` through `import stdlib.config` | record update fully lowers through native codegen; supported full config-overlay calls over record-literal patches lower before Dataflow Core, including module-qualified/destructured `stdlib.config` aliases, while residual/partial overlay forms remain backend-gated |
| [`print`](spec/v1/05-effects.md) | `print text` and handled operation `io.print` | prelude compatibility binding; source handlers can intercept io.print and host `run`/compiled binaries dispatch residual io.print at runtime |
| Dynamic data loading | `loadZti path`, `loadZt path`, or `perform load.zti path` / `perform load.zt path` | explicit host capability/effect; `run` and compiled binaries dispatch runtime-selected `.zti`/`.zt` loads to a first-order `Data` envelope; handlers can intercept operations before the host boundary |
| [Stream-backed generators](v3_spec/01-generators.md) | `stream { yield expr; ... }`, tail `yield from`, and `Stream A` | `Stream A` is demand-driven **codata** (`Unit -> { #nil; #cons : { head; tail }; }`), not a memoizing lazy list; pure finite *and* infinite generators type-check and evaluate on both the interpreter and native backend. Tail `yield from` lowers; non-tail delegation is deliberately refused. Effectful generators (`yield perform op`) run under a granting handler; raw-cell generator cells for supported custom effects and ambient `io.print` lower natively through the `Computation` driver, while standard host operations such as `fs.read` lower through the host boundary under an explicit grant. Custom-effect lazy escapes still refuse when forced outside the grant. `finally` has native parity for supported handled-effect shapes; cooperative cancellation and the resource-lifetime contract remain interpreter-oracle behavior |

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
- import path escapes the project directory (absolute path or `..`-traversal outside the importing file's directory)
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
- trailing binary operator without a right operand
- field access written without a field name after `.` or `?.`
- local typed binding written with `::` instead of `:`
- tagged value payload written with `:`
- union variant payload written with `=`
- stale `type Name = ...` declaration syntax

## Further reading

- [v0 language specification](spec/v0/00-index.md)
- [v1 deferred feature specification](spec/v1/00-index.md)
- [standard library](stdlib/00-index.md)
- [implementation status](ARCHIVED.md)
