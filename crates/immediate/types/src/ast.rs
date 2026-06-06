use std::ops::Deref;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, PartialEq)]
pub struct Block(pub Vec<Pair>);

impl Deref for Block {
    type Target = Vec<Pair>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, PartialEq)]
pub struct Pair {
    pub field_name: String,
    pub value: Value,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, PartialEq)]
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
