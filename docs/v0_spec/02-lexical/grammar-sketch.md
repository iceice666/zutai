## Core grammar sketch

This grammar is intentionally a sketch. It defines the surface shape, not the full precedence parser.

```ebnf
File
  ::= (TopDecl TopSep)* Expr

TopSep
  ::= line boundary at delimiter depth 0

TopDecl
  ::= Ident ":=" Expr                                          (* inferred value binding *)
   | Ident ":" TypeExpr "=" Expr                              (* annotated value binding *)
   | Ident "::" TypeParamList? "type" TypeExpr                (* type alias / type value *)
   | Ident "::" TypeParamList? TypeExpr ("::" Clause)+        (* function: sig + clauses *)
   | Ident "::" Clause+                                       (* function: clauses only, type inferred *)

Clause
  ::= Pattern ("->" Pattern)* Guard? "{" Block "}"

Block
  ::= (Ident ":=" Expr ";")* Expr

Expr
  ::= Literal
   | Ident
   | Atom
   | Group
   | BlockExpr
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

Group
  ::= "(" Expr ")"

BlockExpr
  ::= "{" Block "}"

Literal
  ::= "true"
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
  ::= "(" TupleItem "," TupleItem ("," TupleItem)* ")"
   | "(" ")"

TupleItem
  ::= FieldName "=" Expr                       (* named field *)
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
  ::= "type" TypeExpr

TypeExpr
  ::= TypeRecord
   | TypeUnion
   | TypeTuple
   | OptionalType
   | FunctionType
   | Expr

TypeTuple
  ::= "(" TypeTupleItem "," TypeTupleItem ("," TypeTupleItem)* ")"
   | "(" ")"

TypeTupleItem
  ::= FieldName ":" TypeExpr                   (* named field *)
   | TypeExpr                                 (* positional element *)

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
   | "&&" | "||"
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
  ::= "(" TuplePatternItem "," TuplePatternItem ("," TuplePatternItem)* ")"
   | "(" ")"

TuplePatternItem
  ::= FieldName "=" Pattern                    (* named field *)
   | Pattern                                  (* positional element *)

RecordPattern
  ::= "{" (FieldName "=" Pattern ";")* "}"

If
  ::= "if" Expr "then" Expr "else" Expr

TypeParamList
  ::= "[" TypeVar ("," TypeVar)* "]"
```

Important grammar interpretation:

`TopSep` is a line boundary at delimiter depth zero. Top-level declarations are not terminated by semicolons. Semicolons terminate fields, list items, local block bindings, and match arms.

`TypeExpr` is a contextual grammar category used wherever the surrounding syntax expects a type: annotations, function-type operands, optional-type operands, type-record fields, and type-union items. Since `Type` values are first-class compile-time values in v0, a `TypeExpr` may include pure expressions that evaluate to `Type`.

`Name :: type TypeExpr` is the canonical type-alias form. `Name : Type = type TypeExpr` is also an annotated type-valued binding, but examples should prefer `:: type` for named types.

A `[` appearing immediately after `::` in a `TopDecl` is parsed as a `TypeParamList`, not a union type literal. The two are syntactically unambiguous: `TypeParamList` items are separated by `,` and have no trailing `;`, while `TypeUnion` items are terminated by `;`. `[A]` is therefore always a single-parameter type param list; `[A;]` is a single-member union type.

The two field-binding sigils are kept strictly separate. `:` is **type annotation** and appears only in type positions: type-record fields (`type { host : Text; }`), named tuple type fields (`(#circle, radius : Float)`), and optional-field markers (`host? : Text`). `=` is **value/pattern binding** and appears everywhere a field is given a value or matched: value records (`{ host = "localhost"; }`), named tuple construction (`(#circle, radius = 5.0)`), and all patterns (record `{ host = h; }`, tuple `(#circle, radius = r)`). This makes a `{ }` block unambiguous in type context versus value context.

Block disambiguation: a `{` following the pattern list in a `::` clause is a block body. A `{` in expression position is a value record if followed by `field_name =`, and a block expression otherwise. An empty `{}` in expression position is an empty value record.

Parentheses disambiguation: a parenthesized single expression is a `Group`; a comma makes a tuple. Parentheses starting with an atom and followed by named fields are still parsed as tuples, not as a separate form.

The forms `|>`, `<|`, `->`, `??`, `&&`, `||`, application, field access, and postfix `?` are parsed by the precedence rules in [Operator precedence](operator-precedence.md).

---

v1 grammar extensions (not in v0): row tails (`...;`, `...Rest;`) in record and union types; union spreads such as `...Shape;`; `select` projection; constraint declarations (`ConstraintDecl`); witness declarations (`WitnessDecl`); constrained/kinded type parameters. See [v1 spec](../../v1_spec/00-index.md).
