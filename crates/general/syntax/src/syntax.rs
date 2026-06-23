use crate::numlit::{PostfixCheck, classify_postfix};
use rowan::{GreenNodeBuilder, Language};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZutaiLang {}

impl Language for ZutaiLang {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        SyntaxKind::from_raw(raw.0)
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

pub type SyntaxNode = rowan::SyntaxNode<ZutaiLang>;
pub type SyntaxToken = rowan::SyntaxToken<ZutaiLang>;
pub type SyntaxElement = rowan::SyntaxElement<ZutaiLang>;

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SyntaxKind {
    SourceFile = 0,
    Error = 1,
    Whitespace = 2,
    Newline = 3,
    LineComment = 4,
    DocComment = 5,
    BlockComment = 6,
    Ident = 7,
    FieldName = 8,
    Atom = 9,
    Integer = 10,
    Float = 11,
    String = 12,
    KeywordType = 13,
    KeywordMatch = 14,
    KeywordIf = 15,
    KeywordThen = 16,
    KeywordElse = 17,
    KeywordImport = 18,
    KeywordTrue = 19,
    KeywordFalse = 20,
    KeywordSelect = 21,
    KeywordPerform = 22,
    KeywordHandle = 23,
    KeywordWith = 24,
    KeywordResume = 25,
    LBrace = 26,
    RBrace = 27,
    LBracket = 28,
    RBracket = 29,
    LParen = 30,
    RParen = 31,
    Semicolon = 32,
    Comma = 33,
    Dot = 34,
    DotDotDot = 35,
    Colon = 36,
    ColonColon = 37,
    ColonEq = 38,
    Eq = 39,
    FatArrow = 40,
    Backslash = 41,
    Pipe = 42,
    PipeForward = 43,
    PipePipe = 44,
    PipeBackward = 45,
    Arrow = 46,
    Bang = 47,
    Question = 48,
    QuestionQuestion = 49,
    QuestionDot = 50,
    Plus = 51,
    Minus = 52,
    Star = 53,
    Slash = 54,
    AmpAmp = 55,
    EqEq = 56,
    BangEq = 57,
    Lt = 58,
    LtEq = 59,
    Gt = 60,
    GtEq = 61,
    Unknown = 62,
    At = 63,
    PostfixedNumber = 64,
    SelectOperator = 65,
    Caret = 66,
}

impl SyntaxKind {
    fn from_raw(raw: u16) -> Self {
        match raw {
            0 => Self::SourceFile,
            1 => Self::Error,
            2 => Self::Whitespace,
            3 => Self::Newline,
            4 => Self::LineComment,
            5 => Self::DocComment,
            6 => Self::BlockComment,
            7 => Self::Ident,
            8 => Self::FieldName,
            9 => Self::Atom,
            10 => Self::Integer,
            11 => Self::Float,
            12 => Self::String,
            13 => Self::KeywordType,
            14 => Self::KeywordMatch,
            15 => Self::KeywordIf,
            16 => Self::KeywordThen,
            17 => Self::KeywordElse,
            18 => Self::KeywordImport,
            19 => Self::KeywordTrue,
            20 => Self::KeywordFalse,
            21 => Self::KeywordSelect,
            22 => Self::KeywordPerform,
            23 => Self::KeywordHandle,
            24 => Self::KeywordWith,
            25 => Self::KeywordResume,
            26 => Self::LBrace,
            27 => Self::RBrace,
            28 => Self::LBracket,
            29 => Self::RBracket,
            30 => Self::LParen,
            31 => Self::RParen,
            32 => Self::Semicolon,
            33 => Self::Comma,
            34 => Self::Dot,
            35 => Self::DotDotDot,
            36 => Self::Colon,
            37 => Self::ColonColon,
            38 => Self::ColonEq,
            39 => Self::Eq,
            40 => Self::FatArrow,
            41 => Self::Backslash,
            42 => Self::Pipe,
            43 => Self::PipeForward,
            44 => Self::PipePipe,
            45 => Self::PipeBackward,
            46 => Self::Arrow,
            47 => Self::Bang,
            48 => Self::Question,
            49 => Self::QuestionQuestion,
            50 => Self::QuestionDot,
            51 => Self::Plus,
            52 => Self::Minus,
            53 => Self::Star,
            54 => Self::Slash,
            55 => Self::AmpAmp,
            56 => Self::EqEq,
            57 => Self::BangEq,
            58 => Self::Lt,
            59 => Self::LtEq,
            60 => Self::Gt,
            61 => Self::GtEq,
            62 => Self::Unknown,
            63 => Self::At,
            64 => Self::PostfixedNumber,
            65 => Self::SelectOperator,
            66 => Self::Caret,
            _ => Self::Unknown,
        }
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        rowan::SyntaxKind(kind as u16)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: SyntaxKind,
    pub text: &'a str,
    pub offset: usize,
}

pub fn tokenize(src: &str) -> Vec<Token<'_>> {
    let mut lexer = Lexer::new(src);
    lexer.lex();
    lexer.tokens
}

pub fn parse_lossless(src: &str) -> SyntaxNode {
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::SourceFile.into());
    for token in tokenize(src) {
        builder.token(token.kind.into(), token.text);
    }
    builder.finish_node();
    SyntaxNode::new_root(builder.finish())
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    tokens: Vec<Token<'a>>,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            pos: 0,
            tokens: Vec::new(),
        }
    }

    fn lex(&mut self) {
        while self.pos < self.src.len() {
            let start = self.pos;
            let kind = self.next_kind();
            self.push(kind, start);
        }
    }

    fn next_kind(&mut self) -> SyntaxKind {
        let start = self.pos;
        if self.starts_with("--[") {
            self.consume_block_comment();
            return SyntaxKind::BlockComment;
        }
        if self.starts_with("--|") {
            self.consume_line();
            return SyntaxKind::DocComment;
        }
        if self.starts_with("--") {
            self.consume_line();
            return SyntaxKind::LineComment;
        }

        let ch = self.peek_char().expect("lexer called at valid position");
        match ch {
            '\n' => {
                self.pos += 1;
                SyntaxKind::Newline
            }
            ' ' | '\t' | '\r' => {
                self.consume_while(|c| matches!(c, ' ' | '\t' | '\r'));
                SyntaxKind::Whitespace
            }
            '"' => {
                self.consume_string();
                SyntaxKind::String
            }
            '#' => {
                self.consume_atom();
                SyntaxKind::Atom
            }
            '0'..='9' => self.consume_number(false),
            '-' if self.next_char_is_ascii_digit() => self.consume_number(true),
            'A'..='Z' | 'a'..='z' | '_' => self.consume_word(start),
            '{' => self.bump(SyntaxKind::LBrace),
            '}' => self.bump(SyntaxKind::RBrace),
            '[' => self.bump(SyntaxKind::LBracket),
            ']' => self.bump(SyntaxKind::RBracket),
            '(' => self.bump(SyntaxKind::LParen),
            ')' => self.bump(SyntaxKind::RParen),
            ';' => self.bump(SyntaxKind::Semicolon),
            ',' => self.bump(SyntaxKind::Comma),
            '.' if self.starts_with("...") => self.bump_n(3, SyntaxKind::DotDotDot),
            '.' => self.bump(SyntaxKind::Dot),
            ':' if self.starts_with("::") => self.bump_n(2, SyntaxKind::ColonColon),
            ':' if self.starts_with(":=") => self.bump_n(2, SyntaxKind::ColonEq),
            ':' => self.bump(SyntaxKind::Colon),
            '=' if self.starts_with("==") => self.bump_n(2, SyntaxKind::EqEq),
            '=' if self.starts_with("=>") => self.bump_n(2, SyntaxKind::FatArrow),
            '=' => self.bump(SyntaxKind::Eq),
            '\\' => self.bump(SyntaxKind::Backslash),
            '|' if self.starts_with("|>") => self.bump_n(2, SyntaxKind::PipeForward),
            '|' if self.starts_with("||") => self.bump_n(2, SyntaxKind::PipePipe),
            '|' => self.bump(SyntaxKind::Pipe),
            '<' if self.starts_with("<|") => self.bump_n(2, SyntaxKind::PipeBackward),
            '<' if self.starts_with("<=") => self.bump_n(2, SyntaxKind::LtEq),
            '<' => self.bump(SyntaxKind::Lt),
            '>' if self.starts_with(">>=") => self.bump_n(3, SyntaxKind::SelectOperator),
            '>' if self.starts_with(">=") => self.bump_n(2, SyntaxKind::GtEq),
            '>' => self.bump(SyntaxKind::Gt),
            '-' if self.starts_with("->") => self.bump_n(2, SyntaxKind::Arrow),
            '-' => self.bump(SyntaxKind::Minus),
            '?' if self.starts_with("??") => self.bump_n(2, SyntaxKind::QuestionQuestion),
            '?' if self.starts_with("?.") => self.bump_n(2, SyntaxKind::QuestionDot),
            '?' => self.bump(SyntaxKind::Question),
            '+' => self.bump(SyntaxKind::Plus),
            '*' => self.bump(SyntaxKind::Star),
            '/' => self.bump(SyntaxKind::Slash),
            '&' if self.starts_with("&&") => self.bump_n(2, SyntaxKind::AmpAmp),
            '!' if self.starts_with("!=") => self.bump_n(2, SyntaxKind::BangEq),
            '!' => self.bump(SyntaxKind::Bang),
            '^' => self.bump(SyntaxKind::Caret),
            '@' => self.bump(SyntaxKind::At),
            _ => {
                self.pos += ch.len_utf8();
                SyntaxKind::Unknown
            }
        }
    }

    fn push(&mut self, kind: SyntaxKind, start: usize) {
        self.tokens.push(Token {
            kind,
            text: &self.src[start..self.pos],
            offset: start,
        });
    }

    fn bump(&mut self, kind: SyntaxKind) -> SyntaxKind {
        self.pos += self.peek_char().map_or(1, char::len_utf8);
        kind
    }

    fn bump_n(&mut self, n: usize, kind: SyntaxKind) -> SyntaxKind {
        self.pos += n;
        kind
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s)
    }

    fn peek_char(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn next_char_is_ascii_digit(&self) -> bool {
        self.src[self.pos + 1..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit())
    }

    fn consume_while(&mut self, mut f: impl FnMut(char) -> bool) {
        while let Some(ch) = self.peek_char() {
            if !f(ch) {
                break;
            }
            self.pos += ch.len_utf8();
        }
    }

    fn consume_line(&mut self) {
        self.consume_while(|ch| ch != '\n');
    }

    fn consume_block_comment(&mut self) {
        self.pos += 3;
        let mut depth = 1usize;
        while self.pos < self.src.len() && depth > 0 {
            if self.starts_with("--[") {
                self.pos += 3;
                depth += 1;
            } else if self.starts_with("]--") {
                self.pos += 3;
                depth -= 1;
            } else if let Some(ch) = self.peek_char() {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn consume_string(&mut self) {
        self.pos += 1;
        while self.pos < self.src.len() {
            let Some(ch) = self.peek_char() else { break };
            self.pos += ch.len_utf8();
            match ch {
                '\\' => {
                    if let Some(escaped) = self.peek_char() {
                        self.pos += escaped.len_utf8();
                    }
                }
                '"' => break,
                _ => {}
            }
        }
    }

    fn consume_atom(&mut self) {
        self.pos += 1;
        self.consume_while(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    }

    fn consume_word(&mut self, start: usize) -> SyntaxKind {
        self.consume_while(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        match &self.src[start..self.pos] {
            "type" => SyntaxKind::KeywordType,
            "match" => SyntaxKind::KeywordMatch,
            "if" => SyntaxKind::KeywordIf,
            "then" => SyntaxKind::KeywordThen,
            "else" => SyntaxKind::KeywordElse,
            "import" => SyntaxKind::KeywordImport,
            "true" => SyntaxKind::KeywordTrue,
            "false" => SyntaxKind::KeywordFalse,
            "select" => SyntaxKind::KeywordSelect,
            "perform" => SyntaxKind::KeywordPerform,
            "handle" => SyntaxKind::KeywordHandle,
            "with" => SyntaxKind::KeywordWith,
            "resume" => SyntaxKind::KeywordResume,
            _ => SyntaxKind::Ident,
        }
    }

    fn consume_number(&mut self, has_sign: bool) -> SyntaxKind {
        if has_sign {
            self.pos += 1;
        }
        self.consume_while(|ch| ch.is_ascii_digit());
        let mut has_frac_or_exp = false;
        if self.starts_with(".")
            && self.src[self.pos + 1..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit())
        {
            has_frac_or_exp = true;
            self.pos += 1;
            self.consume_while(|ch| ch.is_ascii_digit());
        }
        if matches!(self.peek_char(), Some('e' | 'E')) {
            let exp = self.pos;
            self.pos += 1;
            if matches!(self.peek_char(), Some('+' | '-')) {
                self.pos += 1;
            }
            let digit_start = self.pos;
            self.consume_while(|ch| ch.is_ascii_digit());
            if self.pos > digit_start {
                has_frac_or_exp = true;
            } else {
                self.pos = exp;
            }
        }

        let body_kind = if has_frac_or_exp {
            SyntaxKind::Float
        } else {
            SyntaxKind::Integer
        };
        let body_end = self.pos;
        self.consume_while(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        match classify_postfix(&self.src[body_end..self.pos], has_sign, has_frac_or_exp) {
            PostfixCheck::None => body_kind,
            PostfixCheck::Valid(_) => SyntaxKind::PostfixedNumber,
            PostfixCheck::Unknown
            | PostfixCheck::IntOnFloatBody
            | PostfixCheck::UnsignedNegative => SyntaxKind::Error,
        }
    }
}
