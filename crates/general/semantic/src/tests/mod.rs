use super::*;

#[test]
fn parse_error_stops_before_hir() {
    let analysis = analyze("{1; 2}");

    assert!(analysis.has_parse_errors());
    assert!(analysis.hir.is_none());
    assert!(analysis.thir.is_none());
}

#[test]
fn hir_error_stops_before_thir() {
    let analysis = analyze("missing");

    assert!(!analysis.has_parse_errors());
    assert!(analysis.has_hir_errors());
    assert!(analysis.hir.is_some());
    assert!(analysis.thir.is_none());
}

#[test]
fn structural_key_hir_error_stops_before_thir() {
    let analysis = analyze("T :: type { a : Int; a : Text; };\nT");

    assert!(!analysis.has_parse_errors());
    assert!(analysis.has_hir_errors());
    assert!(analysis.hir.is_some());
    assert!(analysis.thir.is_none());
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.kind,
            SemanticDiagnosticKind::Hir(zutai_hir::HirDiagnostic {
                kind: zutai_hir::HirDiagnosticKind::DuplicateTypeRecordField { .. },
                ..
            })
        )
    }));
}

#[test]
fn valid_parse_and_hir_reaches_thir_stage() {
    let analysis = analyze("x ::= 1;\nx");

    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors());
    assert!(analysis.ast.is_some());
    assert!(analysis.hir.is_some());
    assert!(analysis.thir.is_some());
    assert!(analysis.is_thir_complete());
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn thir_type_error_is_reported_by_semantic_analysis() {
    let analysis = analyze("x :: Int = \"bad\";\nx");

    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors());
    assert!(analysis.hir.is_some());
    assert!(analysis.thir.is_some());
    assert!(!analysis.is_thir_complete());
    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                kind: zutai_thir::ThirDiagnosticKind::TypeMismatch { expected, found },
                ..
            }) if expected == "Int" && found == "Text"
        )
    }));
}

/// Whether `analysis` carries a `WitnessReflectNotInScope` diagnostic whose
/// constraint (and, when given, target) match. Mirrors the removed THIR
/// bare-dispatch gate, now enforced during TLC lowering against the merged
/// witness registry.
fn has_witness_not_in_scope(analysis: &Analysis, constraint: &str, target: Option<&str>) -> bool {
    analysis.diagnostics.iter().any(|diagnostic| {
            matches!(
                &diagnostic.kind,
                SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                    kind: zutai_thir::ThirDiagnosticKind::WitnessReflectNotInScope { constraint: c, target: t },
                    ..
                }) if c == constraint && target.is_none_or(|want| t == want)
            )
        })
}

/// A bare constraint-method dispatch to a concrete target with no witness in
/// scope is refused during analysis (relocated from the THIR gate): THIR is
/// complete, but the S1 witness-existence gate raises
/// `WitnessReflectNotInScope` so `check`/`compile`/eval refuse the call.
#[test]
fn bare_method_call_missing_witness_rejected_by_analysis() {
    let analysis = analyze("Show :: <A> @A { show :: A -> Text; }\nshow 42");

    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors());
    assert!(analysis.is_thir_complete());
    assert!(
        has_witness_not_in_scope(&analysis, "Show", Some("Int")),
        "expected WitnessReflectNotInScope for Show @Int; diagnostics: {:?}",
        analysis.diagnostics
    );
}

/// A bare dispatch on a structural union value (`show (#err)`) whose only
/// witness targets the nominal union does not match the operand key, so the
/// gate refuses it — the structural-target rejection the THIR gate covered.
#[test]
fn bare_method_call_structural_target_without_witness_rejected() {
    let analysis = analyze(
        "Status :: type { #ok; #err; };\nShow :: <A> @A { show :: A -> Text; }\nShow @Status :: { show = \\s. \"shown\"; }\nshow (#err)",
    );

    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors());
    assert!(
        has_witness_not_in_scope(&analysis, "Show", None),
        "expected WitnessReflectNotInScope for `Show`; diagnostics: {:?}",
        analysis.diagnostics
    );
}

/// A concrete named target with a witness in scope stays valid: the gate
/// resolves it against the registry and emits nothing.
#[test]
fn bare_method_call_with_witness_is_accepted() {
    let analysis = analyze(
        "Show :: <A> @A { show :: A -> Text; }\nShow @Text :: { show = \\s. s; }\nshow \"x\"",
    );

    assert!(analysis.is_thir_complete());
    assert!(
        !has_witness_not_in_scope(&analysis, "Show", None),
        "unexpected witness-not-in-scope; diagnostics: {:?}",
        analysis.diagnostics
    );
}

/// Q4 regression: a bare record-literal argument to a derived/constraint
/// method is inferred structurally and unified into the parameter's inference
/// variable, so the witness gate names a concrete record target
/// (`Show @{ x : Int }`) instead of leaking the raw `?N` metavariable that
/// the old `ExpectedRecord` refusal produced. The call still has no witness,
/// so it is refused — but with an actionable target name.
#[test]
fn q4_record_literal_dispatch_names_concrete_target_not_metavar() {
    let analysis = analyze(
        "Show :: <A> @A { show :: A -> Text; }\nShow @Int :: { show = \\n. \"N\"; }\nshow { x = 1; }",
    );

    assert!(analysis.is_thir_complete());
    let target = analysis
        .diagnostics
        .iter()
        .find_map(|diagnostic| match &diagnostic.kind {
            SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                kind:
                    zutai_thir::ThirDiagnosticKind::WitnessReflectNotInScope { constraint, target },
                ..
            }) if constraint == "Show" => Some(target.clone()),
            _ => None,
        });
    let target = target.expect("expected a Show witness-not-in-scope diagnostic");
    assert!(
        target.contains("x") && !target.contains('?'),
        "expected a concrete record target with no leaked metavariable, got {target:?}"
    );
}

// ── `.zti` imports ────────────────────────────────────────────────────────

fn imports_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports")
}

fn analyze_in_imports(input: &str) -> Analysis {
    analyze_with_base(input, Some(&imports_dir()), AnalysisOptions::default())
}

fn has_import_diagnostic(analysis: &Analysis, expected: &ImportDiagnosticKind) -> bool {
    analysis.diagnostics.iter().any(|diagnostic| {
        matches!(
            &diagnostic.kind,
            SemanticDiagnosticKind::Import(import) if &import.kind == expected
        )
    })
}

#[test]
fn import_with_base_completes() {
    let analysis = analyze_in_imports("cfg ::= import \"config.zti\";\ncfg.port");
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

// ── filesystem stdlib + destructuring imports ──────────────────────────────

#[test]
fn stdlib_stream_import_resolves_without_base() {
    // Dotted stdlib imports need no user-module base directory.
    let analysis =
        analyze("s ::= import stdlib.stream;\ns.fold (\\acc x. acc + x) 0 (s.singleton 5)");
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_stream_exports_encode_without_exporting_constraint_values() {
    let analysis =
        analyze("s ::= import stdlib.stream;\nencoded :: s.Data = s.encode {1; 2; 3;};\nencoded");
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn unknown_stdlib_module_is_diagnosed() {
    let analysis = analyze("s ::= import stdlib.nope;\ns");
    assert!(has_import_diagnostic(
        &analysis,
        &ImportDiagnosticKind::UnknownStdlibModule {
            name: "nope".to_string(),
        },
    ));
}

#[test]
fn destructured_stdlib_members_bind_unqualified() {
    let analysis = analyze(
        "s ::= import stdlib.stream;\n{ map; fold; singleton; } ::= s;\nfold (\\a x. a + x) 0 (map (\\n. n + 1) (singleton 4))",
    );
    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn destructuring_an_unknown_field_is_diagnosed() {
    let analysis = analyze("s ::= import stdlib.stream;\n{ nope; } ::= s;\nnope");
    assert!(!analysis.is_thir_complete());
}

// ── ambient function prelude (stdlib slice B) ──────────────────────────────

#[test]
fn function_prelude_ambient_resolves_without_import() {
    // `id`/`const`/`compose`/`flip` are ambient (no import); a higher-order
    // use type-checks end-to-end through the semantic facade.
    let analysis = analyze("compose (\\x. x + 1) (\\x. x * 2) 3");
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_prelude_import_resolves_without_base() {
    let analysis = analyze("p ::= import stdlib.prelude;\np.compose (\\x. x + 1) (\\x. x * 2) 3");
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_optional_import_resolves_without_base() {
    let analysis =
        analyze("o ::= import stdlib.optional;\no.withDefault 0 (o.map (\\x. x + 1) (#some (41)))");
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_result_import_resolves_without_base() {
    let analysis = analyze(
        "r ::= import stdlib.result;\n\
             res :: r.Result Text Int = r.ok 41;\n\
             r.withDefault 0 (r.map (\\x. x + 1) res)",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn destructured_stdlib_result_members_bind_unqualified() {
    let analysis = analyze(
        "{ ok; err; map; withDefault; } ::= import stdlib.result;\n\
             withDefault 0 (map (\\n. n + 1) (if true then ok 4 else err \"x\"))",
    );
    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_num_import_resolves_without_base() {
    let analysis = analyze(
        "n ::= import stdlib.num;\n\
             n.clamp 0 10 (n.max (n.min 99 7) (n.abs (0 - 3)))",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn destructured_stdlib_num_members_bind_unqualified() {
    let analysis = analyze(
        "{ gcd; rem; pow; toFloat; round; truncate; } ::= import stdlib.num;\n\
             gcd 54 24 + rem 17 5 + pow 2 3 + round (toFloat 4) + truncate 3.9",
    );
    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_text_import_resolves_without_base() {
    let analysis = analyze(
        "t ::= import stdlib.text;\n\
             t.length (t.replace \"a\" \"o\" (t.trim \" cat \")) + t.length (t.show \"x\")",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_cmp_import_resolves_without_base() {
    let analysis = analyze(
        "c ::= import stdlib.cmp;\n\
             c.then (c.compareInt 1 2) (c.reverse c.gt)",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_list_import_resolves_without_base() {
    let analysis = analyze(
        "l ::= import stdlib.list;\n\
             c ::= import stdlib.cmp;\n\
             l.sum (l.take 3 (l.sortBy c.compareInt {3; 1; 2; 4;})) + l.product {2; 3;}",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_data_import_resolves_without_base() {
    let analysis = analyze(
        "d ::= import stdlib.data;\n\
             value ::= d.record { d.fieldOf \"port\" (d.int 8080); };\n\
             match d.field \"port\" value { | #ok { value = found; } => d.asInt found; | #err { error = error; } => #err { error = error; }; }",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_validate_import_resolves_without_base() {
    let analysis = analyze(
        "v ::= import stdlib.validate;\n\
             length (v.errors (v.map3 (\\a b c. a + b + c) (v.valid 1) (v.intRange \"x\" 0 10 20) (v.required \"name\" (#none))))",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_net_import_resolves_without_base() {
    let analysis = analyze(
        "net ::= import stdlib.net;\n\
             main :: Net -> net.Server Text\n\
               = cap => [\n\
                 listener := net.listen cap 7777;\n\
                 net.withConnection cap listener (\\conn. [\n\
                   line := net.read cap conn;\n\
                   net.write cap line;\n\
                   line\n\
                 ])\n\
               ];\n\
             main",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn browser_stdlib_modules_resolve_together() {
    let analysis = analyze(
        "css ::= import stdlib.css;\n\
             html ::= import stdlib.html;\n\
             browser ::= import stdlib.browser;\n\
             (css, html, browser)",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_config_import_resolves_and_overlay_alias_typechecks() {
    let analysis = analyze(
        "cfg ::= import stdlib.config;\n\
             Server :: type { host : Text; port : Int; };\n\
             base :: Server = { host = \"127.0.0.1\"; port = 8080; };\n\
             (cfg.overlay { port = 9090; } base).port",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn destructured_stdlib_config_overlay_shadows_builtin_and_typechecks() {
    let analysis = analyze(
        "{ overlay; } ::= import stdlib.config;\n\
             Server :: type { host : Text; port : Int; };\n\
             base :: Server = { host = \"127.0.0.1\"; port = 8080; };\n\
             (overlay { port = 9090; } base).port",
    );
    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn stdlib_reflect_import_resolves_without_base() {
    let analysis = analyze(
        "refl ::= import stdlib.reflect;\n\
             Server :: type { host : Text; port : Int; };\n\
             length ((refl.schema Server).fields ?? {;})",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis.reflection_builtin_program().is_some());
    assert!(analysis.aot_reflection_program().is_some());
}

#[test]
fn destructured_stdlib_reflect_alias_use_triggers_reflection_gate() {
    let analysis = analyze(
        "{ schema; } ::= import stdlib.reflect;\n\
             Server :: type { host : Text; port : Int; };\n\
             length ((schema Server).fields ?? {;})",
    );
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(analysis.reflection_builtin_program().is_some());
    assert!(analysis.aot_reflection_program().is_some());
}

#[test]
fn unused_destructured_stdlib_reflect_import_does_not_trip_backend_gate() {
    let analysis = analyze("{ schema; } ::= import stdlib.reflect;\n1");
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(analysis.reflection_builtin_program().is_none());
    assert!(analysis.aot_reflection_program().is_none());
}

#[test]
fn unused_stdlib_config_and_reflect_imports_do_not_trip_backend_gates() {
    let analysis = analyze("cfg ::= import stdlib.config;\nrefl ::= import stdlib.reflect;\n1");
    assert!(!analysis.has_parse_errors());
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(analysis.config_overlay_builtin_program().is_none());
    assert!(analysis.reflection_builtin_program().is_none());
    assert!(analysis.aot_reflection_program().is_none());
}

#[test]
fn destructured_stdlib_optional_members_bind_unqualified() {
    let analysis = analyze(
        "{ map; withDefault; } ::= import stdlib.optional;\nwithDefault 0 (map (\\n. n + 1) (#some (4)))",
    );
    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn user_binding_shadows_prelude_without_duplicate_diagnostic() {
    // The prelude is a fallback: a user `id` of the same name wins and raises
    // no duplicate-binding diagnostic.
    let analysis = analyze("id :: Int -> Int = x => x + 1;\nid 5");
    assert!(!analysis.has_parse_errors());
    assert!(!analysis.has_hir_errors(), "{:?}", analysis.diagnostics);
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
}

#[test]
fn analyze_path_resolves_relative_import() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/importer.zt");
    let analysis = analyze_path(&path).expect("read importer.zt");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn analyze_path_resolves_relative_zt_import() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/zt_importer.zt");
    let analysis = analyze_path(&path).expect("read zt_importer.zt");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn analyze_sources_resolves_transitive_zt_and_zti_imports() {
    let sources = BTreeMap::from([
        (
            "main.zt".to_string(),
            "lib ::= import \"modules/site.zt\";\nlib.port".to_string(),
        ),
        (
            "modules/site.zt".to_string(),
            "cfg ::= import \"../config.zti\";\n{ port = cfg.port; }".to_string(),
        ),
        ("config.zti".to_string(), "{ port = 8787; }".to_string()),
    ]);

    let analysis = analyze_sources("main.zt", &sources, AnalysisOptions::default()).unwrap();
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.import_modules.len(), 1);
}

#[test]
fn imported_data_type_mismatch_retains_zti_value_span() {
    let a = "{\n  host = \"localhost\";\n  port = \"wrong\";\n}\n";
    let sources = BTreeMap::from([
        (
            "C.zt".to_string(),
            "b ::= import \"B.zt\";\na ::= import \"A.zti\";\nchecked :: b.Config = a;\nchecked\n"
                .to_string(),
        ),
        (
            "B.zt".to_string(),
            "Config :: type { host : Text; port : Int; };\n{ Config = Config; }\n".to_string(),
        ),
        ("A.zti".to_string(), a.to_string()),
    ]);

    let analysis = analyze_sources("C.zt", &sources, AnalysisOptions::default()).unwrap();
    let origin = analysis
        .diagnostics
        .iter()
        .find_map(|diagnostic| match &diagnostic.kind {
            SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                kind:
                    zutai_thir::ThirDiagnosticKind::ImportedDataTypeMismatch {
                        expected,
                        found,
                        origin,
                    },
                ..
            }) if expected == "Int" && found == "Text" => Some(origin),
            _ => None,
        })
        .expect("expected imported-data mismatch");
    assert_eq!(
        &a[origin.span.start as usize..origin.span.end as usize],
        "\"wrong\""
    );
    assert!(matches!(
        &origin.source,
        zutai_hir::HirImportSource::String(path) if path == "A.zti"
    ));
}

#[test]
fn heterogeneous_imported_list_checks_each_item_against_context() {
    let a = "{ ports = [1; \"wrong\";]; }";
    let sources = BTreeMap::from([
        (
            "C.zt".to_string(),
            "b ::= import \"B.zt\";\na ::= import \"A.zti\";\nchecked :: b.Config = a;\nchecked\n"
                .to_string(),
        ),
        (
            "B.zt".to_string(),
            "Config :: type { ports : List Int; };\n{ Config = Config; }\n".to_string(),
        ),
        ("A.zti".to_string(), a.to_string()),
    ]);

    let analysis = analyze_sources("C.zt", &sources, AnalysisOptions::default()).unwrap();
    let origin = analysis
        .diagnostics
        .iter()
        .find_map(|diagnostic| match &diagnostic.kind {
            SemanticDiagnosticKind::Thir(zutai_thir::ThirDiagnostic {
                kind: zutai_thir::ThirDiagnosticKind::ImportedDataTypeMismatch { origin, .. },
                ..
            }) => Some(origin),
            _ => None,
        })
        .expect("expected heterogeneous-list mismatch");
    assert_eq!(
        &a[origin.span.start as usize..origin.span.end as usize],
        "\"wrong\""
    );
}

#[test]
fn analyze_sources_rejects_non_normalized_or_missing_entries() {
    let sources = BTreeMap::from([("main.zt".to_string(), "1".to_string())]);
    assert!(matches!(
        analyze_sources("../main.zt", &sources, AnalysisOptions::default()),
        Err(SourceMapError::InvalidPath { .. })
    ));
    assert!(matches!(
        analyze_sources("C:/main.zt", &sources, AnalysisOptions::default()),
        Err(SourceMapError::InvalidPath { .. })
    ));
    assert!(matches!(
        analyze_sources("missing.zt", &sources, AnalysisOptions::default()),
        Err(SourceMapError::MissingEntry { .. })
    ));

    let invalid = BTreeMap::from([("dir\\main.zt".to_string(), "1".to_string())]);
    assert!(matches!(
        analyze_sources("dir/main.zt", &invalid, AnalysisOptions::default()),
        Err(SourceMapError::InvalidPath { .. })
    ));
}

#[test]
fn analyze_sources_confines_imports_to_virtual_root() {
    let sources = BTreeMap::from([
        (
            "main.zt".to_string(),
            "secret ::= import \"../secret.zti\";\nsecret".to_string(),
        ),
        ("secret.zti".to_string(), "value = 1".to_string()),
    ]);
    let analysis = analyze_sources("main.zt", &sources, AnalysisOptions::default()).unwrap();
    assert!(has_import_diagnostic(
        &analysis,
        &ImportDiagnosticKind::PathTraversal {
            path: "../secret.zti".to_string(),
        },
    ));
}

#[test]
fn analyze_path_recording_captures_transitive_sources() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/chain_top.zt");
    let recorded = analyze_path_recording(&path).expect("record source graph");
    assert_eq!(recorded.entry, "chain_top.zt");
    assert_eq!(
        recorded.sources.keys().cloned().collect::<Vec<_>>(),
        vec![
            "chain_mid.zt".to_string(),
            "chain_top.zt".to_string(),
            "config.zti".to_string(),
        ]
    );
    assert_eq!(
        recorded.stdlib_sources.keys().cloned().collect::<Vec<_>>(),
        vec!["prelude".to_string(), "stream".to_string()]
    );
    assert_eq!(
        recorded.stdlib_compiler_compatibility,
        STDLIB_COMPILER_COMPATIBILITY
    );
    assert!(recorded.analysis.is_thir_complete());
}

#[test]
fn recording_bundles_only_used_stdlib_modules_plus_ambient_preludes() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-semantic-stdlib-recording-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join("main.zt");
    std::fs::write(&entry, "n ::= import stdlib.num;\nn.abs -1\n").unwrap();

    let recorded = analyze_path_recording(&entry).expect("record stdlib source graph");
    assert_eq!(
        recorded.stdlib_sources.keys().cloned().collect::<Vec<_>>(),
        vec![
            "num".to_string(),
            "prelude".to_string(),
            "stream".to_string(),
        ]
    );
    assert!(recorded.analysis.is_thir_complete());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_recording_root_allows_sibling_imports_inside_root() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-semantic-source-root-{}-{unique}",
        std::process::id()
    ));
    let app = root.join("app");
    let shared = root.join("shared");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&shared).unwrap();
    let entry = app.join("main.zt");
    std::fs::write(&entry, "cfg ::= import \"../shared/config.zti\";\ncfg.port").unwrap();
    std::fs::write(shared.join("config.zti"), "{ port = 8787; }").unwrap();

    let recorded = analyze_path_recording_with_root(&entry, &root).unwrap();
    assert_eq!(recorded.entry, "app/main.zt");
    assert_eq!(
        recorded.sources.keys().cloned().collect::<Vec<_>>(),
        vec!["app/main.zt".to_string(), "shared/config.zti".to_string()]
    );
    assert!(
        recorded.analysis.is_thir_complete(),
        "{:?}",
        recorded.analysis.diagnostics
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analysis_cache_reuses_unchanged_import_graph() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-semantic-cache-reuse-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join("main.zt");
    std::fs::write(&entry, "mid ::= import \"mid.zt\";\nmid\n").unwrap();
    std::fs::write(root.join("mid.zt"), "leaf ::= import \"leaf.zt\";\nleaf\n").unwrap();
    std::fs::write(root.join("leaf.zt"), "42\n").unwrap();

    let cache = AnalysisCache::default();
    let first = analyze_path_with_cache(&entry, &cache).unwrap();
    assert!(first.is_thir_complete(), "{:?}", first.diagnostics);
    assert_eq!(
        cache.stats(),
        AnalysisCacheStats {
            module_hits: 0,
            module_misses: 2,
        }
    );

    let second = analyze_path_with_cache(&entry, &cache).unwrap();
    assert!(second.is_thir_complete(), "{:?}", second.diagnostics);
    assert_eq!(
        cache.stats(),
        AnalysisCacheStats {
            module_hits: 1,
            module_misses: 2,
        }
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analysis_cache_invalidates_changed_module_and_dependents_only() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-semantic-cache-invalidation-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join("main.zt");
    std::fs::write(
        &entry,
        "left ::= import \"left.zt\";\nright ::= import \"right.zt\";\nleft + right\n",
    )
    .unwrap();
    std::fs::write(root.join("left.zt"), "leaf ::= import \"leaf.zt\";\nleaf\n").unwrap();
    std::fs::write(root.join("right.zt"), "2\n").unwrap();
    std::fs::write(root.join("leaf.zt"), "1\n").unwrap();

    let cache = AnalysisCache::default();
    let first = analyze_path_with_cache(&entry, &cache).unwrap();
    assert!(first.is_thir_complete(), "{:?}", first.diagnostics);
    assert_eq!(cache.stats().module_misses, 3);

    std::fs::write(root.join("leaf.zt"), "3\n").unwrap();
    let second = analyze_path_with_cache(&entry, &cache).unwrap();
    assert!(second.is_thir_complete(), "{:?}", second.diagnostics);
    assert_eq!(
        cache.stats(),
        AnalysisCacheStats {
            module_hits: 1,
            module_misses: 5,
        },
        "the independent right module should hit while leaf and left miss"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analysis_cache_invalidates_data_import_dependents() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-semantic-cache-data-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join("main.zt");
    std::fs::write(&entry, "m ::= import \"module.zt\";\nm.port\n").unwrap();
    std::fs::write(
        root.join("module.zt"),
        "cfg ::= import \"config.zti\";\ncfg\n",
    )
    .unwrap();
    std::fs::write(root.join("config.zti"), "{ port = 1; }\n").unwrap();

    let cache = AnalysisCache::default();
    let first = analyze_path_with_cache(&entry, &cache).unwrap();
    assert!(first.is_thir_complete(), "{:?}", first.diagnostics);
    std::fs::write(root.join("config.zti"), "{ port = 2; }\n").unwrap();
    let second = analyze_path_with_cache(&entry, &cache).unwrap();
    assert!(second.is_thir_complete(), "{:?}", second.diagnostics);
    assert_eq!(
        cache.stats(),
        AnalysisCacheStats {
            module_hits: 0,
            module_misses: 2,
        }
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analysis_recording_captures_all_public_package_modules() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-semantic-recorded-public-modules-{}-{unique}",
        std::process::id()
    ));
    let app = root.join("app");
    let dep = root.join("dep");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::create_dir_all(dep.join("src")).unwrap();
    std::fs::write(
            app.join("zutai.zti"),
            format!(
                "{{ formatVersion = 1; name = \"app\"; compilerCompatibility = \"{}\"; modules = []; dependencies = [{{ alias = \"dep\"; path = \"../dep\"; }};]; }}",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();
    std::fs::write(
            dep.join("zutai.zti"),
            format!(
                "{{ formatVersion = 1; name = \"dep\"; compilerCompatibility = \"{}\"; modules = [{{ name = \"api\"; path = \"src/api.zt\"; }}; {{ name = \"unused\"; path = \"src/unused.zt\"; }};]; dependencies = []; }}",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();
    let entry = app.join("src/main.zt");
    let api = dep.join("src/api.zt");
    let unused = dep.join("src/unused.zt");
    std::fs::write(&entry, "api ::= import dep.api;\napi.answer\n").unwrap();
    std::fs::write(&api, "answer ::= 1;\n{ answer = answer; }\n").unwrap();
    std::fs::write(&unused, "unused ::= 2;\n{ unused = unused; }\n").unwrap();

    let recorded = analyze_path_recording(&entry).unwrap();
    let dep_package = recorded
        .packages
        .packages
        .values()
        .find(|package| package.name == "dep")
        .unwrap();
    assert_eq!(
        dep_package.sources.keys().cloned().collect::<Vec<_>>(),
        vec!["src/api.zt", "src/unused.zt"]
    );
    assert_eq!(
        recorded
            .source_paths
            .values()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            std::fs::canonicalize(api).unwrap(),
            std::fs::canonicalize(unused).unwrap(),
        ])
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analysis_cache_hits_preserve_recorded_source_graph() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-semantic-cache-recording-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join("main.zt");
    std::fs::write(&entry, "mid ::= import \"mid.zt\";\nmid\n").unwrap();
    std::fs::write(
        root.join("mid.zt"),
        "cfg ::= import \"config.zti\";\ncfg.port\n",
    )
    .unwrap();
    std::fs::write(root.join("config.zti"), "{ port = 1; }\n").unwrap();

    let cache = AnalysisCache::default();
    let first = analyze_path_recording_with_root_and_cache(&entry, &root, &cache).unwrap();
    let second = analyze_path_recording_with_root_and_cache(&entry, &root, &cache).unwrap();
    assert!(
        second.analysis.is_thir_complete(),
        "{:?}",
        second.analysis.diagnostics
    );
    assert_eq!(second.sources, first.sources);
    assert_eq!(second.source_paths, first.source_paths);
    assert!(cache.stats().module_hits >= 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn analysis_cache_hits_preserve_explicit_stdlib_sources() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-semantic-cache-stdlib-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join("main.zt");
    std::fs::write(&entry, "mid ::= import \"mid.zt\";\nmid\n").unwrap();
    std::fs::write(
        root.join("mid.zt"),
        "num ::= import stdlib.num;\nnum.abs -1\n",
    )
    .unwrap();

    let cache = AnalysisCache::default();
    let first = analyze_path_recording_with_root_and_cache(&entry, &root, &cache).unwrap();
    let second = analyze_path_recording_with_root_and_cache(&entry, &root, &cache).unwrap();
    assert!(
        second.analysis.is_thir_complete(),
        "{:?}",
        second.analysis.diagnostics
    );
    assert_eq!(second.stdlib_sources, first.stdlib_sources);
    assert!(second.stdlib_sources.contains_key("num"));
    assert!(cache.stats().module_hits >= 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn import_without_base_reports_no_base_directory() {
    let analysis = analyze("cfg ::= import \"config.zti\";\ncfg.port");
    assert!(!analysis.is_thir_complete());
    assert!(has_import_diagnostic(
        &analysis,
        &ImportDiagnosticKind::NoBaseDirectory
    ));
}

#[test]
fn import_missing_file_reports_file_not_found() {
    let analysis = analyze_in_imports("cfg ::= import \"nope.zti\";\ncfg");
    assert!(!analysis.is_thir_complete());
    assert!(analysis.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.kind,
        SemanticDiagnosticKind::Import(import)
            if matches!(import.kind, ImportDiagnosticKind::FileNotFound { .. })
    )));
}

#[test]
fn zt_import_data_module_completes() {
    let analysis = analyze_in_imports("m ::= import \"data_module.zt\";\nm.doubled");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn zt_import_transitive_completes() {
    // chain_top.zt -> chain_mid.zt -> config.zti.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/chain_top.zt");
    let analysis = analyze_path(&path).expect("read chain_top.zt");
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
}

#[test]
fn zt_import_function_module_completes() {
    // func_module.zt exports a function; the import must now succeed and
    // produce a complete THIR (no UnsupportedExport diagnostic).
    let analysis = analyze_in_imports("f ::= import \"func_module.zt\";\nf");
    assert!(
        analysis.is_thir_complete(),
        "expected complete THIR, got diagnostics: {:?}",
        analysis.diagnostics
    );
}

#[test]
fn imported_higher_kinded_witness_is_native_matchable() {
    let source = "m ::= import \"hkt_witness_list.zt\";\nm\n";
    let analysis = analyze_in_imports(source);
    assert!(
        analysis.native_import_diagnostics().is_empty(),
        "bare first-order constructor witness should cross imports"
    );
    assert!(analysis.witness_exports.iter().any(|witness| {
        witness.constraint == "Functor"
            && witness.target_key == "List"
            && witness.conditional.is_none()
    }));
}

#[test]
fn imported_partial_alias_witness_is_runtime_matchable() {
    let source = "m ::= import \"hkt_witness_result.zt\";\nm\n";
    let analysis = analyze_in_imports(source);
    assert!(analysis.native_import_diagnostics().is_empty());
    let witness = analysis
        .witness_exports
        .iter()
        .find(|witness| witness.constraint == "Functor")
        .expect("Functor witness export");
    assert!(witness.target_key.starts_with("Result['"));
    let conditional = witness
        .conditional
        .as_ref()
        .expect("partial alias witness should export a matcher");
    assert_eq!(conditional.param_bounds.len(), 1);
    assert!(
        zutai_thir::match_pattern_key(
            &conditional.pattern,
            "Result[Text][Int]",
            conditional.param_bounds.len(),
        )
        .is_some()
    );
    assert!(
        zutai_thir::match_pattern_key(
            &conditional.pattern,
            "<ok({value:Int})|err({error:Text})>",
            conditional.param_bounds.len(),
        )
        .is_none()
    );
}

mod imports_backend;
