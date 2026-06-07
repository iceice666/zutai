; ─── Keywords ──────────────────────────────────────────────────────────────────
(bool) @boolean

; ─── Literals ───────────────────────────────────────────────────────────────────
(string)     @string
(escape_seq) @string.escape
(integer)    @number
(float)      @number.float

; Atoms: #prod, #x86_64-linux
(atom) @label

; ─── Field names ────────────────────────────────────────────────────────────────
(pair name: (field_name) @property)

; ─── Operators / punctuation ────────────────────────────────────────────────────
"=" @operator
[ "{" "}" "[" "]" ] @punctuation.bracket
";"                  @punctuation.delimiter
