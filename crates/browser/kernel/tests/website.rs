use zutai_browser::{Attribute, Html, decode_program, prerender_document};
use zutai_eval::{EffectHandler, EvalError, TlcSession, Value};

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

#[test]
fn self_hosted_website_decodes_and_prerenders_through_the_browser_contract() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("browser crate lives under crates/browser/kernel");
    let analysis = zutai_semantic::analyze_path(&root.join("website/main.zt")).unwrap();
    let session = TlcSession::from_analysis(&analysis).unwrap();
    let program = decode_program(&session, session.entry().unwrap()).unwrap();
    let model = program.initialize(&session, &RejectEffects).unwrap();
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
        events >= 4,
        "expected interactive controls, found {events} events"
    );

    let page = prerender_document(&document, "/_zutai/test/bootstrap.js").unwrap();
    assert!(page.html.starts_with("<!doctype html><html lang=\"en\">"));
    assert!(page.html.contains("data-zutai-bootstrap"));
    assert!(page.html.contains("Zutai"));
}
