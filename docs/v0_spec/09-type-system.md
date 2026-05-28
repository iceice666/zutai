## 9. Type system

Zutai has a full type system with first-class type-level computation.

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

Type annotations are type expressions that evaluate to types:

```zt
port : Int = 8080
```

The grammar is:

```ebnf
type_annotation ::= ":" type_expr
```

A `type_expr` may contain arbitrary pure expressions that evaluate to `Type`. It is also a type context: bare `{ ... }` and `[ ... ]` are parsed as record and union type literals without a repeated `type` keyword.

Example:

```zt
getPort :: { port : Int; ...; } -> Int
    :: server { server.port }
```

Example:

```zt
Response :: Type -> Type
    :: Body { type {
    status : Int;
    body : Body?;
  } }

value : Response Text = {
  status = 200;
  body = "ok";
}
```

### 9.2 The `Type` type

Types are first-class compile-time values.

```zt
Server :: type {
  host : Text;
  port : Int;
}
```

Type aliases are ordinary bindings:

```zt
Host :: type Text
Port :: type Int
UserId :: type Text
```

Type constructors are functions that return `Type`:

```zt
Pair :: Type -> Type -> Type
    :: A -> B { type {
    first : A;
    second : B;
  } }
```

Usage:

```zt
TextIntPair :: type Pair Text Int

pair : TextIntPair = {
  first = "hello";
  second = 42;
}
```

### 9.3 Typing approach

Zutai uses bidirectional type checking with inference for local expressions where the principal type is straightforward.

Polymorphism is explicit at API boundaries using `forall`, with HM-style let generalization permitted for simple let-bound expressions.

Record typing includes closed records, open view types, and row-polymorphic records. See [Polymorphism](18-polymorphism.md) and [Record types](10-record-types.md).

Type equality is checked after normalizing pure type-level expressions, subject to deterministic compile-time evaluation limits.

### 9.4 Universe levels

The surface language exposes `Type`.

The implementation should use internal universe levels to avoid literal unsoundness such as `Type : Type`.

Conceptually, users write:

```zt
Int: Type
Text: Type
Server: Type
```

Internally, the implementation may model this as:

```text
Type0 : Type1
Type1 : Type2
Type2 : Type3
```

The user normally writes only `Type`.

---
