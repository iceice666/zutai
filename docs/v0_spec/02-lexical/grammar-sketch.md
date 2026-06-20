## Core grammar sketch

This grammar is intentionally a sketch. It defines the surface shape, not the full precedence parser.

```ebnf
File
  ::= (TopDecl TopSep)* Expr

Trivia
  ::= Whitespace
   | LineComment
   | BlockComment
   | DocComment

LineComment
  ::= "--" not ("[" | "|") chars-until-line-end

BlockComment
  ::= "--[" (BlockComment | any-char-except-unmatched-end)* "]--"

DocComment
  ::= "--|" chars-until-line-end

TopSep
  ::= line boundary at delimiter depth 0

TopDecl
  ::= Ident ":=" Expr                                          (* inferred value binding *)
   | Ident "::" TypeExpr "=" Expr                             (* typed value binding *)
   | Ident "::" TypeParamList? "type" TypeExpr                (* type alias / type value *)
   | Ident "::" TypeParamList? TypeExpr "{" FuncClause+ "}"   (* function: sig + block *)
   | Ident Pattern+ "=" Expr                                  (* function: no-sig single definition *)

FuncClause
  ::= "|" Pattern+ Guard? "=>" Expr ";"

Block
  ::= (Ident ":=" Expr ";")* Expr

Expr
  ::= Literal
   | Ident
   | Atom
   | TaggedValue
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

TaggedValue
  ::= Atom Record                              (* #tag { field = val; } *)

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
  ::= "\" Pattern+ "." Expr

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
  ::= "[" TaggedUnionItem* "]"

TaggedUnionItem
  ::= TagName ";"                           (* singleton tag member *)
   | TagName ":" TypeRecord ";"            (* tag with record payload *)

TagName
  ::= [A-Za-z_][A-Za-z0-9_-]*

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
  ::= "|" Pattern Guard? "=>" Expr ";"     (* identical to FuncClause *)

Guard
  ::= "if" Expr

Pattern
  ::= Literal
   | Atom
   | Ident
   | "_"
   | TaggedPattern
   | TuplePattern
   | RecordPattern

TaggedPattern
  ::= Atom RecordPattern                       (* #tag { field = p; } *)

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
  ::= "<" TypeVar ("," TypeVar)* ">"
```

Important grammar interpretation:

`TopSep` is a line boundary at delimiter depth zero. Top-level declarations are not terminated by semicolons. Semicolons terminate fields, list items, local block bindings, and match arms.

`TypeExpr` is a contextual grammar category used wherever the surrounding syntax expects a type: annotations, function-type operands, optional-type operands, type-record fields, and type-union items. Since `Type` values are first-class compile-time values in v0, a `TypeExpr` may include pure expressions that evaluate to `Type`.

`Name :: type TypeExpr` is the canonical type-alias form. `Name :: type TypeExpr` is preferred over `Name :: TypeExpr` when the right-hand side is a `type { ... }` or `type [ ... ]` expression.

A `<` appearing immediately after `::` in a `TopDecl` begins a `TypeParamList`. The `>` closes it; what follows is the type signature or `type` keyword. `<A, B>` is a two-parameter type param list.

`FuncClause` and `MatchCase` are the same production — `| pat => expr;` — and appear inside a `{ }` block. The enclosing context determines the role: `{` opened after a `:: TypeExpr` in a `TopDecl` is a function block; `{` opened after `match Expr` is a match block. A `|` inside an expression (e.g., in `||`) is lexed as part of that operator and is never a clause or arm introducer.

Declaration disambiguation: after `Ident "::"`, the parser looks for `type` (type alias), then a complete `TypeParamList? TypeExpr`. If that is followed by `=`, the declaration is a typed value binding; if followed by `{`, it opens a function block.

The two field-binding sigils are kept strictly separate. `:` is **type annotation** and appears only in type positions: type-record fields (`type { host : Text; }`), tagged union payload types (`circle: { radius : Float; }`), and optional-field markers (`host? : Text`). `=` is **value/pattern binding** and appears everywhere a field is given a value or matched: value records (`{ host = "localhost"; }`), tagged union construction (`#circle { radius = 5.0; }`), and all patterns (record `{ host = h; }`, tagged `#circle { radius = r; }`). This makes a `{ }` block unambiguous in type context versus value context.

Block disambiguation: a `{` in expression position is a value record if followed by `field_name =`, and a block expression otherwise. An empty `{}` in expression position is an empty value record.

Parentheses disambiguation: a parenthesized single expression is a `Group`; a comma makes a tuple. Parentheses starting with an atom and followed by named fields are still parsed as tuples, not as a separate form.

Lambda disambiguation: the `.` in `\params. body` is the lambda dot. It must be preceded by whitespace after the final pattern. `\x.y` with no space is a parse error; write `\x. y`. A `.` following an identifier in expression position (outside lambda pattern context) is always field access.

The forms `|>`, `<|`, `->`, `??`, `&&`, `||`, application, field access, and postfix `?` are parsed by the precedence rules in [Operator precedence](operator-precedence.md).

---

v1 grammar extensions (not in v0): row tails (`...;`, `...Rest;`) in record and union types; union spreads such as `...Shape;`; `select` projection; constraint declarations (`ConstraintDecl`); witness declarations (`WitnessDecl`); constrained/kinded type parameters. See [v1 spec](../../v1_spec/00-index.md).
