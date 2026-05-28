## 10. Record types

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
}
```

Value:

```zt
server : Server = {
  host = "localhost";
  port = 8080;
  tls = true;
}
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

### 10.1 Closed records

Record types are closed in v0.

Given:

```zt
Server :: type {
  host : Text;
  port : Int;
}
```

This is valid:

```zt
server : Server = {
  host = "localhost";
  port = 8080;
}
```

This is invalid:

```zt
server : Server = {
  host = "localhost";
  port = 8080;
  tls = true;
}
```

because `tls` is not declared in `Server`.

### 10.2 Record update

Record update syntax is not part of v0.

In particular, v0 does not define:

```zt
{ x with port = 8080; }
```

A post-v0 decision accepts strict, non-extending, non-deleting record update using:

```zt
record with {
  field = value;
}
```

See [Decision 0001: Record Update and Config Layering](../../decisions/0001-record-update-and-config-layering.md).

---

Open record types (view types), named row tails, row-polymorphic field access, and selective projection are v1 features. See [Row polymorphism](../../v1_spec/01-row-polymorphism.md).
