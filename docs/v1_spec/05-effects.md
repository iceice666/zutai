# Algebraic Effects (v1)

Algebraic effects extend Zutai's pure v0 core with explicit, typed effectful computation. Effects are not ambient primitives: an effect operation runs only when code uses `perform`, and the operation must be handled or provided by a host boundary.

The v1 effect syntax uses four reserved words:

```zt
perform
handle
with
resume
```

---

## Effect Rows

Pure functions remain the default:

```zt
normalize :: RawConfig -> Config
```

An effectful function appends `!` and an effect row to its result type:

```zt
parse :: Text -> Config ! { fail ParseError }
check :: Config -> Config ! { warn Diagnostic }
read :: Path -> Text ! { fs.read : Path -> Text, fail IOError }
```

No `!` suffix means the function is pure and cannot perform effects.

An effect row contains operation names. Operation names may be plain identifiers or dotted capability operation names. Operations may use compact standard aliases such as `fail ParseError`, or explicit operation types such as `fs.read : Path -> Text`.

---

## Performing Operations

`perform` invokes an effect operation:

```zt
perform fail err
perform warn diagnostic
perform ask ()
```

The operation must appear in the surrounding function's effect row unless it is handled locally.

Standard aliases are:

```zt
fail E == fail : E -> Never
warn D == warn : D -> Unit
log D  == log : D -> Unit
ask R  == ask : Unit -> R
```

`Never` is the result type of an operation that does not resume normally.

---

## Handling Operations

`handle expr with { ... }` evaluates `expr` under a handler:

```zt
handle parse text with {
  value = \cfg => (#ok, value = cfg);
  fail = \err => (#err, error = err);
}
```

The `value` field handles the final value produced by the handled expression. `value` is a special handler field name, not a new keyword. If omitted, it defaults to identity.

Operation fields handle performed operations:

```zt
handle check cfg with {
  warn = \diagnostic => {
    perform log diagnostic;
    resume ();
  };
}
```

`resume expr` is valid only inside an operation handler clause. It resumes the suspended computation with `expr` as the result of the operation.

Handler clauses may either resume the suspended computation or return directly from the handler:

```zt
handle requirePresent field with {
  fail = \err => {
    (#missing, error = err)
  };
}
```

Resumptions are one-shot in v1: an operation handler clause may call `resume` at most once.

---

## Effect Row Handling

A handler removes the effects it handles and forwards effects it does not handle.

For example, this function handles `fail` but still forwards `log`:

```zt
parseOrDefault :: Text -> Config -> Config ! { log ParseError }
  = text fallback =>
    handle parse text with {
      value = \cfg => cfg;
      fail = \err => {
        perform log err;
        fallback
      };
    };
```

The handled expression may perform `fail ParseError`, and the handler body may perform `log ParseError`. After the handler, `fail ParseError` is removed from the surrounding effect row, while `log ParseError` remains.

---

## Capabilities and Authority

Authority-sensitive effects require explicit capability values in addition to effect rows:

```zt
load :: FsRead -> Path -> Text ! { fs.read : Path -> Text, fail IOError }
  = fs path => perform fs.read path;
```

The effect row records what may happen. The capability argument records who authorized it. Host-provided capabilities include operations such as filesystem access, networking, environment access, clocks, and randomness.

The pure core still has no ambient forms such as direct filesystem reads, environment lookup, clocks, or randomness. Those operations must be expressed as effects and authorized with capabilities.

---

## Laziness and Ordering

Effectful computations are not ordinary inert data. Phase 16 fixes the
reference ordering model as an explicit sequencing boundary:

- `perform op arg` first evaluates `arg`, then suspends the nearest enclosing
  handled computation at the operation point.
- `handle expr with clauses` evaluates `expr` under the operation clauses.
  When `expr` completes normally, the optional `value` clause runs last; if it
  is omitted, the produced value is returned unchanged.
- An operation clause receives the operation payload. It may return directly as
  the whole handler result, or call `resume value` once.
- `resume value` first evaluates `value`, then re-enters the suspended
  continuation with that value as the result of the original `perform`. The
  continuation includes the handler's `value` clause, so `resume` itself has the
  handler result type.
- Sequence expressions evaluate left-to-right. This is the only ordering
  guarantee introduced for the otherwise pure lazy core; pure data construction
  remains demand-driven outside explicit effect sequencing.
- Top-level non-function value bindings may not have an effect type. This keeps
  module initialization from firing effects for unused values. Effectful
  top-level functions are inert until called because evaluating the binding only
  creates a closure.

Compilation keeps Dataflow Core, ANF, SSA, and LLVM pure in this phase. TLC
carries dedicated `perform`/`handle`/`resume`/sequence markers for the reference
evaluator. The compile/dataflow commands reject any residual effect marker or
non-empty function effect row after TLC lowering, so LLVM support is not claimed
until an effect lowering exists past TLC.
