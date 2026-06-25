use rustc_hash::{FxHashMap, FxHashSet};

use crate::*;

use super::*;

pub(super) fn node_exists(graph: &DataflowGraph, id: NodeId) -> bool {
    (id.into_raw().into_u32() as usize) < graph.nodes.len()
}

pub(super) fn type_exists(graph: &DataflowGraph, id: DfTyId) -> bool {
    (id.into_raw().into_u32() as usize) < graph.types.len()
}

pub(super) fn same_type(graph: &DataflowGraph, expected: DfTyId, actual: DfTyId) -> bool {
    fn go(
        graph: &DataflowGraph,
        expected: DfTyId,
        actual: DfTyId,
        seen: &mut FxHashSet<(DfTyId, DfTyId)>,
    ) -> bool {
        if expected == actual {
            return true;
        }
        if !type_exists(graph, expected) || !type_exists(graph, actual) {
            return false;
        }
        if !seen.insert((expected, actual)) {
            return true;
        }

        match (&graph.types[expected], &graph.types[actual]) {
            (DfTy::Int, DfTy::Int)
            | (DfTy::Float, DfTy::Float)
            | (DfTy::Bool, DfTy::Bool)
            | (DfTy::True, DfTy::True)
            | (DfTy::False, DfTy::False)
            | (DfTy::Text, DfTy::Text)
            | (DfTy::Atom, DfTy::Atom)
            | (DfTy::Type, DfTy::Type)
            | (DfTy::Error, DfTy::Error) => true,
            (DfTy::Opaque(a), DfTy::Opaque(b)) => a == b,
            (DfTy::Posit(a), DfTy::Posit(b)) => a == b,
            (DfTy::Bool, DfTy::True | DfTy::False) | (DfTy::True | DfTy::False, DfTy::Bool) => true,
            (DfTy::TyVar(a), DfTy::TyVar(b)) => a == b,
            (DfTy::List(a), DfTy::List(b))
            | (DfTy::Optional(a), DfTy::Optional(b))
            | (DfTy::Maybe(a), DfTy::Maybe(b)) => go(graph, *a, *b, seen),
            (DfTy::Fun(a_arg, a_result), DfTy::Fun(b_arg, b_result)) => {
                go(graph, *a_arg, *b_arg, seen) && go(graph, *a_result, *b_result, seen)
            }
            (DfTy::Record(a), DfTy::Record(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        a.name == b.name && a.optional == b.optional && go(graph, a.ty, b.ty, seen)
                    })
            }
            (DfTy::Union(a), DfTy::Union(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(a, b)| a.tag == b.tag && go(graph, a.ty, b.ty, seen))
            }
            (DfTy::Tuple(a), DfTy::Tuple(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| match (a, b) {
                        (
                            DfTupleField::Named {
                                name: a_name,
                                ty: a_ty,
                            },
                            DfTupleField::Named {
                                name: b_name,
                                ty: b_ty,
                            },
                        ) => a_name == b_name && go(graph, *a_ty, *b_ty, seen),
                        (DfTupleField::Positional(a_ty), DfTupleField::Positional(b_ty)) => {
                            go(graph, *a_ty, *b_ty, seen)
                        }
                        _ => false,
                    })
            }
            (DfTy::TyFun(a_params, a_body), DfTy::TyFun(b_params, b_body)) => {
                a_params == b_params && go(graph, *a_body, *b_body, seen)
            }
            (DfTy::TyApp(a_func, a_args), DfTy::TyApp(b_func, b_args)) => {
                go(graph, *a_func, *b_func, seen)
                    && a_args.len() == b_args.len()
                    && a_args
                        .iter()
                        .zip(b_args)
                        .all(|(a, b)| go(graph, *a, *b, seen))
            }
            _ => false,
        }
    }

    go(graph, expected, actual, &mut FxHashSet::default())
}

/// Whether a cross-module global reference is type-compatible with the
/// dependency value it points to: structurally identical except that an
/// *abstract* leaf — a `TyVar` (polymorphic), `Opaque`, `Error`, or `Type` — on
/// **either** side matches any subterm on the other.
///
/// Two soundness arguments, both resting on the untagged-i64 ABI (D-0002), where
/// every value is one machine word and a parametric value is compiled once,
/// bit-identical across instantiations:
///
/// - **`def`-side abstraction** (the dependency is polymorphic): a generic global
///   is lowered with the dependency's free-`TyVar` type (e.g. `Fun(TyVar, TyVar)`)
///   while the use site has a concrete instantiation (`Fun(Int, Int)`).
/// - **`actual`-side abstraction** (the *use site* is opaque): an import whose type
///   cannot be reconstructed structurally through the finite `ImportedType`
///   boundary — notably the recursive codata `Stream`, abstracted to a fresh
///   `TyVar` at the recursion horizon — references a dependency whose real value is
///   fully structural. The use site never inspects that value's structure, so the
///   one-word value is layout-identical to the concrete definition it stands in for.
///
/// The non-abstract structure must still match exactly, which keeps genuine shape
/// mismatches (e.g. record-vs-tuple, differing field names) rejected.
pub(super) fn is_instantiation_of(graph: &DataflowGraph, def: DfTyId, actual: DfTyId) -> bool {
    fn go(
        graph: &DataflowGraph,
        def: DfTyId,
        actual: DfTyId,
        seen: &mut FxHashSet<(DfTyId, DfTyId)>,
    ) -> bool {
        if def == actual {
            return true;
        }
        if !type_exists(graph, def) || !type_exists(graph, actual) {
            return false;
        }
        if !seen.insert((def, actual)) {
            // Coinductive back-edge on a re-encountered pair. Unlike `same_type`'s
            // equality back-edge, this assumes the pair *instantiates*; it is
            // justified by the untagged-i64 ABI (a `TyVar` slot is layout-irrelevant,
            // so cycle structure under a substituted position cannot miscompile),
            // not by a general type-equality claim.
            return true;
        }
        match (&graph.types[def], &graph.types[actual]) {
            // A type variable in the definition matches any instantiation.
            (DfTy::TyVar(_), _) => true,
            // Symmetric: an abstract leaf on the use-site (`actual`) side matches
            // any definition shape it stands in for. `TyApp` is excluded — it has
            // its own structural arm below — so only genuine opaque leaves wildcard.
            (_, DfTy::TyVar(_) | DfTy::Opaque(_) | DfTy::Error | DfTy::Type) => true,
            (DfTy::List(a), DfTy::List(b))
            | (DfTy::Optional(a), DfTy::Optional(b))
            | (DfTy::Maybe(a), DfTy::Maybe(b)) => go(graph, *a, *b, seen),
            (DfTy::Fun(a_arg, a_res), DfTy::Fun(b_arg, b_res)) => {
                go(graph, *a_arg, *b_arg, seen) && go(graph, *a_res, *b_res, seen)
            }
            (DfTy::Record(a), DfTy::Record(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        a.name == b.name && a.optional == b.optional && go(graph, a.ty, b.ty, seen)
                    })
            }
            (DfTy::Union(a), DfTy::Union(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(a, b)| a.tag == b.tag && go(graph, a.ty, b.ty, seen))
            }
            (DfTy::Tuple(a), DfTy::Tuple(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| match (a, b) {
                        (
                            DfTupleField::Named { name: an, ty: at },
                            DfTupleField::Named { name: bn, ty: bt },
                        ) => an == bn && go(graph, *at, *bt, seen),
                        (DfTupleField::Positional(at), DfTupleField::Positional(bt)) => {
                            go(graph, *at, *bt, seen)
                        }
                        _ => false,
                    })
            }
            (DfTy::TyApp(a_func, a_args), DfTy::TyApp(b_func, b_args)) => {
                go(graph, *a_func, *b_func, seen)
                    && a_args.len() == b_args.len()
                    && a_args
                        .iter()
                        .zip(b_args)
                        .all(|(a, b)| go(graph, *a, *b, seen))
            }
            (DfTy::TyFun(a_params, a_body), DfTy::TyFun(b_params, b_body)) => {
                a_params == b_params && go(graph, *a_body, *b_body, seen)
            }
            // Leaves / mismatched constructors: require exact equality.
            _ => same_type(graph, def, actual),
        }
    }
    go(graph, def, actual, &mut FxHashSet::default())
}

pub(super) fn check_node_ref(
    graph: &DataflowGraph,
    owner: NodeId,
    field: &'static str,
    target: NodeId,
    errors: &mut Vec<ValidationError>,
) -> bool {
    if node_exists(graph, target) {
        true
    } else {
        errors.push(ValidationError::InvalidNodeRef {
            owner,
            field,
            target,
        });
        false
    }
}

pub(super) fn check_type_ref(
    graph: &DataflowGraph,
    owner: DfTyId,
    field: &'static str,
    target: DfTyId,
    errors: &mut Vec<ValidationError>,
) -> bool {
    if type_exists(graph, target) {
        true
    } else {
        errors.push(ValidationError::InvalidTypeRef {
            owner,
            field,
            target,
        });
        false
    }
}

/// Collect all `Bind` nodes introduced by a pattern.
pub(super) fn collect_bind_nodes(pat: &DfPattern, out: &mut Vec<NodeId>) {
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
            for (_, _, p) in fields {
                collect_bind_nodes(p, out);
            }
        }
        DfPattern::Variant { pattern, .. } => collect_bind_nodes(pattern, out),
        DfPattern::Wildcard | DfPattern::Lit(_) | DfPattern::Atom(_) => {}
    }
}

pub(super) fn collect_bind_owners(
    graph: &DataflowGraph,
    errors: &mut Vec<ValidationError>,
) -> FxHashMap<NodeId, BindOwner> {
    let mut counts: FxHashMap<NodeId, usize> = FxHashMap::default();
    let mut candidates: FxHashMap<NodeId, BindOwner> = FxHashMap::default();

    for (owner, node) in graph.nodes.iter() {
        match &node.kind {
            DfNodeKind::Lambda { param, .. } => {
                if node_exists(graph, *param) {
                    if matches!(&graph.nodes[*param].kind, DfNodeKind::Bind) {
                        *counts.entry(*param).or_default() += 1;
                        candidates.entry(*param).or_insert(BindOwner::Lambda(owner));
                    } else {
                        errors.push(ValidationError::UnexpectedNodeKind {
                            owner,
                            field: "param",
                            target: *param,
                            expected: "Bind",
                        });
                    }
                }
            }
            DfNodeKind::Match { arms, .. } => {
                for (arm_index, arm) in arms.iter().enumerate() {
                    let mut bind_nodes = Vec::new();
                    collect_bind_nodes(&arm.pattern, &mut bind_nodes);
                    for bind in bind_nodes {
                        if node_exists(graph, bind) {
                            if matches!(&graph.nodes[bind].kind, DfNodeKind::Bind) {
                                *counts.entry(bind).or_default() += 1;
                                candidates.entry(bind).or_insert(BindOwner::Arm {
                                    match_node: owner,
                                    arm_index,
                                });
                            } else {
                                errors.push(ValidationError::UnexpectedNodeKind {
                                    owner,
                                    field: "pattern.bind",
                                    target: bind,
                                    expected: "Bind",
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let mut owners = FxHashMap::default();
    for (node_id, node) in graph.nodes.iter() {
        if matches!(&node.kind, DfNodeKind::Bind) {
            let count = counts.get(&node_id).copied().unwrap_or(0);
            if count != 1 {
                errors.push(ValidationError::BindOwnershipViolation { count });
            } else if let Some(owner) = candidates.get(&node_id).copied() {
                owners.insert(node_id, owner);
            }
        }
    }

    owners
}

pub(super) fn check_same_type(
    graph: &DataflowGraph,
    owner: NodeId,
    field: &'static str,
    expected: DfTyId,
    actual: DfTyId,
    errors: &mut Vec<ValidationError>,
) {
    if type_exists(graph, expected)
        && type_exists(graph, actual)
        && !same_type(graph, expected, actual)
    {
        errors.push(ValidationError::TypeMismatch {
            owner,
            field,
            expected,
            actual,
        });
    }
}

pub(super) fn unexpected_type(
    owner: NodeId,
    field: &'static str,
    expected: &'static str,
    actual: DfTyId,
    errors: &mut Vec<ValidationError>,
) {
    errors.push(ValidationError::UnexpectedTypeKind {
        owner,
        field,
        expected,
        actual,
    });
}

pub(super) fn child_ty(graph: &DataflowGraph, target: NodeId) -> Option<DfTyId> {
    if node_exists(graph, target) {
        let ty = graph.nodes[target].ty;
        type_exists(graph, ty).then_some(ty)
    } else {
        None
    }
}

pub(super) fn is_numeric_type(graph: &DataflowGraph, ty: DfTyId) -> bool {
    type_exists(graph, ty) && matches!(graph.types[ty], DfTy::Int | DfTy::Float | DfTy::Posit(_))
}

pub(super) fn is_opaque_shape_type(graph: &DataflowGraph, ty: DfTyId) -> bool {
    type_exists(graph, ty)
        && matches!(
            graph.types[ty],
            DfTy::TyVar(_) | DfTy::TyApp(_, _) | DfTy::Opaque(_) | DfTy::Type | DfTy::Error
        )
}

pub(super) fn is_wrapper_type(graph: &DataflowGraph, ty: DfTyId) -> bool {
    type_exists(graph, ty) && matches!(graph.types[ty], DfTy::Optional(_) | DfTy::Maybe(_))
}

pub(super) fn is_bool_type(graph: &DataflowGraph, ty: DfTyId) -> bool {
    type_exists(graph, ty) && matches!(graph.types[ty], DfTy::Bool)
}

pub(super) fn expect_bool_type(
    graph: &DataflowGraph,
    owner: NodeId,
    field: &'static str,
    ty: DfTyId,
    errors: &mut Vec<ValidationError>,
) {
    if type_exists(graph, ty) && !is_bool_type(graph, ty) {
        unexpected_type(owner, field, "Bool", ty, errors);
    }
}

pub(super) fn validate_type_refs(graph: &DataflowGraph, errors: &mut Vec<ValidationError>) {
    for (ty_id, ty) in graph.types.iter() {
        match ty {
            DfTy::List(inner) => {
                check_type_ref(graph, ty_id, "element", *inner, errors);
            }
            DfTy::Optional(inner) | DfTy::Maybe(inner) => {
                check_type_ref(graph, ty_id, "inner", *inner, errors);
            }
            DfTy::Record(fields) => {
                for field in fields {
                    check_type_ref(graph, ty_id, "field", field.ty, errors);
                }
            }
            DfTy::Union(members) => {
                for member in members {
                    check_type_ref(graph, ty_id, "member", member.ty, errors);
                }
            }
            DfTy::Tuple(fields) => {
                for field in fields {
                    match field {
                        DfTupleField::Named { ty, .. } | DfTupleField::Positional(ty) => {
                            check_type_ref(graph, ty_id, "field", *ty, errors);
                        }
                    }
                }
            }
            DfTy::Fun(arg, result) => {
                check_type_ref(graph, ty_id, "arg", *arg, errors);
                check_type_ref(graph, ty_id, "result", *result, errors);
            }
            DfTy::TyFun(_, body) => {
                check_type_ref(graph, ty_id, "body", *body, errors);
            }
            DfTy::TyApp(func, args) => {
                check_type_ref(graph, ty_id, "function", *func, errors);
                for arg in args {
                    check_type_ref(graph, ty_id, "argument", *arg, errors);
                }
            }
            DfTy::Int
            | DfTy::Float
            | DfTy::Posit(_)
            | DfTy::Bool
            | DfTy::Text
            | DfTy::Atom
            | DfTy::Opaque(_)
            | DfTy::True
            | DfTy::False
            | DfTy::TyVar(_)
            | DfTy::Type
            | DfTy::Error => {}
        }
    }
}

pub(super) fn check_pattern_refs(
    graph: &DataflowGraph,
    owner: NodeId,
    pat: &DfPattern,
    errors: &mut Vec<ValidationError>,
) {
    match pat {
        DfPattern::Bind(target) => {
            check_node_ref(graph, owner, "pattern.bind", *target, errors);
        }
        DfPattern::Tuple(items) => {
            for item in items {
                match item {
                    DfTuplePatItem::Named { pattern, .. } => {
                        check_pattern_refs(graph, owner, pattern, errors);
                    }
                    DfTuplePatItem::Positional(pattern) => {
                        check_pattern_refs(graph, owner, pattern, errors);
                    }
                }
            }
        }
        DfPattern::Record(fields) => {
            for (_, _, pattern) in fields {
                check_pattern_refs(graph, owner, pattern, errors);
            }
        }
        DfPattern::Variant { pattern, .. } => check_pattern_refs(graph, owner, pattern, errors),
        DfPattern::Wildcard | DfPattern::Lit(_) | DfPattern::Atom(_) => {}
    }
}

pub(super) fn check_node_refs(
    graph: &DataflowGraph,
    owner: NodeId,
    errors: &mut Vec<ValidationError>,
) {
    let node = &graph.nodes[owner];
    match &node.kind {
        DfNodeKind::Lambda { param, body } => {
            check_node_ref(graph, owner, "param", *param, errors);
            check_node_ref(graph, owner, "body", *body, errors);
        }
        DfNodeKind::Apply { func, arg } => {
            check_node_ref(graph, owner, "func", *func, errors);
            check_node_ref(graph, owner, "arg", *arg, errors);
        }
        DfNodeKind::HostPrint { arg } => {
            check_node_ref(graph, owner, "arg", *arg, errors);
        }
        DfNodeKind::HostOp { arg, .. } => {
            check_node_ref(graph, owner, "arg", *arg, errors);
        }
        DfNodeKind::TyLam { body, .. } => {
            check_node_ref(graph, owner, "body", *body, errors);
        }
        DfNodeKind::TyApp { poly, ty_args } => {
            check_node_ref(graph, owner, "poly", *poly, errors);
            if type_exists(graph, node.ty) {
                for ty_arg in ty_args {
                    check_type_ref(graph, node.ty, "ty_arg", *ty_arg, errors);
                }
            }
        }
        DfNodeKind::Record(fields) => {
            for (_, value) in fields {
                check_node_ref(graph, owner, "field", *value, errors);
            }
        }
        DfNodeKind::RecordUpdate { base, updates } => {
            check_node_ref(graph, owner, "base", *base, errors);
            for (_, _, value) in updates {
                check_node_ref(graph, owner, "update", *value, errors);
            }
        }
        DfNodeKind::Tuple(items) => {
            for item in items {
                match item {
                    DfTupleNodeItem::Named { value, .. } | DfTupleNodeItem::Positional(value) => {
                        check_node_ref(graph, owner, "item", *value, errors);
                    }
                }
            }
        }
        DfNodeKind::List(items) => {
            for item in items {
                check_node_ref(graph, owner, "element", *item, errors);
            }
        }
        DfNodeKind::Variant { value, .. } => {
            check_node_ref(graph, owner, "payload", *value, errors);
        }
        DfNodeKind::Select { base, .. } => {
            check_node_ref(graph, owner, "base", *base, errors);
        }
        DfNodeKind::Match { scrutinee, arms } => {
            check_node_ref(graph, owner, "scrutinee", *scrutinee, errors);
            for arm in arms {
                check_pattern_refs(graph, owner, &arm.pattern, errors);
                if let Some(guard) = arm.guard {
                    check_node_ref(graph, owner, "guard", guard, errors);
                }
                check_node_ref(graph, owner, "body", arm.body, errors);
            }
        }
        DfNodeKind::Coalesce { value, fallback } => {
            check_node_ref(graph, owner, "value", *value, errors);
            check_node_ref(graph, owner, "fallback", *fallback, errors);
        }
        DfNodeKind::Builtin(_, lhs, rhs) => {
            check_node_ref(graph, owner, "lhs", *lhs, errors);
            check_node_ref(graph, owner, "rhs", *rhs, errors);
        }
        DfNodeKind::Sequence(items) => {
            for item in items {
                check_node_ref(graph, owner, "item", *item, errors);
            }
        }
        DfNodeKind::Lit(_)
        | DfNodeKind::Bind
        | DfNodeKind::GlobalRef(_)
        | DfNodeKind::Import { .. }
        | DfNodeKind::Error => {}
    }
}
