use super::*;

fn diag_kinds(src: &str) -> Vec<HirDiagnosticKind> {
    lower(src).diagnostics.into_iter().map(|d| d.kind).collect()
}

fn has<F: Fn(&HirDiagnosticKind) -> bool>(src: &str, pred: F) -> bool {
    diag_kinds(src).iter().any(pred)
}

#[test]
fn level_binder_defines_level_param() {
    let file = lower_no_diag("Id :: <$l> $l -> $l\n  = x => x;\n1").file;
    let binding = find_binding_by_name(&file, "l").expect("`l` should be bound");
    assert_eq!(
        file.bindings[binding.0 as usize].kind,
        BindingKind::LevelParam
    );
}

#[test]
fn level_var_used_as_type_is_rejected() {
    // `<$l>` declares a level; using bare `l` in type position is an error.
    assert!(has("F :: <$l> l -> l\n  = x => x;\n1", |k| matches!(
        k,
        HirDiagnosticKind::LevelVarAsType { .. }
    )));
}

#[test]
fn non_level_used_as_level_is_rejected() {
    // `$A` where `A` is a type parameter, not a level variable.
    assert!(has("F :: <A> $A -> A\n  = x => x;\n1", |k| matches!(
        k,
        HirDiagnosticKind::NonLevelAsLevel { .. }
    )));
}

#[test]
fn unknown_level_var_is_rejected() {
    // `$m` is never declared.
    assert!(has("F :: <$l> $m -> $l\n  = x => x;\n1", |k| matches!(
        k,
        HirDiagnosticKind::UnknownLevelVar { .. }
    )));
}

#[test]
fn unused_level_param_is_reported() {
    assert!(has("F :: <$l> Int -> Int\n  = x => x;\n1", |k| matches!(
        k,
        HirDiagnosticKind::UnusedLevelParam { .. }
    )));
}

#[test]
fn used_level_param_is_not_reported_unused() {
    assert!(!has("F :: <$l> $l -> $l\n  = x => x;\n1", |k| matches!(
        k,
        HirDiagnosticKind::UnusedLevelParam { .. }
    )));
}
