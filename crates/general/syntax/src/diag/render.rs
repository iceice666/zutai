use ariadne::{Color, Label, Report, ReportKind, Source};

use super::{Diagnostic, Severity};

/// Print diagnostics to stderr using ariadne's rich format.
///
/// Uses `filename` as the display name and `src` as the source text.
pub fn eprint_diagnostics(diags: &[Diagnostic], filename: &str, src: &str) {
    for diag in diags {
        let start = u32::from(diag.range.start()) as usize;
        let end = u32::from(diag.range.end()) as usize;
        let end = end.max(start + 1);

        let kind = match diag.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };

        let mut builder = Report::build(kind, start..end)
            .with_message(format!("[{}] {}", diag.code.as_str(), &diag.message))
            .with_label(
                Label::new(start..end)
                    .with_message(&diag.message)
                    .with_color(Color::Red),
            );

        for label in &diag.labels {
            let ls = u32::from(label.range.start()) as usize;
            let le = u32::from(label.range.end()) as usize;
            builder = builder.with_label(
                Label::new(ls..le.max(ls + 1))
                    .with_message(&label.message)
                    .with_color(Color::Blue),
            );
        }

        // `filename` is used for display only; ariadne renders it in the header.
        let source = Source::from(src);
        let _ = builder
            .with_note(format!("--> {filename}"))
            .finish()
            .eprint(source);
    }
}
