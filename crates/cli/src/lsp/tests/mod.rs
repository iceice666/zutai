use super::*;

#[test]
fn positions_use_utf16_code_units() {
    let source = "x😀\n終";
    assert_eq!(position_at(source, 5), json!({ "line": 0, "character": 3 }));
    assert_eq!(
        offset_at(source, &json!({ "line": 0, "character": 3 })),
        Some(5)
    );
}

#[test]
fn parse_hover_and_publish_diagnostics() {
    let uri = "file:///tmp/example.zt";
    let mut server = Server::default();
    let mut output = Vec::new();
    let open = json!({ "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "text": "x ::= 1;\nx" } } });
    server.handle(open, &mut output).unwrap();
    assert!(String::from_utf8_lossy(&output).contains("publishDiagnostics"));

    let hover = server.hover(
        &json!({ "textDocument": { "uri": uri }, "position": { "line": 1, "character": 0 } }),
    );
    assert_eq!(
        hover.pointer("/contents/value").and_then(Value::as_str),
        Some("```zutai\nInt\n```")
    );
}

#[test]
fn definition_resolves_value_and_type_bindings_with_utf16_ranges() {
    let uri = "file:///tmp/definition.zt";
    let mut server = Server::default();
    server.documents.insert(
        uri.to_string(),
        Document {
            text: "名 ::= 1;\nCount :: type Int;\nvalue :: Count = 名;\nvalue".to_string(),
            version: None,
        },
    );

    let value = server.definition(
        &json!({ "textDocument": { "uri": uri }, "position": { "line": 3, "character": 1 } }),
    );
    assert_eq!(
        value,
        json!({
            "uri": uri,
            "range": {
                "start": { "line": 2, "character": 0 },
                "end": { "line": 2, "character": 5 }
            }
        })
    );

    let ty = server.definition(
        &json!({ "textDocument": { "uri": uri }, "position": { "line": 2, "character": 10 } }),
    );
    assert_eq!(
        ty.pointer("/range/start/line").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        ty.pointer("/range/end/character").and_then(Value::as_u64),
        Some(5)
    );

    let unicode = server.definition(
        &json!({ "textDocument": { "uri": uri }, "position": { "line": 2, "character": 17 } }),
    );
    assert_eq!(
        unicode.pointer("/range/start").cloned(),
        Some(json!({ "line": 0, "character": 0 }))
    );
    assert_eq!(
        unicode.pointer("/range/end").cloned(),
        Some(json!({ "line": 0, "character": 1 }))
    );
}

#[test]
fn document_formatting_uses_overlays_and_full_utf16_ranges() {
    let zt_uri = "file:///tmp/format.zt";
    let zt_source = "名 ::= {\nx = 1;\n};\n名";
    let mut server = Server::default();
    server.documents.insert(
        zt_uri.to_owned(),
        Document {
            text: zt_source.to_owned(),
            version: Some(2),
        },
    );
    let edits = server.formatting(&json!({
        "textDocument": { "uri": zt_uri },
        "options": { "tabSize": 8, "insertSpaces": false }
    }));
    assert_eq!(
        edits.pointer("/0/newText").and_then(Value::as_str),
        Some("名 ::= {\n  x = 1;\n};\n名\n")
    );
    assert_eq!(
        edits.pointer("/0/range"),
        Some(&range(zt_source, 0, zt_source.len()))
    );

    let zti_uri = "file:///tmp/format.zti";
    server.documents.insert(
        zti_uri.to_owned(),
        Document {
            text: "{second=[2;1;];first=true;}".to_owned(),
            version: None,
        },
    );
    let zti_edits = server.formatting(&json!({
        "textDocument": { "uri": zti_uri },
        "options": { "tabSize": 2, "insertSpaces": true }
    }));
    let formatted = zti_edits
        .pointer("/0/newText")
        .and_then(Value::as_str)
        .unwrap();
    assert!(formatted.find("second").unwrap() < formatted.find("first").unwrap());
    assert!(formatted.find("2;").unwrap() < formatted.find("1;").unwrap());
    assert_eq!(zutai_im::format_source(formatted).unwrap(), formatted);
}

#[test]
fn definition_works_when_later_type_checking_fails() {
    let uri = "file:///tmp/incomplete.zt";
    let mut server = Server::default();
    server.documents.insert(
        uri.to_string(),
        Document {
            text: "answer ::= 42;\nanswer + \"bad\"".to_string(),
            version: None,
        },
    );

    let result = server.definition(
        &json!({ "textDocument": { "uri": uri }, "position": { "line": 1, "character": 2 } }),
    );
    assert_eq!(
        result.pointer("/range/start").cloned(),
        Some(json!({ "line": 0, "character": 0 }))
    );
    assert_eq!(
        result.pointer("/range/end").cloned(),
        Some(json!({ "line": 0, "character": 6 }))
    );
}

#[test]
fn initialize_advertises_definition_support() {
    let mut server = Server::default();
    let mut output = Vec::new();
    server
        .handle(json!({ "id": 1, "method": "initialize" }), &mut output)
        .unwrap();
    let message = String::from_utf8(output).unwrap();
    assert!(message.contains("definitionProvider"));
    assert!(message.contains("referencesProvider"));
    assert!(message.contains("renameProvider"));
    assert!(message.contains("completionProvider"));
    assert!(message.contains("codeActionProvider"));
    assert!(message.contains("documentFormattingProvider"));
}

#[test]
fn incremental_changes_preserve_utf16_positions_and_diagnostic_versions() {
    let uri = "file:///tmp/change.zt";
    let mut server = Server::default();
    let mut output = Vec::new();
    server
        .handle(
            json!({
                "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": uri, "version": 1, "text": "名 ::= 1;\n名" } }
            }),
            &mut output,
        )
        .unwrap();
    output.clear();
    server
        .handle(
            json!({
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": uri, "version": 2 },
                    "contentChanges": [{
                        "range": {
                            "start": { "line": 1, "character": 0 },
                            "end": { "line": 1, "character": 1 }
                        },
                        "text": "名 + \"bad\""
                    }]
                }
            }),
            &mut output,
        )
        .unwrap();

    assert_eq!(
        server.source_for(uri).as_deref(),
        Some("名 ::= 1;\n名 + \"bad\"")
    );
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("\"version\":2"), "{output}");
    assert!(output.contains("publishDiagnostics"), "{output}");
}

#[test]
fn references_and_rename_are_binding_accurate() {
    let uri = "file:///tmp/rename.zt";
    let mut server = Server::default();
    server.documents.insert(
        uri.to_string(),
        Document {
            text: "value ::= 1;\nuse ::= value;\nvalue".to_string(),
            version: Some(4),
        },
    );
    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 8 },
        "context": { "includeDeclaration": true }
    });
    let references = server.references(&params);
    assert_eq!(references.as_array().map(Vec::len), Some(3));
    assert_eq!(
        server
            .prepare_rename(&params)
            .pointer("/start/line")
            .and_then(Value::as_u64),
        Some(0)
    );

    let rename = server.rename(&json!({
        "textDocument": { "uri": uri },
        "position": { "line": 1, "character": 8 },
        "newName": "renamed"
    }));
    assert_eq!(
        rename
            .get("changes")
            .and_then(|changes| changes.get(uri))
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(3)
    );
    assert_eq!(
        server.rename(&json!({
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": 8 },
            "newName": "match"
        })),
        Value::Null
    );
}

#[test]
fn symbols_completion_and_signature_help_use_semantic_information() {
    let uri = "file:///tmp/features.zt";
    let mut server = Server::default();
    server.documents.insert(
        uri.to_string(),
        Document {
            text: "id :: Int -> Int\n  = x => x;\nvalue ::= id 1;\nvalue".to_string(),
            version: None,
        },
    );

    let symbols = server.document_symbols(&json!({ "textDocument": { "uri": uri } }));
    let names: Vec<_> = symbols
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|symbol| symbol.get("name").and_then(Value::as_str))
        .collect();
    assert_eq!(names, ["id", "value"]);

    let completions = server.completion(&json!({
        "textDocument": { "uri": uri },
        "position": { "line": 3, "character": 3 }
    }));
    assert!(
        completions
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item.get("label").and_then(Value::as_str) == Some("value") })
    );

    let signature = server.signature_help(&json!({
        "textDocument": { "uri": uri },
        "position": { "line": 2, "character": 10 }
    }));
    assert_eq!(
        signature
            .pointer("/signatures/0/label")
            .and_then(Value::as_str),
        Some("id : Int -> Int")
    );
}

#[test]
fn completion_does_not_treat_import_prefixed_identifiers_as_imports() {
    let uri = "file:///tmp/import-prefixed.zt";
    let mut server = Server::default();
    let source = "important ::= 1;\nimportant";
    server.documents.insert(
        uri.to_owned(),
        Document {
            text: source.to_owned(),
            version: None,
        },
    );

    let completions = server.completion(&json!({
        "textDocument": { "uri": uri },
        "position": position_at(source, source.len())
    }));
    assert!(
        completions
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item.get("label").and_then(Value::as_str) == Some("important") }),
        "{completions}"
    );
}

#[test]
fn parser_fixes_are_published_as_quick_fixes() {
    let uri = "file:///tmp/fix.zt";
    let mut server = Server::default();
    server.documents.insert(
        uri.to_string(),
        Document {
            text: "value : Int = 1;\nvalue".to_string(),
            version: None,
        },
    );

    let actions = server.code_actions(&json!({
        "textDocument": { "uri": uri },
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 16 }
        },
        "context": { "diagnostics": [] }
    }));
    assert_eq!(actions.as_array().map(Vec::len), Some(1));
    assert_eq!(
        actions.pointer("/0/title").and_then(Value::as_str),
        Some("Use `::` for typed binding")
    );
    assert_eq!(
        actions
            .pointer("/0/edit/changes/file:~1~1~1tmp~1fix.zt/0/newText")
            .and_then(Value::as_str),
        Some("::")
    );
}

#[test]
fn hir_diagnostic_carries_code_severity_and_related_information() {
    let source = "T :: type { a : Int; a : Text; };\nT";
    let analysis = analyze(source, "file:///tmp/duplicate.zt").unwrap();
    let duplicate = diagnostics(source, &analysis)
        .into_iter()
        .find(|diagnostic| diagnostic["code"] == json!("zutai::hir::duplicate_type_record_field"))
        .expect("expected duplicate-type-record-field diagnostic");

    assert_eq!(duplicate["severity"], json!(1));
    assert_eq!(duplicate["source"], json!("zutai"));
    let related = duplicate["relatedInformation"].as_array().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0]["message"], json!("first occurrence"));
}

#[test]
fn parser_diagnostic_includes_protocol_range() {
    let analysis = analyze("x ::= ;\nx", "file:///tmp/bad.zt").unwrap();
    let diagnostics = diagnostics("x ::= ;\nx", &analysis);
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].get("range").is_some());
}

#[test]
fn stdlib_html_and_css_type_errors_publish_lsp_diagnostics() {
    let cases = [
        (
            "file:///tmp/bad-html.zt",
            "html ::= import stdlib.html;\nMsg :: type { #save; };\nbad :: html.Html Msg = html.button { html.onClick 1; } { html.text \"save\"; };\nbad",
            "type mismatch",
        ),
        (
            "file:///tmp/bad-css.zt",
            "css ::= import stdlib.css;\nbad :: css.Stylesheet = css.stylesheet { css.rule { css.class \"card\"; } { css.padding (css.rem \"large\"); }; };\nbad",
            "expected Float, found Text",
        ),
    ];

    for (uri, source, expected) in cases {
        let analysis = analyze(source, uri).unwrap();
        let diagnostics = diagnostics(source, &analysis);
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains(expected))),
            "expected `{expected}` for {uri}, got {diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.get("range").is_some())
        );
    }
}

#[test]
fn derive_diagnostic_carries_definition_related_information() {
    let source = "Ord :: <A> @A { compare :: A -> A -> Bool; } derive\nOrd @Int :: derive\n1";
    let analysis = analyze(source, "file:///tmp/derive.zt").unwrap();

    let diagnostics = diagnostics(source, &analysis);
    let derive = diagnostics
        .iter()
        .find(|d| {
            d.get("message")
                .and_then(|m| m.as_str())
                .is_some_and(|m| m.contains("cannot derive `Ord`"))
        })
        .expect("expected the derive diagnostic");
    let related = derive
        .get("relatedInformation")
        .and_then(|r| r.as_array())
        .expect("derive diagnostic should carry relatedInformation");
    assert_eq!(related.len(), 1);
    assert_eq!(
        related[0]["message"].as_str(),
        Some("constraint defined here")
    );
    // The related range starts on line 0 (the constraint declaration), while
    // the primary range is the derive request on line 1.
    assert_eq!(
        related[0]["location"]["range"]["start"]["line"].as_u64(),
        Some(0)
    );

    assert_eq!(
        derive["code"],
        json!("zutai::thir::derive_unsupported_method")
    );
    assert_eq!(derive["severity"], json!(1));
    assert_eq!(
        derive["range"]["start"]["line"].as_u64(),
        Some(1),
        "primary range should sit at the derive request"
    );
}

#[test]
fn native_gated_import_warning_retains_use_and_export_locations() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "zutai-lsp-native-import-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let dep_path = dir.join("dep.zt");
    let root_path = dir.join("main.zt");
    let dep_source = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nConfig :: type { port : Int; };\nEq @(Patch Config) :: { eq = \\a b. true; }\n1\n";
    let root_source = "m ::= import \"dep.zt\";\nm\n";
    std::fs::write(&dep_path, dep_source).unwrap();
    std::fs::write(&root_path, root_source).unwrap();
    let root_uri = file_uri(&root_path);
    let mut server = Server::default();
    server.documents.insert(
        root_uri.clone(),
        Document {
            text: root_source.to_owned(),
            version: Some(1),
        },
    );
    let project = server
        .analyze_with_overlays(&root_uri, root_source)
        .expect("project analysis");
    let diagnostics = server.routed_diagnostics(&root_uri, root_source, &project);
    let warning = diagnostics
        .iter()
        .map(|(_, diagnostic)| diagnostic)
        .find(|diagnostic| {
            diagnostic
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains("non-matchable typeclass instances"))
        })
        .expect("native import warning");
    assert_eq!(
        warning["code"],
        json!("zutai::backend::import_witness_non_matchable")
    );
    assert_eq!(warning["severity"], json!(2));
    assert_eq!(warning["range"]["start"]["line"], json!(0));
    let related = warning["relatedInformation"].as_array().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0]["location"]["uri"], json!(file_uri(&dep_path)));
    assert_eq!(related[0]["location"]["range"]["start"]["line"], json!(2));
    assert_eq!(
        related[0]["message"],
        json!("non-matchable witness exported here")
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn backend_warnings_expose_stable_codes_and_entry_ranges() {
    for (source, code) in [
        ("perform io.print \"x\"", "zutai::backend::residual_effect"),
        (
            "Server :: type { host : Text; };\nfields Server",
            "zutai::backend::reflection_not_foldable",
        ),
    ] {
        let uri = "file:///tmp/backend-warning.zt";
        let mut server = Server::default();
        server.documents.insert(
            uri.to_owned(),
            Document {
                text: source.to_owned(),
                version: Some(1),
            },
        );
        let project = server
            .analyze_with_overlays(uri, source)
            .expect("project analysis");
        let warning = server
            .routed_diagnostics(uri, source, &project)
            .into_iter()
            .map(|(_, diagnostic)| diagnostic)
            .find(|diagnostic| diagnostic["code"] == json!(code))
            .unwrap_or_else(|| panic!("missing {code} warning"));
        assert_eq!(warning["severity"], json!(2));
        assert_ne!(
            warning["range"]["start"], warning["range"]["end"],
            "backend warning must point at the entry expression"
        );
    }
}

#[test]
fn imported_zti_mismatch_is_published_to_data_uri_and_cleared_from_overlay() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "zutai-lsp-imported-data-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let b_path = dir.join("B.zt");
    let a_path = dir.join("A.zti");
    let c_path = dir.join("C.zt");
    let b_source = "Config :: type { port : Int; };\n{ Config = Config; }\n";
    let bad_a = "{\n  port = \"wrong\";\n}\n";
    let good_a = "{\n  port = 8080;\n}\n";
    let c_source =
        "b ::= import \"B.zt\";\na ::= import \"A.zti\";\nchecked :: b.Config = a;\nchecked\n";
    std::fs::write(&b_path, b_source).unwrap();
    std::fs::write(&a_path, bad_a).unwrap();
    std::fs::write(&c_path, c_source).unwrap();
    let a_uri = format!("file://{}", a_path.display());
    let c_uri = format!("file://{}", c_path.display());

    let mut server = Server::default();
    let mut output = Vec::new();
    server
        .handle(
            json!({ "method": "textDocument/didOpen", "params": { "textDocument": {
                    "uri": c_uri, "version": 1, "text": c_source
                } } }),
            &mut output,
        )
        .unwrap();
    server
        .handle(
            json!({ "method": "textDocument/didOpen", "params": { "textDocument": {
                    "uri": a_uri, "version": 1, "text": bad_a
                } } }),
            &mut output,
        )
        .unwrap();
    let published = String::from_utf8_lossy(&output);
    assert!(published.contains(&a_uri), "{published}");
    assert!(
        published.contains("expected Int, found Text"),
        "{published}"
    );
    assert!(published.contains("relatedInformation"), "{published}");
    assert!(
        published.contains("zutai::thir::imported_data_type_mismatch"),
        "{published}"
    );
    assert!(published.contains("\"severity\":1"), "{published}");

    output.clear();
    server
        .handle(
            json!({ "method": "textDocument/didChange", "params": {
                    "textDocument": { "uri": a_uri, "version": 2 },
                    "contentChanges": [{ "text": good_a }]
                } }),
            &mut output,
        )
        .unwrap();
    let cleared = String::from_utf8_lossy(&output);
    assert!(cleared.contains(&a_uri), "{cleared}");
    assert!(cleared.contains("\"diagnostics\":[]"), "{cleared}");
}

mod package;
