//! Bounded breadth-first explicit-state search over one scenario's model.
//!
//! State values are cheap-clone `Value`s held in a flat `Vec<SearchNode>`; the
//! frontier is a FIFO `VecDeque<usize>` of node indices and the visited set is
//! an `FxHashMap<String, usize>` keyed by canonical fingerprint. A `SearchNode`
//! records only the state value, its parent node index, and the rendered
//! incoming action, so predecessor tracking never duplicates whole states.
//!
//! FIFO ordering plus source-list insertion order makes the first reconstructed
//! failure the shortest deterministic witness.

use std::collections::VecDeque;

use rustc_hash::FxHashMap;
use zutai_eval::{TlcSession, Value};

use crate::ModelError;
use crate::decode::{Expectation, Predicate, Scenario};
use crate::fingerprint::{fingerprint, value_kind};

/// A visited state and how it was reached.
struct SearchNode {
    /// The forced, validated state value (cheap `Rc` clone).
    state: Value,
    /// Parent node index, or `None` for a seed initial state.
    parent: Option<usize>,
    /// Canonical rendering of the incoming action, or `None` for a seed.
    action: Option<String>,
    /// Canonical rendering of this state, for the counterexample body.
    rendered: String,
}

/// The outcome of exploring one scenario.
pub(crate) enum SearchResult {
    Safe {
        visited: usize,
    },
    ExpectedViolation {
        property: String,
        visited: usize,
    },
    InvariantFailure {
        property: String,
        counterexample: String,
    },
    MissingViolation {
        property: String,
    },
    ReachabilityUnmet {
        properties: Vec<String>,
    },
    Limit {
        visited: usize,
    },
}

/// Explore one scenario with bounded BFS.
pub(crate) fn run_scenario(
    session: &TlcSession,
    scenario: &Scenario,
    max_states: usize,
) -> Result<SearchResult, ModelError> {
    if scenario.model.initial.is_empty() {
        return Err(ModelError::EmptyInitial(scenario.name.clone()));
    }

    let model = &scenario.model;
    let mut nodes: Vec<SearchNode> = Vec::new();
    let mut index: FxHashMap<String, usize> = FxHashMap::default();
    let mut frontier: VecDeque<usize> = VecDeque::new();

    // Track which reachability obligations have been met, aligned to
    // `model.reachability`. Ignored for `#violates` scenarios.
    let mut reached = vec![false; model.reachability.len()];
    let checking_reachability = matches!(scenario.expect, Expectation::Safe);

    // Seed every initial state, deduplicating identical initials.
    for initial in &model.initial {
        let (state, rendered) = fingerprint(session, initial.clone(), false)?;
        if index.contains_key(&rendered) {
            continue;
        }
        if nodes.len() >= max_states {
            return Ok(SearchResult::Limit {
                visited: nodes.len(),
            });
        }
        let node = nodes.len();
        index.insert(rendered.clone(), node);
        nodes.push(SearchNode {
            state,
            parent: None,
            action: None,
            rendered,
        });
        frontier.push_back(node);
    }

    while let Some(node) = frontier.pop_front() {
        let state = nodes[node].state.clone();

        // Safety predicates first. `#safe` checks every property; `#violates`
        // intentionally checks only the named mutation property and ignores
        // reachability obligations.
        match &scenario.expect {
            Expectation::Safe => {
                for safety in &model.safety {
                    if !holds(session, safety, &state)? {
                        return finish_safety_failure(
                            session,
                            scenario,
                            &nodes,
                            node,
                            &safety.name,
                        );
                    }
                }
            }
            Expectation::Violates { property } => {
                if let Some(safety) = model.safety.iter().find(|s| &s.name == property)
                    && !holds(session, safety, &state)?
                {
                    return finish_safety_failure(session, scenario, &nodes, node, &safety.name);
                }
            }
        }

        // Reachability obligations (only meaningful for `#safe`).
        if checking_reachability {
            for (i, obligation) in model.reachability.iter().enumerate() {
                if !reached[i] && holds(session, obligation, &state)? {
                    reached[i] = true;
                }
            }
        }
        // A reachability-only scenario cannot change its verdict once every
        // obligation has been met. Avoid expanding the remaining frontier;
        // safety-bearing scenarios must still exhaust it to prove invariants.
        if model.safety.is_empty() && reachability_complete(&reached) {
            return Ok(SearchResult::Safe {
                visited: nodes.len(),
            });
        }

        // Expand successors in source-list order.
        let transitions = session.force(session.apply(model.next.clone(), state.clone())?)?;
        let Value::List(items) = transitions else {
            return Err(ModelError::WrongKind {
                context: "next result",
                expected: "List",
                found: value_kind(&transitions),
            });
        };
        for item in items.iter() {
            let transition = session.force_thunk(item)?;
            let Value::Record(fields) = &transition else {
                return Err(ModelError::WrongKind {
                    context: "transition",
                    expected: "Record",
                    found: value_kind(&transition),
                });
            };
            let action_value = field(session, fields, "action")?;
            let next_state_value = field(session, fields, "state")?;
            // Force/validate the action for its rendering (rejects non-first-order).
            let (_, action_rendered) = fingerprint(session, action_value, true)?;
            let (next_state, next_rendered) = fingerprint(session, next_state_value, false)?;

            if index.contains_key(&next_rendered) {
                continue;
            }
            if nodes.len() >= max_states {
                return Ok(SearchResult::Limit {
                    visited: nodes.len(),
                });
            }
            let child = nodes.len();
            index.insert(next_rendered.clone(), child);
            nodes.push(SearchNode {
                state: next_state,
                parent: Some(node),
                action: Some(action_rendered),
                rendered: next_rendered,
            });
            frontier.push_back(child);
        }
    }

    // Frontier drained without a safety failure.
    match &scenario.expect {
        Expectation::Violates { property } => Ok(SearchResult::MissingViolation {
            property: property.clone(),
        }),
        Expectation::Safe => {
            let properties = unmet_reachability(&model.reachability, &reached);
            if properties.is_empty() {
                Ok(SearchResult::Safe {
                    visited: nodes.len(),
                })
            } else {
                Ok(SearchResult::ReachabilityUnmet { properties })
            }
        }
    }
}

fn reachability_complete(reached: &[bool]) -> bool {
    reached.iter().all(|met| *met)
}

fn unmet_reachability(obligations: &[Predicate], reached: &[bool]) -> Vec<String> {
    obligations
        .iter()
        .zip(reached)
        .filter(|(_, met)| !**met)
        .map(|(obligation, _)| obligation.name.clone())
        .collect()
}

/// Resolve a safety failure into the scenario verdict. For a `#violates`
/// scenario naming this property, it is the expected pass; for any other
/// property, or a `#safe` scenario, it is an invariant failure with a witness.
fn finish_safety_failure(
    session: &TlcSession,
    scenario: &Scenario,
    nodes: &[SearchNode],
    node: usize,
    property: &str,
) -> Result<SearchResult, ModelError> {
    if let Expectation::Violates { property: expected } = &scenario.expect
        && expected == property
    {
        return Ok(SearchResult::ExpectedViolation {
            property: property.to_owned(),
            visited: nodes.len(),
        });
    }
    let _ = session;
    Ok(SearchResult::InvariantFailure {
        property: property.to_owned(),
        counterexample: render_counterexample(nodes, node),
    })
}

/// Reconstruct the shortest action/state path from a seed initial state to the
/// failing node through parent indices.
fn render_counterexample(nodes: &[SearchNode], failing: usize) -> String {
    let mut path: Vec<usize> = Vec::new();
    let mut current = Some(failing);
    while let Some(node) = current {
        path.push(node);
        current = nodes[node].parent;
    }
    path.reverse();

    let mut out = String::from("counterexample:\n");
    for (position, &node) in path.iter().enumerate() {
        let entry = &nodes[node];
        if position == 0 {
            out.push_str(&format!("  initial: {}\n", entry.rendered));
        } else {
            let action = entry.action.as_deref().unwrap_or("<unknown>");
            out.push_str(&format!("  {action} -> {}\n", entry.rendered));
        }
    }
    out
}

/// Apply a predicate to a state and force the `Bool` result.
fn holds(session: &TlcSession, predicate: &Predicate, state: &Value) -> Result<bool, ModelError> {
    let result = session.force(session.apply(predicate.callable.clone(), state.clone())?)?;
    match result {
        Value::Bool(b) => Ok(b),
        other => Err(ModelError::WrongKind {
            context: "predicate result",
            expected: "Bool",
            found: value_kind(&other),
        }),
    }
}

fn field(
    session: &TlcSession,
    fields: &[(std::rc::Rc<str>, zutai_eval::Thunk)],
    name: &str,
) -> Result<Value, ModelError> {
    let (_, value) = fields
        .iter()
        .find(|(field, _)| field.as_ref() == name)
        .ok_or_else(|| ModelError::MissingField(name.to_owned()))?;
    Ok(session.force_thunk(value)?)
}
