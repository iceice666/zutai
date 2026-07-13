//! Pure, DOM-independent reconciliation of ordered node lists: sibling
//! `Html` lists (`diff_children`) and `Document.head` (`diff_head`).
//!
//! Steady-state updates retain the previously rendered tree (`App::rendered`
//! in `dom.rs`) in memory, so matching between renders can be computed as
//! plain data instead of reading identity back off the live DOM. Hydration
//! has no such retained tree — the only "old" state at that point is the
//! server-rendered HTML — and keeps using the DOM-walking patch in `dom.rs`
//! instead of this module.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::{Attribute, Declaration, Element, HeadNode, Html, StaticAttribute};

/// One operation per `new`-list position, produced by `diff_children` or
/// `diff_head`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOp {
    /// Reuse the node at `old_index`; it is already in the correct position
    /// relative to every other reused node, so no DOM move is needed.
    Keep { old_index: usize },
    /// Reuse the node at `old_index`, but move it into place first.
    Move { old_index: usize },
    /// No compatible old node exists at this position; create fresh DOM.
    Create,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListDiff {
    /// One entry per `new` position, in `new` order.
    pub ops: Vec<ListOp>,
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
pub fn diff_children<Msg>(old: &[Html<Msg>], new: &[Html<Msg>]) -> ListDiff {
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

    build_list_diff(old.len(), matches)
}

/// Diff a `Document.head` list the same way `diff_children` diffs sibling
/// `Html`, but matched by full structural equality instead of keys or tags.
/// Head nodes carry no independent state (no listeners, no typed input), so
/// two equal `HeadNode`s are fully interchangeable regardless of position —
/// unlike children, a `Keep`/`Move` match here never needs a content update,
/// only a possible reposition, and a changed node is simply an independent
/// `Create` (old copy removed, new one created — see `dom.rs`).
pub fn diff_head(old: &[HeadNode], new: &[HeadNode]) -> ListDiff {
    let mut consumed = vec![false; old.len()];
    let matches: Vec<Option<usize>> = new
        .iter()
        .map(|node| {
            let found = old
                .iter()
                .enumerate()
                .find(|(index, old_node)| !consumed[*index] && *old_node == node);
            found.map(|(index, _)| {
                consumed[index] = true;
                index
            })
        })
        .collect();

    build_list_diff(old.len(), matches)
}

/// Turn a list of per-new-position matches (`Some(old_index)` or `None` for
/// no match) against an old list of length `old_len` into a `ListDiff`: the
/// longest increasing subsequence of matched old indices needs no move
/// (`Keep`); every other match is a reposition (`Move`); unmatched
/// positions are `Create`; old indices that matched nothing are
/// `removed_old_indices`.
fn build_list_diff(old_len: usize, matches: Vec<Option<usize>>) -> ListDiff {
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

    let mut consumed = vec![false; old_len];
    let ops = matches
        .iter()
        .enumerate()
        .map(|(new_index, m)| match m {
            Some(old_index) => {
                consumed[*old_index] = true;
                if keep_positions.contains(&new_index) {
                    ListOp::Keep {
                        old_index: *old_index,
                    }
                } else {
                    ListOp::Move {
                        old_index: *old_index,
                    }
                }
            }
            None => ListOp::Create,
        })
        .collect();

    let removed_old_indices = consumed
        .iter()
        .enumerate()
        .filter_map(|(index, &used)| (!used).then_some(index))
        .collect();

    ListDiff {
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

/// What a changed or newly-set attribute name should become. Kept as
/// structured data (not a rendered string) so comparing an unchanged
/// `Styles` declaration list never needs to re-render CSS just to check
/// equality; only the apply step, when something actually changed, renders.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeEffect {
    Text(String),
    Styles(Vec<Declaration>),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AttributeDiff {
    pub removed: Vec<String>,
    pub set: Vec<(String, AttributeEffect)>,
}

/// Diff an element's non-event attributes (plus its `key`, which the DOM
/// carries as a `data-zutai-key` attribute). `value`/`checked` are excluded
/// entirely: those are DOM *properties* that can drift from their last
/// declared value through user interaction (typing, checking a box), so
/// they are always compared against live DOM state at apply time, not
/// old-vs-new declared value — see `diff_apply_element_attributes` in
/// `dom.rs`.
pub fn diff_element_attributes<Msg>(old: &Element<Msg>, new: &Element<Msg>) -> AttributeDiff {
    diff_attribute_maps(element_attribute_map(old), element_attribute_map(new))
}

/// Diff a plain `StaticAttribute` list (used for `Document.body_attributes`,
/// which has no `key`, no properties, and no events).
pub fn diff_static_attributes(old: &[StaticAttribute], new: &[StaticAttribute]) -> AttributeDiff {
    diff_attribute_maps(static_attribute_map(old), static_attribute_map(new))
}

fn diff_attribute_maps(
    old: HashMap<String, AttributeEffect>,
    new: HashMap<String, AttributeEffect>,
) -> AttributeDiff {
    let removed = old
        .keys()
        .filter(|name| !new.contains_key(name.as_str()))
        .cloned()
        .collect();
    let set = new
        .into_iter()
        .filter(|(name, effect)| old.get(name) != Some(effect))
        .collect();
    AttributeDiff { removed, set }
}

/// The final name -> effect map an element's attribute list produces,
/// mirroring `dom.rs`'s old clear-and-reapply loop: entries are applied in
/// list order and a later entry for the same name overwrites an earlier
/// one, but an attribute that renders to nothing (e.g. `Bool { value: false
/// }`) does not erase an earlier same-named entry, since the original loop
/// never called `remove_attribute` for it either.
fn element_attribute_map<Msg>(element: &Element<Msg>) -> HashMap<String, AttributeEffect> {
    let mut map = HashMap::new();
    if let Some(key) = &element.key {
        map.insert(
            "data-zutai-key".to_string(),
            AttributeEffect::Text(key.clone()),
        );
    }
    for attribute in &element.attributes {
        match attribute {
            Attribute::Static(attribute) => {
                if let Some((name, effect)) = static_attribute_effect(attribute) {
                    map.insert(name, effect);
                }
            }
            Attribute::TextProperty { name, .. } if name == "value" => {}
            Attribute::TextProperty { name, value } => {
                map.insert(name.clone(), AttributeEffect::Text(value.clone()));
            }
            Attribute::BoolProperty { name, .. } if name == "checked" => {}
            Attribute::BoolProperty { name, value: true } => {
                map.insert(name.clone(), AttributeEffect::Text(String::new()));
            }
            Attribute::BoolProperty { .. } | Attribute::Event(_) => {}
        }
    }
    map
}

fn static_attribute_map(attributes: &[StaticAttribute]) -> HashMap<String, AttributeEffect> {
    let mut map = HashMap::new();
    for attribute in attributes {
        if let Some((name, effect)) = static_attribute_effect(attribute) {
            map.insert(name, effect);
        }
    }
    map
}

fn static_attribute_effect(attribute: &StaticAttribute) -> Option<(String, AttributeEffect)> {
    match attribute {
        StaticAttribute::Text { name, value } => {
            Some((name.clone(), AttributeEffect::Text(value.clone())))
        }
        StaticAttribute::Bool { name, value: true } => {
            Some((name.clone(), AttributeEffect::Text(String::new())))
        }
        StaticAttribute::Bool { value: false, .. } => None,
        StaticAttribute::Styles(declarations) => Some((
            "style".to_string(),
            AttributeEffect::Styles(declarations.clone()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CssValue, Stylesheet};

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
            vec![ListOp::Keep { old_index: 0 }, ListOp::Keep { old_index: 1 },]
        );
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn tag_change_at_same_position_creates_fresh_node() {
        let old = vec![el("div", None)];
        let new = vec![el("span", None)];
        let diff = diff_children(&old, &new);
        assert_eq!(diff.ops, vec![ListOp::Create]);
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
                ListOp::Keep { old_index: 1 },
                ListOp::Keep { old_index: 2 },
                ListOp::Move { old_index: 0 },
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
            vec![ListOp::Keep { old_index: 0 }, ListOp::Create]
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
        assert_eq!(diff.ops, vec![ListOp::Keep { old_index: 0 }]);
        assert_eq!(diff.removed_old_indices, vec![1, 2]);
    }

    #[test]
    fn growing_list_creates_new_trailing_nodes() {
        let old = vec![el("li", Some("a"))];
        let new = vec![el("li", Some("a")), el("li", Some("b"))];
        let diff = diff_children(&old, &new);
        assert_eq!(
            diff.ops,
            vec![ListOp::Keep { old_index: 0 }, ListOp::Create]
        );
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn text_nodes_match_positionally() {
        let old = vec![text("hello")];
        let new = vec![text("world")];
        let diff = diff_children(&old, &new);
        assert_eq!(diff.ops, vec![ListOp::Keep { old_index: 0 }]);
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn text_and_element_at_same_position_do_not_match() {
        let old = vec![text("hello")];
        let new = vec![el("span", None)];
        let diff = diff_children(&old, &new);
        assert_eq!(diff.ops, vec![ListOp::Create]);
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
            .filter(|op| matches!(op, ListOp::Create))
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
        assert!(diff.ops.iter().all(|op| !matches!(op, ListOp::Create)));
    }

    fn elem(tag: &str, attributes: Vec<Attribute>) -> Element {
        Element {
            tag: tag.to_string(),
            key: None,
            attributes,
            children: Vec::new(),
        }
    }

    fn text_attr(name: &str, value: &str) -> Attribute {
        Attribute::Static(StaticAttribute::Text {
            name: name.to_string(),
            value: value.to_string(),
        })
    }

    fn bool_attr(name: &str, value: bool) -> Attribute {
        Attribute::Static(StaticAttribute::Bool {
            name: name.to_string(),
            value,
        })
    }

    #[test]
    fn unchanged_attributes_produce_no_diff() {
        let old = elem("div", vec![text_attr("class", "a")]);
        let new = elem("div", vec![text_attr("class", "a")]);
        assert_eq!(
            diff_element_attributes(&old, &new),
            AttributeDiff::default()
        );
    }

    #[test]
    fn changed_attribute_value_only_touches_that_name() {
        let old = elem("div", vec![text_attr("class", "a"), text_attr("id", "x")]);
        let new = elem("div", vec![text_attr("class", "b"), text_attr("id", "x")]);
        let diff = diff_element_attributes(&old, &new);
        assert!(diff.removed.is_empty());
        assert_eq!(
            diff.set,
            vec![("class".to_string(), AttributeEffect::Text("b".to_string()))]
        );
    }

    #[test]
    fn dropped_attribute_is_removed() {
        let old = elem("div", vec![text_attr("title", "hi")]);
        let new = elem("div", vec![]);
        let diff = diff_element_attributes(&old, &new);
        assert_eq!(diff.removed, vec!["title".to_string()]);
        assert!(diff.set.is_empty());
    }

    #[test]
    fn added_attribute_is_set() {
        let old = elem("div", vec![]);
        let new = elem("div", vec![text_attr("title", "hi")]);
        let diff = diff_element_attributes(&old, &new);
        assert!(diff.removed.is_empty());
        assert_eq!(
            diff.set,
            vec![("title".to_string(), AttributeEffect::Text("hi".to_string()))]
        );
    }

    #[test]
    fn bool_attribute_true_to_false_removes_it() {
        let old = elem("input", vec![bool_attr("disabled", true)]);
        let new = elem("input", vec![bool_attr("disabled", false)]);
        let diff = diff_element_attributes(&old, &new);
        assert_eq!(diff.removed, vec!["disabled".to_string()]);
        assert!(diff.set.is_empty());
    }

    #[test]
    fn bool_attribute_false_to_true_sets_it() {
        let old = elem("input", vec![bool_attr("disabled", false)]);
        let new = elem("input", vec![bool_attr("disabled", true)]);
        let diff = diff_element_attributes(&old, &new);
        assert!(diff.removed.is_empty());
        assert_eq!(
            diff.set,
            vec![("disabled".to_string(), AttributeEffect::Text(String::new()))]
        );
    }

    #[test]
    fn unchanged_style_declarations_produce_no_diff_even_though_css_is_unrendered() {
        let declarations = vec![Declaration {
            property: "opacity".to_string(),
            value: CssValue::Number(0.5),
            important: false,
        }];
        let old = elem(
            "div",
            vec![Attribute::Static(StaticAttribute::Styles(
                declarations.clone(),
            ))],
        );
        let new = elem(
            "div",
            vec![Attribute::Static(StaticAttribute::Styles(declarations))],
        );
        assert_eq!(
            diff_element_attributes(&old, &new),
            AttributeDiff::default()
        );
    }

    #[test]
    fn key_change_diffs_as_the_data_zutai_key_attribute() {
        let old: Element = Element {
            tag: "li".to_string(),
            key: Some("a".to_string()),
            attributes: Vec::new(),
            children: Vec::new(),
        };
        let new: Element = Element {
            tag: "li".to_string(),
            key: Some("b".to_string()),
            attributes: Vec::new(),
            children: Vec::new(),
        };
        let diff = diff_element_attributes(&old, &new);
        assert_eq!(
            diff.set,
            vec![(
                "data-zutai-key".to_string(),
                AttributeEffect::Text("b".to_string())
            )]
        );
    }

    #[test]
    fn declared_value_and_checked_never_appear_in_the_attribute_diff() {
        let old = elem(
            "input",
            vec![
                Attribute::TextProperty {
                    name: "value".to_string(),
                    value: "old".to_string(),
                },
                Attribute::BoolProperty {
                    name: "checked".to_string(),
                    value: false,
                },
            ],
        );
        let new = elem(
            "input",
            vec![
                Attribute::TextProperty {
                    name: "value".to_string(),
                    value: "new".to_string(),
                },
                Attribute::BoolProperty {
                    name: "checked".to_string(),
                    value: true,
                },
            ],
        );
        assert_eq!(
            diff_element_attributes(&old, &new),
            AttributeDiff::default()
        );
    }

    #[test]
    fn static_attribute_list_diffs_the_same_way() {
        let old = vec![StaticAttribute::Text {
            name: "class".to_string(),
            value: "a".to_string(),
        }];
        let new = vec![StaticAttribute::Text {
            name: "class".to_string(),
            value: "b".to_string(),
        }];
        let diff = diff_static_attributes(&old, &new);
        assert_eq!(
            diff.set,
            vec![("class".to_string(), AttributeEffect::Text("b".to_string()))]
        );
    }

    fn meta(name: &str, content: &str) -> HeadNode {
        HeadNode::MetaName {
            name: name.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn unchanged_head_list_is_entirely_kept() {
        let old = vec![meta("description", "a"), meta("author", "b")];
        let new = old.clone();
        let diff = diff_head(&old, &new);
        assert_eq!(
            diff.ops,
            vec![ListOp::Keep { old_index: 0 }, ListOp::Keep { old_index: 1 },]
        );
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn changed_head_node_content_is_replaced_not_kept() {
        // Head nodes carry no independent state, so a content change is
        // just "old copy gone, new copy created" rather than an in-place
        // update — there is nothing meaningful to patch in place.
        let old = vec![meta("description", "old css-driven copy")];
        let new = vec![meta("description", "new css-driven copy")];
        let diff = diff_head(&old, &new);
        assert_eq!(diff.ops, vec![ListOp::Create]);
        assert_eq!(diff.removed_old_indices, vec![0]);
    }

    #[test]
    fn unchanged_style_node_is_kept_without_touching_the_dom() {
        let old = vec![HeadNode::Style(Stylesheet::default())];
        let new = vec![HeadNode::Style(Stylesheet::default())];
        let diff = diff_head(&old, &new);
        assert_eq!(diff.ops, vec![ListOp::Keep { old_index: 0 }]);
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn reordered_head_nodes_move_only_the_one_that_needs_to() {
        let old = vec![meta("a", "1"), meta("b", "2"), meta("c", "3")];
        let new = vec![meta("b", "2"), meta("c", "3"), meta("a", "1")];
        let diff = diff_head(&old, &new);
        assert_eq!(
            diff.ops,
            vec![
                ListOp::Keep { old_index: 1 },
                ListOp::Keep { old_index: 2 },
                ListOp::Move { old_index: 0 },
            ]
        );
        assert!(diff.removed_old_indices.is_empty());
    }

    #[test]
    fn dropped_head_node_is_removed_and_not_recreated() {
        let old = vec![meta("a", "1"), meta("b", "2")];
        let new = vec![meta("a", "1")];
        let diff = diff_head(&old, &new);
        assert_eq!(diff.ops, vec![ListOp::Keep { old_index: 0 }]);
        assert_eq!(diff.removed_old_indices, vec![1]);
    }

    #[test]
    fn added_head_node_is_created() {
        let old = vec![meta("a", "1")];
        let new = vec![meta("a", "1"), meta("b", "2")];
        let diff = diff_head(&old, &new);
        assert_eq!(
            diff.ops,
            vec![ListOp::Keep { old_index: 0 }, ListOp::Create]
        );
        assert!(diff.removed_old_indices.is_empty());
    }
}
