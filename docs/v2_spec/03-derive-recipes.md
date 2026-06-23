# Derive Recipes (v2)

[Constraints (v1)](../spec/v1/03-constraints.md) let a constraint be marked
`derive`, and the compiler synthesizes a structural witness from a type's shape.
v1 supplies exactly one built-in recipe — the equality family (`eq`, `!=`) — and
states that user-defined derive recipes are post-v1. v2 lets a constraint carry
its own compile-time derivation recipe, built on type reflection and a new
witness-reflection primitive.

---

## Witness Reflection

`witness C @T` evaluates, at compile time, to the in-scope witness dictionary for
constraint `C` at type `T`:

```zt
witness Eq @Int            // the Eq dictionary for Int
(witness Eq @Int).eq 1 1   // true
```

If no witness for `(C, T)` is in scope, `witness C @T` is a compile error — the
same resolution and coherence rules as implicit witness passing (see
[constraints](../spec/v1/03-constraints.md)), made explicit. Witness reflection
is available inside derive recipes, where it lets a recipe delegate to the
witnesses of a type's components.

---

## Recipes

A derivable constraint may attach a recipe: a compile-time function from the
target type to the witness it should produce. The recipe replaces the built-in
structural recipe for that constraint.

```zt
Show :: <A> @A {
  show :: A -> Text;
} derive = <T> => deriveShow T;
```

`deriveShow :: Type -> { show : T -> Text; }` runs at compile time. It reflects
the structure of `T` and, for each component, delegates through
`witness Show @Component`. For a record `Point :: type { x : Int; y : Int; }`,
the recipe produces a witness equivalent to:

```zt
{
  show = \p => showRecord [
    ("x", (witness Show @Int).show p.x);
    ("y", (witness Show @Int).show p.y);
  ];
}
```

where `showRecord :: [(Text, Text)] -> Text` is a standard formatting helper. A
witness then requests derivation exactly as in v1:

```zt
Show @Point :: derive
```

---

## Reflecting Structure

A recipe consumes the reflection builtins, extended in v2 so that every
component type is reachable:

- `fields T` — for record types, the list of
  `{ name : Text; Type : Type; optional : Bool; }` (v1).
- `variants T` — for union types, the list of `{ name : Text; fields : [...]; }`.
  (v1 `fields` rejects unions; v2 adds `variants`.)
- `schema T` — the serializable shape (v1), now also defined over open rows and
  recursive types via named back-references (see
  [recursive types](01-recursive-types.md)).

The embedded `Type` value in a reflected field is a first-class compile-time
`Type` and may be used directly as the `@`-argument to `witness`:

```zt
\field => witness Show @(field.Type)
```

---

## Semantics

A recipe is pure, compile-time code, evaluated under the type-level fuel bound
(see [universe levels](04-universe-levels.md) and [`tlc-core.md`](../tlc-core.md)
§10). Its evaluation:

1. runs once per `(constraint, type)` derivation request;
2. iterates the reflected structure of the target — field by field for records,
   variant by variant for unions — with the iteration unrolled at compile time,
   so the generated method bodies are specialized to the type's shape rather than
   interpreting reflection data at run time;
3. resolves each component obligation through `witness C @Component`, failing
   with a compile error if a component witness is missing;
4. produces a witness record that is type-checked against the constraint's method
   signatures before it enters the dictionary-passing path.

The resulting witness obeys the usual coherence rule: at most one witness per
`(constraint, type)` pair (see [constraints](../spec/v1/03-constraints.md)).
Recipe failures — a missing component witness, a fuel-exhausted recipe, or a
result that does not match the method signatures — are compile errors located at
the derivation request.

---

## The Built-in Recipe

The structural equality recipe from v1 remains the default: a `derive`
constraint with no attached recipe uses the compiler's built-in structural
recipe for the equality family. That recipe is itself expressible over
`fields`/`variants` and `witness Eq`, so the built-in is a default, not a special
case. A constraint that attaches its own recipe overrides the built-in for that
constraint.

---

## Example: Lexicographic `Ord`

```zt
Ord :: <A: Eq> @A {
  compare :: A -> A -> Ordering;
} derive = <T> => deriveOrdLex T;
```

`deriveOrdLex` reflects the fields of `T` in declaration order and folds their
`compare` results — `(witness Ord @Field).compare` per field — returning the
first non-`#eq` result. For unions it orders by variant position, then by payload
fields. The recipe author writes this fold once; it then derives `Ord` for every
record and union whose components are themselves `Ord`.

---

## Support Level

Derive recipes require a witness-reflection primitive (`witness C @T`),
reflection over unions (`variants`) and recursive types, and a compile-time
staging boundary that runs a recipe during witness elaboration and reifies its
result into a dictionary. In the current implementation, reflection runs in the
type-value evaluator while derive synthesis runs during TLC dictionary lowering;
unifying these two paths is the substance of the v2 milestone tracked in
[`TBD.md`](../TBD.md). Until it lands, `derive` provides only the built-in
structural equality recipe.
