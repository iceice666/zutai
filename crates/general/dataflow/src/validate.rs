use std::collections::{HashMap, HashSet};

use crate::{
    DataflowGraph, DfBuiltinOp, DfNodeKind, DfPattern, DfTupleField, DfTupleNodeItem,
    DfTuplePatItem, DfTy, DfTyId, NodeId, ValidationError,
};

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

fn node_exists(graph: &DataflowGraph, id: NodeId) -> bool {
    (id.into_raw().into_u32() as usize) < graph.nodes.len()
}

fn type_exists(graph: &DataflowGraph, id: DfTyId) -> bool {
    (id.into_raw().into_u32() as usize) < graph.types.len()
}

fn same_type(graph: &DataflowGraph, expected: DfTyId, actual: DfTyId) -> bool {
    fn go(
        graph: &DataflowGraph,
        expected: DfTyId,
        actual: DfTyId,
        seen: &mut HashSet<(DfTyId, DfTyId)>,
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
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| go(graph, *a, *b, seen))
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

    go(graph, expected, actual, &mut HashSet::new())
}

fn check_node_ref(
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

fn check_type_ref(
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
            for (_, _, p) in fields {
                collect_bind_nodes(p, out);
            }
        }
        DfPattern::Variant(_, inner) => collect_bind_nodes(inner, out),
        DfPattern::Wildcard | DfPattern::Lit(_) | DfPattern::Atom(_) => {}
    }
}

fn collect_bind_owners(
    graph: &DataflowGraph,
    errors: &mut Vec<ValidationError>,
) -> HashMap<NodeId, BindOwner> {
    let mut counts: HashMap<NodeId, usize> = HashMap::new();
    let mut candidates: HashMap<NodeId, BindOwner> = HashMap::new();

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

    let mut owners = HashMap::new();
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

fn check_same_type(
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

fn unexpected_type(
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

fn child_ty(graph: &DataflowGraph, target: NodeId) -> Option<DfTyId> {
    if node_exists(graph, target) {
        let ty = graph.nodes[target].ty;
        type_exists(graph, ty).then_some(ty)
    } else {
        None
    }
}

fn is_numeric_type(graph: &DataflowGraph, ty: DfTyId) -> bool {
    type_exists(graph, ty) && matches!(graph.types[ty], DfTy::Int | DfTy::Float | DfTy::Posit(_))
}

fn is_opaque_shape_type(graph: &DataflowGraph, ty: DfTyId) -> bool {
    type_exists(graph, ty)
        && matches!(
            graph.types[ty],
            DfTy::TyVar(_) | DfTy::TyApp(_, _) | DfTy::Type | DfTy::Error
        )
}

fn is_wrapper_type(graph: &DataflowGraph, ty: DfTyId) -> bool {
    type_exists(graph, ty) && matches!(graph.types[ty], DfTy::Optional(_) | DfTy::Maybe(_))
}

fn is_bool_type(graph: &DataflowGraph, ty: DfTyId) -> bool {
    type_exists(graph, ty) && matches!(graph.types[ty], DfTy::Bool)
}

fn expect_bool_type(
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

fn validate_type_refs(graph: &DataflowGraph, errors: &mut Vec<ValidationError>) {
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
                    check_type_ref(graph, ty_id, "member", *member, errors);
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
            | DfTy::True
            | DfTy::False
            | DfTy::TyVar(_)
            | DfTy::Type
            | DfTy::Error => {}
        }
    }
}

fn check_pattern_refs(
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
        DfPattern::Variant(_, inner) => check_pattern_refs(graph, owner, inner, errors),
        DfPattern::Wildcard | DfPattern::Lit(_) | DfPattern::Atom(_) => {}
    }
}

fn check_node_refs(graph: &DataflowGraph, owner: NodeId, errors: &mut Vec<ValidationError>) {
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
        DfNodeKind::Variant(_, payload) => {
            check_node_ref(graph, owner, "payload", *payload, errors);
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
        DfNodeKind::Lit(_)
        | DfNodeKind::Bind
        | DfNodeKind::GlobalRef(_)
        | DfNodeKind::Import { .. }
        | DfNodeKind::Error => {}
    }
}

fn check_record_literal(
    graph: &DataflowGraph,
    owner: NodeId,
    fields: &[(String, NodeId)],
    errors: &mut Vec<ValidationError>,
) {
    let DfTy::Record(type_fields) = &graph.types[graph.nodes[owner].ty] else {
        if is_opaque_shape_type(graph, graph.nodes[owner].ty) {
            return;
        }
        unexpected_type(owner, "type", "Record", graph.nodes[owner].ty, errors);
        return;
    };

    for type_field in type_fields {
        if !type_field.optional && !fields.iter().any(|(name, _)| name == &type_field.name) {
            errors.push(ValidationError::MissingRequiredField {
                owner,
                field: type_field.name.clone(),
            });
        }
    }

    for (name, value) in fields {
        if let Some(type_field) = type_fields.iter().find(|field| field.name == *name)
            && let Some(value_ty) = child_ty(graph, *value)
        {
            check_same_type(graph, owner, "field", type_field.ty, value_ty, errors);
        }
    }
}

fn check_record_update(
    graph: &DataflowGraph,
    owner: NodeId,
    base: NodeId,
    updates: &[(String, usize, NodeId)],
    errors: &mut Vec<ValidationError>,
) {
    if let Some(base_ty) = child_ty(graph, base)
        && !matches!(&graph.types[base_ty], DfTy::Record(_))
        && !is_opaque_shape_type(graph, base_ty)
    {
        unexpected_type(owner, "base", "Record", base_ty, errors);
    }

    let DfTy::Record(result_fields) = &graph.types[graph.nodes[owner].ty] else {
        if is_opaque_shape_type(graph, graph.nodes[owner].ty) {
            return;
        }
        unexpected_type(owner, "type", "Record", graph.nodes[owner].ty, errors);
        return;
    };

    for (name, _, value) in updates {
        if let Some(type_field) = result_fields.iter().find(|field| field.name == *name) {
            if let Some(value_ty) = child_ty(graph, *value) {
                check_same_type(graph, owner, "update", type_field.ty, value_ty, errors);
            }
        } else {
            errors.push(ValidationError::MissingRequiredField {
                owner,
                field: name.clone(),
            });
        }
    }
}

fn check_tuple(
    graph: &DataflowGraph,
    owner: NodeId,
    items: &[DfTupleNodeItem],
    errors: &mut Vec<ValidationError>,
) {
    let DfTy::Tuple(type_fields) = &graph.types[graph.nodes[owner].ty] else {
        if is_opaque_shape_type(graph, graph.nodes[owner].ty) {
            return;
        }
        unexpected_type(owner, "type", "Tuple", graph.nodes[owner].ty, errors);
        return;
    };

    if items.len() != type_fields.len() {
        unexpected_type(
            owner,
            "type",
            "Tuple with matching arity",
            graph.nodes[owner].ty,
            errors,
        );
        return;
    }

    for (item, type_field) in items.iter().zip(type_fields) {
        match (item, type_field) {
            (
                DfTupleNodeItem::Named { name, value },
                DfTupleField::Named {
                    name: type_name,
                    ty,
                },
            ) if name == type_name => {
                if let Some(value_ty) = child_ty(graph, *value) {
                    check_same_type(graph, owner, "item", *ty, value_ty, errors);
                }
            }
            (DfTupleNodeItem::Positional(value), DfTupleField::Positional(ty)) => {
                if let Some(value_ty) = child_ty(graph, *value) {
                    check_same_type(graph, owner, "item", *ty, value_ty, errors);
                }
            }
            _ => {
                unexpected_type(
                    owner,
                    "type",
                    "Tuple with matching field names",
                    graph.nodes[owner].ty,
                    errors,
                );
                return;
            }
        }
    }
}

fn check_select(
    graph: &DataflowGraph,
    owner: NodeId,
    base: NodeId,
    selected: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(base_ty) = child_ty(graph, base) else {
        return;
    };

    let DfTy::Record(type_fields) = &graph.types[base_ty] else {
        if is_opaque_shape_type(graph, base_ty) {
            return;
        }
        unexpected_type(owner, "base", "Record", base_ty, errors);
        return;
    };

    let Some(field) = type_fields.iter().find(|field| field.name == selected) else {
        // Witness dictionaries and some type aliases keep enough type information
        // for THIR, but DC no longer has the source field label here.
        return;
    };

    if field.optional {
        let DfTy::Maybe(inner) = &graph.types[graph.nodes[owner].ty] else {
            unexpected_type(owner, "type", "Maybe", graph.nodes[owner].ty, errors);
            return;
        };
        check_same_type(graph, owner, "field", field.ty, *inner, errors);
    } else {
        check_same_type(
            graph,
            owner,
            "field",
            field.ty,
            graph.nodes[owner].ty,
            errors,
        );
    }
}

fn check_builtin(
    graph: &DataflowGraph,
    owner: NodeId,
    op: DfBuiltinOp,
    lhs: NodeId,
    rhs: NodeId,
    errors: &mut Vec<ValidationError>,
) {
    let lhs_ty = child_ty(graph, lhs);
    let rhs_ty = child_ty(graph, rhs);
    let result_ty = graph.nodes[owner].ty;

    match op {
        DfBuiltinOp::Add | DfBuiltinOp::Sub | DfBuiltinOp::Mul | DfBuiltinOp::Div => {
            let expected = lhs_ty.or(rhs_ty);
            if let Some(expected) = expected {
                // Generic operator witnesses can leave arithmetic operand shape
                // opaque in DC (`add a b = a + b`). Only concrete primitive numeric
                // operands have enough information for a conservative type check.
                if !is_numeric_type(graph, expected) {
                    return;
                }
                if let Some(lhs_ty) = lhs_ty {
                    check_same_type(graph, owner, "lhs", expected, lhs_ty, errors);
                }
                if let Some(rhs_ty) = rhs_ty {
                    check_same_type(graph, owner, "rhs", expected, rhs_ty, errors);
                }
                check_same_type(graph, owner, "type", expected, result_ty, errors);
            }
        }
        DfBuiltinOp::Eq
        | DfBuiltinOp::Ne
        | DfBuiltinOp::Lt
        | DfBuiltinOp::Le
        | DfBuiltinOp::Gt
        | DfBuiltinOp::Ge => {
            if let (Some(lhs_ty), Some(rhs_ty)) = (lhs_ty, rhs_ty) {
                check_same_type(graph, owner, "rhs", lhs_ty, rhs_ty, errors);
            }
            expect_bool_type(graph, owner, "type", result_ty, errors);
        }
        DfBuiltinOp::Posit { op, spec } => {
            if let Some(lhs_ty) = lhs_ty
                && !matches!(graph.types[lhs_ty], DfTy::Posit(actual) if actual == spec)
            {
                unexpected_type(owner, "lhs", "matching Posit", lhs_ty, errors);
            }
            if let Some(rhs_ty) = rhs_ty
                && !matches!(graph.types[rhs_ty], DfTy::Posit(actual) if actual == spec)
            {
                unexpected_type(owner, "rhs", "matching Posit", rhs_ty, errors);
            }
            if matches!(
                op,
                crate::DfPositOp::Eq
                    | crate::DfPositOp::Ne
                    | crate::DfPositOp::Lt
                    | crate::DfPositOp::Le
                    | crate::DfPositOp::Gt
                    | crate::DfPositOp::Ge
            ) {
                expect_bool_type(graph, owner, "type", result_ty, errors);
            } else if !matches!(graph.types[result_ty], DfTy::Posit(actual) if actual == spec) {
                unexpected_type(owner, "type", "matching Posit", result_ty, errors);
            }
        }
        DfBuiltinOp::And | DfBuiltinOp::Or => {
            if let Some(lhs_ty) = lhs_ty {
                expect_bool_type(graph, owner, "lhs", lhs_ty, errors);
            }
            if let Some(rhs_ty) = rhs_ty {
                expect_bool_type(graph, owner, "rhs", rhs_ty, errors);
            }
            expect_bool_type(graph, owner, "type", result_ty, errors);
        }
    }
}

fn check_node_type_compat(graph: &DataflowGraph, owner: NodeId, errors: &mut Vec<ValidationError>) {
    let node = &graph.nodes[owner];
    if !type_exists(graph, node.ty) {
        return;
    }

    match &node.kind {
        DfNodeKind::Lambda { param, .. } => match &graph.types[node.ty] {
            DfTy::Fun(param_ty, _) => {
                if let Some(actual_param_ty) = child_ty(graph, *param) {
                    check_same_type(graph, owner, "param", *param_ty, actual_param_ty, errors);
                }
                // Current clause and witness lowering can assign lambda bodies a
                // surrounding function or witness type rather than the immediate
                // result type (`id x = x`, `Eq @Int`). Child existence and lexical
                // bind scope remain checked by the traversal; body type equality
                // waits for TLC/DC result-typed clause bodies.
            }
            _ => unexpected_type(owner, "type", "Fun", node.ty, errors),
        },
        DfNodeKind::Apply { .. } => {
            // Witness and derived-dictionary lowering can represent callable values
            // with type shapes that are not a plain `DfTy::Fun` at the Apply node.
            // Keep Apply edges existence-checked until TLC/DC witness application
            // types are normalized.
        }
        DfNodeKind::TyLam { ty_params, body } => match &graph.types[node.ty] {
            DfTy::TyFun(params, body_ty) if params == ty_params => {
                if let Some(actual_body_ty) = child_ty(graph, *body) {
                    check_same_type(graph, owner, "body", *body_ty, actual_body_ty, errors);
                }
            }
            DfTy::TyFun(_, _) => unexpected_type(
                owner,
                "type",
                "TyFun with matching parameters",
                node.ty,
                errors,
            ),
            _ => unexpected_type(owner, "type", "TyFun", node.ty, errors),
        },
        DfNodeKind::TyApp { .. } => {}
        DfNodeKind::Record(fields) => check_record_literal(graph, owner, fields, errors),
        DfNodeKind::RecordUpdate { base, updates } => {
            check_record_update(graph, owner, *base, updates, errors);
        }
        DfNodeKind::Tuple(items) => check_tuple(graph, owner, items, errors),
        DfNodeKind::List(items) => match &graph.types[node.ty] {
            DfTy::List(elem_ty) => {
                for item in items {
                    if let Some(item_ty) = child_ty(graph, *item) {
                        check_same_type(graph, owner, "element", *elem_ty, item_ty, errors);
                    }
                }
            }
            _ => unexpected_type(owner, "type", "List", node.ty, errors),
        },
        DfNodeKind::Variant(_, _) => {}
        DfNodeKind::Select { base, field, .. } => check_select(graph, owner, *base, field, errors),
        DfNodeKind::Match { arms, .. } => {
            let clause_match_has_function_type = matches!(&graph.types[node.ty], DfTy::Fun(_, _));
            for arm in arms {
                if let Some(guard) = arm.guard
                    && let Some(guard_ty) = child_ty(graph, guard)
                {
                    expect_bool_type(graph, owner, "guard", guard_ty, errors);
                }
                // Current function-clause lowering (`id x = x`) gives the synthetic
                // Match the surrounding function type while its arms produce result
                // values. Existence and scope are still checked; body type equality
                // waits for that IR shape to be normalized.
                if !clause_match_has_function_type && let Some(body_ty) = child_ty(graph, arm.body)
                {
                    check_same_type(graph, owner, "body", node.ty, body_ty, errors);
                }
            }
        }
        DfNodeKind::Coalesce { value, fallback } => {
            if let Some(value_ty) = child_ty(graph, *value) {
                match &graph.types[value_ty] {
                    DfTy::Optional(inner) | DfTy::Maybe(inner) => {
                        if let Some(fallback_ty) = child_ty(graph, *fallback) {
                            check_same_type(graph, owner, "fallback", *inner, fallback_ty, errors);
                        }
                        check_same_type(graph, owner, "type", *inner, node.ty, errors);
                    }
                    _ => unexpected_type(owner, "value", "Optional or Maybe", value_ty, errors),
                }
            }
        }
        DfNodeKind::Builtin(op, lhs, rhs) => check_builtin(graph, owner, *op, *lhs, *rhs, errors),
        DfNodeKind::GlobalRef(name) => {
            if let Some(&target) = graph.globals.get(name.as_str())
                && check_node_ref(graph, owner, "global", target, errors)
                && let Some(target_ty) = child_ty(graph, target)
            {
                // A polymorphic global reference is stored at its instantiated
                // use-site type before TyApp; `id x = x; id 42` is the current
                // lowering shape. `#none` also lowers through an Error sentinel
                // under an Optional annotation. Stray refs and target existence
                // remain checked.
                if !matches!(&graph.types[target_ty], DfTy::TyFun(_, _))
                    && !is_opaque_shape_type(graph, target_ty)
                    && !is_wrapper_type(graph, target_ty)
                    && !is_wrapper_type(graph, node.ty)
                {
                    check_same_type(graph, owner, "global", target_ty, node.ty, errors);
                }
            }
        }
        DfNodeKind::Lit(_) | DfNodeKind::Bind | DfNodeKind::Import { .. } | DfNodeKind::Error => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_child(
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

fn walk_node(
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
        DfNodeKind::Variant(_, payload) => {
            walk_child(
                graph, id, "payload", *payload, scope, owners, visited, errors,
            );
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
        DfNodeKind::Lit(_)
        | DfNodeKind::Bind
        | DfNodeKind::GlobalRef(_)
        | DfNodeKind::Import { .. }
        | DfNodeKind::Error => {}
    }
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
