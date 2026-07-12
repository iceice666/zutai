# Standard Library: Config

## Status

Accepted and implemented as an explicit filesystem source module:
`cfg ::= import stdlib.config`. The ambient compatibility names `overlay` and
`overlayDeep` still exist, but nothing from `stdlib.config` becomes ambient.

This module supports layered configuration without making merge behavior part of core record syntax.

## Core idea

```text
record update   = local typed structural replacement
config overlay  = policy-driven application of partial layers
normalization   = conversion from raw partial config to final validated config
```

Record update belongs to the core expression language. Config overlay belongs
here, in the standard library.

## Names

The config API uses ordinary standard-library bindings, not keywords:

```zt
cfg ::= import stdlib.config;

cfg.overlay
cfg.overlayDeep
cfg.Patch
cfg.DeepPatch
```

`Patch` and `DeepPatch` are type-level schema utilities. They may be compiler-backed type constructors if the implementation needs that, but they should still be exposed as library names.

Module-qualified and destructured aliases are recognized by THIR call lowering:

```zt
{ overlayDeep; } ::= import stdlib.config;

overlayDeep { nested = { tls = true; }; } base
```

Supported full applications with record-literal patches lower exactly like the
ambient builtin forms before TLC/Dataflow lowering. Partial overlay values,
unsupported patch shapes, and unsupported optional nested-record deep overlays
remain backend-gated.

## Shallow overlay

### Type

```zt
overlay :: [T] Patch T -> T -> T
```

`overlay upper lower` applies the upper patch to the lower value.

The patch argument comes first so the function composes naturally with pipelines:

```zt
raw ::=
  defaults
    |> overlay project
    |> overlay local
    |> overlay cli;
```

This is equivalent to applying `project`, then `local`, then `cli` over `defaults`.

### Semantics

```text
- If a field is absent in the upper layer, keep the lower value.
- If a field is present in the upper layer, replace the lower value.
- Nested records are replaced as whole values.
- Lists are replaced as whole values.
- `#none` is treated as a value, not deletion.
```

Shallow overlay is the simplest config-composition primitive and should remain predictable.

## Deep overlay

### Type

```zt
overlayDeep :: [T] DeepPatch T -> T -> T
```

`overlayDeep upper lower` applies the upper deep patch to the lower value.

Pipeline usage:

```zt
raw ::=
  defaults
    |> overlayDeep project
    |> overlayDeep local
    |> overlayDeep cli;
```

### Semantics

```text
- Missing field in upper layer: keep lower field.
- Present scalar in upper layer: replace lower field.
- Present record in upper layer: recursively merge.
- Present list in upper layer: replace whole list.
- Present union tuple in upper layer: replace the whole tuple value.
- Present `#none`: set field to `#none`, if the type allows it.
- Unknown field: type error unless a row-polymorphic open-record target type explicitly permits it.
```

Lists are not concatenated by default. List merge behavior is domain-specific and should be explicit.

## Patch types

For a schema:

```zt
Server :: type {
  host : Text;
  port : Int;
  tls : Bool;
};
```

A shallow patch type:

```zt
Patch Server
```

conceptually becomes:

```zt
type {
  host? : Text;
  port? : Int;
  tls? : Bool;
}
```

For nested schemas, a deep patch type recursively turns record fields into patchable fields:

```zt
DeepPatch Config
```

`Patch T` and `DeepPatch T` model field presence with `Maybe`, separately from optional field values. This preserves the distinction between these cases:

```text
field absent          = #absent; do not change the lower value
field present X       = #present (X); set the field to X
field present #none   = #present (#none); set the field to #none, if allowed by the field type
```

Deletion is not part of `Patch` or `DeepPatch`.

## Raw-then-normalize workflow

The preferred configuration model is:

```text
partial raw layers -> merged raw config -> normalized final config
```

Example:

```zt
RawServer :: type {
  host? : Text;
  port? : Int;
  tls? : Bool;
};

Server :: type {
  host : Text;
  port : Int;
  tls : Bool;
};

normalizeServer :: RawServer -> Server
  = raw => {
    host = raw.host ?? "127.0.0.1";
    port = raw.port ?? 8080;
    tls = raw.tls ?? false;
  };

raw ::=
  defaults
    |> overlay project
    |> overlay local
    |> overlay cli;

server :: Server = normalizeServer raw;
```

This keeps defaulting, validation, and cross-field logic centralized in normalization functions.

## Deletion is deferred

Absence and `#none` already mean different things:

```text
#absent          = no field/value provided
#none            = explicit Optional value
#present (#none) = a present field whose value is #none
```

Deletion must not overload either meaning.

If config deletion is added later, it should use an explicit patch-level marker or utility type, such as:

```zt
#delete
```
