//! Bounded explicit-state model checker for pure typed Zutai (`.zt`).
//!
//! `zutai model-check PATH` evaluates `PATH` through [`zutai_eval::TlcSession`]
//! and decodes the final value record's `scenarios` field into transition-system
//! models. Each model is explored with bounded breadth-first search over
//! first-order runtime states; safety predicates, reachability obligations, and
//! `#violates` expectations produce a per-scenario verdict.
//!
//! This crate adds no syntax, HIR, THIR, TLC IR, SMT, refinement-type, or
//! proof-term capability. It is a host-side reference tool over ordinary pure
//! `.zt` values: a wrong verdict is worse than a refused check, so any state or
//! action carrying a non-first-order runtime value is rejected outright.

mod decode;
mod fingerprint;
mod search;

#[cfg(test)]
mod tests;

use zutai_eval::TlcSession;

/// Default per-scenario distinct-state budget. Far above the demonstrated
/// BootState workload (1,424 states); hitting it is inconclusive, never safe.
pub const DEFAULT_MAX_STATES: usize = 1_000_000;

/// Options controlling a model-check run.
#[derive(Clone, Copy, Debug)]
pub struct CheckOptions {
    /// Maximum distinct states to visit per scenario.
    pub max_states: usize,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            max_states: DEFAULT_MAX_STATES,
        }
    }
}

/// How a scenario passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PassedKind {
    /// `#safe`: frontier emptied with every safety predicate holding and every
    /// reachability obligation met.
    Safe,
    /// `#violates { property }`: the named safety predicate became false at a
    /// reachable state.
    ExpectedViolation { property: String },
}

/// A completed passing scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioReport {
    pub name: String,
    pub visited: usize,
    pub kind: PassedKind,
}

/// The overall verdict for a model-check run.
///
/// `check_analysis` writes nothing; the caller renders reports and owns exit
/// codes. `completed` retains every scenario that passed before a later
/// scenario failed or hit the state limit.
#[derive(Clone, Debug)]
pub enum CheckOutcome {
    /// Every scenario met its expectation.
    Passed { scenarios: Vec<ScenarioReport> },
    /// A scenario failed its expectation. `message` is the fully rendered
    /// failure body (fixed CLI wording, including any counterexample).
    Failed {
        completed: Vec<ScenarioReport>,
        message: String,
    },
    /// A scenario discovered a new unseen state at the state limit.
    Inconclusive {
        completed: Vec<ScenarioReport>,
        scenario: String,
        visited: usize,
    },
}

/// Errors that abort a model-check run before or during search.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// The analysis has diagnostics or incomplete typed IR; a model must be a
    /// fully type-checked runnable program.
    #[error("model program is incomplete or not runnable")]
    IncompleteAnalysis,
    /// A wrapped evaluator error from forcing or applying model functions.
    #[error(transparent)]
    Eval(#[from] zutai_eval::EvalError),
    /// A required interface field is absent.
    #[error("model is missing required field `{0}`")]
    MissingField(String),
    /// A runtime value had the wrong kind for its interface position.
    #[error("expected {expected} for {context}, found {found}")]
    WrongKind {
        context: &'static str,
        expected: &'static str,
        found: &'static str,
    },
    /// An `expect` field used an unknown tag.
    #[error("unknown expectation tag `#{0}`")]
    UnknownExpectation(String),
    /// A field that must be callable (`next`, `holds`, `reached`) was not.
    #[error("`{0}` must be callable")]
    NonCallable(String),
    /// Two scenarios share a name.
    #[error("duplicate scenario name \"{0}\"")]
    DuplicateScenario(String),
    /// Two safety properties within one model share a name.
    #[error("duplicate safety property name \"{0}\"")]
    DuplicateSafety(String),
    /// Two reachability obligations within one model share a name.
    #[error("duplicate reachability name \"{0}\"")]
    DuplicateReachability(String),
    /// The model exported no scenarios.
    #[error("`scenarios` must be non-empty")]
    EmptyScenarios,
    /// A scenario's `initial` list was empty.
    #[error("scenario \"{0}\" has an empty initial state list")]
    EmptyInitial(String),
    /// `#violates` named a safety property that does not exist in the model.
    #[error("expectation names unknown safety property \"{0}\"")]
    UnknownViolatesProperty(String),
    /// The state budget was zero.
    #[error("max_states must be at least 1")]
    ZeroStateLimit,
    /// A state carried a value outside the first-order grammar.
    #[error("state contains non-first-order value: {0}")]
    NonFirstOrderState(&'static str),
    /// An action carried a value outside the first-order grammar.
    #[error("action contains non-first-order value: {0}")]
    NonFirstOrderAction(&'static str),
}

/// Model-check a completed semantic analysis.
///
/// Evaluates the program's final expression, decodes `scenarios`, validates the
/// interface, and runs bounded BFS per scenario in source order.
pub fn check_analysis(
    analysis: &zutai_semantic::Analysis,
    options: CheckOptions,
) -> Result<CheckOutcome, ModelError> {
    if options.max_states < 1 {
        return Err(ModelError::ZeroStateLimit);
    }
    if !analysis.is_thir_complete() {
        return Err(ModelError::IncompleteAnalysis);
    }
    let session =
        TlcSession::from_analysis(analysis).map_err(|_| ModelError::IncompleteAnalysis)?;
    let entry = session.entry()?;
    let scenarios = decode::decode_scenarios(&session, entry)?;

    let mut completed: Vec<ScenarioReport> = Vec::new();
    for scenario in &scenarios {
        match search::run_scenario(&session, scenario, options.max_states)? {
            search::SearchResult::Safe { visited } => completed.push(ScenarioReport {
                name: scenario.name.clone(),
                visited,
                kind: PassedKind::Safe,
            }),
            search::SearchResult::ExpectedViolation { property, visited } => {
                completed.push(ScenarioReport {
                    name: scenario.name.clone(),
                    visited,
                    kind: PassedKind::ExpectedViolation { property },
                })
            }
            search::SearchResult::InvariantFailure {
                property,
                counterexample,
            } => {
                let message = format!(
                    "scenario \"{}\": FAILED invariant \"{property}\"\n{counterexample}expected: safe",
                    scenario.name
                );
                return Ok(CheckOutcome::Failed { completed, message });
            }
            search::SearchResult::MissingViolation { property } => {
                let message = format!(
                    "scenario \"{}\": FAILED (expected violation of \"{property}\", none found)",
                    scenario.name
                );
                return Ok(CheckOutcome::Failed { completed, message });
            }
            search::SearchResult::ReachabilityUnmet { properties } => {
                let message = render_reachability_failure(&scenario.name, &properties);
                return Ok(CheckOutcome::Failed { completed, message });
            }
            search::SearchResult::Limit { visited } => {
                return Ok(CheckOutcome::Inconclusive {
                    completed,
                    scenario: scenario.name.clone(),
                    visited,
                });
            }
        }
    }

    Ok(CheckOutcome::Passed {
        scenarios: completed,
    })
}

fn render_reachability_failure(scenario: &str, properties: &[String]) -> String {
    match properties {
        [property] => {
            format!("scenario \"{scenario}\": FAILED reachability \"{property}\" never reached")
        }
        [] => unreachable!("reachability failure requires an unmet obligation"),
        properties => {
            let obligations = properties
                .iter()
                .map(|property| format!("  - \"{property}\""))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "scenario \"{scenario}\": FAILED reachability obligations never reached:\n{obligations}"
            )
        }
    }
}
