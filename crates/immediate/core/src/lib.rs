pub use zutai_types::{Block, Pair, Value};

#[cfg(not(any(feature = "syntax", feature = "simd")))]
compile_error!("zutai-im: enable at least one of the `syntax` or `simd` features");

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[cfg(feature = "syntax")]
    #[error("syntax parse error: {0}")]
    Syntax(String),
    #[cfg(feature = "syntax")]
    #[error("trailing data after parsed document")]
    TrailingData,
    #[cfg(feature = "simd")]
    #[error("simd parse error: {0}")]
    Simd(#[from] zutai_im_simd::ParseError),
}

pub fn parse(input: &str) -> Result<Block, Error> {
    dispatch(input)
}

#[cfg(feature = "simd")]
fn dispatch(input: &str) -> Result<Block, Error> {
    Ok(zutai_im_simd::parse(input)?)
}

#[cfg(all(not(feature = "simd"), feature = "syntax"))]
fn dispatch(input: &str) -> Result<Block, Error> {
    parse_syntax(input)
}

#[cfg(feature = "syntax")]
pub fn parse_syntax(input: &str) -> Result<Block, Error> {
    let mut s = input;
    let b = zutai_im_syntax::parser::parse(&mut s).map_err(|e| Error::Syntax(e.to_string()))?;
    if !s.trim().is_empty() {
        return Err(Error::TrailingData);
    }
    Ok(b)
}

#[cfg(feature = "simd")]
pub fn parse_simd(input: &str) -> Result<Block, Error> {
    Ok(zutai_im_simd::parse(input)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURSED: &str = include_str!("../../fixtures/cursed.zti");

    fn field<'a>(block: &'a Block, name: &str) -> &'a Value {
        &block
            .iter()
            .find(|p| p.field_name == name)
            .unwrap_or_else(|| panic!("field {name:?} not found"))
            .value
    }

    fn as_array(v: &Value) -> &[Value] {
        match v {
            Value::Array(a) => a,
            other => panic!("expected Array, got {other:?}"),
        }
    }

    fn as_block(v: &Value) -> &Block {
        match v {
            Value::Block(b) => b,
            other => panic!("expected Block, got {other:?}"),
        }
    }

    // The cursed fixture contains a raw null byte inside the string "null\x00byte"
    // (line ~55 of the fixture). The SIMD parser rejects this as InvalidString; the
    // winnow parser accepts it. Tests below that inspect the AST use parse_syntax
    // explicitly to pin them to winnow behavior regardless of the active dispatch backend.

    #[cfg(feature = "syntax")]
    #[test]
    fn parse_cursed_succeeds() {
        parse_syntax(CURSED).unwrap();
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn cursed_basic_scalars() {
        let doc = parse_syntax(CURSED).unwrap();
        assert_eq!(field(&doc, "none_field"), &Value::Atom("none".into()));
        assert_eq!(field(&doc, "true_field"), &Value::True);
        assert_eq!(field(&doc, "false_field"), &Value::False);
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn cursed_numbers_array() {
        let doc = parse_syntax(CURSED).unwrap();
        let nums = as_array(field(&doc, "numbers"));
        assert_eq!(nums.len(), 20);
        assert_eq!(nums[0], Value::Integer(0));
        assert_eq!(nums[2], Value::Integer(1));
        assert_eq!(nums[3], Value::Integer(-1));
        assert_eq!(nums[4], Value::Integer(123_456_789));
        assert_eq!(nums[5], Value::Integer(-987_654_321));
        assert_eq!(nums[6], Value::Float(0.0));
        assert_eq!(nums[8], Value::Float(314.0 / 100.0));
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn cursed_strings_array_length() {
        let doc = parse_syntax(CURSED).unwrap();
        let strs = as_array(field(&doc, "strings_with_escapes"));
        assert_eq!(strs.len(), 14);
        assert_eq!(strs[0], Value::String(String::new()));
        assert_eq!(strs[3], Value::String("line1\nline2".into()));
        assert_eq!(strs[4], Value::String("tab\there".into()));
        assert_eq!(strs[5], Value::String("quote\"inside".into()));
        assert_eq!(strs[6], Value::String("backslash\\here".into()));
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn cursed_empty_structures() {
        let doc = parse_syntax(CURSED).unwrap();
        let empties = as_block(field(&doc, "empty_structures"));
        assert_eq!(as_block(field(empties, "empty_record")).len(), 0);
        assert_eq!(as_array(field(empties, "empty_list")).len(), 0);
        let list_of_empty = as_array(field(empties, "list_of_empty_records"));
        assert_eq!(list_of_empty.len(), 3);
        assert_eq!(as_block(&list_of_empty[0]).len(), 0);
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn cursed_all_in_one_list() {
        let doc = parse_syntax(CURSED).unwrap();
        let mixed = as_array(field(&doc, "all_in_one_list"));
        assert_eq!(mixed.len(), 12);
        assert_eq!(mixed[0], Value::Atom("none".into()));
        assert_eq!(mixed[1], Value::True);
        assert_eq!(mixed[2], Value::False);
        assert_eq!(mixed[3], Value::Integer(0));
        assert_eq!(mixed[4], Value::Integer(-1));
        assert_eq!(mixed[5], Value::Float(314.0 / 100.0));
        assert_eq!(mixed[6], Value::String("string".into()));
        assert_eq!(mixed[7], Value::Atom("atom".into()));
        assert_eq!(as_block(&mixed[8]).len(), 0);
        assert_eq!(as_array(&mixed[9]).len(), 0);
        assert_eq!(as_block(&mixed[10]).len(), 1);
        assert_eq!(as_array(&mixed[11]).len(), 3);
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn cursed_nested_same_key_depth() {
        let doc = parse_syntax(CURSED).unwrap();
        let mut cur = as_block(field(&doc, "nested_same_key"));
        for _ in 0..7 {
            cur = as_block(field(cur, "a"));
        }
        assert_eq!(field(cur, "a"), &Value::True);
    }

    // The SIMD parser rejects raw null bytes in strings (correct behavior per spec).
    #[cfg(feature = "simd")]
    #[test]
    fn cursed_simd_rejects_null_byte() {
        assert!(parse_simd(CURSED).is_err());
    }

    #[test]
    fn parse_dispatch_parses_simple_block() {
        let doc = parse("{ a = 1; ok = true; }").unwrap();
        assert_eq!(field(&doc, "a"), &Value::Integer(1));
        assert_eq!(field(&doc, "ok"), &Value::True);
    }

    #[cfg(feature = "simd")]
    #[test]
    fn parse_simd_error_is_wrapped() {
        let err = parse_simd("{ a = 1 }").unwrap_err();
        assert!(err.to_string().contains("simd parse error"));
    }
}
