## 18. Polymorphism

Zutai uses explicit `forall` for polymorphic types.

```zt
let id: forall A. A -> A =
  fn x => x
```

Multiple type parameters:

```zt
let const: forall A B. A -> B -> A =
  fn x y => x
```

Example with optionals:

```zt
let unwrapOr: forall T. T? -> T -> T =
  fn value fallback => value ?? fallback
```

Type variables are capitalized:

```zt
A
B
T
Item
Body
```

### 18.1 Type-system model

The v0 type system is best described as:

> predicative explicit polymorphism with HM-style let generalization, bidirectional checking, row-polymorphic records, and bounded compile-time type evaluation.

It is HM-inspired, but not plain Hindley-Milner, because Zutai also has:

* first-class `Type`
* type-level functions
* arbitrary pure expressions in type annotations
* row-polymorphic records
* compile-time type normalization with deterministic evaluator limits

It is not full higher-order dependent type inference. Implementations should keep inference predictable and require annotations where inference would become ambiguous.

### 18.2 Instantiation

Polymorphic functions are implicitly instantiated at call sites.

Given:

```zt
let id: forall A. A -> A =
  fn x => x
```

these calls instantiate `A` differently:

```zt
id 1
id "hello"
id #prod
```

Explicit type application syntax is not part of v0. If it is added later, it must not conflict with ordinary function application or type-level function application.

### 18.3 Let generalization

Implementations may generalize let-bound expressions when all free type variables can be generalized safely.

For example, an implementation may infer:

```zt
let id = fn x => x
```

as:

```zt
forall A. A -> A
```

However, public APIs, exported module fields, and complex polymorphic functions should be annotated explicitly:

```zt
let mapList: forall A B. (A -> B) -> List A -> List B =
  fn f xs => ...
```

### 18.4 Predicativity and rank

v0 polymorphism is predicative.

A `forall` type may be instantiated with ordinary monotypes, including record types, union types, list types, and type-level function results that normalize to types.

v0 does not require impredicative instantiation, where a type variable is instantiated with another polymorphic `forall` type.

v0 also does not require higher-rank inference. Functions that accept polymorphic functions as arguments are reserved for a future version unless explicitly supported by an implementation extension.

### 18.5 Type equality and normalization

Type annotations are type expressions that evaluate to types. A type expression may contain arbitrary pure expressions, so type checking uses normalization of type-level expressions.

Example:

```zt
let Response: Type -> Type =
  fn Body => type {
    status = Int;
    body = Body?;
  }

let A: Type = Response Text
let B: Type = type { status = Int; body = Text?; }
```

`A` and `B` are the same type after type-level evaluation.

Type-level evaluation is pure and deterministic, but it is bounded by implementation limits. If normalization does not terminate within those limits, type checking fails with a deterministic error.

### 18.6 Row polymorphism

Record types are closed by default, but v0 also supports row-polymorphic record types.

An anonymous row tail creates an open record/view type:

```zt
type {
  host = Text;
  ...;
}
```

This means any record with at least `host: Text`.

A named row tail preserves the rest of the record:

```zt
type {
  host = Text;
  ...Rest;
}
```

Example:

```zt
let getHost: forall Rest. { host = Text; ...Rest; } -> Text =
  fn x => x.host
```

If the rest of the row does not need to be named, the shorter view type is preferred:

```zt
let getHost: { host = Text; ...; } -> Text =
  fn x => x.host
```

Row variables range over fields. They must not duplicate fields explicitly listed in the same record type.

### 18.7 Inference limits

Implementations should support straightforward first-order unification of type variables and row variables after type-level normalization.

Implementations are not required to infer:

* arbitrary higher-rank polymorphic types
* impredicative instantiations
* higher-order unification problems
* complex row constraints involving record update or field absence beyond duplicate-field checks

When inference is not principal or not obvious, implementations should ask for an explicit type annotation instead of guessing.
