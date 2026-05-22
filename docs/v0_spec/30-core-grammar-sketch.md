## 30. Core grammar sketch

This grammar is intentionally a sketch. It defines the surface shape, not the full precedence parser.

```ebnf
File
  ::= LetBinding* Expr

LetBinding
  ::= "let" Ident TypeAnnotation? "=" Expr

TypeAnnotation
  ::= ":" TypeExpr

Expr
  ::= Literal
   | Ident
   | Atom
   | Record
   | List
   | Function
   | Match
   | If
   | Import
   | TypeForm
   | Call
   | Access
   | OptionalAccess
   | Select
   | Binary
   | Pipeline
   | OptionalType
   | FunctionType
   | Forall

Literal
  ::= "none"
   | "true"
   | "false"
   | Number
   | String

Atom
  ::= "#" AtomBody

AtomBody
  ::= [A-Za-z_][A-Za-z0-9_-]*

Ident
  ::= [A-Za-z_][A-Za-z0-9_]*

TypeVar
  ::= Ident

FieldName
  ::= [A-Za-z_][A-Za-z0-9_-]*

Record
  ::= "{" ValueField* "}"

ValueField
  ::= FieldName "=" Expr ";"

List
  ::= "[" ListItem* "]"

ListItem
  ::= Expr ";"

Function
  ::= "fn" Pattern+ "=>" Expr

Import
  ::= "import" String
   | "import" ImportPath

TypeForm
  ::= "type" TypeRecord
   | "type" TypeUnion

TypeExpr
  ::= TypeRecord
   | TypeUnion
   | OptionalType
   | FunctionType
   | Forall
   | Expr

TypeRecord
  ::= "{" TypeRecordItem* "}"

TypeRecordItem
  ::= TypeField
   | TypeRowTail

TypeField
  ::= FieldName "=" TypeExpr ";"
   | FieldName "?" "=" TypeExpr ";"

TypeRowTail
  ::= "..." ";"
   | "..." TypeVar ";"

TypeUnion
  ::= "[" TypeUnionItem* "]"

TypeUnionItem
  ::= TypeExpr ";"

OptionalType
  ::= TypeExpr "?"

Access
  ::= Expr "." FieldName

OptionalAccess
  ::= Expr "?." FieldName

Select
  ::= "select" Expr "{" SelectField* "}"

SelectField
  ::= FieldName ";"

Call
  ::= Expr Expr

Binary
  ::= Expr BinaryOperator Expr

BinaryOperator
  ::= "==" | "!=" | "<" | "<=" | ">" | ">="
   | "+" | "-" | "*" | "/"
   | "??"

Pipeline
  ::= Expr "|>" Expr
   | Expr "<|" Expr

FunctionType
  ::= TypeExpr "->" TypeExpr

Match
  ::= "match" Expr "{" MatchCase* "}"

MatchCase
  ::= Pattern "=>" Expr ";"

If
  ::= "if" Expr "then" Expr "else" Expr

Forall
  ::= "forall" TypeVar+ "." TypeExpr
```

Important grammar interpretation:

`TypeExpr` is a contextual grammar category used wherever the surrounding syntax expects a type: type annotations, `forall` bodies, function-type operands, optional-type operands, type-record fields, and type-union items. It can still be an arbitrary expression checked to evaluate to `Type`, but if it starts with `{` or `[` it is parsed as a record or union type literal rather than as a value record or value list. This makes type annotations and tagged-union variants concise:

```zt
let getHost: { host = Text; ...; } -> Text =
  fn x => x.host

let Shape: Type = type [
  { kind = #circle; radius = Float; };
  { kind = #rect; width = Float; height = Float; };
]
```

The forms `|>`, `<|`, `->`, `??`, application, field access, and postfix `?` are parsed by the precedence rules in [Operator precedence](27-operator-precedence.md).

The form:

```zt
select x { a; b; }
```

is value projection when `x` is a record value and type projection when `x` is a record type value.
