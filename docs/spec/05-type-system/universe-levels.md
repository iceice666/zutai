# Universe Levels

[Type-level computation](../05-type-system/type-level-computation.md) exposes a
single surface sort, `Type`, and notes that the implementation *should* use
internal universe levels to avoid literal `Type : Type`. Zutai leaves the core flat
— `Type : Type` — and relies on a deterministic fuel bound so that type-level
evaluation always terminates; because evaluation terminates, flat `Type : Type`
is not a run-time soundness risk in Zutai. Zutai formalizes internal universe levels as
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
[constraints](../06-polymorphism/constraints.md)) are likewise level-polymorphic, so
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
Stratification rules it out statically. No well-founded Zutai program is newly
rejected, because cumulativity and level defaulting subsume the flat typing of
ordinary and higher-kinded code.

---

## Relationship to Fuel

Levels and fuel are orthogonal and both retained:

- **Fuel** bounds type-level *evaluation* — it guarantees normalization
  terminates, handling non-productive recursive type functions such as
  `Loop :: Type -> Type = T => Loop T;` (see
  [type-level computation](../05-type-system/type-level-computation.md)).
- **Levels** bound type-level *self-reference* — they rule out universe-circular
  definitions statically.

Type equality remains normalization-based (NbE) under the fuel bound; the
normalizer additionally tracks levels so that equality and unification do not
conflate types at incompatible universes (see [`tlc-core.md`](../../tlc-core.md)
§10).

---

## Surface Impact

None by default: programs continue to write `Type` with no level syntax, and
cumulativity plus defaulting preserve every well-founded Zutai typing. Explicit
level syntax — `$ℓ` leveled universes and `<$l>` binders — is an opt-in
layer for advanced type-level code, specified in
[Explicit Level Syntax](#explicit-level-syntax); it is never required to type a
well-founded program.

---

## Explicit Level Syntax

The default surface stays level-free; the forms below are an opt-in layer for
type-level code that needs to name or relate universes. They are surface sugar
over the internal level algebra (`UniverseLevel`: `Known | Meta | Succ | Max`)
and add nothing to TLC, Dataflow Core, or the runtime — levels still erase
before lowering.

### A leveled universe: `$ℓ`

A universe at an explicit level is written `$` followed by a level. Bare `Type`
is unchanged and means `$<inferred>` — the same universe with its level left to
inference. There is no `Type$ℓ` form: `$ℓ` *is* the leveled universe, so the
common case stays short (`$0`, `$l`).

```ebnf
UniverseType ::= "Type"           // universe at an inferred level
              |  "$" LevelArg     // universe at an explicit level

LevelArg ::= IntLit              // $0
          |  Ident               // $l
          |  "(" Level ")"       // $(l + 1), $(max a b)

Level    ::= LevelAtom ("+" IntLit)?    // successor (n applications)
          |  "max" LevelArg LevelArg    // least upper bound
          |  LevelAtom

LevelAtom ::= IntLit | Ident | "(" Level ")"
```

A bare atom (`$0`, `$l`) needs no parentheses; compound level expressions are
parenthesized (`$(l + 1)`, `$(max a b)`). The `$` spelling is a dedicated
universe-level sigil that is otherwise unused in the surface language, so it
overloads nothing: in particular it is distinct from `@` (explicit type-level
argument, `Functor @List`) and from `#` (tags and labels, `#tag`,
`{ #left : X; }`). A `$` only ever introduces a leveled universe.

The level sub-grammar maps one-to-one onto the internal algebra:

| Surface | Internal `UniverseLevel` |
| --- | --- |
| `$0` | `Known(0)` |
| `$l` | the level bound to `l` (a shared meta per use) |
| `$(l + n)` | `Succ` applied `n` times |
| `$(max a b)` | `Max([a, b])` |

`max` is binary at the surface; nest with parentheses (`max a (max b c)`). `+`
takes an integer literal only — `l + 2` is two successors, while `l + m` (adding
two levels) is not a level operation and is rejected. The pure-inference level
(`Meta`) has no surface spelling; it is what bare `Type` elaborates to.

### Naming a level: the `<$l>` binder

A level variable is declared with the same `$` sigil it is used with, in a binder
list before the signature. `<$l>` declares one level; `<$a, $b>` declares two.

```ebnf
LevelBinders ::= "<" "$" Ident ("," "$" Ident)* ">"
```

```zt
Pair :: <$l> $l -> $l -> $l
  = A B => type { first : A; second : B; };
```

A `$`-prefixed name in binder position always denotes a level variable, so the
form is self-describing and needs no sort keyword — there is no separate `Level`
sort to name. Using a level variable where a type is expected, or a type or value
name where a level is expected, is a static error. A declared level variable that
is never used is reported as an unused parameter.

### Meaning: per-use level linking

A `<$l>` binder does not introduce prenex level polymorphism. Each use
of the declaration binds one shared level for every occurrence of `l` in that
signature; the shared level is solved from the use site and defaulted to the
lowest consistent universe, exactly like an inferred `Type`. The binder *links*
occurrences (they must agree) and documents intent — it does not generalize. A
signature may mix linked `$l` and independent bare `Type` freely.

Because solving and defaulting are unchanged, explicit levels reject nothing
that bare `Type` accepts; they add the ability to *pin* and *relate* universes:

```zt
Small       :: $0 = Int
TypeOfTypes :: $1 = $0          // $0 : $1

Pair Int Text   // l defaults to 0                     : $0
Pair Int Type   // Type : $1, cumulativity lifts Int   : $1

Sum :: <$a, $b> $a -> $b -> $(max a b)
  = X Y => type { #left : X; #right : Y; };
```

An explicit level that is *too low* for the annotated definition is rejected;
cumulativity makes the reverse harmless:

```zt
Bad :: $0 = $0   // reject: $0 : $1, not $0
Ok  :: $5 = Int  // accept: Int : $0 within $5 (cumulativity)
```

The new forms introduce four diagnostics: an explicit level below the required
universe, a level variable used as a type, a non-level name used as a level, and
an unknown level variable.

---

## Support Level

Internal universe levels have **landed** (`docs/ARCHIVED.md` Phase 24): the TLC
core kind carries a level slot (`Type(level)`), and level inference,
cumulativity, and level-polymorphic defaulting now flow through THIR kind
checking and TLC kind lowering rather than pinning every kind to level 0. Type
constructors and higher-kinded constraints are level-polymorphic and default to
the lowest consistent universe, so ordinary programs stay accepted while
`Pair Int Type` checks at a higher inferred universe; universe-circular
definitions produce a dedicated kind diagnostic. Cumulativity and defaulting are
required parts of the feature, not optional refinements: without them,
introducing levels would reject currently-accepted higher-kinded programs.
Type-level fuel still bounds normalization only, and runtime erasure and backend
output for ordinary value programs are unchanged. Explicit surface level syntax
has now **landed** ([Explicit Level Syntax](#explicit-level-syntax)): `$ℓ`
(`$0`, `$l`, `$(l + n)`, `$(max a b)`) and the `<$l>` binder parse, resolve, and
check. The implementation is front-end-only — the surface forms desugar to the
internal level algebra (`TypeKind::Type` carries a `UniverseLevel`) and erase
before TLC, Dataflow Core, and the runtime. Level binders link per use (not
prenex polymorphism): every `$l` in a signature shares one inferred level,
defaulted to the lowest consistent universe exactly like bare `Type`, so explicit
levels reject nothing a well-founded bare-`Type` program already accepts. Four
diagnostics guard misuse: an explicit level below the required universe, a level
variable used as a type, a non-level name used as a level, and an unknown level
variable. See `docs/ARCHIVED.md` "V2-A" for the landed milestone.
