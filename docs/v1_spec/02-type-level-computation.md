# Type-Level Computation (v1)

These features extend the v0 type system to treat types as first-class compile-time values. They are deferred from v0 because they significantly complicate the type checker.

---

## First-Class `Type` Values

In v1, types are first-class compile-time values. A binding annotated `: Type` holds a type:

```zt
Server :: type {
  host : Text;
  port : Int;
}
```

Type constructors are functions that return `Type`:

```zt
Pair :: Type -> Type -> Type
    :: A -> B { type {
    first : A;
    second : B;
  } }
```

Usage:

```zt
TextIntPair :: type Pair Text Int

pair : TextIntPair = {
  first = "hello";
  second = 42;
}
```

In v0, the equivalent is written as a generic type alias with `[A, B]` syntax without `Type -> Type` function signatures.

---

## Type Annotations as Expressions

When types are first-class, type annotations are type expressions that evaluate to types:

```zt
port : Int = 8080
```

A `type_expr` may contain arbitrary pure expressions that evaluate to `Type`. It is also a type context: bare `{ ... }` and `[ ... ]` are parsed as record and union type literals without a repeated `type` keyword.

Example:

```zt
Response :: Type -> Type
    :: Body { type {
    status : Int;
    body : Body?;
  } }

value : Response Text = {
  status = 200;
  body = "ok";
}
```

---

## Type-Level Computation

Type-level computation uses the same pure expression language.

Example:

```zt
Response :: Type -> Type
   :: Body { type {
    status : Int;
    body : Body?;
  } }

User :: type {
  id : Text;
  name : Text;
}

UserResponse : Type = Response User
```

Zutai does not require the type-level language to be total.

Instead, it uses pragmatic compile-time evaluation with deterministic evaluator limits.

This is syntactically allowed:

```zt
Loop :: Type -> Type
   :: T { Loop T }

Bad : Type = Loop Int
```

but it fails during type evaluation:

```text
error: type-level computation exceeded evaluation limit
```

Type-level programming is powerful and pure, but successful type checking requires type-level evaluation to terminate within compiler limits.

---

## Type Equality and Normalization

Type annotations are type expressions that evaluate to types. A type expression may contain arbitrary pure expressions, so type checking uses normalization of type-level expressions.

Example:

```zt
Response :: Type -> Type
         :: Body { type {
             status : Int;
             body : Body?;
           } }

A : Type = Response Text
B :: type { status : Int; body : Text?; }
```

`A` and `B` are the same type after type-level evaluation.

Type-level evaluation is pure and deterministic, but it is bounded by implementation limits. If normalization does not terminate within those limits, type checking fails with a deterministic error.

---

## Universe Levels

The surface language exposes `Type`.

The implementation should use internal universe levels to avoid literal unsoundness such as `Type : Type`.

Conceptually, users write:

```zt
Int: Type
Text: Type
Server: Type
```

Internally, the implementation may model this as:

```text
Type0 : Type1
Type1 : Type2
Type2 : Type3
```

The user normally writes only `Type`.
