//! Decode the final value record's `scenarios` field into checkable models.
//!
//! Reuses the established browser-kernel decoder pattern (`force_field`,
//! `decode_list`, `expect_record`, `expect_tagged`, `is_callable`). Ordinary
//! fields are forced through the session; `next`, `holds`, and `reached` are
//! retained as callable `Value`s. Extra fields on the final record and on
//! individual scenario records are ignored so a model module may export query
//! helpers beside `scenarios`.

use std::rc::Rc;

use zutai_eval::{Thunk, TlcSession, Value};

use crate::ModelError;

type ValueFields<'a> = &'a [(Rc<str>, Thunk)];

/// A model-defined safety predicate or reachability obligation: a name plus the
/// retained callable that maps a state to `Bool`.
pub(crate) struct Predicate {
    pub name: String,
    pub callable: Value,
}

/// A decoded transition-system model.
pub(crate) struct Model {
    pub initial: Vec<Value>,
    pub next: Value,
    pub safety: Vec<Predicate>,
    pub reachability: Vec<Predicate>,
}

/// What a scenario expects the checker to conclude.
pub(crate) enum Expectation {
    Safe,
    Violates { property: String },
}

/// One named scenario: a model and its expected verdict.
pub(crate) struct Scenario {
    pub name: String,
    pub model: Model,
    pub expect: Expectation,
}

/// Decode and validate the final record's `scenarios` field.
pub(crate) fn decode_scenarios(
    session: &TlcSession,
    entry: Value,
) -> Result<Vec<Scenario>, ModelError> {
    let entry = session.force(entry)?;
    let fields = expect_record(&entry, "model")?;
    let scenarios_value = force_field(session, fields, "scenarios")?;
    let scenarios = decode_list(session, scenarios_value, |value| {
        decode_scenario(session, value)
    })?;

    if scenarios.is_empty() {
        return Err(ModelError::EmptyScenarios);
    }
    let mut seen: Vec<&str> = Vec::with_capacity(scenarios.len());
    for scenario in &scenarios {
        if seen.contains(&scenario.name.as_str()) {
            return Err(ModelError::DuplicateScenario(scenario.name.clone()));
        }
        seen.push(&scenario.name);
    }
    Ok(scenarios)
}

fn decode_scenario(session: &TlcSession, value: Value) -> Result<Scenario, ModelError> {
    let value = session.force(value)?;
    let fields = expect_record(&value, "scenario")?;
    let name = text(
        session,
        force_field(session, fields, "name")?,
        "scenario name",
    )?;
    let model = decode_model(session, force_field(session, fields, "model")?)?;
    let expect = decode_expectation(session, force_field(session, fields, "expect")?)?;

    if let Expectation::Violates { property } = &expect
        && !model.safety.iter().any(|s| &s.name == property)
    {
        return Err(ModelError::UnknownViolatesProperty(property.clone()));
    }

    Ok(Scenario {
        name,
        model,
        expect,
    })
}

fn decode_model(session: &TlcSession, value: Value) -> Result<Model, ModelError> {
    let value = session.force(value)?;
    let fields = expect_record(&value, "model")?;

    let initial = decode_list(session, force_field(session, fields, "initial")?, Ok)?;

    let next = force_field(session, fields, "next")?;
    if !is_callable(&next) {
        return Err(ModelError::NonCallable("next".to_owned()));
    }

    let safety = decode_list(session, force_field(session, fields, "safety")?, |value| {
        decode_predicate(session, value, "holds")
    })?;
    let reachability = decode_list(
        session,
        force_field(session, fields, "reachability")?,
        |value| decode_predicate(session, value, "reached"),
    )?;

    ensure_unique(&safety, ModelError::DuplicateSafety)?;
    ensure_unique(&reachability, ModelError::DuplicateReachability)?;

    Ok(Model {
        initial,
        next,
        safety,
        reachability,
    })
}

fn decode_predicate(
    session: &TlcSession,
    value: Value,
    callable_field: &'static str,
) -> Result<Predicate, ModelError> {
    let value = session.force(value)?;
    let fields = expect_record(&value, "predicate")?;
    let name = text(
        session,
        force_field(session, fields, "name")?,
        "predicate name",
    )?;
    let callable = force_field(session, fields, callable_field)?;
    if !is_callable(&callable) {
        return Err(ModelError::NonCallable(callable_field.to_owned()));
    }
    Ok(Predicate { name, callable })
}

fn decode_expectation(session: &TlcSession, value: Value) -> Result<Expectation, ModelError> {
    let value = session.force(value)?;
    let (tag, payload) = expect_tagged(&value)?;
    match tag {
        "safe" => Ok(Expectation::Safe),
        "violates" => {
            let property = text(
                session,
                force_field(session, payload, "property")?,
                "violates property",
            )?;
            Ok(Expectation::Violates { property })
        }
        other => Err(ModelError::UnknownExpectation(other.to_owned())),
    }
}

fn ensure_unique(
    predicates: &[Predicate],
    make_error: impl Fn(String) -> ModelError,
) -> Result<(), ModelError> {
    let mut seen: Vec<&str> = Vec::with_capacity(predicates.len());
    for predicate in predicates {
        if seen.contains(&predicate.name.as_str()) {
            return Err(make_error(predicate.name.clone()));
        }
        seen.push(&predicate.name);
    }
    Ok(())
}

// ─── shared decoder helpers (reused from the browser-kernel pattern) ───────────

fn decode_list<T>(
    session: &TlcSession,
    value: Value,
    mut decode: impl FnMut(Value) -> Result<T, ModelError>,
) -> Result<Vec<T>, ModelError> {
    let value = session.force(value)?;
    let Value::List(items) = value else {
        return Err(wrong_kind("list", "List", &value));
    };
    items
        .iter()
        .map(|item| session.force_thunk(item).map_err(ModelError::from))
        .map(|value| value.and_then(&mut decode))
        .collect()
}

fn force_field(
    session: &TlcSession,
    fields: ValueFields<'_>,
    name: &str,
) -> Result<Value, ModelError> {
    let (_, value) = fields
        .iter()
        .find(|(field, _)| field.as_ref() == name)
        .ok_or_else(|| ModelError::MissingField(name.to_owned()))?;
    Ok(session.force_thunk(value)?)
}

fn text(session: &TlcSession, value: Value, context: &'static str) -> Result<String, ModelError> {
    let value = session.force(value)?;
    match value {
        Value::Text(value) => Ok(value.to_string()),
        other => Err(wrong_kind(context, "Text", &other)),
    }
}

fn expect_record<'a>(
    value: &'a Value,
    context: &'static str,
) -> Result<ValueFields<'a>, ModelError> {
    match value {
        Value::Record(fields) => Ok(fields.as_slice()),
        other => Err(wrong_kind(context, "Record", other)),
    }
}

fn expect_tagged(value: &Value) -> Result<(&str, ValueFields<'_>), ModelError> {
    match value {
        Value::Atom(tag) => Ok((tag.as_ref(), &[])),
        Value::TaggedValue { tag, payload } => Ok((tag.as_ref(), payload.as_slice())),
        other => Err(wrong_kind("expectation", "tagged value", other)),
    }
}

fn is_callable(value: &Value) -> bool {
    matches!(
        value,
        Value::Closure(_) | Value::TlcClosure(_) | Value::Builtin(_) | Value::BuiltinPartial { .. }
    )
}

fn wrong_kind(context: &'static str, expected: &'static str, value: &Value) -> ModelError {
    ModelError::WrongKind {
        context,
        expected,
        found: crate::fingerprint::value_kind(value),
    }
}
