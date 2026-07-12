# Metaprogramming

Metaprogramming is based on reflection and schema reification over type values
and type functions. Reflection is compile-time and does not add ambient runtime
behavior.

---

## Type Values

```zt
WithId :: Type -> Type
  = T => type {
    id : Text;
    value : T;
  };
```

Type functions are available in Zutai; metaprogramming APIs can consume the type values they produce.

Usage:

```zt
NamedText :: Type = WithId Text
```

---

## Reflection

Reflection inspects types at compile time:

```zt
serverFields ::= fields Server
```

`fields` currently reflects closed record types. The exact result shape is a list of field
metadata records:

```zt
[
  {
    name = "host";
    Type = Text;
    optional = false;
  };

  {
    name = "port";
    Type = Int;
    optional = false;
  };
]
```

`name` is `Text`, `Type` is an embedded `Type` value, and `optional` is `Bool`.
Because this result contains `Type` values, it is useful for metaprogramming but
not serializable. `fields` rejects union types; use `schema` for union variants.

---

## Schema Reification

To produce serializable data, use explicit schema conversion:

```zt
serverSchema ::= schema Server

serverSchema
```

Record schema output is ordinary serializable data:

```zti
{
  kind = #record;
  fields = [
    {
      name = "host";
      type = "Text";
      optional = false;
    };

    {
      name = "port";
      type = "Int";
      optional = false;
    };
  ];
}
```

Union schema output uses a `variants` list. Payload fields use the same field schema records;
singleton variants have an empty `fields` list:

```zti
{
  kind = #union;
  variants = [
    { name = "ok"; fields = [{ name = "value"; type = "Text"; optional = false; }]; };
    { name = "done"; fields = []; };
  ];
}
```

Open record and union rows are rejected initially instead of encoded in schema output.

So:

```zt
fields Server
```

is compile-time reflection.

```zt
schema Server
```

is serializable schema data.
