use std::fmt::Write;

use zutai_types::{Block, Pair, Value};

use crate::parser;

const INDENT: &str = "  ";

/// Format an immediate-mode document in canonical, order-preserving form.
pub fn format_source(input: &str) -> Result<String, String> {
    let mut remaining = input;
    let block = parser::parse(&mut remaining).map_err(|error| error.to_string())?;
    if !remaining.trim().is_empty() {
        return Err("trailing data after parsed document".to_owned());
    }

    let mut output = String::new();
    write_block(&mut output, &block, 0);
    output.push('\n');
    Ok(output)
}

fn write_block(output: &mut String, block: &Block, depth: usize) {
    if block.is_empty() {
        output.push_str("{}");
        return;
    }

    output.push_str("{\n");
    for pair in block.iter() {
        write_indent(output, depth + 1);
        write_pair(output, pair, depth + 1);
        output.push('\n');
    }
    write_indent(output, depth);
    output.push('}');
}

fn write_pair(output: &mut String, pair: &Pair, depth: usize) {
    output.push_str(&pair.field_name);
    output.push_str(" = ");
    write_value(output, &pair.value, depth);
    output.push(';');
}

fn write_array(output: &mut String, values: &[Value], depth: usize) {
    if values.is_empty() {
        output.push_str("[]");
        return;
    }

    output.push_str("[\n");
    for value in values {
        write_indent(output, depth + 1);
        write_value(output, value, depth + 1);
        output.push_str(";\n");
    }
    write_indent(output, depth);
    output.push(']');
}

fn write_value(output: &mut String, value: &Value, depth: usize) {
    match value {
        Value::True => output.push_str("true"),
        Value::False => output.push_str("false"),
        Value::Atom(atom) => {
            output.push('#');
            output.push_str(atom);
        }
        Value::String(string) => write_string(output, string),
        Value::Float(number) => {
            write!(output, "{number:?}").expect("writing to String cannot fail");
        }
        Value::Integer(number) => {
            write!(output, "{number}").expect("writing to String cannot fail");
        }
        Value::Array(values) => write_array(output, values, depth),
        Value::Block(block) => write_block(output, block, depth),
    }
}

fn write_string(output: &mut String, value: &str) {
    output.push('"');
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch <= '\u{1f}' => {
                write!(output, "\\u{:04x}", ch as u32).expect("writing to String cannot fail");
            }
            ch => output.push(ch),
        }
    }
    output.push('"');
}

fn write_indent(output: &mut String, depth: usize) {
    for _ in 0..depth {
        output.push_str(INDENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_preserves_field_and_item_order_and_is_idempotent() {
        let source = "{second=[#b;#a;];first={z=2;y=1;};}";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            concat!(
                "{\n",
                "  second = [\n",
                "    #b;\n",
                "    #a;\n",
                "  ];\n",
                "  first = {\n",
                "    z = 2;\n",
                "    y = 1;\n",
                "  };\n",
                "}\n",
            )
        );
        assert!(formatted.find("second").unwrap() < formatted.find("first").unwrap());
        assert!(formatted.find("#b").unwrap() < formatted.find("#a").unwrap());
        assert_eq!(format_source(&formatted).unwrap(), formatted);

        let mut before = source;
        let mut after = formatted.as_str();
        assert_eq!(
            parser::parse(&mut before).unwrap(),
            parser::parse(&mut after).unwrap()
        );
    }

    #[test]
    fn format_canonicalizes_strings_and_floats_without_changing_values() {
        let source = "{text=\"line\\n\\u263a\";whole=1e2;fraction=-0.5;}";
        let formatted = format_source(source).unwrap();
        let mut before = source;
        let mut after = formatted.as_str();
        assert_eq!(
            parser::parse(&mut before).unwrap(),
            parser::parse(&mut after).unwrap()
        );
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }
}
