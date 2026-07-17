use rustc_hash::FxHashSet;

mod entry;
mod erase;
mod ir;
mod lower;
mod monomorphize;
mod normalize;

#[cfg(test)]
mod tests;

pub use ir::{
    BuiltinOp, HostEffectSet, HostOp, Kind, Literal, PrimTy, Row, TlcAlt, TlcDecl, TlcDeclId,
    TlcExpr, TlcExprId, TlcHandleClause, TlcModule, TlcPat, TlcPatItem, TlcTupleField,
    TlcTupleItem, TlcType, TlcTypeId, TlcTypeVar,
};
pub use lower::{
    ExternConditionalWitness, lower_thir, lower_thir_for_backend, lower_thir_with_extern_witnesses,
    lower_thir_with_extern_witnesses_for_backend,
};
#[doc(hidden)]
pub use monomorphize::push_child_exprs;
pub use monomorphize::{monomorphize_open_row_selects, reachable_exprs};
pub use normalize::{DEFAULT_FUEL, NormalizeError};

/// Prepare a TLC module's algebraic effects for the native backend.
///
/// `lower_thir` deliberately leaves handled effects the lexical CPS path could
/// not discharge as residual `Handle`/`Perform`/`Resume` nodes — the reference
/// interpreter (`zutai-eval`) is the semantics oracle and evaluates those with
/// its own `handle_control`/`run_finally`, so the shared lowering must not
/// rewrite them. The *compile* path calls this afterwards to lower those
/// residuals for Dataflow Core: it desugars `finally` to outer-row sequencing,
/// then reifies recursive/higher-order/etc. handled effects into a free-monad
/// `Computation` driver, then erases now-cleared effect rows. A genuinely
/// unhandled effect is left residual and refused by the gate.
pub fn lower_effects_for_backend(module: &mut TlcModule) {
    // Desugar `finally` first, then re-run the lexical effect passes so the
    // freshly-introduced inner handles are inlined/elaborated like any other,
    // then reify whatever residual handled effects remain, then erase cleared
    // rows. The backend inliner preserves deferred generator-cell performs for
    // the reifier instead of using the interpreter-oriented eager inline path.
    module.desugar_finally();
    module.inline_effectful_calls_preserving_deferred_performs();
    module.elaborate_effects_preserving_deferred_performs();
    module.reify_residual_effects();
    if residual_effect_reason(module).is_none() {
        module.erase_effects();
    }
}

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
/// `io.print` is always ambient for source compatibility. Host-capability
/// entry points extend this set with explicitly requested standard operations.
pub fn residual_effect_reason_with_grants(
    module: &TlcModule,
    grants: HostEffectSet,
) -> Option<&'static str> {
    // A `finally` teardown clause is interpreter-only (resource finalization for
    // effectful generators, V3-G4): native lowering of effect handlers carrying
    // one is refused before Dataflow Core, precisely rather than via the generic
    // residual-effect message. Checked over reachable code only.
    if crate::monomorphize::reachable_exprs(module)
        .iter()
        .any(|id| {
            matches!(
                module.expr_arena[*id],
                TlcExpr::Handle {
                    finally: Some(_),
                    ..
                }
            )
        })
    {
        return Some(
            "a `finally` handler clause is interpreter-only; native compilation of resource-finalization handlers is not supported",
        );
    }

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

    // Clause 2 is scoped to types actually reachable from live code. Effect
    // inlining can leave an inlined-away callee's effectful function type orphaned
    // in the arena (la_arena never removes nodes); such dead types must not reject
    // a program whose live code is effect-free, matching the interpreter.
    let final_has_unsupported_type = module.final_expr.is_some_and(|expr| {
        let mut visited = FxHashSet::default();
        reachable_expr_type_has_unsupported_effect(module, expr, &mut visited, grants, true)
    });
    let decl_has_unsupported_type = module.decls.iter().any(|&decl_id| {
        let TlcDecl::Value { body, .. } = module.decl_arena[decl_id] else {
            return false;
        };
        let mut visited = FxHashSet::default();
        reachable_expr_type_has_unsupported_effect(module, body, &mut visited, grants, true)
    });
    if final_has_unsupported_type || decl_has_unsupported_type {
        return Some(
            "unsupported effectful function types remain after TLC lowering; compile/dataflow effect lowering is not implemented yet or the host capability was not granted",
        );
    }

    None
}

fn reachable_expr_type_has_unsupported_effect(
    module: &TlcModule,
    id: TlcExprId,
    visited: &mut FxHashSet<(TlcExprId, bool)>,
    grants: HostEffectSet,
    check_current_type: bool,
) -> bool {
    if !visited.insert((id, check_current_type)) {
        return false;
    }
    if check_current_type
        && !matches!(module.expr_arena[id], TlcExpr::Import(_))
        && module.expr_types.get(&id).is_some_and(|&ty| {
            let mut seen = FxHashSet::default();
            type_has_unsupported_effect(module, ty, grants, &mut seen)
        })
    {
        return true;
    }

    match &module.expr_arena[id] {
        TlcExpr::Var(_) | TlcExpr::Lit(_) | TlcExpr::Import(_) => false,
        TlcExpr::Lam(_, _, body) | TlcExpr::TyLam(_, _, body) => {
            reachable_expr_type_has_unsupported_effect(module, *body, visited, grants, true)
        }
        TlcExpr::TyApp(body, _) => {
            reachable_expr_type_has_unsupported_effect(module, *body, visited, grants, false)
        }
        TlcExpr::App(func, arg)
        | TlcExpr::Builtin(_, func, arg)
        | TlcExpr::ListAppend(func, arg) => {
            reachable_expr_type_has_unsupported_effect(module, *func, visited, grants, false)
                || reachable_expr_type_has_unsupported_effect(module, *arg, visited, grants, true)
        }
        TlcExpr::Let { value, body, .. } => {
            reachable_expr_type_has_unsupported_effect(module, *value, visited, grants, true)
                || reachable_expr_type_has_unsupported_effect(module, *body, visited, grants, true)
        }
        TlcExpr::Letrec { bindings, body } => {
            bindings.iter().any(|(_, _, value)| {
                reachable_expr_type_has_unsupported_effect(module, *value, visited, grants, true)
            }) || reachable_expr_type_has_unsupported_effect(module, *body, visited, grants, true)
        }
        TlcExpr::Case(scrutinee, alts) => {
            reachable_expr_type_has_unsupported_effect(module, *scrutinee, visited, grants, true)
                || alts.iter().any(|alt| {
                    alt.guard.is_some_and(|guard| {
                        reachable_expr_type_has_unsupported_effect(
                            module, guard, visited, grants, true,
                        )
                    }) || reachable_expr_type_has_unsupported_effect(
                        module, alt.body, visited, grants, true,
                    )
                })
        }
        TlcExpr::Record(fields) => fields.iter().any(|(_, value)| {
            reachable_expr_type_has_unsupported_effect(module, *value, visited, grants, true)
        }),
        TlcExpr::RecordUpdate { receiver, fields } => {
            reachable_expr_type_has_unsupported_effect(module, *receiver, visited, grants, true)
                || fields.iter().any(|(_, value)| {
                    reachable_expr_type_has_unsupported_effect(
                        module, *value, visited, grants, true,
                    )
                })
        }
        TlcExpr::GetField(base, _) => {
            reachable_expr_type_has_unsupported_effect(module, *base, visited, grants, false)
        }
        TlcExpr::Tuple(items) => items.iter().any(|item| match item {
            TlcTupleItem::Named { value, .. } | TlcTupleItem::Positional(value) => {
                reachable_expr_type_has_unsupported_effect(module, *value, visited, grants, true)
            }
        }),
        TlcExpr::List(items) | TlcExpr::Sequence(items) => items.iter().any(|item| {
            reachable_expr_type_has_unsupported_effect(module, *item, visited, grants, true)
        }),
        TlcExpr::Variant(_, payload) | TlcExpr::Resume { value: payload } => {
            reachable_expr_type_has_unsupported_effect(module, *payload, visited, grants, true)
        }
        TlcExpr::Perform { arg, .. } => {
            reachable_expr_type_has_unsupported_effect(module, *arg, visited, grants, true)
        }
        TlcExpr::Handle {
            expr,
            value,
            finally,
            ops,
        } => {
            reachable_expr_type_has_unsupported_effect(module, *expr, visited, grants, true)
                || value.is_some_and(|value| {
                    reachable_expr_type_has_unsupported_effect(module, value, visited, grants, true)
                })
                || finally.is_some_and(|finally| {
                    reachable_expr_type_has_unsupported_effect(
                        module, finally, visited, grants, true,
                    )
                })
                || ops.iter().any(|clause| {
                    reachable_expr_type_has_unsupported_effect(
                        module,
                        clause.body,
                        visited,
                        grants,
                        true,
                    )
                })
        }
    }
}

fn row_has_unsupported_effect(row: &Row, grants: HostEffectSet) -> bool {
    match row {
        Row::REmpty => false,
        Row::RExtend { label, tail, .. } => {
            HostOp::from_name(label).is_none_or(|op| !grants.contains(op))
                || row_has_unsupported_effect(tail, grants)
        }
        // Row variables are type-level openness, not runtime effects. Concrete
        // residual `perform`/`handle` nodes are checked separately above, and
        // concrete row entries still gate when the host capability is absent.
        Row::RVar(_) => false,
    }
}

/// Whether `ty`, walked structurally, contains a function arrow carrying an
/// effect row unsupported under `grants`. Used to scope the residual-effect gate
/// to types reachable from live code.
fn type_has_unsupported_effect(
    module: &TlcModule,
    ty: TlcTypeId,
    grants: HostEffectSet,
    seen: &mut FxHashSet<TlcTypeId>,
) -> bool {
    if !seen.insert(ty) {
        return false;
    }
    match &module.type_arena[ty] {
        TlcType::Fun(arg, ret, row) => {
            row_has_unsupported_effect(row, grants)
                || type_has_unsupported_effect(module, *arg, grants, seen)
                || type_has_unsupported_effect(module, *ret, grants, seen)
        }
        TlcType::ForAll(_, _, body) | TlcType::TyLamK(_, _, body) => {
            type_has_unsupported_effect(module, *body, grants, seen)
        }
        TlcType::TyApp(func, arg) => {
            type_has_unsupported_effect(module, *func, grants, seen)
                || type_has_unsupported_effect(module, *arg, grants, seen)
        }
        TlcType::Record(row) | TlcType::VariantT(row) => {
            row_field_has_unsupported_effect(module, row, grants, seen)
        }
        TlcType::Tuple(fields) => fields.iter().any(|field| {
            let inner = match field {
                TlcTupleField::Named { ty, .. } | TlcTupleField::Positional(ty) => *ty,
            };
            type_has_unsupported_effect(module, inner, grants, seen)
        }),
        TlcType::List(inner) | TlcType::Optional(inner) | TlcType::Maybe(inner) => {
            type_has_unsupported_effect(module, *inner, grants, seen)
        }
        TlcType::Prim(_) | TlcType::Opaque(_) | TlcType::Singleton(_) | TlcType::TyVar(_, _) => {
            false
        }
    }
}

/// Walk a record/variant row's field types for unsupported effectful arrows. A
/// bare row variable here is record-row polymorphism, not an effect, so it is
/// not itself unsupported.
fn row_field_has_unsupported_effect(
    module: &TlcModule,
    row: &Row,
    grants: HostEffectSet,
    seen: &mut FxHashSet<TlcTypeId>,
) -> bool {
    match row {
        Row::REmpty | Row::RVar(_) => false,
        Row::RExtend { ty, tail, .. } => {
            type_has_unsupported_effect(module, *ty, grants, seen)
                || row_field_has_unsupported_effect(module, tail, grants, seen)
        }
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
        TlcExpr::App(func, arg)
        | TlcExpr::Builtin(_, func, arg)
        | TlcExpr::ListAppend(func, arg) => {
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
        TlcExpr::Handle {
            expr,
            value,
            finally,
            ops,
        } => {
            reachable_host_io_print(module, *expr, visited)
                || value.is_some_and(|value| reachable_host_io_print(module, value, visited))
                || finally.is_some_and(|finally| reachable_host_io_print(module, finally, visited))
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
        TlcExpr::App(func, arg)
        | TlcExpr::Builtin(_, func, arg)
        | TlcExpr::ListAppend(func, arg) => {
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
