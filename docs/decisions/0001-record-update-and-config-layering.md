# Decision 0001: Record Update and Config Layering

## Status

Accepted for post-v0 core/library design.

## Context

Zutai v0 intentionally excludes record update to avoid premature complexity around overwrite semantics, field absence, and row-polymorphic constraints.

Practical configuration work still needs two related but distinct capabilities:

1. Local modification of an existing typed record.
2. Layered composition of partial configuration files.

These are separate concepts:

```text
record update   = local typed structural replacement
config overlay  = policy-driven application of partial layers
normalization   = conversion from raw partial config to final validated config
```

## Decision

Add record update as a post-v0 core expression form:

```zt
record with {
  field = value;
}
```

Keep config layering in the standard library through functions and type-level schema utilities:

```zt
overlay
overlayDeep
Patch T
DeepPatch T
```

These are not keywords. They are standard-library names, though `Patch` and `DeepPatch` may be compiler-backed type constructors if needed.

## Record update

Record update is a pure structural copy operation.

It evaluates the base record, copies its fields, replaces the specified fields, and returns a new record. It does not mutate the original value.

Example:

```zt
let server2 =
  server with {
    port = 9090;
  }
```

Rules for the first version:

```text
- Updates existing fields only.
- Does not add new fields.
- Does not delete fields.
- Replacement values must type-check against the existing field type.
- Closed record types remain closed.
- Named row tails are preserved.
- Optional fields may be assigned, but the declared type is preserved.
- none is a value, not deletion.
```

### Closed record example

```zt
let Server: Type = type {
  host = Text;
  port = Int;
  tls = Bool;
}

let server2: Server =
  server with {
    port = 9090;
  }
```

This is valid because `port` exists and `9090` has type `Int`.

This is invalid:

```zt
server with {
  timeout = 30;
}
```

because `timeout` is not a known field of `server`.

### Row-polymorphic update

```zt
let setPort:
  forall Rest. { port = Int; ...Rest; } -> Int -> { port = Int; ...Rest; } =
  fn r p =>
    r with {
      port = p;
    }
```

The update preserves the unknown remainder of the record.

### Optional fields

Given:

```zt
let RawServer: Type = type {
  host? = Text;
  port? = Int;
  tls? = Bool;
}
```

This is valid:

```zt
raw with {
  port = 8080;
}
```

The result still has type `RawServer`. The type system does not refine the result to make `port` required.

This does not delete `port`:

```zt
raw with {
  port = none;
}
```

It is only valid if the field type allows `none`.

## Config layering

Config layering is not record update.

Record update modifies one known record. Config layering combines multiple partial configuration layers using explicit policy.

Typical layers:

```text
built-in defaults
project config
local user config
environment config
CLI overrides
```

Preferred workflow:

```text
partial raw layers -> merged raw config -> normalized final config
```

Example:

```zt
let RawServer: Type = type {
  host? = Text;
  port? = Int;
  tls? = Bool;
}

let Server: Type = type {
  host = Text;
  port = Int;
  tls = Bool;
}

let normalizeServer: RawServer -> Server =
  fn raw => {
    host = raw.host ?? "127.0.0.1";
    port = raw.port ?? 8080;
    tls = raw.tls ?? false;
  }
```

See [standard library config](../stdlib/config.md) for the library API.

## Deletion is deferred

Absence and `none` mean different things in Zutai:

```text
absent field = no value provided
none         = explicit value representing absence
```

Config deletion should not overload either meaning.

If deletion is added later, it must use an explicit patch-level marker, such as:

```zt
#delete
```

or a dedicated patch utility type.
