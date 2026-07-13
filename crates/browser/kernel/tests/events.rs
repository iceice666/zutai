use zutai_browser::{Attribute, EventHandler, Html, decode_document};
use zutai_eval::TlcSession;

const SOURCE: &str = "
html ::= import stdlib.html;

Msg :: type {
  #onChangeMsg : { value : Text; };
  #onSubmitMsg;
  #onBlurMsg;
  #onFocusMsg;
  #onKeyDownMsg : { key : Text; };
  #onKeyUpMsg : { key : Text; };
};

toChangeMsg :: Text -> Msg = value => #onChangeMsg { value = value; };
toKeyDownMsg :: Text -> Msg = key => #onKeyDownMsg { key = key; };
toKeyUpMsg :: Text -> Msg = key => #onKeyUpMsg { key = key; };
onSubmitMsg :: Msg = #onSubmitMsg;
onBlurMsg :: Msg = #onBlurMsg;
onFocusMsg :: Msg = #onFocusMsg;

html.document \"en\" \"Events\" {;} {;} {
  html.input { html.onChange toChangeMsg; };
  html.form { html.onSubmit onSubmitMsg; } {;};
  html.input { html.onBlur onBlurMsg; };
  html.input { html.onFocus onFocusMsg; };
  html.input { html.onKeyDown toKeyDownMsg; };
  html.input { html.onKeyUp toKeyUpMsg; };
}
";

fn event_handlers(nodes: &[Html]) -> Vec<&EventHandler> {
    nodes
        .iter()
        .filter_map(|node| match node {
            Html::Element(element) => Some(element),
            Html::Text(_) => None,
        })
        .flat_map(|element| {
            element
                .attributes
                .iter()
                .filter_map(|attribute| match attribute {
                    Attribute::Event(handler) => Some(handler),
                    _ => None,
                })
        })
        .collect()
}

#[test]
fn decodes_change_submit_blur_focus_and_key_events() {
    let analysis = zutai_semantic::analyze(SOURCE);
    let session = TlcSession::from_analysis(&analysis).expect("analysis should type-check");
    let entry = session.entry().expect("module should have an entry value");
    let document = decode_document(&session, entry).expect("document should decode");

    let handlers = event_handlers(&document.body);
    assert_eq!(handlers.len(), 6, "expected one event handler per element");

    assert!(matches!(handlers[0], EventHandler::Change { .. }));
    assert!(matches!(handlers[1], EventHandler::Submit { .. }));
    assert!(matches!(handlers[2], EventHandler::Blur { .. }));
    assert!(matches!(handlers[3], EventHandler::Focus { .. }));
    assert!(matches!(handlers[4], EventHandler::KeyDown { .. }));
    assert!(matches!(handlers[5], EventHandler::KeyUp { .. }));
}
