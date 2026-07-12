# Row Polymorphism

Row polymorphism extends closed record and union types with open types and named
row tails. Non-principal inference requires an explicit annotation.

---

## Open Record / View Types

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
  = x => x.host;
```

The function accepts all of these values:

```zt
{ host = "localhost"; }
{ host = "localhost"; port = 8080; }
{ host = "localhost"; port = 8080; tls = true; }
```

A view type exposes only the fields named in the view. Extra fields are accepted for type checking, but the anonymous rest of the row is not named and cannot be preserved in the result type.

---

## Named Row Tails

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
identityHostRecord :: <Rest> { host : Text; ...Rest; } -> { host : Text; ...Rest; }
  = x => x;
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

---

## Row-Polymorphic Field Access

A function that only reads a field can be typed with a view type:

```zt
portOrDefault :: { port : Int?; ...; } -> Int
  = x => x.port ?? 8080;
```

A function that must preserve extra fields should use a named row tail:

```zt
keep :: <Rest> { port : Int; ...Rest; } -> { port : Int; ...Rest; }
  = x => x;
```

Implementations may infer simple row-polymorphic types for local expressions, but exported or top-level polymorphic APIs should use explicit annotations.

---

## Selective Projection
Selective projection uses either keyword or postfix-operator syntax. Both forms
are Zutai features for values and type values.

For values:

```zt
select server { host; port; }
```

The postfix operator form is equivalent:

```zt
server >>= { host; port; }
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

The type postfix operator form is equivalent:

```zt
Server >>= { host; port; }
```

returns the closed record type:

```zt
type {
  host : Text;
  port : Int;
}
```

`select` preserves field order as written in the selection list and reports an unknown-field error if a selected field is absent from the input record or record type.

Zutai has first-class type values and pure type-level computation, but it does not
include `select` syntax. See
[type-level computation](../05-type-system/type-level-computation.md)
for Zutai baseline that Zutai type-value projection builds on.

---

## Open Union Types

Analogous to open record types, a union type may have an anonymous or named row tail to accept additional members.

An anonymous union tail:

```zt
type {
  #dev;
  #test;
  ...;
}
```

means any union that includes at least `#dev` and `#test` as members.

A named union tail preserves the extra members in the result type:

```zt
type {
  #dev;
  #test;
  ...Rest;
}
```

Example:

```zt
handle_env :: <Rest> { #dev; #test; ...Rest; } -> Text -> Text
  = #dev msg  => "dev: ";
  = #test msg => "test: ";
  = _ msg     => msg;
```

Union tails also work with tuple members:

```zt
type {
  #circle: { radius : Float; };
  ...Rest;
}
```

Union extension spreads an existing union type into a new union type and then adds new members:

```zt
Shape3D :: type {
  * Shape;
  #sphere: { radius : Float; };
}
```

Here `* Shape;` means "include the members of the existing union type `Shape`." This is distinct from `...Rest;`, where `Rest` is a row variable introduced by a polymorphic type parameter list.

---

## Row Polymorphism in Generic Functions

With row polymorphism, type parameters may include row variables:

```zt
getHost :: <Rest> { host : Text; ...Rest; } -> Text
  = x => x.host;
```

Named row tails allow preserving the rest of the record through transformations.

---

## Predicativity and Inference Boundaries

Zutai polymorphism is predicative. A type variable may be instantiated with ordinary monotypes, including record types, union types, list types.

Zutai row polymorphism remains predicative and does not include impredicative instantiation, where a type variable is instantiated with another polymorphic type.

Zutai does not infer higher-rank types. Functions that accept polymorphic
functions use explicit annotations; see
[higher-rank polymorphism](higher-rank-polymorphism.md).

---

## Extended Inference

With row polymorphism, implementations must support straightforward first-order unification of type variables and row variables after type-level normalization.

Implementations are not required to infer:

* arbitrary higher-rank polymorphic types
* impredicative instantiations
* higher-order unification problems
* complex row constraints involving record update or field absence beyond duplicate-field checks

When inference is not principal or not obvious, implementations should ask for an explicit type annotation instead of guessing.
