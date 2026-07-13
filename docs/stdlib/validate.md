# Standard Library: Validate

## Status

Accepted and implemented as an explicit filesystem source module:
`v ::= import stdlib.validate`. Validation is an opt-in errors-as-data surface,
not an ambient prelude convention.

The module source lives at `stdlib/packages/data/modules/validate.zt` and
is registered by the filesystem stdlib manifest.

## Types

```zt
ValidationError :: type {
  #required : { field : Text; };
  #invalid : { field : Text; message : Text; };
  #outOfRange : { field : Text; min : Int; max : Int; actual : Int; };
  #custom : { message : Text; };
};
```

Validation values use the `stdlib.result.Validation ValidationError A` shape.
`toResult` and `fromResult` use `stdlib.result.Result (List ValidationError) A`.
The module exports `Validation` and `Result` forwarding aliases for imported
type annotations and pattern matching.

## API

```zt
valid invalid invalidOne custom errors
map map2 map3
required satisfy nonEmptyText intRange oneOfText oneOfInt
toResult fromResult
```

`invalidOne` wraps a single structured validation error, while `custom` creates
a `#custom` error directly. `map2` and `map3` accumulate errors from every invalid input. `required` turns
`#none` into `#required { field = ...; }`. `satisfy`, `nonEmptyText`,
`intRange`, `oneOfText`, and `oneOfInt` produce structured `ValidationError`
values rather than free-text-only failures.

## Implementation Notes

This is a pure source module over `stdlib.result`, list append, pattern
matching, and scalar bridge helpers for text length. It adds no runtime ABI,
Dataflow, SSA, or codegen primitive.
