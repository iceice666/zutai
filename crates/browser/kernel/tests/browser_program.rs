//! Validates the Zutai program and stdlib subset used by the wasm-bindgen-test
//! browser harness (`browser_hydration.rs`, wasm32-only) — everything
//! `start()` does up to the DOM patch runs on the host, so this catches any
//! syntax/type mistake in the shared fixture without needing a browser.
#![cfg(not(target_arch = "wasm32"))]

mod fixture;

use zutai_browser::{Html, WebBundleV3, decode_program};
use zutai_eval::TlcSession;
use zutai_semantic::{AnalysisOptions, StdlibSources};

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
