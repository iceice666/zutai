## Type system

Zutai has a static type system with inference, explicit parametric generics, and first-class compile-time `Type` values.

Core built-in type values include:

```zt
Type
Text
Bool
Int
Float
List
```

Implementations may provide additional standard-library types such as:

```zt
Number
Result
Map
```

but they are not required for the v0 core.

A value of type `Type` describes a type. Type values may be bound, passed to type-level functions, imported from `.zt` modules, and used in type annotations. They are not serializable final outputs.

### Type annotations

Type annotations appear after `::` in declarations:

```zt
port :: Int = 8080
```

The grammar is:

```ebnf
type_annotation ::= "::" type_expr
```

A `type_expr` is a type-level expression. In v0, type expressions are pure expressions that evaluate to `Type`, including type names, type applications, optional types (`T?`), record types (`{ ... }`), union types (`[ ... ]`), and list types (`List T`).

Example:

```zt
getPort :: { port : Int } -> Int
        | server => server.port
```

### Type aliases and generic types

A named type alias binds a name to a type expression. The canonical spelling uses `:: type`:

```zt
Host :: type Text
Port :: type Int

Server :: type {
  host : Text;
  port : Int;
}
```

The equivalent annotated type-valued binding is valid, but less idiomatic for named types:

```zt
Server : Type = type {
  host : Text;
  port : Int;
}
```

Generic type aliases use a type parameter list `<...>`:

```zt
Pair :: <A, B> type {
  first : A;
  second : B;
}

Response :: <Body> type {
  status : Int;
  body : Body?;
}
```

Usage:

```zt
pair : Pair Text Int = {
  first = "hello";
  second = 42;
}

response : Response Text = {
  status = 200;
  body = "ok";
}
```

Type parameters are uppercase. A generic type alias is instantiated at use sites with concrete types.

Type constructors may also be ordinary functions returning `Type`:

```zt
PairFn :: Type -> Type -> Type
       | A B => type {
           first : A;
           second : B;
         }
```

### Typing approach

Zutai uses bidirectional type checking with inference for local expressions where the principal type is straightforward.

Type-level evaluation is pure and deterministic, but implementations must bound it. If a type-level expression does not normalize within the implementation limit, type checking fails with a source-located error.

Polymorphism is explicit at API boundaries using `<...>` type parameter lists, with HM-style let generalization permitted for simple let-bound expressions.

Record typing uses closed records in v0. See [Record types](records.md).

When inference is ambiguous, implementations should ask for an explicit annotation.
