# Higher-Rank Polymorphism (v2)

v1 polymorphism is predicative and rank-1: every quantifier sits at the outside
of a type, and a function that takes a polymorphic function as an argument is
[reserved for a future version](../spec/v1/01-row-polymorphism.md). v2 adds
higher-rank polymorphism — quantifiers nested inside a function's argument types
— with explicit annotations and bidirectional checking. Inference stays
predicative.

---

## Nested Quantifiers

An argument type may carry its own type-parameter list, making the argument
polymorphic:

```zt
applyId :: (<A> A -> A) -> { i : Int; t : Text; }
  = f => { i = f 1; t = f "x"; };
```

`f` is used at two different types in one body — `Int -> Int` and `Text -> Text`
— which rank-1 polymorphism cannot express. The `<A>` before `A -> A` quantifies
the argument, not the enclosing function.

Quantifiers may carry constraints, exactly as on declarations:

```zt
showBoth :: (<A: Show> A -> Text) -> { left : Text; right : Text; }
  = render => { left = render 1; right = render true; };
```

---

## Bidirectional Checking

Higher-rank types are checked, not guessed. Inference remains predicative and
rank-1: the compiler never *synthesizes* a higher-rank type for an unannotated
binding. Instead, a written higher-rank annotation is *pushed inward* — when the
expected type of an argument is `(<A> A -> A)`, the argument lambda is checked
against that polymorphic type.

A higher-rank function used without sufficient annotation, where the type is not
principal, is a type error asking for an annotation — consistent with v1's
inference-boundary policy (see [row polymorphism](../spec/v1/01-row-polymorphism.md)).
This keeps inference decidable and predictable.

---

## Predicativity

v2 higher-rank polymorphism stays **predicative**: a type variable is never
instantiated with a polymorphic type. So

```zt
// not inferred, and not accepted by instantiation:
list :: List (<A> A -> A)
```

is outside v2. Impredicative instantiation — placing a quantified type where a
monotype is expected — is reserved for a future version. Rank is unbounded in
annotations (rank-N), but instantiation remains first-order.

---

## Elaboration

Nested quantifiers elaborate to TLC `ForAll` in argument position; a higher-rank
argument becomes a value that is itself type-abstracted (`TyLam`), applied at
each use site with the appropriate type arguments. The mechanism already exists
for top-level polymorphism and dictionary passing (see [`tlc-core.md`](../tlc-core.md));
v2 extends *where* a `ForAll` may appear, not the core term forms.

Constrained higher-rank arguments pass their dictionaries the same way: the
argument value carries the constraint dictionaries it needs, supplied at each
call.

---

## Support Level

Higher-rank polymorphism has **landed** with reference-interpreter support
(`docs/ARCHIVED.md` Phase 26) for explicitly annotated nested quantifiers in
direct function argument positions. The parser, HIR, THIR, and TLC preserve
`ForAll`; THIR checks written higher-rank annotations bidirectionally while
inference remains predicative and rank-1; TLC elaborates applications with
`TyApp` plus dictionary `App` nodes for constrained arguments, so `applyId` and
constrained `showBoth` type-check and run through THIR and TLC. A `ForAll` in a
structural non-argument position (record field, union variant, list element, or
tuple item) rejects with an `UnsupportedFeature("impredicative type")`
diagnostic, and impredicative instantiation such as `List (<A> A -> A)` likewise
rejects with a dedicated diagnostic.
