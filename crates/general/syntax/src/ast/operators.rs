use crate::SyntaxKind;

/// The operator of a `BINARY_EXPR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    // Comparison (non-associative)
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // Logical
    And,
    Or,
    // Defaulting / coalescing
    Coalesce,
}

impl BinaryOp {
    pub fn from_kind(kind: SyntaxKind) -> Option<Self> {
        Some(match kind {
            SyntaxKind::PLUS => Self::Add,
            SyntaxKind::MINUS => Self::Sub,
            SyntaxKind::STAR => Self::Mul,
            SyntaxKind::SLASH => Self::Div,
            SyntaxKind::EQ_EQ => Self::Eq,
            SyntaxKind::BANG_EQ => Self::Ne,
            SyntaxKind::LT => Self::Lt,
            SyntaxKind::LT_EQ => Self::Le,
            SyntaxKind::GT => Self::Gt,
            SyntaxKind::GT_EQ => Self::Ge,
            SyntaxKind::AMP_AMP => Self::And,
            SyntaxKind::PIPE_PIPE => Self::Or,
            SyntaxKind::QUESTION_QUESTION => Self::Coalesce,
            _ => return None,
        })
    }
}

/// The direction of a `PIPELINE_EXPR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineDir {
    Forward,  // |>
    Backward, // <|
}
