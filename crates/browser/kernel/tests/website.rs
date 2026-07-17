// `zutai_semantic::analyze_path` reads both the entry file and the stdlib
// from disk, so this test is native-only (see tests/fixture/mod.rs).
#![cfg(not(target_arch = "wasm32"))]

mod fixture;

use std::cell::RefCell;
use std::rc::Rc;

use zutai_browser::{
    Attribute, BrowserProgram, Document, Element, EventHandler, Html, ListOp, StaticAttribute,
    decode_program, diff_children, prerender_document,
};
use zutai_eval::{EffectHandler, EvalError, TlcSession, Value};
use zutai_semantic::{AnalysisOptions, StdlibSources};

#[derive(Default)]
struct RecordFocus {
    ops: RefCell<Vec<String>>,
}

impl EffectHandler for RecordFocus {
    fn handle(&self, operation: &str, _argument: Value) -> Result<Value, EvalError> {
        self.ops.borrow_mut().push(operation.to_string());
        match operation {
            "browser.focus" => Ok(Value::Tuple(Rc::from([]))),
            _ => Err(EvalError::UnhandledEffect(operation.to_string())),
        }
    }
}

struct RejectEffects;

impl EffectHandler for RejectEffects {
    fn handle(&self, operation: &str, _argument: Value) -> Result<Value, EvalError> {
        panic!("unexpected effect during website initialization: {operation}")
    }
}

fn visit_html(nodes: &[Html], elements: &mut usize, events: &mut usize) {
    for node in nodes {
        let Html::Element(element) = node else {
            continue;
        };
        *elements += 1;
        *events += element
            .attributes
            .iter()
            .filter(|attribute| matches!(attribute, Attribute::Event(_)))
            .count();
        visit_html(&element.children, elements, events);
    }
}

fn load_website_program() -> (TlcSession, BrowserProgram, Value) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("browser crate lives under crates/browser/kernel");
    let analysis = zutai_semantic::analyze_path(&root.join("website/main.zt")).unwrap();
    let session = TlcSession::from_analysis(&analysis).unwrap();
    let program = decode_program(&session, session.entry().unwrap()).unwrap();
    let model = program.initialize(&session, &RejectEffects).unwrap();
    (session, program, model)
}

fn load_bundled_website_program() -> (TlcSession, BrowserProgram, Value) {
    let bundle = fixture::website_bundle();
    let json = serde_json::to_string(&bundle).expect("website bundle serializes");
    let bundle: zutai_browser::WebBundleV3 =
        serde_json::from_str(&json).expect("website bundle deserializes");
    bundle
        .validate_version()
        .expect("website bundle version matches");
    let stdlib =
        StdlibSources::from_memory(bundle.stdlib_compiler_compatibility, bundle.stdlib_sources)
            .expect("embedded website stdlib subset is well-formed");
    let analysis = zutai_semantic::analyze_sources_with_stdlib_and_packages(
        &bundle.entry,
        &bundle.sources,
        AnalysisOptions::default(),
        &stdlib,
        bundle.packages,
    )
    .expect("website analyzes from its portable package graph");
    let session = TlcSession::from_analysis(&analysis).expect("bundled website type-checks");
    let program = decode_program(
        &session,
        session.entry().expect("website has an entry value"),
    )
    .expect("bundled website decodes as a browser program");
    let model = program
        .initialize(&session, &RejectEffects)
        .expect("bundled website initializes without effects");
    (session, program, model)
}

fn find_by_id<'a>(nodes: &'a [Html], id: &str) -> Option<&'a Element> {
    for node in nodes {
        let Html::Element(element) = node else {
            continue;
        };
        let has_id = element.attributes.iter().any(|attribute| {
            matches!(
                attribute,
                Attribute::Static(StaticAttribute::Text { name, value })
                    if name == "id" && value == id
            )
        });
        if has_id {
            return Some(element);
        }
        if let Some(found) = find_by_id(&element.children, id) {
            return Some(found);
        }
    }
    None
}

fn element_text(element: &Element) -> String {
    element
        .children
        .iter()
        .filter_map(|child| match child {
            Html::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn click_message(element: &Element) -> Value {
    element
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            Attribute::Event(EventHandler::Click { message, .. }) => Some(message.clone()),
            _ => None,
        })
        .expect("element has a click handler")
}

fn input_message(element: &Element) -> Value {
    element
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            Attribute::Event(EventHandler::Input { to_message, .. }) => Some(to_message.clone()),
            _ => None,
        })
        .expect("element has an input handler")
}

fn element_value(element: &Element, name: &str) -> Option<String> {
    element
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            Attribute::Static(StaticAttribute::Text {
                name: attr_name,
                value,
            }) if attr_name == name => Some(value.to_string()),
            Attribute::TextProperty {
                name: attr_name,
                value,
            } if attr_name == name => Some(value.to_string()),
            _ => None,
        })
}

fn has_class(element: &Element, class_name: &str) -> bool {
    element.attributes.iter().any(|attribute| {
        matches!(
            attribute,
            Attribute::Static(StaticAttribute::Text { name, value })
                if name == "class" && value.split_whitespace().any(|class| class == class_name)
        )
    })
}

fn find_by_class<'a>(nodes: &'a [Html], class_name: &str) -> Option<&'a Element> {
    for node in nodes {
        let Html::Element(element) = node else {
            continue;
        };
        if has_class(element, class_name) {
            return Some(element);
        }
        if let Some(found) = find_by_class(&element.children, class_name) {
            return Some(found);
        }
    }
    None
}

fn render_demo_roster_keys(
    session: &TlcSession,
    program: &BrowserProgram,
    model: &Value,
) -> Vec<String> {
    let document = program.render(session, model.clone()).unwrap();
    let roster =
        find_by_id(&document.body, "demo-roster").expect("service roster renders for the demo");
    roster
        .children
        .iter()
        .filter_map(|child| match child {
            Html::Element(element) => element.key.clone(),
            _ => None,
        })
        .collect()
}

fn read_demo_counts(
    session: &TlcSession,
    program: &BrowserProgram,
    model: &Value,
) -> (String, String) {
    let document = program.render(session, model.clone()).unwrap();
    let ready_count = element_text(
        find_by_id(&document.body, "demo-ready-count").expect("ready count renders in demo"),
    );
    let total_count = element_text(
        find_by_id(&document.body, "demo-total-count").expect("total count renders in demo"),
    );
    (ready_count, total_count)
}

fn roster_row<'a>(document: &'a Document, key: &str) -> Option<&'a Element> {
    let roster = find_by_id(&document.body, "demo-roster")?;
    roster.children.iter().find_map(|child| match child {
        Html::Element(row) if row.key.as_deref() == Some(key) => Some(row),
        _ => None,
    })
}

fn roster_status_text(row: &Element) -> String {
    element_text(
        find_by_class(&row.children, "roster-status")
            .expect("status text container renders in service row"),
    )
}

#[test]
fn self_hosted_website_decodes_and_prerenders_through_the_browser_contract() {
    let (session, program, model) = load_website_program();
    let document = program.render(&session, model).unwrap();

    assert_eq!(document.language, "en");
    assert!(document.title.contains("Zutai"));
    assert!(!document.head.is_empty());

    let mut elements = 0;
    let mut events = 0;
    visit_html(&document.body, &mut elements, &mut events);
    assert!(
        elements >= 40,
        "expected a substantial website, found {elements} elements"
    );
    assert!(
        events > 0,
        "expected interactive controls, found {events} events"
    );

    let page = prerender_document(&document, "/_zutai/test/bootstrap.js").unwrap();
    assert!(page.html.starts_with("<!doctype html><html lang=\"en\">"));
    assert!(page.html.contains("data-zutai-bootstrap"));
    assert!(page.html.contains("Zutai"));
}

#[test]
fn self_hosted_website_portable_package_bundle_matches_native_entry() {
    let (native_session, native_program, native_model) = load_website_program();
    let (bundle_session, bundle_program, bundle_model) = load_bundled_website_program();

    let native_document = native_program
        .render(&native_session, native_model)
        .expect("native website render succeeds");
    let bundled_document = bundle_program
        .render(&bundle_session, bundle_model.clone())
        .expect("bundled website render succeeds");
    let native_page = prerender_document(&native_document, "/_zutai/test/bootstrap.js")
        .expect("native website prerenders");
    let bundled_page = prerender_document(&bundled_document, "/_zutai/test/bootstrap.js")
        .expect("bundled website prerenders");
    assert_eq!(bundled_page.html, native_page.html);

    let toggle_button = find_by_id(&bundled_document.body, "toggle-payments-edge")
        .expect("package-backed toggle button renders");
    let toggled_model = bundle_program
        .transition(
            &bundle_session,
            click_message(toggle_button),
            bundle_model,
            &RejectEffects,
        )
        .expect("package-backed transition succeeds");
    let keys = render_demo_roster_keys(&bundle_session, &bundle_program, &toggled_model);
    assert_eq!(
        keys,
        vec![
            "auth-core",
            "search-index",
            "batch-archiver",
            "payments-edge"
        ]
    );
    let toggled_document = bundle_program
        .render(&bundle_session, toggled_model)
        .expect("package-backed re-render succeeds");
    let row = roster_row(&toggled_document, "payments-edge")
        .expect("package-backed service row still renders");
    assert!(has_class(row, "is-paused"));
}

#[test]
fn self_hosted_website_initial_rollup_and_demo_sort_order() {
    let (session, program, model) = load_website_program();
    let (ready_count, total_count) = read_demo_counts(&session, &program, &model);
    assert_eq!(ready_count, "2");
    assert_eq!(total_count, "4");

    let keys = render_demo_roster_keys(&session, &program, &model);
    assert_eq!(
        keys,
        vec![
            "auth-core",
            "payments-edge",
            "search-index",
            "batch-archiver"
        ]
    );
}

#[test]
fn self_hosted_website_add_service_recomputes_rollup_and_records_focus() {
    let (session, program, model) = load_website_program();
    let document = program.render(&session, model.clone()).unwrap();

    let draft_input = find_by_id(&document.body, "demo-draft").expect("demo draft input renders");
    let set_draft = session
        .apply(
            input_message(draft_input),
            Value::Text(Rc::from("cache-warm")),
        )
        .unwrap();
    let add_button = find_by_id(&document.body, "demo-add").expect("demo add button renders");
    let add_message = click_message(add_button);

    let focused_model = program
        .transition(&session, set_draft, model, &RejectEffects)
        .expect("set draft transition succeeds");
    let handler = RecordFocus::default();
    let added_model = program
        .transition(&session, add_message, focused_model, &handler)
        .expect("add service transition succeeds");

    let (ready_count, total_count) = read_demo_counts(&session, &program, &added_model);
    assert_eq!(ready_count, "3");
    assert_eq!(total_count, "5");
    assert_eq!(&*handler.ops.borrow(), &[String::from("browser.focus")]);
    let keys = render_demo_roster_keys(&session, &program, &added_model);
    assert_eq!(
        keys,
        vec![
            "auth-core",
            "cache-warm",
            "payments-edge",
            "search-index",
            "batch-archiver"
        ]
    );
}

#[test]
fn self_hosted_website_toggle_service_reclassifies_and_reorders() {
    let (session, program, model) = load_website_program();
    let initial_document = program.render(&session, model.clone()).unwrap();
    let initial_roster =
        find_by_id(&initial_document.body, "demo-roster").expect("initial demo roster renders");
    let initial_children = initial_roster.children.clone();

    let toggle_button = find_by_id(&initial_document.body, "toggle-payments-edge")
        .expect("toggle payments button renders");
    let toggle_message = click_message(toggle_button);

    let model = program
        .transition(&session, toggle_message, model, &RejectEffects)
        .expect("toggle transition succeeds");
    let paused_document = program.render(&session, model.clone()).unwrap();
    let ready_count = element_text(
        find_by_id(&paused_document.body, "demo-ready-count").expect("ready count renders in demo"),
    );
    assert_eq!(ready_count, "1");

    let paused_roster =
        find_by_id(&paused_document.body, "demo-roster").expect("demo roster renders after pause");
    let diff = diff_children(&initial_children, &paused_roster.children);
    assert!(
        diff.ops.iter().any(|op| matches!(op, ListOp::Move { .. })),
        "expected a keyed move when pause toggles order"
    );
    assert!(
        diff.removed_old_indices.is_empty(),
        "pause should reorder without removing rows"
    );

    let keys: Vec<String> = paused_roster
        .children
        .iter()
        .filter_map(|child| match child {
            Html::Element(row) => row.key.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(
        keys,
        vec![
            "auth-core",
            "search-index",
            "batch-archiver",
            "payments-edge"
        ]
    );

    let row = roster_row(&paused_document, "payments-edge").expect("payments-edge row renders");
    assert!(has_class(row, "is-paused"));
}

#[test]
fn self_hosted_website_log_error_moves_service_to_erroring() {
    let (session, program, model) = load_website_program();
    let document = program.render(&session, model.clone()).unwrap();
    let error_button =
        find_by_id(&document.body, "error-auth-core").expect("auth-core error button renders");
    let error_message = click_message(error_button);

    let model = program
        .transition(&session, error_message, model, &RejectEffects)
        .expect("log error transition succeeds");
    let (ready_count, _) = read_demo_counts(&session, &program, &model);
    assert_eq!(ready_count, "1");

    let document = program.render(&session, model).unwrap();
    let row = roster_row(&document, "auth-core").expect("auth-core row renders");
    assert!(has_class(row, "is-erroring"));
    let status = roster_status_text(row);
    assert!(status.contains("erroring"));
    assert!(status.contains("1"));
}

#[test]
fn self_hosted_website_clear_errors_restores_readiness() {
    let (session, program, model) = load_website_program();
    let document = program.render(&session, model.clone()).unwrap();
    let clear_button = find_by_id(&document.body, "clear-search-index")
        .expect("search-index clear-error button renders");
    let clear_message = click_message(clear_button);

    let model = program
        .transition(&session, clear_message, model, &RejectEffects)
        .expect("clear errors transition succeeds");
    let (ready_count, _) = read_demo_counts(&session, &program, &model);
    assert_eq!(ready_count, "3");

    let document = program.render(&session, model).unwrap();
    let row = roster_row(&document, "search-index").expect("search-index row renders");
    assert!(has_class(row, "is-ready"));
}

#[test]
fn self_hosted_website_remove_service_updates_totals_and_keys() {
    let (session, program, model) = load_website_program();
    let document = program.render(&session, model.clone()).unwrap();
    let remove_button = find_by_id(&document.body, "remove-batch-archiver")
        .expect("remove batch-archiver button renders");
    let remove_message = click_message(remove_button);

    let model = program
        .transition(&session, remove_message, model, &RejectEffects)
        .expect("remove transition succeeds");
    let (ready_count, total_count) = read_demo_counts(&session, &program, &model);
    assert_eq!(total_count, "3");
    assert_eq!(ready_count, "2");

    let keys = render_demo_roster_keys(&session, &program, &model);
    assert_eq!(keys, vec!["auth-core", "payments-edge", "search-index"]);
}

#[test]
fn self_hosted_website_draft_round_trips_and_clears_on_add() {
    let (session, program, model) = load_website_program();
    let document = program.render(&session, model.clone()).unwrap();
    let draft_input = find_by_id(&document.body, "demo-draft").expect("demo draft input renders");
    let set_draft = session
        .apply(input_message(draft_input), Value::Text(Rc::from("abc")))
        .unwrap();

    let model = program
        .transition(&session, set_draft, model, &RejectEffects)
        .expect("set draft transition succeeds");
    let drafted_document = program.render(&session, model.clone()).unwrap();
    let drafted_input = find_by_id(&drafted_document.body, "demo-draft")
        .expect("demo draft input renders after editing");
    assert_eq!(
        element_value(drafted_input, "value")
            .as_deref()
            .unwrap_or(""),
        "abc"
    );

    let add_button = find_by_id(&drafted_document.body, "demo-add")
        .expect("demo add button renders after editing");
    let add_message = click_message(add_button);
    let handler = RecordFocus::default();
    let model = program
        .transition(&session, add_message, model, &handler)
        .expect("add transition succeeds");
    let added_document = program.render(&session, model).unwrap();
    let post_add_input =
        find_by_id(&added_document.body, "demo-draft").expect("draft input renders after add");
    assert_eq!(
        element_value(post_add_input, "value")
            .as_deref()
            .unwrap_or(""),
        ""
    );
}
