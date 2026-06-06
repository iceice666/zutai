use crate::span::Span;

#[derive(Debug, Clone)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
    pub expected: Vec<&'static str>,
}

impl ParseError {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            expected: vec![],
        }
    }

    pub fn with_expected(mut self, expected: Vec<&'static str>) -> Self {
        self.expected = expected;
        self
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}..{}] {}",
            self.span.start, self.span.end, self.message
        )?;
        if !self.expected.is_empty() {
            write!(f, " (expected: {})", self.expected.join(", "))?;
        }
        Ok(())
    }
}
