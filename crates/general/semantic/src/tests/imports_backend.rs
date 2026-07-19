use super::*;

#[test]
fn genuinely_nonmatchable_witness_keeps_source_located_refusal() {
    let source = "m ::= import \"nonmatchable_witness.zt\";\nm\n";
    let analysis = analyze_in_imports(source);
    let diagnostics = analysis.native_import_diagnostics();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(
        &source[diagnostic.span.start as usize..diagnostic.span.end as usize],
        "import \"nonmatchable_witness.zt\""
    );
    assert_eq!(diagnostic.related.len(), 1);
    assert!(
        diagnostic.related[0]
            .path
            .ends_with("nonmatchable_witness.zt")
    );
    assert_eq!(
        diagnostic.related[0].label,
        "non-matchable witness exported here"
    );
}
#[test]
fn cross_module_conflicting_witnesses_report_import_error() {
    let analysis = analyze_in_imports(
        r#"
a ::= import "witness_eq_int_a.zt";
b ::= import "witness_eq_int_b.zt";
(a, b)
"#,
    );

    assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.kind,
        SemanticDiagnosticKind::Import(import)
            if matches!(
                &import.kind,
                ImportDiagnosticKind::ConflictingWitness { constraint, target }
                    if constraint == "Eq" && target == "Int"
            )
    )));
    assert!(analysis.blocking_diagnostics().next().is_some());
}

#[test]
fn imported_witness_conflicts_with_local_witness() {
    let analysis = analyze_in_imports(
        r#"
a ::= import "witness_eq_int_a.zt";
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
a
"#,
    );

    assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.kind,
        SemanticDiagnosticKind::Import(import)
            if matches!(
                &import.kind,
                ImportDiagnosticKind::ConflictingWitness { constraint, target }
                    if constraint == "Eq" && target == "Int"
            )
    )));
    assert!(analysis.blocking_diagnostics().next().is_some());
}

#[test]
fn cross_module_distinct_witness_targets_complete() {
    let analysis = analyze_in_imports(
        r#"
a ::= import "witness_eq_int_a.zt";
b ::= import "witness_eq_bool.zt";
(a, b)
"#,
    );

    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn cross_module_same_witness_reexport_is_deduped() {
    let analysis = analyze_in_imports(
        r#"
a ::= import "witness_reexport_a.zt";
b ::= import "witness_reexport_b.zt";
(a, b)
"#,
    );

    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn zt_import_cycle_is_reported() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/cycle_a.zt");
    let analysis = analyze_path(&path).expect("read cycle_a.zt");
    assert!(!analysis.is_thir_complete());
    assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.kind,
        SemanticDiagnosticKind::Import(import)
            if matches!(import.kind, ImportDiagnosticKind::ImportCycle { .. })
    )));
    let cycle = analysis
        .diagnostics
        .iter()
        .find_map(|diagnostic| match &diagnostic.kind {
            SemanticDiagnosticKind::Import(import)
                if matches!(import.kind, ImportDiagnosticKind::ImportCycle { .. }) =>
            {
                Some(import)
            }
            _ => None,
        })
        .expect("cycle diagnostic");
    assert!(!cycle.related.is_empty());
    assert!(cycle.related.iter().all(|location| {
        location
            .path
            .extension()
            .is_some_and(|extension| extension == "zt")
    }));
}

#[test]
fn bad_zti_reports_parse_error() {
    let analysis = analyze_in_imports("cfg ::= import \"bad.zti\";\ncfg");
    assert!(!analysis.is_thir_complete());
    assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.kind,
        SemanticDiagnosticKind::Import(import)
            if matches!(import.kind, ImportDiagnosticKind::ParseError { .. })
    )));
}

#[test]
fn empty_imported_list_still_completes() {
    // Empty `.zti` array → `List(InferVar)`; a free inference variable is
    // allowed in completed THIR.
    let analysis = analyze_in_imports("cfg ::= import \"empty_list.zti\";\ncfg");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
}

#[test]
fn mixed_imported_list_still_completes() {
    // Heterogeneous `.zti` array → `List(Union(...))`.
    let analysis = analyze_in_imports("cfg ::= import \"mixed_list.zti\";\ncfg");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
}

#[test]
fn import_absolute_path_is_rejected() {
    // Absolute paths are rejected before any filesystem access, so no fixture
    // file is needed; the guard fires in `relative_path`.
    let analysis = analyze_in_imports("cfg ::= import \"/etc/hosts.zti\";\ncfg");
    assert!(
        !analysis.is_thir_complete(),
        "expected incomplete THIR for absolute import"
    );
    assert!(
        has_import_diagnostic(
            &analysis,
            &ImportDiagnosticKind::PathTraversal {
                path: "/etc/hosts.zti".to_string()
            }
        ),
        "expected PathTraversal diagnostic, got: {:?}",
        analysis.diagnostics
    );
}

#[test]
fn import_parent_traversal_is_rejected() {
    // `../expr_core.zt` exists one directory above imports/, but resolving it
    // would escape the imports/ base directory and must be rejected.
    let analysis = analyze_in_imports("cfg ::= import \"../expr_core.zt\";\ncfg");
    assert!(
        !analysis.is_thir_complete(),
        "expected incomplete THIR for parent-traversal import"
    );
    assert!(
        has_import_diagnostic(
            &analysis,
            &ImportDiagnosticKind::PathTraversal {
                path: "../expr_core.zt".to_string()
            }
        ),
        "expected PathTraversal diagnostic, got: {:?}",
        analysis.diagnostics
    );
}

#[test]
fn import_dotdot_within_base_is_allowed() {
    // `../imports/config.zti` canonicalizes back to the imports/ directory
    // itself, so it stays within the base and must succeed.
    let analysis = analyze_in_imports("cfg ::= import \"../imports/config.zti\";\ncfg.port");
    assert!(
        analysis.is_thir_complete(),
        "expected complete THIR for in-base dotdot import, got: {:?}",
        analysis.diagnostics
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "expected no diagnostics, got: {:?}",
        analysis.diagnostics
    );
}

#[test]
fn tlc_is_some_for_complete_thir() {
    let analysis = analyze("x ::= 42;\nx");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.tlc.is_some(),
        "expected TLC module for complete program"
    );
}

#[test]
fn tlc_is_none_for_type_error() {
    let analysis = analyze("x :: Int = \"bad\";\nx");
    assert!(!analysis.is_thir_complete());
    assert!(
        analysis.tlc.is_none(),
        "expected no TLC module for type-error program"
    );
}

#[test]
fn effectful_program_predicate_detects_phase15_effects() {
    let analysis = analyze(
        r#"
Config :: type { value : Text; };
ParseError :: type Text;
parse :: Text -> Config ! { fail ParseError; }
  = text => perform fail text;
parse
"#,
    );
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(analysis.tlc.is_some(), "check path still builds TLC");
    assert!(analysis.effectful_program().is_some());
}

#[test]
fn effectful_program_predicate_ignores_pure_programs() {
    let analysis = analyze("x ::= 1;\nx");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert_eq!(analysis.effectful_program(), None);
}

#[test]
fn tlc_function_row_keeps_parametric_effect_alias() {
    let analysis = analyze(
        r#"
Config :: type { value : Text; };
Eff :: <A> type A ! { fail Text; };
parse :: Text -> Eff Config
  = text => perform fail text;
parse
"#,
    );
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    let tlc = analysis.tlc.expect("check path should build TLC");
    let has_fail_row = tlc.type_arena.iter().any(|(_, ty)| {
        matches!(ty, zutai_tlc::TlcType::Fun(_, _, zutai_tlc::Row::RExtend { label, .. })
                if label == "fail")
    });
    assert!(
        has_fail_row,
        "expected TLC function effect row to include fail"
    );
}

/// Walk to the tail of a TLC row, returning its row variable if open.
fn row_tail_var(row: &zutai_tlc::Row) -> Option<zutai_tlc::TlcTypeVar> {
    match row {
        zutai_tlc::Row::RVar(v) => Some(*v),
        zutai_tlc::Row::RExtend { tail, .. } => row_tail_var(tail),
        zutai_tlc::Row::REmpty => None,
    }
}

#[test]
fn tlc_emits_row_variable_for_named_row_tail() {
    let analysis = analyze(
        "idHost :: <Rest> { host : Text; ...Rest; } -> { host : Text; ...Rest; }\n  = x => x;\nidHost",
    );
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    let tlc = analysis
        .tlc
        .expect("expected TLC module for a complete row program");
    // The row-polymorphic function quantifies its `<Rest>` tail with Kind::Row.
    let has_row_forall = tlc
        .type_arena
        .iter()
        .any(|(_, t)| matches!(t, zutai_tlc::TlcType::ForAll(_, zutai_tlc::Kind::Row(_), _)));
    assert!(
        has_row_forall,
        "expected a ForAll quantifying a row variable with Kind::Row"
    );
    // A record row ends in a named row variable (`...Rest`).
    let has_named_rvar = tlc.type_arena.iter().any(|(_, t)| {
        matches!(t, zutai_tlc::TlcType::Record(row)
                if matches!(row_tail_var(row), Some(zutai_tlc::TlcTypeVar::Named(_))))
    });
    assert!(
        has_named_rvar,
        "expected a record row ending in RVar(Named)"
    );
}

#[test]
fn tlc_closed_record_has_no_free_row_variable() {
    let analysis = analyze("s :: { host : Text; port : Int; } = { host = \"h\"; port = 1; };\ns");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    let tlc = analysis.tlc.expect("expected TLC module");
    for (_, t) in tlc.type_arena.iter() {
        if let zutai_tlc::TlcType::Record(row) = t {
            assert!(
                row_tail_var(row).is_none(),
                "closed record must contain no row variable: {row:?}"
            );
        }
    }
}

#[test]
fn witness_reflection_is_aot_only_not_run_routing() {
    // `witness C @T` reflection must trigger the compile-time AOT-fold gate
    // but NOT the run-time THIR-routing gate: the TLC evaluator handles it,
    // and routing it to the THIR oracle would regress witness dispatch.
    let analysis = analyze(
        "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: derive\nwitness Eq @Int",
    );
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.aot_reflection_program().is_some(),
        "witness reflection must be detected for AOT folding"
    );
    assert!(
        analysis.reflection_builtin_program().is_none(),
        "witness reflection must NOT trigger THIR run routing"
    );
}

#[test]
fn variants_reflection_is_aot_only_not_run_routing() {
    // `variants` folds/evaluates on the TLC path, so it belongs to the AOT
    // gate but not the THIR-routing gate.
    let analysis = analyze("Color :: type { #red: {}; #green: {}; };\nvariants (Color)");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.aot_reflection_program().is_some(),
        "variants reflection must be detected for AOT folding"
    );
    assert!(
        analysis.reflection_builtin_program().is_none(),
        "variants reflection must NOT trigger THIR run routing"
    );
}

#[test]
fn fields_reflection_triggers_both_gates() {
    // `fields` does not fold during THIR→TLC elaboration, so it still needs
    // the THIR oracle and triggers both gates.
    let analysis = analyze("Server :: type { host : Text; };\nfields Server");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(analysis.reflection_builtin_program().is_some());
    assert!(analysis.aot_reflection_program().is_some());
}

#[test]
fn folded_schema_reflection_triggers_neither_gate() {
    // `schema` on a concrete type folds to a data literal during THIR→TLC
    // elaboration, so it no longer needs the THIR oracle or the AOT-fold
    // gate — unlike `fields` above, which does not fold.
    let analysis = analyze("Server :: type { host : Text; };\nschema Server");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(analysis.reflection_builtin_program().is_none());
    assert!(analysis.aot_reflection_program().is_none());
}

#[test]
fn implicit_witness_dispatch_is_not_reflection() {
    // Implicit method dispatch (`lt 1 2`) is not the `witness` reflection
    // expression and must lower natively — neither gate fires.
    let analysis = analyze(
        "Ord :: <A> @A { lt :: A -> A -> Bool; }\nOrd @Int :: { lt = \\a b. a < b; }\nlt 1 2",
    );
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(analysis.aot_reflection_program().is_none());
    assert!(analysis.reflection_builtin_program().is_none());
}
