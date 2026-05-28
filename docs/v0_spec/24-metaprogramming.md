## 24. Metaprogramming

Metaprogramming is based on type values, type functions, reflection, and schema reification.

### 24.1 Type functions

```zt
WithId :: Type -> Type
   :: T { type {
    id : Text;
    value : T;
  } }
```

Usage:

```zt
NamedText : Type = WithId Text
```

### 24.2 Reflection

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

### 24.3 Schema reification

To produce serializable data, use explicit schema conversion:

```zt
serverSchema := schema Server

serverSchema
```

That can output plain data:

```zt
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

---
