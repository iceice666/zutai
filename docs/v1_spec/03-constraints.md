# Constraints (v1)

Constraints are named behavioral interfaces over types. They enable polymorphic functions to require specific operations from their type parameters, and allow ad-hoc polymorphism over types that row polymorphism cannot express — including primitives like `Int` and `Text`, and cross-type relationships.

---

## Constraint Definitions

A constraint is declared with `::`, a type parameter list `[...]`, a witness target `@T`, and a body of method signatures:

```zt
Eq :: [A] @A {
  eq :: A -> A -> Bool;
}
```

The `[A]` after `::` declares the type parameter. `@A` marks which type this constraint is "about" — the witness target. Method signatures inside the body use `::` (not `:`, which is for record field types).

Operators are declared with parenthesised names, mirroring their call syntax:

```zt
Ord :: [A: Eq] @A {
  compare :: A -> A -> Ordering;
  (<)     :: A -> A -> Bool;
  (<=)    :: A -> A -> Bool;
  (>)     :: A -> A -> Bool;
  (>=)    :: A -> A -> Bool;
  max     :: A -> A -> A;
  min     :: A -> A -> A;
}
```

---

## Superconstraints

A constraint may require other constraints on its type parameter via bounds in `[...]`:

```zt
Ord :: [A: Eq] @A {
  compare :: A -> A -> Ordering;
}
```

This means: to provide a witness for `Ord A`, the type `A` must already have a witness for `Eq`.

Multiple bounds on one parameter use `+`:

```zt
Hash :: [A: Eq + Show] @A {
  hash :: A -> Int;
}
```

---

## Default Methods

A method may have a default implementation. Mark it optional with `?` and follow immediately with an anonymous `::` clause providing the default:

```zt
Ord :: [A: Eq] @A {
  compare :: A -> A -> Ordering;

  max? :: A -> A -> A;
       :: a -> b { if a >= b then a else b }

  min? :: A -> A -> A;
       :: a -> b { if a <= b then a else b }
}
```

A witness that omits an optional method receives the default. A witness that provides the method overrides it.

---

## Witnesses

A witness provides implementations for a constraint on a specific type. The `@T` pinning comes before `::`:

```zt
Eq @Int :: {
  eq = \a b => a == b;
}
```

Operator methods use the same parenthesised names as in the constraint definition:

```zt
Ord @Int :: {
  compare = compareInt;
  (<)     = \a b => a < b;
  (<=)    = \a b => a <= b;
  (>)     = \a b => a > b;
  (>=)    = \a b => a >= b;
}
```

Methods with defaults may be omitted:

```zt
Ord @Int :: {
  compare = compareInt;
}
```

Here `max` and `min` use the defaults derived from `compare` via `>=` and `<=`.

---

## Conditional Witnesses

A witness for a parameterised type may require constraints on its type arguments. Declare them in `[...]` after `::`:

```zt
Eq @(List A) :: [A: Eq] {
  eq = eqList;
}

Ord @(List A) :: [A: Ord] {
  compare = compareList;
}
```

---

## Using Constraints in Functions

Type parameters and their constraints are declared in `[...]` immediately after `::`:

```zt
contains :: [A: Eq] List A -> A -> Bool
         :: xs -> x { containsImpl xs x }

sort :: [A: Ord] List A -> List A
     :: xs { sortImpl xs }
```

Unconstrained parameters omit the `:` bound:

```zt
id      :: [A] A -> A
        :: x { x }

mapList :: [A, B] (A -> B) -> List A -> List B
        :: f -> xs { mapListImpl f xs }
```

Multiple parameters with independent constraints:

```zt
zipWith :: [A, B, C] (A -> B -> C) -> List A -> List B -> List C
        :: f -> xs -> ys { zipWithImpl f xs ys }
```

Witnesses are resolved implicitly at call sites — callers do not pass them explicitly.

---

## Coherence

At most one witness for a given `(Constraint, Type)` pair may be in scope. If two witnesses for the same pair are imported, the compiler rejects the ambiguity:

```
error: conflicting witnesses for Eq Int
```

Witnesses are automatically exported when their defining module is imported. There are no orphan restrictions in v1, but implementations may warn when a witness is defined outside both the constraint's module and the type's module.

---

## Higher-Kinded Constraints

Type parameters in `[...]` may carry a kind annotation using `::`, allowing constraints over type constructors rather than concrete types:

```zt
Functor :: [F :: Type -> Type] @F {
  map :: [A, B] (A -> B) -> F A -> F B;
}

Foldable :: [F :: Type -> Type] @F {
  fold :: [A, B] (B -> A -> B) -> B -> F A -> B;
}
```

`F :: Type -> Type` means F must be a type constructor — it takes one type and produces a type. The kind annotation is written in the constraint definition. At use sites, the kind is inferred from the constraint so callers need not repeat it:

```zt
mapTwice :: [F: Functor, A] (A -> A) -> F A -> F A
         :: f -> xs { map f (map f xs) }
```

Witnesses target the type constructor without arguments:

```zt
Functor @List :: {
  map = mapList;
}

Foldable @List :: {
  fold = foldList;
}
```

Partial type application is allowed in witness targets. A type constructor of kind `Type -> Type -> Type` applied to one argument yields a `Type -> Type` and may be witnessed:

For a `Result` type constructor with kind `Type -> Type -> Type`, fixing the error type `E` yields `Result E` with kind `Type -> Type`:

```zt
Functor @(Result E) :: [E] {
  map = \f r => match r {
    (#ok,  value = v) => (#ok,  value = f v);
    (#err, error = e) => (#err, error = e);
  };
}
```

v1 higher-kinded constraints target constructors of kind `Type -> Type`. Constructors of higher arity may be partially applied until they have kind `Type -> Type`; higher-order kind targets are reserved for a future version.

---

## Derive

A constraint may declare that it supports **structural derivation** — automatic witness synthesis from a type's shape.

**Marking a constraint as derivable**

Add `derive` after the constraint body to declare it supports structural derivation:

```zt
Eq :: [A] @A {
  eq :: A -> A -> Bool;
} derive

Ord :: [A: Eq] @A {
  compare :: A -> A -> Ordering;
  max?    :: A -> A -> A;
          :: a -> b { if a >= b then a else b }
  min?    :: A -> A -> A;
          :: a -> b { if a <= b then a else b }
} derive
```

**Requesting derivation**

A witness uses `derive` as its body instead of `{ ... }`:

```zt
Eq  @Server :: derive
Ord @Server :: derive
```

The compiler synthesizes the witness body from the type's structure. This fails with a compile error if:
- the constraint does not declare `derive`
- any component type lacks a witness for the constraint (e.g., a field of type `T` where `Eq @T` is not in scope)

**Structural derivation semantics**

For **record types**, derivation proceeds field by field. `eq` on `Server` becomes:

```zt
eq = \a b => (eq a.host b.host) && (eq a.port b.port);
```

where each field's `eq` is resolved from the in-scope witness for that field's type.

For **union types**, derivation compares by member shape first, then compares tuple fields field by field:

For `Status :: type [#active; (#suspended, reason : Text);]`, the derived equality shape is:

```zt
eq = \a b => match (a, b) {
  (#active, #active)                                    => true;
  ((#suspended, reason = r1), (#suspended, reason = r2)) => eq r1 r2;
  _                                                     => false;
};
```

User-defined derive recipes are post-v1. In v1 the compiler supplies the structural recipe for any constraint marked `derive`. Future versions will allow constraints to supply their own compile-time recipe using `fields` and witness reflection.
