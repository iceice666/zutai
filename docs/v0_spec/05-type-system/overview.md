## 9. Type system

Zutai has a static type system with inference and explicit parametric generics.

Core built-in types include:

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

### 9.1 Type annotations

Type annotations appear after `:` in declarations and field definitions:

```zt
port : Int = 8080
```

The grammar is:

```ebnf
type_annotation ::= ":" type_expr
```

A `type_expr` is a type-level expression. In v0, type expressions are type names, type applications, optional types (`T?`), record types (`{ ... }`), union types (`[ ... ]`), and list types (`List T`).

Example:

```zt
getPort :: { port : Int } -> Int
    :: server { server.port }
```

### 9.2 Type aliases and generic types

A named type alias binds a name to a type expression:

```zt
Host :: type Text
Port :: type Int

Server :: type {
  host : Text;
  port : Int;
}
```

Generic type aliases use a type parameter list `[...]`:

```zt
Pair :: [A, B] type {
  first : A;
  second : B;
}

Response :: [Body] type {
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

### 9.3 Typing approach

Zutai uses bidirectional type checking with inference for local expressions where the principal type is straightforward.

Polymorphism is explicit at API boundaries using `[...]` type parameter lists, with HM-style let generalization permitted for simple let-bound expressions.

Record typing uses closed records in v0. See [Record types](records.md).

When inference is ambiguous, implementations should ask for an explicit annotation.
