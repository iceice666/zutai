# Dataflow Core IR

The Dataflow Core (DC) is the graph-based intermediate representation that sits between TLC (Type Lambda Calculus) and Administrative Normal Form (ANF).

## Pipeline position

```
Source → HIR → THIR → TLC
                        ↓  [TLC→DC: tree-to-graph, sharing, explicit recursion]
                   Dataflow Core
                        ↓  [DC→ANF: topo-sort SCCs, name every node, introduce let/letrec]
                       ANF
                        ↓  [ANF→SSA: basic blocks, phi-nodes]
                       SSA
                        ↓  [SSA→LLVM: emit LLVM IR]
                    LLVM IR
```

THIR is the error-tolerant, source-preserving typed IR used by the language server (LSP) and diagnostics tooling. It carries spans on every node, tolerates partial type information, and is produced even when type checking fails partially. TLC (Type Lambda Calculus) is the fully-elaborated IR produced only when type checking succeeds: all inference variables are resolved, polymorphism is explicit via `TyLam`/`TyApp`, and complete type information is guaranteed. The Dataflow Core lowering takes TLC as its sole input. TLC is specified in [`docs/tlc-core.md`](tlc-core.md).

## Effect boundary

General source effect control is eliminated before Dataflow Core. TLC may contain
temporary `Perform`/`Handle`/`Resume`/`Sequence` markers, but TLC→DC accepts only
ordinary values plus the dedicated host-boundary `HostPrint` node for residual
ambient `io.print`.

- `Perform` for unsupported operations, `Handle`, `Resume`, and effect
  sequencing markers are compile/dataflow errors at this boundary.
- Closed `io.print` rows may lower through `HostPrint`; unsupported or open
  effect rows remain gated so DC callers cannot silently erase effects.
- Handler-passing CPS elaboration represents source handlers with existing
  `Lambda`, `Apply`, `Match`, `Record`, `Variant`, and recursion structure.

`HostPrint` is deliberately narrow: it evaluates its text operand, calls the
runtime `zutai.print_text` ABI, and returns the same text value. It is not a
general effect system in DC.

## Why a graph?

THIR is an expression tree: every sub-expression has exactly one parent. The Dataflow Core is a directed graph: a node may be referenced by any number of consumers. This changes what each step in the pipeline must do.

**Sharing.** When a THIR `Block` binds `x := normalizeServer raw.server` and then uses `x` three times, THIR stores three `BindingRef(x)` nodes (one per use-site). The DC represents all three uses as edges from three consumer nodes to a single `normalizeServer raw.server` node. CSE and dead-code elimination are both natural graph operations on this representation.

**Laziness.** Zutai general mode is pure and lazy. In a graph, a binding is evaluated only when its node is reachable from the root (the module output node). Unreachable nodes are never computed. The DC encodes laziness topologically rather than operationally — there are no thunk objects, just graph connectivity.

**Recursion.** A recursive function like `factorial` creates a back-edge: the Lambda body contains a `GlobalRef(factorial)` node, which the graph-level view connects back to the `factorial` global's own Lambda node, forming a cycle. The DC→ANF phase identifies these cycles via SCC analysis and emits `letrec` bindings for them. A non-cyclic graph edge becomes a plain `let`.

## Node representation

Each node has a unique `NodeId` (an index into the arena), a type `TyId`, and a `DfNodeKind`. A parallel `spans` table maps `NodeId` → `Span` for nodes that have source locations (used for LLVM debug info and error attribution).

```
NodeId  = u32

DfNode {
    ty:   TyId,
    kind: DfNodeKind,
}

spans: Vec<Option<Span>>   // indexed by NodeId, same length as node arena
```

### Node kinds

```
DfNodeKind:

  -- Leaves --
  Lit(Lit)
    A literal constant: Bool, Int, Float, Posit, Text, or Atom.
    Lit = Bool(bool) | Int(i64) | Float(f64) | Posit { nbits: u8, es: u8, bits: u64 } | Text(String) | Atom(Symbol)

  Bind
    A binding site. Introduced by Lambda (parameter) and by match-arm Bind patterns.
    Has no children. Consumer nodes reference this node's ID to use the bound value.
    Every Bind node is owned by exactly one Lambda or DfArm.

  GlobalRef(Symbol)
    A reference to a named top-level definition.
    When the referenced global is the definition currently being lowered, this
    creates a back-edge (cycle) in the graph, which the ANF phase handles as letrec.

  Import { path: String, kind: ImportKind }
    An external data source.
    ImportKind = Zti | Zt

  Error
    Propagated error sentinel from THIR diagnostics.
    Typed as the error type; used to allow lowering to continue past errors.

  -- Abstraction and application --
  Lambda { param: NodeId, body: NodeId }
    A function value. `param` must be a Bind node.
    The body sub-graph may contain edges to `param` (representing uses of the parameter)
    and edges to Bind nodes of enclosing Lambdas (closure captures).
    Curried functions nest Lambdas: `\x y. body` = Lambda(x, Lambda(y, body)).

  Apply { func: NodeId, arg: NodeId }
    Curried function application. `f x y` = Apply(Apply(f, x), y).

  -- Host boundary --
  HostPrint { arg: NodeId }
    Runtime `io.print` dispatch. `arg` must be `Text`; lowering evaluates it,
    calls the runtime text-print ABI, and returns the same `Text` value. This is
    the only ambient host operation represented in Dataflow Core.

  -- Type polymorphism --
  TyLam { ty_params: Vec<TyVar>, body: NodeId }
    Type abstraction. Wraps a polymorphic Lambda: `<A, B> \x. body`.
    Produced for every function declaration with `<...>` type parameters.

  TyApp { poly: NodeId, ty_args: Vec<TyId> }
    Type application. Instantiates a TyLam at a call site.
    Implicit in v0 source (no explicit type application syntax); inserted by TLC→DC
    lowering when a polymorphic GlobalRef is applied to arguments with inferred types.

  -- Data construction --
  Record(Vec<(Symbol, NodeId)>)
    Record literal. Each field is a (name, value-node) pair.

  Tuple(Vec<TupleNodeItem>)
    Tuple literal.
    TupleNodeItem = Named { name: Symbol, value: NodeId } | Positional(NodeId)

  List(Vec<NodeId>)
    List literal. All elements must have the same type.

  -- Data elimination --
  Select { base: NodeId, field: Symbol }
    Record field projection. Returns the field's value.
    For optional fields, returns `Maybe(T)`.

  Match { scrutinee: NodeId, arms: Vec<DfArm> }
    Pattern-matching / case analysis.
    Arms are tested top-to-bottom; the first matching arm's body is the result.
    Used both for explicit `match` expressions and for multi-clause function dispatch.

  Coalesce { value: NodeId, fallback: NodeId }
    Wrapper unwrap: `value ?? fallback`.
    `value` must have type `Optional(T)` or `Maybe(T)`; `fallback` must have type `T`.
    Result has type `T`.
```

### Match arms

```
DfArm {
    pattern: DfPattern,
    guard:   Option<NodeId>,   // Bool-typed expression; None = always matches
    body:    NodeId,
}

DfPattern:
  Wildcard
  Lit(Lit)
  Atom(Symbol)
  Bind(NodeId)             -- NodeId must be a Bind node owned by this arm
  Tuple(Vec<DfTuplePatItem>)
  Record(Vec<(Symbol, DfPattern)>)

DfTuplePatItem:
  Named    { name: Symbol, pattern: DfPattern }
  Positional(DfPattern)
```

A `Bind(n)` pattern introduces a new `Bind` node `n` that is in scope for the arm's guard and body. The arm "owns" all `Bind` nodes introduced by its pattern.

## Type representation

DC types are copied from THIR's type arena and extended with type-level lambdas:

```
TyKind:
  -- Primitives --
  Bool | Int | Float | Posit(nbits: u8, es: u8) | Text | Atom(Symbol)
  True | False          -- singleton types for union arm discrimination


  -- Composite --
  List(TyId)
  Optional(TyId)
  Maybe(TyId)
  Record(Vec<TyRecordField>)       -- TyRecordField { name, optional, ty }
  Union(Vec<TyId>)
  Tuple(Vec<TyTupleItem>)          -- TyTupleItem = Named{name,ty} | Positional(ty)
  Fun(TyId, TyId)                  -- single-argument arrow (curried)

  -- Polymorphism --
  TyVar(TyVar)                     -- a bound type variable (named, from source)
  TyFun(Vec<TyVar>, TyId)          -- type-level lambda: <A, B> => body
  TyApp(TyId, Vec<TyId>)           -- type application: Pair Text Int

  -- Meta --
  Type                             -- the type of type-valued expressions
  Error                            -- error sentinel
```

Posit values use the backend's universal `i64` value representation: p32 occupies the low 32 bits, and p64 uses all 64 bits. Arithmetic and comparison operations carry `(nbits, es)` metadata so LLVM can call the matching external helper.

Type variables in `TyVar` are named (carrying the `BindingId` from THIR) rather than De Bruijn indexed, to preserve readability in error messages and debug output.

`TyFun` is the Dataflow Core representation of a generic type alias:
```zt
Pair :: <A, B> type { first: A; second: B; };
```
becomes `TyFun([A, B], Record([("first", TyVar(A)), ("second", TyVar(B))]))`.

`TyApp` is how a generic type is used at a concrete instantiation:
```zt
pair :: Pair Text Int = { first = "hello"; second = 42; };
```
The type of `pair` is `TyApp(TyFun([A, B], ...), [Text, Int])`, which the type normalizer reduces to `Record([("first", Text), ("second", Int)])`.

## Graph structure

```
DataflowGraph {
    nodes:   Arena<DfNode>,           -- NodeId is the index
    types:   Arena<DfTy>,             -- TyId is the index
    globals: IndexMap<Symbol, NodeId>,-- ordered top-level definitions
    root:    NodeId,                  -- the module's output node
    spans:   Vec<Option<Span>>,       -- parallel to nodes arena
}
```

`globals` maps each top-level declared name to its node. The order reflects declaration order, which is semantically irrelevant (Zutai is lazy) but used for deterministic output.

`root` is the NodeId for the module's final expression (the last expression in a `.zt` file, which is the module's output value).

## TLC → DC lowering

The lowering pass walks a `TlcModule` and builds a `DataflowGraph`. The key invariant is:

> **Each TLC local binding is lowered exactly once. All references to that binding become edges to the same DC NodeId.**

This is the tree-to-graph transformation. In TLC, two uses of the same binding are two separate `Var` nodes. In DC, they are two edges pointing to a single node.

### Local binding table

The lowerer maintains a `HashMap<BindingId, NodeId>` mapping local bindings to their DC nodes. When a THIR `Block { bindings, result }` is lowered:

1. For each `binding` in `bindings`:
   - Lower the binding's value expression to a DC node `v`.
   - Insert `binding.id → v` in the table.
2. Lower `result` with the extended table.
3. Return `result`'s NodeId. The "block" node disappears; its bindings are just nodes in the graph now.

### Global bindings

Top-level declarations are lowered into the `globals` map. When a top-level `BindingRef` is encountered during lowering:
- If the referenced binding is a global → emit `GlobalRef(name)`.
- If the referenced binding is a local → look up its NodeId in the table and use it directly (no GlobalRef — this is the sharing).

### Function declarations

A function `f :: <A, B> A -> B -> A` with clause `= x _ => x;` is lowered as:

1. Create `Bind` node `p1` (parameter `x`, type `A`).
2. Create `Bind` node `p2` (parameter `_`, type `B`).
3. Lower the clause body referencing `p1` as the return value → body_node = `p1`.
4. Wrap: `Lambda { param: p2, body: body_node }` → inner_lambda.
5. Wrap: `Lambda { param: p1, body: inner_lambda }` → outer_lambda.
6. If polymorphic: `TyLam { ty_params: [A, B], body: outer_lambda }` → final_node.
7. Insert `globals["const"] = final_node`.

### Multi-clause functions

A function with multiple clauses is desugared into a Match on a synthetic tuple of all parameters:

```zt
area :: Shape -> Float
  = (#circle, radius = r) => r * r * 3.14159;
  = (#square, length = l) => l * l;
  = (#rect, width = w, height = h) => w * h;
```

Becomes (conceptually):
```
Lambda {
    param: p (Shape),
    body: Match {
        scrutinee: p,
        arms: [
            DfArm { pat: Tuple(Atom(#circle), Named(radius, Bind(r))), body: ... },
            DfArm { pat: Tuple(Atom(#square), Named(length, Bind(l))), body: ... },
            DfArm { pat: Tuple(Atom(#rect), Named(width,Bind(w)), Named(height,Bind(h))), body: ... },
        ]
    }
}
```

For multi-argument multi-clause functions, parameters are collected into a single synthetic tuple as the scrutinee. Each arm's pattern destructs that tuple. The synthetic tuple itself may be optimized away by the ANF/SSA passes.

### Recursive functions

When lowering a global `factorial`:

1. Before lowering the body, insert a placeholder `GlobalRef("factorial")` in the lowerer's global-in-progress set.
2. Lower the body. When a `BindingRef(factorial_binding)` is encountered, emit `GlobalRef("factorial")`.
3. This `GlobalRef` node's implicit edge to `globals["factorial"]` (which is being constructed) creates a cycle.
4. Complete `globals["factorial"] = Lambda { ... }`.

The cycle is not an error. The DC→ANF pass will detect it as an SCC and emit a `letrec`.

Mutually recursive definitions (`even` and `odd`) work identically: both `globals["even"]` and `globals["odd"]` contain `GlobalRef` nodes pointing to each other, forming a 2-node SCC.

## DC → ANF lowering (overview)

The ANF pass converts the DC graph to a linear let/letrec schedule:

1. **SCC analysis.** Compute SCCs of the global dependency graph (globals and their `GlobalRef` edges).
2. **Topological sort.** Sort SCCs so that each SCC depends only on earlier SCCs.
3. **Emit bindings.** For each SCC:
   - Single-node, no self-loop → `let name = lower(node)`.
   - Single-node, self-loop, or multi-node → `letrec { name₁ = ...; name₂ = ...; }`.
4. **Name all sub-expressions.** Every non-trivial sub-expression in a node's lowering gets a fresh ANF name. `Apply(Apply(f, x), y)` becomes `let t1 = f x; let t2 = t1 y; ...`.

The full ANF design lives in `docs/anf.md` (to be written when that phase begins).

## Invariants

The following invariants must hold in a well-formed DataflowGraph:

1. **Type consistency.** Every NodeId referenced inside a node's `DfNodeKind` must be present in the arena and have a compatible type with the context in which it appears.
2. **Bind ownership.** Every `Bind` node is referenced as `param` by exactly one `Lambda`, or as `Bind(n)` in exactly one `DfPattern` within one `DfArm`. A `Bind` node may be used as an expression result when it is in lexical scope; ownership is tracked separately from use.
3. **Arm bind scope.** A `Bind` node introduced in a `DfArm`'s pattern is only referenced within that arm's `guard` or `body`.
4. **Lambda capture.** A `Bind` node `p` owned by `Lambda L` may be referenced by any node in `L`'s body sub-graph, including nodes inside nested Lambdas (closure capture). It must not be referenced outside `L`.
5. **No stray GlobalRef.** Every `GlobalRef(name)` must name a key present in `globals` (after the full module is lowered — cycles are fine, dangling references are not).
6. **Span table size.** `spans.len() == nodes.len()`.

Structural invariants (1, 2, 5, 6 plus reference/type-shape bounds) are checked in every build — including release — by `validate_structural`, which is O(n) with no scope cloning. A refused compile beats a wrong binary. The lambda-capture/arm-bind scope walk (invariants 3 and 4) is O(node × scope) and remains debug-only, run by the full `validate`.

## Crate location

The Dataflow Core lives at `crates/general/dataflow/`. Its Cargo package name is `zutai-dataflow`. It depends on `zutai-tlc` (for the lowering input) and `zutai-syntax` (for `Span`).
