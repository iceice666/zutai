# Universe Levels (v2)

[Type-level computation (v1)](../spec/v1/02-type-level-computation.md) exposes a
single surface sort, `Type`, and notes that the implementation *should* use
internal universe levels to avoid literal `Type : Type`. v1 leaves the core flat
— `Type : Type` — and relies on a deterministic fuel bound so that type-level
evaluation always terminates; because evaluation terminates, flat `Type : Type`
is not a run-time soundness risk in v1. v2 formalizes internal universe levels as
a static hygiene layer beneath the unchanged surface.

---

## The Hierarchy

Internally, every type inhabits a universe level:

```text
Type0 : Type1 : Type2 : ...
```

Ordinary data types inhabit `Type0`: `Int : Type0`, and a record of data types
is `Type0`. A type whose definition mentions `Type` itself inhabits a higher
level. Users continue to write only `Type`; the level is inferred.

---

## Cumulativity

Universes are cumulative: a type at level `l` is also a type at every level above
it.

```text
T : Type-l   =>   T : Type-(l+1)
```

Cumulativity is what keeps stratification invisible in practice: a `Type0` value
flows wherever a higher universe is expected, so ordinary programs that mix
concrete types and type parameters are unaffected.

---

## Level Inference and Polymorphism

The surface `Type` elaborates to a level metavariable, solved during type
checking and defaulted to the lowest consistent level. Generic type functions are
level-polymorphic, so a single definition works at every level it is used:

```zt
Pair :: Type -> Type -> Type
  = A B => type { first : A; second : B; };
```

`Pair` quantifies over the levels of `A` and `B`; `Pair Int Text` stays at
`Type0`, while `Pair Int Type` is accepted at the appropriate higher level.
Higher-kinded constraint targets such as `F :: Type -> Type` (see
[constraints](../spec/v1/03-constraints.md)) are likewise level-polymorphic, so
witnesses like `Functor @List` resolve without level annotations.

---

## What Stratification Rejects

Strict levels reject definitions that would require a universe to contain itself
— the encodings that flat `Type : Type` admits and that underlie type-theoretic
paradoxes:

```zt
// rejected under stratification: a definition that quantifies over its own
// universe and inhabits that same universe, i.e. requires Type-l : Type-l.
```

Such a definition is accepted today only because the core is flat; fuel stops its
*evaluation* from diverging but does not rule out the ill-founded definition.
Stratification rules it out statically. No well-founded v1 program is newly
rejected, because cumulativity and level defaulting subsume the flat typing of
ordinary and higher-kinded code.

---

## Relationship to Fuel

Levels and fuel are orthogonal and both retained:

- **Fuel** bounds type-level *evaluation* — it guarantees normalization
  terminates, handling non-productive recursive type functions such as
  `Loop :: Type -> Type = T => Loop T;` (see
  [type-level computation](../spec/v1/02-type-level-computation.md)).
- **Levels** bound type-level *self-reference* — they rule out universe-circular
  definitions statically.

Type equality remains normalization-based (NbE) under the fuel bound; the
normalizer additionally tracks levels so that equality and unification do not
conflate types at incompatible universes (see [`tlc-core.md`](../tlc-core.md)
§10).

---

## Surface Impact

None by default: programs continue to write `Type` with no level syntax, and
cumulativity plus defaulting preserve every well-founded v1 typing. An explicit
level annotation for advanced type-level code is reserved and is not part of the
default surface.

---

## Support Level

The TLC core kind already carries a level slot (`Type(level)`); v2 wires level
inference, cumulativity, and level-polymorphic defaulting through the front end
and the kind lowering, which currently pin every kind to level 0. Cumulativity
and defaulting are required parts of the feature, not optional refinements:
without them, introducing levels would reject currently-accepted higher-kinded
programs. Implementation is tracked as a v2 milestone in [`TBD.md`](../TBD.md).
