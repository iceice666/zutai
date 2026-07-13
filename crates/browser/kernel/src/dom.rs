use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use web_sys::{Document as WebDocument, Element as WebElement, Event, EventTarget, Node};

use zutai_eval::{EffectHandler, EvalError, TlcSession, Value};
use zutai_semantic::AnalysisOptions;

use crate::css::{render_declarations, render_stylesheet};
use crate::diff::{
    AttributeDiff, AttributeEffect, ChildOp, child_key, diff_children, diff_element_attributes,
    diff_static_attributes,
};
use crate::render::validate_document;
use crate::{
    Attribute, BrowserProgram, Document, Element, EventHandler, EventOptions, HeadNode, Html,
    StaticAttribute, WebBundleV3, decode_program, is_void_element,
};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct App {
    session: TlcSession,
    program: BrowserProgram,
    model: Value,
    rendered: Document,
    listeners: Vec<Listener>,
    generation: u64,
    development: bool,
}

struct Listener {
    target: EventTarget,
    event_name: &'static str,
    callback: Closure<dyn FnMut(Event)>,
}

#[derive(Clone)]
enum PendingHandler {
    Click(Value, EventOptions),
    Input(Value, EventOptions),
    Change(Value, EventOptions),
    Submit(Value, EventOptions),
    Blur(Value, EventOptions),
    Focus(Value, EventOptions),
    KeyDown(Value, EventOptions),
    KeyUp(Value, EventOptions),
}

#[derive(Default)]
struct BrowserEffects {
    focus: RefCell<Vec<String>>,
}

impl EffectHandler for BrowserEffects {
    fn handle(&self, operation: &str, argument: Value) -> Result<Value, EvalError> {
        if operation != "browser.focus" {
            return Err(EvalError::EffectfulNotExecutable(format!(
                "host effect `{operation}` is unavailable in the browser"
            )));
        }
        let Value::Text(element_id) = argument else {
            return Err(EvalError::TypeMismatch {
                expected: "Text",
                found: value_kind(&argument),
            });
        };
        self.focus.borrow_mut().push(element_id.to_string());
        Ok(Value::Tuple(Rc::from([])))
    }
}

/// Start the single whole-document Zutai application contained in `bundle_json`.
///
/// The generated bootstrap catches errors from this function and intentionally
/// leaves the build-time prerendered document in place.
#[wasm_bindgen]
pub fn start(bundle_json: &str, development: bool) -> Result<(), JsValue> {
    console_error_panic_hook();

    let bundle: WebBundleV3 = serde_json::from_str(bundle_json).map_err(js_error)?;
    bundle.validate_version().map_err(js_error)?;
    let stdlib = zutai_semantic::StdlibSources::from_memory(
        bundle.stdlib_compiler_compatibility,
        bundle.stdlib_sources,
    )
    .map_err(js_error)?;
    let analysis = zutai_semantic::analyze_sources_with_stdlib_and_packages(
        &bundle.entry,
        &bundle.sources,
        AnalysisOptions::default(),
        &stdlib,
        bundle.packages.clone(),
    )
    .map_err(js_error)?;
    let session = TlcSession::from_analysis(&analysis).map_err(js_error)?;
    let entry = session.entry().map_err(js_error)?;
    let program = decode_program(&session, entry).map_err(js_error)?;
    let effects = BrowserEffects::default();
    let model = program.initialize(&session, &effects).map_err(js_error)?;
    let rendered = program.render(&session, model.clone()).map_err(js_error)?;
    validate_document(&rendered).map_err(js_error)?;

    let document = browser_document()?;
    patch_document(&document, &rendered)?;
    let generation = 1;
    let listeners = attach_listeners(&document, &rendered, generation)?;

    APP.with(|slot| {
        *slot.borrow_mut() = Some(App {
            session,
            program,
            model,
            rendered,
            listeners,
            generation,
            development,
        });
    });

    flush_focus(&document, &effects);
    remove_bootstrap(&document);
    clear_development_error(&document);
    Ok(())
}

fn dispatch(generation: u64, pending: PendingHandler, event: Event) {
    let options = match &pending {
        PendingHandler::Click(_, options)
        | PendingHandler::Input(_, options)
        | PendingHandler::Change(_, options)
        | PendingHandler::Submit(_, options)
        | PendingHandler::Blur(_, options)
        | PendingHandler::Focus(_, options)
        | PendingHandler::KeyDown(_, options)
        | PendingHandler::KeyUp(_, options) => *options,
    };
    if options.prevent_default {
        event.prevent_default();
    }
    if options.stop_propagation {
        event.stop_propagation();
    }

    let result = APP.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(app) = slot.as_mut() else {
            return Ok(());
        };
        if app.generation != generation {
            return Ok(());
        }

        let message = match pending {
            PendingHandler::Click(message, _)
            | PendingHandler::Submit(message, _)
            | PendingHandler::Blur(message, _)
            | PendingHandler::Focus(message, _) => message,
            PendingHandler::Input(to_message, _) | PendingHandler::Change(to_message, _) => {
                let value = event_input_value(&event)?;
                app.session
                    .apply(to_message, Value::Text(Rc::from(value)))?
            }
            PendingHandler::KeyDown(to_message, _) | PendingHandler::KeyUp(to_message, _) => {
                let value = event_key_value(&event)?;
                app.session
                    .apply(to_message, Value::Text(Rc::from(value)))?
            }
        };

        let effects = BrowserEffects::default();
        let next_model =
            app.program
                .transition(&app.session, message, app.model.clone(), &effects)?;
        let next_document = app
            .program
            .render(&app.session, next_model.clone())
            .map_err(|err| EvalError::EffectfulNotExecutable(err.to_string()))?;
        validate_document(&next_document)
            .map_err(|err| EvalError::EffectfulNotExecutable(err.to_string()))?;

        let document = browser_document()
            .map_err(|err| EvalError::EffectfulNotExecutable(js_value_text(err)))?;
        let selection = SelectionSnapshot::capture(&document);
        detach_listeners(&mut app.listeners);
        if let Err(err) = diff_patch_document(&document, &app.rendered, &next_document) {
            app.listeners = attach_listeners(&document, &app.rendered, app.generation)
                .map_err(|attach| EvalError::EffectfulNotExecutable(js_value_text(attach)))?;
            return Err(EvalError::EffectfulNotExecutable(js_value_text(err)));
        }

        app.generation = app.generation.wrapping_add(1);
        app.listeners = attach_listeners(&document, &next_document, app.generation)
            .map_err(|err| EvalError::EffectfulNotExecutable(js_value_text(err)))?;
        app.model = next_model;
        app.rendered = next_document;
        if let Some(selection) = selection {
            selection.restore(&document);
        }
        flush_focus(&document, &effects);
        clear_development_error(&document);
        Ok(())
    });

    if let Err(err) = result {
        web_sys::console::error_1(&JsValue::from_str(&err.to_string()));
        APP.with(|slot| {
            if slot.borrow().as_ref().is_some_and(|app| app.development)
                && let Ok(document) = browser_document()
            {
                show_development_error(&document, &err.to_string());
            }
        });
    }
}

fn attach_listeners(
    document: &WebDocument,
    rendered: &Document,
    generation: u64,
) -> Result<Vec<Listener>, JsValue> {
    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("document has no body"))?;
    let mut listeners = Vec::new();
    attach_children(
        &body.clone().unchecked_into::<Node>(),
        &rendered.body,
        generation,
        &mut listeners,
    )?;
    Ok(listeners)
}

fn attach_children(
    parent: &Node,
    children: &[Html],
    generation: u64,
    listeners: &mut Vec<Listener>,
) -> Result<(), JsValue> {
    for (index, child) in children.iter().enumerate() {
        let Some(node) = parent.child_nodes().item(index as u32) else {
            return Err(JsValue::from_str("hydrated DOM child is missing"));
        };
        let Html::Element(element) = child else {
            continue;
        };
        let target: EventTarget = node.clone().dyn_into()?;
        for attribute in &element.attributes {
            let Attribute::Event(handler) = attribute else {
                continue;
            };
            let (event_name, pending) = match handler {
                EventHandler::Click { message, options } => {
                    ("click", PendingHandler::Click(message.clone(), *options))
                }
                EventHandler::Input {
                    to_message,
                    options,
                } => ("input", PendingHandler::Input(to_message.clone(), *options)),
                EventHandler::Change {
                    to_message,
                    options,
                } => (
                    "change",
                    PendingHandler::Change(to_message.clone(), *options),
                ),
                EventHandler::Submit { message, options } => {
                    ("submit", PendingHandler::Submit(message.clone(), *options))
                }
                EventHandler::Blur { message, options } => {
                    ("blur", PendingHandler::Blur(message.clone(), *options))
                }
                EventHandler::Focus { message, options } => {
                    ("focus", PendingHandler::Focus(message.clone(), *options))
                }
                EventHandler::KeyDown {
                    to_message,
                    options,
                } => (
                    "keydown",
                    PendingHandler::KeyDown(to_message.clone(), *options),
                ),
                EventHandler::KeyUp {
                    to_message,
                    options,
                } => ("keyup", PendingHandler::KeyUp(to_message.clone(), *options)),
            };
            let callback = {
                let pending = pending.clone();
                Closure::<dyn FnMut(Event)>::new(move |event| {
                    dispatch(generation, pending.clone(), event);
                })
            };
            target
                .add_event_listener_with_callback(event_name, callback.as_ref().unchecked_ref())?;
            listeners.push(Listener {
                target: target.clone(),
                event_name,
                callback,
            });
        }
        attach_children(&node, &element.children, generation, listeners)?;
    }
    Ok(())
}

fn detach_listeners(listeners: &mut Vec<Listener>) {
    for listener in listeners.drain(..) {
        let _ = listener.target.remove_event_listener_with_callback(
            listener.event_name,
            listener.callback.as_ref().unchecked_ref(),
        );
    }
}

fn patch_document(document: &WebDocument, rendered: &Document) -> Result<(), JsValue> {
    let root = document
        .document_element()
        .ok_or_else(|| JsValue::from_str("document has no documentElement"))?;
    root.set_attribute("lang", &rendered.language)?;
    document.set_title(&rendered.title);
    patch_head(document, rendered)?;

    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("document has no body"))?;
    clear_attributes(&body.clone().unchecked_into())?;
    apply_static_attributes(&body.clone().unchecked_into(), &rendered.body_attributes)?;
    patch_children(&body.unchecked_into(), &rendered.body)
}

fn patch_head(document: &WebDocument, rendered: &Document) -> Result<(), JsValue> {
    let head = document
        .head()
        .ok_or_else(|| JsValue::from_str("document has no head"))?;
    let managed = head.query_selector_all("[data-zutai-managed]")?;
    for index in (0..managed.length()).rev() {
        if let Some(node) = managed.item(index) {
            let _ = head.remove_child(&node);
        }
    }
    let bootstrap = head.query_selector("script[data-zutai-bootstrap]")?;
    for head_node in &rendered.head {
        let element = create_head_node(document, head_node)?;
        head.insert_before(
            &element,
            bootstrap.as_ref().map(|item| item.unchecked_ref()),
        )?;
    }
    Ok(())
}

fn create_head_node(document: &WebDocument, node: &HeadNode) -> Result<Node, JsValue> {
    let element = match node {
        HeadNode::MetaName { name, content } => {
            let element = document.create_element("meta")?;
            element.set_attribute("name", name)?;
            element.set_attribute("content", content)?;
            element
        }
        HeadNode::MetaProperty { property, content } => {
            let element = document.create_element("meta")?;
            element.set_attribute("property", property)?;
            element.set_attribute("content", content)?;
            element
        }
        HeadNode::Link {
            rel,
            href,
            mime,
            media,
            sizes,
            cross_origin,
        } => {
            let element = document.create_element("link")?;
            element.set_attribute("rel", rel)?;
            element.set_attribute("href", href)?;
            set_optional_attribute(&element, "type", mime.as_deref())?;
            set_optional_attribute(&element, "media", media.as_deref())?;
            set_optional_attribute(&element, "sizes", sizes.as_deref())?;
            set_optional_attribute(&element, "crossorigin", cross_origin.as_deref())?;
            element
        }
        HeadNode::Base { href, target } => {
            let element = document.create_element("base")?;
            element.set_attribute("href", href)?;
            set_optional_attribute(&element, "target", target.as_deref())?;
            element
        }
        HeadNode::Style(sheet) => {
            let element = document.create_element("style")?;
            let css = render_stylesheet(sheet, false).map_err(js_error)?;
            element.set_text_content(Some(&css));
            element
        }
    };
    element.set_attribute("data-zutai-managed", "")?;
    Ok(element.unchecked_into())
}

fn patch_children(parent: &Node, children: &[Html]) -> Result<(), JsValue> {
    for (index, child) in children.iter().enumerate() {
        let key = child_key(child);
        let current = parent.child_nodes().item(index as u32);
        let candidate = if let Some(key) = key {
            find_keyed_child(parent, key)
        } else {
            current.clone().filter(|node| node_key(node).is_none())
        };

        if let Some(candidate) = &candidate
            && current
                .as_ref()
                .is_none_or(|current| !candidate.is_same_node(Some(current)))
        {
            parent.insert_before(candidate, current.as_ref())?;
        }

        let current = parent.child_nodes().item(index as u32);
        match_node(parent, current, child)?;
    }

    while parent.child_nodes().length() > children.len() as u32 {
        let index = parent.child_nodes().length() - 1;
        if let Some(extra) = parent.child_nodes().item(index) {
            parent.remove_child(&extra)?;
        }
    }
    Ok(())
}

fn match_node(parent: &Node, existing: Option<Node>, next: &Html) -> Result<Node, JsValue> {
    match next {
        Html::Text(text) => {
            if let Some(existing_node) = existing.as_ref()
                && existing_node.node_type() == Node::TEXT_NODE
            {
                if existing_node.node_value().as_deref() != Some(text.as_str()) {
                    existing_node.set_node_value(Some(text));
                }
                return Ok(existing_node.clone());
            }
            replace_or_append(parent, existing, create_node(parent, next)?)
        }
        Html::Element(element) => {
            if let Some(existing_node) = existing.as_ref()
                && let Ok(dom_element) = existing_node.clone().dyn_into::<WebElement>()
                && dom_element.tag_name().eq_ignore_ascii_case(&element.tag)
                && node_key(existing_node).as_deref() == element.key.as_deref()
            {
                patch_element(&dom_element, element)?;
                return Ok(existing_node.clone());
            }
            replace_or_append(parent, existing, create_node(parent, next)?)
        }
    }
}

fn create_node(parent: &Node, next: &Html) -> Result<Node, JsValue> {
    let document = parent
        .owner_document()
        .ok_or_else(|| JsValue::from_str("DOM node has no owner document"))?;
    match next {
        Html::Text(text) => Ok(document.create_text_node(text).unchecked_into()),
        Html::Element(element) => {
            let dom_element = document.create_element(&element.tag)?;
            patch_element(&dom_element, element)?;
            Ok(dom_element.unchecked_into())
        }
    }
}

fn patch_element(dom_element: &WebElement, element: &Element) -> Result<(), JsValue> {
    apply_element_attributes(dom_element, element)?;
    if !is_void_element(&element.tag) {
        patch_children(&dom_element.clone().unchecked_into(), &element.children)?;
    }
    Ok(())
}

/// Shared by the hydration DOM-walk (`patch_element`) and the steady-state
/// retained-tree diff (`diff_patch_element`) below. Attribute diffing itself
/// is still clear-and-reapply here; that is a later milestone.
fn apply_element_attributes(dom_element: &WebElement, element: &Element) -> Result<(), JsValue> {
    clear_attributes(dom_element)?;
    if let Some(key) = &element.key {
        dom_element.set_attribute("data-zutai-key", key)?;
    }
    for attribute in &element.attributes {
        match attribute {
            Attribute::Static(attribute) => apply_static_attribute(dom_element, attribute)?,
            Attribute::TextProperty { name, value } if name == "value" => {
                if let Some(input) = dom_element.dyn_ref::<web_sys::HtmlInputElement>() {
                    if input.value() != *value {
                        input.set_value(value);
                    }
                } else if let Some(textarea) = dom_element.dyn_ref::<web_sys::HtmlTextAreaElement>()
                {
                    if textarea.value() != *value {
                        textarea.set_value(value);
                    }
                } else {
                    dom_element.set_attribute(name, value)?;
                }
            }
            Attribute::TextProperty { name, value } => dom_element.set_attribute(name, value)?,
            Attribute::BoolProperty { name, value } if name == "checked" => {
                if let Some(input) = dom_element.dyn_ref::<web_sys::HtmlInputElement>() {
                    input.set_checked(*value);
                } else if *value {
                    dom_element.set_attribute(name, "")?;
                }
            }
            Attribute::BoolProperty { name, value } if *value => {
                dom_element.set_attribute(name, "")?;
            }
            Attribute::BoolProperty { .. } | Attribute::Event(_) => {}
        }
    }
    Ok(())
}

/// Patch the live DOM against a newly rendered `Document`, diffing against
/// `old` (the previous `Document`, retained in `App::rendered`) as plain
/// data instead of reading identity back off the DOM. Only used once an old
/// tree exists in memory; hydration has none and uses `patch_document`.
fn diff_patch_document(
    document: &WebDocument,
    old: &Document,
    new: &Document,
) -> Result<(), JsValue> {
    let root = document
        .document_element()
        .ok_or_else(|| JsValue::from_str("document has no documentElement"))?;
    root.set_attribute("lang", &new.language)?;
    document.set_title(&new.title);
    patch_head(document, new)?;

    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("document has no body"))?;
    diff_apply_static_attributes(
        &body.clone().unchecked_into(),
        &old.body_attributes,
        &new.body_attributes,
    )?;
    diff_patch_children(&body.unchecked_into(), &old.body, &new.body)
}

fn diff_patch_children(parent: &Node, old: &[Html], new: &[Html]) -> Result<(), JsValue> {
    let diff = diff_children(old, new);

    let snapshot: Vec<Node> = (0..old.len() as u32)
        .map(|index| parent.child_nodes().item(index))
        .collect::<Option<_>>()
        .ok_or_else(|| JsValue::from_str("live DOM children do not match the retained tree"))?;

    // Remove stale nodes first so none of them can end up used as an anchor
    // below.
    for &old_index in &diff.removed_old_indices {
        parent.remove_child(&snapshot[old_index])?;
    }

    // Walk backwards, so each already-placed node can serve as the
    // insertion anchor for whatever precedes it. `Keep` nodes are trusted
    // to already be in the right relative order (that is what the diff's
    // longest-increasing-subsequence selection guarantees) and are never
    // moved; only `Move` and `Create` ever call `insert_before`.
    let mut anchor: Option<Node> = None;
    for (new_index, op) in diff.ops.iter().enumerate().rev() {
        let node = match *op {
            ChildOp::Keep { old_index } => {
                let node = snapshot[old_index].clone();
                diff_patch_node(&node, &old[old_index], &new[new_index])?;
                node
            }
            ChildOp::Move { old_index } => {
                let node = snapshot[old_index].clone();
                parent.insert_before(&node, anchor.as_ref())?;
                diff_patch_node(&node, &old[old_index], &new[new_index])?;
                node
            }
            ChildOp::Create => {
                let node = create_node(parent, &new[new_index])?;
                parent.insert_before(&node, anchor.as_ref())?;
                node
            }
        };
        anchor = Some(node);
    }
    Ok(())
}

fn diff_patch_node(node: &Node, old: &Html, new: &Html) -> Result<(), JsValue> {
    match new {
        Html::Text(text) => {
            if !matches!(old, Html::Text(old_text) if old_text == text) {
                node.set_node_value(Some(text));
            }
            Ok(())
        }
        Html::Element(element) => {
            let Html::Element(old_element) = old else {
                unreachable!("diff_children only reuses nodes of a matching kind")
            };
            let dom_element: WebElement = node.clone().dyn_into()?;
            diff_patch_element(&dom_element, old_element, element)
        }
    }
}

fn diff_patch_element(
    dom_element: &WebElement,
    old: &Element,
    new: &Element,
) -> Result<(), JsValue> {
    diff_apply_element_attributes(dom_element, old, new)?;
    if !is_void_element(&new.tag) {
        diff_patch_children(
            &dom_element.clone().unchecked_into(),
            &old.children,
            &new.children,
        )?;
    }
    Ok(())
}

/// Apply only the attributes that actually changed between `old` and `new`,
/// using `diff_element_attributes` (pure, see `diff.rs`). `value`/`checked`
/// are excluded from that diff and handled here exactly as
/// `apply_element_attributes` always has: compared against live DOM state
/// unconditionally, since a user's typing or checking can diverge the
/// live property from whatever was last declared.
fn diff_apply_element_attributes(
    dom_element: &WebElement,
    old: &Element,
    new: &Element,
) -> Result<(), JsValue> {
    apply_attribute_diff(dom_element, &diff_element_attributes(old, new))?;
    for attribute in &new.attributes {
        match attribute {
            Attribute::TextProperty { name, value } if name == "value" => {
                if let Some(input) = dom_element.dyn_ref::<web_sys::HtmlInputElement>() {
                    if input.value() != *value {
                        input.set_value(value);
                    }
                } else if let Some(textarea) = dom_element.dyn_ref::<web_sys::HtmlTextAreaElement>()
                {
                    if textarea.value() != *value {
                        textarea.set_value(value);
                    }
                } else {
                    dom_element.set_attribute(name, value)?;
                }
            }
            Attribute::BoolProperty { name, value } if name == "checked" => {
                if let Some(input) = dom_element.dyn_ref::<web_sys::HtmlInputElement>() {
                    input.set_checked(*value);
                } else if *value {
                    dom_element.set_attribute(name, "")?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn diff_apply_static_attributes(
    dom_element: &WebElement,
    old: &[StaticAttribute],
    new: &[StaticAttribute],
) -> Result<(), JsValue> {
    apply_attribute_diff(dom_element, &diff_static_attributes(old, new))
}

fn apply_attribute_diff(dom_element: &WebElement, diff: &AttributeDiff) -> Result<(), JsValue> {
    for name in &diff.removed {
        dom_element.remove_attribute(name)?;
    }
    for (name, effect) in &diff.set {
        match effect {
            AttributeEffect::Text(value) => dom_element.set_attribute(name, value)?,
            AttributeEffect::Styles(declarations) => {
                let mut css = String::new();
                render_declarations(declarations, false, &mut css).map_err(js_error)?;
                dom_element.set_attribute(name, &css)?;
            }
        }
    }
    Ok(())
}

fn clear_attributes(element: &WebElement) -> Result<(), JsValue> {
    let attributes = element.attributes();
    let mut names = Vec::with_capacity(attributes.length() as usize);
    for index in 0..attributes.length() {
        if let Some(attribute) = attributes.item(index) {
            names.push(attribute.name());
        }
    }
    for name in names {
        element.remove_attribute(&name)?;
    }
    Ok(())
}

fn apply_static_attributes(
    element: &WebElement,
    attributes: &[StaticAttribute],
) -> Result<(), JsValue> {
    for attribute in attributes {
        apply_static_attribute(element, attribute)?;
    }
    Ok(())
}

fn apply_static_attribute(
    element: &WebElement,
    attribute: &StaticAttribute,
) -> Result<(), JsValue> {
    match attribute {
        StaticAttribute::Text { name, value } => element.set_attribute(name, value)?,
        StaticAttribute::Bool { name, value: true } => element.set_attribute(name, "")?,
        StaticAttribute::Bool { value: false, .. } => {}
        StaticAttribute::Styles(declarations) => {
            let mut css = String::new();
            render_declarations(declarations, false, &mut css).map_err(js_error)?;
            element.set_attribute("style", &css)?;
        }
    }
    Ok(())
}

fn replace_or_append(parent: &Node, existing: Option<Node>, next: Node) -> Result<Node, JsValue> {
    if let Some(existing) = existing {
        parent.replace_child(&next, &existing)?;
    } else {
        parent.append_child(&next)?;
    }
    Ok(next)
}

fn node_key(node: &Node) -> Option<String> {
    node.dyn_ref::<WebElement>()
        .and_then(|element| element.get_attribute("data-zutai-key"))
}

fn find_keyed_child(parent: &Node, key: &str) -> Option<Node> {
    let children = parent.child_nodes();
    (0..children.length())
        .filter_map(|index| children.item(index))
        .find(|node| node_key(node).as_deref() == Some(key))
}

fn set_optional_attribute(
    element: &WebElement,
    name: &str,
    value: Option<&str>,
) -> Result<(), JsValue> {
    if let Some(value) = value {
        element.set_attribute(name, value)?;
    }
    Ok(())
}

fn event_input_value(event: &Event) -> Result<String, EvalError> {
    let target = event.current_target().ok_or_else(|| {
        EvalError::EffectfulNotExecutable("input event has no currentTarget".into())
    })?;
    if let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>() {
        return Ok(input.value());
    }
    if let Some(textarea) = target.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        return Ok(textarea.value());
    }
    if let Some(select) = target.dyn_ref::<web_sys::HtmlSelectElement>() {
        return Ok(select.value());
    }
    Err(EvalError::EffectfulNotExecutable(
        "input handler requires an input, textarea, or select element".into(),
    ))
}

fn event_key_value(event: &Event) -> Result<String, EvalError> {
    let keyboard_event = event.dyn_ref::<web_sys::KeyboardEvent>().ok_or_else(|| {
        EvalError::EffectfulNotExecutable("key handler requires a KeyboardEvent".into())
    })?;
    Ok(keyboard_event.key())
}

fn flush_focus(document: &WebDocument, effects: &BrowserEffects) {
    for element_id in effects.focus.borrow_mut().drain(..) {
        let Some(element) = document.get_element_by_id(&element_id) else {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "browser.focus: no element has id `{element_id}`"
            )));
            continue;
        };
        if let Some(element) = element.dyn_ref::<web_sys::HtmlElement>() {
            let _ = element.focus();
        }
    }
}

struct SelectionSnapshot {
    element: WebElement,
    start: Option<u32>,
    end: Option<u32>,
}

impl SelectionSnapshot {
    fn capture(document: &WebDocument) -> Option<Self> {
        let element = document.active_element()?;
        if let Some(input) = element.dyn_ref::<web_sys::HtmlInputElement>() {
            return Some(Self {
                element: element.clone(),
                start: input.selection_start().ok().flatten(),
                end: input.selection_end().ok().flatten(),
            });
        }
        if let Some(textarea) = element.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            return Some(Self {
                element: element.clone(),
                start: textarea.selection_start().ok().flatten(),
                end: textarea.selection_end().ok().flatten(),
            });
        }
        None
    }

    fn restore(self, document: &WebDocument) {
        let Some(root) = document.document_element() else {
            return;
        };
        if !root.contains(Some(&self.element)) {
            return;
        }
        if document
            .active_element()
            .as_ref()
            .is_none_or(|active| !active.is_same_node(Some(&self.element)))
            && let Some(html) = self.element.dyn_ref::<web_sys::HtmlElement>()
        {
            let _ = html.focus();
        }
        if let (Some(start), Some(end)) = (self.start, self.end) {
            if let Some(input) = self.element.dyn_ref::<web_sys::HtmlInputElement>() {
                let max = input.value().encode_utf16().count() as u32;
                let _ = input.set_selection_range(start.min(max), end.min(max));
            } else if let Some(textarea) = self.element.dyn_ref::<web_sys::HtmlTextAreaElement>() {
                let max = textarea.value().encode_utf16().count() as u32;
                let _ = textarea.set_selection_range(start.min(max), end.min(max));
            }
        }
    }
}

fn remove_bootstrap(document: &WebDocument) {
    if let Ok(Some(script)) = document.query_selector("script[data-zutai-bootstrap]")
        && let Some(parent) = script.parent_node()
    {
        let _ = parent.remove_child(&script);
    }
}

fn show_development_error(document: &WebDocument, message: &str) {
    clear_development_error(document);
    let Ok(element) = document.create_element("pre") else {
        return;
    };
    element.set_id("zutai-development-error");
    let _ = element.set_attribute(
        "style",
        "position:fixed;inset:auto 1rem 1rem 1rem;z-index:2147483647;padding:1rem;max-height:40vh;overflow:auto;background:#170b22;color:#ff8ea1;border:1px solid #ff5678;border-radius:.5rem;white-space:pre-wrap",
    );
    element.set_text_content(Some(message));
    if let Some(body) = document.body() {
        let _ = body.append_child(&element);
    }
}

fn clear_development_error(document: &WebDocument) {
    if let Some(element) = document.get_element_by_id("zutai-development-error")
        && let Some(parent) = element.parent_node()
    {
        let _ = parent.remove_child(&element);
    }
}

fn browser_document() -> Result<WebDocument, JsValue> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("browser document is unavailable"))
}

fn console_error_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&JsValue::from_str(&info.to_string()));
    }));
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn js_value_text(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "browser DOM operation failed".to_owned())
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Posit(_) => "Posit",
        Value::Text(_) => "Text",
        Value::Atom(_) => "Atom",
        Value::List(_) => "List",
        Value::Tuple(_) => "Tuple",
        Value::Record(_) => "Record",
        Value::Closure(_) | Value::TlcClosure(_) => "Function",
        Value::TypeValue(_) => "Type",
        Value::TaggedValue { .. } => "TaggedValue",
        Value::Nothing => "Nothing",
        Value::HostHandle(_) => "HostHandle",
        Value::WitnessDict(_) => "WitnessDict",
        Value::Builtin(_) | Value::BuiltinPartial { .. } => "Builtin",
    }
}
