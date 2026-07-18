//! Validates the Zutai program and stdlib subset used by the wasm-bindgen-test
//! browser harness (`browser_hydration.rs`, wasm32-only) — everything
//! `start()` does up to the DOM patch runs on the host, so this catches any
//! syntax/type mistake in the shared fixture without needing a browser.
#![cfg(not(target_arch = "wasm32"))]

mod fixture;

use zutai_browser::{HeadNode, Html, WebBundleV3, decode_program, render_stylesheet};
use zutai_eval::TlcSession;
use zutai_semantic::{AnalysisOptions, StdlibSources};

const DATA_ENCODING_SOURCE: &str = r#"
Mode :: type #prod;
Choice :: type { #off; #count : { value : Int; }; };
Point :: type { x : Int; flags : List Bool; note : Text?; mode : Mode; choice : Choice; };
ToData @Mode :: derive
ToData @Choice :: derive
ToData @Point :: derive
FromData @Mode :: derive
FromData @Choice :: derive
FromData @Point :: derive
value :: Point = { x = 3; flags = {true; false;}; note = #some ("browser"); mode = #prod; choice = #count { value = 9; }; };
result :: Validation DecodeIssue Point = decode (encode value);
result
"#;

#[test]
fn derived_data_encoding_round_trips_in_portable_bundle() {
    let mut bundle = fixture::bundle();
    bundle.entry = "encode.zt".to_string();
    bundle
        .sources
        .insert(bundle.entry.clone(), DATA_ENCODING_SOURCE.to_string());
    let json = serde_json::to_string(&bundle).expect("bundle serializes");
    let bundle: WebBundleV3 = serde_json::from_str(&json).expect("bundle deserializes");
    let stdlib =
        StdlibSources::from_memory(bundle.stdlib_compiler_compatibility, bundle.stdlib_sources)
            .expect("embedded stdlib subset is well-formed");
    let analysis = zutai_semantic::analyze_sources_with_stdlib_and_packages(
        &bundle.entry,
        &bundle.sources,
        AnalysisOptions::default(),
        &stdlib,
        bundle.packages,
    )
    .expect("encoded program analyzes against the embedded stdlib subset");
    let session = TlcSession::from_analysis(&analysis).expect("encoded program type-checks");
    let value = session.entry().expect("encoded program has an entry value");
    let zutai_eval::Value::TaggedValue { tag, payload } = value else {
        panic!("expected validation result");
    };
    assert_eq!(tag.as_ref(), "valid");
    let decoded = payload[0].1.peek().expect("valid payload is forced");
    let zutai_eval::Value::Record(fields) = decoded else {
        panic!("expected decoded point");
    };
    assert_eq!(
        fields
            .iter()
            .find(|(name, _)| name.as_ref() == "x")
            .and_then(|(_, value)| value.peek()),
        Some(zutai_eval::Value::Int(3))
    );
}

#[test]
fn keyed_list_program_round_trips_through_the_bundle_and_analyzes() {
    let bundle = fixture::bundle();
    let json = serde_json::to_string(&bundle).expect("bundle serializes");
    let bundle: WebBundleV3 = serde_json::from_str(&json).expect("bundle deserializes");
    bundle.validate_version().expect("bundle version matches");

    let stdlib =
        StdlibSources::from_memory(bundle.stdlib_compiler_compatibility, bundle.stdlib_sources)
            .expect("embedded stdlib subset is well-formed");
    let analysis = zutai_semantic::analyze_sources_with_stdlib_and_packages(
        &bundle.entry,
        &bundle.sources,
        AnalysisOptions::default(),
        &stdlib,
        bundle.packages,
    )
    .expect("program analyzes against the embedded stdlib subset");
    let session = TlcSession::from_analysis(&analysis).expect("analysis type-checks");
    let entry = session.entry().expect("module has an entry value");
    let program = decode_program(&session, entry).expect("entry decodes as a browser program");

    let model = program
        .initialize(&session, &fixture::RejectEffects)
        .expect("init runs without effects");
    let document = program
        .render(&session, model.clone())
        .expect("initial render succeeds");

    assert_eq!(document.title, "Test");
    let sheet = document
        .head
        .iter()
        .find_map(|node| match node {
            HeadNode::Style(sheet) => Some(sheet),
            _ => None,
        })
        .expect("typed stylesheet renders in the document head");
    assert_eq!(
        render_stylesheet(sheet, false).expect("structured stylesheet renders safely"),
        "#status{font-weight:700;color:#0f7285;}@media (prefers-reduced-motion:reduce){*{transition:none;}}"
    );
    let status = fixture::find_by_id(&document.body, "status").expect("status span renders");
    assert_eq!(fixture::element_text(status), "off");
    let list = fixture::find_by_id(&document.body, "list").expect("list renders");
    assert_eq!(list.children.len(), 1, "seeded item should render once");
    let Html::Element(seed_item) = &list.children[0] else {
        panic!("list child should be an element");
    };
    assert_eq!(seed_item.key.as_deref(), Some("seed"));

    let toggle_button =
        fixture::find_by_id(&document.body, "toggle-btn").expect("toggle button renders");
    let toggle_message = fixture::click_message(toggle_button);
    let toggled_model = program
        .transition(&session, toggle_message, model, &fixture::RejectEffects)
        .expect("toggle transition runs without effects");
    let toggled_document = program
        .render(&session, toggled_model.clone())
        .expect("re-render after toggle succeeds");
    let status =
        fixture::find_by_id(&toggled_document.body, "status").expect("status span still renders");
    assert_eq!(fixture::element_text(status), "on");

    let remove_button = seed_item
        .children
        .iter()
        .find_map(|child| match child {
            Html::Element(element) if element.tag == "button" => Some(element),
            _ => None,
        })
        .expect("seeded item has a remove button");
    let remove_message = fixture::click_message(remove_button);
    let emptied_model = program
        .transition(
            &session,
            remove_message,
            toggled_model,
            &fixture::RejectEffects,
        )
        .expect("remove transition runs without effects");
    let emptied_document = program
        .render(&session, emptied_model)
        .expect("re-render after removal succeeds");
    let list = fixture::find_by_id(&emptied_document.body, "list").expect("list still renders");
    assert!(
        list.children.is_empty(),
        "removing the only item should empty the list"
    );
}
