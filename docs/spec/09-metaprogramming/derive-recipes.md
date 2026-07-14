# Derive Recipes

[Constraints](../06-polymorphism/constraints.md) let a constraint be marked
`derive`, and the compiler synthesizes a structural witness from a type's shape.
Zutai supplies the equality family (`eq`, `!=`) as the built-in structural
recipe and also lets a constraint carry its own compile-time derivation recipe,
built on type reflection and witness reflection.

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
[constraints](../06-polymorphism/constraints.md)), made explicit. Witness reflection
is available inside derive recipes, where it lets a recipe delegate to the
witnesses of a type's components.

---

## Recipes

A derivable constraint may attach a recipe: a compile-time function from the
target type to the witness it should produce. The recipe replaces the built-in
structural recipe for that constraint.

A recipe may return a quoted witness record directly or through the supported
pure `Code` reducer. Field names are taken from the expanded record rather than
recognized by the compiler, so arbitrary constraint methods work:

```zt
Const :: <A> @A { constant :: A -> Int; }
  derive = <T> => quote({ constant = \value. 7; })
```

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
witness then requests derivation exactly as in Zutai:

```zt
Show @Point :: derive
```

---

## Reflecting Structure

A recipe consumes the reflection builtins, extended in Zutai so that every
component type is reachable:

- `fields T` — for record types, the list of
  `{ name : Text; Type : Type; optional : Bool; }`.
- `variants T` — for union types, the list of `{ name : Text; fields : [...]; }`.
  (Zutai `fields` rejects unions; Zutai adds `variants`.)
- `schema T` — the serializable shape, now also defined over open rows and
  recursive types via named back-references (see
  [recursive types](../05-type-system/recursive-types.md)).

The embedded `Type` value in a reflected field is a first-class compile-time
`Type` and may be used directly as the `@`-argument to `witness`:

```zt
\field => witness Show @(field.Type)
```

---

## Semantics

A recipe is pure, compile-time code, evaluated under the type-level fuel bound
(see [universe levels](../05-type-system/universe-levels.md) and
[`compiler/tlc.md`](../../compiler/tlc.md)
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
`(constraint, type)` pair (see [constraints](../06-polymorphism/constraints.md)).
Recipe failures — a missing component witness, a fuel-exhausted recipe, or a
result that does not match the method signatures — are compile errors whose
primary location is the derivation request. When the constraint is declared in
the same source, the diagnostic also carries a secondary "constraint defined
here" location pointing at the constraint's declaration, so both the request and
the recipe definition are visible. A constraint imported from another module or
provided by the prelude has no in-file definition location, so only the request
location is shown.

---

## The Built-in Recipe

The structural equality recipe from Zutai remains the default: a `derive`
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
result into a dictionary. These have **landed** ([2026 H1 history](../../history/2026-h1.md), Phase 28):
constraint declarations carry `derive = <T> => ...` recipe bodies through
Syntax/HIR/THIR — the recipe is type-checked before TLC consumes the marker —
and quoted witness records expand generically before TLC dictionary passing.
The expanded record type is checked against the concrete constraint at the
derive request. Pattern-complete evaluation and typed reflection folds are
still open. The established Show/Ord structural builders remain compatibility fallbacks,
including same-variant payload ordering. `witness C @T` is parsed, typed as a
method-record dictionary, and resolved through the same concrete/conditional
lookup as implicit dispatch (accepting conditional witnesses such as
`Eq @(List A)` and reporting `WitnessReflectNotInScope` otherwise); type-value
reflection now exposes `variants` alongside `fields` and `schema`, preserving
recursive `Type` back-references. The built-in structural equality recipe
remains the default when a constraint attaches no recipe. At the backend,
`compile`/`dataflow` fold serializable reflection to constants and reject
residual reflection (a raw `witness` dictionary or a `Type`-valued result)
before lowering — the intended fold-or-reject model, not a gap.
