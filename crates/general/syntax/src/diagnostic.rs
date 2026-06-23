use crate::error::ParseErrorKind;
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelStyle {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticLabel {
    pub span: Span,
    pub message: String,
    pub style: LabelStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub span: Span,
    pub replacement: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applicability {
    MachineApplicable,
    MaybeIncorrect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticFix {
    pub title: String,
    pub edits: Vec<TextEdit>,
    pub applicability: Applicability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub kind: ParseErrorKind,
    pub code: &'static str,
    pub message: String,
    pub labels: Vec<DiagnosticLabel>,
    pub help: Option<String>,
    pub notes: Vec<String>,
    pub fixes: Vec<DiagnosticFix>,
}

impl Diagnostic {
    pub fn from_parse_error(err: crate::error::ParseError) -> Self {
        let kind = err.kind;
        let code = kind.code();
        let help = kind.help().map(str::to_string);
        let span = err.span;
        // For the generic fallback the synthesized `message` carries the
        // specific "unexpected `x`" / "unexpected end of input" detail; surface
        // it at the caret and keep a stable "syntax error" headline. Curated
        // kinds keep their static label and detailed message.
        let (headline, label_message) = if matches!(kind, ParseErrorKind::Generic) {
            ("syntax error".to_string(), err.message)
        } else {
            (err.message, kind.label().to_string())
        };
        let labels = vec![DiagnosticLabel {
            span,
            message: label_message,
            style: LabelStyle::Primary,
        }];
        let fixes = fix_for(span, &kind);
        Self {
            severity: Severity::Error,
            kind,
            code,
            message: headline,
            labels,
            help,
            notes: Vec::new(),
            fixes,
        }
    }

    pub fn primary_span(&self) -> Span {
        self.labels
            .iter()
            .find(|label| label.style == LabelStyle::Primary)
            .map(|label| label.span)
            .unwrap_or_default()
    }
}

fn fix_for(span: Span, kind: &ParseErrorKind) -> Vec<DiagnosticFix> {
    let Some((title, replacement)) = replacement_for(kind) else {
        return Vec::new();
    };
    vec![DiagnosticFix {
        title: title.to_string(),
        edits: vec![TextEdit {
            span,
            replacement: replacement.to_string(),
        }],
        applicability: Applicability::MachineApplicable,
    }]
}

fn replacement_for(kind: &ParseErrorKind) -> Option<(&'static str, &'static str)> {
    match kind {
        ParseErrorKind::LambdaArrow => Some(("Use lambda dot", ".")),
        ParseErrorKind::LambdaDotNeedsWhitespace => {
            Some(("Insert whitespace after lambda dot", ". "))
        }
        ParseErrorKind::ValueRecordFieldUsesColon => Some(("Use `=` for value record field", "=")),
        ParseErrorKind::TopLevelSingleColon => Some(("Use `::` for typed binding", "::")),
        ParseErrorKind::TypeRecordFieldUsesEquals => Some(("Use `:` for type record field", ":")),
        ParseErrorKind::MissingListItemSemicolon => {
            Some(("Insert missing list item semicolon", ";]"))
        }
        _ => None,
    }
}
