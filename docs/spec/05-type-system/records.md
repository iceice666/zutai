## Record types

Record types use:

```zt
type {
  field : TypeExpr;
}
```

Example:

```zt
Server :: type {
  host : Text;
  port : Int;
  tls : Bool;
};
```

Value:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
  tls = true;
};
```

This is the core symmetry:

```zt
{
  host = "localhost";
  port = 8080;
}
```

is a value record.

```zt
type {
  host : Text;
  port : Int;
}
```

is a record type.

The field syntax is the same shape. At the outer boundary, the `type` prefix changes the interpretation.

Inside a type context, such as a type annotation or a field inside a type record, nested record or union type literals omit a repeated `type` prefix:

```zt
type {
  server : {
    host : Text;
    port : Int;
  };
}
```

### Duplicate fields

Duplicate field names in the same record value or record type are invalid. There is no first-wins or last-wins rule.

### Closed records

Record types are closed unless they contain a row tail.

Given:

```zt
Server :: type {
  host : Text;
  port : Int;
};
```

This is valid:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
};
```

This is invalid:

```zt
server :: Server = {
  host = "localhost";
  port = 8080;
  tls = true;
};
```

because `tls` is not declared in `Server`.

### Record update

Zutai does not use the older nested update spelling:

```zt
{ x with port = 8080; }
```

Record update is strict, non-extending, and non-deleting:

```zt
record with {
  field = value;
}
```


---

Open record types (view types), named row tails, row-polymorphic field access,
and selective projection are specified in [Row polymorphism](../06-polymorphism/row-polymorphism.md).
