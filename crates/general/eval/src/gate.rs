use super::*;

// ─── pre-flight gate ──────────────────────────────────────────────────────────

/// Check that `analysis` is fully typed and has no reachable error node.
///
/// Returns a reference to the completed `ThirFile` or an `EvalError`
/// describing exactly why evaluation is blocked.
pub fn check_well_typed(analysis: &zutai_semantic::Analysis) -> Result<&ThirFile, EvalError> {
    // 1. Reject if parse or HIR diagnostics are present.
    let blocking: Vec<String> = analysis
        .blocking_diagnostics()
        .map(|d| format!("{:?}", d.kind))
        .collect();
    if !blocking.is_empty() {
        return Err(EvalError::NotRunnable(blocking));
    }

    // 2. Reject if THIR type checking failed or is incomplete.
    if !analysis.is_thir_complete() {
        let thir_msgs: Vec<String> = analysis
            .thir
            .as_ref()
            .map(|lt| lt.diagnostics.iter().map(format_thir_diagnostic).collect())
            .unwrap_or_default();
        return Err(EvalError::TypeCheckFailed(thir_msgs));
    }

    // 3. Unwrap the ThirFile (guaranteed Some by is_thir_complete).
    let file = analysis.thir.as_ref().unwrap().file.as_ref().unwrap();

    // 4. Belt-and-suspenders: walk reachable exprs for Error nodes.
    if has_reachable_error(file) {
        return Err(EvalError::ErrorNodeReachable);
    }

    Ok(file)
}

/// Check that `analysis` is safe for the legacy THIR evaluator.
pub fn check_runnable(analysis: &zutai_semantic::Analysis) -> Result<&ThirFile, EvalError> {
    let file = check_well_typed(analysis)?;
    if let Some(reason) = analysis.effectful_program() {
        return Err(EvalError::EffectfulNotExecutable(reason.to_string()));
    }

    Ok(file)
}

fn format_thir_diagnostic(d: &zutai_thir::ThirDiagnostic) -> String {
    use zutai_thir::ThirDiagnosticKind::*;
    match &d.kind {
        TypeMismatch { expected, found } => {
            format!("type mismatch: expected {expected}, found {found}")
        }
        UnsupportedFeature { feature } => {
            format!("unsupported feature: {feature}")
        }
        ExpectedFunction { found } => format!("expected function, found {found}"),
        FunctionClauseArityMismatch { expected, found } => {
            format!("function clause arity mismatch: expected {expected} params, found {found}")
        }
        ExpectedRecord { found } => format!("expected record, found {found}"),
        ExpectedList { found } => format!("expected list, found {found}"),
        ExpectedTuple { found } => format!("expected tuple, found {found}"),
        ExpectedOptional { found } => format!("expected optional, found {found}"),
        EmptyListNeedsType => "empty list needs a type annotation".to_string(),
        TupleArityMismatch { expected, found } => {
            format!("tuple arity mismatch: expected {expected}, found {found}")
        }
        TupleFieldNameMismatch { expected, found } => {
            format!("tuple field name mismatch: expected {expected}, found {found}")
        }
        InvalidBinaryOperands { op, lhs, rhs } => {
            format!("invalid binary operands for `{op}`: {lhs} and {rhs}")
        }
        MissingRecordField { name } => format!("missing required record field `{name}`"),
        UnexpectedRecordField { name } => format!("unexpected record field `{name}`"),
        UnknownField { name } => format!("unknown field `{name}`"),
        AliasCycle { name } => format!("type alias cycle involving `{name}`"),
        ValueTypeUnavailable { name } => format!("type of `{name}` is unavailable"),
        InvalidTypeExpression { reason } => format!("invalid type expression: {reason}"),
        TypeCheckerNotImplemented => "type checker not yet implemented for this form".to_string(),
        LambdaNeedsTypeContext => "lambda expression requires type context".to_string(),
        MatchArmPatternCountMismatch { found } => {
            format!("match arm must have exactly 1 pattern, found {found}")
        }
        NonExhaustiveMatch { witness } => {
            format!("non-exhaustive patterns: `{witness}` not covered")
        }
        UnreachableMatchArm => "unreachable match arm".to_string(),
        TypeConstructorArityMismatch {
            name,
            expected,
            found,
        } => {
            format!("type constructor `{name}` expects {expected} argument(s), found {found}")
        }
        TypeLevelEvalLimitExceeded => {
            "type-level computation exceeded evaluation limit".to_string()
        }
        WitnessFieldTypeMismatch {
            name,
            expected,
            found,
        } => {
            format!("witness field `{name}` has type {found}, expected {expected}")
        }
        MissingWitnessField { name } => format!("missing witness field `{name}`"),
        UnknownWitnessField { name } => format!("unknown witness field `{name}`"),
        DeriveConstraintNotDerivable { constraint } => {
            format!("constraint `{constraint}` does not support derive")
        }
        DeriveComponentMissingWitness {
            constraint,
            component,
        } => {
            format!(
                "cannot derive `{constraint}` because component type `{component}` has no witness"
            )
        }
        DeriveUnsupportedMethod { constraint, method } => {
            format!(
                "cannot derive `{constraint}`: method `{method}` has no structural derivation recipe"
            )
        }
        ConflictingWitness { constraint, target } => {
            format!("conflicting witnesses for constraint `{constraint}` at type `{target}`")
        }
        RecursiveWitness { constraint } => {
            format!(
                "recursive witness for constraint `{constraint}`: target is one of its own type parameters"
            )
        }
        WitnessTargetKindMismatch { constraint, target } => {
            format!("witness target `{target}` has the wrong kind for constraint `{constraint}`")
        }
        UnsupportedMultiParamConstraint { name } => {
            format!("multi-param constraint `{name}` is not yet supported")
        }
        OverlappingRowField { name } => {
            format!("row tail introduces a field already declared: `{name}`")
        }
        RowAnnotationRequired => {
            "row-polymorphic inference is not principal here; add a type annotation".to_string()
        }
        EffectNotInRow { op } => {
            format!("effect `{op}` is not declared in the current effect row")
        }
        MalformedEffectOp { op, reason } => format!("malformed effect operation `{op}`: {reason}"),
        ResumeTypeMismatch { expected, found } => {
            format!("resume type mismatch: expected {expected}, found {found}")
        }
        HandlerClauseArityMismatch {
            op,
            expected,
            found,
        } => {
            format!("handler clause `{op}` expects {expected} parameter(s), found {found}")
        }
        MultipleResume { op } => {
            format!("handler clause `{op}` may resume more than once on one path")
        }
    }
}

/// Walk all reachable expressions in `file` and check for `Error` nodes.
fn has_reachable_error(file: &ThirFile) -> bool {
    // Check the final expression and all top-level declaration expressions.
    let mut to_visit: Vec<zutai_thir::ThirExprId> = vec![file.final_expr];
    for (_, decl) in file.decl_arena.iter() {
        match &decl.kind {
            zutai_thir::ThirDeclKind::Value { value, .. } => to_visit.push(*value),
            zutai_thir::ThirDeclKind::Function { clauses, .. } => {
                for clause in clauses {
                    to_visit.push(clause.body);
                    if let Some(g) = clause.guard {
                        to_visit.push(g);
                    }
                }
            }
            zutai_thir::ThirDeclKind::TypeAlias { .. } => {}
            // Constraint decls have no expr nodes to walk.
            zutai_thir::ThirDeclKind::Constraint { .. } => {}
            // Witness field values must be error-walked: a malformed field should
            // refuse evaluation just like a malformed top-level binding.
            zutai_thir::ThirDeclKind::Witness { fields, .. } => {
                for f in fields {
                    to_visit.push(f.value);
                }
            }
        }
    }

    let mut visited = std::collections::HashSet::new();
    let mut stack = to_visit;
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let expr = &file.expr_arena[id];
        match &expr.kind {
            ThirExprKind::Error => return true,
            ThirExprKind::Block { bindings, result } => {
                for b in bindings {
                    stack.push(b.value);
                }
                stack.push(*result);
            }
            ThirExprKind::Lambda { body, .. } => stack.push(*body),
            ThirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                stack.extend([*cond, *then_branch, *else_branch]);
            }
            ThirExprKind::Match { scrutinee, arms } => {
                stack.push(*scrutinee);
                for arm in arms {
                    stack.push(arm.body);
                    if let Some(g) = arm.guard {
                        stack.push(g);
                    }
                }
            }
            ThirExprKind::Apply { func, arg, .. } => stack.extend([*func, *arg]),
            ThirExprKind::Binary { lhs, rhs, .. } => stack.extend([*lhs, *rhs]),
            ThirExprKind::Access { receiver, .. }
            | ThirExprKind::OptionalAccess { receiver, .. } => stack.push(*receiver),
            ThirExprKind::Record(fields) => {
                for f in fields {
                    stack.push(f.value);
                }
            }
            ThirExprKind::Tuple(items) => {
                for item in items {
                    match item {
                        zutai_thir::ThirTupleItem::Named { value, .. } => stack.push(*value),
                        zutai_thir::ThirTupleItem::Positional(e) => stack.push(*e),
                    }
                }
            }
            ThirExprKind::List(items) => stack.extend(items.iter().copied()),
            ThirExprKind::TaggedValue { payload, .. } => stack.push(*payload),
            ThirExprKind::Perform { arg, .. } => stack.push(*arg),
            ThirExprKind::Resume { value } => stack.push(*value),
            ThirExprKind::Handle { expr, value, ops } => {
                stack.push(*expr);
                if let Some(value) = value {
                    stack.push(*value);
                }
                for op in ops {
                    stack.push(op.body);
                }
            }
            ThirExprKind::Sequence(items) => stack.extend(items.iter().copied()),
            // Leaves — no sub-expressions.
            ThirExprKind::True
            | ThirExprKind::False
            | ThirExprKind::Integer(_)
            | ThirExprKind::Float(_)
            | ThirExprKind::String(_)
            | ThirExprKind::Atom(_)
            | ThirExprKind::BindingRef(_)
            | ThirExprKind::Import(_)
            | ThirExprKind::TypeValue(_) => {}
        }
    }
    false
}
