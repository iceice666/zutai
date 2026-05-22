use crate::ast::*;
use std::fmt;

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_block(self, f, "")
    }
}

fn write_block(block: &Block, f: &mut fmt::Formatter<'_>, indent: &str) -> fmt::Result {
    if block.is_empty() {
        return writeln!(f, "{}Block (empty)", indent);
    }

    writeln!(f, "{}Block", indent)?;
    write_block_entries(block, f, indent)
}

fn write_block_entries(block: &Block, f: &mut fmt::Formatter<'_>, indent: &str) -> fmt::Result {
    for (index, pair) in block.0.iter().enumerate() {
        let is_last = index + 1 == block.len();
        write_pair(pair, f, indent, is_last)?;
    }

    Ok(())
}

fn write_pair(pair: &Pair, f: &mut fmt::Formatter<'_>, indent: &str, is_last: bool) -> fmt::Result {
    let connector = if is_last { "└─ " } else { "├─ " };

    write!(f, "{}{}{} = ", indent, connector, pair.field_name)?;

    match &pair.value {
        Value::Block(block) => {
            writeln!(f, "Block")?;
            if block.is_empty() {
                writeln!(f, "{}(empty)", child_indent(indent, is_last))
            } else {
                write_block_entries(block, f, &child_indent(indent, is_last))
            }
        }
        Value::Array(values) => {
            writeln!(f, "Array[{}]", values.len())?;
            write_array(values, f, &child_indent(indent, is_last))
        }
        value => {
            write_scalar(value, f)?;
            writeln!(f)
        }
    }
}

fn write_array(values: &[Value], f: &mut fmt::Formatter<'_>, indent: &str) -> fmt::Result {
    if values.is_empty() {
        return writeln!(f, "{}(empty)", indent);
    }

    for (index, value) in values.iter().enumerate() {
        let is_last = index + 1 == values.len();
        let connector = if is_last { "└─ " } else { "├─ " };

        write!(f, "{}{}[{}] = ", indent, connector, index)?;

        match value {
            Value::Block(block) => {
                writeln!(f, "Block")?;
                if block.is_empty() {
                    writeln!(f, "{}(empty)", child_indent(indent, is_last))?;
                } else {
                    write_block_entries(block, f, &child_indent(indent, is_last))?;
                }
            }
            Value::Array(inner) => {
                writeln!(f, "Array[{}]", inner.len())?;
                write_array(inner, f, &child_indent(indent, is_last))?;
            }
            inner => {
                write_scalar(inner, f)?;
                writeln!(f)?;
            }
        }
    }

    Ok(())
}

fn child_indent(indent: &str, is_last: bool) -> String {
    if is_last {
        format!("{indent}    ")
    } else {
        format!("{indent}│   ")
    }
}

fn write_scalar(value: &Value, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match value {
        Value::True => write!(f, "True"),
        Value::False => write!(f, "False"),
        Value::None => write!(f, "None"),
        Value::Atom(atom) => write!(f, "Atom({atom})"),
        Value::String(text) => {
            write!(f, "String(\"")?;
            write_escaped_string(text, f)?;
            write!(f, "\")")
        }
        Value::Float(value) => write!(f, "Float({value})"),
        Value::Integer(value) => write!(f, "Integer({value})"),
        Value::Array(values) => write!(f, "Array[{}]", values.len()),
        Value::Block(block) => write!(f, "Block[{}]", block.len()),
    }
}

fn write_escaped_string(value: &str, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for ch in value.chars() {
        match ch {
            '"' => write!(f, "\\\""),
            '\\' => write!(f, "\\\\"),
            '\x08' => write!(f, "\\b"),
            '\x0C' => write!(f, "\\f"),
            '\n' => write!(f, "\\n"),
            '\r' => write!(f, "\\r"),
            '\t' => write!(f, "\\t"),
            ch if ch.is_control() => write!(f, "\\u{:04x}", ch as u32),
            ch => write!(f, "{ch}"),
        }?;
    }

    Ok(())
}
