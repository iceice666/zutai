# Standard Library: Reflect

## Status

Accepted and implemented as an explicit filesystem source module:
`refl ::= import stdlib.reflect`. The module wraps the existing reflection
intrinsics without making new names ambient.

The module source lives at `crates/general/stdlib/src/packages/data/modules/reflect.zt` and is
registered by the filesystem stdlib manifest.

## API

```zt
SchemaKind
SchemaField
SchemaVariant
Schema
fields
variants
schema
```

`witness C @T` remains syntax and is intentionally not exported as a module
field.

## Support Level

- `refl.fields T` and destructured `fields T` keep the THIR type-value oracle
  route used by the ambient `fields T` builtin.
- `refl.schema T` and destructured `schema T` keep the THIR type-value oracle
  route used by the ambient `schema T` builtin.
- `refl.fields`, `refl.variants`, and `refl.schema` keep compile/dataflow
  fold-or-reject behavior: serializable reflection folds to backend constants;
  residual `Type` values, raw witness dictionaries, functions, or effectful
  reflection programs reject before Dataflow Core.
- Importing or re-exporting `stdlib.reflect` without actually using a reflection
  operation does not trigger the backend reflection gate.
