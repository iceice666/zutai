//! Pure, DOM-independent reconciliation of sibling `Html` lists.
//!
//! Steady-state updates retain the previously rendered tree (`App::rendered`
//! in `dom.rs`) in memory, so matching children between renders can be
//! computed as plain data instead of reading identity back off the live DOM.
//! Hydration has no such retained tree — the only "old" state at that point
//! is the server-rendered HTML — and keeps using the DOM-walking patch in
//! `dom.rs` instead of this module.

use std::collections::HashMap;

use crate::{Element, Html};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildOp {
    /// Reuse the DOM node that currently corresponds to `old[old_index]`.
    Update { old_index: usize },
    /// No compatible old node exists at this position; create fresh DOM.
    Create,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildDiff {
    /// One entry per `new` position, in `new` order.
    pub ops: Vec<ChildOp>,
    /// Old indices with no surviving match, ascending, for removal.
    pub removed_old_indices: Vec<usize>,
}

/// Match `new` children against `old` children by key (anywhere in the list)
/// or, for unkeyed nodes, by identical position, mirroring the matching
/// rules the DOM-walking hydration patch already applies.
pub fn diff_children<Msg>(old: &[Html<Msg>], new: &[Html<Msg>]) -> ChildDiff {
    let mut keyed_old: HashMap<&str, usize> = HashMap::new();
    for (index, node) in old.iter().enumerate() {
        if let Some(key) = child_key(node) {
            keyed_old.entry(key).or_insert(index);
        }
    }

    let mut consumed = vec![false; old.len()];
    let ops = new
        .iter()
        .enumerate()
        .map(|(new_index, node)| {
            let candidate = match child_key(node) {
                Some(key) => keyed_old.get(key).copied(),
                None => (new_index < old.len() && child_key(&old[new_index]).is_none())
                    .then_some(new_index),
            };
            match candidate {
                Some(old_index) if !consumed[old_index] && compatible(&old[old_index], node) => {
                    consumed[old_index] = true;
                    ChildOp::Update { old_index }
                }
                _ => ChildOp::Create,
            }
        })
        .collect();

    let removed_old_indices = consumed
        .iter()
        .enumerate()
        .filter_map(|(index, &used)| (!used).then_some(index))
        .collect();

    ChildDiff {
        ops,
        removed_old_indices,
    }
}

fn compatible<Msg>(old: &Html<Msg>, new: &Html<Msg>) -> bool {
    match (old, new) {
        (Html::Text(_), Html::Text(_)) => true,
        (Html::Element(a), Html::Element(b)) => a.tag.eq_ignore_ascii_case(&b.tag),
        _ => false,
    }
}

pub(crate) fn child_key<Msg>(node: &Html<Msg>) -> Option<&str> {
    match node {
        Html::Element(Element { key, .. }) => key.as_deref(),
        Html::Text(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Html {
        Html::Text(value.to_string())
    }

    fn el(tag: &str, key: Option<&str>) -> Html {
        Html::Element(Element {
            tag: tag.to_string(),
            key: key.map(str::to_string),
            attributes: Vec::new(),
            children: Vec::new(),
        })
    }

    #[test]
    fn matches_unkeyed_same_tag_at_same_position() {
        let old = vec![el("div", None), el("span", None)];
        let new = vec![el("div", None), el("span", None)];
        let diff = diff_children(&old, &new);
        assert_eq!(
            diff.ops,
            vec![
                ChildOp::Update { old_index: 0 },
                ChildOp::Update { old_index: 1 },
            ]
        );
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn tag_change_at_same_position_creates_fresh_node() {
        let old = vec![el("div", None)];
        let new = vec![el("span", None)];
        let diff = diff_children(&old, &new);
        assert_eq!(diff.ops, vec![ChildOp::Create]);
        assert_eq!(diff.removed_old_indices, vec![0]);
    }

    #[test]
    fn keyed_children_reuse_regardless_of_position() {
        let old = vec![
            el("li", Some("a")),
            el("li", Some("b")),
            el("li", Some("c")),
        ];
        let new = vec![
            el("li", Some("c")),
            el("li", Some("a")),
            el("li", Some("b")),
        ];
        let diff = diff_children(&old, &new);
        assert_eq!(
            diff.ops,
            vec![
                ChildOp::Update { old_index: 2 },
                ChildOp::Update { old_index: 0 },
                ChildOp::Update { old_index: 1 },
            ]
        );
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn duplicate_key_in_new_list_only_reuses_the_old_node_once() {
        let old = vec![el("li", Some("a"))];
        let new = vec![el("li", Some("a")), el("li", Some("a"))];
        let diff = diff_children(&old, &new);
        assert_eq!(
            diff.ops,
            vec![ChildOp::Update { old_index: 0 }, ChildOp::Create]
        );
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn shrinking_list_marks_dropped_old_nodes_removed() {
        let old = vec![
            el("li", Some("a")),
            el("li", Some("b")),
            el("li", Some("c")),
        ];
        let new = vec![el("li", Some("a"))];
        let diff = diff_children(&old, &new);
        assert_eq!(diff.ops, vec![ChildOp::Update { old_index: 0 }]);
        assert_eq!(diff.removed_old_indices, vec![1, 2]);
    }

    #[test]
    fn growing_list_creates_new_trailing_nodes() {
        let old = vec![el("li", Some("a"))];
        let new = vec![el("li", Some("a")), el("li", Some("b"))];
        let diff = diff_children(&old, &new);
        assert_eq!(
            diff.ops,
            vec![ChildOp::Update { old_index: 0 }, ChildOp::Create]
        );
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn text_nodes_match_positionally() {
        let old = vec![text("hello")];
        let new = vec![text("world")];
        let diff = diff_children(&old, &new);
        assert_eq!(diff.ops, vec![ChildOp::Update { old_index: 0 }]);
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn text_and_element_at_same_position_do_not_match() {
        let old = vec![text("hello")];
        let new = vec![el("span", None)];
        let diff = diff_children(&old, &new);
        assert_eq!(diff.ops, vec![ChildOp::Create]);
        assert_eq!(diff.removed_old_indices, vec![0]);
    }
}
