## Core grammar reference

This is a human-readable reference for the implemented general-mode (`.zt`) surface grammar. It is not a parser-generator grammar: the executable source of truth is `crates/general/syntax/src/parser/`, and operator binding is defined in [Operator precedence](operator-precedence.md).

Notation: quoted text is literal syntax, `?` means optional, `*` means zero or more, `+` means one or more, and `|` separates alternatives.

```ebnf
File
  ::= TopDecl* Expr EOF

TopDecl
  ::= Ident "::=" Expr ";"
   | Ident "::" TypeExpr "=" Expr ";"
   | Ident "::" TypeParamList? "type" TypeExpr ";"
   | Ident "::" TypeParamList? TypeExpr FunctionClause+
   | Ident Pattern+ "=" Expr ";"
   | Ident "::" TypeParamList? "@" TypeAtom "{" ConstraintMethod* "}" "derive"?
   | Ident "@" TypeAtom "::" TypeParamList? WitnessBody

FunctionClause
  ::= "=" Pattern+ Guard? "=>" Expr ";"?

ClauseBlock
  ::= "{" MatchClause* "}"

MatchClause
  ::= "|" Pattern+ Guard? "=>" Expr ";"

Guard
  ::= "if" Expr

WitnessBody
  ::= "derive"
   | "{" WitnessField* "}"

WitnessField
  ::= MethodName "=" Expr ";"?

ConstraintMethod
  ::= MethodName "?"? "::" TypeParamList? TypeExpr ";"
   | MethodName "?"? "::" TypeParamList? TypeExpr FunctionClause+ ";"?

MethodName
  ::= Ident
   | "(" operator-token ")"

TypeParamList
  ::= "<" TypeParam ("," TypeParam)* ">"

TypeParam
  ::= Ident
   | Ident ":" Ident ("+" Ident)*
   | Ident "::" TypeExpr
   | LevelBinder

LevelBinder
  ::= "$" Ident
```

### Expressions

Expression precedence is listed from lowest binding to highest binding.

```ebnf
Expr
  ::= Pipeline

Pipeline
  ::= Coalesce (("|>" | "<|") Coalesce)*

Coalesce
  ::= Or ("??" Coalesce)?

Or
  ::= And ("||" And)*

And
  ::= Compare ("&&" Compare)*

Compare
  ::= Add (CompareOp Add)?

CompareOp
  ::= "==" | "!=" | "<" | "<=" | ">" | ">="

Add
  ::= Mul (("+" | "-") Mul)*

Mul
  ::= SelectOp (("*" | "/" | "%") SelectOp)*

SelectOp
  ::= Application (">>=" SelectFields)*

Application
  ::= Postfix (application-whitespace Postfix)*

Postfix
  ::= AtomExpr (("." | "?.") FieldName)*

AtomExpr
  ::= Literal
   | Ident
   | Atom
   | TaggedValue
   | Group
   | Block
   | Record
   | Tuple
   | List
   | Lambda
   | If
   | Match
   | Import
   | TypeForm
   | Select
   | Perform
   | Handle
   | Resume

Literal
  ::= "true" | "false" | Number | String

TaggedValue
  ::= Atom Record
   | Atom TaggedTuplePayload

Group
  ::= "(" Expr ")"

Record
  ::= "{}"
   | "{" ValueField+ "}"

ValueField
  ::= FieldName "=" Expr ";"
   | FieldName "=" ";"

List
  ::= "{;}"
   | "{" (Expr ";")+ "}"

Block
  ::= "[]"
   | "[" LocalBinding* Expr (";" Expr)* ";"? "]"

LocalBinding
  ::= Ident ":=" Expr ";"
   | Ident ":" TypeExpr "=" Expr ";"

Tuple
  ::= "()"
   | "(" TupleItem "," (TupleItem ("," TupleItem)* ","?)? ")"

TaggedTuplePayload
  ::= "(" (TupleItem ("," TupleItem)* ","?)? ")"

TupleItem
  ::= FieldName "=" Expr
   | Expr

Lambda
  ::= "\\" Pattern+ "." whitespace Expr

If
  ::= "if" Expr "then" Expr "else" Expr

Match
  ::= "match" Expr ClauseBlock

Import
  ::= "import" String
   | "import" ImportPath

ImportPath
  ::= FieldName ("." FieldName)*

TypeForm
  ::= "type" TypeExpr

Select
  ::= "select" Postfix SelectFields

SelectFields
  ::= "{" (FieldName ";")* "}"

Perform
  ::= ("perform" | "!") EffectPath Expr

Handle
  ::= "handle" Expr "with" "{" HandleClause* "}"

HandleClause
  ::= EffectPath "=" Expr ";"

Resume
  ::= ("resume" | "^") Expr

EffectPath
  ::= FieldName ("." FieldName)*
```

### Type expressions

Type expressions use their own precedence parser. Function arrows are right-associative.

```ebnf
TypeExpr
  ::= TypeEffect ("->" TypeExpr)?

TypeEffect
  ::= TypeSelectOp ("!" EffectRow)?

TypeSelectOp
  ::= TypeApplication (">>=" SelectFields)*

TypeApplication
  ::= TypePostfix (inline-whitespace TypePostfix)*

TypePostfix
  ::= TypeAtom (("." | "?.") FieldName | "?")*

TypeAtom
  ::= Ident
   | Atom
   | "true"
   | "false"
   | TypeRecord
   | TypeUnion
   | TypeTupleOrGroup
   | TypeSelect
   | UniverseType
   | ExprEscape

UniverseType
  ::= "$" LevelArg

LevelArg
  ::= IntLit
   | Ident
   | "(" Level ")"

Level
  ::= LevelAtom ("+" IntLit)?
   | "max" LevelArg LevelArg
   | LevelAtom

LevelAtom
  ::= IntLit
   | Ident
   | "(" Level ")"

TypeRecord
  ::= "{" TypeRecordField* RowTail? "}"

TypeRecordField
  ::= FieldName "?"? ":" TypeExpr ";"

TypeUnion
  ::= "{" (UnionVariant | RowTail)+ "}"

UnionVariant
  ::= Atom ";"
   | Atom ":" TypeExpr ";"

RowTail
  ::= "..." ";"
   | "..." Ident ";"
   | "..." Ident ("." FieldName)+ ";"

TypeTupleOrGroup
  ::= "()"
   | "(" TypeExpr ")"
   | "(" NamedTypeTupleItem ")"
   | "(" TypeTupleItem "," (TypeTupleItem ("," TypeTupleItem)* ","?)? ")"

NamedTypeTupleItem
  ::= FieldName ":" TypeExpr

TypeTupleItem
  ::= NamedTypeTupleItem
   | TypeExpr

TypeSelect
  ::= "select" TypePostfix SelectFields

EffectRow
  ::= "{" (EffectRowItem (("," | ";") EffectRowItem)* ("," | ";")?)? "}"

EffectRowItem
  ::= EffectOp
   | EffectRowTail

EffectRowTail
  ::= "..."
   | "..." Ident
   | "..." Ident ("." FieldName)+

EffectOp
  ::= EffectPath
   | EffectPath TypePostfix
   | EffectPath ":" TypeExpr

ExprEscape
  ::= application-level Expr
```

### Patterns

```ebnf
Pattern
  ::= "_"
   | Literal
   | Atom
   | Ident
   | TaggedPattern
   | TuplePattern
   | RecordPattern
   | ListPattern

TaggedPattern
  ::= Atom RecordPattern
   | Atom TaggedTuplePatternPayload

TuplePattern
  ::= "()"
   | "(" TuplePatternItem "," (TuplePatternItem ("," TuplePatternItem)* ","?)? ")"

TaggedTuplePatternPayload
  ::= "(" (TuplePatternItem ("," TuplePatternItem)* ","?)? ")"

TuplePatternItem
  ::= FieldName "=" Pattern
   | Pattern

RecordPattern
  ::= "{" (FieldName "=" Pattern ";")* "}"

ListPattern
  ::= "{;}"
   | "{" Pattern "}"
   | "{" Pattern "..." Pattern "}"
```

### Lexical forms

```ebnf
Ident
  ::= ("_" | XID_Start) ("_" | XID_Continue)* ("'"*) "?"?

FieldName
  ::= ("_" | XID_Start) ("_" | XID_Continue)* "?"?

Atom
  ::= "#" ("_" | XID_Start) ("_" | XID_Continue | "-")*

Number
  ::= NumericBody NumberTypePostfix?

NumericBody
  ::= "-"? digit+ ("." digit+)? (("e" | "E") ("+" | "-")? digit+)?

NumberTypePostfix
  ::= "i8" | "i16" | "i32" | "i64"
   | "u8" | "u16" | "u32" | "u64"
   | "f32" | "f64"
   | "p32" | "p64" | "p32e" digit+ | "p64e" digit+

String
  ::= '"' string-fragment* '"'

LineComment
  ::= "--" not ("[" | "|") chars-until-line-end

DocComment
  ::= "--|" chars-until-line-end

BlockComment
  ::= "--[" (BlockComment | any-char-except-unmatched-end)* "]--"
```

Reserved words are not identifiers: `type`, `match`, `if`, `then`, `else`, `import`, `true`, `false`, `select`, `perform`, `handle`, `with`, `resume`.

### Interpretation rules

- A file contains zero or more top-level declarations followed by one optional final expression. Each value-like top-level declaration ends in `;`; the trailing expression has no `;`. A trailing `;` (or no tail at all) yields `()`.
- Top-level function declarations and constraint-method defaults use `= pat => body` clauses after the signature. `match` arms use `{ | pat => body; }` clause blocks.
- `;` is the universal terminator/separator. An expression written with a trailing `;` is a statement of type `()` (Unit), Rust-style.
- The container glyph picks the shape: `{ … }` is a parallel container — a record (`name = value;` entries) or a list (bare `value;` entries) — while `[ … ]` is a serial do-block (local bindings followed by a tail expression that is the block's value).
- The scope picks the binding operator: top-level (the parallel/letrec scope) uses `::=` (inferred) and `:: T =` (typed); a local binding inside a `[ … ]` do-block uses `:=` (inferred) and `: T =` (typed). Mnemonic: `::` is top-level, `:` is local.
- `:` is type binding; `=` is value or pattern binding. Type record fields, type tuple fields, and union payload annotations use `:`. Value records, tuples, witness fields, and patterns use `=`.
- In expression position, `{}` is the empty value record and `{;}` is the empty list. A non-empty `{ ... }` is a value record when its non-spread items start as `FieldName =`; otherwise it is a list of `;`-terminated expressions. Value records and lists accept spread items (`* record;`, `* list;`) in item order. A spread-only literal (`{ * x; }`) requires an expected record/list type. A `[ ... ]` is a do-block; an empty `[]` is the empty do-block.
- A value-record field written `name =;` is field-pun shorthand for `name = name;`: the omitted value is the identifier `name`. It applies to record literals and record-update (`with`) fields.
- In type position, `{ field : Type; }` is a record type and `{ #tag; }` is a union type.
- Record row tails (`...;`, `...Rest;`) are final and unique. Named/qualified row spreads use explicit `* Shape;` / `* m.Shape;`; union row spreads may appear among variants.
- Effect rows accept named or qualified effect-type spreads (`* Name;`, `* m.Name;`) before the final row tail. Anonymous tails and row-variable tails (`...;`, `...e;`) are final.
- Function application by whitespace is left-associative. At delimiter depth zero, a newline stops application unless an enclosing operator production consumes it.
- `|>` and `<|` chains are left-associative but cannot mix directions in a single chain.
- `??` is right-associative. Comparisons are non-associative: write `(a < b) && (b < c)`, not `a < b < c`.
- `?` is postfix optional type syntax only in type context. `?.` is optional field access in value and type contexts. `??` is value-level defaulting.
- A parenthesized single positional type `(T)` is grouping. A named type tuple item `(field : T)` remains a one-field tuple type. Value and pattern tuples require a comma except for unit `()`.
- Tagged values and patterns support record payloads (`#tag { field = value; }`) and tuple payloads (`#tag (value, name = value)`).
- `type TypeExpr` constructs a first-class type value in expression position. `ExprEscape` keeps pure compile-time expressions available in type contexts.
- General-mode `NumberTypePostfix` is valid only on `Number` literals. Integer postfixes reject fractional/exponent bodies; unsigned postfixes also reject a leading `-`; float postfixes accept integer, fractional, or exponent bodies. A non-empty alphanumeric/underscore run after a numeric body must be one of the listed postfixes.
