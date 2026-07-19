//! Runtime value model for the Zutai reference interpreter.
//!
//! All heap payloads use `Rc` so that `Value::clone()` is cheap regardless of
//! depth.  This module is deliberately IR-agnostic: nothing here imports THIR
//! directly; the THIR-specific eval walker lives in `eval.rs`.

use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::fmt;
use std::rc::Rc;

use zutai_hir::BindingId;
use zutai_syntax::posit::PositLiteral;
use zutai_thir::{ThirClause, TypeId};
use zutai_tlc::TlcExprId;

use crate::posit::format_posit;
use crate::{EvalError, env::Env, thunk::Thunk};

/// Index into the module registry held by the evaluator.
///
/// Each evaluated `.zt` module is assigned a `ModuleId` so that closures and
/// thunks can record their home module and re-enter the correct arena when
/// forced or applied across a module boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub usize);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeType {
    pub module: ModuleId,
    pub ty: TypeId,
    pub subst: Rc<[(BindingId, RuntimeType)]>,
}

impl RuntimeType {
    pub fn new(module: ModuleId, ty: TypeId) -> Self {
        Self {
            module,
            ty,
            subst: Rc::from([]),
        }
    }

    pub fn with_subst(module: ModuleId, ty: TypeId, subst: Rc<[(BindingId, RuntimeType)]>) -> Self {
        Self { module, ty, subst }
    }

    pub fn with_ty(&self, ty: TypeId) -> Self {
        Self {
            module: self.module,
            ty,
            subst: self.subst.clone(),
        }
    }
}

/// A fully-evaluated or partially-applied Zutai runtime value.
#[derive(Clone, Debug)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Posit(PositLiteral),
    Text(Rc<str>),
    Atom(Rc<str>),
    /// Lazy list — elements are thunks evaluated on demand.
    List(Rc<[crate::thunk::Thunk]>),
    /// Lazy tuple — items may be named.
    Tuple(Rc<[TupleField]>),
    /// Lazy record — only PRESENT fields are stored.
    Record(Rc<Vec<(Rc<str>, crate::thunk::Thunk)>>),
    Closure(Rc<Closure>),
    TypeValue(RuntimeType),
    /// A tagged union value: `#tag { field = value; ... }`.
    TaggedValue {
        tag: Rc<str>,
        payload: Rc<Vec<(Rc<str>, crate::thunk::Thunk)>>,
    },
    /// Internal missing-field sentinel, not the public Optional `#none` case.
    Nothing,
    /// Opaque host resource handle used by explicit host effects.
    HostHandle(HostHandle),
    /// A resolved constraint witness dictionary mapping method/operator name to
    /// the evaluated closure for that field.  Injected into the environment at
    /// bounded call sites so that method dispatch inside the body can fall back
    /// to this dict when the type key is a TypeVar at the call site.
    WitnessDict(FxHashMap<Rc<str>, Value>),
    /// A closure created by the TLC evaluator — stores a single-parameter lambda
    /// body and its captured environment.  Distinct from `Closure` (which is
    /// THIR-based) so the two evaluators never confuse each other's closures.
    TlcClosure(Rc<TlcClosure>),
    /// A compiler-provided builtin function value (the prelude). Seeded into the
    /// top-level environment by name; applied specially by both evaluators.
    Builtin(BuiltinFn),
    BuiltinPartial {
        func: BuiltinFn,
        args: SmallVec<[Thunk; 2]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostHandle {
    pub kind: HostHandleKind,
    pub id: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostHandleKind {
    Reader,
    Writer,
}

/// A compiler-provided builtin function. `print` is re-pointed to the
/// `io.print` effect by the TLC evaluator; source handlers can intercept it and
/// the host run boundary handles residual `io.print`. `fields` and `schema`
/// reflect normalized type values through the THIR evaluator. `overlay` and
/// `overlayDeep` merge config-shaped record values in the reference evaluators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinFn {
    Print,
    LoadZti,
    LoadZt,
    Fields,
    Schema,
    Variants,
    Overlay,
    OverlayDeep,
    /// Stream↔list bridge primitives. The builtin `List` has no source-level
    /// head/tail ops, so these leaf operations let the `.zt` `toList`/`fromList`
    /// combinators build and destructure it. `listHead`/`listTail` are partial
    /// (defined only on a non-nil list); `fromList` guards them with `listIsNil`.
    ListEmpty,
    ListCons,
    ListAppend,
    ListIsNil,
    ListHead,
    ListTail,
    ListFoldlStrict,
    /// Internal scalar bridge primitives used by explicit `stdlib.num`.
    NumAbs,
    NumRem,
    NumPow,
    NumToFloat,
    NumRound,
    NumTruncate,
    /// Internal text bridge primitives used by explicit `stdlib.text`.
    TextLength,
    TextSplit,
    TextJoin,
    TextTrim,
    TextToUpper,
    TextToLower,
    TextContains,
    TextReplace,
    TextShow,
    TextParseInt,
    TextParseFloat,
}

impl BuiltinFn {
    /// Resolve a prelude builtin by its binding name. Mirrors
    /// `zutai_hir::BUILTIN_VALUE_NAMES`; returns `None` for any other name.
    pub fn from_name(name: &str) -> Option<BuiltinFn> {
        match name {
            "print" => Some(BuiltinFn::Print),
            "loadZti" => Some(BuiltinFn::LoadZti),
            "loadZt" => Some(BuiltinFn::LoadZt),
            "fields" => Some(BuiltinFn::Fields),
            "schema" => Some(BuiltinFn::Schema),
            "variants" => Some(BuiltinFn::Variants),
            "overlay" => Some(BuiltinFn::Overlay),
            "overlayDeep" => Some(BuiltinFn::OverlayDeep),
            "listEmpty" => Some(BuiltinFn::ListEmpty),
            "listCons" => Some(BuiltinFn::ListCons),
            "listAppend" => Some(BuiltinFn::ListAppend),
            "listIsNil" => Some(BuiltinFn::ListIsNil),
            "listHead" => Some(BuiltinFn::ListHead),
            "listTail" => Some(BuiltinFn::ListTail),
            "listFoldlStrict" => Some(BuiltinFn::ListFoldlStrict),
            "__numAbs" => Some(BuiltinFn::NumAbs),
            "__numRem" => Some(BuiltinFn::NumRem),
            "__numPow" => Some(BuiltinFn::NumPow),
            "__numToFloat" => Some(BuiltinFn::NumToFloat),
            "__numRound" => Some(BuiltinFn::NumRound),
            "__numTruncate" => Some(BuiltinFn::NumTruncate),
            "__textLength" => Some(BuiltinFn::TextLength),
            "__textSplit" => Some(BuiltinFn::TextSplit),
            "__textJoin" => Some(BuiltinFn::TextJoin),
            "__textTrim" => Some(BuiltinFn::TextTrim),
            "__textToUpper" => Some(BuiltinFn::TextToUpper),
            "__textToLower" => Some(BuiltinFn::TextToLower),
            "__textContains" => Some(BuiltinFn::TextContains),
            "__textReplace" => Some(BuiltinFn::TextReplace),
            "__textShow" => Some(BuiltinFn::TextShow),
            "__textParseInt" => Some(BuiltinFn::TextParseInt),
            "__textParseFloat" => Some(BuiltinFn::TextParseFloat),
            _ => None,
        }
    }

    pub fn arity(self) -> usize {
        match self {
            BuiltinFn::Print
            | BuiltinFn::LoadZti
            | BuiltinFn::LoadZt
            | BuiltinFn::Fields
            | BuiltinFn::Variants
            | BuiltinFn::Schema => 1,
            BuiltinFn::Overlay | BuiltinFn::OverlayDeep => 2,
            BuiltinFn::ListEmpty
            | BuiltinFn::ListIsNil
            | BuiltinFn::ListHead
            | BuiltinFn::ListTail
            | BuiltinFn::NumAbs
            | BuiltinFn::NumToFloat
            | BuiltinFn::NumRound
            | BuiltinFn::NumTruncate
            | BuiltinFn::TextLength
            | BuiltinFn::TextTrim
            | BuiltinFn::TextToUpper
            | BuiltinFn::TextToLower
            | BuiltinFn::TextShow
            | BuiltinFn::TextParseInt
            | BuiltinFn::TextParseFloat => 1,
            BuiltinFn::ListCons
            | BuiltinFn::ListAppend
            | BuiltinFn::NumRem
            | BuiltinFn::NumPow
            | BuiltinFn::TextSplit
            | BuiltinFn::TextJoin
            | BuiltinFn::TextContains => 2,
            BuiltinFn::ListFoldlStrict | BuiltinFn::TextReplace => 3,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BuiltinFn::Print => "print",
            BuiltinFn::LoadZti => "loadZti",
            BuiltinFn::LoadZt => "loadZt",
            BuiltinFn::Fields => "fields",
            BuiltinFn::Variants => "variants",
            BuiltinFn::Schema => "schema",
            BuiltinFn::Overlay => "overlay",
            BuiltinFn::OverlayDeep => "overlayDeep",
            BuiltinFn::ListEmpty => "listEmpty",
            BuiltinFn::ListCons => "listCons",
            BuiltinFn::ListAppend => "listAppend",
            BuiltinFn::ListIsNil => "listIsNil",
            BuiltinFn::ListHead => "listHead",
            BuiltinFn::ListTail => "listTail",
            BuiltinFn::ListFoldlStrict => "listFoldlStrict",
            BuiltinFn::NumAbs => "__numAbs",
            BuiltinFn::NumRem => "__numRem",
            BuiltinFn::NumPow => "__numPow",
            BuiltinFn::NumToFloat => "__numToFloat",
            BuiltinFn::NumRound => "__numRound",
            BuiltinFn::NumTruncate => "__numTruncate",
            BuiltinFn::TextLength => "__textLength",
            BuiltinFn::TextSplit => "__textSplit",
            BuiltinFn::TextJoin => "__textJoin",
            BuiltinFn::TextTrim => "__textTrim",
            BuiltinFn::TextToUpper => "__textToUpper",
            BuiltinFn::TextToLower => "__textToLower",
            BuiltinFn::TextContains => "__textContains",
            BuiltinFn::TextReplace => "__textReplace",
            BuiltinFn::TextShow => "__textShow",
            BuiltinFn::TextParseInt => "__textParseInt",
            BuiltinFn::TextParseFloat => "__textParseFloat",
        }
    }
}

pub(crate) fn eval_num_builtin_values(func: BuiltinFn, args: &[Value]) -> Result<Value, EvalError> {
    match func {
        BuiltinFn::NumAbs => {
            let value = expect_int(&args[0])?;
            value
                .checked_abs()
                .map(Value::Int)
                .ok_or(EvalError::IntOverflow("abs"))
        }
        BuiltinFn::NumRem => {
            let dividend = expect_int(&args[0])?;
            let divisor = expect_int(&args[1])?;
            if divisor == 0 {
                return Err(EvalError::RemByZero);
            }
            dividend
                .checked_rem(divisor)
                .map(Value::Int)
                .ok_or(EvalError::IntOverflow("rem"))
        }
        BuiltinFn::NumPow => {
            let base = expect_int(&args[0])?;
            let exponent = expect_int(&args[1])?;
            if exponent < 0 {
                return Err(EvalError::InvalidNumericArgument(
                    "pow exponent must be non-negative",
                ));
            }
            if exponent > u32::MAX as i64 {
                return Err(EvalError::InvalidNumericArgument(
                    "pow exponent must fit u32",
                ));
            }
            base.checked_pow(exponent as u32)
                .map(Value::Int)
                .ok_or(EvalError::IntOverflow("pow"))
        }
        BuiltinFn::NumToFloat => Ok(Value::Float(expect_int(&args[0])? as f64)),
        BuiltinFn::NumRound => {
            let value = expect_float(&args[0], "round requires finite Float")?;
            float_to_int(value.round(), "round result outside Int range")
        }
        BuiltinFn::NumTruncate => {
            let value = expect_float(&args[0], "truncate requires finite Float")?;
            float_to_int(value.trunc(), "truncate result outside Int range")
        }
        _ => Err(EvalError::Internal(
            "non-numeric builtin dispatched to numeric helper",
        )),
    }
}

fn expect_int(value: &Value) -> Result<i64, EvalError> {
    match value {
        Value::Int(n) => Ok(*n),
        other => Err(EvalError::TypeMismatch {
            expected: "Int",
            found: runtime_value_type_name(other),
        }),
    }
}

fn expect_float(value: &Value, finite_message: &'static str) -> Result<f64, EvalError> {
    match value {
        Value::Float(f) if f.is_finite() => Ok(*f),
        Value::Float(_) => Err(EvalError::InvalidNumericArgument(finite_message)),
        other => Err(EvalError::TypeMismatch {
            expected: "Float",
            found: runtime_value_type_name(other),
        }),
    }
}

fn float_to_int(value: f64, range_message: &'static str) -> Result<Value, EvalError> {
    const INT_MIN_INCLUSIVE: f64 = -9_223_372_036_854_775_808.0;
    const INT_MAX_EXCLUSIVE: f64 = 9_223_372_036_854_775_808.0;
    if !(INT_MIN_INCLUSIVE..INT_MAX_EXCLUSIVE).contains(&value) {
        return Err(EvalError::InvalidNumericArgument(range_message));
    }
    Ok(Value::Int(value as i64))
}

pub(crate) fn eval_text_builtin_values(
    func: BuiltinFn,
    args: &[Value],
) -> Result<Value, EvalError> {
    match func {
        BuiltinFn::TextLength => Ok(Value::Int(expect_text(&args[0])?.chars().count() as i64)),
        BuiltinFn::TextSplit => {
            let separator = expect_text(&args[0])?;
            let value = expect_text(&args[1])?;
            let items: Vec<Thunk> = value
                .split(separator.as_ref())
                .map(|part| Thunk::ready(Value::Text(Rc::from(part))))
                .collect();
            Ok(Value::List(Rc::from(items)))
        }
        BuiltinFn::TextJoin => {
            let separator = expect_text(&args[0])?;
            let Value::List(items) = &args[1] else {
                return Err(EvalError::TypeMismatch {
                    expected: "List",
                    found: runtime_value_type_name(&args[1]),
                });
            };
            let mut parts = Vec::with_capacity(items.len());
            for item in items.iter() {
                match item.peek() {
                    Some(value) => parts.push(expect_text(&value)?.to_string()),
                    None => {
                        return Err(EvalError::Internal("text join received deferred list item"));
                    }
                }
            }
            Ok(Value::Text(Rc::from(parts.join(separator.as_ref()))))
        }
        BuiltinFn::TextTrim => Ok(Value::Text(Rc::from(expect_text(&args[0])?.trim()))),
        BuiltinFn::TextToUpper => Ok(Value::Text(Rc::from(expect_text(&args[0])?.to_uppercase()))),
        BuiltinFn::TextToLower => Ok(Value::Text(Rc::from(expect_text(&args[0])?.to_lowercase()))),
        BuiltinFn::TextContains => {
            let needle = expect_text(&args[0])?;
            let value = expect_text(&args[1])?;
            Ok(Value::Bool(value.contains(needle.as_ref())))
        }
        BuiltinFn::TextReplace => {
            let from = expect_text(&args[0])?;
            let to = expect_text(&args[1])?;
            let value = expect_text(&args[2])?;
            Ok(Value::Text(Rc::from(
                value.replace(from.as_ref(), to.as_ref()),
            )))
        }
        BuiltinFn::TextShow => Ok(Value::Text(Rc::from(quote_text(expect_text(&args[0])?)))),
        BuiltinFn::TextParseInt => match expect_text(&args[0])?.trim().parse::<i64>() {
            Ok(value) => Ok(optional_value(Value::Int(value))),
            Err(_) => Ok(Value::Atom(Rc::from("none"))),
        },
        BuiltinFn::TextParseFloat => match expect_text(&args[0])?.trim().parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(optional_value(Value::Float(value))),
            _ => Ok(Value::Atom(Rc::from("none"))),
        },
        _ => Err(EvalError::Internal(
            "non-text builtin dispatched to text helper",
        )),
    }
}

fn expect_text(value: &Value) -> Result<&Rc<str>, EvalError> {
    match value {
        Value::Text(text) => Ok(text),
        other => Err(EvalError::TypeMismatch {
            expected: "Text",
            found: runtime_value_type_name(other),
        }),
    }
}

fn optional_value(value: Value) -> Value {
    Value::TaggedValue {
        tag: Rc::from("some"),
        payload: Rc::new(vec![(Rc::from("0"), Thunk::ready(value))]),
    }
}

fn quote_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// A single-parameter closure produced by the TLC evaluator.
#[derive(Clone, Debug)]
pub struct TlcClosure {
    pub param: BindingId,
    pub body: TlcExprId,
    pub env: Env,
    pub home: ModuleId,
}

impl Value {
    /// Convert a parsed `.zti` immediate-mode value into a runtime value.
    ///
    /// Blocks become records and arrays become lists (per the import spec);
    /// every element is already fully evaluated, so its thunk is `ready`.
    pub fn from_immediate(value: &zutai_im::Value) -> Value {
        use zutai_im::Value as Im;
        match value {
            Im::True => Value::Bool(true),
            Im::False => Value::Bool(false),
            Im::Integer(n) => Value::Int(*n),
            Im::Float(f) => Value::Float(*f),
            Im::String(s) => Value::Text(Rc::from(s.as_str())),
            Im::Atom(s) => Value::Atom(Rc::from(s.as_str())),
            Im::Array(items) => Value::List(
                items
                    .iter()
                    .map(|item| crate::thunk::Thunk::ready(Value::from_immediate(item)))
                    .collect(),
            ),
            Im::Block(block) => Value::Record(Rc::new(
                block
                    .iter()
                    .map(|pair| {
                        (
                            Rc::from(pair.field_name.as_str()),
                            crate::thunk::Thunk::ready(Value::from_immediate(&pair.value)),
                        )
                    })
                    .collect(),
            )),
        }
    }

    /// Why this fully-forced value cannot be rendered by the runtime ABI, or
    /// `None` when it is first-order serializable data.
    ///
    /// This mirrors the native backend's entry-value contract
    /// (`zutai_codegen::unsupported_entry_type_reason`) at the value level so
    /// the reference interpreter refuses exactly the entries native compilation
    /// refuses: functions/closures, runtime `Type` values, constraint
    /// witnesses, and opaque host handles — including when nested inside a
    /// list, tuple, record, or tagged payload. The returned reason strings match
    /// the native diagnostics so `run`/`json` and `compile` agree.
    ///
    /// Values are deep-forced before this runs, so an unforced nested thunk is
    /// an interpreter bug and is treated as a non-data value.
    pub fn runtime_abi_reason(&self) -> Option<&'static str> {
        fn thunk_reason(thunk: &crate::thunk::Thunk) -> Option<&'static str> {
            match thunk.peek() {
                Some(value) => value.runtime_abi_reason(),
                None => Some(
                    "compiled entry point returns a function, which cannot be shown by the runtime ABI",
                ),
            }
        }
        match self {
            Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::Posit(_)
            | Value::Text(_)
            | Value::Atom(_)
            | Value::Nothing => None,
            Value::List(items) => items.iter().find_map(thunk_reason),
            Value::Tuple(items) => items.iter().find_map(|field| thunk_reason(&field.value)),
            Value::Record(fields) => fields.iter().find_map(|(_, thunk)| thunk_reason(thunk)),
            Value::TaggedValue { payload, .. } => {
                payload.iter().find_map(|(_, thunk)| thunk_reason(thunk))
            }
            Value::TypeValue(_) => {
                Some("compiled entry point returns Type, which cannot be shown by the runtime ABI")
            }
            Value::HostHandle(_) => Some(
                "compiled entry point returns an opaque host handle, which cannot be shown by the runtime ABI",
            ),
            Value::Closure(_)
            | Value::TlcClosure(_)
            | Value::WitnessDict(_)
            | Value::Builtin(_)
            | Value::BuiltinPartial { .. } => Some(
                "compiled entry point returns a function, which cannot be shown by the runtime ABI",
            ),
        }
    }

    /// Convert a fully `force_deep`'d runtime value into natural JSON.
    ///
    /// Booleans, numbers, text, lists, and records map to their JSON
    /// counterparts; atoms become strings with the leading `#`; tagged union
    /// values become `{ "tag": ..., "payload": ... }`. Non-data values
    /// (closures, types, witnesses, builtins) and non-finite floats are
    /// rejected to keep the output JSON-compliant and lossless-free.
    ///
    /// Thunks must already be forced; this never forces. An unforced thunk
    /// means an internal value escaped the deep-force performed by the eval
    /// entry points, which is an interpreter bug.
    pub fn to_json(&self) -> Result<serde_json::Value, EvalError> {
        use serde_json::Value as J;
        match self {
            Value::Bool(b) => Ok(J::Bool(*b)),
            Value::Int(n) => Ok(serde_json::json!(n)),
            Value::Float(x) => {
                if x.is_finite() {
                    Ok(serde_json::json!(x))
                } else {
                    Err(EvalError::Internal(
                        "cannot serialize non-finite float to JSON",
                    ))
                }
            }
            Value::Posit(literal) => Ok(J::String(format_posit(*literal))),
            Value::Text(s) => Ok(J::String(s.to_string())),
            Value::Atom(a) => Ok(J::String(format!("#{a}"))),
            Value::Nothing => Ok(J::Null),
            Value::HostHandle(_) => Err(EvalError::Internal(
                "cannot serialize opaque host handle to JSON",
            )),
            Value::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items.iter() {
                    out.push(forced_json_value(item)?);
                }
                Ok(J::Array(out))
            }
            Value::Tuple(items) => {
                let mut out = Vec::with_capacity(items.len());
                for field in items.iter() {
                    out.push(forced_json_value(&field.value)?);
                }
                Ok(J::Array(out))
            }
            Value::Record(fields) => {
                let mut map = serde_json::Map::with_capacity(fields.len());
                for (name, thunk) in fields.iter() {
                    map.insert(name.to_string(), forced_json_value(thunk)?);
                }
                Ok(J::Object(map))
            }
            Value::TaggedValue { tag, payload } => {
                let payload_json = if payload.is_empty() {
                    J::Null
                } else {
                    let positional = payload
                        .iter()
                        .enumerate()
                        .all(|(index, (name, _))| name.parse::<usize>() == Ok(index));
                    if positional {
                        let mut out = Vec::with_capacity(payload.len());
                        for (_, thunk) in payload.iter() {
                            out.push(forced_json_value(thunk)?);
                        }
                        J::Array(out)
                    } else {
                        let mut map = serde_json::Map::with_capacity(payload.len());
                        for (name, thunk) in payload.iter() {
                            map.insert(name.to_string(), forced_json_value(thunk)?);
                        }
                        J::Object(map)
                    }
                };
                let mut map = serde_json::Map::with_capacity(2);
                map.insert("tag".to_string(), J::String(tag.to_string()));
                map.insert("payload".to_string(), payload_json);
                Ok(J::Object(map))
            }
            Value::Closure(_)
            | Value::TlcClosure(_)
            | Value::TypeValue(_)
            | Value::WitnessDict(_)
            | Value::Builtin(_)
            | Value::BuiltinPartial { .. } => Err(EvalError::Internal(
                "cannot serialize non-data runtime value to JSON",
            )),
        }
    }
}

/// Convert an already-forced thunk to JSON, rejecting unforced thunks.
///
/// Eval entry points deep-force their result, so a `None` peek here means an
/// internal unforced value escaped into the serializer — an interpreter bug.
fn forced_json_value(thunk: &crate::thunk::Thunk) -> Result<serde_json::Value, EvalError> {
    match thunk.peek() {
        Some(value) => value.to_json(),
        None => Err(EvalError::Internal(
            "cannot serialize unforced thunk to JSON",
        )),
    }
}

pub(crate) fn update_record_value(
    metadata: &[(String, bool)],
    base_fields: &Rc<Vec<(Rc<str>, Thunk)>>,
    updates: &[(String, Thunk)],
) -> Value {
    let mut fields = Vec::with_capacity(metadata.len());
    for (name, optional) in metadata {
        if let Some((_, thunk)) = updates.iter().find(|(field, _)| field == name) {
            fields.push((Rc::from(name.as_str()), thunk.clone()));
        } else if let Some((runtime_name, thunk)) = base_fields
            .iter()
            .find(|(field, _)| field.as_ref() == name.as_str())
        {
            fields.push((runtime_name.clone(), thunk.clone()));
        } else if !optional {
            // Well-typed source cannot create a missing required field here.
        }
    }
    Value::Record(Rc::new(fields))
}

pub(crate) fn append_list_values(left: Value, right: Value) -> Result<Value, EvalError> {
    let Value::List(left_items) = left else {
        return Err(EvalError::TypeMismatch {
            expected: "List",
            found: runtime_value_type_name(&left),
        });
    };
    let Value::List(right_items) = right else {
        return Err(EvalError::TypeMismatch {
            expected: "List",
            found: runtime_value_type_name(&right),
        });
    };
    let mut elems = Vec::with_capacity(left_items.len() + right_items.len());
    elems.extend(left_items.iter().cloned());
    elems.extend(right_items.iter().cloned());
    Ok(Value::List(Rc::from(elems)))
}

pub(crate) fn overlay_value<F>(
    base: Value,
    patch: Value,
    deep: bool,
    force: &mut F,
) -> Result<Value, EvalError>
where
    F: FnMut(&Thunk) -> Result<Value, EvalError>,
{
    let Value::Record(base_fields) = base else {
        return Err(EvalError::TypeMismatch {
            expected: "Record",
            found: runtime_value_type_name(&base),
        });
    };
    let Value::Record(patch_fields) = patch else {
        return Err(EvalError::TypeMismatch {
            expected: "Record",
            found: runtime_value_type_name(&patch),
        });
    };
    overlay_record_fields(&base_fields, &patch_fields, deep, force)
}

fn overlay_record_fields<F>(
    base_fields: &Rc<Vec<(Rc<str>, Thunk)>>,
    patch_fields: &Rc<Vec<(Rc<str>, Thunk)>>,
    deep: bool,
    force: &mut F,
) -> Result<Value, EvalError>
where
    F: FnMut(&Thunk) -> Result<Value, EvalError>,
{
    let mut out = Vec::with_capacity(base_fields.len() + patch_fields.len());
    for (base_name, base_thunk) in base_fields.iter() {
        let thunk = match patch_fields
            .iter()
            .find(|(patch_name, _)| patch_name.as_ref() == base_name.as_ref())
        {
            Some((_, patch_thunk)) if deep => {
                let patch_value = force(patch_thunk)?;
                if let Value::Record(patch_nested) = patch_value {
                    match force(base_thunk)? {
                        Value::Record(base_nested) => Thunk::ready(overlay_record_fields(
                            &base_nested,
                            &patch_nested,
                            true,
                            force,
                        )?),
                        _ => patch_thunk.clone(),
                    }
                } else {
                    patch_thunk.clone()
                }
            }
            Some((_, patch_thunk)) => patch_thunk.clone(),
            None => base_thunk.clone(),
        };
        out.push((base_name.clone(), thunk));
    }
    for (patch_name, patch_thunk) in patch_fields.iter() {
        if base_fields
            .iter()
            .all(|(base_name, _)| base_name.as_ref() != patch_name.as_ref())
        {
            if deep && matches!(force(patch_thunk)?, Value::Record(_)) {
                continue;
            }
            out.push((patch_name.clone(), patch_thunk.clone()));
        }
    }
    Ok(Value::Record(Rc::new(out)))
}

fn runtime_value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Posit(_) => "Posit",
        Value::Text(_) => "Text",
        Value::Atom(_) => "Atom",
        Value::List(_) => "List",
        Value::Tuple(_) => "Tuple",
        Value::Record(_) => "Record",
        Value::Closure(_)
        | Value::TlcClosure(_)
        | Value::Builtin(_)
        | Value::BuiltinPartial { .. } => "Function",
        Value::TypeValue(_) => "Type",
        Value::TaggedValue { .. } => "TaggedValue",
        Value::Nothing => "Nothing",
        Value::HostHandle(handle) => match handle.kind {
            HostHandleKind::Reader => "Reader",
            HostHandleKind::Writer => "Writer",
        },
        Value::WitnessDict(_) => "WitnessDict",
    }
}

/// A named or positional tuple field carrying a lazy value.
#[derive(Clone, Debug)]
pub struct TupleField {
    pub name: Option<Rc<str>>,
    pub value: crate::thunk::Thunk,
}

/// A function (or partially-applied curried function).
#[derive(Clone, Debug)]
pub struct Closure {
    /// The `BindingId` of the top-level `Function` declaration this was built
    /// from, or `None` for an anonymous lambda.  Used only for display.
    pub binding: Option<BindingId>,
    /// Total number of value arguments the function expects (the number of
    /// `ThirPatId`s in `clauses[0].patterns`).
    pub arity: usize,
    /// All clauses of the function, shared across partial-application clones.
    pub clauses: Rc<[ThirClause]>,
    /// The environment captured at the point the closure was created.
    pub env: Env,
    /// Arguments already applied (thunks, in order).  Length < arity.
    pub applied: SmallVec<[crate::thunk::Thunk; 4]>,
    /// The module in whose arena the clauses' `ThirExprId`s / `ThirPatId`s live.
    /// `apply_closure` switches the active module to this before evaluating any
    /// clause body or guard so arena look-ups hit the right file.
    pub home: ModuleId,
}

// ─── PartialEq ───────────────────────────────────────────────────────────────

/// Structural `PartialEq` for tests.  Container variants compare forced-thunk
/// contents (via `Thunk::peek`); after `eval_file`/`force_deep` all thunks are
/// in `Forced` state.  Closures compare by pointer identity.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Posit(a), Value::Posit(b)) => a == b,
            (Value::Text(a), Value::Text(b)) => a == b,
            (Value::Atom(a), Value::Atom(b)) => a == b,
            (Value::Nothing, Value::Nothing) => true,
            (Value::HostHandle(a), Value::HostHandle(b)) => a == b,
            (Value::TypeValue(a), Value::TypeValue(b)) => a == b,
            (Value::List(a), Value::List(b)) => a.len() == b.len()
                && a.iter().zip(b.iter()).all(
                    |(ta, tb)| matches!((ta.peek(), tb.peek()), (Some(va), Some(vb)) if va == vb),
                ),
            (Value::Tuple(a), Value::Tuple(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b.iter()).all(|(fa, fb)| {
                        fa.name == fb.name
                            && matches!((fa.value.peek(), fb.value.peek()),
                                (Some(va), Some(vb)) if va == vb)
                    })
            }
            (Value::Record(a), Value::Record(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                // Order-independent: sort by field NAME CONTENT (not pointer
                // address — that is nondeterministic across runs and makes record
                // equality flip true/false). Mirrors `values_equal`.
                let mut fa: Vec<_> = a.iter().collect();
                let mut fb: Vec<_> = b.iter().collect();
                fa.sort_by(|(na, _), (nb, _)| na.as_ref().cmp(nb.as_ref()));
                fb.sort_by(|(na, _), (nb, _)| na.as_ref().cmp(nb.as_ref()));
                fa.iter().zip(fb.iter()).all(|((na, ta), (nb, tb))| {
                    na == nb && matches!((ta.peek(), tb.peek()), (Some(va), Some(vb)) if va == vb)
                })
            }
            (
                Value::TaggedValue {
                    tag: ta,
                    payload: pa,
                },
                Value::TaggedValue {
                    tag: tb,
                    payload: pb,
                },
            ) => {
                ta == tb
                    && pa.len() == pb.len()
                    && pa.iter().zip(pb.iter()).all(|((na, va), (nb, vb))| {
                        na == nb
                            && matches!((va.peek(), vb.peek()), (Some(xa), Some(xb)) if xa == xb)
                    })
            }
            (Value::Closure(a), Value::Closure(b)) => Rc::ptr_eq(a, b),
            (Value::TlcClosure(a), Value::TlcClosure(b)) => Rc::ptr_eq(a, b),
            (Value::Builtin(a), Value::Builtin(b)) => a == b,
            (
                Value::BuiltinPartial { func: af, args: aa },
                Value::BuiltinPartial { func: bf, args: ba },
            ) => af == bf && aa.len() == ba.len(),
            // WitnessDicts are opaque to user-level equality.
            (Value::WitnessDict(_), _) | (_, Value::WitnessDict(_)) => false,
            _ => false,
        }
    }
}

// ─── structural equality (runtime) ───────────────────────────────────────────

/// Structural equality for Zutai values.  Forces thunks as needed.
///
/// Returns `Err` only for non-comparable values (`Closure`, `TypeValue`),
/// which are unreachable for well-typed `==` expressions.
pub fn values_equal(
    a: &Value,
    b: &Value,
    ev: &crate::eval::Evaluator<'_>,
) -> Result<bool, EvalError> {
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => Ok(x == y),
        (Value::Int(x), Value::Int(y)) => Ok(x == y),
        (Value::Float(x), Value::Float(y)) => Ok(x == y),
        (Value::Posit(x), Value::Posit(y)) => Ok(x == y),
        (Value::Text(x), Value::Text(y)) => Ok(x == y),
        (Value::Atom(x), Value::Atom(y)) => Ok(x == y),
        (Value::Nothing, Value::Nothing) => Ok(true),
        (Value::HostHandle(_), _) | (_, Value::HostHandle(_)) => Err(EvalError::Internal(
            "equality on opaque host handle (unreachable in well-typed code)",
        )),
        (Value::List(a), Value::List(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (ta, tb) in a.iter().zip(b.iter()) {
                let va = ta.force(ev)?;
                let vb = tb.force(ev)?;
                if !values_equal(&va, &vb, ev)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Tuple(a), Value::Tuple(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (fa, fb) in a.iter().zip(b.iter()) {
                if fa.name != fb.name {
                    return Ok(false);
                }
                let va = fa.value.force(ev)?;
                let vb = fb.value.force(ev)?;
                if !values_equal(&va, &vb, ev)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Record(a), Value::Record(b)) => {
            // Order-independent: sort by field name, then compare.
            let mut fa: Vec<_> = a.iter().collect();
            let mut fb: Vec<_> = b.iter().collect();
            fa.sort_by_key(|(n, _)| n.as_ref());
            fb.sort_by_key(|(n, _)| n.as_ref());
            if fa.len() != fb.len() {
                return Ok(false);
            }
            for ((na, ta), (nb, tb)) in fa.iter().zip(fb.iter()) {
                if na != nb {
                    return Ok(false);
                }
                let va = ta.force(ev)?;
                let vb = tb.force(ev)?;
                if !values_equal(&va, &vb, ev)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (
            Value::TaggedValue {
                tag: ta,
                payload: pa,
            },
            Value::TaggedValue {
                tag: tb,
                payload: pb,
            },
        ) => {
            if ta != tb {
                return Ok(false);
            }
            let mut fa: Vec<_> = pa.iter().collect();
            let mut fb: Vec<_> = pb.iter().collect();
            fa.sort_by_key(|(n, _)| n.as_ref());
            fb.sort_by_key(|(n, _)| n.as_ref());
            if fa.len() != fb.len() {
                return Ok(false);
            }
            for ((na, ta), (nb, tb)) in fa.iter().zip(fb.iter()) {
                if na != nb {
                    return Ok(false);
                }
                let va = ta.force(ev)?;
                let vb = tb.force(ev)?;
                if !values_equal(&va, &vb, ev)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        // Closures and TypeValues are not comparable under `==` in well-typed
        // programs; this branch is an internal error if ever reached.
        (Value::Closure(_), _) | (_, Value::Closure(_)) => Err(EvalError::Internal(
            "equality on closure (unreachable in well-typed code)",
        )),
        (Value::TlcClosure(_), _) | (_, Value::TlcClosure(_)) => Err(EvalError::Internal(
            "equality on TLC closure (unreachable in well-typed code)",
        )),
        (Value::TypeValue(_), _) | (_, Value::TypeValue(_)) => Err(EvalError::Internal(
            "equality on type value (unreachable in well-typed code)",
        )),
        // WitnessDicts are internal; comparing them is an internal error.
        (Value::WitnessDict(_), _) | (_, Value::WitnessDict(_)) => Err(EvalError::Internal(
            "equality on witness dict (unreachable in well-typed code)",
        )),
        _ => Ok(false),
    }
}

// ─── Display ─────────────────────────────────────────────────────────────────

/// Display a fully `force_deep`'d value.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Float(x) => {
                // Non-finite floats (`inf`, `-inf`, `NaN`) have no integer
                // ambiguity, so emit the bare form rather than appending `.0`
                // (which would produce the malformed `inf.0` / `NaN.0`).
                let s = format!("{x:?}"); // Rust's shortest round-trip repr
                if !x.is_finite() || s.contains('.') || s.contains('e') || s.contains('E') {
                    write!(f, "{s}")
                } else {
                    write!(f, "{s}.0")
                }
            }
            Value::Posit(literal) => write!(f, "{}", format_posit(*literal)),
            Value::Text(s) => {
                write!(f, "\"")?;
                for ch in s.chars() {
                    match ch {
                        '"' => write!(f, "\\\"")?,
                        '\\' => write!(f, "\\\\")?,
                        '\n' => write!(f, "\\n")?,
                        '\r' => write!(f, "\\r")?,
                        '\t' => write!(f, "\\t")?,
                        c => write!(f, "{c}")?,
                    }
                }
                write!(f, "\"")
            }
            Value::Atom(a) => write!(f, "#{a}"),
            Value::Nothing => write!(f, "#absent"),
            Value::HostHandle(handle) => match handle.kind {
                HostHandleKind::Reader => write!(f, "<Reader>"),
                HostHandleKind::Writer => write!(f, "<Writer>"),
            },
            Value::List(items) => {
                write!(f, "[")?;
                for (i, t) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    // By the time Display is called the value should be
                    // force_deep'd; display whatever we have.
                    match t.peek() {
                        Some(v) => write!(f, "{v}")?,
                        None => write!(f, "<thunk>")?,
                    }
                }
                write!(f, "]")
            }
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, field) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if let Some(name) = &field.name {
                        write!(f, "{name} = ")?;
                    }
                    match field.value.peek() {
                        Some(v) => write!(f, "{v}")?,
                        None => write!(f, "<thunk>")?,
                    }
                }
                write!(f, ")")
            }
            Value::Record(fields) => {
                write!(f, "{{")?;
                // Display fields in canonical name-sorted order so interpreter
                // output matches the compiled backend, which lays records out in
                // sorted slot order. The stored Value keeps source order, which
                // diagnostics and `basic.rs` rely on.
                let mut ordered: Vec<_> = fields.iter().collect();
                ordered.sort_by(|a, b| a.0.cmp(&b.0));
                for (i, (name, t)) in ordered.into_iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, " {name} = ")?;
                    match t.peek() {
                        Some(v) => write!(f, "{v}")?,
                        None => write!(f, "<thunk>")?,
                    }
                }
                write!(f, " }}")
            }
            Value::TaggedValue { tag, payload } => {
                write!(f, "#{tag}")?;
                if !payload.is_empty() {
                    let positional = payload
                        .iter()
                        .enumerate()
                        .all(|(index, (name, _))| name.parse::<usize>() == Ok(index));
                    if positional {
                        write!(f, " (")?;
                        for (i, (_, t)) in payload.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            match t.peek() {
                                Some(v) => write!(f, "{v}")?,
                                None => write!(f, "<thunk>")?,
                            }
                        }
                        write!(f, ")")?;
                    } else {
                        write!(f, " {{")?;
                        // Record-style tagged payloads use the same canonical
                        // name-sorted order as ordinary records and the native
                        // descriptor layout. Keep tuple payloads positional.
                        let mut ordered: Vec<_> = payload.iter().collect();
                        ordered.sort_by(|a, b| a.0.cmp(&b.0));
                        for (i, (name, t)) in ordered.into_iter().enumerate() {
                            if i > 0 {
                                write!(f, ";")?;
                            }
                            write!(f, " {name} = ")?;
                            match t.peek() {
                                Some(v) => write!(f, "{v}")?,
                                None => write!(f, "<thunk>")?,
                            }
                        }
                        write!(f, " }}")?;
                    }
                }
                Ok(())
            }
            Value::Closure(c) => {
                // The HIR binding name isn't stored here; use the arity.
                write!(f, "<function/{}>", c.arity - c.applied.len())
            }
            Value::TlcClosure(_) => write!(f, "<function/1>"),
            Value::TypeValue(_) => write!(f, "<type>"),
            Value::WitnessDict(_) => write!(f, "<witness>"),
            Value::Builtin(func) => write!(f, "<builtin {}>", func.name()),
            Value::BuiltinPartial { func, args } => {
                write!(f, "<builtin {}/{}>", func.name(), func.arity() - args.len())
            }
        }
    }
}
