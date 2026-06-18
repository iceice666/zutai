use std::collections::HashMap;

use crate::{DataflowGraph, DfNodeKind, DfPattern, DfTuplePatItem, NodeId, ValidationError};

/// Collect all `Bind` nodes introduced by a pattern.
fn collect_bind_nodes(pat: &DfPattern, out: &mut Vec<NodeId>) {
    match pat {
        DfPattern::Bind(n) => out.push(*n),
        DfPattern::Tuple(items) => {
            for item in items {
                match item {
                    DfTuplePatItem::Named { pattern, .. } => collect_bind_nodes(pattern, out),
                    DfTuplePatItem::Positional(p) => collect_bind_nodes(p, out),
                }
            }
        }
        DfPattern::Record(fields) => {
            for (_, p) in fields {
                collect_bind_nodes(p, out);
            }
        }
        DfPattern::Variant(_, inner) => collect_bind_nodes(inner, out),
        _ => {}
    }
}

pub(crate) fn validate(graph: &DataflowGraph) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    // Invariant 6: span table size.
    if graph.spans.len() != graph.nodes.len() {
        errors.push(ValidationError::SpanTableSizeMismatch {
            spans: graph.spans.len(),
            nodes: graph.nodes.len(),
        });
    }

    // Traverse all nodes to build ownership and GlobalRef sets.
    let mut bind_owner_count: HashMap<NodeId, usize> = HashMap::new();
    let mut stray_refs: Vec<String> = Vec::new();

    for (_, node) in graph.nodes.iter() {
        match &node.kind {
            DfNodeKind::Lambda { param, .. } => {
                *bind_owner_count.entry(*param).or_default() += 1;
            }
            DfNodeKind::Match { arms, .. } => {
                for arm in arms {
                    let mut bind_nodes = Vec::new();
                    collect_bind_nodes(&arm.pattern, &mut bind_nodes);
                    for n in bind_nodes {
                        *bind_owner_count.entry(n).or_default() += 1;
                    }
                }
            }
            DfNodeKind::GlobalRef(name) => {
                if !graph.globals.contains_key(name.as_str()) {
                    stray_refs.push(name.clone());
                }
            }
            _ => {}
        }
    }

    // Invariant 2: Bind ownership — each Bind owned by exactly one Lambda or arm.
    for (node_id, node) in graph.nodes.iter() {
        if matches!(node.kind, DfNodeKind::Bind) {
            let count = bind_owner_count.get(&node_id).copied().unwrap_or(0);
            if count != 1 {
                errors.push(ValidationError::BindOwnershipViolation { count });
            }
        }
    }

    // Invariant 5: No stray GlobalRefs.
    for name in stray_refs {
        errors.push(ValidationError::StrayGlobalRef { name });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
