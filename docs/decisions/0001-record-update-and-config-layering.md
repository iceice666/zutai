# Decision 0001: Record Update and Config Layering

## Status

Accepted for post-v0 design.

## Decision

v0 does not include record update syntax.

Post-v0 record update uses strict, non-extending, non-deleting structural replacement:

```zt
record with {
  field = value;
}
```

The updated field must already exist in the record type. The operation does not add fields, remove fields, or recursively merge nested records.

Layered configuration is not core syntax. It belongs in the standard library as explicit functions such as `overlay` and `overlayDeep`.

## Rationale

Record update and config layering solve different problems:

- record update is local typed field replacement
- config overlay is policy-driven composition of partial layers

Keeping overlay behavior in the standard library avoids hard-coding merge policy into the core record syntax.
