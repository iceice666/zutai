# Standard Library: Host Capabilities

`stdlib.env`, `stdlib.clock`, `stdlib.rng`, and `stdlib.load` are explicit source
modules over Zutai's existing host operations. They add aliases and thin wrappers;
they add no ambient authority and no runtime operation.

```zt
env ::= import stdlib.env;
clock ::= import stdlib.clock;
rng ::= import stdlib.rng;
load ::= import stdlib.load;
```

Each wrapper still takes the corresponding opaque advisory capability. The entry
boundary supplies only capability parameters named by the program's leading
parameter or closed capability record.

## Environment

`stdlib.env` exports:

| Name | Type | Meaning |
| --- | --- | --- |
| `GetEffects` | `Unit ! { env.get : Text -> Text?; }` | Composable operation row. |
| `Get A` | `A ! { * GetEffects; }` | Result-position effect alias. |
| `get` | `Env -> Text -> Get Text?` | Looks up one process environment variable. |

A missing variable yields `#none`.

## Clock

`stdlib.clock` exports `NowEffects`, `Now A`, and:

```zt
now :: Clock -> Now Instant
```

`Instant` is the existing text-shaped host timestamp. This wrapper performs
`clock.now ()`; it does not add timers, sleeping, monotonic time, or scheduling.

## Randomness

`stdlib.rng` exports `NextEffects`, `Next A`, and:

```zt
next :: Rng -> Next Int
```

The value comes from the existing `rng.next ()` host operation. The module does
not add seeding, ranges, distributions, cryptographic guarantees, or ambient
randomness.

## Dynamic loading

`stdlib.load` exports `ZtiEffects`, `ZtEffects`, their combined `Effects` pack,
`Zti A`, `Zt A`, `DynamicLoad A`, and:

```zt
zti :: Load -> Path -> Zti Data
zt  :: Load -> Path -> Zt Data
```

`zti` parses inert `.zti` data into the existing first-order `Data` envelope.
`zt` evaluates a general-mode file and converts only supported first-order final
values to `Data`; unsupported final values keep the existing host-boundary
rejection. The ambient compatibility names `loadZti` and `loadZt` remain
unchanged.

## Mocking and composition

Because these are ordinary effects, source handlers can mock every operation
without invoking the host:

```zt
env ::= import stdlib.env;
rng ::= import stdlib.rng;
absent :: Text? = #none;

example :: Env -> Rng -> { name : Text; number : Int; }
  ! { * env.GetEffects; * rng.NextEffects; }
  = envCap rngCap =>
handle [
  name := env.get envCap "NAME" ?? "unknown";
  number := rng.next rngCap;
  { name =; number =; }
] with {
  env.get = \key. resume absent;
  rng.next = \unit. resume 7;
};
```

`examples/host_capabilities_mock.zt` intercepts every wrapper with source
handlers. `examples/host_capabilities.zt` composes all four modules through one
explicit entry record and runs on both the reference interpreter and native
backend; parity checks stable fields and value shapes because separate
host-backed clock invocations are intentionally nondeterministic.
