use rowan::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
pub enum SyntaxKind {
    // ── Trivia ───────────────────────────────────────────────────────────────
    WHITESPACE = 0,
    COMMENT,
    DOC_COMMENT,

    // ── Literals ─────────────────────────────────────────────────────────────
    INT,
    FLOAT,
    STRING,

    // ── Atom token (`#name`) ─────────────────────────────────────────────────
    ATOM,

    // ── Identifiers ──────────────────────────────────────────────────────────
    IDENT,
    UNDERSCORE,

    // ── Keywords ─────────────────────────────────────────────────────────────
    KW_TYPE,
    KW_MATCH,
    KW_IF,
    KW_THEN,
    KW_ELSE,
    KW_IMPORT,
    KW_TRUE,
    KW_FALSE,
    KW_NONE,
    KW_SELECT,

    // ── Compound symbols (maximal-munch, longest first) ───────────────────────
    COLON_COLON,       // ::
    COLON_EQ,          // :=
    COLON,             // :
    EQ_EQ,             // ==
    FAT_ARROW,         // =>
    EQ,                // =
    BANG_EQ,           // !=
    ARROW,             // ->
    NODE_COMMENT,      // --/
    MINUS,             // -
    OPTIONAL_DOT,      // ?.
    QUESTION_QUESTION, // ??
    QUESTION,          // ?
    LT_EQ,             // <=
    ARROW_PIPE,        // <|
    LT,                // <
    GT_EQ,             // >=
    GT,                // >
    PIPE_ARROW,        // |>
    PIPE_PIPE,         // ||
    AMP_AMP,           // &&
    ELLIPSIS,          // ...
    DOT,               // .
    BACKSLASH,         // \
    PLUS,              // +
    STAR,              // *
    SLASH,             // /
    SEMI,              // ;
    COMMA,             // ,
    L_BRACE,           // {
    R_BRACE,           // }
    L_BRACK,           // [
    R_BRACK,           // ]
    L_PAREN,           // (
    R_PAREN,           // )

    // ── Error token ──────────────────────────────────────────────────────────
    ERROR,

    // ── Node kinds ───────────────────────────────────────────────────────────
    FILE,
    ERROR_NODE,

    // Expressions
    LITERAL,
    PAREN_EXPR,
    RECORD_EXPR,
    VALUE_FIELD,
    TUPLE_EXPR,
    TUPLE_ITEM,
    LIST_EXPR,
    LIST_ITEM,
    LAMBDA_EXPR,
    MATCH_EXPR,
    MATCH_CASE,
    IF_EXPR,
    IMPORT_EXPR,
    IMPORT_PATH,
    TYPE_FORM,
    CALL_EXPR,
    ACCESS_EXPR,
    OPTIONAL_ACCESS_EXPR,
    BINARY_EXPR,
    PIPELINE_EXPR,

    // Type expressions
    TYPE_RECORD,
    TYPE_FIELD,
    TYPE_UNION,
    TYPE_UNION_ITEM,
    TYPE_TUPLE_FIELD,
    OPTIONAL_TYPE,
    FUNCTION_TYPE,

    // Field name node (reassembled from IDENT (MINUS IDENT)* in field positions)
    FIELD_NAME,

    // Patterns
    WILDCARD_PATTERN,
    PAREN_PATTERN,
    TUPLE_PATTERN,
    RECORD_PATTERN,
    PATTERN_FIELD,

    // Top-level declarations
    INFERRED_BINDING,
    ANNOTATED_BINDING,
    FUNC_DECL,
    TYPE_PARAM_LIST,
    CLAUSE,
    GUARD,
    BLOCK,
    NODE_COMMENT_NODE,
    LOCAL_BINDING,

    // ── Internal parser sentinels (never enter the green tree) ────────────────
    TOMBSTONE,
    EOF,
}

impl SyntaxKind {
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::WHITESPACE | SyntaxKind::COMMENT | SyntaxKind::DOC_COMMENT
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZutaiLanguage {}

impl Language for ZutaiLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        assert!(raw.0 <= SyntaxKind::LOCAL_BINDING as u16);
        // SAFETY: SyntaxKind is repr(u16) with consecutive discriminants 0..=LOCAL_BINDING.
        unsafe { std::mem::transmute(raw.0) }
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

pub type SyntaxNode = rowan::SyntaxNode<ZutaiLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<ZutaiLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<ZutaiLanguage>;

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> rowan::SyntaxKind {
        ZutaiLanguage::kind_to_raw(kind)
    }
}

/// Map a token's literal syntax to its [`SyntaxKind`].
///
/// Tokens that are not valid Rust token sequences (`?.`, `|>`, `<|`, `...`, `\`)
/// must use [`SyntaxKind`] variants directly.
#[macro_export]
macro_rules! T {
    // Keywords
    [type]   => { $crate::SyntaxKind::KW_TYPE };
    [match]  => { $crate::SyntaxKind::KW_MATCH };
    [if]     => { $crate::SyntaxKind::KW_IF };
    [then]   => { $crate::SyntaxKind::KW_THEN };
    [else]   => { $crate::SyntaxKind::KW_ELSE };
    [import] => { $crate::SyntaxKind::KW_IMPORT };
    [true]   => { $crate::SyntaxKind::KW_TRUE };
    [false]  => { $crate::SyntaxKind::KW_FALSE };
    [none]   => { $crate::SyntaxKind::KW_NONE };
    [select] => { $crate::SyntaxKind::KW_SELECT };
    // Literal-kind names
    [ident]  => { $crate::SyntaxKind::IDENT };
    [int]    => { $crate::SyntaxKind::INT };
    [float]  => { $crate::SyntaxKind::FLOAT };
    [string] => { $crate::SyntaxKind::STRING };
    [atom]   => { $crate::SyntaxKind::ATOM };
    // Wildcard
    [_]      => { $crate::SyntaxKind::UNDERSCORE };
    // Multi-char compound operators
    [::]     => { $crate::SyntaxKind::COLON_COLON };
    [:=]     => { $crate::SyntaxKind::COLON_EQ };
    [==]     => { $crate::SyntaxKind::EQ_EQ };
    [=>]     => { $crate::SyntaxKind::FAT_ARROW };
    [!=]     => { $crate::SyntaxKind::BANG_EQ };
    [->]     => { $crate::SyntaxKind::ARROW };
    [??]     => { $crate::SyntaxKind::QUESTION_QUESTION };
    [<=]     => { $crate::SyntaxKind::LT_EQ };
    [>=]     => { $crate::SyntaxKind::GT_EQ };
    [&&]     => { $crate::SyntaxKind::AMP_AMP };
    [||]     => { $crate::SyntaxKind::PIPE_PIPE };
    // Single-char operators
    [:]      => { $crate::SyntaxKind::COLON };
    [=]      => { $crate::SyntaxKind::EQ };
    [-]      => { $crate::SyntaxKind::MINUS };
    [?]      => { $crate::SyntaxKind::QUESTION };
    [<]      => { $crate::SyntaxKind::LT };
    [>]      => { $crate::SyntaxKind::GT };
    [.]      => { $crate::SyntaxKind::DOT };
    [+]      => { $crate::SyntaxKind::PLUS };
    [*]      => { $crate::SyntaxKind::STAR };
    [/]      => { $crate::SyntaxKind::SLASH };
    [;]      => { $crate::SyntaxKind::SEMI };
    [,]      => { $crate::SyntaxKind::COMMA };
    // Delimiters — char-literal syntax avoids parsing ambiguity with macro brackets
    ['(']    => { $crate::SyntaxKind::L_PAREN };
    [')']    => { $crate::SyntaxKind::R_PAREN };
    ['{']    => { $crate::SyntaxKind::L_BRACE };
    ['}']    => { $crate::SyntaxKind::R_BRACE };
    ['[']    => { $crate::SyntaxKind::L_BRACK };
    [']']    => { $crate::SyntaxKind::R_BRACK };
}
