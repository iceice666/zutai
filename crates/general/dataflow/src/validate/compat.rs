use crate::*;

use super::refs::*;

pub(super) fn check_record_literal(
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

pub(super) fn check_record_update(
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

pub(super) fn check_tuple(
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

pub(super) fn check_select(
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

pub(super) fn check_builtin(
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

pub(super) fn check_node_type_compat(
    graph: &DataflowGraph,
    owner: NodeId,
    errors: &mut Vec<ValidationError>,
) {
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
        DfNodeKind::Variant { .. } => {}
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
        DfNodeKind::HostPrint { arg } => {
            if let Some(arg_ty) = child_ty(graph, *arg) {
                match &graph.types[arg_ty] {
                    DfTy::Text => check_same_type(graph, owner, "type", arg_ty, node.ty, errors),
                    _ => unexpected_type(owner, "arg", "Text", arg_ty, errors),
                }
            }
        }
        DfNodeKind::Builtin(op, lhs, rhs) => check_builtin(graph, owner, *op, *lhs, *rhs, errors),
        DfNodeKind::Sequence(items) => {
            if let Some(last) = items.last().and_then(|item| child_ty(graph, *item)) {
                check_same_type(graph, owner, "last", last, node.ty, errors);
            }
        }
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
