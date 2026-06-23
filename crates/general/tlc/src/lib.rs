use rustc_hash::FxHashSet;

mod erase;
mod ir;
mod lower;
mod normalize;

#[cfg(test)]
mod tests;

pub use ir::{
    BuiltinOp, HostEffectSet, HostOp, Kind, Literal, PrimTy, Row, TlcAlt, TlcDecl, TlcDeclId,
    TlcExpr, TlcExprId, TlcHandleClause, TlcModule, TlcPat, TlcPatItem, TlcTupleField,
    TlcTupleItem, TlcType, TlcTypeId, TlcTypeVar,
};
pub use lower::{lower_thir, lower_thir_with_extern_witnesses};
pub use normalize::{DEFAULT_FUEL, NormalizeError};

/// Return why a TLC module still cannot enter Dataflow Core.
///
/// The implemented Phase 23 lowering removes general source effect control
/// before TLC enters Dataflow Core. Residual unsupported effect nodes or open /
/// unsupported effect rows must not be silently erased before compilation.
pub fn residual_effect_reason(module: &TlcModule) -> Option<&'static str> {
    residual_effect_reason_with_grants(module, HostEffectSet::AMBIENT)
}

/// Return why a TLC module cannot enter Dataflow Core under a host grant set.
///
/// `io.print` is always ambient for source compatibility. v2 host capability
/// entry points extend this set with explicitly requested standard operations.
pub fn residual_effect_reason_with_grants(
    module: &TlcModule,
    grants: HostEffectSet,
) -> Option<&'static str> {
    let final_has_residual_effect = module.final_expr.is_some_and(|expr| {
        let mut visited = FxHashSet::default();
        reachable_expr_has_effect(module, expr, &mut visited, grants)
    });
    let decl_has_residual_effect = module.decls.iter().any(|&decl_id| {
        let TlcDecl::Value { body, .. } = module.decl_arena[decl_id] else {
            return false;
        };
        let mut visited = FxHashSet::default();
        reachable_expr_has_effect(module, body, &mut visited, grants)
    });
    let has_residual_effect = final_has_residual_effect || decl_has_residual_effect;

    if has_residual_effect {
        return Some(
            "algebraic effects remain after TLC lowering; compile/dataflow effect lowering is not implemented yet or the host capability was not granted",
        );
    }

    if module.type_arena.iter().any(|(_, ty)| {
        matches!(
            ty,
            TlcType::Fun(_, _, row) if row_has_unsupported_effect(row, grants)
        )
    }) {
        return Some(
            "unsupported effectful function types remain after TLC lowering; compile/dataflow effect lowering is not implemented yet or the host capability was not granted",
        );
    }

    None
}

fn row_has_unsupported_effect(row: &Row, grants: HostEffectSet) -> bool {
    match row {
        Row::REmpty => false,
        Row::RExtend { label, tail, .. } => {
            HostOp::from_name(label).is_none_or(|op| !grants.contains(op))
                || row_has_unsupported_effect(tail, grants)
        }
        Row::RVar(_) => true,
    }
}

/// Return whether reachable TLC still contains ambient host `io.print`.
///
/// The final expression may lower to the runtime HostPrint path, but reflection
/// AOT folding still evaluates the whole program at compile time. Callers that
/// fold reflection must reject this until reflection is moved behind runtime
/// effect lowering.
pub fn contains_host_io_print(module: &TlcModule) -> bool {
    module
        .final_expr
        .is_some_and(|expr| reachable_host_io_print(module, expr, &mut FxHashSet::default()))
        || module.decls.iter().any(|&decl_id| {
            let TlcDecl::Value { body, .. } = module.decl_arena[decl_id] else {
                return false;
            };
            reachable_host_io_print(module, body, &mut FxHashSet::default())
        })
}

fn reachable_expr_has_effect(
    module: &TlcModule,
    id: TlcExprId,
    visited: &mut FxHashSet<TlcExprId>,
    grants: HostEffectSet,
) -> bool {
    if !visited.insert(id) {
        return false;
    }
    match &module.expr_arena[id] {
        TlcExpr::Perform { op, arg } => {
            if HostOp::from_name(op).is_some_and(|host_op| grants.contains(host_op)) {
                reachable_expr_has_effect(module, *arg, visited, grants)
            } else {
                true
            }
        }
        TlcExpr::Handle { .. } | TlcExpr::Resume { .. } => true,
        TlcExpr::Sequence(items) => items
            .iter()
            .any(|item| reachable_expr_has_effect(module, *item, visited, grants)),
        TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => false,
        TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) | TlcExpr::TyApp(body, _) => {
            reachable_expr_has_effect(module, *body, visited, grants)
        }
        TlcExpr::App(func, arg) | TlcExpr::Builtin(_, func, arg) => {
            reachable_expr_has_effect(module, *func, visited, grants)
                || reachable_expr_has_effect(module, *arg, visited, grants)
        }
        TlcExpr::RecordUpdate { receiver, fields } => {
            reachable_expr_has_effect(module, *receiver, visited, grants)
                || fields
                    .iter()
                    .any(|(_, value)| reachable_expr_has_effect(module, *value, visited, grants))
        }
        TlcExpr::Let { value, body, .. } => {
            reachable_expr_has_effect(module, *value, visited, grants)
                || reachable_expr_has_effect(module, *body, visited, grants)
        }
        TlcExpr::Letrec { bindings, body } => {
            bindings
                .iter()
                .any(|(_, _, value)| reachable_expr_has_effect(module, *value, visited, grants))
                || reachable_expr_has_effect(module, *body, visited, grants)
        }
        TlcExpr::Case(scrutinee, alts) => {
            reachable_expr_has_effect(module, *scrutinee, visited, grants)
                || alts.iter().any(|alt| {
                    alt.guard.is_some_and(|guard| {
                        reachable_expr_has_effect(module, guard, visited, grants)
                    }) || reachable_expr_has_effect(module, alt.body, visited, grants)
                })
        }
        TlcExpr::Record(fields) => fields
            .iter()
            .any(|(_, value)| reachable_expr_has_effect(module, *value, visited, grants)),
        TlcExpr::GetField(expr, _) | TlcExpr::Variant(_, expr) => {
            reachable_expr_has_effect(module, *expr, visited, grants)
        }
        TlcExpr::Tuple(items) => items.iter().any(|item| match item {
            TlcTupleItem::Named { value, .. } | TlcTupleItem::Positional(value) => {
                reachable_expr_has_effect(module, *value, visited, grants)
            }
        }),
        TlcExpr::List(items) => items
            .iter()
            .any(|item| reachable_expr_has_effect(module, *item, visited, grants)),
    }
}

fn reachable_host_io_print(
    module: &TlcModule,
    id: TlcExprId,
    visited: &mut FxHashSet<TlcExprId>,
) -> bool {
    if !visited.insert(id) {
        return false;
    }
    match &module.expr_arena[id] {
        TlcExpr::Perform { op, arg } => {
            op == "io.print" || reachable_host_io_print(module, *arg, visited)
        }
        TlcExpr::Handle { expr, value, ops } => {
            reachable_host_io_print(module, *expr, visited)
                || value.is_some_and(|value| reachable_host_io_print(module, value, visited))
                || ops
                    .iter()
                    .any(|clause| reachable_host_io_print(module, clause.body, visited))
        }
        TlcExpr::Resume { value } => reachable_host_io_print(module, *value, visited),
        TlcExpr::Sequence(items) | TlcExpr::List(items) => items
            .iter()
            .any(|item| reachable_host_io_print(module, *item, visited)),
        TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => false,
        TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) | TlcExpr::TyApp(body, _) => {
            reachable_host_io_print(module, *body, visited)
        }
        TlcExpr::App(func, arg) | TlcExpr::Builtin(_, func, arg) => {
            reachable_host_io_print(module, *func, visited)
                || reachable_host_io_print(module, *arg, visited)
        }
        TlcExpr::RecordUpdate { receiver, fields } => {
            reachable_host_io_print(module, *receiver, visited)
                || fields
                    .iter()
                    .any(|(_, value)| reachable_host_io_print(module, *value, visited))
        }
        TlcExpr::Let { value, body, .. } => {
            reachable_host_io_print(module, *value, visited)
                || reachable_host_io_print(module, *body, visited)
        }
        TlcExpr::Letrec { bindings, body } => {
            bindings
                .iter()
                .any(|(_, _, value)| reachable_host_io_print(module, *value, visited))
                || reachable_host_io_print(module, *body, visited)
        }
        TlcExpr::Case(scrutinee, alts) => {
            reachable_host_io_print(module, *scrutinee, visited)
                || alts.iter().any(|alt| {
                    alt.guard
                        .is_some_and(|guard| reachable_host_io_print(module, guard, visited))
                        || reachable_host_io_print(module, alt.body, visited)
                })
        }
        TlcExpr::Record(fields) => fields
            .iter()
            .any(|(_, value)| reachable_host_io_print(module, *value, visited)),
        TlcExpr::GetField(expr, _) | TlcExpr::Variant(_, expr) => {
            reachable_host_io_print(module, *expr, visited)
        }
        TlcExpr::Tuple(items) => items.iter().any(|item| match item {
            TlcTupleItem::Named { value, .. } | TlcTupleItem::Positional(value) => {
                reachable_host_io_print(module, *value, visited)
            }
        }),
    }
}
