use std::ops::Deref;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Block(pub Vec<Pair>);

impl Deref for Block {
    type Target = Vec<Pair>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    pub field_name: String,
    pub value: Value,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    True,
    False,
    Atom(String),
    String(String),
    Float(f64),
    Integer(i64),
    Array(Vec<Value>),
    Block(Block),
}

/// Byte offsets into the original `.zti` source.
///
/// Immediate values intentionally remain source-free so their serde shape and
/// runtime representation stay stable. Located parsing returns this parallel
/// tree only to tools that need source attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocatedBlock {
    pub value: Block,
    pub span: ByteSpan,
    pub fields: Vec<LocatedPair>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocatedPair {
    pub field_name: String,
    pub name_span: ByteSpan,
    pub value: LocatedValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocatedValue {
    pub value: Value,
    pub span: ByteSpan,
    pub children: LocatedChildren,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocatedChildren {
    Scalar,
    Array(Vec<LocatedValue>),
    Block(Vec<LocatedPair>),
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let original = Block(vec![
            Pair {
                field_name: "flag".into(),
                value: Value::True,
            },
            Pair {
                field_name: "count".into(),
                value: Value::Integer(42),
            },
            Pair {
                field_name: "ratio".into(),
                value: Value::Float(3.14),
            },
            Pair {
                field_name: "tag".into(),
                value: Value::Atom("release".into()),
            },
            Pair {
                field_name: "items".into(),
                value: Value::Array(vec![Value::Integer(1), Value::Integer(2)]),
            },
            Pair {
                field_name: "nested".into(),
                value: Value::Block(Block(vec![Pair {
                    field_name: "x".into(),
                    value: Value::Atom("none".into()),
                }])),
            },
        ]);
        let json = serde_json::to_string(&original).unwrap();
        let restored: Block = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }
}
