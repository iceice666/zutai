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

#[cfg(test)]
mod tests {
    use crate::ast::{Block, Pair, Value};

    fn make_block(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Block {
        Block(
            pairs
                .into_iter()
                .map(|(name, value)| Pair {
                    field_name: name.to_string(),
                    value,
                })
                .collect(),
        )
    }

    #[test]
    fn display_empty_block() {
        let b = Block(vec![]);
        let s = b.to_string();
        assert!(s.contains("Block (empty)"), "got: {s:?}");
    }

    #[test]
    fn display_block_with_integer() {
        let b = make_block([("count", Value::Integer(42))]);
        let s = b.to_string();
        assert!(s.contains("Integer(42)"), "got: {s:?}");
        assert!(s.contains("count"), "got: {s:?}");
    }

    #[test]
    fn display_block_with_float() {
        let b = make_block([("ratio", Value::Float(1.5))]);
        let s = b.to_string();
        assert!(s.contains("Float(1.5)"), "got: {s:?}");
    }

    #[test]
    fn display_block_with_true() {
        let b = make_block([("flag", Value::True)]);
        let s = b.to_string();
        assert!(s.contains("True"), "got: {s:?}");
    }

    #[test]
    fn display_block_with_false() {
        let b = make_block([("flag", Value::False)]);
        let s = b.to_string();
        assert!(s.contains("False"), "got: {s:?}");
    }

    #[test]
    fn display_block_with_atom() {
        let b = make_block([("tag", Value::Atom("hello".to_string()))]);
        let s = b.to_string();
        assert!(s.contains("Atom(hello)"), "got: {s:?}");
    }

    #[test]
    fn display_block_with_string() {
        let b = make_block([("name", Value::String("world".to_string()))]);
        let s = b.to_string();
        assert!(s.contains("String(\"world\")"), "got: {s:?}");
    }

    #[test]
    fn display_string_with_escape_sequences() {
        let b = make_block([("msg", Value::String("say \"hi\"\nnewline".to_string()))]);
        let s = b.to_string();
        // quote and newline should be escaped
        assert!(s.contains("\\\""), "quote not escaped in: {s:?}");
        assert!(s.contains("\\n"), "newline not escaped in: {s:?}");
    }

    #[test]
    fn display_string_with_backslash_escape() {
        let b = make_block([("path", Value::String("a\\b".to_string()))]);
        let s = b.to_string();
        assert!(s.contains("\\\\"), "backslash not escaped in: {s:?}");
    }

    #[test]
    fn display_string_with_tab_cr_escapes() {
        let b = make_block([("v", Value::String("\t\r".to_string()))]);
        let s = b.to_string();
        assert!(s.contains("\\t"), "tab not escaped in: {s:?}");
        assert!(s.contains("\\r"), "cr not escaped in: {s:?}");
    }

    #[test]
    fn display_string_with_backspace_formfeed_escapes() {
        let b = make_block([("v", Value::String("\x08\x0C".to_string()))]);
        let s = b.to_string();
        assert!(s.contains("\\b"), "backspace not escaped in: {s:?}");
        assert!(s.contains("\\f"), "formfeed not escaped in: {s:?}");
    }

    #[test]
    fn display_string_with_control_char_escape() {
        // \x01 is a control character that should become 
        let b = make_block([("ctrl", Value::String("\x01".to_string()))]);
        let s = b.to_string();
        assert!(s.contains("\\u0001"), "control char not escaped in: {s:?}");
    }

    #[test]
    fn display_block_with_empty_array() {
        let b = make_block([("items", Value::Array(vec![]))]);
        let s = b.to_string();
        // Array with 0 elements: "Array[0]" or shows (empty) in child
        assert!(s.contains("Array[0]"), "got: {s:?}");
    }

    #[test]
    fn display_block_with_integer_array() {
        let b = make_block([(
            "nums",
            Value::Array(vec![Value::Integer(1), Value::Integer(2)]),
        )]);
        let s = b.to_string();
        assert!(s.contains("Array[2]"), "got: {s:?}");
        assert!(s.contains("Integer(1)"), "got: {s:?}");
        assert!(s.contains("Integer(2)"), "got: {s:?}");
    }

    #[test]
    fn display_array_with_nested_array() {
        let b = make_block([(
            "nested",
            Value::Array(vec![Value::Array(vec![Value::Integer(99)])]),
        )]);
        let s = b.to_string();
        assert!(s.contains("Array[1]"), "got: {s:?}");
        assert!(s.contains("Integer(99)"), "got: {s:?}");
    }

    #[test]
    fn display_array_with_nested_block() {
        let inner = make_block([("x", Value::Integer(7))]);
        let b = make_block([("arr", Value::Array(vec![Value::Block(inner)]))]);
        let s = b.to_string();
        assert!(s.contains("Integer(7)"), "got: {s:?}");
    }

    #[test]
    fn display_pair_with_nested_empty_block() {
        let inner = Block(vec![]);
        let b = make_block([("child", Value::Block(inner))]);
        let s = b.to_string();
        assert!(s.contains("(empty)"), "got: {s:?}");
    }

    #[test]
    fn display_pair_with_nested_block() {
        let inner = make_block([("val", Value::Integer(5))]);
        let b = make_block([("child", Value::Block(inner))]);
        let s = b.to_string();
        assert!(s.contains("Integer(5)"), "got: {s:?}");
    }

    #[test]
    fn display_block_with_multiple_entries_shows_connectors() {
        let b = make_block([("a", Value::Integer(1)), ("b", Value::Integer(2))]);
        let s = b.to_string();
        // non-last entry uses ├─, last uses └─
        assert!(s.contains('├'), "expected ├ connector in: {s:?}");
        assert!(s.contains('└'), "expected └ connector in: {s:?}");
    }

    #[test]
    fn display_nested_array_and_block_exact_tree() {
        let nested = make_block([("z", Value::String("ok".to_string()))]);
        let b = make_block([
            ("a", Value::Array(vec![Value::True, Value::Block(nested)])),
            ("b", Value::Block(Block(vec![]))),
        ]);

        assert_eq!(
            b.to_string(),
            "Block\n├─ a = Array[2]\n│   ├─ [0] = True\n│   └─ [1] = Block\n│       └─ z = String(\"ok\")\n└─ b = Block\n    (empty)\n"
        );
    }
}
