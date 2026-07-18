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
```

`FieldDescriptor` (`{ name : Text; Type : Type; optional : Bool; }`) and
`VariantDescriptor` (`{ name : Text; fields : List FieldDescriptor; }`) are the
typed rank-2 descriptors: each carries the reflected component's own `Type`
rather than the text-erased `type : Text` of the `Schema*` family. `fields` and
`variants` reflect directly into `List FieldDescriptor` / `List VariantDescriptor`;
`schema` returns the serialization-oriented `Schema`.

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
- `refl.fields`, `refl.variants`, and `refl.schema` keep compile/dataflow
  fold-or-reject behavior: serializable reflection folds to backend constants;
  residual `Type` values, raw witness dictionaries, functions, or effectful
  reflection programs reject before Dataflow Core.
- Importing or re-exporting `stdlib.reflect` without actually using a reflection
  operation does not trigger the backend reflection gate.
