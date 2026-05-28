use rustc_hash::{FxHashMap, FxHashSet};
use text_size::TextRange;

use crate::{
    SyntaxKind, SyntaxNode,
    diag::{Diagnostic, ErrorCode, Severity},
};

/// Post-parse validation pass: walk the tree and emit lint diagnostics.
///
/// Current lints:
/// - **W0001** capitalization: a type-definition binding whose name starts
///   with a lowercase letter (warning).
/// - **E0050** reserved-name: `forall` or `select` used as a binding name.
/// - **E0010** duplicate-binding: two top-level declarations share a name.
/// - **E0011** duplicate-field: two fields in the same record have the same name.
pub fn validate(root: &SyntaxNode, diags: &mut Vec<Diagnostic>) {
    let mut seen_names: FxHashMap<String, TextRange> = FxHashMap::default();

    for child in root.children() {
        match child.kind() {
            SyntaxKind::INFERRED_BINDING | SyntaxKind::ANNOTATED_BINDING => {
                if let Some(tok) = ident_token(&child) {
                    let name = tok.text().to_owned();
                    let range = tok.text_range();
                    check_reserved(&name, range, diags);
                    check_duplicate(&name, range, &mut seen_names, diags);
                }
            }
            SyntaxKind::FUNC_DECL => {
                if let Some(tok) = ident_token(&child) {
                    let name = tok.text().to_owned();
                    let range = tok.text_range();
                    check_reserved(&name, range, diags);
                    check_duplicate(&name, range, &mut seen_names, diags);
                    check_type_def_capitalization(&child, &name, range, diags);
                }
            }
            _ => {}
        }
    }

    // Duplicate-field check inside every record expression in the file.
    validate_record_fields(root, diags);
}

fn ident_token(node: &SyntaxNode) -> Option<crate::SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == SyntaxKind::IDENT)
}

/// Reserved identifiers that are not lexer keywords but are forbidden as names.
const RESERVED_NAMES: &[&str] = &["forall", "select"];

fn check_reserved(name: &str, range: TextRange, diags: &mut Vec<Diagnostic>) {
    if RESERVED_NAMES.contains(&name) {
        diags.push(Diagnostic {
            range,
            severity: Severity::Error,
            code: ErrorCode::ReservedName,
            message: format!(
                "`{name}` is a reserved identifier and cannot be used as a binding name"
            ),
            labels: vec![],
        });
    }
}

fn check_duplicate(
    name: &str,
    range: TextRange,
    seen: &mut FxHashMap<String, TextRange>,
    diags: &mut Vec<Diagnostic>,
) {
    if let Some(&prior) = seen.get(name) {
        diags.push(
            Diagnostic::error(
                range,
                ErrorCode::DuplicateBinding,
                format!("duplicate binding `{name}`"),
            )
            .with_label(prior, "first binding here"),
        );
    } else {
        seen.insert(name.to_owned(), range);
    }
}

/// Warn if a `FUNC_DECL` whose first non-whitespace body child is a `TYPE_FORM`
/// (i.e., a type definition) has a name that starts with a lowercase letter.
fn check_type_def_capitalization(
    decl: &SyntaxNode,
    name: &str,
    range: TextRange,
    diags: &mut Vec<Diagnostic>,
) {
    let is_type_def = decl.children().any(|c| c.kind() == SyntaxKind::TYPE_FORM);
    if is_type_def {
        if let Some(first_char) = name.chars().next() {
            if first_char.is_ascii_lowercase() {
                diags.push(Diagnostic {
                    range,
                    severity: Severity::Warning,
                    code: ErrorCode::CapitalizationConvention,
                    message: format!(
                        "type definition `{name}` should start with an uppercase letter"
                    ),
                    labels: vec![],
                });
            }
        }
    }
}

/// Walk all `RECORD_EXPR` nodes in the subtree and flag duplicate field names.
fn validate_record_fields(root: &SyntaxNode, diags: &mut Vec<Diagnostic>) {
    for node in root.descendants() {
        if node.kind() == SyntaxKind::RECORD_EXPR {
            check_record_field_duplicates(&node, diags);
        }
    }
}

fn check_record_field_duplicates(record: &SyntaxNode, diags: &mut Vec<Diagnostic>) {
    let mut seen: FxHashSet<String> = FxHashSet::default();
    for child in record.children() {
        if child.kind() == SyntaxKind::VALUE_FIELD {
            if let Some(field_name_node) = child
                .children()
                .find(|c| c.kind() == SyntaxKind::FIELD_NAME)
            {
                let name = field_name_node.text().to_string();
                let range = field_name_node.text_range();
                if !seen.insert(name.clone()) {
                    diags.push(Diagnostic::error(
                        range,
                        ErrorCode::DuplicateKey,
                        format!("duplicate field `{name}` in record"),
                    ));
                }
            }
        }
    }
}
