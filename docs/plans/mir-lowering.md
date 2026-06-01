# MIR Lowering — Implementation Guide

This document is the implementation reference for `zutai_mir::lower::lower_module`.
Read it before touching `crates/general/mir/src/lower/mod.rs`.

## Pipeline context

```
HirFile (zutai-hir)          — name-resolved, desugared, near-surface HIR
    ↓  lower_module           ← this document
MirModule (zutai-mir)        — ANF, explicit closures, compiled patterns
    ↓  emit_llvm / eval
LLVM IR / bytecode interpreter
```

The MIR's job is to bridge the gap between "what the programmer wrote
(desugared)" and "what LLVM needs to see":

| HIR (rich, tree-shaped)           | MIR (flat, CFG, ANF)                     |
|-----------------------------------|------------------------------------------|
| Nested `Apply(Apply(f, a), b)`    | `v1=Call(f,a); v2=Call(v1,b)`            |
| Implicit closure captures         | Explicit `MakeClosure(func, env_record)` |
| `Match` with n arms               | Decision tree of `Switch` terminators    |
| Lazy semantics (spec §…)          | `MakeThunk(expr)` / `Force(thunk)`       |
| Generic polymorphism              | Monomorphized copies or uniform boxing   |

---

## A-Normal Form (ANF) transformation

**Invariant**: after lowering expression `e`, every sub-expression of `e` is
bound to its own `MirVar`. No `MirInstr::Call` may have a nested call as an
argument.

### Algorithm

Maintain two outputs from `lower_expr`:
- `result: MirVar` — the variable holding `e`'s value.
- `current_block: MirBlockId` — the block to append future instructions to
  (may change if `e` introduces control flow).

```
lower_expr(e, func, block, module) -> (MirVar, MirBlockId):

  case e of

    Literal(v):
      dest = func.fresh_var()
      block.instrs.push(Const { dest, val: MirConst::from(v) })
      return (dest, block)

    Var(sym_id):
      // The lowering context maps SymbolId → MirVar.
      // Look up the variable in the current scope map.
      return (scope[sym_id], block)

    Let { name, value, body }:
      (val_var, block) = lower_expr(value, func, block, module)
      scope[name] = val_var
      return lower_expr(body, func, block, module)

    Apply { fun, arg }:
      (fun_var, block) = lower_expr(fun, func, block, module)
      (arg_var, block) = lower_expr(arg, func, block, module)
      dest = func.fresh_var()
      block.instrs.push(Call { dest, func: fun_var, arg: arg_var })
      return (dest, block)

    If { cond, then, else_ }:
      (cond_var, block) = lower_expr(cond, func, block, module)
      then_block  = func.alloc_block()
      else_block  = func.alloc_block()
      join_block  = func.alloc_block()
      result_var  = func.fresh_var()
      join_block.params = [result_var]

      block.terminator = Switch {
          scrutinee: cond_var,
          arms: [(MirTest::Bool(true), then_block)],
          default: else_block,
      }

      (then_var, then_end) = lower_expr(then, func, then_block, module)
      then_end.terminator = Jump { target: join_block, args: [then_var] }

      (else_var, else_end) = lower_expr(else_, func, else_block, module)
      else_end.terminator = Jump { target: join_block, args: [else_var] }

      return (result_var, join_block)

    Lambda { param, body }:
      // See §Closure conversion below.

    Match { scrutinee, arms }:
      // See §Pattern-match compilation below.
```

---

## Closure conversion

Zutai functions have lexical scope. A `Lambda` can reference variables from
enclosing scopes. MIR makes these captures **explicit**.

### Free-variable analysis

Before lowering a `Lambda`, compute its free variables:

```
free_vars(expr, bound) -> Set<SymbolId>:
  case expr of
    Var(sym) if sym not in bound   -> {sym}
    Var(sym)                        -> {}
    Lambda { param, body }          -> free_vars(body, bound ∪ {param})
    Let { name, value, body }       -> free_vars(value, bound)
                                     ∪ free_vars(body, bound ∪ {name})
    Apply { fun, arg }              -> free_vars(fun, bound) ∪ free_vars(arg, bound)
    // … recurse into all sub-expressions …
```

### Emission

For each `Lambda { param, body }` in HIR:

1. Allocate a new `MirFunc` for the closure body.
2. Its params are `[param, env]` where `env` holds the captured variables.
   (Or: treat `env` as a record-shaped `MirVar` passed at call-site.)
3. At the top of the closure body, emit `LoadCapture` instructions to
   reconstruct each free variable from `env`.
4. Lower the body normally with the reconstructed scope.
5. At the call site, emit `MakeClosure { func: new_func_id, env: env_record }`.

**Key choice** (see §Open questions): does `env` use a typed struct (one type
per closure signature) or a uniform `*mut ()` passed as an extra argument?
The former is safer for LLVM; the latter is simpler to implement first.

---

## Pattern-match compilation (Maranget algorithm)

HIR `Match { scrutinee, arms }` compiles to a tree of `Switch` terminators.

### Concepts

A **pattern matrix** is a 2D grid where rows are match arms and columns are
the components of the scrutinee. Maranget's algorithm works column-major:

1. **Specialization**: for each constructor `C` of the scrutinee's type,
   produce a sub-matrix that only includes arms whose first column matches `C`,
   with the `C`-field columns prepended.
2. **Default matrix**: rows that begin with a wildcard (`_` or bind pattern),
   minus the first column.
3. Recurse on sub-matrices; base case is an empty matrix (no more arms →
   unreachable) or an empty column list (all arms covered → emit the arm body).

### MIR emission

Each recursive call to `compile_match` produces:
- A `Switch { scrutinee, arms, default }` terminator in the current block.
- Each arm's target block is recursively compiled.

```
compile_match(scrutinee, pattern_matrix, func, block, module):
  if pattern_matrix.rows is empty:
    block.terminator = Unreachable
    return

  if pattern_matrix.cols is empty:
    // All patterns matched — emit the body of the first arm.
    (result, block) = lower_expr(first_arm.body, func, block, module)
    block.terminator = Jump(join_block, [result])
    return

  // Pick the first column (heuristics can optimize later).
  for each constructor C of the scrutinee's type:
    sub_matrix = specialize(pattern_matrix, C)
    C_block = func.alloc_block()
    // emit field-extraction instructions into C_block
    compile_match(fields_of_C, sub_matrix, func, C_block, module)
    arms.push((MirTest::Tag(C), C_block))

  default_block = func.alloc_block()
  compile_match(scrutinee, default_matrix(pattern_matrix), func, default_block, module)

  block.terminator = Switch { scrutinee, arms, default: default_block }
```

### Guards

Per spec, guarded arms do **not** contribute to exhaustiveness coverage.
Lower a guard as: test `guard_expr`, if true emit arm body, if false fall
through to the next arm (emit as a sub-match on the remaining arms).

---

## Open questions — resolve before implementing

The following design decisions significantly affect the MIR shape. Each has
tradeoffs listed; pick one and be consistent.

### 1. Strict vs lazy

Zutai is spec'd as lazy (general mode is pure + lazy like Haskell). The
question is where laziness is implemented:

**Option A — Thunks in MIR**
- `MirInstr::MakeThunk { dest, func }` — wrap an unevaluated expression.
- `MirInstr::Force { dest, thunk }` — evaluate a thunk.
- Advantage: LLVM can emit actual thunk structs; interpreter is honest about laziness.
- Disadvantage: every expression emits `MakeThunk`/`Force` pairs; the MIR
  is much larger and harder to read during development.

**Option B — Strict MIR, lazy at interpreter level**
- MIR is evaluated eagerly. The interpreter (not MIR) handles thunks.
- Advantage: simpler MIR for v0; faster to get to a working interpreter.
- Disadvantage: LLVM backend will need a separate laziness pass later;
  MIR doesn't faithfully represent the language's semantics.

**Recommendation for v0**: Start with Option B (strict MIR). Add thunks when
you implement the LLVM backend or when you hit programs that require laziness
to terminate.

### 2. Monomorphize vs uniform boxing

Zutai has parametric polymorphism (`[A, B]` type params). At MIR level:

**Option A — Monomorphization**
- For each instantiation `f @ [Int]`, emit a separate `MirFunc` with `Int`
  substituted everywhere.
- Advantage: zero-cost generics like Rust/C++; LLVM sees concrete types.
- Disadvantage: code blowup; requires a monomorphization pass that reads
  `Symbol::ty` for every call site and substitutes type variables.

**Option B — Uniform representation (boxing)**
- All values are represented as `*mut MirValue` (a tagged union or object).
- Polymorphic functions work on opaque pointers; field access goes through
  a runtime tag check or vtable.
- Advantage: simple; no blowup; closures/records are natural.
- Disadvantage: dynamic dispatch overhead; LLVM cannot optimize across the
  opaque boundary without heroic effort.

**Option C — Dictionary passing**
- Translate `[A]` to a runtime dictionary argument carrying size/eq/hash ops.
- The Haskell approach for typeclasses; more relevant if constraints land.

**Recommendation for v0**: Option B (boxing) for the interpreter; plan for
Option A (monomorphization) for the LLVM backend. Design the `lower_module`
interface to accept a "monomorphize: bool" flag so you can switch later.

### 3. Closure representation

Two sub-questions:

**Environment shape**: typed struct per closure (one MIR type per distinct
free-variable set) vs uniform `*mut ()` with a runtime layout.
- Typed structs: safer, LLVM-friendly, but need a type-generation pass.
- Uniform pointer: simpler for v0 interpreter; harder for LLVM.

**Calling convention**: pass `env` as an explicit extra argument vs embed
it in the closure object itself.
- Extra arg: `Call(closure_func, arg)` also needs to pass `env` somehow —
  requires a wrapper function or a two-field struct `{ func_ptr, env }`.
- Embedded: `MakeClosure` produces an object; `Call` on a closure implicitly
  loads the env. Cleaner; slightly more MIR infrastructure.

**Recommendation**: Use a two-field struct `{ func_ptr: MirFuncId, env: Vec<MirVar> }`
represented as `MirInstr::MakeClosure`. This is what the current scaffold assumes.

### 4. Curried vs uncurried

Zutai functions are curried: `f : Int -> Text -> Bool` is `Int -> (Text -> Bool)`.

**Option A — Curried MIR** (current scaffold assumption)
- Each `Lambda` lowers to a `MirFunc` taking one argument and returning a closure.
- `f a b` → `v1=Call(f,a); v2=Call(v1,b)`.
- Faithful to the language; simple to implement.
- Disadvantage: two `Call` instructions for a two-arg function; overhead if
  LLVM doesn't inline the intermediate closure.

**Option B — Uncurried MIR with arity tracking**
- Track the arity of each function in `MirFunc`. Saturated calls emit a single
  `CallN { dest, func, args: Vec<MirVar> }` instruction.
- Partial application emits a closure capturing the partial args.
- More complex to implement (need an arity oracle from the type system) but
  produces cleaner LLVM IR.

**Recommendation**: Start with Option A (curried). Add arity-based uncurrying
as an optimization pass after the interpreter is working.

### 5. Int representation

HIR uses `i64` for integer literals (committed in Phase 3). MIR inherits this.
If Zutai ever needs arbitrary-precision integers, change `MirConst::Int(i64)` to
`MirConst::Int(BigInt)` — the change is local to `func.rs` and `mir-lowering`.

---

## Test strategy

1. **Unit tests on ANF**: given a hand-constructed `HirExpr` with nested calls,
   assert that `lower_expr` produces a flat list of `MirInstr::Call`s.
2. **Snapshot tests**: lower `.zt` fixtures and compare `MirModule` debug output
   to `expect_test` snapshots (like the HIR golden tests).
3. **Round-trip**: lower a program to MIR, evaluate it with the interpreter,
   compare output to direct HIR evaluation.
4. **Exhaustiveness**: every `Match` in a program known to be exhaustive should
   produce no `Unreachable` at the join; non-exhaustive matches should contain
   exactly one `Unreachable`.

---

## References

- Maranget, L. (2008). *Compiling Pattern Matching to Good Decision Trees.*
  Proceedings of the 2008 ACM SIGPLAN Workshop on ML. The canonical algorithm
  for pattern-match compilation.
- Flanagan et al. (1993). *The Essence of Compiling with Continuations.*
  Background on ANF vs CPS; ANF is strictly simpler for a first backend.
- GHC STG machine — the model for thunk representation in lazy languages.
  Relevant if you choose Option A (thunks in MIR) for the laziness question.
- `docs/v0_spec/` — language semantics; read §evaluation order before
  deciding between strict and lazy MIR.
