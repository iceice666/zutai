use std::collections::{HashMap, HashSet};

use crate::*;

use super::refs::*;
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn walk_child(
    graph: &DataflowGraph,
    owner: NodeId,
    field: &'static str,
    target: NodeId,
    scope: &mut Scope,
    owners: &HashMap<NodeId, BindOwner>,
    visited: &mut HashSet<(NodeId, Scope)>,
    errors: &mut Vec<ValidationError>,
) {
    if check_node_ref(graph, owner, field, target, errors) {
        walk_node(graph, target, scope, owners, visited, errors);
    }
}

pub(super) fn walk_node(
    graph: &DataflowGraph,
    id: NodeId,
    scope: &mut Scope,
    owners: &HashMap<NodeId, BindOwner>,
    visited: &mut HashSet<(NodeId, Scope)>,
    errors: &mut Vec<ValidationError>,
) {
    if !node_exists(graph, id) {
        return;
    }

    if !visited.insert((id, scope.clone())) {
        return;
    }

    if matches!(&graph.nodes[id].kind, DfNodeKind::Bind)
        && let Some(owner) = owners.get(&id).copied()
    {
        match owner {
            BindOwner::Lambda(owner_lambda) => {
                if !scope.lambdas.contains(&owner_lambda) {
                    errors.push(ValidationError::LambdaCaptureViolation {
                        bind: id,
                        owner_lambda,
                        use_site: id,
                    });
                }
            }
            BindOwner::Arm {
                match_node,
                arm_index,
            } => {
                if !scope.arm_binds.contains(&id) {
                    errors.push(ValidationError::ArmBindScopeViolation {
                        bind: id,
                        match_node,
                        arm_index,
                        use_site: id,
                    });
                }
            }
        }
    }

    match &graph.nodes[id].kind {
        DfNodeKind::Lambda { body, .. } => {
            scope.lambdas.push(id);
            walk_child(graph, id, "body", *body, scope, owners, visited, errors);
            scope.lambdas.pop();
        }
        DfNodeKind::Apply { func, arg } => {
            walk_child(graph, id, "func", *func, scope, owners, visited, errors);
            walk_child(graph, id, "arg", *arg, scope, owners, visited, errors);
        }
        DfNodeKind::TyLam { body, .. } => {
            walk_child(graph, id, "body", *body, scope, owners, visited, errors);
        }
        DfNodeKind::TyApp { poly, .. } => {
            walk_child(graph, id, "poly", *poly, scope, owners, visited, errors);
        }
        DfNodeKind::Record(fields) => {
            for (_, value) in fields {
                walk_child(graph, id, "field", *value, scope, owners, visited, errors);
            }
        }
        DfNodeKind::RecordUpdate { base, updates } => {
            walk_child(graph, id, "base", *base, scope, owners, visited, errors);
            for (_, _, value) in updates {
                walk_child(graph, id, "update", *value, scope, owners, visited, errors);
            }
        }
        DfNodeKind::Tuple(items) => {
            for item in items {
                match item {
                    DfTupleNodeItem::Named { value, .. } | DfTupleNodeItem::Positional(value) => {
                        walk_child(graph, id, "item", *value, scope, owners, visited, errors);
                    }
                }
            }
        }
        DfNodeKind::List(items) => {
            for item in items {
                walk_child(graph, id, "element", *item, scope, owners, visited, errors);
            }
        }
        DfNodeKind::Variant { value, .. } => {
            walk_child(graph, id, "payload", *value, scope, owners, visited, errors);
        }
        DfNodeKind::Select { base, .. } => {
            walk_child(graph, id, "base", *base, scope, owners, visited, errors);
        }
        DfNodeKind::Match { scrutinee, arms } => {
            walk_child(
                graph,
                id,
                "scrutinee",
                *scrutinee,
                scope,
                owners,
                visited,
                errors,
            );
            for (arm_index, arm) in arms.iter().enumerate() {
                let mut bind_nodes = Vec::new();
                collect_bind_nodes(&arm.pattern, &mut bind_nodes);
                let old_len = scope.arm_binds.len();
                scope.arm_binds.extend(bind_nodes);
                if let Some(guard) = arm.guard {
                    walk_child(graph, id, "guard", guard, scope, owners, visited, errors);
                }
                walk_child(graph, id, "body", arm.body, scope, owners, visited, errors);
                scope.arm_binds.truncate(old_len);

                debug_assert!(arm_index < arms.len());
            }
        }
        DfNodeKind::Coalesce { value, fallback } => {
            walk_child(graph, id, "value", *value, scope, owners, visited, errors);
            walk_child(
                graph, id, "fallback", *fallback, scope, owners, visited, errors,
            );
        }
        DfNodeKind::Builtin(_, lhs, rhs) => {
            walk_child(graph, id, "lhs", *lhs, scope, owners, visited, errors);
            walk_child(graph, id, "rhs", *rhs, scope, owners, visited, errors);
        }
        DfNodeKind::HostPrint { arg } => {
            walk_child(graph, id, "arg", *arg, scope, owners, visited, errors);
        }
        DfNodeKind::HostOp { arg, .. } => {
            walk_child(graph, id, "arg", *arg, scope, owners, visited, errors);
        }
        DfNodeKind::Sequence(items) => {
            for item in items {
                walk_child(graph, id, "item", *item, scope, owners, visited, errors);
            }
        }
        DfNodeKind::Lit(_)
        | DfNodeKind::Bind
        | DfNodeKind::GlobalRef(_)
        | DfNodeKind::Import { .. }
        | DfNodeKind::Error => {}
    }
}
