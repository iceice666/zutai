# Recursive Types (v2)

v0 and v1 provide one recursive structure, the built-in `List`. User-defined
record and union types are finite trees: a type alias may not refer to itself,
and the front end rejects an alias cycle. v2 lifts this restriction with
user-defined recursive and mutually recursive data types.

---

## Recursive Type Definitions

A type alias may refer to itself when the recursive occurrence is guarded by a
constructor — a record, union, tuple, list, optional, or function type:

```zt
Tree :: type {
  #leaf;
  #node : { value : Int; left : Tree; right : Tree; };
}
```

`Tree` occurs inside its own `#node` payload. Values are ordinary finite data:

```zt
example :: Tree =
  (#node,
    value = 1,
    left  = #leaf,
    right = (#node, value = 2, left = #leaf, right = #leaf));
```

Construction and pattern matching are unchanged from v1; the recursion lives in
the type, not in any new term form.

---

## Generic Recursive Types

A type function may produce a recursive type:

```zt
Tree :: Type -> Type
  = A => type {
    #leaf;
    #node : { value : A; left : Tree A; right : Tree A; };
  };
```

`Tree A` refers to itself at the same type argument, so the recursion is
uniform.

---

## Mutual Recursion

Top-level type aliases may refer to each other. The compiler resolves mutually
recursive aliases as a single binding group, so definition order does not
matter:

```zt
Expr :: type {
  #num  : { value : Int; };
  #call : { callee : Text; args : Args; };
}

Args :: type {
  #end;
  #arg : { head : Expr; rest : Args; };
}
```

`Expr` mentions `Args` and `Args` mentions `Expr`; both are checked together.

---

## Guardedness

A recursive occurrence must be guarded by a constructor. A bare alias cycle is
rejected:

```zt
Bad :: type Bad   // error: recursive type alias is not productive
```

This distinguishes recursive *data types* — productive, with the recursion under
a record/union/tuple/list/optional/function constructor — from recursive
*type-level functions* such as `Loop :: Type -> Type = T => Loop T;`, which are
non-productive and fail type-level evaluation by exhausting fuel (see
[type-level computation extensions](../v1_spec/02-type-level-computation.md)).
Guardedness is the static rule; fuel is the dynamic backstop.

Recursion is permitted in every guarded position, including under a function
arrow. This is what lets the algebraic-effect free-monad encoding represent a
suspended computation `resume : R -> Free Op A` (see [`tlc-core.md`](../tlc-core.md)
§9).

---

## Equirecursive Semantics

v2 recursive types are equirecursive: a recursive type and its one-step
unfolding denote the same type. There is no explicit roll/unroll term and no
nominal identity. Type equality unfolds recursive references on demand during
normalization, bounded by the type-level fuel limit, so two structurally equal
recursive types — however they are written — are interchangeable.

Nominal recursive types, which carry distinct identity and do not unfold
structurally, are reserved for a future version.

---

## Reflection and Derivation

A recursive type reflects (`fields`, `schema`) to a finite shape: a recursive
occurrence is a named back-reference rather than an infinitely expanded tree.
Recursion-aware reflection is what lets user-defined [derive
recipes](03-derive-recipes.md) traverse recursive types.

A structurally derived witness for a recursive type is itself recursive.
Deriving `Eq @Tree` produces an equality witness that refers to itself on the
recursive fields, resolved as a recursive binding — the same mechanism that lets
conditional witnesses like `Eq @(List A) :: <A: Eq>` recurse through their type
arguments (see [constraints](../v1_spec/03-constraints.md)).

---

## Backend and Support Level

Recursive types lower to cyclic type descriptors; the compilation stages below
TLC carry recursive type references by identity rather than by expansion (see
[`dataflow-core.md`](../dataflow-core.md) and [`runtime-abi.md`](../runtime-abi.md)).
Finite recursive *values* — ordinary trees and lists — render and compare in
finite time. The pure core does not construct cyclic values.

Recursive type support is structural (equirecursive). Implementation is tracked
as a v2 milestone in [`TBD.md`](../TBD.md); until it lands, the front end
continues to reject recursive and mutually recursive aliases with a diagnostic.
