; ─── Comments ───────────────────────────────────────────────────────────────────
(doc_comment)   @comment.documentation
(line_comment)  @comment.line
(block_comment) @comment.block

; ─── Keywords ──────────────────────────────────────────────────────────────────
"type"   @keyword.type
"match"  @keyword.control
"if"     @keyword.control
"then"   @keyword.control
"else"   @keyword.control
"import" @keyword.import

(bool) @boolean

; ─── Declaration operators ──────────────────────────────────────────────────────
"::"  @keyword.operator
":="  @operator

; ─── Operators ──────────────────────────────────────────────────────────────────
"->"  @operator
"=>"  @operator
"|>"  @operator
"<|"  @operator
"??"  @operator
"&&"  @operator
"||"  @operator
"=="  @operator
"!="  @operator
"<"   @operator
"<="  @operator
">"   @operator
">="  @operator
"+"   @operator
"-"   @operator
"*"   @operator
"/"   @operator
"?"   @operator
"\\"  @operator
"|"   @operator
"."   @operator
"?."  @operator

; ─── Literals ───────────────────────────────────────────────────────────────────
(string)     @string
(escape_seq) @string.escape
(integer)    @number
(float)      @number.float

; Atoms: #prod, #x86_64-linux
(atom) @label

; ─── Identifiers ────────────────────────────────────────────────────────────────
; Type names: start with uppercase
((identifier) @type
  (#match? @type "^[A-Z]"))

; Function / value names
((identifier) @variable
  (#not-match? @variable "^[A-Z]"))

; Field access targets (after `.`)
(field_identifier) @property

; Hyphenated identifiers used as field names inside records/type-records
(hyphenated_identifier) @property

; ─── Declaration names ──────────────────────────────────────────────────────────
; Type definition names
(declaration
  name: (identifier) @type
  (#match? @type "^[A-Z]"))

; Function / binding definition names
(declaration
  name: (identifier) @function.definition
  (#not-match? @function.definition "^[A-Z]"))

; ─── Record / type-record fields ────────────────────────────────────────────────
(record_field name: (identifier) @property)
(type_field   name: (identifier) @property)

; `:` in type-record fields
":" @operator

; ─── Wildcards / patterns ───────────────────────────────────────────────────────
(wildcard) @constant.builtin

; ─── Punctuation ────────────────────────────────────────────────────────────────
[ "{" "}" "[" "]" "(" ")" ] @punctuation.bracket
[ ";" "," ]                  @punctuation.delimiter
