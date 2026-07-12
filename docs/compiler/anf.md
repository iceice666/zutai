# Administrative Normal Form (ANF) IR

ANF is the linear intermediate representation that sits between Dataflow Core (DC) and SSA in the Zutai compilation pipeline.

## Pipeline position

```
Source → HIR → THIR → TLC
                        ↓  TLC→DC: tree-to-graph, sharing, recursion explicit
                   Dataflow Core
                        ↓  DC→ANF: topo-sort SCCs, name every node, let/letrec
                       ANF
                        ↓  ANF→SSA: basic blocks, phi-nodes
                       SSA
                        ↓  SSA→LLVM
                    LLVM IR
```

## Purpose

Dataflow Core is a directed graph: a node may have multiple in-edges (multiple consumers share one node), and global back-edges (cycles) represent recursion. ANF converts the graph into a linear sequence of named bindings — every intermediate value has a unique name, function arguments are always atoms, and the dependency structure is made explicit in the binding order.

General source effects are not an ANF feature. Handler-passing CPS elaboration
means ANF sees ordinary control/data dependencies: applications, matches,
variant/record construction, and `letrec` handler interpreters. The sole host
boundary operation, `io.print`, arrives from Dataflow Core as `HostPrint` and is
scheduled like any other named binding; ANF must not add a second effect
sequencing convention.

The ANF representation makes three things easy for downstream passes:
- **SSA**: basic blocks, phi-nodes, and register allocation are straightforward when every sub-expression is already named.
- **Dead-code elimination**: a binding with no uses can be dropped (the ANF form makes uses explicit).
- **Inlining**: substituting an atom for a name is a one-step rewrite.

## Core grammar

```
module  ::= decl* body

decl    ::= let  name = body
          | letrec { (name = body)+ }

body    ::= (name = expr ;)* atom    -- sequence of let-bindings + final atom

atom    ::= name                     -- variable reference
          | lit                      -- literal constant
          | global-name              -- top-level global reference

expr    ::= atom                     -- trivial expression
          | atom atom                -- curried function application (args must be atoms)
          | atom [ty, ...]           -- type application
          | λ name . body            -- lambda abstraction
          | Λ [tyvar, ...] . body    -- type abstraction
          | { field = atom; ... }    -- record literal
          | ( item, ... )            -- tuple literal
          | [ atom; ... ]            -- list literal
          | atom . field             -- record field selection
          | match atom { arm* }      -- pattern match
          | atom ?? atom             -- optional coalesce
          | atom ⊕ atom              -- binary built-in operation
          | atom#tag                 -- variant construction
          | error                    -- error sentinel

arm     ::= pat (guard body)? => body

pat     ::= _                        -- wildcard
          | lit                      -- literal match
          | #atom                    -- atom tag match
          | name                     -- binding (introduces a name in scope)
          | ( pat-item, ... )        -- tuple deconstruction
          | { field: pat; ... }      -- record deconstruction
          | #tag pat                 -- variant deconstruction
```

**Key ANF invariant:** every argument to a function application, binary operation, match scrutinee, record field, tuple element, list element, and coalesce operand must be an *atom* (variable reference, literal, or global name). Complex sub-expressions are always lifted to a preceding `name = expr` binding.

Posit literals/types are carried from Dataflow Core unchanged inside `DfLit`/`DfTy`. The backend still uses one `i64` slot for every SSA value: p32 posits occupy the low 32 bits, and p64 posits occupy the full 64 bits.

## DC → ANF lowering

The lowering pass has three stages:

### Stage 1 — SCC analysis

Build a global dependency graph:
- For each top-level global `G` in the DC `globals` map, collect all `GlobalRef(name)` nodes reachable from `G`'s root node (transitively, including inside nested lambdas). The resulting edges form a directed graph: `G → name` for each referenced global `name`.

Compute strongly connected components (SCCs) using Tarjan's algorithm. Tarjan naturally emits SCCs in **forward topological order**: because it recurses into dependencies before completing the current SCC root, a dependency's SCC is output before its dependents. No explicit reversal step is needed.

### Stage 2 — Letrec decision

For each SCC:
- **Size > 1** (mutually recursive) → emit `letrec`.
- **Size = 1 with a self-edge** (directly recursive) → emit `letrec`.
- **Size = 1, no self-edge** (non-recursive) → emit `let`.

A self-edge exists when global `G`'s reachable `GlobalRef` set contains `G` itself.

### Stage 3 — Node lowering

Each global's root DC node is lowered into an `AnfBody`: a sequence of `(name, expr)` bindings followed by a result atom.

**Atoms (no binding introduced):**
- `Lit(l)` → atom `l` directly.
- `Bind` → atom `Var(param_name)` where `param_name` is the variable name assigned to this `Bind` node.
- `GlobalRef(name)` → atom `Global(name)`.

**Complex nodes (introduce a fresh binding `_anfN`):**
- `Apply { func, arg }` → lower both to atoms, emit `_anfN = func_atom arg_atom`.
- `TyApp { poly, ty_args }` → lower `poly` to atom, emit `_anfN = poly_atom[ty, ...]`.
- `Lambda { param, body }` → assign `param` a fresh name, lower `body` as a new `AnfBody` scope, emit `_anfN = λ param_name . body`.
- `TyLam { ty_params, body }` → lower body in fresh scope, emit `_anfN = Λ[tvars] . body`.
- `Record(fields)` → lower each field value to atom, emit `_anfN = { f1 = a1; ... }`.
- `Tuple(items)` → lower each item to atom, emit `_anfN = (a1, a2, ...)`.
- `List(elems)` → lower each element to atom, emit `_anfN = [a1; a2; ...]`.
- `Select { base, field }` → lower base to atom, emit `_anfN = base_atom.field`.
- `Match { scrutinee, arms }` → lower scrutinee to atom, lower each arm's guard and body in fresh scope, emit `_anfN = match scrutinee_atom { ... }`.
- `Coalesce { value, fallback }` → lower both to atoms, emit `_anfN = value_atom ?? fallback_atom`.
- `Builtin(op, lhs, rhs)` → lower both to atoms, emit `_anfN = lhs_atom ⊕ rhs_atom`.
- `Variant(tag, value)` → lower value to atom, emit `_anfN = value_atom#tag`.
- `Import`, `Error` → emit `_anfN = error` (defensive; these do not appear in well-typed programs).

**Sharing within a body scope:** when lowering a Lambda body, a memoization table (`memo: HashMap<NodeId, AnfAtom>`) prevents re-lowering the same DC node twice within that scope. If a node has already been lowered (and its result bound to a name), subsequent uses return the same atom — no duplicate bindings. The memo is reset when entering a nested Lambda body, so sharing does not propagate across lambda scopes (cross-lambda sharing is deferred to a future CSE pass).

**Lambda parameter naming:** every DC `Bind` node is assigned a unique name (`_bindN` where N is the raw node index) before lowering the scope that references it. The name assignment for a `Lambda`'s `param` happens before recursing into the `body`. For match-arm patterns, all `Bind` nodes in the pattern are named before the arm's guard and body are lowered.

## IR types

```rust
pub enum AnfAtom {
    Var(String),       // local variable or lambda parameter
    Lit(DfLit),        // literal constant
    Global(String),    // top-level global reference
}

pub struct AnfBody {
    pub bindings: Vec<(String, AnfExpr)>,
    pub result: AnfAtom,
}

pub enum AnfExpr {
    Atom(AnfAtom),
    Apply { func: AnfAtom, arg: AnfAtom },
    TyApp { poly: AnfAtom, ty_args: Vec<DfTyId> },
    Lambda { param: String, body: AnfBody },
    TyLam { ty_params: Vec<DfTyVar>, body: AnfBody },
    Record(Vec<(String, AnfAtom)>),
    Tuple(Vec<AnfTupleItem>),
    List(Vec<AnfAtom>),
    Select { base: AnfAtom, field: String },
    Match { scrutinee: AnfAtom, arms: Vec<AnfArm> },
    Coalesce { value: AnfAtom, fallback: AnfAtom },
    Builtin { op: DfBuiltinOp, lhs: AnfAtom, rhs: AnfAtom },
    Variant { tag: String, value: AnfAtom },
    Error,
}

pub struct AnfArm {
    pub pattern: AnfPattern,
    pub guard: Option<AnfBody>,
    pub body: AnfBody,
}

pub enum AnfPattern {
    Wildcard,
    Lit(DfLit),
    Atom(String),
    Bind(String),
    Tuple(Vec<AnfTuplePatItem>),
    Record(Vec<(String, AnfPattern)>),
    Variant(String, Box<AnfPattern>),
}

pub enum AnfDecl {
    Let { name: String, body: AnfBody },
    Letrec { bindings: Vec<(String, AnfBody)> },
}

pub struct AnfModule {
    pub decls: Vec<AnfDecl>,
    pub root: AnfBody,
    pub root_ty: DfTy,
}
```

## Invariants

A well-formed `AnfModule` satisfies:

1. **Atom-only arguments.** All arguments in `Apply`, `Builtin`, `Coalesce`, `Record`, `Tuple`, `List`, `Select`, `TyApp`, and `Variant` must be atoms (`AnfAtom`), not complex expressions.
2. **Def-before-use.** Within any `AnfBody`, every `Var(name)` or `Global(name)` atom appears as the LHS of a preceding binding in that `AnfBody`, or as a lambda parameter or match-arm binding in an enclosing scope, or is a declared global in `AnfModule.decls`.
3. **Letrec only for cycles.** A `Letrec` declaration is emitted if and only if the corresponding SCC contains a cycle: either a self-edge (`G` references itself) or more than one member.
4. **Fresh names.** All `_anfN` names introduced by the lowering pass are unique across the entire module. Lambda parameter names (`_bindN`) are unique by DC node index.

## Crate location

The ANF IR lives at `crates/general/anf/`. Its Cargo package name is `zutai-anf`. It depends on `zutai-dataflow` (for the DC IR input and `DfLit`, `DfBuiltinOp`, `DfTyId`, `DfTyVar` types).
