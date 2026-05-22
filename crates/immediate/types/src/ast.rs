use std::ops::Deref;

#[derive(Debug, PartialEq)]
pub struct Block(pub Vec<Pair>);

impl Deref for Block {
    type Target = Vec<Pair>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, PartialEq)]
pub struct Pair {
    pub field_name: String,
    pub value: Value,
}

#[derive(Debug, PartialEq)]
pub enum Value {
    True,
    False,
    None,
    Atom(String),
    String(String),
    Float(f64),
    Integer(i64),
    Array(Vec<Value>),
    Block(Block),
}
