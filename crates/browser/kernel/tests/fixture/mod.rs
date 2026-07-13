//! Shared, target-agnostic fixture for the browser-kernel tests that need a
//! real (small) Zutai program rather than hand-built `Html`/`Document`
//! values: `../browser_program.rs` (native — validates the program analyzes,
//! decodes, and runs correctly) and `../browser_hydration.rs` (wasm32-only —
//! drives the same program through the real `start()` entry point in a
//! browser). Keeping this in one place means a change to the program only
//! needs to stay consistent with `decode.rs`'s contract once, not twice.
#![allow(dead_code)]

use std::collections::BTreeMap;

use zutai_browser::{Attribute, Element, EventHandler, Html, StaticAttribute, WebBundleV3};
use zutai_eval::{EffectHandler, EvalError, Value};

const STREAM_ZT: &str = include_str!("../../../../../stdlib/packages/base/modules/stream.zt");
const PRELUDE_ZT: &str = include_str!("../../../../../stdlib/packages/base/modules/prelude.zt");
const HTML_ZT: &str = include_str!("../../../../../stdlib/packages/web/modules/html.zt");
const CSS_ZT: &str = include_str!("../../../../../stdlib/packages/web/modules/css.zt");

/// A small model/update/view program: a toggleable status, a keyed list of
/// text items (add via a draft `<input>`, remove per-item), covering
/// hydration, keyed-list add/remove, `value` live-sync, and (via the
/// `description` head entry) head-node identity in one program.
pub const SOURCE: &str = r#"
html ::= import stdlib.html;

Model :: type {
  active : Bool;
  items : List Text;
  draft : Text;
};

Msg :: type {
  #toggle;
  #setDraft : { value : Text; };
  #addItem;
  #removeItem : { key : Text; };
};

toggleMsg :: Msg = #toggle;
addItemMsg :: Msg = #addItem;
setDraftMsg :: Text -> Msg = value => #setDraft { value = value; };
removeItemMsg :: Text -> Msg = key => #removeItem { key = key; };

init :: Unit -> Model
  = _ => { active = false; items = {"seed";}; draft = ""; };

update :: Msg -> Model -> Model
  = message model => match message {
    | #toggle => model with { active = not model.active; };
    | #setDraft { value = v; } => model with { draft = v; };
    | #addItem => model with { items = append model.items {model.draft;}; draft = ""; };
    | #removeItem { key = k; } => model with { items = filter (\item. not (item == k)) model.items; };
  };

listItem :: Text -> html.Html Msg
  = itemText => html.li { html.key itemText; } {
      html.text itemText;
      html.button { html.onClick (removeItemMsg itemText); } { html.text "remove"; };
    };

view :: Model -> html.Document Msg
  = model =>
    (html.document
      "en"
      "Test"
      { html.description "test app"; }
      {;}
      {
        html.span { html.idAttr "status"; } { html.text (if model.active then "on" else "off"); };
        html.button { html.idAttr "toggle-btn"; html.onClick toggleMsg; } { html.text "toggle"; };
        html.input { html.idAttr "draft-input"; html.value model.draft; html.onInput setDraftMsg; };
        html.button { html.idAttr "add-btn"; html.onClick addItemMsg; } { html.text "add"; };
        html.ul { html.idAttr "list"; } (map listItem model.items);
      });

{ init = init; update = update; view = view; }
"#;

/// A portable `WebBundleV3` for `SOURCE`, with only the stdlib modules it
/// actually imports (transitively: `html` -> `css`, plus the always-required
/// `stream`/`prelude` ambient preludes) embedded via `include_str!` — no
/// filesystem access, so this also works compiled to wasm32.
pub fn bundle() -> WebBundleV3 {
    let mut sources = BTreeMap::new();
    sources.insert("main.zt".to_string(), SOURCE.to_string());
    let mut stdlib_sources = BTreeMap::new();
    stdlib_sources.insert("stream".to_string(), STREAM_ZT.to_string());
    stdlib_sources.insert("prelude".to_string(), PRELUDE_ZT.to_string());
    stdlib_sources.insert("html".to_string(), HTML_ZT.to_string());
    stdlib_sources.insert("css".to_string(), CSS_ZT.to_string());
    WebBundleV3::new(
        "main.zt".to_string(),
        sources,
        zutai_semantic::STDLIB_COMPILER_COMPATIBILITY.to_string(),
        stdlib_sources,
        Default::default(),
    )
}

pub struct RejectEffects;

impl EffectHandler for RejectEffects {
    fn handle(&self, operation: &str, _argument: Value) -> Result<Value, EvalError> {
        panic!("unexpected effect during test: {operation}")
    }
}

pub fn find_by_id<'a>(nodes: &'a [Html], id: &str) -> Option<&'a Element> {
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

pub fn click_message(element: &Element) -> Value {
    element
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            Attribute::Event(EventHandler::Click { message, .. }) => Some(message.clone()),
            _ => None,
        })
        .expect("element has a click handler")
}

pub fn element_text(element: &Element) -> String {
    element
        .children
        .iter()
        .filter_map(|child| match child {
            Html::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
