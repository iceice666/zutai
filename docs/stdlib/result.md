# Standard Library: Result

## Status

Accepted and implemented as an explicit filesystem source module:
`r ::= import stdlib.result`. Result and Validation helpers stay opt-in and do
not become ambient prelude names.

The module source lives at `crates/general/stdlib/src/packages/base/modules/result.zt` and is
registered by the filesystem stdlib manifest.

## API

```zt
Result Validation
ok err valid invalid invalidOne
map map2 map3 mapErr andThen withDefault
isOk isErr toOptional fromOptional ensure orElse errors
```

`Result E A` is the short-circuiting errors-as-data shape:
`#ok { value : A; }` or `#err { error : E; }`. `Validation E A` accumulates
`List E` errors with `#valid { value : A; }` or `#invalid { errors : List E; }`.

## Semantics

- `map`, `mapErr`, `andThen`, `withDefault`, `ensure`, `orElse`,
  `toOptional`, and `fromOptional` operate on `Result`.
- `map2`, `map3`, `invalidOne`, and `errors` operate on `Validation`.
- `map2` and `map3` accumulate all invalid input errors in left-to-right
  argument order.
- `orElse fallback result` returns the original `#ok` value when present and
  uses `fallback` only for `#err`.

## Implementation Notes

The module is pure `.zt` over tagged unions, list literals, and `append`. No
runtime ABI, Dataflow, SSA, or codegen primitive is added for this module.
