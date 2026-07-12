use std::fmt::Write;

use crate::{
    CssValue, Declaration, Keyframe, KeyframeStop, MediaQuery, Pseudo, Rule, Selector, Separator,
    Stylesheet,
};

#[derive(Debug, thiserror::Error)]
pub enum CssRenderError {
    #[error("CSS numeric values must be finite")]
    NonFiniteNumber,
    #[error("invalid CSS identifier `{0}`")]
    InvalidIdentifier(String),
    #[error("CSS percentages must be between 0 and 100")]
    InvalidPercentage,
    #[error("unsafe CSS is disabled for this render")]
    UnsafeCssDisabled,
}

pub fn render_stylesheet(sheet: &Stylesheet, allow_unsafe: bool) -> Result<String, CssRenderError> {
    let mut out = String::new();
    for rule in &sheet.rules {
        render_rule(rule, allow_unsafe, &mut out)?;
    }
    Ok(out)
}

fn render_rule(rule: &Rule, allow_unsafe: bool, out: &mut String) -> Result<(), CssRenderError> {
    match rule {
        Rule::Style {
            selectors,
            declarations,
        } => {
            for (index, selector) in selectors.iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                render_selector(selector, allow_unsafe, out)?;
            }
            out.push('{');
            render_declarations(declarations, allow_unsafe, out)?;
            out.push('}');
        }
        Rule::Media { query, rules } => {
            out.push_str("@media ");
            render_media_query(query, allow_unsafe, out)?;
            out.push('{');
            for rule in rules {
                render_rule(rule, allow_unsafe, out)?;
            }
            out.push('}');
        }
        Rule::Keyframes { name, frames } => {
            validate_identifier(name)?;
            write!(out, "@keyframes {name}{{").expect("writing to String cannot fail");
            for frame in frames {
                render_keyframe(frame, allow_unsafe, out)?;
            }
            out.push('}');
        }
        Rule::UnsafeRaw(css) if allow_unsafe => out.push_str(css),
        Rule::UnsafeRaw(_) => return Err(CssRenderError::UnsafeCssDisabled),
    }
    Ok(())
}

fn render_selector(
    selector: &Selector,
    allow_unsafe: bool,
    out: &mut String,
) -> Result<(), CssRenderError> {
    match selector {
        Selector::All => out.push('*'),
        Selector::Tag(name) => {
            validate_identifier(name)?;
            out.push_str(name);
        }
        Selector::Class(name) => {
            validate_identifier(name)?;
            out.push('.');
            out.push_str(name);
        }
        Selector::Id(name) => {
            validate_identifier(name)?;
            out.push('#');
            out.push_str(name);
        }
        Selector::Compound(parts) => {
            for part in parts {
                render_selector(part, allow_unsafe, out)?;
            }
        }
        Selector::Descendant(ancestor, descendant) => {
            render_selector(ancestor, allow_unsafe, out)?;
            out.push(' ');
            render_selector(descendant, allow_unsafe, out)?;
        }
        Selector::Child(parent, child) => {
            render_selector(parent, allow_unsafe, out)?;
            out.push('>');
            render_selector(child, allow_unsafe, out)?;
        }
        Selector::Pseudo(base, pseudo) => {
            render_selector(base, allow_unsafe, out)?;
            out.push(':');
            out.push_str(match pseudo {
                Pseudo::Hover => "hover",
                Pseudo::Focus => "focus",
                Pseudo::FocusVisible => "focus-visible",
                Pseudo::Disabled => "disabled",
                Pseudo::FirstChild => "first-child",
                Pseudo::LastChild => "last-child",
            });
        }
        Selector::UnsafeRaw(css) if allow_unsafe => out.push_str(css),
        Selector::UnsafeRaw(_) => return Err(CssRenderError::UnsafeCssDisabled),
    }
    Ok(())
}

fn render_media_query(
    query: &MediaQuery,
    allow_unsafe: bool,
    out: &mut String,
) -> Result<(), CssRenderError> {
    match query {
        MediaQuery::MinWidth(px) => write!(out, "(min-width:{px}px)"),
        MediaQuery::MaxWidth(px) => write!(out, "(max-width:{px}px)"),
        MediaQuery::PrefersDark => write!(out, "(prefers-color-scheme:dark)"),
        MediaQuery::PrefersLight => write!(out, "(prefers-color-scheme:light)"),
        MediaQuery::PrefersReducedMotion => write!(out, "(prefers-reduced-motion:reduce)"),
        MediaQuery::And(items) => {
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push_str(" and ");
                }
                render_media_query(item, allow_unsafe, out)?;
            }
            return Ok(());
        }
        MediaQuery::UnsafeRaw(css) if allow_unsafe => {
            out.push_str(css);
            return Ok(());
        }
        MediaQuery::UnsafeRaw(_) => return Err(CssRenderError::UnsafeCssDisabled),
    }
    .expect("writing to String cannot fail");
    Ok(())
}

fn render_keyframe(
    frame: &Keyframe,
    allow_unsafe: bool,
    out: &mut String,
) -> Result<(), CssRenderError> {
    match frame.stop {
        KeyframeStop::From => out.push_str("from"),
        KeyframeStop::To => out.push_str("to"),
        KeyframeStop::Percent(value) => {
            if !value.is_finite() {
                return Err(CssRenderError::NonFiniteNumber);
            }
            if !(0.0..=100.0).contains(&value) {
                return Err(CssRenderError::InvalidPercentage);
            }
            write_number(value, out)?;
            out.push('%');
        }
    }
    out.push('{');
    render_declarations(&frame.declarations, allow_unsafe, out)?;
    out.push('}');
    Ok(())
}

pub(crate) fn render_declarations(
    declarations: &[Declaration],
    allow_unsafe: bool,
    out: &mut String,
) -> Result<(), CssRenderError> {
    for declaration in declarations {
        validate_property(&declaration.property)?;
        out.push_str(&declaration.property);
        out.push(':');
        render_value(&declaration.value, allow_unsafe, out)?;
        if declaration.important {
            out.push_str("!important");
        }
        out.push(';');
    }
    Ok(())
}

fn render_value(
    value: &CssValue,
    allow_unsafe: bool,
    out: &mut String,
) -> Result<(), CssRenderError> {
    match value {
        CssValue::Keyword(value) | CssValue::Color(value) => {
            if value.chars().any(|c| matches!(c, ';' | '{' | '}')) {
                return Err(CssRenderError::InvalidIdentifier(value.clone()));
            }
            out.push_str(value);
        }
        CssValue::Number(value) => write_number(*value, out)?,
        CssValue::Length { value, unit } | CssValue::Duration { value, unit } => {
            write_number(*value, out)?;
            if unit == "%" {
                out.push('%');
            } else {
                validate_identifier(unit)?;
                out.push_str(unit);
            }
        }
        CssValue::String(value) => quote_css_string(value, out),
        CssValue::Sequence { separator, values } => {
            let separator = match separator {
                Separator::Space => " ",
                Separator::Comma => ",",
                Separator::Slash => "/",
            };
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    out.push_str(separator);
                }
                render_value(value, allow_unsafe, out)?;
            }
        }
        CssValue::Function { name, arguments } => {
            validate_identifier(name)?;
            out.push_str(name);
            out.push('(');
            for (index, argument) in arguments.iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                render_value(argument, allow_unsafe, out)?;
            }
            out.push(')');
        }
        CssValue::Variable { name, fallback } => {
            validate_custom_property(name)?;
            out.push_str("var(");
            out.push_str(name);
            if let Some(fallback) = fallback {
                out.push(',');
                render_value(fallback, allow_unsafe, out)?;
            }
            out.push(')');
        }
        CssValue::UnsafeRaw(css) if allow_unsafe => out.push_str(css),
        CssValue::UnsafeRaw(_) => return Err(CssRenderError::UnsafeCssDisabled),
    }
    Ok(())
}

fn write_number(value: f64, out: &mut String) -> Result<(), CssRenderError> {
    if !value.is_finite() {
        return Err(CssRenderError::NonFiniteNumber);
    }
    write!(out, "{value}").expect("writing to String cannot fail");
    Ok(())
}

fn quote_css_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\a "),
            '\r' => out.push_str("\\d "),
            '\0' => out.push_str("\\fffd "),
            other => out.push(other),
        }
    }
    out.push('"');
}

fn validate_property(value: &str) -> Result<(), CssRenderError> {
    if value.starts_with("--") {
        validate_custom_property(value)
    } else {
        validate_identifier(value)
    }
}

fn validate_custom_property(value: &str) -> Result<(), CssRenderError> {
    if !value.starts_with("--") || value.len() <= 2 {
        return Err(CssRenderError::InvalidIdentifier(value.to_owned()));
    }
    validate_identifier(&value[2..])
}

fn validate_identifier(value: &str) -> Result<(), CssRenderError> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(CssRenderError::InvalidIdentifier(value.to_owned()));
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '-')
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err(CssRenderError::InvalidIdentifier(value.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_structured_rules_and_reduced_motion() {
        let sheet = Stylesheet {
            rules: vec![
                Rule::Style {
                    selectors: vec![Selector::Pseudo(
                        Box::new(Selector::Class("button".into())),
                        Pseudo::FocusVisible,
                    )],
                    declarations: vec![Declaration {
                        property: "outline".into(),
                        value: CssValue::Sequence {
                            separator: Separator::Space,
                            values: vec![
                                CssValue::Length {
                                    value: 2.0,
                                    unit: "px".into(),
                                },
                                CssValue::Keyword("solid".into()),
                                CssValue::Color("#5fffd7".into()),
                            ],
                        },
                        important: false,
                    }],
                },
                Rule::Media {
                    query: MediaQuery::PrefersReducedMotion,
                    rules: vec![],
                },
            ],
        };
        assert_eq!(
            render_stylesheet(&sheet, false).unwrap(),
            ".button:focus-visible{outline:2px solid #5fffd7;}@media (prefers-reduced-motion:reduce){}"
        );
    }

    #[test]
    fn rejects_non_finite_and_unsafe_values() {
        let sheet = Stylesheet {
            rules: vec![Rule::UnsafeRaw("body{}".into())],
        };
        assert!(matches!(
            render_stylesheet(&sheet, false),
            Err(CssRenderError::UnsafeCssDisabled)
        ));
        let sheet = Stylesheet {
            rules: vec![Rule::Style {
                selectors: vec![Selector::All],
                declarations: vec![Declaration {
                    property: "opacity".into(),
                    value: CssValue::Number(f64::NAN),
                    important: false,
                }],
            }],
        };
        assert!(matches!(
            render_stylesheet(&sheet, false),
            Err(CssRenderError::NonFiniteNumber)
        ));
    }

    #[test]
    fn renders_percentage_units_with_percent_sign() {
        let sheet = Stylesheet {
            rules: vec![Rule::Style {
                selectors: vec![Selector::All],
                declarations: vec![Declaration {
                    property: "width".into(),
                    value: CssValue::Length {
                        value: 50.0,
                        unit: "%".into(),
                    },
                    important: false,
                }],
            }],
        };
        assert_eq!(render_stylesheet(&sheet, false).unwrap(), "*{width:50%;}");
    }
}
