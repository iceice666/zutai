//! Pure, DOM-independent reconciliation of sibling `Html` lists.
//!
//! Steady-state updates retain the previously rendered tree (`App::rendered`
//! in `dom.rs`) in memory, so matching children between renders can be
//! computed as plain data instead of reading identity back off the live DOM.
//! Hydration has no such retained tree — the only "old" state at that point
//! is the server-rendered HTML — and keeps using the DOM-walking patch in
//! `dom.rs` instead of this module.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::{Element, Html};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildOp {
    /// Reuse the DOM node at `old_index`; it is already in the correct
    /// position relative to every other reused node, so no DOM move is
    /// needed.
    Keep { old_index: usize },
    /// Reuse the DOM node at `old_index`, but move it into place first.
    Move { old_index: usize },
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

/// Match `new` children against `old` children, then choose a minimal move
/// set so callers only need to reposition nodes that actually changed order.
///
/// Matching rules:
/// - A keyed node matches the old node with the same key, anywhere in the
///   list.
/// - An unkeyed node matches the earliest not-yet-consumed unkeyed old node
///   of the same kind (text, or element with the same tag), so a single
///   mid-list insert or removal re-syncs on the next unkeyed sibling instead
///   of cascading into replacing everything after it.
///
/// Move selection: the longest increasing subsequence of matched old indices
/// is already in correct relative order and is marked `Keep` (no DOM move);
/// every other match is `Move`.
pub fn diff_children<Msg>(old: &[Html<Msg>], new: &[Html<Msg>]) -> ChildDiff {
    let mut keyed_old: HashMap<&str, usize> = HashMap::new();
    let mut unkeyed_old: HashMap<UnkeyedKind, VecDeque<usize>> = HashMap::new();
    for (index, node) in old.iter().enumerate() {
        match child_key(node) {
            Some(key) => {
                keyed_old.entry(key).or_insert(index);
            }
            None => unkeyed_old
                .entry(unkeyed_kind(node))
                .or_default()
                .push_back(index),
        }
    }

    let mut consumed = vec![false; old.len()];
    let matches: Vec<Option<usize>> = new
        .iter()
        .map(|node| {
            let candidate = match child_key(node) {
                Some(key) => keyed_old
                    .get(key)
                    .copied()
                    .filter(|&index| !consumed[index]),
                None => unkeyed_old
                    .get_mut(&unkeyed_kind(node))
                    .and_then(VecDeque::pop_front),
            };
            match candidate {
                Some(old_index) if compatible(&old[old_index], node) => {
                    consumed[old_index] = true;
                    Some(old_index)
                }
                _ => None,
            }
        })
        .collect();

    let matched_positions: Vec<usize> = matches
        .iter()
        .enumerate()
        .filter_map(|(new_index, m)| m.map(|_| new_index))
        .collect();
    let matched_old_indices: Vec<usize> = matches.iter().filter_map(|m| *m).collect();
    let keep_positions: HashSet<usize> = longest_increasing_subsequence(&matched_old_indices)
        .into_iter()
        .map(|i| matched_positions[i])
        .collect();

    let ops = matches
        .iter()
        .enumerate()
        .map(|(new_index, m)| match m {
            Some(old_index) if keep_positions.contains(&new_index) => ChildOp::Keep {
                old_index: *old_index,
            },
            Some(old_index) => ChildOp::Move {
                old_index: *old_index,
            },
            None => ChildOp::Create,
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

/// Indices into `values` forming a strictly increasing subsequence of
/// maximal length, in ascending index order. `values` must contain no
/// duplicates (guaranteed here since each old index is matched at most
/// once).
fn longest_increasing_subsequence(values: &[usize]) -> Vec<usize> {
    let mut tails: Vec<usize> = Vec::new();
    let mut predecessors: Vec<Option<usize>> = vec![None; values.len()];

    for (i, &value) in values.iter().enumerate() {
        let position = tails.partition_point(|&tail_index| values[tail_index] < value);
        if position > 0 {
            predecessors[i] = Some(tails[position - 1]);
        }
        if position == tails.len() {
            tails.push(i);
        } else {
            tails[position] = i;
        }
    }

    let mut result = Vec::with_capacity(tails.len());
    let mut current = tails.last().copied();
    while let Some(index) = current {
        result.push(index);
        current = predecessors[index];
    }
    result.reverse();
    result
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum UnkeyedKind {
    Text,
    Element(String),
}

fn unkeyed_kind<Msg>(node: &Html<Msg>) -> UnkeyedKind {
    match node {
        Html::Text(_) => UnkeyedKind::Text,
        Html::Element(element) => UnkeyedKind::Element(element.tag.clone()),
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
    fn lis_finds_a_maximal_increasing_subsequence() {
        assert_eq!(longest_increasing_subsequence(&[1, 2, 0]), vec![0, 1]);
        assert_eq!(
            longest_increasing_subsequence(&[3, 1, 2, 0, 4]),
            vec![1, 2, 4]
        );
        assert_eq!(longest_increasing_subsequence(&[]), Vec::<usize>::new());
        assert_eq!(longest_increasing_subsequence(&[5]), vec![0]);
    }

    #[test]
    fn matches_unkeyed_same_tag_at_same_position() {
        let old = vec![el("div", None), el("span", None)];
        let new = vec![el("div", None), el("span", None)];
        let diff = diff_children(&old, &new);
        assert_eq!(
            diff.ops,
            vec![
                ChildOp::Keep { old_index: 0 },
                ChildOp::Keep { old_index: 1 },
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
    fn keyed_children_reorder_with_a_single_minimal_move() {
        let old = vec![
            el("li", Some("a")),
            el("li", Some("b")),
            el("li", Some("c")),
        ];
        // Rotate: only "a" needs to move to the end; "b" and "c" are already
        // in relative order and should stay put.
        let new = vec![
            el("li", Some("b")),
            el("li", Some("c")),
            el("li", Some("a")),
        ];
        let diff = diff_children(&old, &new);
        assert_eq!(
            diff.ops,
            vec![
                ChildOp::Keep { old_index: 1 },
                ChildOp::Keep { old_index: 2 },
                ChildOp::Move { old_index: 0 },
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
            vec![ChildOp::Keep { old_index: 0 }, ChildOp::Create]
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
        assert_eq!(diff.ops, vec![ChildOp::Keep { old_index: 0 }]);
        assert_eq!(diff.removed_old_indices, vec![1, 2]);
    }

    #[test]
    fn growing_list_creates_new_trailing_nodes() {
        let old = vec![el("li", Some("a"))];
        let new = vec![el("li", Some("a")), el("li", Some("b"))];
        let diff = diff_children(&old, &new);
        assert_eq!(
            diff.ops,
            vec![ChildOp::Keep { old_index: 0 }, ChildOp::Create]
        );
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn text_nodes_match_positionally() {
        let old = vec![text("hello")];
        let new = vec![text("world")];
        let diff = diff_children(&old, &new);
        assert_eq!(diff.ops, vec![ChildOp::Keep { old_index: 0 }]);
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

    #[test]
    fn unkeyed_mid_list_insert_does_not_cascade_into_replacing_the_tail() {
        // A single `div` inserted in the middle of an unkeyed list used to
        // (pre-milestone-2) desync every following sibling by position and
        // recreate all of them. Same-kind FIFO matching should instead
        // resync on the next unkeyed sibling of matching tag and reuse all
        // three original nodes, creating only the one genuinely new node.
        let old = vec![el("div", None), el("span", None), el("div", None)];
        let new = vec![
            el("div", None),
            el("div", None),
            el("span", None),
            el("div", None),
        ];
        let diff = diff_children(&old, &new);
        assert!(
            diff.removed_old_indices.is_empty(),
            "all three original nodes should have been reused, not replaced"
        );
        let creates = diff
            .ops
            .iter()
            .filter(|op| matches!(op, ChildOp::Create))
            .count();
        assert_eq!(creates, 1, "only the genuinely new node should be created");
    }

    #[test]
    fn unkeyed_mid_list_removal_does_not_cascade_into_replacing_the_tail() {
        let old = vec![
            el("div", None),
            el("div", None),
            el("span", None),
            el("div", None),
        ];
        let new = vec![el("div", None), el("span", None), el("div", None)];
        let diff = diff_children(&old, &new);
        assert_eq!(
            diff.removed_old_indices.len(),
            1,
            "exactly one stale node should be dropped, not the whole tail"
        );
        assert!(diff.ops.iter().all(|op| !matches!(op, ChildOp::Create)));
    }
}
