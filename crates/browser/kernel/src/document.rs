use zutai_eval::Value;

#[derive(Clone, Debug)]
pub struct Document<Msg = Value> {
    pub language: String,
    pub title: String,
    pub head: Vec<HeadNode>,
    pub body_attributes: Vec<StaticAttribute>,
    pub body: Vec<Html<Msg>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HeadNode {
    MetaName {
        name: String,
        content: String,
    },
    MetaProperty {
        property: String,
        content: String,
    },
    Link {
        rel: String,
        href: String,
        mime: Option<String>,
        media: Option<String>,
        sizes: Option<String>,
        cross_origin: Option<String>,
    },
    Base {
        href: String,
        target: Option<String>,
    },
    Style(Stylesheet),
}

#[derive(Clone, Debug)]
pub enum Html<Msg = Value> {
    Text(String),
    Element(Element<Msg>),
}

#[derive(Clone, Debug)]
pub struct Element<Msg = Value> {
    pub tag: String,
    pub key: Option<String>,
    pub attributes: Vec<Attribute<Msg>>,
    pub children: Vec<Html<Msg>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StaticAttribute {
    Text { name: String, value: String },
    Bool { name: String, value: bool },
    Styles(Vec<Declaration>),
}

#[derive(Clone, Debug)]
pub enum Attribute<Msg = Value> {
    Static(StaticAttribute),
    TextProperty { name: String, value: String },
    BoolProperty { name: String, value: bool },
    Event(EventHandler<Msg>),
}

#[derive(Clone, Debug)]
pub enum EventHandler<Msg = Value> {
    Click {
        message: Msg,
        options: EventOptions,
    },
    Input {
        to_message: Msg,
        options: EventOptions,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventOptions {
    pub prevent_default: bool,
    pub stop_propagation: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    Style {
        selectors: Vec<Selector>,
        declarations: Vec<Declaration>,
    },
    Media {
        query: MediaQuery,
        rules: Vec<Rule>,
    },
    Keyframes {
        name: String,
        frames: Vec<Keyframe>,
    },
    UnsafeRaw(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub stop: KeyframeStop,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum KeyframeStop {
    From,
    To,
    Percent(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Selector {
    All,
    Tag(String),
    Class(String),
    Id(String),
    Compound(Vec<Selector>),
    Descendant(Box<Selector>, Box<Selector>),
    Child(Box<Selector>, Box<Selector>),
    Pseudo(Box<Selector>, Pseudo),
    UnsafeRaw(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pseudo {
    Hover,
    Focus,
    FocusVisible,
    Disabled,
    FirstChild,
    LastChild,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaQuery {
    MinWidth(i64),
    MaxWidth(i64),
    PrefersDark,
    PrefersLight,
    PrefersReducedMotion,
    And(Vec<MediaQuery>),
    UnsafeRaw(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub property: String,
    pub value: CssValue,
    pub important: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CssValue {
    Keyword(String),
    Number(f64),
    Length {
        value: f64,
        unit: String,
    },
    Duration {
        value: f64,
        unit: String,
    },
    Color(String),
    String(String),
    Sequence {
        separator: Separator,
        values: Vec<CssValue>,
    },
    Function {
        name: String,
        arguments: Vec<CssValue>,
    },
    Variable {
        name: String,
        fallback: Option<Box<CssValue>>,
    },
    UnsafeRaw(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Separator {
    Space,
    Comma,
    Slash,
}

pub fn is_void_element(tag: &str) -> bool {
    matches!(tag, "br" | "img" | "input")
}
