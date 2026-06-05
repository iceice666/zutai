## 30. Core grammar sketch

This grammar is intentionally a sketch. It defines the surface shape, not the full precedence parser.

```ebnf
File
  ::= TopDecl* Expr

TopDecl
  ::= "--/" TopDecl                                           (* node comment: excluded decl *)
   | Ident ":=" Expr                                          (* inferred value binding *)
   | Ident ":" TypeExpr "=" Expr                             (* annotated value binding *)
   | Ident "::" TypeParamList? TypeExpr ("::" Clause)+       (* function: sig + clauses *)
   | Ident "::" Clause+                                      (* function: clauses only, type inferred *)

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
   | Binary
   | Pipeline
   | OptionalType
   | FunctionType

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
  ::= "--/" ValueField                        (* node comment: excluded field *)
   | FieldName "=" Expr ";"

Tuple
  ::= "(" TupleItem ("," TupleItem)* ")"
   | "(" ")"

TupleItem
  ::= "--/" TupleItem                         (* node comment: excluded item *)
   | FieldName "=" Expr                       (* named value field *)
   | FieldName ":" TypeExpr                   (* named type field, only in type context *)
   | Expr                                     (* positional element *)

List
  ::= "[" ListItem* "]"

ListItem
  ::= "--/" ListItem                          (* node comment: excluded item *)
   | Expr ";"

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
   | Tuple
   | OptionalType
   | FunctionType
   | Expr

TypeRecord
  ::= "{" TypeField* "}"

TypeField
  ::= FieldName ":" TypeExpr ";"
   | FieldName "?" ":" TypeExpr ";"

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
   | ParenPattern
   | TuplePattern
   | RecordPattern

ParenPattern
  ::= "(" Pattern ")"

TuplePattern
  ::= "(" ")"
   | "(" TuplePatternItem ("," TuplePatternItem)* ")"

TuplePatternItem
  ::= PatternField
   | Pattern

PatternField
  ::= FieldName "=" Pattern

RecordPattern
  ::= "{" (FieldName "=" Pattern ";")* "}"

If
  ::= "if" Expr "then" Expr "else" Expr

TypeParamList
  ::= "[" TypeVar ("," TypeVar)* "]"
```

Important grammar interpretation:

`TypeExpr` is a contextual grammar category used wherever the surrounding syntax expects a type: function-type operands, optional-type operands, type-record fields, and type-union items.

A `[` appearing immediately after `::` in a `TopDecl` is parsed as a `TypeParamList`, not a union type literal. The two are syntactically unambiguous: `TypeParamList` items are separated by `,` and have no trailing `;`, while `TypeUnion` items are terminated by `;`. `[A]` is therefore always a single-parameter type param list; `[A;]` is a single-variant union type.

The two field-binding sigils are kept strictly separate. `:` is **type annotation** and appears only in type positions: type-record fields (`type { host : Text; }`), tuple type fields (`(#circle, radius : Float)`), and optional-field markers (`host? : Text`). `=` is **value/pattern binding** and appears everywhere a field is given a value or matched: value records (`{ host = "localhost"; }`), tuple values (`(#circle, radius = 5.0)`), and all patterns (record `{ host = h; }`, tuple `(#circle, radius = r)`). This makes a `{ }` block unambiguous: a `:` inside it means a type record, a field `=` means a value record.

Block disambiguation: a `{` following a `->` return type in a `::` clause is a block body. A `{` in expression position is a value record if followed by `ident =`, and a block expression otherwise.

The forms `|>`, `<|`, `->`, `??`, application, field access, and postfix `?` are parsed by the precedence rules in [Operator precedence](operator-precedence.md).

---

v1 grammar extensions (not in v0): row tails (`...;`, `...Rest;`) in record and union types; `select` projection; constraint declarations (`ConstraintDecl`); witness declarations (`WitnessDecl`); constrained/kinded type parameters. See [v1 spec](../../v1_spec/00-index.md).
