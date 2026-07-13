# Standard Library: Data

## Status

Accepted and implemented as an explicit filesystem source module:
`d ::= import stdlib.data`. The module provides a first-order data envelope and
structured decoding helpers; it is not ambient.

The module source lives at `crates/general/stdlib/src/modules/data.zt` and is
registered by the filesystem stdlib manifest.

## Types

```zt
DataField :: type { name : Text; value : Data; };

Data :: type {
  #bool : { value : Bool; };
  #int : { value : Int; };
  #float : { value : Float; };
  #text : { value : Text; };
  #atom : { value : Text; };
  #list : { items : List Data; };
  #record : { fields : List DataField; };
  #tagged : { tag : Text; payload : Data; };
};

DecodeError :: type {
  #expected : { expected : Text; actual : Text; };
  #missingField : { name : Text; };
  #indexOutOfBounds : { index : Int; };
  #custom : { message : Text; };
};
```

Decoder results use the `stdlib.result.Result DecodeError A` shape. The module
exports `Result` as a forwarding type alias so imported decoder results can be
pattern-matched ergonomically.

Runtime-loaded `Data` can also be decoded structurally through the provisional
ambient `FromData` constraint and `decode` helper:

```zt
Config :: type { port : Int; owner : { name : Text; }; };
FromData @Config :: derive
value :: Validation DecodeIssue Config = decode rawData;
```

`DecodeIssue` carries a `List DecodePath`; nested record, list, and union
failures prepend `#field`, `#index`, and `#variant` segments. Record and list
decoding accumulates independent failures in declaration/index order and
ignores unknown input record fields.

## API

Constructors:

```zt
bool int float text atom list record tagged fieldOf
```

Decoders and accessors:

```zt
kind
asBool asInt asFloat asText asAtom asList asRecord asTagged
field field? at tag payload mapList
```

Shared decoder types exported by `stdlib.data`:

```zt
Validation DecodePath DecodeIssue
```

Ambient decoder names: `FromData`, `fromData`, and `decode`.

Supported derived targets are `Bool`, `Int`, `Float`, `Text`, atom singleton
types, `List`, `Optional`, closed records (including physical optional fields),
and closed unions. A missing optional field becomes absent; a present one is
decoded normally. Open rows, tuples, recursive targets, and fixed-width/posit
scalars are rejected at the derive request. The constraint and helper are
ambient in this slice; `stdlib.data` exports the shared
`Validation`, `DecodePath`, and `DecodeIssue` types. Moving synthesis onto the
typed reflection recipe API remains roadmap work.

The reference/TLC evaluator supports the full target set above. Native output
currently runs primitive and flat-record decoders; nested derived records remain
an explicit backend-parity blocker rather than a claimed support level.

`field name data` rejects missing fields with `#missingField`. `field? name data`
returns `#ok #none` for a missing field and still returns `#err` when `data` is
not a record. `at index data` rejects negative and out-of-range indexes with
`#indexOutOfBounds`.

## Implementation Notes

This is a pure source module over records, unions, lists, pattern matching, and
`stdlib.result` type shapes. It adds no runtime ABI, Dataflow, SSA, or codegen
primitive.
