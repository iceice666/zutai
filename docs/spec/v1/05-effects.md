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

Effect rows may also spread a named effect type alias, including a qualified
type alias exported by an imported module. This keeps larger rows readable while
preserving the same checked operation set:

```zt
FsReadEffects :: type Unit ! { fs.read : Path -> Text; };
FsWriteEffects :: type Unit ! { fs.write : WriteAllRequest -> Unit; };
FsFileEffects :: type Unit ! { ...FsReadEffects; ...FsWriteEffects; };
LogEffects :: type Unit ! { log Text; };
fs ::= import stdlib.fs;

loadAndLog :: Path -> Text ! { ...fs.WholeReadEffects; ...LogEffects; }
```

A final open row tail can still follow named or qualified spreads for
row-polymorphic signatures:

```zt
withRead :: <A, e> (Path -> A ! { ...FsReadEffects; ...e; }) -> A ! { ...FsReadEffects; ...e; }
```

For result-position reuse, define an effectful type alias:

```zt
FsFile :: <A> type A ! { ...FsFileEffects; };
main :: { read : FsRead; write : FsWrite; } -> FsFile Text
```

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

The effect row records what may happen. The capability argument records who
authorized it. In the current implementation, `io.print` is the only host
capability provided by the default `run` boundary. Filesystem access,
networking, environment access, clocks, and randomness are reserved for explicit
capability values; they are not ambient prelude primitives and are not
implicitly available to compiled code.

The pure core still has no ambient forms such as direct filesystem reads,
environment lookup, clocks, or randomness. Those operations must be expressed as
effects and authorized with capability values.

See [host capabilities](../../v2_spec/02-host-capabilities.md) for the v2 design that makes these capabilities real.

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

Compilation keeps general source effect control out of the backend. The
implemented v1 path uses pre-DC handler-passing CPS elaboration:
`perform`/`handle`/`resume`/sequence markers become ordinary functions,
applications, matches, records, variants, and recursive handler structure before
TLC→DC. Compile/dataflow still reject residual unsupported operations, open or
unsupported effect rows, and effectful entry shapes the runtime ABI cannot show.
The supported ambient host exception is `io.print`: residual `io.print` lowers
to the runtime `HostPrint`/`zutai.print_text` path and returns the printed text.
