//! ANF → SSA lowering.
//!
//! Converts flat ANF bindings into basic blocks with phi nodes at join points.

use crate::*;
use zutai_anf::{AnfArm, AnfAtom, AnfPattern, AnfTuplePatItem};

use super::*;

use super::expr::*;

// ── Match lowering ─────────────────────────────────────────────────────────────

/// Lower a match expression into explicit tests, arm blocks, and a join phi.
pub(super) fn lower_match(
    dest: &str,
    scrutinee: &AnfAtom,
    arms: &[AnfArm],
    fb: &mut FuncBuilder,
    ctx: &mut Ctx,
) {
    let scrutinee_value = lower_atom_value(scrutinee, fb, ctx);

    if arms.is_empty() {
        fb.push(SsaInstr {
            dest: dest.to_string(),
            op: SsaOp::Alias {
                value: SsaValue::Lit(DfLit::Int(0)),
            },
        });
        return;
    }

    let join_label = ctx.fresh.next_label("join");
    let miss_label = ctx.fresh.next_label("match_miss");
    let arm_labels: Vec<String> = (0..arms.len())
        .map(|_| ctx.fresh.next_label("arm"))
        .collect();
    let mut test_labels = Vec::with_capacity(arms.len());
    test_labels.push(fb.active.label.clone());
    test_labels.extend((1..arms.len()).map(|_| ctx.fresh.next_label("match_test")));

    let mut phi_branches: Vec<(String, SsaValue)> = Vec::with_capacity(arms.len());

    for (i, arm) in arms.iter().enumerate() {
        let arm_label = arm_labels[i].clone();
        let next_label = if i + 1 < arms.len() {
            test_labels[i + 1].clone()
        } else {
            miss_label.clone()
        };

        lower_match_arm_test(&scrutinee_value, arm, &arm_label, &next_label, fb, ctx);

        if arm.guard.is_none() {
            bind_pattern(
                &arm.pattern,
                &scrutinee_value,
                &mut fb.active,
                &mut ctx.fresh,
            );
        }
        let arm_result = lower_body(&arm.body, fb, ctx);
        let arm_exit_label = fb.active.label.clone();
        phi_branches.push((arm_exit_label, arm_result));

        let next_active = if i + 1 < arms.len() {
            test_labels[i + 1].clone()
        } else {
            miss_label.clone()
        };
        fb.finish_and_start(SsaTerminator::Jump(join_label.clone()), next_active);
    }

    // Semantic checking is responsible for exhaustiveness. Keep the generated
    // control-flow graph structurally complete if an earlier stage lets a
    // non-exhaustive match through.
    fb.finish_and_start(
        SsaTerminator::Return(SsaValue::Lit(DfLit::Int(0))),
        join_label,
    );
    fb.push(SsaInstr {
        dest: dest.to_string(),
        op: SsaOp::Phi {
            branches: phi_branches,
        },
    });
}

pub(super) fn lower_match_arm_test(
    scrutinee: &SsaValue,
    arm: &AnfArm,
    arm_label: &str,
    next_label: &str,
    fb: &mut FuncBuilder,
    ctx: &mut Ctx,
) {
    let guard_label = if arm.guard.is_some() {
        ctx.fresh.next_label("guard")
    } else {
        arm_label.to_string()
    };

    match emit_pattern_test(&arm.pattern, scrutinee, fb, ctx) {
        Some(cond) => fb.finish_and_start(
            SsaTerminator::Branch {
                cond,
                then_label: guard_label.clone(),
                else_label: next_label.to_string(),
            },
            guard_label.clone(),
        ),
        None => fb.finish_and_start(
            SsaTerminator::Jump(guard_label.clone()),
            guard_label.clone(),
        ),
    }

    if let Some(guard) = &arm.guard {
        bind_pattern(&arm.pattern, scrutinee, &mut fb.active, &mut ctx.fresh);
        let guard_cond = lower_body(guard, fb, ctx);
        fb.finish_and_start(
            SsaTerminator::Branch {
                cond: guard_cond,
                then_label: arm_label.to_string(),
                else_label: next_label.to_string(),
            },
            arm_label.to_string(),
        );
    }
}

pub(super) fn emit_pattern_test(
    pattern: &AnfPattern,
    scrutinee: &SsaValue,
    fb: &mut FuncBuilder,
    ctx: &mut Ctx,
) -> Option<SsaValue> {
    match pattern {
        AnfPattern::Wildcard | AnfPattern::Bind(_) => None,
        AnfPattern::Lit(lit) => Some(emit_value_eq(
            scrutinee.clone(),
            SsaValue::Lit(lit.clone()),
            fb,
            ctx,
        )),
        AnfPattern::Atom(name) => Some(emit_value_eq(
            scrutinee.clone(),
            SsaValue::Lit(DfLit::Atom(name.clone())),
            fb,
            ctx,
        )),
        AnfPattern::Tuple(items) => {
            let mut combined = None;
            for (i, item) in items.iter().enumerate() {
                let inner = match item {
                    AnfTuplePatItem::Named { pattern, .. }
                    | AnfTuplePatItem::Positional(pattern) => pattern,
                };
                let field = emit_select_for_pattern(scrutinee.clone(), i, fb, ctx);
                combined = combine_optional_conditions(
                    combined,
                    emit_pattern_test(inner, &field, fb, ctx),
                    fb,
                    ctx,
                );
            }
            combined
        }
        AnfPattern::ListNil => {
            let is_nil = ctx.fresh.next_label("list_is_nil");
            fb.push(SsaInstr {
                dest: is_nil.clone(),
                op: SsaOp::ListPrim {
                    op: DfListPrimOp::IsNil,
                    args: vec![scrutinee.clone()],
                },
            });
            Some(SsaValue::Reg(is_nil))
        }
        AnfPattern::ListCons { head, tail } => {
            let is_nil = ctx.fresh.next_label("list_is_nil");
            fb.push(SsaInstr {
                dest: is_nil.clone(),
                op: SsaOp::ListPrim {
                    op: DfListPrimOp::IsNil,
                    args: vec![scrutinee.clone()],
                },
            });
            let not_nil = emit_value_eq(
                SsaValue::Reg(is_nil),
                SsaValue::Lit(DfLit::Bool(false)),
                fb,
                ctx,
            );
            let head_value = emit_list_part_for_pattern(
                scrutinee.clone(),
                DfListPrimOp::Head,
                "list_head",
                fb,
                ctx,
            );
            let tail_value = emit_list_part_for_pattern(
                scrutinee.clone(),
                DfListPrimOp::Tail,
                "list_tail",
                fb,
                ctx,
            );
            let with_head = combine_optional_conditions(
                Some(not_nil),
                emit_pattern_test(head, &head_value, fb, ctx),
                fb,
                ctx,
            );
            combine_optional_conditions(
                with_head,
                emit_pattern_test(tail, &tail_value, fb, ctx),
                fb,
                ctx,
            )
        }
        AnfPattern::Record(fields) => {
            let mut combined = None;
            for (slot, inner) in fields {
                let field = emit_select_for_pattern(scrutinee.clone(), *slot, fb, ctx);
                combined = combine_optional_conditions(
                    combined,
                    emit_pattern_test(inner, &field, fb, ctx),
                    fb,
                    ctx,
                );
            }
            combined
        }
        AnfPattern::Variant {
            tag_index, pattern, ..
        } => {
            let tag = ctx.fresh.next_label("variant_tag");
            fb.push(SsaInstr {
                dest: tag.clone(),
                op: SsaOp::MatchDiscriminant {
                    scrutinee: scrutinee.clone(),
                },
            });
            let tag_matches = emit_value_eq(
                SsaValue::Reg(tag),
                SsaValue::Lit(DfLit::Int(*tag_index as i64)),
                fb,
                ctx,
            );
            let payload = ctx.fresh.next_label("variant_payload");
            fb.push(SsaInstr {
                dest: payload.clone(),
                op: SsaOp::VariantValue {
                    scrutinee: scrutinee.clone(),
                },
            });
            combine_optional_conditions(
                Some(tag_matches),
                emit_pattern_test(pattern, &SsaValue::Reg(payload), fb, ctx),
                fb,
                ctx,
            )
        }
    }
}

pub(super) fn emit_select_for_pattern(
    scrutinee: SsaValue,
    slot: usize,
    fb: &mut FuncBuilder,
    ctx: &mut Ctx,
) -> SsaValue {
    let dest = ctx.fresh.next_label("pat_field");
    fb.push(SsaInstr {
        dest: dest.clone(),
        op: SsaOp::Select {
            base: scrutinee,
            slot,
        },
    });
    SsaValue::Reg(dest)
}

pub(super) fn emit_list_part_for_pattern(
    scrutinee: SsaValue,
    op: DfListPrimOp,
    label: &str,
    fb: &mut FuncBuilder,
    ctx: &mut Ctx,
) -> SsaValue {
    let dest = ctx.fresh.next_label(label);
    fb.push(SsaInstr {
        dest: dest.clone(),
        op: SsaOp::ListPrim {
            op,
            args: vec![scrutinee],
        },
    });
    SsaValue::Reg(dest)
}

pub(super) fn emit_value_eq(
    lhs: SsaValue,
    rhs: SsaValue,
    fb: &mut FuncBuilder,
    ctx: &mut Ctx,
) -> SsaValue {
    let dest = ctx.fresh.next_label("pat_eq");
    fb.push(SsaInstr {
        dest: dest.clone(),
        op: SsaOp::Builtin {
            op: DfBuiltinOp::Eq,
            lhs,
            rhs,
        },
    });
    SsaValue::Reg(dest)
}

pub(super) fn combine_optional_conditions(
    lhs: Option<SsaValue>,
    rhs: Option<SsaValue>,
    fb: &mut FuncBuilder,
    ctx: &mut Ctx,
) -> Option<SsaValue> {
    match (lhs, rhs) {
        (None, None) => None,
        (Some(cond), None) | (None, Some(cond)) => Some(cond),
        (Some(lhs), Some(rhs)) => {
            let dest = ctx.fresh.next_label("pat_and");
            fb.push(SsaInstr {
                dest: dest.clone(),
                op: SsaOp::Builtin {
                    op: DfBuiltinOp::And,
                    lhs,
                    rhs,
                },
            });
            Some(SsaValue::Reg(dest))
        }
    }
}

// ── Pattern binding ────────────────────────────────────────────────────────────

/// Emit instructions to bind pattern variables from a scrutinee value.
pub(super) fn bind_pattern(
    pattern: &AnfPattern,
    scrutinee: &SsaValue,
    bb: &mut BlockBuilder,
    fresh: &mut Fresh,
) {
    match pattern {
        AnfPattern::Wildcard | AnfPattern::Lit(_) | AnfPattern::Atom(_) => {}
        AnfPattern::Bind(name) => {
            bb.instrs.push(SsaInstr {
                dest: name.clone(),
                op: SsaOp::Alias {
                    value: scrutinee.clone(),
                },
            });
        }
        AnfPattern::Tuple(items) => {
            for (i, item) in items.iter().enumerate() {
                match item {
                    AnfTuplePatItem::Named {
                        name,
                        pattern: inner,
                    } => {
                        let tmp = fresh.next_label(&format!("tup_{i}_{name}"));
                        bb.instrs.push(SsaInstr {
                            dest: tmp.clone(),
                            op: SsaOp::Select {
                                base: scrutinee.clone(),
                                slot: i,
                            },
                        });
                        bind_pattern(inner, &SsaValue::Reg(tmp), bb, fresh);
                    }
                    AnfTuplePatItem::Positional(inner) => {
                        let tmp = fresh.next_label(&format!("tup_{i}"));
                        bb.instrs.push(SsaInstr {
                            dest: tmp.clone(),
                            op: SsaOp::Select {
                                base: scrutinee.clone(),
                                slot: i,
                            },
                        });
                        bind_pattern(inner, &SsaValue::Reg(tmp), bb, fresh);
                    }
                }
            }
        }
        AnfPattern::ListNil => {}
        AnfPattern::ListCons { head, tail } => {
            let head_tmp = fresh.next_label("list_head");
            bb.instrs.push(SsaInstr {
                dest: head_tmp.clone(),
                op: SsaOp::ListPrim {
                    op: DfListPrimOp::Head,
                    args: vec![scrutinee.clone()],
                },
            });
            bind_pattern(head, &SsaValue::Reg(head_tmp), bb, fresh);

            let tail_tmp = fresh.next_label("list_tail");
            bb.instrs.push(SsaInstr {
                dest: tail_tmp.clone(),
                op: SsaOp::ListPrim {
                    op: DfListPrimOp::Tail,
                    args: vec![scrutinee.clone()],
                },
            });
            bind_pattern(tail, &SsaValue::Reg(tail_tmp), bb, fresh);
        }
        AnfPattern::Record(fields) => {
            for (slot, inner) in fields {
                let tmp = fresh.next_label(&format!("rec_{slot}"));
                bb.instrs.push(SsaInstr {
                    dest: tmp.clone(),
                    op: SsaOp::Select {
                        base: scrutinee.clone(),
                        slot: *slot,
                    },
                });
                bind_pattern(inner, &SsaValue::Reg(tmp), bb, fresh);
            }
        }
        AnfPattern::Variant { tag, pattern, .. } => {
            let tmp = fresh.next_label(&format!("var_{tag}"));
            bb.instrs.push(SsaInstr {
                dest: tmp.clone(),
                op: SsaOp::VariantValue {
                    scrutinee: scrutinee.clone(),
                },
            });
            bind_pattern(pattern, &SsaValue::Reg(tmp), bb, fresh);
        }
    }
}
