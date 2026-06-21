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
            .map(|lt| {
                lt.diagnostics
                    .iter()
                    .map(describe_thir_diagnostic)
                    .collect()
            })
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

pub fn describe_thir_diagnostic(d: &zutai_thir::ThirDiagnostic) -> String {
    use zutai_thir::{RowOverlapItem, ThirDiagnosticKind::*};
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
        ExpectedOptionalOrMaybe { found } => format!("expected Optional or Maybe, found {found}"),
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
        OverlappingRowField {
            item: RowOverlapItem::RecordField,
            source,
            name,
            existing,
            incoming,
        } => {
            format!(
                "record row tail `...{source}` overlaps explicit field `{name}`: existing `{existing}`, incoming `{incoming}`"
            )
        }
        OverlappingRowField {
            item: RowOverlapItem::UnionMember,
            source,
            name,
            existing,
            incoming,
        } => {
            format!(
                "union row tail `...{source}` overlaps explicit member `#{name}`: existing `{existing}`, incoming `{incoming}`"
            )
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
        InfiniteType => {
            "infinite type: a value cannot have a type that contains itself".to_string()
        }
    }
}

/// Human-readable description of a HIR (name-resolution) diagnostic.
pub fn describe_hir_diagnostic(d: &zutai_hir::HirDiagnostic) -> String {
    use zutai_hir::HirDiagnosticKind::*;
    match &d.kind {
        DuplicateBinding { name, .. } => format!("duplicate binding `{name}`"),
        DuplicateRecordField { name, .. } => format!("duplicate record field `{name}`"),
        DuplicateTypeRecordField { name, .. } => format!("duplicate record field `{name}`"),
        DuplicateRecordPatternField { name, .. } => {
            format!("duplicate field `{name}` in record pattern")
        }
        DuplicateTupleField { name, .. } => format!("duplicate named tuple field `{name}`"),
        DuplicateTypeTupleField { name, .. } => format!("duplicate named tuple field `{name}`"),
        DuplicateTuplePatternField { name, .. } => {
            format!("duplicate field `{name}` in tuple pattern")
        }
        UnknownIdentifier { name } => format!("unknown identifier `{name}`"),
        DuplicateConstraintMethod { name, .. } => format!("duplicate constraint method `{name}`"),
        DuplicateWitnessField { name, .. } => format!("duplicate witness field `{name}`"),
        UnknownConstraint { name } => format!("unknown constraint `{name}`"),
        DuplicateSelectField { name, .. } => {
            format!("duplicate field `{name}` in select projection")
        }
        InvalidRowTailTarget { name } => {
            format!("row tail `...{name}` is neither a row variable nor a spreadable type")
        }
        ResumeOutsideHandler => "resume outside an operation handler clause".to_string(),
    }
}

/// Human-readable message plus byte span `(start, end)` for a semantic
/// diagnostic that carries a source location (THIR type errors and HIR
/// name-resolution errors). Returns `None` for stages rendered elsewhere
/// (parse and import diagnostics).
pub fn describe_semantic_diagnostic(
    d: &zutai_semantic::SemanticDiagnostic,
) -> Option<(String, u32, u32)> {
    match &d.kind {
        zutai_semantic::SemanticDiagnosticKind::Thir(t) => {
            Some((describe_thir_diagnostic(t), t.span.start, t.span.end))
        }
        zutai_semantic::SemanticDiagnosticKind::Hir(h) => {
            Some((describe_hir_diagnostic(h), h.span.start, h.span.end))
        }
        _ => None,
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
            ThirExprKind::RecordUpdate { receiver, fields } => {
                stack.push(*receiver);
                for field in fields {
                    stack.push(field.value);
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
