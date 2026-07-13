//! End-to-end browser coverage for the steady-state reconciler
//! (milestones 1-4 of the browser-kernel roadmap item) that the pure
//! `diff.rs` unit tests structurally cannot reach: hydration through the
//! real `start()` entry point, DOM node *identity* across a patch (proving
//! `Keep` really means "never touched", not just "ends up correct"), and
//! focus/selection restore.
//!
//! Needs a headless browser + WebDriver on `PATH` (e.g. Chromium +
//! chromedriver). Run with:
//!
//! ```text
//! cargo test --target wasm32-unknown-unknown -p zutai-browser --test browser_hydration
//! ```
//!
//! Scoped to `--test browser_hydration` deliberately: this is the only test
//! binary in the crate written against `wasm-bindgen-test`, and it shares
//! one browser page/DOM across its tests, which is why everything below is
//! one `#[wasm_bindgen_test]` walking a single scenario end to end rather
//! than several independent tests that could stomp on each other's `<body>`.
#![cfg(target_arch = "wasm32")]

mod fixture;

use wasm_bindgen::JsCast;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use web_sys::{Element, Event, HtmlElement, HtmlInputElement, Node};

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window()
        .expect("browser window")
        .document()
        .expect("browser document")
}

fn query(selector: &str) -> Element {
    document()
        .query_selector(selector)
        .expect("query_selector does not throw")
        .unwrap_or_else(|| panic!("missing element for selector `{selector}`"))
}

fn click(selector: &str) {
    let element: HtmlElement = query(selector).dyn_into().expect("clickable element");
    element.click();
}

fn type_into(selector: &str, value: &str) {
    let input: HtmlInputElement = query(selector).dyn_into().expect("input element");
    input.set_value(value);
    let event = Event::new("input").expect("input event constructs");
    input
        .dyn_ref::<web_sys::EventTarget>()
        .expect("input is an event target")
        .dispatch_event(&event)
        .expect("dispatch_event does not throw");
}

fn same_node(a: &Node, b: &Node) -> bool {
    a.is_same_node(Some(b))
}

#[wasm_bindgen_test]
fn hydrates_reconciles_keyed_list_and_restores_focus() {
    let bundle_json = serde_json::to_string(&fixture::bundle()).expect("bundle serializes");
    zutai_browser::start(&bundle_json, false).expect("start hydrates the document");

    // --- Hydration ---
    assert_eq!(query("#status").text_content().as_deref(), Some("off"));
    assert_eq!(
        document()
            .query_selector_all("#list li")
            .expect("query_selector_all does not throw")
            .length(),
        1,
        "the seeded item should hydrate"
    );
    let seed_node: Node = query("li[data-zutai-key='seed']").into();

    // --- Keyed-list add: the reconciler must reuse the existing node ---
    type_into("#draft-input", "new-item");
    click("#add-btn");
    assert_eq!(
        document()
            .query_selector_all("#list li")
            .expect("query_selector_all does not throw")
            .length(),
        2,
        "adding an item should grow the list"
    );
    let seed_node_after_add: Node = query("li[data-zutai-key='seed']").into();
    assert!(
        same_node(&seed_node, &seed_node_after_add),
        "adding a sibling must not recreate the existing keyed node"
    );
    let draft_input: HtmlInputElement = query("#draft-input").dyn_into().unwrap();
    assert_eq!(draft_input.value(), "", "draft should clear after add");

    // --- Unrelated re-render (toggle) must not disturb list items or head ---
    let new_item_node: Node = query("li[data-zutai-key='new-item']").into();
    let description_node: Node = document()
        .query_selector("meta[name='description']")
        .expect("query_selector does not throw")
        .expect("description meta renders")
        .into();
    click("#toggle-btn");
    assert_eq!(query("#status").text_content().as_deref(), Some("on"));
    assert!(
        same_node(
            &seed_node_after_add,
            &query("li[data-zutai-key='seed']").into()
        ),
        "an unrelated toggle must not recreate the seed item"
    );
    assert!(
        same_node(
            &new_item_node,
            &query("li[data-zutai-key='new-item']").into()
        ),
        "an unrelated toggle must not recreate the new item"
    );
    let description_node_after_toggle: Node = document()
        .query_selector("meta[name='description']")
        .expect("query_selector does not throw")
        .expect("description meta still renders")
        .into();
    assert!(
        same_node(&description_node, &description_node_after_toggle),
        "unchanged head content must not be torn down and rebuilt"
    );

    // --- Keyed-list removal: the surviving sibling must keep its node ---
    click("li[data-zutai-key='seed'] button");
    assert_eq!(
        document()
            .query_selector_all("#list li")
            .expect("query_selector_all does not throw")
            .length(),
        1,
        "removing an item should shrink the list"
    );
    assert!(
        document()
            .query_selector("li[data-zutai-key='seed']")
            .expect("query_selector does not throw")
            .is_none(),
        "the removed item should be gone"
    );
    assert!(
        same_node(
            &new_item_node,
            &query("li[data-zutai-key='new-item']").into()
        ),
        "removing a sibling must not recreate the surviving node"
    );

    // --- Focus/selection restore across an unrelated re-render ---
    let draft_input: HtmlInputElement = query("#draft-input").dyn_into().unwrap();
    draft_input.focus().expect("focus does not throw");
    type_into("#draft-input", "partial text");
    draft_input
        .set_selection_range(2, 5)
        .expect("set_selection_range does not throw");
    click("#toggle-btn");
    let active = document()
        .active_element()
        .expect("something should be focused after the patch");
    assert!(
        same_node(draft_input.as_ref(), active.as_ref()),
        "focus should be restored to the draft input across the patch"
    );
    let active_input: HtmlInputElement = active.dyn_into().expect("draft input stays an input");
    assert_eq!(active_input.selection_start().ok().flatten(), Some(2));
    assert_eq!(active_input.selection_end().ok().flatten(), Some(5));
}
