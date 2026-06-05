use std::fmt;
use std::path::PathBuf;

use zutai_syntax::diag::Diagnostic;

/// Convenience result type for evaluator operations.
pub type EvalResult<T> = Result<T, EvalError>;

/// Error returned by the general-mode evaluator.
#[derive(Debug)]
pub struct EvalError {
    pub kind: EvalErrorKind,
}

impl EvalError {
    pub fn new(kind: EvalErrorKind) -> Self {
        Self { kind }
    }

    pub fn not_implemented(feature: impl Into<String>) -> Self {
        Self::new(EvalErrorKind::NotImplemented {
            feature: feature.into(),
        })
    }
}

/// Coarse evaluator error categories.
///
/// Parser and semantic diagnostics stay in Zutai's normal diagnostic type so
/// callers can render them with the existing diagnostic renderer.
#[derive(Debug)]
pub enum EvalErrorKind {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Diagnostics {
        diagnostics: Vec<Diagnostic>,
    },
    NotImplemented {
        feature: String,
    },
    Runtime {
        message: String,
    },
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            EvalErrorKind::Io { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            EvalErrorKind::Diagnostics { diagnostics } => {
                write!(
                    f,
                    "{} diagnostic(s) prevented evaluation",
                    diagnostics.len()
                )
            }
            EvalErrorKind::NotImplemented { feature } => {
                write!(f, "{feature} is not implemented yet")
            }
            EvalErrorKind::Runtime { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for EvalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            EvalErrorKind::Io { source, .. } => Some(source),
            EvalErrorKind::Diagnostics { .. }
            | EvalErrorKind::NotImplemented { .. }
            | EvalErrorKind::Runtime { .. } => None,
        }
    }
}
