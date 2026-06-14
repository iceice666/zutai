use std::collections::HashMap;

use crate::diagnostic::{HirDiagnostic, HirDiagnosticKind};
use crate::ir::{
    HirExprKind, HirFile, HirPatKind, HirRecordField, HirRecordPatField, HirTupleItem,
    HirTuplePatItem, HirTypeKind, HirTypeRecordField, HirTypeTupleItem,
};
use crate::pass::HirPass;

#[derive(Debug, Default)]
pub struct StructuralKeyValidationPass;

impl HirPass for StructuralKeyValidationPass {
    fn name(&self) -> &'static str {
        "structural_key_validation"
    }

    fn run(&mut self, file: &mut HirFile, diagnostics: &mut Vec<HirDiagnostic>) {
        for (_, expr) in file.expr_arena.iter() {
            match &expr.kind {
                HirExprKind::Record(fields) => {
                    validate_record_fields(fields, diagnostics);
                }
                HirExprKind::Tuple(items) => {
                    validate_tuple_items(items, diagnostics);
                }
                _ => {}
            }
        }

        for (_, pat) in file.pat_arena.iter() {
            match &pat.kind {
                HirPatKind::Record(fields) => {
                    validate_record_pattern_fields(fields, diagnostics);
                }
                HirPatKind::Tuple(items) => {
                    validate_tuple_pattern_items(items, diagnostics);
                }
                _ => {}
            }
        }

        for (_, ty) in file.type_arena.iter() {
            match &ty.kind {
                HirTypeKind::Record(fields) => {
                    validate_type_record_fields(fields, diagnostics);
                }
                HirTypeKind::Tuple(items) => {
                    validate_type_tuple_items(items, diagnostics);
                }
                _ => {}
            }
        }
    }
}

fn validate_record_fields(fields: &[HirRecordField], diagnostics: &mut Vec<HirDiagnostic>) {
    let mut seen = HashMap::new();
    for field in fields {
        if let Some(first_span) = seen.get(&field.name).copied() {
            diagnostics.push(HirDiagnostic {
                kind: HirDiagnosticKind::DuplicateRecordField {
                    name: field.name.clone(),
                    first_span,
                },
                span: field.span,
            });
        } else {
            seen.insert(field.name.clone(), field.span);
        }
    }
}

fn validate_type_record_fields(
    fields: &[HirTypeRecordField],
    diagnostics: &mut Vec<HirDiagnostic>,
) {
    let mut seen = HashMap::new();
    for field in fields {
        if let Some(first_span) = seen.get(&field.name).copied() {
            diagnostics.push(HirDiagnostic {
                kind: HirDiagnosticKind::DuplicateTypeRecordField {
                    name: field.name.clone(),
                    first_span,
                },
                span: field.span,
            });
        } else {
            seen.insert(field.name.clone(), field.span);
        }
    }
}

fn validate_record_pattern_fields(
    fields: &[HirRecordPatField],
    diagnostics: &mut Vec<HirDiagnostic>,
) {
    let mut seen = HashMap::new();
    for field in fields {
        if let Some(first_span) = seen.get(&field.name).copied() {
            diagnostics.push(HirDiagnostic {
                kind: HirDiagnosticKind::DuplicateRecordPatternField {
                    name: field.name.clone(),
                    first_span,
                },
                span: field.span,
            });
        } else {
            seen.insert(field.name.clone(), field.span);
        }
    }
}

fn validate_tuple_items(items: &[HirTupleItem], diagnostics: &mut Vec<HirDiagnostic>) {
    let mut seen = HashMap::new();
    for item in items {
        let HirTupleItem::Named { name, span, .. } = item else {
            continue;
        };
        if let Some(first_span) = seen.get(name).copied() {
            diagnostics.push(HirDiagnostic {
                kind: HirDiagnosticKind::DuplicateTupleField {
                    name: name.clone(),
                    first_span,
                },
                span: *span,
            });
        } else {
            seen.insert(name.clone(), *span);
        }
    }
}

fn validate_type_tuple_items(items: &[HirTypeTupleItem], diagnostics: &mut Vec<HirDiagnostic>) {
    let mut seen = HashMap::new();
    for item in items {
        let HirTypeTupleItem::Named { name, span, .. } = item else {
            continue;
        };
        if let Some(first_span) = seen.get(name).copied() {
            diagnostics.push(HirDiagnostic {
                kind: HirDiagnosticKind::DuplicateTypeTupleField {
                    name: name.clone(),
                    first_span,
                },
                span: *span,
            });
        } else {
            seen.insert(name.clone(), *span);
        }
    }
}

fn validate_tuple_pattern_items(items: &[HirTuplePatItem], diagnostics: &mut Vec<HirDiagnostic>) {
    let mut seen = HashMap::new();
    for item in items {
        let HirTuplePatItem::Named { name, span, .. } = item else {
            continue;
        };
        if let Some(first_span) = seen.get(name).copied() {
            diagnostics.push(HirDiagnostic {
                kind: HirDiagnosticKind::DuplicateTuplePatternField {
                    name: name.clone(),
                    first_span,
                },
                span: *span,
            });
        } else {
            seen.insert(name.clone(), *span);
        }
    }
}
