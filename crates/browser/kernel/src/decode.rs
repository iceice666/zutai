use std::rc::Rc;

use zutai_eval::{EffectHandler, EvalError, Thunk, TlcSession, Value};

use crate::{
    Attribute, CssValue, Declaration, Document, Element, EventHandler, EventOptions, HeadNode,
    Html, Keyframe, KeyframeStop, MediaQuery, Pseudo, Rule, Selector, Separator, StaticAttribute,
    Stylesheet,
};

type ValueFields<'a> = &'a [(Rc<str>, Thunk)];

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error(transparent)]
    Eval(#[from] EvalError),
    #[error("expected {expected}, found {found}")]
    Type {
        expected: &'static str,
        found: &'static str,
    },
    #[error("missing required field `{0}`")]
    MissingField(String),
    #[error("unknown {kind} variant `#{tag}`")]
    UnknownVariant { kind: &'static str, tag: String },
    #[error("invalid browser program: {0}")]
    InvalidProgram(String),
}

#[derive(Clone, Debug)]
pub struct BrowserProgram {
    pub init: Value,
    pub update: Value,
    pub view: Value,
}

impl BrowserProgram {
    pub fn initialize(
        &self,
        session: &TlcSession,
        handler: &dyn EffectHandler,
    ) -> Result<Value, EvalError> {
        session.apply_with_handler(self.init.clone(), unit(), handler)
    }

    pub fn transition(
        &self,
        session: &TlcSession,
        message: Value,
        model: Value,
        handler: &dyn EffectHandler,
    ) -> Result<Value, EvalError> {
        session.apply2_with_handler(self.update.clone(), message, model, handler)
    }

    pub fn render(&self, session: &TlcSession, model: Value) -> Result<Document, DecodeError> {
        let value = session.apply(self.view.clone(), model)?;
        decode_document(session, value)
    }
}

pub fn decode_program(session: &TlcSession, entry: Value) -> Result<BrowserProgram, DecodeError> {
    let entry = session.force(entry)?;
    let fields = expect_record(&entry)?;
    let init = force_field(session, fields, "init")?;
    let update = force_field(session, fields, "update")?;
    let view = force_field(session, fields, "view")?;
    for (name, value) in [("init", &init), ("update", &update), ("view", &view)] {
        if !is_callable(value) {
            return Err(DecodeError::InvalidProgram(format!(
                "`{name}` must be a function"
            )));
        }
    }
    Ok(BrowserProgram { init, update, view })
}

pub fn decode_document(session: &TlcSession, value: Value) -> Result<Document, DecodeError> {
    let value = session.force(value)?;
    let fields = expect_record(&value)?;
    Ok(Document {
        language: text(session, force_field(session, fields, "language")?)?,
        title: text(session, force_field(session, fields, "title")?)?,
        head: decode_list(session, force_field(session, fields, "head")?, |value| {
            decode_head(session, value)
        })?,
        body_attributes: decode_list(
            session,
            force_field(session, fields, "bodyAttributes")?,
            |value| decode_static_attribute(session, value),
        )?,
        body: decode_list(session, force_field(session, fields, "body")?, |value| {
            decode_html(session, value)
        })?,
    })
}

fn decode_head(session: &TlcSession, value: Value) -> Result<HeadNode, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "metaName" | "meta" => Ok(HeadNode::MetaName {
            name: decode_named_tag(session, force_payload_field(session, payload, "name")?)?,
            content: text(session, force_payload_field(session, payload, "content")?)?,
        }),
        "metaProperty" => Ok(HeadNode::MetaProperty {
            property: text(session, force_payload_field(session, payload, "property")?)?,
            content: text(session, force_payload_field(session, payload, "content")?)?,
        }),
        "link" => Ok(HeadNode::Link {
            rel: decode_named_tag(session, force_payload_field(session, payload, "rel")?)?,
            href: text(session, force_payload_field(session, payload, "href")?)?,
            mime: optional_text(session, force_payload_field(session, payload, "mime")?)?,
            media: optional_text(session, force_payload_field(session, payload, "media")?)?,
            sizes: optional_text(session, force_payload_field(session, payload, "sizes")?)?,
            cross_origin: optional_named_tag(
                session,
                force_payload_field(session, payload, "crossOrigin")?,
            )?,
        }),
        "base" => Ok(HeadNode::Base {
            href: text(session, force_payload_field(session, payload, "href")?)?,
            target: optional_text(session, force_payload_field(session, payload, "target")?)?,
        }),
        "style" => Ok(HeadNode::Style(decode_stylesheet(
            session,
            force_payload_field(session, payload, "sheet")?,
        )?)),
        _ => Err(unknown("head", tag)),
    }
}

fn decode_html(session: &TlcSession, value: Value) -> Result<Html, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "text" => Ok(Html::Text(text(
            session,
            force_payload_field(session, payload, "value")?,
        )?)),
        "element" => {
            let tag = decode_named_tag(session, force_payload_field(session, payload, "tag")?)?;
            let raw_attributes = decode_list(
                session,
                force_payload_field(session, payload, "attributes")?,
                |value| decode_attribute(session, value),
            )?;
            let mut key = None;
            let mut attributes = Vec::with_capacity(raw_attributes.len());
            for decoded in raw_attributes {
                match decoded {
                    DecodedAttribute::Key(value) => key = Some(value),
                    DecodedAttribute::Attribute(value) => attributes.push(value),
                }
            }
            Ok(Html::Element(Element {
                tag,
                key,
                attributes,
                children: decode_list(
                    session,
                    force_payload_field(session, payload, "children")?,
                    |value| decode_html(session, value),
                )?,
            }))
        }
        _ => Err(unknown("HTML", tag)),
    }
}

enum DecodedAttribute {
    Key(String),
    Attribute(Attribute),
}

fn decode_attribute(session: &TlcSession, value: Value) -> Result<DecodedAttribute, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "static" => Ok(DecodedAttribute::Attribute(Attribute::Static(
            decode_static_attribute(session, force_payload_field(session, payload, "attribute")?)?,
        ))),
        "textAttribute" => Ok(DecodedAttribute::Attribute(Attribute::Static(
            StaticAttribute::Text {
                name: decode_attribute_name(
                    session,
                    force_payload_field(session, payload, "name")?,
                )?,
                value: text(session, force_payload_field(session, payload, "value")?)?,
            },
        ))),
        "boolAttribute" => Ok(DecodedAttribute::Attribute(Attribute::Static(
            StaticAttribute::Bool {
                name: decode_attribute_name(
                    session,
                    force_payload_field(session, payload, "name")?,
                )?,
                value: boolean(session, force_payload_field(session, payload, "value")?)?,
            },
        ))),
        "textProperty" => Ok(DecodedAttribute::Attribute(Attribute::TextProperty {
            name: decode_attribute_name(session, force_payload_field(session, payload, "name")?)?,
            value: text(session, force_payload_field(session, payload, "value")?)?,
        })),
        "boolProperty" => Ok(DecodedAttribute::Attribute(Attribute::BoolProperty {
            name: decode_attribute_name(session, force_payload_field(session, payload, "name")?)?,
            value: boolean(session, force_payload_field(session, payload, "value")?)?,
        })),
        "styles" => Ok(DecodedAttribute::Attribute(Attribute::Static(
            StaticAttribute::Styles(decode_list(
                session,
                force_payload_field(session, payload, "declarations")?,
                |value| decode_declaration(session, value),
            )?),
        ))),
        "key" => Ok(DecodedAttribute::Key(text(
            session,
            force_payload_field(session, payload, "value")?,
        )?)),
        "event" => Ok(DecodedAttribute::Attribute(Attribute::Event(decode_event(
            session,
            force_payload_field(session, payload, "handler")?,
        )?))),
        _ => Err(unknown("attribute", tag)),
    }
}

fn decode_static_attribute(
    session: &TlcSession,
    value: Value,
) -> Result<StaticAttribute, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "textAttribute" | "text" => Ok(StaticAttribute::Text {
            name: decode_attribute_name(session, force_payload_field(session, payload, "name")?)?,
            value: text(session, force_payload_field(session, payload, "value")?)?,
        }),
        "boolAttribute" | "bool" => Ok(StaticAttribute::Bool {
            name: decode_attribute_name(session, force_payload_field(session, payload, "name")?)?,
            value: boolean(session, force_payload_field(session, payload, "value")?)?,
        }),
        "styles" => Ok(StaticAttribute::Styles(decode_list(
            session,
            force_payload_field(session, payload, "declarations")?,
            |value| decode_declaration(session, value),
        )?)),
        _ => Err(unknown("static attribute", tag)),
    }
}

fn decode_event(session: &TlcSession, value: Value) -> Result<EventHandler, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    let options = decode_event_options(session, force_payload_field(session, payload, "options")?)?;
    match tag {
        "click" => Ok(EventHandler::Click {
            message: force_payload_field(session, payload, "message")?,
            options,
        }),
        "input" => Ok(EventHandler::Input {
            to_message: force_payload_field(session, payload, "toMessage")?,
            options,
        }),
        _ => Err(unknown("event", tag)),
    }
}

fn decode_event_options(session: &TlcSession, value: Value) -> Result<EventOptions, DecodeError> {
    let value = session.force(value)?;
    let fields = expect_record(&value)?;
    Ok(EventOptions {
        prevent_default: boolean(session, force_field(session, fields, "preventDefault")?)?,
        stop_propagation: boolean(session, force_field(session, fields, "stopPropagation")?)?,
    })
}

fn decode_stylesheet(session: &TlcSession, value: Value) -> Result<Stylesheet, DecodeError> {
    let value = session.force(value)?;
    let fields = expect_record(&value)?;
    Ok(Stylesheet {
        rules: decode_list(session, force_field(session, fields, "rules")?, |value| {
            decode_rule(session, value)
        })?,
    })
}

fn decode_rule(session: &TlcSession, value: Value) -> Result<Rule, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "style" => Ok(Rule::Style {
            selectors: decode_list(
                session,
                force_payload_field(session, payload, "selectors")?,
                |value| decode_selector(session, value),
            )?,
            declarations: decode_list(
                session,
                force_payload_field(session, payload, "declarations")?,
                |value| decode_declaration(session, value),
            )?,
        }),
        "media" => Ok(Rule::Media {
            query: decode_media_query(session, force_payload_field(session, payload, "query")?)?,
            rules: decode_list(
                session,
                force_payload_field(session, payload, "rules")?,
                |value| decode_rule(session, value),
            )?,
        }),
        "keyframes" => Ok(Rule::Keyframes {
            name: text(session, force_payload_field(session, payload, "name")?)?,
            frames: decode_list(
                session,
                force_payload_field(session, payload, "frames")?,
                |value| decode_keyframe(session, value),
            )?,
        }),
        "unsafeRaw" => Ok(Rule::UnsafeRaw(text(
            session,
            force_payload_field(session, payload, "css")?,
        )?)),
        _ => Err(unknown("CSS rule", tag)),
    }
}

fn decode_keyframe(session: &TlcSession, value: Value) -> Result<Keyframe, DecodeError> {
    let value = session.force(value)?;
    let fields = expect_record(&value)?;
    let stop_value = force_field(session, fields, "stop")?;
    let stop_value = session.force(stop_value)?;
    let (tag, payload) = expect_tagged(&stop_value)?;
    let stop = match tag {
        "from" => KeyframeStop::From,
        "to" => KeyframeStop::To,
        "percent" => KeyframeStop::Percent(number(
            session,
            force_payload_field(session, payload, "value")?,
        )?),
        _ => return Err(unknown("keyframe stop", tag)),
    };
    Ok(Keyframe {
        stop,
        declarations: decode_list(
            session,
            force_field(session, fields, "declarations")?,
            |value| decode_declaration(session, value),
        )?,
    })
}

fn decode_selector(session: &TlcSession, value: Value) -> Result<Selector, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "all" => Ok(Selector::All),
        "tag" => Ok(Selector::Tag(text(
            session,
            force_payload_field(session, payload, "name")?,
        )?)),
        "class" => Ok(Selector::Class(text(
            session,
            force_payload_field(session, payload, "name")?,
        )?)),
        "id" => Ok(Selector::Id(text(
            session,
            force_payload_field(session, payload, "name")?,
        )?)),
        "compound" => Ok(Selector::Compound(decode_list(
            session,
            force_payload_field(session, payload, "parts")?,
            |value| decode_selector(session, value),
        )?)),
        "descendant" => Ok(Selector::Descendant(
            Box::new(decode_selector(
                session,
                force_payload_field(session, payload, "ancestor")?,
            )?),
            Box::new(decode_selector(
                session,
                force_payload_field(session, payload, "descendant")?,
            )?),
        )),
        "child" => Ok(Selector::Child(
            Box::new(decode_selector(
                session,
                force_payload_field(session, payload, "parent")?,
            )?),
            Box::new(decode_selector(
                session,
                force_payload_field(session, payload, "child")?,
            )?),
        )),
        "pseudo" => Ok(Selector::Pseudo(
            Box::new(decode_selector(
                session,
                force_payload_field(session, payload, "base")?,
            )?),
            decode_pseudo(session, force_payload_field(session, payload, "pseudo")?)?,
        )),
        "unsafeRaw" => Ok(Selector::UnsafeRaw(text(
            session,
            force_payload_field(session, payload, "css")?,
        )?)),
        _ => Err(unknown("selector", tag)),
    }
}

fn decode_pseudo(session: &TlcSession, value: Value) -> Result<Pseudo, DecodeError> {
    let value = session.force(value)?;
    let (tag, _) = expect_tagged(&value)?;
    match tag {
        "hover" => Ok(Pseudo::Hover),
        "focus" => Ok(Pseudo::Focus),
        "focusVisible" => Ok(Pseudo::FocusVisible),
        "disabled" => Ok(Pseudo::Disabled),
        "firstChild" => Ok(Pseudo::FirstChild),
        "lastChild" => Ok(Pseudo::LastChild),
        _ => Err(unknown("pseudo selector", tag)),
    }
}

fn decode_media_query(session: &TlcSession, value: Value) -> Result<MediaQuery, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "minWidth" => Ok(MediaQuery::MinWidth(integer(
            session,
            force_payload_field(session, payload, "pixels")?,
        )?)),
        "maxWidth" => Ok(MediaQuery::MaxWidth(integer(
            session,
            force_payload_field(session, payload, "pixels")?,
        )?)),
        "prefersDark" => Ok(MediaQuery::PrefersDark),
        "prefersLight" => Ok(MediaQuery::PrefersLight),
        "prefersReducedMotion" => Ok(MediaQuery::PrefersReducedMotion),
        "and" => Ok(MediaQuery::And(decode_list(
            session,
            force_payload_field(session, payload, "queries")?,
            |value| decode_media_query(session, value),
        )?)),
        "unsafeRaw" => Ok(MediaQuery::UnsafeRaw(text(
            session,
            force_payload_field(session, payload, "css")?,
        )?)),
        _ => Err(unknown("media query", tag)),
    }
}

fn decode_declaration(session: &TlcSession, value: Value) -> Result<Declaration, DecodeError> {
    let value = session.force(value)?;
    let fields = expect_record(&value)?;
    Ok(Declaration {
        property: decode_property(session, force_field(session, fields, "property")?)?,
        value: decode_css_value(session, force_field(session, fields, "value")?)?,
        important: boolean(session, force_field(session, fields, "important")?)?,
    })
}

fn decode_property(session: &TlcSession, value: Value) -> Result<String, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    let known = match tag {
        "display" => "display",
        "position" => "position",
        "top" => "top",
        "right" => "right",
        "bottom" => "bottom",
        "left" => "left",
        "width" => "width",
        "minWidth" => "min-width",
        "maxWidth" => "max-width",
        "height" => "height",
        "minHeight" => "min-height",
        "maxHeight" => "max-height",
        "margin" => "margin",
        "padding" => "padding",
        "gap" => "gap",
        "gridTemplateColumns" => "grid-template-columns",
        "gridColumn" => "grid-column",
        "alignItems" => "align-items",
        "justifyContent" => "justify-content",
        "flexDirection" => "flex-direction",
        "flexWrap" => "flex-wrap",
        "color" => "color",
        "background" => "background",
        "border" => "border",
        "borderRadius" => "border-radius",
        "boxShadow" => "box-shadow",
        "fontFamily" => "font-family",
        "fontSize" => "font-size",
        "fontWeight" => "font-weight",
        "lineHeight" => "line-height",
        "letterSpacing" => "letter-spacing",
        "textAlign" => "text-align",
        "textTransform" => "text-transform",
        "textDecoration" => "text-decoration",
        "cursor" => "cursor",
        "overflow" => "overflow",
        "opacity" => "opacity",
        "transform" => "transform",
        "transition" => "transition",
        "animation" => "animation",
        "outline" => "outline",
        "custom" => {
            return text(session, force_payload_field(session, payload, "name")?);
        }
        "unsafeRaw" => {
            return Err(DecodeError::InvalidProgram(
                "unsafeRaw CSS properties are disabled for browser documents".into(),
            ));
        }
        _ => return Err(unknown("CSS property", tag)),
    };
    Ok(known.to_owned())
}

fn decode_css_value(session: &TlcSession, value: Value) -> Result<CssValue, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "keyword" => Ok(CssValue::Keyword(text(
            session,
            force_payload_field(session, payload, "value")?,
        )?)),
        "number" => Ok(CssValue::Number(number(
            session,
            force_payload_field(session, payload, "value")?,
        )?)),
        "length" => Ok(CssValue::Length {
            value: number(session, force_payload_field(session, payload, "value")?)?,
            unit: decode_css_unit(session, force_payload_field(session, payload, "unit")?)?,
        }),
        "duration" => Ok(CssValue::Duration {
            value: number(session, force_payload_field(session, payload, "value")?)?,
            unit: decode_css_unit(session, force_payload_field(session, payload, "unit")?)?,
        }),
        "color" => Ok(CssValue::Color(text(
            session,
            force_payload_field(session, payload, "value")?,
        )?)),
        "string" => Ok(CssValue::String(text(
            session,
            force_payload_field(session, payload, "value")?,
        )?)),
        "sequence" => Ok(CssValue::Sequence {
            separator: decode_separator(
                session,
                force_payload_field(session, payload, "separator")?,
            )?,
            values: decode_list(
                session,
                force_payload_field(session, payload, "values")?,
                |value| decode_css_value(session, value),
            )?,
        }),
        "function" => Ok(CssValue::Function {
            name: text(session, force_payload_field(session, payload, "name")?)?,
            arguments: decode_list(
                session,
                force_payload_field(session, payload, "arguments")?,
                |value| decode_css_value(session, value),
            )?,
        }),
        "variable" => Ok(CssValue::Variable {
            name: text(session, force_payload_field(session, payload, "name")?)?,
            fallback: optional_value(session, force_payload_field(session, payload, "fallback")?)?
                .map(|value| decode_css_value(session, value).map(Box::new))
                .transpose()?,
        }),
        "unsafeRaw" => Ok(CssValue::UnsafeRaw(text(
            session,
            force_payload_field(session, payload, "css")?,
        )?)),
        _ => Err(unknown("CSS value", tag)),
    }
}

fn decode_separator(session: &TlcSession, value: Value) -> Result<Separator, DecodeError> {
    let value = session.force(value)?;
    let (tag, _) = expect_tagged(&value)?;
    match tag {
        "space" => Ok(Separator::Space),
        "comma" => Ok(Separator::Comma),
        "slash" => Ok(Separator::Slash),
        _ => Err(unknown("CSS separator", tag)),
    }
}

fn decode_css_unit(session: &TlcSession, value: Value) -> Result<String, DecodeError> {
    let unit = decode_named_tag(session, value)?;
    Ok(if unit == "percent" { "%".into() } else { unit })
}

fn decode_attribute_name(session: &TlcSession, value: Value) -> Result<String, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    let name = match tag {
        "id" => "id",
        "class" => "class",
        "href" => "href",
        "src" => "src",
        "alt" => "alt",
        "name" => "name",
        "placeholder" => "placeholder",
        "inputType" => "type",
        "role" => "role",
        "title" => "title",
        "forId" => "for",
        "target" => "target",
        "rel" => "rel",
        "disabled" => "disabled",
        "required" => "required",
        "readOnly" => "readonly",
        "value" => "value",
        "checked" => "checked",
        "aria" => {
            return Ok(format!(
                "aria-{}",
                text(session, force_payload_field(session, payload, "name")?)?
            ));
        }
        "data" => {
            return Ok(format!(
                "data-{}",
                text(session, force_payload_field(session, payload, "name")?)?
            ));
        }
        _ => return Err(unknown("attribute name", tag)),
    };
    Ok(name.to_owned())
}

fn decode_named_tag(session: &TlcSession, value: Value) -> Result<String, DecodeError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "custom" => text(session, force_payload_field(session, payload, "name")?),
        other => Ok(camel_to_kebab(other)),
    }
}

fn optional_named_tag(session: &TlcSession, value: Value) -> Result<Option<String>, DecodeError> {
    optional_value(session, value)?
        .map(|value| decode_named_tag(session, value))
        .transpose()
}

fn optional_text(session: &TlcSession, value: Value) -> Result<Option<String>, DecodeError> {
    optional_value(session, value)?
        .map(|value| text(session, value))
        .transpose()
}

fn optional_value(session: &TlcSession, value: Value) -> Result<Option<Value>, DecodeError> {
    let value = session.force(value)?;
    match &value {
        Value::Atom(tag) if tag.as_ref() == "none" => Ok(None),
        Value::TaggedValue { tag, payload } if tag.as_ref() == "some" => {
            Ok(Some(force_payload_field(session, payload, "0")?))
        }
        other => Err(type_error("Optional", other)),
    }
}

fn decode_list<T>(
    session: &TlcSession,
    value: Value,
    mut decode: impl FnMut(Value) -> Result<T, DecodeError>,
) -> Result<Vec<T>, DecodeError> {
    let value = session.force(value)?;
    let Value::List(items) = value else {
        return Err(type_error("List", &value));
    };
    items
        .iter()
        .map(|item| session.force_thunk(item).map_err(DecodeError::from))
        .map(|value| value.and_then(&mut decode))
        .collect()
}

fn force_field(
    session: &TlcSession,
    fields: &[(Rc<str>, Thunk)],
    name: &str,
) -> Result<Value, DecodeError> {
    let (_, value) = fields
        .iter()
        .find(|(field, _)| field.as_ref() == name)
        .ok_or_else(|| DecodeError::MissingField(name.to_owned()))?;
    Ok(session.force_thunk(value)?)
}

fn force_payload_field(
    session: &TlcSession,
    fields: &[(Rc<str>, Thunk)],
    name: &str,
) -> Result<Value, DecodeError> {
    force_field(session, fields, name)
}

fn text(session: &TlcSession, value: Value) -> Result<String, DecodeError> {
    let value = session.force(value)?;
    match value {
        Value::Text(value) => Ok(value.to_string()),
        other => Err(type_error("Text", &other)),
    }
}

fn boolean(session: &TlcSession, value: Value) -> Result<bool, DecodeError> {
    let value = session.force(value)?;
    match value {
        Value::Bool(value) => Ok(value),
        other => Err(type_error("Bool", &other)),
    }
}

fn integer(session: &TlcSession, value: Value) -> Result<i64, DecodeError> {
    let value = session.force(value)?;
    match value {
        Value::Int(value) => Ok(value),
        other => Err(type_error("Int", &other)),
    }
}

fn number(session: &TlcSession, value: Value) -> Result<f64, DecodeError> {
    let value = session.force(value)?;
    match value {
        Value::Float(value) => Ok(value),
        Value::Int(value) => Ok(value as f64),
        other => Err(type_error("Float", &other)),
    }
}

fn expect_record(value: &Value) -> Result<ValueFields<'_>, DecodeError> {
    match value {
        Value::Record(fields) => Ok(fields.as_slice()),
        other => Err(type_error("Record", other)),
    }
}

fn expect_tagged(value: &Value) -> Result<(&str, ValueFields<'_>), DecodeError> {
    match value {
        Value::Atom(tag) => Ok((tag.as_ref(), &[])),
        Value::TaggedValue { tag, payload } => Ok((tag.as_ref(), payload.as_slice())),
        other => Err(type_error("tagged value", other)),
    }
}

fn is_callable(value: &Value) -> bool {
    matches!(
        value,
        Value::Closure(_) | Value::TlcClosure(_) | Value::Builtin(_) | Value::BuiltinPartial { .. }
    )
}

fn unit() -> Value {
    Value::Tuple(Rc::from([]))
}

fn unknown(kind: &'static str, tag: &str) -> DecodeError {
    DecodeError::UnknownVariant {
        kind,
        tag: tag.to_owned(),
    }
}

fn type_error(expected: &'static str, value: &Value) -> DecodeError {
    DecodeError::Type {
        expected,
        found: value_kind(value),
    }
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

fn camel_to_kebab(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 4);
    for ch in value.chars() {
        if ch.is_ascii_uppercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}
