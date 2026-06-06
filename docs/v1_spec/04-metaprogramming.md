# Metaprogramming (v1)

Metaprogramming is based on reflection and schema reification over v0 type values and type functions. Reflection APIs such as `fields` and `schema` are deferred from v0.

---

## Type Values

```zt
WithId :: Type -> Type {
  | T => type {
    id : Text;
    value : T;
  };
}
```

Type functions are available in v0; metaprogramming APIs can consume the type values they produce.

Usage:

```zt
NamedText : Type = WithId Text
```

---

## Reflection

Reflection inspects types at compile time:

```zt
serverFields := fields Server
```

A conceptual result:

```zt
[
  {
    name = #host;
    Type = Text;
    optional = false;
  };

  {
    name = #port;
    Type = Int;
    optional = false;
  };
]
```

This result may contain `Type` values, so it is useful for metaprogramming but not necessarily serializable.

---

## Schema Reification

To produce serializable data, use explicit schema conversion:

```zt
serverSchema := schema Server

serverSchema
```

That can output plain data:

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

So:

```zt
fields Server
```

is compile-time reflection.

```zt
schema Server
```

is serializable schema data.
