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

Record types are closed by default.

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

### 10.2 Open record/view types

An open record type, also called a view type, uses an anonymous row tail:

```zt
type {
  host : Text;
  ...;
}
```

This means:

> any record with at least a `host: Text` field.

Example:

```zt
getHost :: { host : Text; ...; } -> Text
    :: x { x.host }
```

The function accepts all of these values:

```zt
{ host = "localhost"; }
{ host = "localhost"; port = 8080; }
{ host = "localhost"; port = 8080; tls = true; }
```

A view type exposes only the fields named in the view. Extra fields are accepted for type checking, but the anonymous rest of the row is not named and cannot be preserved in the result type.

### 10.3 Named row tails

A named row tail preserves information about the rest of a record:

```zt
type {
  host : Text;
  ...Rest;
}
```

`Rest` is a row variable. Row variables range over record fields, not ordinary runtime values.

Example:

```zt
identityHostRecord ::
  forall Rest. { host : Text; ...Rest; } -> { host : Text; ...Rest; }
    :: x { x }
```

This function returns a record with exactly the same additional fields it received.

Row tails must not overlap explicitly declared fields. For example, in:

```zt
type {
  host : Text;
  ...Rest;
}
```

`Rest` cannot contain another `host` field.

### 10.4 Row-polymorphic field access

A function that only reads a field can be typed with a view type:

```zt
portOrDefault :: { port : Int?; ...; } -> Int
    :: x { x.port ?? 8080 }
```

A function that must preserve extra fields should use a named row tail:

```zt
keep ::
  forall Rest. { port : Int; ...Rest; } -> { port : Int; ...Rest; }
    :: x { x }
```

Implementations may infer simple row-polymorphic types for local expressions, but exported or top-level polymorphic APIs should use explicit annotations.

### 10.5 Selective projection

Selective projection uses `select`.

For values:

```zt
select server { host; port; }
```

means:

```zt
{
  host = server.host;
  port = server.port;
}
```

For type values:

```zt
select Server { host; port; }
```

returns the closed record type:

```zt
type {
  host : Text;
  port : Int;
}
```

assuming `Server` is a record type with those fields.

Selection is explicit projection. It is different from a view type:

```zt
type { host : Text; port : Int; ...; }
```

which accepts records with at least those fields, without discarding extra fields at runtime.

`select` preserves field order as written in the selection list and reports an unknown-field error if a selected field is absent from the input record or record type.

### 10.6 Record update

Record update syntax is not part of v0.

In particular, v0 does not define:

```zt
{ x with port = 8080; }
```

This avoids introducing overwrite and field-absence constraints before they are needed.

A post-v0 decision accepts strict, non-extending, non-deleting record update using:

```zt
record with {
  field = value;
}
```

