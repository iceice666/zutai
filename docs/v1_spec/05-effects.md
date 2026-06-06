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
parseOrDefault | text fallback =>
  handle parse text with {
    value = \cfg => cfg;
    fail = \err => {
      perform log err;
      fallback
    };
  }
```

The handled expression may perform `fail ParseError`, and the handler body may perform `log ParseError`. After the handler, `fail ParseError` is removed from the surrounding effect row, while `log ParseError` remains.

---

## Capabilities and Authority

Authority-sensitive effects require explicit capability values in addition to effect rows:

```zt
load :: FsRead -> Path -> Text ! { fs.read : Path -> Text, fail IOError }
load | fs path => perform fs.read path
```

The effect row records what may happen. The capability argument records who authorized it. Host-provided capabilities include operations such as filesystem access, networking, environment access, clocks, and randomness.

The pure core still has no ambient forms such as direct filesystem reads, environment lookup, clocks, or randomness. Those operations must be expressed as effects and authorized with capabilities.

---

## Laziness and Ordering

Effectful computations are not ordinary inert data. A v1 implementation must preserve explicit ordering around `perform`, `handle`, `with`, and `resume` so effects do not run merely because a lazy value is demanded unpredictably.

Detailed forcing and sequencing rules are intentionally left as a v1 implementation design constraint. The committed surface rule is that effect execution is explicit: effects are introduced by `perform` and controlled by handlers or host entrypoints.
