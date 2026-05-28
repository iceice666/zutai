## 30. Core grammar sketch

This grammar is intentionally a sketch. It defines the surface shape, not the full precedence parser.

```ebnf
File
  ::= TopDecl* Expr

TopDecl
  ::= Ident ":=" Expr                         (* inferred value binding *)
   | Ident ":" TypeExpr "=" Expr              (* annotated value binding *)
   | Ident "::" TypeExpr ("::" Clause)+       (* function: sig + clauses *)
   | Ident "::" Clause+                       (* function: clauses only, type inferred *)

Clause
  ::= Pattern ("->" Pattern)* "{" Block "}"

Block
  ::= (Ident ":=" Expr ";")* Expr

Expr
  ::= Literal
   | Ident
   | Atom
   | Record
   | Tuple
   | List
   | Lambda
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

Tuple
  ::= "(" TupleItem ("," TupleItem)* ")"
   | "(" ")"

TupleItem
  ::= Atom                                    (* tagged tuple discriminant *)
   | Ident "=" Expr                           (* named field *)
   | Expr                                     (* positional element *)

List
  ::= "[" ListItem* "]"

ListItem
  ::= Expr ";"

Lambda
  ::= "\" Pattern+ "=>" Expr                        (* short form *)
   | "\" Pattern+ "{" Block "}"                     (* block form *)

Import
  ::= "import" String
   | "import" ImportPath

ImportPath
  ::= FieldName ("." FieldName)*               (* unquoted shorthand, e.g. config.zti *)

TypeForm
  ::= "type" TypeRecord
   | "type" TypeUnion

TypeExpr
  ::= TypeRecord
   | TypeUnion
   | VariantType
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
  ::= FieldName ":" TypeExpr ";"
   | FieldName "?" ":" TypeExpr ";"

TypeRowTail
  ::= "..." ";"
   | "..." TypeVar ";"

TypeUnion
  ::= "[" TypeUnionItem* "]"

TypeUnionItem
  ::= VariantType ";"
   | TypeExpr ";"
   | TypeUnionRowTail

TypeUnionRowTail
  ::= "..." ";"
   | "..." TypeVar ";"

VariantType
  ::= "(" Atom ("," VariantField)* ")"

VariantField
  ::= Ident ":" TypeExpr

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
  ::= Pattern Guard? "=>" Expr ";"

Guard
  ::= "if" Expr

Pattern
  ::= Literal
   | Atom
   | Ident
   | "_"
   | TuplePattern
   | RecordPattern

TuplePattern
  ::= "(" Atom ("," PatternField)* ")"
   | "(" ")"

PatternField
  ::= Ident "=" Pattern

RecordPattern
  ::= "{" (FieldName "=" Pattern ";")* "}"

If
  ::= "if" Expr "then" Expr "else" Expr

Forall
  ::= "forall" TypeVar+ "." TypeExpr
```

Important grammar interpretation:

`TypeExpr` is a contextual grammar category used wherever the surrounding syntax expects a type: type annotations, `forall` bodies, function-type operands, optional-type operands, type-record fields, and type-union items. It can still be an arbitrary expression checked to evaluate to `Type`, but if it starts with `{` it is parsed as a record type literal and if it starts with `[` as a union type literal, rather than as a value record or value list.

The two field-binding sigils are kept strictly separate. `:` is **type annotation** and appears only in type positions: type-record fields (`type { host : Text; }`), variant type fields (`(#circle, radius : Float)`), and optional-field markers (`host? : Text`). `=` is **value/pattern binding** and appears everywhere a field is given a value or matched: value records (`{ host = "localhost"; }`), variant construction (`(#circle, radius = 5.0)`), and all patterns (record `{ host = h; }`, tuple/variant `(#circle, radius = r)`). This makes a `{ }` block unambiguous: a `:` inside it means a type record, a field `=` means a value record.

Block disambiguation: a `{` following a `->` return type in a `::` clause is a block body. A `{` in expression position is a value record if followed by `ident =`, and a block expression otherwise.

The forms `|>`, `<|`, `->`, `??`, application, field access, and postfix `?` are parsed by the precedence rules in [Operator precedence](27-operator-precedence.md).

The form:

```zt
select x { a; b; }
```

is value projection when `x` is a record value and type projection when `x` is a record type value.
