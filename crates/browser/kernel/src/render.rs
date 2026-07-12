use std::collections::HashSet;

use crate::css::{CssRenderError, render_declarations, render_stylesheet};
use crate::{Attribute, Document, Element, HeadNode, Html, StaticAttribute, is_void_element};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrerenderedPage {
    pub html: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Css(#[from] CssRenderError),
    #[error("document language must be a non-empty language tag")]
    InvalidLanguage,
    #[error("document contains more than one base element")]
    DuplicateBase,
    #[error("duplicate element id `{0}`")]
    DuplicateId(String),
    #[error("duplicate sibling key `{0}`")]
    DuplicateKey(String),
    #[error("void element <{0}> cannot have children")]
    VoidElementChildren(String),
    #[error("unsafe URL scheme in `{0}`")]
    UnsafeUrl(String),
    #[error("invalid HTML name `{0}`")]
    InvalidHtmlName(String),
}

/// Render a complete semantic HTML document and its generated module loader.
///
/// The UTF-8 declaration and loader are kernel-owned. All other visible head
/// and body content comes from the evaluated Zutai `Document` value.
pub fn prerender_document(
    document: &Document,
    bootstrap_src: &str,
) -> Result<PrerenderedPage, RenderError> {
    validate_document(document)?;

    let mut out = String::with_capacity(16 * 1024);
    out.push_str("<!doctype html><html lang=\"");
    escape_attribute(&document.language, &mut out);
    out.push_str("\"><head><meta charset=\"utf-8\"><title>");
    escape_text(&document.title, &mut out);
    out.push_str("</title>");

    for node in &document.head {
        render_head(node, &mut out)?;
    }

    validate_url(bootstrap_src)?;
    out.push_str("<script data-zutai-bootstrap type=\"module\" src=\"");
    escape_attribute(bootstrap_src, &mut out);
    out.push_str("\"></script></head><body");
    for attr in &document.body_attributes {
        render_static_attribute(attr, &mut out)?;
    }
    out.push('>');
    for child in &document.body {
        render_html(child, &mut out)?;
    }
    out.push_str("</body></html>");
    Ok(PrerenderedPage { html: out })
}

pub(crate) fn validate_document(document: &Document) -> Result<(), RenderError> {
    if document.language.is_empty()
        || !document
            .language
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(RenderError::InvalidLanguage);
    }

    let base_count = document
        .head
        .iter()
        .filter(|node| matches!(node, HeadNode::Base { .. }))
        .count();
    if base_count > 1 {
        return Err(RenderError::DuplicateBase);
    }

    let mut ids = HashSet::new();
    validate_siblings(&document.body, &mut ids)
}

fn validate_siblings(nodes: &[Html], ids: &mut HashSet<String>) -> Result<(), RenderError> {
    let mut keys = HashSet::new();
    for node in nodes {
        let Html::Element(element) = node else {
            continue;
        };
        if let Some(key) = &element.key
            && !keys.insert(key.clone())
        {
            return Err(RenderError::DuplicateKey(key.clone()));
        }
        if is_void_element(&element.tag) && !element.children.is_empty() {
            return Err(RenderError::VoidElementChildren(element.tag.clone()));
        }
        for attr in &element.attributes {
            if let Attribute::Static(StaticAttribute::Text { name, value }) = attr {
                if name == "id" && !ids.insert(value.clone()) {
                    return Err(RenderError::DuplicateId(value.clone()));
                }
                if matches!(name.as_str(), "href" | "src") {
                    validate_url(value)?;
                }
            }
        }
        validate_siblings(&element.children, ids)?;
    }
    Ok(())
}

fn render_head(node: &HeadNode, out: &mut String) -> Result<(), RenderError> {
    match node {
        HeadNode::MetaName { name, content } => {
            validate_html_name(name)?;
            out.push_str("<meta data-zutai-managed name=\"");
            escape_attribute(name, out);
            out.push_str("\" content=\"");
            escape_attribute(content, out);
            out.push_str("\">");
        }
        HeadNode::MetaProperty { property, content } => {
            validate_html_name(property)?;
            out.push_str("<meta data-zutai-managed property=\"");
            escape_attribute(property, out);
            out.push_str("\" content=\"");
            escape_attribute(content, out);
            out.push_str("\">");
        }
        HeadNode::Link {
            rel,
            href,
            mime,
            media,
            sizes,
            cross_origin,
        } => {
            validate_html_name(rel)?;
            validate_url(href)?;
            out.push_str("<link data-zutai-managed rel=\"");
            escape_attribute(rel, out);
            out.push_str("\" href=\"");
            escape_attribute(href, out);
            out.push('"');
            render_optional_attribute("type", mime.as_deref(), out)?;
            render_optional_attribute("media", media.as_deref(), out)?;
            render_optional_attribute("sizes", sizes.as_deref(), out)?;
            render_optional_attribute("crossorigin", cross_origin.as_deref(), out)?;
            out.push('>');
        }
        HeadNode::Base { href, target } => {
            validate_url(href)?;
            out.push_str("<base data-zutai-managed href=\"");
            escape_attribute(href, out);
            out.push('"');
            render_optional_attribute("target", target.as_deref(), out)?;
            out.push('>');
        }
        HeadNode::Style(sheet) => {
            let css = render_stylesheet(sheet, false)?;
            out.push_str("<style data-zutai-managed data-zutai-style>");
            // The HTML parser recognizes a literal `</style` even inside a CSS
            // string. Escaping the slash preserves the CSS string while keeping
            // structured source from terminating the element.
            out.push_str(&css.replace("</style", "<\\/style"));
            out.push_str("</style>");
        }
    }
    Ok(())
}

fn render_html(node: &Html, out: &mut String) -> Result<(), RenderError> {
    match node {
        Html::Text(text) => escape_text(text, out),
        Html::Element(element) => render_element(element, out)?,
    }
    Ok(())
}

fn render_element(element: &Element, out: &mut String) -> Result<(), RenderError> {
    validate_html_name(&element.tag)?;
    out.push('<');
    out.push_str(&element.tag);
    if let Some(key) = &element.key {
        out.push_str(" data-zutai-key=\"");
        escape_attribute(key, out);
        out.push('"');
    }
    for attr in &element.attributes {
        match attr {
            Attribute::Static(attr) => render_static_attribute(attr, out)?,
            Attribute::TextProperty { name, value } => {
                validate_html_name(name)?;
                out.push(' ');
                out.push_str(name);
                out.push_str("=\"");
                escape_attribute(value, out);
                out.push('"');
            }
            Attribute::BoolProperty { name, value } if *value => {
                validate_html_name(name)?;
                out.push(' ');
                out.push_str(name);
            }
            Attribute::BoolProperty { .. } | Attribute::Event(_) => {}
        }
    }
    out.push('>');
    if !is_void_element(&element.tag) {
        for child in &element.children {
            render_html(child, out)?;
        }
        out.push_str("</");
        out.push_str(&element.tag);
        out.push('>');
    }
    Ok(())
}

fn render_static_attribute(attr: &StaticAttribute, out: &mut String) -> Result<(), RenderError> {
    match attr {
        StaticAttribute::Text { name, value } => {
            validate_html_name(name)?;
            if matches!(name.as_str(), "href" | "src") {
                validate_url(value)?;
            }
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            escape_attribute(value, out);
            out.push('"');
        }
        StaticAttribute::Bool { name, value: true } => {
            validate_html_name(name)?;
            out.push(' ');
            out.push_str(name);
        }
        StaticAttribute::Bool { value: false, .. } => {}
        StaticAttribute::Styles(declarations) => {
            let mut css = String::new();
            render_declarations(declarations, false, &mut css)?;
            out.push_str(" style=\"");
            escape_attribute(&css, out);
            out.push('"');
        }
    }
    Ok(())
}

fn render_optional_attribute(
    name: &str,
    value: Option<&str>,
    out: &mut String,
) -> Result<(), RenderError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_html_name(name)?;
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    escape_attribute(value, out);
    out.push('"');
    Ok(())
}

fn validate_url(url: &str) -> Result<(), RenderError> {
    let trimmed = url.trim_start();
    let scheme = trimmed
        .split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    if matches!(scheme.as_deref(), Some("javascript" | "vbscript" | "data")) {
        return Err(RenderError::UnsafeUrl(url.to_owned()));
    }
    Ok(())
}

fn validate_html_name(name: &str) -> Result<(), RenderError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(RenderError::InvalidHtmlName(name.to_owned()));
    };
    if !first.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
    {
        return Err(RenderError::InvalidHtmlName(name.to_owned()));
    }
    Ok(())
}

fn escape_text(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

fn escape_attribute(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CssValue, Declaration, EventHandler, EventOptions};
    use zutai_eval::Value;

    fn page(children: Vec<Html>) -> Document {
        Document {
            language: "en".into(),
            title: "Zutai <browser>".into(),
            head: vec![HeadNode::MetaName {
                name: "description".into(),
                content: "typed & pure".into(),
            }],
            body_attributes: vec![],
            body: children,
        }
    }

    #[test]
    fn prerenders_semantic_html_and_omits_handlers() {
        let button = Html::Element(Element {
            tag: "button".into(),
            key: Some("counter".into()),
            attributes: vec![
                Attribute::Static(StaticAttribute::Text {
                    name: "id".into(),
                    value: "increment".into(),
                }),
                Attribute::Event(EventHandler::Click {
                    message: Value::Int(1),
                    options: EventOptions::default(),
                }),
            ],
            children: vec![Html::Text("Add <one>".into())],
        });
        let rendered = prerender_document(&page(vec![button]), "/_zutai/a/bootstrap.js")
            .unwrap()
            .html;
        assert!(rendered.starts_with("<!doctype html><html lang=\"en\">"));
        assert!(rendered.contains("<title>Zutai &lt;browser&gt;</title>"));
        assert!(rendered.contains("data-zutai-key=\"counter\""));
        assert!(rendered.contains("Add &lt;one&gt;"));
        assert!(!rendered.contains("onclick"));
    }

    #[test]
    fn rejects_duplicate_ids_keys_and_unsafe_urls() {
        let element = |key: &str, id: &str| {
            Html::Element(Element {
                tag: "a".into(),
                key: Some(key.into()),
                attributes: vec![Attribute::Static(StaticAttribute::Text {
                    name: "id".into(),
                    value: id.into(),
                })],
                children: vec![],
            })
        };
        assert!(matches!(
            prerender_document(
                &page(vec![element("same", "a"), element("same", "b")]),
                "/boot.js"
            ),
            Err(RenderError::DuplicateKey(_))
        ));
        assert!(matches!(
            prerender_document(
                &page(vec![element("a", "same"), element("b", "same")]),
                "/boot.js"
            ),
            Err(RenderError::DuplicateId(_))
        ));

        let bad = Html::Element(Element {
            tag: "a".into(),
            key: None,
            attributes: vec![Attribute::Static(StaticAttribute::Text {
                name: "href".into(),
                value: " javascript:alert(1)".into(),
            })],
            children: vec![],
        });
        assert!(matches!(
            prerender_document(&page(vec![bad]), "/boot.js"),
            Err(RenderError::UnsafeUrl(_))
        ));
    }

    #[test]
    fn renders_inline_structured_styles() {
        let element = Html::Element(Element {
            tag: "div".into(),
            key: None,
            attributes: vec![Attribute::Static(StaticAttribute::Styles(vec![
                Declaration {
                    property: "opacity".into(),
                    value: CssValue::Number(0.5),
                    important: false,
                },
            ]))],
            children: vec![],
        });
        let html = prerender_document(&page(vec![element]), "/boot.js")
            .unwrap()
            .html;
        assert!(html.contains("style=\"opacity:0.5;\""));
    }
}
