# TLC — Type Lambda Calculus IR

TLC (Type Lambda Calculus) is the fully-elaborated intermediate representation that sits between THIR and Dataflow Core. It is the **stable compile target**: every downstream stage (Dataflow Core → ANF → SSA → LLVM) reads only TLC; upstream surface-syntax changes require elaboration changes in the THIR→TLC lowering pass, never new core IR nodes.

> **Status:** Design approved.
> **Crate:** `crates/general/tlc/` (`zutai-tlc`)
> **Input:** completed `ThirFile` (only produced when `is_thir_complete()` is true)
> **Output:** `TlcModule`

---

## Pipeline position

```
Source → HIR → THIR → TLC
                        ↓  [TLC→DC: tree-to-graph, sharing, explicit recursion]
                   Dataflow Core
                        ↓  [DC→ANF: topo-sort SCCs, name every node, let/letrec]
                       ANF
                        ↓  [ANF→SSA: basic blocks, phi-nodes]
                       SSA
                        ↓  [SSA→LLVM: emit LLVM IR]
                    LLVM IR
```

**THIR** is error-tolerant and source-preserving — the foundation for LSP tooling (diagnostics, hover types, go-to-definition). It carries spans on every node and is produced even when type inference is incomplete.

**TLC** is produced only when `is_thir_complete()` is true. It has no free inference variables, no unresolved aliases, and explicit polymorphism via `TyLam`/`TyApp`. Spans are stored in a side-table only. The Dataflow Core lowering takes TLC as its sole input.

See [`docs/dataflow-core.md`](dataflow-core.md) for the next stage's IR specification.

---

## 1. Design thesis

**The fixpoint claim.** No new core IR nodes are needed for any planned frontend feature. Every hard surface feature maps to existing core mechanisms (or the two genuinely-new additions — `VariantT` type former and `Variant` term injection — that are unavoidable for sum types). This is proved by §6 (constraint witnesses) and §7 (algebraic effects).

**Why the current lowering fails this contract.** Three live bugs in `crates/general/tlc/src/lower/types.rs` silently destroy information:

| Location | Bug |
|---|---|
| `:64` | `TypeKind::Union(_) ⟹ TlcType::Record(Vec::new())` — every `type [...]` sum collapses to `{}` |
| `:20–22` | `TypeKind::Bool \| TypeKind::True \| TypeKind::False ⟹ PrimTy::Bool` — `True`/`False` singleton discrimination lost |
| `:83` | `TypeKind::Type \| TypeKind::Error ⟹ TlcType::Record(Vec::new())` — first-class `Type` erased incorrectly |

And a fourth data-loss issue at `:24`: `TypeKind::Atom(_) ⟹ TlcType::Prim(PrimTy::Atom)` — the atom's symbol payload is discarded. Additionally, `lower/expr.rs` currently lowers `ThirPatKind::TaggedValue` to `TlcPat::Wildcard` as a stopgap, losing tagged-union patterns entirely.

These are latent today because `zutai-eval` still walks THIR. They become correctness bugs the moment any downstream stage reads TLC. The root cause is the same for all: **the current `TlcType` enum has no sum former and no singleton-literal type former.** The design below adds both.

**The fix.** Extend the core with exactly the nodes needed to express every v0 and v1 construct as elaboration:
- Types and terms share one language, with `Type ℓ` universes separating levels.
- Phase distinction is enforced by a **runtime-erasure pass** before Dataflow Core, not a syntactic split. Types never depend on runtime values (Decision 0002), so after erasure downstream stages see a simply-typed program.
- The **Row kind** parameterizes record rows, union rows, and effect rows — one shared row machinery.
- Type equality = **normalization by evaluation (NbE)** with a deterministic fuel limit; no coercion/cast nodes needed until GADT-style local type equalities are introduced.

---

## 2. Frontend-proof argument

Every hard frontend feature maps to existing (or the two genuinely-new) core mechanisms:

| Frontend feature | Core mechanism | New core nodes? |
|---|---|---|
| Generics / polymorphism (`<A>`) | `ForAll(a:Kind, T)` + `TyLam(a:Kind, e)` / `TyApp(e, T)` | none (gains Kind annotation) |
| Higher-kinded types (`F :: Type -> Type`) | Arrow kinds (`Kind -> Kind`) + `TyLamK` | type layer only |
| Type constructors / type-level λ | `TyLamK(a:Kind, body)` as a type-level term | type layer only |
| Type-level computation, recursive types | `TyApp`+`TyLamK` normalized by NbE+fuel | kernel pass, no nodes |
| **Unions / sums** (`type [ ... ]`) | **`VariantT(Row)` type** + **`Variant(label, e)` term** | **2 new nodes — the fix** |
| **Singletons** (`#atom`, `true`, `false` as types) | **`Singleton(Lit)` type** | **1 new type node** |
| `Optional T` | sugar for `VariantT {some: T, none: Unit}` | none (VariantT subsumes) |
| Row polymorphism (open records, open unions) | `Row` kind + `RVar r` row variables | type layer only |
| Constraint witnesses (`Eq :: <A> @A { … }`) | **dictionaries-as-records** (see §6) | **zero** |
| Algebraic effects (`perform`/`handle`/`resume`) | effect row on `Fun` + elaborate-to-pure (§7) | **zero** |
| Type-level first-class `Type` values | bindings in type layer, erased before DC | type layer only |
| Pattern matching on singletons / union arms | `Variant(label, pat)` pattern arm in `Case` | pattern arm only |
| Record update (`with { field = v }`) | elaborates to `Record` literals (consistent with Decision 0001's stdlib-overlay stance) | none |
| Modules / imports | resolved in HIR; TLC sees flat `BindingId`s | none |
| Multi-clause functions | desugar to `Lam(x, Case(Var(x), alts))` | none |
| `If`/`else`, `Block`, `Binary` | `Case`, nested `Let`, `Builtin` | none |

The two genuinely new nodes — `VariantT` (type former) and `Variant` (term injection) — are unavoidable. Everything else is elaboration. The zero-new-node claims for constraint witnesses and effects are load-bearing and proved in §6 and §7.

---

## 3. Kind language

```
Kind ::= Type ℓ           -- universe at level ℓ  (ℓ ∈ ℕ)
                           -- Type 0 contains ground types, Type 1 contains Type 0, etc.
                           -- Written "Type" at surface; ℓ is an internal implementation detail
       | Row Kind          -- Row κ: a row whose entries have kind κ
                           -- records  →  Row (Type 0)
                           -- unions   →  Row (Type 0)
                           -- effects  →  Row Effect  (Effect ≅ Type 0 at v1)
       | Kind -> Kind      -- type-constructor kind  e.g. Type -> Type  [HKT / F-ω layer]
```

Universe levels: `Type 0 : Type 1`, `Type 1 : Type 2`, … — this avoids `Type : Type` unsoundness (see `v1_spec/02-type-level-computation.md` §"Universe Levels").

`Row κ` is deliberately parameterized. Records, unions, and effect rows all use the same row machinery (§4 Row constructors), just at different entry kinds.

Arrow kinds give HKT type variables: `F : Type -> Type` means F takes one concrete type and returns a concrete type — the basis for the constraint system's `Functor`, `Foldable`, etc. (see `v1_spec/03-constraints.md` §"Higher-Kinded Constraints").

---

## 4. Type language

The following replaces the `TlcType` enum in `crates/general/tlc/src/ir.rs`.

```
Ty  ::=
    -- Variables --
    TyVar(a: TlcTypeVar, k: Kind)       -- a kinded type variable (named or inferred)
                                         -- TlcTypeVar = Named(u32) | Inferred(u32)

    -- Primitives --
    Prim(P)                             -- Int | Float | Bool | Str | Atom | Nothing
                                         -- Note: code enum uses `Str`; THIR surface says `Text`;
                                         -- types.rs:23 bridges TypeKind::Text → PrimTy::Str
    Singleton(Lit)                      -- #atom, true, false, integer literal, … as a type
                                         -- NEW; fixes True/False singleton loss and Atom symbol loss

    -- Type-operator layer (F-ω) --
    TyLamK(a: TlcTypeVar, k: Kind, body: TlcTypeId)
                                        -- type-level λ: Λ(a:κ). T
                                         -- subsumes aliases, type constructors, HKT
    TyApp(f: TlcTypeId, arg: TlcTypeId)
                                        -- type application (spine)
    ForAll(a: TlcTypeVar, k: Kind, body: TlcTypeId)
                                        -- ∀(a:κ). T  (kinded quantification)

    -- Function types --
    Fun(from: TlcTypeId, to: TlcTypeId, eff: EffRow)
                                        -- A -{ε}> B  (eff = REmpty in v0 = pure)

    -- Composite types --
    RecordT(Row)                        -- record built from a row
    VariantT(Row)                       -- union / sum built from a row     NEW; fixes Union→{}
    TupleT(Vec<TlcTupleField>)          -- (label: T, T, …)
    ListT(TlcTypeId)                    -- [T]

Row ::=
    REmpty                              -- closed row: no more fields
    RExtend(label: String, ty: TlcTypeId, tail: Row)
                                        -- add one field/arm to a row
    RVar(r: TlcTypeVar)                 -- open tail (row variable — v1 row polymorphism)

EffRow ::= Row                          -- same Row type; entries have kind Effect
                                        -- REmpty = pure function  (v0 default)
```

### Key notes

**`VariantT` is the real sum former.** The surface `type [ #dev; #test; #prod; ]` elaborates to:
```
VariantT(
  RExtend("dev",  Singleton(Atom("dev")),
  RExtend("test", Singleton(Atom("test")),
  RExtend("prod", Singleton(Atom("prod")),
  REmpty))))
```
Tuple-member arms like `(#circle, radius: Float)` elaborate to a record arm inside `VariantT`:
```
RExtend("circle", RecordT(RExtend("radius", Prim(Float), REmpty)), …)
```

**`Optional T` is sugar** for `VariantT { some: T, none: Unit }`. The existing `TlcType::Optional` variant may be kept as a convenience alias during the transition but is defined in terms of `VariantT` — no independent semantics.

**`Singleton(Lit)` supplies DC's `True`/`False` discrimination.** Dataflow Core uses `True | False` singleton types (see `dataflow-core.md` §"Type representation"). This node closes the gap. Atom literals in type position (e.g., `#dev` as a type) are also `Singleton(Atom("dev"))`.

**`Fun` carries an effect row.** In v0, `eff = REmpty` (pure) always. The field costs nothing for v0 programs and gives effects a type-level hook in v1 without adding new node kinds.

**`TyLamK` replaces eager alias expansion.** The current `types.rs` eagerly expands `Alias`/`AliasApply` to concrete types, which cannot handle recursive type functions. Instead, `Alias(b)` lowers to `TyVar(b)` (where `b` is the alias binding, kinded `Type 0`) or `TyLamK(params, body)` for generic aliases; `AliasApply { binding, args }` lowers to `TyApp(TyVar(b), args)`. The NbE normalizer (§9) reduces applications on demand.

**`RecordT`/`VariantT` over `Row` — one row machinery.** Closed records (v0) have no `RVar`; open records (v1 row polymorphism, `v1_spec/01-row-polymorphism.md`) carry `RVar r` as the row tail. The identical structure applies to union types and effect rows.

**No `Coerce`/cast node.** Equality is normalization-based (§9). Coercions would be needed only if GADT-style local type equalities were introduced. No such feature is planned; this is an explicit design boundary (see §10 non-goals).

---

## 5. Term language

The existing `TlcExpr` enum requires only **one new variant**:

```
Tm ::=
    -- Unchanged --
    Var(BindingId)
    Lit(Literal)
    Lam(BindingId, TlcTypeId, TlcExprId)         -- λ(x: T). e
    App(TlcExprId, TlcExprId)
    TyLam(TlcTypeVar, Kind, TlcExprId)            -- Λ(a:κ). e  (gains Kind annotation)
    TyApp(TlcExprId, TlcTypeId)
    Let { binding, ty, value, body }
    Letrec { bindings: Vec<(BindingId, TlcTypeId, TlcExprId)>, body }
    Case(TlcExprId, Vec<TlcAlt>)                  -- arms gain VariantPat and SingletonPat
    Record(Vec<(String, TlcExprId)>)
    GetField(TlcExprId, String)
    Tuple(Vec<TlcTupleItem>)
    List(Vec<TlcExprId>)
    Builtin(BuiltinOp, TlcExprId, TlcExprId)      -- always binary in v0

    -- NEW --
    Variant(label: String, value: TlcExprId)      -- inject into a sum: #dev  or  (#circle, …)
```

### Pattern language additions

`TlcPat` gains two new arms to match the two new type formers:

```
TlcPat ::=
    Wildcard | Bind(BindingId) | Lit(Literal)
    Tuple(Vec<TlcPatItem>) | Record(Vec<(String, TlcPat)>)
    Atom(String)            -- unchanged (already present)
    Variant(label: String, inner: TlcPat)   -- NEW: match a sum arm
    Singleton(Literal)                       -- NEW: match a singleton-typed scrutinee
```

In practice `Singleton` pattern matching is identical to `Lit`; the distinction is only in the *type* of the scrutinee. Implementations may unify them.

### Why the term language barely grows

**Dictionaries are `Record` values** (see §6). No `dict`/`witness`/`method` node.

**Effect operations elaborate to `Lam`/`App`/`Case`/`Record`** over a free-monad encoding (see §7). No `perform`/`handle`/`resume` node.

**Type abstraction / application** (`TyLam`/`TyApp`) already exist; they gain only a `Kind` annotation, which is a data change, not a new variant.

---

## 6. Module structure and lowering entry point

The `TlcModule` holds four arenas and two side-tables:

```rust
pub struct TlcModule {
    pub decls:      Vec<TlcDeclId>,
    pub decl_arena: Arena<TlcDecl>,
    pub expr_arena: Arena<TlcExpr>,
    pub type_arena: Arena<TlcType>,
    pub expr_types: HashMap<TlcExprId, TlcTypeId>,  // every node's fully-resolved type
    pub spans:      HashMap<TlcExprId, Span>,         // source location, for diagnostics
}
```

Only value bindings and type aliases survive to TLC. Imports are resolved in HIR; sum/union-variant constructors are lowered to `Variant` injections.

```rust
pub enum TlcDecl {
    Value {
        binding: BindingId,
        ty:      TlcTypeId,
        body:    TlcExprId,
    },
    TypeAlias {
        binding: BindingId,
        body:    TlcTypeId,   // TyLamK(params, ty_body) for generic aliases; TyVar(b) for simple
    },
}
```

**Entry point:** `pub fn lower_thir(file: &ThirFile) -> TlcModule`

Called only when `is_thir_complete(file)` — never on a program with THIR error nodes or incomplete type information.

`zutai-semantic` exposes TLC alongside THIR:

```rust
pub struct SemanticModule {
    pub thir: LoweredThir,
    pub tlc:  Option<TlcModule>,   // None when thir has any error
}
```

**`poly_schemes` prerequisite: already satisfied.** `ThirFile` exposes `pub poly_schemes: HashMap<BindingId, Vec<u32>>` (populated during THIR lowering; verified by regression tests in `zutai-thir`). The TLC lowering pass reads this field directly — no further THIR changes are required.

---

## 7. THIR→TLC lowering pass

### 7.1 Type translation

The full table of THIR `TypeKind` → `TlcType` mappings, incorporating the Phase 0 bug fixes:

| THIR `TypeKind` | Current TLC (wrong where marked) | New TLC |
|---|---|---|
| `InferVar(n)` in `poly_schemes[b]` | `TyVar(fresh)` | `TyVar(fresh, Type 0)` — gains Kind |
| `InferVar(n)` not in any scheme | Hard compiler error | (unchanged: must not survive zonking) |
| `TypeVar(b)` | `TyVar(b)` | `TyVar(b, Type 0)` |
| `Function { from, to }` | `Fun(from, to)` | `Fun(from, to, REmpty)` — gains eff field |
| `Alias(b)` | expand eagerly **BUG** | `TyVar(b, Type 0)` or unfold once + NbE |
| `AliasApply { b, args }` | expand eagerly **BUG** | `TyApp(TyVar(b), args)` — NbE reduces on demand |
| `Record(fields)` | `Record(fields)` | `RecordT(RExtend(…, REmpty))` |
| `Tuple(items)` | `Tuple(items)` | `TupleT(items)` |
| `List(inner)` | `List(inner)` | `ListT(inner)` |
| `Optional(inner)` | `Optional(inner)` | `VariantT { some: inner, none: Unit }` (or keep as transition alias) |
| `Union(members)` | `Record(vec![])` **BUG** | `VariantT(row_of_members)` |
| `True` | `Prim(Bool)` **BUG** | `Singleton(Lit::Bool(true))` |
| `False` | `Prim(Bool)` **BUG** | `Singleton(Lit::Bool(false))` |
| `Atom(sym)` | `Prim(Atom)` (loses symbol) | `Singleton(Lit::Atom(sym))` |
| `Bool` | `Prim(Bool)` | `Prim(Bool)` — unchanged |
| `Text` | `Prim(Str)` | `Prim(Str)` — unchanged (code enum uses `Str`; THIR uses `Text`) |
| Scalars (Int, Float, Nothing) | `Prim(…)` | `Prim(…)` — unchanged |
| `Type` | `Record(vec![])` **BUG** | erased — must not reach TLC type arena as a reified node |
| `Error` | `Record(vec![])` | unreachable — TLC only produced when THIR complete |

**Phase line for `Type` values.** THIR bindings with `TypeKind::Type` in their annotation (type-valued declarations like `Server :: type { ... }`) lower as follows:
- The *type* of the binding in `expr_types` is recorded as `TyVar` of a fresh universe-level variable (kind `Type 1`).
- The *body* of the binding, being a type expression, is consumed by the normalizer and not emitted as a runtime `TlcExpr`. This is the erasure gate.
- Downstream Dataflow Core never sees a `Type`-kinded expression.

### 7.2 Expression desugaring

Surface forms are eliminated here so Dataflow Core never sees them:

| THIR form | TLC form |
|---|---|
| `If { cond, then, else_ }` | `Case(cond, [Alt(Singleton(true), then), Alt(Singleton(false), else_)])` |
| `Block { locals, tail }` | Nested `Let` bindings, `tail` as innermost body |
| `Binary { op, lhs, rhs }` | `Builtin(op, lhs, rhs)` |
| Multi-clause function | `Lam(x, T, Case(Var(x), alts))` per arg, curried |
| Single-clause function | `Lam(x, T, body)` per arg, curried |
| Union arm injection `#dev` | `Variant("dev", Lit(Unit))` |
| Union arm injection `(#circle, r)` | `Variant("circle", Record([("radius", r)]))` |

### 7.3 Polymorphism elaboration

Driven by `poly_schemes: HashMap<BindingId, Vec<u32>>` from `ThirFile`.

**At declaration sites** — for binding `b` where `poly_schemes[b] = [v1, v2, ...]`:
1. Assign fresh `TlcTypeVar::Named(…)` entries as named type variables (one per quantified `InferVar`).
2. Substitute `InferVar(v1) → TyVar(a, Type 0)`, `InferVar(v2) → TyVar(b, Type 0)` throughout the binding's type.
3. Wrap type: `ForAll(a, Type 0, ForAll(b, Type 0, T))`.
4. Wrap body: `TyLam(a, Type 0, TyLam(b, Type 0, body))`.

**At call sites** — `ThirExprKind::Apply { func, arg, instantiation: [T1, T2] }`:
- If the callee is polymorphic: prepend `TyApp` nodes before the value-level `App`.
  - `Var(b)` → `TyApp(TyApp(Var(b), translate(T1)), translate(T2))`
- If `instantiation` is empty: plain `Var`, no `TyApp`.

**`poly_schemes` → kinded `ForAll`/`TyLam`.** Each quantifier carries a `Kind` annotation. In Phase 1 all kinds default to `Type 0` (no behavioral change, full backward compat). Future phases add HKT kinds from constraint definitions.

**DC alignment.** Dataflow Core's type representation (see `dataflow-core.md` §"Type representation") uses different names and shapes. The TLC→DC lowering maps:
- TLC `TyApp(f, arg)` → DC `TyApp(lower(f), [lower(arg)])` (unary→N-ary, trivial wrapper)
- TLC `TyLamK(a, k, body)` → DC `TyFun([a], lower(body))`
- TLC `TyVar(Named(bid), k)` → DC `TyVar(TyVar { binding: BindingId(bid) })`
- TLC `Singleton(Lit::Bool(true))` → DC `True`; `…Bool(false)` → DC `False`
- TLC `VariantT(row)` → DC `Union(members_from_row)` (row flattened to `Vec<TyId>`)

---

## 8. Constraint witnesses — zero new nodes

*(Encodings in this section are approved under Decision 0003 and carried over from the design spec. They have not been re-derived in this consolidation pass.)*

Zutai v1 constraint witnesses (`v1_spec/03-constraints.md`) elaborate entirely into existing term nodes via **dictionaries-as-records**.

**Elaboration rule.** A constrained function:
```zt
Eq :: <A> @A {
  eq :: A -> A -> Bool;
}

eqBoth :: <A: Eq> A -> A -> A -> Bool {
  | x y z => eq x y && eq y z;
}
```
elaborates to a function that takes an extra dictionary argument:
```
eqBoth :: ∀(A:Type). {eq: A -> A -> Bool} -> A -> A -> A -> Bool
eqBoth = TyLam A.
           Lam(dict: {eq: A -> A -> Bool}).
             Lam(x: A). Lam(y: A). Lam(z: A).
               Builtin(And,
                 App(App(GetField(Var dict, "eq"), Var x), Var y),
                 App(App(GetField(Var dict, "eq"), Var y), Var z))
```

**At a call site:**
```zt
eqBoth 1 2 3    -- caller knows A = Int and has witness Eq @Int
```
elaborates to:
```
TyApp(Var(eqBoth), Int)
  `App` Record([("eq", Var(eqIntImpl))])   -- dictionary is a Record
  `App` Lit(Int 1)
  `App` Lit(Int 2)
  `App` Lit(Int 3)
```
The witness `Eq @Int :: { eq = \a b => a == b; }` compiles to an ordinary `Record` value binding.

**Higher-kinded constraints** (`Functor :: <F :: Type -> Type> @F { … }`) add a `Kind` annotation to `ForAll` and `TyLam` nodes that already exist. The dictionary is still a `Record` whose fields are functions.

**`} derive` / `@T :: derive`** (auto-generating witness implementations) is a compile-time elaboration pass that emits standard TLC term nodes. No core change.

**Verdict.** Constraint witnesses require zero new `TlcExpr` variants and zero new `TlcType` variants. The only "change" is that the lowering pass emits additional `Record`, `GetField`, `TyLam(a:Kind)`, and `ForAll(a:Kind)` nodes that it already knows how to emit.

---

## 9. Algebraic effects — zero new nodes

*(Encodings in this section are approved under Decision 0003 and carried over from the design spec. They have not been re-derived in this consolidation pass.)*

Zutai v1 algebraic effects (`v1_spec/05-effects.md`) elaborate to pure terms via a **free-monad encoding**. The effect row on `Fun` (§4) is the only type-level hook.

**The encoding.** An effectful computation of type `A ! { op: P -> R }` is represented as a pure value of type `Free Op A` where:
```
Op     = VariantT { op: RecordT { param: P, resume: R -> Free Op A } }
Free O A = VariantT { pure: A, impure: O }
```
Both `Free` and `Op` are `VariantT` types — no new type former.

**`perform op arg`** elaborates to an injection:
```
Variant("impure",
  Record([
    ("op_name", Variant("op",
      Record([("param", arg), ("resume", Lam(r, Var r))])))
  ]))
```

**`handle expr with { value = f, op = h }`** elaborates to a recursive interpreter:
```
Letrec([
  (handleId, ...,
    Lam(computation, ...
      Case(computation, [
        Alt(Variant("pure", Bind x), App(Var f, Var x)),
        Alt(Variant("impure", Bind cmd),
          Case(Var cmd, [
            Alt(Variant("op", Bind req),
              App(App(Var h, GetField(Var req, "param")),
                  Lam(result, App(Var handleId,
                    App(GetField(Var req, "resume"), Var result)))))
          ]))
      ])))
)], body)
```

Every node here (`Letrec`, `Lam`, `App`, `Case`, `Variant`, `Record`, `GetField`) already exists in §5.

**`resume`** is the continuation passed as a function inside the `resume` field — no new node.

**Effect erasure.** After elaboration, `Fun(A, B, eff)` has its effect row erased to `Fun(A, B, REmpty)` before emission to Dataflow Core. DC sees only the pure type.

**Verdict.** Effects require zero new `TlcExpr` variants and zero new `TlcType` variants beyond `VariantT` (needed for sums anyway) and the effect-row slot on `Fun` (data change, not a new node kind).

---

## 10. Type equality = NbE

The kernel carries a **type normalizer** with a deterministic fuel bound.

### Equality rule

```
Equal(T, U)  ≡  normalize(T) =α= normalize(U)
```

The normalizer is a kernel pass, not a node in the IR.

### Reductions

| Redex | Reduct |
|---|---|
| `TyApp(TyLamK(a, k, body), arg)` | `body[a := arg]` |
| `TyApp(TyVar(b, _), arg)` where `b` is an alias binding | expand alias, then re-reduce |
| `RExtend(l, T, RExtend(l, U, r))` | error: duplicate label |
| Row canonicalization | sort `RExtend` entries by label for α-equality |

### Fuel

Each `TyApp` reduction step consumes one unit of fuel. Default limit: 1 000 steps (configurable). Exhausting fuel is a compile-time error:
```
error: type-level computation exceeded evaluation limit
```
This handles recursive type functions like `Loop :: Type -> Type { | T => Loop T; }` (see `v1_spec/02-type-level-computation.md` §"Recursive Type Functions"). F-ω cannot express such functions (the kind system is strongly normalizing); the unified universe core + fuel handles them correctly.

### Coercion boundary

The coercion-free core is sound **only while no GADT-style local type equalities exist** (a type refined by pattern-matching, e.g., `a ~ Int` in one branch). If GADTs are ever added to the frontend, a coercion/cast node (System F_C style) must be retrofitted. This is an explicit design boundary, not a surprise.

---

## 11. Non-goals (phase line)

The following are explicitly out of scope for TLC-the-core and must not be added without a new design decision:

- **Dependent types at runtime.** Types may not depend on runtime values (Decision 0002). Types are erased before Dataflow Core. "Unified universe core" means types-as-terms in the *type layer*; it does not mean runtime type dispatch.
- **Coercion/cast nodes (`Coerce(e, T, U)`).** Equality is normalization. No coercions until GADTs arrive.
- **Impredicative / higher-rank polymorphism beyond F-ω.** `ForAll` is predicative. `TyLam` at the term level handles rank-2 naturally via dictionaries.
- **`perform`/`handle`/`resume` as core nodes.** Effects elaborate to free-monad/CPS (§9).
- **Record-update node.** Overlay stays stdlib (consistent with Decision 0001).
- **Thunk/strictness annotations.** Laziness is represented structurally in Dataflow Core (reachability), not in TLC.
- **`Type` or `Error` surviving to TLC as runtime expressions.** First-class `Type`-valued bindings live in the type layer and are erased. `TypeKind::Error` means type-checking failed — TLC is never produced when `is_thir_complete()` is false.

---

## 12. Crate structure

```
crates/general/tlc/
  Cargo.toml          -- dependencies: zutai-thir, zutai-hir, zutai-syntax
  src/
    lib.rs            -- pub re-exports; pub fn lower_thir(file: &ThirFile) -> TlcModule
    ir.rs             -- all IR types: TlcModule, TlcDecl, TlcExpr, TlcType,
                      --   TlcAlt, TlcPat, Kind, Row, BuiltinOp, PrimTy, Literal, ids
    lower/
      mod.rs          -- Lowerer struct, entry point, arena allocation helpers
      types.rs        -- ThirType → TlcType; NbE normalizer (Phase 2+)
      expr.rs         -- ThirExpr → TlcExpr desugaring + span recording
      decl.rs         -- ThirDecl → TlcDecl + polymorphism elaboration
    tests.rs          -- unit tests
```

`zutai-semantic` changes:

```rust
pub struct SemanticModule {
    pub thir: LoweredThir,
    pub tlc:  Option<TlcModule>,   // None when thir has any error
}
```

`zutai-semantic`'s `Cargo.toml` gains `zutai-tlc` as a dependency. `zutai-dataflow` depends on `zutai-tlc` for its lowering input.

---

## 13. Testing strategy

**Unit tests in `zutai-tlc`** (`src/tests.rs`):
- Desugaring: each THIR surface form produces the expected TLC shape.
- Polymorphism: monomorphic identity, polymorphic identity, polymorphic pair — assert `ForAll`/`TyLam`/`TyApp` structure.
- Sum types: `type [ #a; #b; ]` produces `VariantT`; tagged tuple `(#circle, r: Float)` produces `VariantT { circle: RecordT { … } }`.
- Singleton types: `True`, `False`, `#atom` in type position produce `Singleton(…)`, not `Prim(Bool)`.
- Alias laziness: `Pair Text Int` produces `TyApp(TyVar(Pair), Text, Int)` — no `Alias` node and no eager expansion.
- NbE (Phase 2+): `Response Text ≡ RecordT { status: Int, body: Optional Text }` by normalization; `Loop Int` exceeds fuel with a clean diagnostic.
- Invariants: every `TlcExprId` has an entry in `expr_types`; no `InferVar`, `Alias`, or `Union` in any `TlcType` in the module.

**Integration tests via `zutai-semantic`**:
- All existing THIR and semantic tests pass (TLC is additive).
- `SemanticModule.tlc` is `Some` for well-typed programs, `None` for programs with THIR errors.
- Differential: for programs `zutai-eval` can run, the TLC type of the final expression matches the runtime type.

**Verification gate:** `cargo test --workspace` green throughout.

---

## 14. Phased implementation roadmap

Each phase is additive and independently testable. No phase breaks anything done earlier.

**Phase 0 — Close the live hole (unblocks all downstream work)**
- Add `VariantT(Row)` to `TlcType`; add `Variant(label, TlcExprId)` to `TlcExpr`.
- Add `Singleton(Literal)` to `TlcType`.
- Fix `lower/types.rs`: `Union` → `VariantT`; `True`/`False` → `Singleton`; `Atom` → `Singleton`.
- Gate `Type`/`Error` at the lowering boundary (unreachable assertion).
- Add `Variant(label, pat)` and `Singleton(Literal)` pattern arms to `TlcPat`.
- Fix `lower/expr.rs` stopgap: `ThirPatKind::TaggedValue` → `TlcPat::Variant(…)`, not `Wildcard`.
- Test: every v0 program lowers without data loss; no `InferVar`/`Alias`/`Union` in any `TlcType`.

**Phase 1 — Kind annotations**
- Add `Kind` enum; kind-annotate `TyVar`, `ForAll`, `TyLam` (term), `TyLamK` (type-level λ).
- Default all kinds to `Type 0` (no behavioral change, full backward compat).
- Test: round-trip kind inference for all existing polymorphic programs.

**Phase 2 — NbE normalizer + alias reform**
- Implement `normalize(ty: TlcTypeId) -> TlcTypeId` with fuel counter in `lower/types.rs`.
- Replace eager alias expansion with `TyApp(TyVar(b), args)` + lazy normalization.
- Wire equality checks through `normalize`.
- Test: `Response Text ≡ RecordT { status: Int, body: Optional Text }` by normalization; `Loop Int` exceeds fuel with a clean diagnostic.

**Phase 3 — Row kind and row polymorphism**
- Add `RVar(TlcTypeVar)` arm to `Row`; add `Row` kind to `Kind`.
- `RecordT`/`VariantT` switch from flat `Vec<TlcRecordField>` to `Row`.
- Lower `...Rest` row tails from THIR open-record types to `RVar`.
- Test: row-polymorphic identity function lowers to `ForAll(Rest: Row, …)`; DC sees flattened closed rows with no `RVar`.

**Phase 4 — Effect row on `Fun`**
- Change `TlcType::Fun(TlcTypeId, TlcTypeId)` to `Fun(TlcTypeId, TlcTypeId, EffRow)`.
- Default `eff = REmpty` for all v0 lowerings (no behavioral change).
- Implement free-monad elaboration for `perform`/`handle`/`resume` in the lowering pass.
- Erase effect rows before emitting DC types.
- Test: effectful programs elaborate to pure TLC; DC type of effectful function = pure function type.

**Phase 5 — Constraint witnesses / dictionaries**
- Implement dictionary-passing elaboration: constraint definition → `Record` type; witness → `Record` value; `<A: C>` → extra `Lam(dict, …)` parameter.
- Wire HKT constraint kinds (Phase 1 prerequisite).
- Migration trigger for eval: switch `zutai-eval` walker to TLC (Decision 0002).
- Test: `eqBoth` from §8 elaborates correctly; derived witness implementations match hand-written.

---

## 15. Invariants

The following invariants hold for every valid `TlcModule`:

1. No `TypeKind::InferVar` appears in any `TlcType`.
2. No `TypeKind::Alias` or `TypeKind::AliasApply` appears in any `TlcType`.
3. No `TypeKind::Union` appears in any `TlcType` (replaced by `VariantT`).
4. No `TypeKind::True`, `TypeKind::False`, or `TypeKind::Atom` flattened to `Prim(Bool)`/`Prim(Atom)` — these are `Singleton` nodes.
5. Every `TlcExprId` in `expr_arena` has a corresponding entry in `expr_types`.
6. `lower_thir` is only called when `is_thir_complete(file)` is true.
7. Polymorphic bindings in `poly_schemes` have `TyLam`-wrapped bodies and `ForAll`-prefixed types.
8. Every polymorphic call site has one `TyApp` per quantified type variable.
9. No `TlcType` node with kind `Type ℓ` for `ℓ ≥ 1` is reified as a runtime expression (phase line).
10. After Phase 4: every `Fun` type in `expr_types` has `eff = REmpty` (effect rows erased before DC).

---

## 16. Feature coverage table

A complete audit of v0 and v1 constructs against this core. "Phase" = which implementation phase (§14) makes the construct work end-to-end.

### v0 constructs

| Construct | Encoding in TLC | Phase |
|---|---|---|
| Literals: int, float, bool, text, atom (`#x`) | `Lit(…)` | 0 |
| Record type `{ field: T }` | `RecordT(RExtend("field", T, REmpty))` | 0 |
| Record value `{ field = e }` | `Record([("field", e)])` | 0 |
| Field access `x.field` | `GetField(e, "field")` | 0 |
| Optional field `field?: T` | `RExtend("field", T, …)` with `optional=true` flag | 0 |
| Optional value `T?` | `VariantT {some: T, none: Unit}` | 0 |
| Optional defaulting `x ?? y` | `Builtin(Coalesce, x, y)` | 0 |
| Union type `type [ #a; #b; ]` | `VariantT(RExtend("a", Singleton(Atom("a")), …))` | **0 (fixes bug)** |
| Union arm with tuple `(#c, x: T)` | `VariantT { c: RecordT { x: T } }` | **0 (fixes bug)** |
| `true`/`false` as singleton type | `Singleton(Lit::Bool(true/false))` | **0 (fixes bug)** |
| `#atom` as singleton type | `Singleton(Lit::Atom("atom"))` | **0 (fixes bug)** |
| Lists `[T]`, list literals | `ListT(T)`, `List([…])` | 0 |
| Tuples `(label: T, T)` | `TupleT(…)`, `Tuple(…)` | 0 |
| Function type `A -> B` | `Fun(A, B, REmpty)` | 0 |
| Function value `\| x => e` | `Lam(x, T, e)` | 0 |
| Multi-clause function | `Lam(x, T, Case(Var x, alts))` (desugared) | 0 |
| `if/else` | `Case(cond, [Alt(Singleton(true), t), Alt(Singleton(false), e)])` | 0 |
| `Block { locals; tail }` | nested `Let` | 0 |
| Binary operators | `Builtin(op, lhs, rhs)` | 0 |
| Generic function `<A>` | `ForAll(A, Type0, body)` + `TyLam(A, Type0, e)` | 0 |
| Generic type alias `<A, B>` | `TyLamK(A, Type0, TyLamK(B, Type0, body))` | 0 |
| Type alias usage `Pair Text Int` | `TyApp(TyApp(TyVar(Pair), Text), Int)` → NbE | 2 |
| Pattern matching `match` | `Case(e, alts)` | 0 |
| Variant pattern `(#circle, …)` | `Alt(Variant("circle", pat), body)` | 0 |
| Module imports | resolved in HIR; flat `BindingId` in TLC | 0 |
| `Type` as a value (`Server :: type { … }`) | type binding, erased before DC | 0 |

### v1 constructs

| Construct | Encoding | Phase |
|---|---|---|
| Open record `{ host: Text; …; }` | `RecordT(RExtend("host", Text, RVar(r)))` | 3 |
| Named row tail `...Rest` | `RVar(Rest)` in `Row` | 3 |
| Open union `type [ #a; …; ]` | `VariantT(RExtend("a", …, RVar(r)))` | 3 |
| Union extension `...Shape` | row spread, resolved at elaboration | 3 |
| HKT param `F :: Type -> Type` | `ForAll(F, Type0 -> Type0, body)` | 1+3 |
| Constraint `<A: Eq>` | extra dict `Lam(dict: {eq: A -> A -> Bool}, …)` | 5 |
| Witness `Eq @Int :: { … }` | `Record([("eq", Var(eqIntImpl))])` | 5 |
| `} derive` / `@T :: derive` | elaboration emit; no new nodes | 5 |
| Recursive type function (`Loop :: Type -> Type { \| T => Loop T; }`) | `TyLamK` + `TyApp` + NbE fuel | 2 |
| Type equality by normalization | NbE kernel | 2 |
| Universe levels `Type 0 : Type 1` | `Kind::Type(ℓ)` | 1 |
| Type-level `select` | elaboration → `RecordT` projection | 3 |
| `perform`/`handle`/`resume` | free-monad elaboration (§9) | 4 |
| Effect row `! { fail ParseError }` | `Fun(…, RExtend("fail", ParseError, REmpty))` | 4 |
| Effectful function type `A -> B ! ε` | `Fun(A, B, eff_row)` | 4 |

---

## 17. Related documents

- **`docs/dataflow-core.md`** — the next compile stage; the TLC→DC lowering pass lives in `zutai-dataflow::lower`.
- **`docs/v0-implementation-roadmap.md`** — Phase 2 is the TLC phase; this document is the canonical spec for it.
- **`docs/v0_spec/`** — source of truth for v0 syntax and semantics; every v0 construct has a Phase 0 encoding in §16.
- **`docs/v1_spec/`** — design context for v1 features; all v1 constructs in §16 are expressible as elaboration in Phases 1–5.
