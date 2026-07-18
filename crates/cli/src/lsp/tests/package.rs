use super::*;

fn package_manifest(name: &str, modules: &str, dependencies: &str) -> String {
    let modules = if modules.is_empty() {
        "[]".to_owned()
    } else {
        format!("[{modules};]")
    };
    let dependencies = if dependencies.is_empty() {
        "[]".to_owned()
    } else {
        format!("[{dependencies};]")
    };
    format!(
        "{{ formatVersion = 1; name = \"{name}\"; compilerCompatibility = \"{}\"; modules = {modules}; dependencies = {dependencies}; }}",
        env!("CARGO_PKG_VERSION")
    )
}

#[test]
fn package_graph_navigation_overlays_and_diagnostics_match_cli_analysis() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("zutai-lsp-package-{}-{nonce}", std::process::id()));
    let app = root.join("app");
    let dep = root.join("dep");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::create_dir_all(dep.join("src")).unwrap();
    std::fs::write(
        app.join("zutai.zti"),
        package_manifest("app", "", "{ alias = \"dep\"; path = \"../dep\"; }"),
    )
    .unwrap();
    std::fs::write(
        dep.join("zutai.zti"),
        package_manifest("dep", "{ name = \"api\"; path = \"src/api.zt\"; }", ""),
    )
    .unwrap();

    let app_path = app.join("src/main.zt");
    let dep_path = dep.join("src/api.zt");
    let app_source = "api ::= import dep.api;\nvalue :: api.Count = api.answer;\nvalue\n";
    let dep_source = "Count :: type Int;\nanswer :: Count = 42;\n{ answer = answer; }\n";
    std::fs::write(&app_path, app_source).unwrap();
    std::fs::write(&dep_path, dep_source).unwrap();
    let app_uri = file_uri(&app_path);
    let dep_uri = file_uri(&dep_path);

    let mut server = Server::default();
    server.documents.insert(
        app_uri.clone(),
        Document {
            text: app_source.to_owned(),
            version: Some(1),
        },
    );

    server.documents.insert(
        dep_uri.clone(),
        Document {
            text: dep_source.to_owned(),
            version: Some(1),
        },
    );
    let value = server.definition(&json!({
        "textDocument": { "uri": app_uri },
        "position": { "line": 1, "character": 27 }
    }));
    assert_eq!(
        value.get("uri").and_then(Value::as_str),
        Some(dep_uri.as_str())
    );
    assert_eq!(
        value.pointer("/range/start").cloned(),
        Some(json!({ "line": 1, "character": 0 }))
    );

    let ty = server.definition(&json!({
        "textDocument": { "uri": app_uri },
        "position": { "line": 1, "character": 14 }
    }));
    assert_eq!(
        ty.get("uri").and_then(Value::as_str),
        Some(dep_uri.as_str())
    );
    assert_eq!(
        ty.pointer("/range/start").cloned(),
        Some(json!({ "line": 0, "character": 0 }))
    );

    server.documents.get_mut(&dep_uri).unwrap().text =
        "Count :: type Bool;\nanswer :: Count = true;\n{ answer = answer; }\n".to_owned();
    let hover = server.hover(&json!({
        "textDocument": { "uri": app_uri },
        "position": { "line": 1, "character": 27 }
    }));
    assert_eq!(
        hover.pointer("/contents/value").and_then(Value::as_str),
        Some("```zutai\nBool\n```")
    );

    let bad_source = "api ::= import missing.api;\napi\n";
    std::fs::write(&app_path, bad_source).unwrap();
    server.documents.get_mut(&app_uri).unwrap().text = bad_source.to_owned();
    let cli = zutai_semantic::analyze_path(&app_path).unwrap();
    let cli_import = cli
        .diagnostics
        .iter()
        .find_map(|diagnostic| match &diagnostic.kind {
            zutai_semantic::SemanticDiagnosticKind::Import(import) => Some(import),
            _ => None,
        })
        .expect("CLI analysis should report the unresolved dependency");
    let project = server.analyze_with_overlays(&app_uri, bad_source).unwrap();
    let lsp_import = project
        .analysis
        .diagnostics
        .iter()
        .find_map(|diagnostic| match &diagnostic.kind {
            zutai_semantic::SemanticDiagnosticKind::Import(import) => Some(import),
            _ => None,
        })
        .expect("LSP analysis should report the unresolved dependency");
    assert_eq!(lsp_import, cli_import);
    let diagnostic = diagnostic_value(
        bad_source,
        &app_uri,
        project
            .analysis
            .diagnostics
            .iter()
            .find(|diagnostic| {
                matches!(
                    diagnostic.kind,
                    zutai_semantic::SemanticDiagnosticKind::Import(_)
                )
            })
            .unwrap(),
    );
    assert_eq!(
        diagnostic.pointer("/range/start").cloned(),
        Some(position_at(bad_source, cli_import.span.start as usize))
    );
    assert_eq!(
        diagnostic.pointer("/range/end").cloned(),
        Some(position_at(bad_source, cli_import.span.end as usize))
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn package_completion_and_workspace_symbols_respect_graph_overlays_and_visibility() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-lsp-package-completion-{}-{nonce}",
        std::process::id()
    ));
    let app = root.join("app");
    let middle = root.join("middle");
    let leaf = root.join("leaf");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::create_dir_all(middle.join("src")).unwrap();
    std::fs::create_dir_all(leaf.join("src")).unwrap();
    std::fs::write(
        app.join("zutai.zti"),
        package_manifest("app", "", "{ alias = \"middle\"; path = \"../middle\"; }"),
    )
    .unwrap();
    std::fs::write(
            middle.join("zutai.zti"),
            package_manifest(
                "middle",
                "{ name = \"api\"; path = \"src/api.zt\"; }; { name = \"extra.tools\"; path = \"src/extra.zt\"; }; { name = \"unused\"; path = \"src/unused.zt\"; }",
                "{ alias = \"leaf\"; path = \"../leaf\"; }",
            ),
        )
        .unwrap();
    std::fs::write(
        leaf.join("zutai.zti"),
        package_manifest("leaf", "{ name = \"api\"; path = \"src/api.zt\"; }", ""),
    )
    .unwrap();

    let app_path = app.join("src/main.zt");
    let middle_path = middle.join("src/api.zt");
    let extra_path = middle.join("src/extra.zt");
    let unused_path = middle.join("src/unused.zt");
    let leaf_path = leaf.join("src/api.zt");
    let app_source = "m ::= import middle.api;\nvalue ::= m.answer;\nvalue\n";
    let middle_source = "l ::= import leaf.api;\nmiddleOnly ::= 1;\n{ Count = l.Count; answer = l.answer; middleOnly = middleOnly; }\n";
    let middle_overlay = "l ::= import leaf.api;\noverlayOnly ::= 2;\n{ Count = l.Count; answer = l.answer; overlayOnly = overlayOnly; }\n";
    let extra_source = "extraTool ::= 7;\n{ extraTool = extraTool; }\n";
    let unused_source = "unusedSymbol ::= 8;\n{ unusedSymbol = unusedSymbol; }\n";
    let leaf_source =
        "Count :: type Int;\nanswer :: Count = 41;\n{ Count = Count; answer = answer; }\n";
    for (path, source) in [
        (&app_path, app_source),
        (&middle_path, middle_source),
        (&extra_path, extra_source),
        (&unused_path, unused_source),
        (&leaf_path, leaf_source),
    ] {
        std::fs::write(path, source).unwrap();
    }
    let app_uri = file_uri(&app_path);
    let middle_uri = file_uri(&middle_path);
    let unused_uri = file_uri(&unused_path);
    let leaf_uri = file_uri(&leaf_path);
    let mut server = Server::default();
    for (uri, text) in [(&app_uri, app_source), (&middle_uri, middle_overlay)] {
        server.documents.insert(
            uri.clone(),
            Document {
                text: text.to_owned(),
                version: Some(1),
            },
        );
    }

    let complete = |server: &Server, source: &str| {
        server.completion(&json!({
            "textDocument": { "uri": app_uri },
            "position": position_at(source, source.trim_end_matches('\n').len())
        }))
    };
    server.documents.get_mut(&app_uri).unwrap().text = "m ::= import mid\n".to_owned();
    let aliases = complete(&server, "m ::= import mid\n");
    assert_eq!(
        aliases.as_array().unwrap()[0]["label"],
        json!("middle"),
        "{aliases}"
    );
    assert_eq!(
        aliases.as_array().unwrap()[0]["textEdit"]["range"]["start"],
        json!({ "line": 0, "character": 13 })
    );

    server.documents.get_mut(&app_uri).unwrap().text = "m ::= import middle.ex\n".to_owned();
    let modules = complete(&server, "m ::= import middle.ex\n");
    assert_eq!(modules.as_array().unwrap()[0]["label"], json!("extra"));
    assert_eq!(
        modules.as_array().unwrap()[0]["detail"],
        json!("module namespace")
    );

    server.documents.get_mut(&app_uri).unwrap().text = "m ::= import middle.extra.to\n".to_owned();
    let nested = complete(&server, "m ::= import middle.extra.to\n");
    assert_eq!(nested.as_array().unwrap()[0]["label"], json!("tools"));

    server.documents.get_mut(&app_uri).unwrap().text = app_source.to_owned();
    let members = server.completion(&json!({
        "textDocument": { "uri": app_uri },
        "position": { "line": 1, "character": 12 }
    }));
    let member_names: Vec<_> = members
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item.get("label").and_then(Value::as_str))
        .collect();
    assert!(member_names.contains(&"answer"), "{members}");
    assert!(member_names.contains(&"overlayOnly"), "{members}");
    assert!(!member_names.contains(&"middleOnly"), "{members}");

    let symbols = server.workspace_symbols(&json!({ "query": "unusedSymbol" }));
    let symbol = &symbols.as_array().unwrap()[0];
    assert_eq!(symbol["name"], json!("unusedSymbol"));
    assert_eq!(symbol["location"]["uri"], json!(unused_uri));
    assert_eq!(
        symbol["location"]["range"],
        range(unused_source, 0, "unusedSymbol".len())
    );
    assert_eq!(symbol["containerName"], json!("middle.unused"));

    let overlay_symbols = server.workspace_symbols(&json!({ "query": "overlayOnly" }));
    let overlay_symbol = &overlay_symbols.as_array().unwrap()[0];
    assert_eq!(overlay_symbol["location"]["uri"], json!(middle_uri));
    assert_eq!(
        overlay_symbol["location"]["range"],
        range(
            middle_overlay,
            middle_overlay.find("overlayOnly").unwrap(),
            middle_overlay.find("overlayOnly").unwrap() + "overlayOnly".len()
        )
    );
    assert_eq!(
        server.workspace_symbols(&json!({ "query": "middleOnly" })),
        json!([])
    );
    let imported_symbols = server.workspace_symbols(&json!({ "query": "answer" }));
    assert!(
        imported_symbols.as_array().unwrap().iter().any(|symbol| {
            symbol["location"]["uri"] == json!(leaf_uri)
                && symbol["location"]["range"]
                    == range(
                        leaf_source,
                        leaf_source.find("answer").unwrap(),
                        leaf_source.find("answer").unwrap() + "answer".len(),
                    )
        }),
        "{imported_symbols}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn package_wide_references_and_safe_rename_respect_overlays_and_shadowing() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-lsp-package-references-{}-{nonce}",
        std::process::id()
    ));
    let app = root.join("app");
    let middle = root.join("middle");
    let leaf = root.join("leaf");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::create_dir_all(middle.join("src")).unwrap();
    std::fs::create_dir_all(leaf.join("src")).unwrap();
    std::fs::write(
        app.join("zutai.zti"),
        package_manifest("app", "", "{ alias = \"middle\"; path = \"../middle\"; }"),
    )
    .unwrap();
    std::fs::write(
            middle.join("zutai.zti"),
            package_manifest(
                "middle",
                "{ name = \"api\"; path = \"src/api.zt\"; }; { name = \"other\"; path = \"src/other.zt\"; }",
                "{ alias = \"leaf\"; path = \"../leaf\"; }",
            ),
        )
        .unwrap();
    std::fs::write(
        leaf.join("zutai.zti"),
        package_manifest("leaf", "{ name = \"api\"; path = \"src/api.zt\"; }", ""),
    )
    .unwrap();

    let app_path = app.join("src/main.zt");
    let middle_path = middle.join("src/api.zt");
    let leaf_path = leaf.join("src/api.zt");
    let other_path = middle.join("src/other.zt");
    let app_source = "m ::= import middle.api;\no ::= import middle.other;\nvalue :: m.Count = m.answer + m.answer;\nunrelated ::= o.answer;\nvalue\n";
    let middle_source = "l ::= import leaf.api;\nvalue :: l.Count = l.answer;\n{ Count = l.Count; answer = l.answer; }\n";
    let other_source =
        "l ::= import leaf.api;\nown ::= 99;\n{ answer = own; retained = l.answer; }\n";
    let leaf_source = "Count :: type Int;\nanswer :: Count = 41;\nshadow ::= (\\answer. answer) 1;\n{ Count = Count; answer = answer; }\n";
    let leaf_overlay = "Count :: type Int;\nanswer :: Count = 42;\nshadow ::= (\\answer. answer) 1;\n{ Count = Count; answer = answer; }\n";
    std::fs::write(&other_path, other_source).unwrap();
    std::fs::write(&app_path, app_source).unwrap();
    std::fs::write(&middle_path, middle_source).unwrap();
    std::fs::write(&leaf_path, leaf_source).unwrap();
    let app_uri = file_uri(&app_path);
    let middle_uri = file_uri(&middle_path);
    let other_uri = file_uri(&other_path);
    let leaf_uri = file_uri(&leaf_path);

    let mut server = Server::default();
    for (uri, text) in [
        (&app_uri, app_source),
        (&middle_uri, middle_source),
        (&other_uri, other_source),
        (&leaf_uri, leaf_overlay),
    ] {
        server.documents.insert(
            uri.clone(),
            Document {
                text: text.to_owned(),
                version: Some(1),
            },
        );
    }

    let references = server.references(&json!({
        "textDocument": { "uri": app_uri },
        "position": { "line": 2, "character": 27 },
        "context": { "includeDeclaration": true }
    }));
    let locations = references.as_array().unwrap();
    assert_eq!(locations.len(), 9, "{references}");
    let by_uri = |uri: &str| {
        locations
            .iter()
            .filter(|location| location.get("uri").and_then(Value::as_str) == Some(uri))
            .count()
    };
    assert_eq!(by_uri(&app_uri), 2, "{references}");
    assert_eq!(by_uri(&middle_uri), 3, "{references}");
    assert_eq!(by_uri(&leaf_uri), 3, "{references}");
    assert_eq!(by_uri(&other_uri), 1, "{references}");
    assert!(!locations.iter().any(|location| {
        location.get("uri").and_then(Value::as_str) == Some(leaf_uri.as_str())
            && location
                .pointer("/range/start/line")
                .and_then(Value::as_u64)
                == Some(2)
    }));

    let rename = server.rename(&json!({
        "textDocument": { "uri": leaf_uri },
        "position": { "line": 1, "character": 2 },
        "newName": "total"
    }));
    let changes = rename
        .pointer("/changes")
        .and_then(Value::as_object)
        .unwrap();
    assert_eq!(changes.len(), 4, "{rename}");
    assert_eq!(changes[&app_uri].as_array().map(Vec::len), Some(2));
    assert_eq!(changes[&middle_uri].as_array().map(Vec::len), Some(3));
    assert_eq!(changes[&leaf_uri].as_array().map(Vec::len), Some(3));
    assert_eq!(changes[&other_uri].as_array().map(Vec::len), Some(1));
    assert!(
        !changes[&leaf_uri]
            .as_array()
            .unwrap()
            .iter()
            .any(|edit| { edit.pointer("/range/start/line").and_then(Value::as_u64) == Some(2) })
    );

    let apply_edits = |source: &str, edits: &Value| {
        let mut replacements: Vec<_> = edits
            .as_array()
            .unwrap()
            .iter()
            .map(|edit| {
                let edit_range = edit.get("range").unwrap();
                let start = offset_at(source, edit_range.get("start").unwrap()).unwrap();
                let end = offset_at(source, edit_range.get("end").unwrap()).unwrap();
                (start, end, edit.get("newText").unwrap().as_str().unwrap())
            })
            .collect();
        replacements.sort_unstable_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        let mut renamed = source.to_owned();
        for (start, end, replacement) in replacements {
            renamed.replace_range(start..end, replacement);
        }
        renamed
    };
    let renamed_app = apply_edits(app_source, &changes[&app_uri]);
    let renamed_middle = apply_edits(middle_source, &changes[&middle_uri]);
    let renamed_leaf = apply_edits(leaf_overlay, &changes[&leaf_uri]);
    let renamed_other = apply_edits(other_source, &changes[&other_uri]);
    let mut renamed_server = Server::default();
    for (uri, text) in [
        (&app_uri, &renamed_app),
        (&middle_uri, &renamed_middle),
        (&other_uri, &renamed_other),
        (&leaf_uri, &renamed_leaf),
    ] {
        renamed_server.documents.insert(
            uri.clone(),
            Document {
                text: text.clone(),
                version: Some(2),
            },
        );
    }
    let renamed_project = renamed_server
        .project_for_document(&app_uri, &renamed_app)
        .expect("renamed project analysis");
    assert!(
        renamed_project
            .modules()
            .into_iter()
            .all(|analysis| analysis.diagnostics.is_empty()),
        "renamed sources must remain valid: {renamed_app}\n{renamed_middle}\n{renamed_leaf}"
    );

    let type_references = server.references(&json!({
        "textDocument": { "uri": app_uri },
        "position": { "line": 2, "character": 13 },
        "context": { "includeDeclaration": true }
    }));
    assert_eq!(type_references.as_array().map(Vec::len), Some(8));

    let mut locked_project = server
        .project_for_document(&app_uri, app_source)
        .expect("three-package project");
    let leaf_id = locked_project
        .packages
        .packages
        .iter()
        .find(|(_, package)| package.name == "leaf")
        .map(|(id, _)| id.clone())
        .unwrap();
    locked_project
        .packages
        .packages
        .get_mut(&leaf_id)
        .unwrap()
        .source = zutai_package::PortablePackageSource::LockedGit;
    let leaf_analysis = locked_project
        .modules()
        .into_iter()
        .find(|analysis| locked_project.path_for(analysis).as_deref() == Some(&leaf_path))
        .unwrap();
    let target = SymbolTarget::ExportedMember {
        module: locked_project.module_identity(leaf_analysis),
        member: "answer".to_owned(),
    };
    assert!(!server.renameable_symbol(&locked_project, &target));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn package_diagnostics_route_imports_and_manifest_provenance() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-lsp-package-diagnostics-{}-{nonce}",
        std::process::id()
    ));

    let unknown = root.join("unknown");
    std::fs::create_dir_all(unknown.join("src")).unwrap();
    std::fs::write(
        unknown.join("zutai.zti"),
        package_manifest("unknown", "", ""),
    )
    .unwrap();
    let unknown_path = unknown.join("src/main.zt");
    let unknown_source = "api ::= import missing.api;\napi\n";
    std::fs::write(&unknown_path, unknown_source).unwrap();
    let unknown_uri = file_uri(&unknown_path);
    let mut server = Server::default();
    server.documents.insert(
        unknown_uri.clone(),
        Document {
            text: unknown_source.to_owned(),
            version: Some(1),
        },
    );
    let project = server
        .analyze_with_overlays(&unknown_uri, unknown_source)
        .unwrap();
    let routed = server.routed_diagnostics(&unknown_uri, unknown_source, &project);
    let (uri, diagnostic) = routed
        .iter()
        .find(|(_, diagnostic)| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("unknown package dependency alias"))
        })
        .expect("unknown package alias diagnostic");
    assert_eq!(uri, &unknown_uri);
    assert_eq!(diagnostic["range"]["start"]["line"], json!(0));

    let duplicate = root.join("duplicate");
    let dep = root.join("dep");
    std::fs::create_dir_all(duplicate.join("src")).unwrap();
    std::fs::create_dir_all(dep.join("src")).unwrap();
    std::fs::write(dep.join("zutai.zti"), package_manifest("dep", "", "")).unwrap();
    let duplicate_manifest = package_manifest(
        "duplicate",
        "",
        "{ alias = \"dep\"; path = \"../dep\"; }; { alias = \"dep\"; path = \"../dep\"; }",
    );
    let duplicate_manifest_path = duplicate.join("zutai.zti");
    std::fs::write(&duplicate_manifest_path, &duplicate_manifest).unwrap();
    let duplicate_path = duplicate.join("src/main.zt");
    let duplicate_source = "api ::= import dep.api;\napi\n";
    std::fs::write(&duplicate_path, duplicate_source).unwrap();
    let duplicate_uri = file_uri(&duplicate_path);
    let duplicate_manifest_uri = file_uri(&duplicate_manifest_path);
    server.documents.insert(
        duplicate_uri.clone(),
        Document {
            text: duplicate_source.to_owned(),
            version: Some(1),
        },
    );
    let project = server
        .analyze_with_overlays(&duplicate_uri, duplicate_source)
        .unwrap();
    let routed = server.routed_diagnostics(&duplicate_uri, duplicate_source, &project);
    let (uri, diagnostic) = routed
        .iter()
        .find(|(_, diagnostic)| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("duplicate dependency alias"))
        })
        .expect("duplicate dependency alias diagnostic");
    assert_eq!(uri, &duplicate_manifest_uri);
    let second_alias = duplicate_manifest.rfind("\"dep\"").unwrap();
    assert_eq!(
        diagnostic["range"]["start"],
        position_at(&duplicate_manifest, second_alias)
    );

    let a = root.join("a");
    let b = root.join("b");
    std::fs::create_dir_all(a.join("src")).unwrap();
    std::fs::create_dir_all(b.join("src")).unwrap();
    std::fs::write(
        a.join("zutai.zti"),
        package_manifest("a", "", "{ alias = \"b\"; path = \"../b\"; }"),
    )
    .unwrap();
    std::fs::write(
        b.join("zutai.zti"),
        package_manifest("b", "", "{ alias = \"a\"; path = \"../a\"; }"),
    )
    .unwrap();
    let cycle_path = a.join("src/main.zt");
    let cycle_source = "api ::= import b.api;\napi\n";
    std::fs::write(&cycle_path, cycle_source).unwrap();
    let cycle_uri = file_uri(&cycle_path);
    server.documents.insert(
        cycle_uri.clone(),
        Document {
            text: cycle_source.to_owned(),
            version: Some(1),
        },
    );
    let project = server
        .analyze_with_overlays(&cycle_uri, cycle_source)
        .unwrap();
    let routed = server.routed_diagnostics(&cycle_uri, cycle_source, &project);
    let (uri, diagnostic) = routed
        .iter()
        .find(|(_, diagnostic)| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("package dependency cycle"))
        })
        .expect("package dependency cycle diagnostic");
    assert_eq!(uri, &cycle_uri);
    let related = diagnostic["relatedInformation"].as_array().unwrap();
    assert_eq!(related.len(), 2);
    assert_eq!(
        related[0]["location"]["uri"],
        json!(file_uri(&a.join("zutai.zti")))
    );
    assert_eq!(
        related[1]["location"]["uri"],
        json!(file_uri(&b.join("zutai.zti")))
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn malformed_package_manifest_diagnostic_survives_overlay_analysis() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "zutai-lsp-package-setup-{}-{nonce}",
        std::process::id()
    ));
    let entry = root.join("src/main.zt");
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    std::fs::write(
        root.join("zutai.zti"),
        "{ formatVersion = \"bad\"; name = \"app\"; modules = []; dependencies = []; }\n",
    )
    .unwrap();
    let source = "1\n";
    std::fs::write(&entry, source).unwrap();

    let cli = zutai_semantic::analyze_path(&entry).unwrap();
    let cli_setup = cli
        .diagnostics
        .iter()
        .find_map(|diagnostic| match &diagnostic.kind {
            zutai_semantic::SemanticDiagnosticKind::Import(import)
                if matches!(
                    import.kind,
                    zutai_semantic::ImportDiagnosticKind::PackageSetup { .. }
                ) =>
            {
                Some(import)
            }
            _ => None,
        })
        .expect("CLI analysis should report the malformed package manifest");

    let uri = file_uri(&entry);
    let mut server = Server::default();
    server.documents.insert(
        uri.clone(),
        Document {
            text: source.to_owned(),
            version: Some(1),
        },
    );
    let project = server.analyze_with_overlays(&uri, source).unwrap();
    let lsp_setup = project
        .analysis
        .diagnostics
        .iter()
        .find_map(|diagnostic| match &diagnostic.kind {
            zutai_semantic::SemanticDiagnosticKind::Import(import)
                if matches!(
                    import.kind,
                    zutai_semantic::ImportDiagnosticKind::PackageSetup { .. }
                ) =>
            {
                Some(import)
            }
            _ => None,
        })
        .expect("LSP analysis should preserve the malformed package manifest diagnostic");
    assert_eq!(lsp_setup, cli_setup);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn independent_qualification_app_lsp_matches_cli_and_navigates_policy() {
    let root =
        std::fs::canonicalize(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .unwrap();
    let entry = root.join("examples/qualification/app/src/main.zt");
    let policy = root.join("examples/qualification/app/policy/src/service.zt");
    let entry_source = std::fs::read_to_string(&entry).unwrap();
    let entry_uri = file_uri(&entry);
    let policy_uri = file_uri(&policy);

    let cli = zutai_semantic::analyze_path(&entry).unwrap();
    let mut server = Server::default();
    server.documents.insert(
        entry_uri.clone(),
        Document {
            text: entry_source.clone(),
            version: Some(1),
        },
    );
    let project = server
        .analyze_with_overlays(&entry_uri, &entry_source)
        .unwrap();
    assert_eq!(project.analysis.diagnostics, cli.diagnostics);

    let location = server.definition(&json!({
        "textDocument": { "uri": entry_uri },
        "position": { "line": 44, "character": 28 }
    }));
    assert_eq!(
        location.get("uri").and_then(Value::as_str),
        Some(policy_uri.as_str())
    );
    assert_eq!(
        location.pointer("/range/start").cloned(),
        Some(json!({ "line": 1, "character": 0 }))
    );
}

#[test]
fn framing_round_trip() {
    let input = b"Content-Length: 17\r\n\r\n{\"method\":\"ping\"}";
    assert_eq!(
        read_message(&mut &input[..]).unwrap(),
        Some(json!({ "method": "ping" }))
    );
}

#[test]
fn file_uris_preserve_absolute_paths() {
    assert_eq!(
        file_path("file:///tmp/example.zt"),
        Some(PathBuf::from("/tmp/example.zt"))
    );
    assert_eq!(
        file_path("file://localhost/tmp/example.zt"),
        Some(PathBuf::from("/tmp/example.zt"))
    );
    let spaced = PathBuf::from("/tmp/Zutai data/A.zti");
    assert_eq!(file_uri(&spaced), "file:///tmp/Zutai%20data/A.zti");
    assert_eq!(file_path(&file_uri(&spaced)), Some(spaced));
}
