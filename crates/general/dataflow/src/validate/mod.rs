use std::collections::HashSet;

mod compat;
mod refs;
mod walk;

use self::compat::*;
use self::refs::*;
use self::walk::*;

use crate::{DataflowGraph, DfNodeKind, NodeId, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BindOwner {
    Lambda(NodeId),
    Arm {
        match_node: NodeId,
        arm_index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
struct Scope {
    lambdas: Vec<NodeId>,
    arm_binds: Vec<NodeId>,
}

pub(crate) fn validate(graph: &DataflowGraph) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    if graph.spans.len() != graph.nodes.len() {
        errors.push(ValidationError::SpanTableSizeMismatch {
            spans: graph.spans.len(),
            nodes: graph.nodes.len(),
        });
    }

    let root_valid = if node_exists(graph, graph.root) {
        true
    } else {
        errors.push(ValidationError::InvalidRootNode { target: graph.root });
        false
    };

    for (node_id, node) in graph.nodes.iter() {
        if !type_exists(graph, node.ty) {
            errors.push(ValidationError::InvalidNodeType {
                node: node_id,
                ty: node.ty,
            });
        }
    }

    validate_type_refs(graph, &mut errors);

    for (node_id, node) in graph.nodes.iter() {
        check_node_refs(graph, node_id, &mut errors);
        if type_exists(graph, node.ty) {
            check_node_type_compat(graph, node_id, &mut errors);
        }
        if let DfNodeKind::GlobalRef(name) = &node.kind
            && !graph.globals.contains_key(name.as_str())
        {
            errors.push(ValidationError::StrayGlobalRef { name: name.clone() });
        }
    }

    let owners = collect_bind_owners(graph, &mut errors);
    let mut visited = HashSet::new();

    if root_valid {
        let mut scope = Scope::default();
        walk_node(
            graph,
            graph.root,
            &mut scope,
            &owners,
            &mut visited,
            &mut errors,
        );
    }

    for &global in graph.globals.values() {
        if node_exists(graph, global) {
            let mut scope = Scope::default();
            walk_node(
                graph,
                global,
                &mut scope,
                &owners,
                &mut visited,
                &mut errors,
            );
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
