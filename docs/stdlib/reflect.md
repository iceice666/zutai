# Standard Library: Reflect

## Status

Accepted and implemented as an explicit filesystem source module:
`refl ::= import stdlib.reflect`. The module wraps the existing reflection
intrinsics without making new names ambient.

The module source lives at `stdlib/packages/data/modules/reflect.zt` and is
registered by the filesystem stdlib manifest.

## API

```zt
FieldDescriptor
VariantDescriptor
SchemaKind
SchemaField
SchemaVariant
Schema
fields
variants
schema
schemaFields
```

`FieldDescriptor` (`{ name : Text; Type : Type; optional : Bool; }`) and
`VariantDescriptor` (`{ name : Text; fields : List FieldDescriptor; }`) are the
typed rank-2 descriptors: each carries the reflected component's own `Type`
rather than the text-erased `type : Text` of the `Schema*` family. `fields` and
`variants` reflect directly into `List FieldDescriptor` / `List VariantDescriptor`;
`schema` returns the serialization-oriented `Schema`.

`schemaFields T` is `(schema T).fields ?? {;}`. `Schema.fields` is optional
because a union has no record fields, so a serialization boundary that wants
erased fields must otherwise discharge an unreachable `#none` for every record
type. The helper answers `{;}` for a union — its correct "no record fields"
answer — and keeps `Schema.fields` optional for callers that distinguish the
two cases. It is the erased counterpart to `fields`, which is non-optional but
type-carrying.

`witness C @T` remains syntax and is intentionally not exported as a module
field.

### Compile-time derive builders

`deriveShow`, `deriveOrdLex`, `deriveFromData`, and `deriveToData` are ambient compile-time
markers naming the generic derive recipes. A constraint recipe body names one
directly — `derive = <T> => deriveShow` — to fold the structural witness
dictionary at the concrete derive target. They are not runtime values, so they
are neither imported through `stdlib.reflect` nor applied as ordinary functions;
see `docs/compiler/derive-recipes.md`.

## Support Level

- `refl.fields T` and destructured `fields T` keep the THIR type-value oracle
  route used by the ambient `fields T` builtin.
- `refl.schema T` and destructured `schema T` keep the THIR type-value oracle
  route used by the ambient `schema T` builtin.
- `refl.schemaFields T` and destructured `schemaFields T` route exactly like
  `refl.schema T`, since the wrapper is defined in terms of it. An open row
  refuses with the same reflection diagnostic rather than answering `{;}`.
- `refl.fields`, `refl.variants`, `refl.schema`, and `refl.schemaFields` keep
  compile/dataflow fold-or-reject behavior: serializable reflection folds to
  backend constants; residual `Type` values, raw witness dictionaries,
  functions, or effectful reflection programs reject before Dataflow Core.
  These gates match export fields **by name**
  (`STDLIB_REFLECT_AOT_FIELDS` in `semantic/src/lib.rs`), so any new exported
  wrapper that reaches a reflection intrinsic must be added there; an unlisted
  export is neither folded nor rejected and reaches the backend as a residual
  `Type` value.
- Importing or re-exporting `stdlib.reflect` without actually using a reflection
  operation does not trigger the backend reflection gate.
