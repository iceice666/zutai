//! Bounded breadth-first explicit-state search over one scenario's model.
//!
//! States remain in deterministic BFS order and use their full canonical
//! rendering as collision-safe identity. The search also retains every ordered
//! generated edge so action invariants, SCC statistics, and temporal witnesses
//! cannot be hidden by duplicate-state elimination.

use std::collections::VecDeque;

use rustc_hash::FxHashMap;
use zutai_eval::{TlcSession, Value};

use crate::decode::{ActionPredicate, Expectation, LeadsTo, Predicate, Scenario};
use crate::fingerprint::{fingerprint, value_kind};
use crate::{GraphStats, ModelError};

/// A visited state and how its shortest deterministic BFS prefix reached it.
struct SearchNode {
    state: Value,
    parent: Option<usize>,
    action: Option<String>,
    rendered: String,
    depth: usize,
    terminal: bool,
}

/// One generated graph edge, retained even when its target was already known.
struct SearchEdge {
    to: usize,
    action: String,
}

/// The outcome of exploring one scenario.
pub(crate) enum SearchResult {
    Safe {
        stats: GraphStats,
    },
    ExpectedViolation {
        property: String,
        stats: GraphStats,
    },
    InvariantFailure {
        property: String,
        counterexample: String,
        stats: GraphStats,
    },
    Deadlock {
        counterexample: String,
        stats: GraphStats,
    },
    TerminalSuccessors {
        counterexample: String,
        stats: GraphStats,
    },
    ActionInvariantFailure {
        property: String,
        counterexample: String,
        stats: GraphStats,
    },
    LeadsToFailure {
        property: String,
        counterexample: String,
        stats: GraphStats,
    },
    MissingViolation {
        property: String,
        stats: GraphStats,
    },
    ReachabilityUnmet {
        properties: Vec<String>,
        stats: GraphStats,
    },
    Limit {
        stats: GraphStats,
    },
}

/// Explore one scenario with bounded BFS, then analyze the complete graph.
pub(crate) fn run_scenario(
    session: &TlcSession,
    scenario: &Scenario,
    max_states: usize,
) -> Result<SearchResult, ModelError> {
    if scenario.model.initial.is_empty() {
        return Err(ModelError::EmptyInitial(scenario.name.clone()));
    }

    let model = &scenario.model;
    let graph_required =
        model.terminal.is_some() || !model.action_safety.is_empty() || !model.leads_to.is_empty();
    let mut nodes: Vec<SearchNode> = Vec::new();
    let mut edges: Vec<Vec<SearchEdge>> = Vec::new();
    let mut index: FxHashMap<String, usize> = FxHashMap::default();
    let mut frontier: VecDeque<usize> = VecDeque::new();
    let mut stats = GraphStats::default();

    let mut reached = vec![false; model.reachability.len()];
    let checking_reachability = matches!(scenario.expect, Expectation::Safe);

    // Initial duplicates are state duplicates, but not generated transitions.
    for initial in &model.initial {
        let (state, rendered) = fingerprint(session, initial.clone(), false)?;
        if index.contains_key(&rendered) {
            stats.duplicates += 1;
            continue;
        }
        if nodes.len() >= max_states {
            finalize_graph_stats(&mut stats, &edges);
            return Ok(SearchResult::Limit { stats });
        }
        let node = nodes.len();
        index.insert(rendered.clone(), node);
        nodes.push(SearchNode {
            state,
            parent: None,
            action: None,
            rendered,
            depth: 0,
            terminal: false,
        });
        edges.push(Vec::new());
        frontier.push_back(node);
    }
    stats.distinct_states = nodes.len();

    while let Some(node) = frontier.pop_front() {
        let state = nodes[node].state.clone();

        match &scenario.expect {
            Expectation::Safe => {
                for safety in &model.safety {
                    if !holds(session, safety, &state, "safety predicate result")? {
                        finalize_graph_stats(&mut stats, &edges);
                        return Ok(SearchResult::InvariantFailure {
                            property: safety.name.clone(),
                            counterexample: render_counterexample(&nodes, node),
                            stats,
                        });
                    }
                }
            }
            Expectation::Violates { property } => {
                if let Some(safety) = model.safety.iter().find(|s| &s.name == property)
                    && !holds(session, safety, &state, "safety predicate result")?
                {
                    finalize_graph_stats(&mut stats, &edges);
                    return Ok(SearchResult::ExpectedViolation {
                        property: safety.name.clone(),
                        stats,
                    });
                }
            }
        }

        if checking_reachability {
            for (i, obligation) in model.reachability.iter().enumerate() {
                if !reached[i]
                    && holds(session, obligation, &state, "reachability predicate result")?
                {
                    reached[i] = true;
                }
            }
        }

        if !graph_required && model.safety.is_empty() && reachability_complete(&reached) {
            finalize_graph_stats(&mut stats, &edges);
            return Ok(SearchResult::Safe { stats });
        }

        let is_terminal = match &model.terminal {
            Some(terminal) => holds_callable(session, terminal, &state, "terminal result")?,
            None => false,
        };
        nodes[node].terminal = is_terminal;
        if is_terminal {
            stats.terminal_states += 1;
        }

        let transitions = session.force(session.apply(model.next.clone(), state.clone())?)?;
        let Value::List(items) = transitions else {
            return Err(ModelError::WrongKind {
                context: "next result",
                expected: "List",
                found: value_kind(&transitions),
            });
        };

        if is_terminal && !items.is_empty() {
            stats.generated_transitions += items.len();
            finalize_graph_stats(&mut stats, &edges);
            return Ok(SearchResult::TerminalSuccessors {
                counterexample: render_counterexample(&nodes, node),
                stats,
            });
        }
        if model.terminal.is_some() && !is_terminal && items.is_empty() {
            stats.unexpected_deadlocks += 1;
            finalize_graph_stats(&mut stats, &edges);
            return Ok(SearchResult::Deadlock {
                counterexample: render_counterexample(&nodes, node),
                stats,
            });
        }

        for item in items.iter() {
            stats.generated_transitions += 1;
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
            let (action, action_rendered) = fingerprint(session, action_value, true)?;
            let (next_state, next_rendered) = fingerprint(session, next_state_value, false)?;

            for predicate in &model.action_safety {
                if !action_holds(session, predicate, &state, &action, &next_state)? {
                    finalize_graph_stats(&mut stats, &edges);
                    return Ok(SearchResult::ActionInvariantFailure {
                        property: predicate.name.clone(),
                        counterexample: render_transition_counterexample(
                            &nodes,
                            node,
                            &action_rendered,
                            &next_rendered,
                        ),
                        stats,
                    });
                }
            }

            let child = if let Some(&child) = index.get(&next_rendered) {
                stats.duplicates += 1;
                child
            } else {
                if nodes.len() >= max_states {
                    finalize_graph_stats(&mut stats, &edges);
                    return Ok(SearchResult::Limit { stats });
                }
                let child = nodes.len();
                let depth = nodes[node].depth + 1;
                stats.max_depth = stats.max_depth.max(depth);
                index.insert(next_rendered.clone(), child);
                nodes.push(SearchNode {
                    state: next_state,
                    parent: Some(node),
                    action: Some(action_rendered.clone()),
                    rendered: next_rendered,
                    depth,
                    terminal: false,
                });
                edges.push(Vec::new());
                frontier.push_back(child);
                child
            };
            edges[node].push(SearchEdge {
                to: child,
                action: action_rendered,
            });
        }
    }

    let components = strongly_connected_components(&edges);
    stats.distinct_states = edges.len();
    apply_component_stats(&mut stats, &components);

    match &scenario.expect {
        Expectation::Violates { property } => Ok(SearchResult::MissingViolation {
            property: property.clone(),
            stats,
        }),
        Expectation::Safe => {
            let properties = unmet_reachability(&model.reachability, &reached);
            if !properties.is_empty() {
                return Ok(SearchResult::ReachabilityUnmet { properties, stats });
            }
            for property in &model.leads_to {
                if let Some(counterexample) = check_leads_to(session, property, &nodes, &edges)? {
                    return Ok(SearchResult::LeadsToFailure {
                        property: property.name.clone(),
                        counterexample,
                        stats,
                    });
                }
            }
            Ok(SearchResult::Safe { stats })
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

fn finalize_graph_stats(stats: &mut GraphStats, edges: &[Vec<SearchEdge>]) {
    stats.distinct_states = edges.len();
    let components = strongly_connected_components(edges);
    apply_component_stats(stats, &components);
}

fn apply_component_stats(stats: &mut GraphStats, components: &[Vec<usize>]) {
    stats.scc_count = components.len();
    stats.largest_scc = components.iter().map(Vec::len).max().unwrap_or(0);
}

fn holds(
    session: &TlcSession,
    predicate: &Predicate,
    state: &Value,
    context: &'static str,
) -> Result<bool, ModelError> {
    holds_callable(session, &predicate.callable, state, context)
}

fn holds_callable(
    session: &TlcSession,
    callable: &Value,
    state: &Value,
    context: &'static str,
) -> Result<bool, ModelError> {
    let result = session.force(session.apply(callable.clone(), state.clone())?)?;
    bool_result(result, context)
}

fn action_holds(
    session: &TlcSession,
    predicate: &ActionPredicate,
    before: &Value,
    action: &Value,
    after: &Value,
) -> Result<bool, ModelError> {
    let after_before = session.apply(predicate.callable.clone(), before.clone())?;
    let after_action = session.apply(after_before, action.clone())?;
    let result = session.force(session.apply(after_action, after.clone())?)?;
    bool_result(result, "action safety predicate result")
}

fn bool_result(result: Value, context: &'static str) -> Result<bool, ModelError> {
    match result {
        Value::Bool(value) => Ok(value),
        other => Err(ModelError::WrongKind {
            context,
            expected: "Bool",
            found: value_kind(&other),
        }),
    }
}

trait EdgeTarget {
    fn target(&self) -> usize;
}

impl EdgeTarget for SearchEdge {
    fn target(&self) -> usize {
        self.to
    }
}

impl EdgeTarget for usize {
    fn target(&self) -> usize {
        *self
    }
}

/// Iterative Kosaraju SCC decomposition, with members ordered by BFS index.
fn strongly_connected_components<E: EdgeTarget>(edges: &[Vec<E>]) -> Vec<Vec<usize>> {
    let mut seen = vec![false; edges.len()];
    let mut finish = Vec::with_capacity(edges.len());
    for start in 0..edges.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![(start, 0usize)];
        while let Some((node, next_edge)) = stack.pop() {
            if next_edge < edges[node].len() {
                stack.push((node, next_edge + 1));
                let child = edges[node][next_edge].target();
                if !seen[child] {
                    seen[child] = true;
                    stack.push((child, 0));
                }
            } else {
                finish.push(node);
            }
        }
    }

    let mut reverse = vec![Vec::new(); edges.len()];
    for (from, outgoing) in edges.iter().enumerate() {
        for edge in outgoing {
            reverse[edge.target()].push(from);
        }
    }

    let mut assigned = vec![false; edges.len()];
    let mut components = Vec::new();
    for &start in finish.iter().rev() {
        if assigned[start] {
            continue;
        }
        assigned[start] = true;
        let mut stack = vec![start];
        let mut component = Vec::new();
        while let Some(node) = stack.pop() {
            component.push(node);
            for &parent in reverse[node].iter().rev() {
                if !assigned[parent] {
                    assigned[parent] = true;
                    stack.push(parent);
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }
    components.sort_by_key(|component| component[0]);
    components
}

fn check_leads_to(
    session: &TlcSession,
    property: &LeadsTo,
    nodes: &[SearchNode],
    edges: &[Vec<SearchEdge>],
) -> Result<Option<String>, ModelError> {
    let mut premise = Vec::with_capacity(nodes.len());
    let mut consequence = Vec::with_capacity(nodes.len());
    for node in nodes {
        premise.push(holds_callable(
            session,
            &property.premise,
            &node.state,
            "leadsTo premise result",
        )?);
        consequence.push(holds_callable(
            session,
            &property.consequence,
            &node.state,
            "leadsTo consequence result",
        )?);
    }

    // Compute cyclicity in the property-specific subgraph where consequence is
    // false. A whole-graph SCC may contain a consequence state and a smaller
    // cycle that can avoid that state forever.
    let restricted = consequence_free_edges(edges, &consequence);
    let restricted_components = strongly_connected_components(&restricted);
    let mut bad_cycle = vec![false; nodes.len()];
    for component in &restricted_components {
        let cyclic = component.len() > 1
            || restricted[component[0]]
                .iter()
                .any(|&target| target == component[0]);
        if cyclic {
            for &node in component {
                bad_cycle[node] = true;
            }
        }
    }
    let mut bad_target = bad_cycle.clone();
    for (node, state) in nodes.iter().enumerate() {
        if state.terminal && !consequence[node] {
            bad_target[node] = true;
        }
    }
    let can_reach_bad = reverse_reachable(&restricted, &bad_target);

    // Source order chooses the first violating premise. Safe premise states are
    // rejected by the reverse-reachability summary without a fresh graph scan.
    for start in 0..nodes.len() {
        if !premise[start] || consequence[start] || !can_reach_bad[start] {
            continue;
        }
        let mut parent: Vec<Option<(usize, usize)>> = vec![None; nodes.len()];
        let mut seen = vec![false; nodes.len()];
        let mut queue = VecDeque::from([start]);
        seen[start] = true;

        while let Some(node) = queue.pop_front() {
            if nodes[node].terminal {
                let path = reconstruct_local_path(start, node, &parent);
                return Ok(Some(render_temporal_finite(nodes, edges, start, &path)));
            }
            if bad_cycle[node]
                && let Some(cycle) = find_cycle(node, edges, &consequence)
            {
                let path = reconstruct_local_path(start, node, &parent);
                return Ok(Some(render_lasso(nodes, edges, start, &path, &cycle)));
            }
            for (edge_index, edge) in edges[node].iter().enumerate() {
                if consequence[edge.to] || seen[edge.to] || !can_reach_bad[edge.to] {
                    continue;
                }
                seen[edge.to] = true;
                parent[edge.to] = Some((node, edge_index));
                queue.push_back(edge.to);
            }
        }
    }
    Ok(None)
}

fn reverse_reachable(edges: &[Vec<usize>], targets: &[bool]) -> Vec<bool> {
    let mut reverse = vec![Vec::new(); edges.len()];
    for (from, outgoing) in edges.iter().enumerate() {
        for &target in outgoing {
            reverse[target].push(from);
        }
    }
    let mut reached = targets.to_vec();
    let mut queue: VecDeque<usize> = targets
        .iter()
        .enumerate()
        .filter_map(|(node, target)| target.then_some(node))
        .collect();
    while let Some(node) = queue.pop_front() {
        for &parent in &reverse[node] {
            if !reached[parent] {
                reached[parent] = true;
                queue.push_back(parent);
            }
        }
    }
    reached
}

fn consequence_free_edges(edges: &[Vec<SearchEdge>], consequence: &[bool]) -> Vec<Vec<usize>> {
    edges
        .iter()
        .enumerate()
        .map(|(from, outgoing)| {
            if consequence[from] {
                return Vec::new();
            }
            outgoing
                .iter()
                .filter_map(|edge| (!consequence[edge.to]).then_some(edge.to))
                .collect()
        })
        .collect()
}

/// Reconstruct edges from a premise occurrence to a bad terminal/cycle node.
fn reconstruct_local_path(
    start: usize,
    end: usize,
    parent: &[Option<(usize, usize)>],
) -> Vec<(usize, usize)> {
    let mut path = Vec::new();
    let mut current = end;
    while current != start {
        let Some((previous, edge)) = parent[current] else {
            break;
        };
        path.push((previous, edge));
        current = previous;
    }
    path.reverse();
    path
}

/// Find a deterministic concrete cycle beginning and ending at `start`.
fn find_cycle(
    start: usize,
    edges: &[Vec<SearchEdge>],
    consequence: &[bool],
) -> Option<Vec<(usize, usize)>> {
    for (edge_index, edge) in edges[start].iter().enumerate() {
        if edge.to == start {
            return Some(vec![(start, edge_index)]);
        }
    }

    let mut parent: Vec<Option<(usize, usize)>> = vec![None; edges.len()];
    let mut seen = vec![false; edges.len()];
    let mut queue = VecDeque::new();
    seen[start] = true;
    for (edge_index, edge) in edges[start].iter().enumerate() {
        if consequence[edge.to] || seen[edge.to] {
            continue;
        }
        seen[edge.to] = true;
        parent[edge.to] = Some((start, edge_index));
        queue.push_back(edge.to);
    }
    while let Some(node) = queue.pop_front() {
        for (edge_index, edge) in edges[node].iter().enumerate() {
            if edge.to == start {
                let mut path = reconstruct_local_path(start, node, &parent);
                path.push((node, edge_index));
                return Some(path);
            }
            if !consequence[edge.to] && !seen[edge.to] {
                seen[edge.to] = true;
                parent[edge.to] = Some((node, edge_index));
                queue.push_back(edge.to);
            }
        }
    }
    None
}

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

fn render_transition_counterexample(
    nodes: &[SearchNode],
    before: usize,
    action: &str,
    after: &str,
) -> String {
    let mut out = render_counterexample(nodes, before);
    out.push_str(&format!("  {action} -> {after}\n"));
    out
}

fn render_temporal_finite(
    nodes: &[SearchNode],
    edges: &[Vec<SearchEdge>],
    start: usize,
    path: &[(usize, usize)],
) -> String {
    let mut out = render_counterexample(nodes, start);
    out.push_str("temporal suffix:\n");
    if path.is_empty() {
        out.push_str(&format!("  premise: {}\n", nodes[start].rendered));
    } else {
        append_edge_path(&mut out, nodes, edges, path);
    }
    out
}

fn render_lasso(
    nodes: &[SearchNode],
    edges: &[Vec<SearchEdge>],
    start: usize,
    path: &[(usize, usize)],
    cycle: &[(usize, usize)],
) -> String {
    let mut out = render_counterexample(nodes, start);
    out.push_str("temporal prefix:\n");
    if path.is_empty() {
        out.push_str(&format!("  premise: {}\n", nodes[start].rendered));
    } else {
        append_edge_path(&mut out, nodes, edges, path);
    }
    out.push_str("cycle:\n");
    append_edge_path(&mut out, nodes, edges, cycle);
    out
}

fn append_edge_path(
    out: &mut String,
    nodes: &[SearchNode],
    edges: &[Vec<SearchEdge>],
    path: &[(usize, usize)],
) {
    for &(from, edge_index) in path {
        let edge = &edges[from][edge_index];
        out.push_str(&format!(
            "  {} -> {}\n",
            edge.action, nodes[edge.to].rendered
        ));
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
