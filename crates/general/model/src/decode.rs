//! Decode the final value record's `scenarios` field into checkable models.
//!
//! Reuses the established browser-kernel decoder pattern (`force_field`,
//! `decode_list`, `expect_record`, `expect_tagged`, `is_callable`). Ordinary
//! fields are forced through the session; `next`, state predicates,
//! `actionSafety` callables, and `leadsTo` premise/consequence callables are
//! retained as `Value`s. Extra final/scenario/model fields remain ignored for
//! forward-compatible host tooling.

use std::rc::Rc;

use zutai_eval::{Thunk, TlcSession, Value};

use crate::ModelError;

type ValueFields<'a> = &'a [(Rc<str>, Thunk)];

/// A model-defined unary state predicate: a name plus a retained callable.
pub(crate) struct Predicate {
    pub name: String,
    pub callable: Value,
}

/// A named invariant over one generated `(before, action, after)` transition.
pub(crate) struct ActionPredicate {
    pub name: String,
    pub callable: Value,
}

/// A named universal temporal obligation over state predicates.
pub(crate) struct LeadsTo {
    pub name: String,
    pub premise: Value,
    pub consequence: Value,
}

/// A decoded transition-system model.
pub(crate) struct Model {
    pub initial: Vec<Value>,
    pub next: Value,
    pub safety: Vec<Predicate>,
    pub reachability: Vec<Predicate>,
    pub terminal: Option<Value>,
    pub action_safety: Vec<ActionPredicate>,
    pub leads_to: Vec<LeadsTo>,
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
    let terminal = force_optional_field(session, fields, "terminal")?;
    if terminal.as_ref().is_some_and(|value| !is_callable(value)) {
        return Err(ModelError::NonCallable("terminal".to_owned()));
    }
    let action_safety = match force_optional_field(session, fields, "actionSafety")? {
        Some(value) => decode_list(session, value, |value| {
            decode_action_predicate(session, value)
        })?,
        None => Vec::new(),
    };
    let leads_to = match force_optional_field(session, fields, "leadsTo")? {
        Some(value) => decode_list(session, value, |value| decode_leads_to(session, value))?,
        None => Vec::new(),
    };

    ensure_unique(
        safety.iter().map(|predicate| predicate.name.as_str()),
        ModelError::DuplicateSafety,
    )?;
    ensure_unique(
        reachability.iter().map(|predicate| predicate.name.as_str()),
        ModelError::DuplicateReachability,
    )?;
    ensure_unique(
        action_safety
            .iter()
            .map(|predicate| predicate.name.as_str()),
        ModelError::DuplicateActionSafety,
    )?;
    ensure_unique(
        leads_to.iter().map(|property| property.name.as_str()),
        ModelError::DuplicateLeadsTo,
    )?;
    if !leads_to.is_empty() && terminal.is_none() {
        return Err(ModelError::LeadsToRequiresTerminal);
    }

    Ok(Model {
        initial,
        next,
        safety,
        reachability,
        terminal,
        action_safety,
        leads_to,
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

fn decode_action_predicate(
    session: &TlcSession,
    value: Value,
) -> Result<ActionPredicate, ModelError> {
    let value = session.force(value)?;
    let fields = expect_record(&value, "action safety predicate")?;
    let name = text(
        session,
        force_field(session, fields, "name")?,
        "action safety name",
    )?;
    let callable = force_field(session, fields, "holds")?;
    if !is_callable(&callable) {
        return Err(ModelError::NonCallable("holds".to_owned()));
    }
    Ok(ActionPredicate { name, callable })
}

fn decode_leads_to(session: &TlcSession, value: Value) -> Result<LeadsTo, ModelError> {
    let value = session.force(value)?;
    let fields = expect_record(&value, "leadsTo property")?;
    let name = text(
        session,
        force_field(session, fields, "name")?,
        "leadsTo name",
    )?;
    let premise = force_field(session, fields, "premise")?;
    if !is_callable(&premise) {
        return Err(ModelError::NonCallable("premise".to_owned()));
    }
    let consequence = force_field(session, fields, "consequence")?;
    if !is_callable(&consequence) {
        return Err(ModelError::NonCallable("consequence".to_owned()));
    }
    Ok(LeadsTo {
        name,
        premise,
        consequence,
    })
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

fn ensure_unique<'a>(
    names: impl IntoIterator<Item = &'a str>,
    make_error: impl Fn(String) -> ModelError,
) -> Result<(), ModelError> {
    let mut seen: Vec<&str> = Vec::new();
    for name in names {
        if seen.contains(&name) {
            return Err(make_error(name.to_owned()));
        }
        seen.push(name);
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

fn force_optional_field(
    session: &TlcSession,
    fields: ValueFields<'_>,
    name: &str,
) -> Result<Option<Value>, ModelError> {
    fields
        .iter()
        .find(|(field, _)| field.as_ref() == name)
        .map(|(_, value)| session.force_thunk(value).map_err(ModelError::from))
        .transpose()
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
