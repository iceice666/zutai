## Polymorphism

Zutai uses a `<...>` type parameter list for polymorphic functions and types, placed immediately after `::`:

```zt
id :: <A> A -> A {
  | x => x;
}
```

Multiple type parameters are comma-separated:

```zt
const :: <A, B> A -> B -> A {
  | x _ => x;
}
```

Example with optionals:

```zt
unwrapOr :: <T> T? -> T -> T {
  | #none              fallback => fallback;
  | #some { value = v } _        => v;
}
```

Type variables are capitalized:

```zt
A
B
T
Item
Body
```

### Type-system model

The v0 type system is predicative explicit polymorphism with HM-style let generalization and bidirectional checking.

Type parameters are unconstrained in v0. A constrained type parameter (e.g., `<A: Eq>`) requires the constraint system, which is a v1 feature. See [Constraints](../../v1_spec/03-constraints.md).

Row-polymorphic records and open union types are also v1 features. See [Row polymorphism](../../v1_spec/01-row-polymorphism.md).

### Instantiation

Polymorphic functions are implicitly instantiated at call sites.

Given:

```zt
id :: <A> A -> A {
  | x => x;
}
```

these calls instantiate `A` differently:

```zt
id 1
id "hello"
id #prod
```

Explicit type application syntax is not part of v0. If it is added later, it must not conflict with ordinary function application.

### Let generalization

Implementations may generalize let-bound expressions when all free type variables can be generalized safely.

For example, an implementation may infer:

```zt
id x = x
```

as `<A> A -> A`.

However, public APIs, exported module fields, and complex polymorphic functions should be annotated explicitly:

```zt
mapList :: <A, B> (A -> B) -> List A -> List B {
  | f xs => mapListImpl f xs;
}
```
