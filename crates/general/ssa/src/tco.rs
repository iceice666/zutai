//! Tail-call optimization for SSA.
//!
//! Two cooperating transforms let tail recursion run in constant stack space by
//! letting codegen emit LLVM `musttail` calls:
//!
//! 1. **Return sinking** — a match in tail position lowers to arm blocks that
//!    `Jump` to a join block of the shape `[%p = phi …] ; Return(%p)`. The
//!    recursive call then sits before a `br`, not a `ret`, so it is not in LLVM
//!    tail position. Sinking rewrites each predecessor's `Jump(join)` into
//!    `Return(value)` and deletes the join. Run to a fixpoint so nested tail
//!    matches peel from the outside in: sinking an outer join turns an inner
//!    join's `Jump` into a `Return`, exposing it for the next round.
//! 2. **Tail marking** — after sinking, any `ApplyClosure` that is the last
//!    instruction of a block whose terminator returns that instruction's `dest`
//!    is a genuine tail call. Mark it so codegen emits `musttail`.
//!
//! Return sinking is an always-safe CFG cleanup (it only collapses a pure
//! `phi`-then-`return` block into direct returns), so it is applied
//! unconditionally rather than gated on the presence of a tail call.

use crate::*;

/// Optimize tail calls across every function in the module in place.
pub fn optimize_tail_calls(module: &mut SsaModule) {
    optimize_func(&mut module.entry);
    for decl in &mut module.decls {
        match decl {
            SsaDecl::Func(func) => optimize_func(func),
            SsaDecl::RecGroup(funcs) => funcs.iter_mut().for_each(optimize_func),
        }
    }
}

fn optimize_func(func: &mut SsaFunc) {
    sink_returns(func);
    mark_tail_calls(func);
}

/// Collapse every `[phi] ; Return(phi)` join into direct returns from its
/// predecessors, to a fixpoint. Each step removes one block, so this
/// terminates.
fn sink_returns(func: &mut SsaFunc) {
    while let Some((join_label, branches)) = find_sinkable_join(func) {
        for (pred, value) in &branches {
            if let Some(block) = func.blocks.iter_mut().find(|b| &b.label == pred) {
                block.terminator = SsaTerminator::Return(value.clone());
            }
        }
        func.blocks.retain(|b| b.label != join_label);
    }
}

/// Find a join block safe to sink: it holds exactly one phi, returns that phi,
/// and every predecessor named by the phi reaches it through an unconditional
/// `Jump`. The entry block is never a join, so it is skipped.
fn find_sinkable_join(func: &SsaFunc) -> Option<(String, Vec<(String, SsaValue)>)> {
    'block: for block in func.blocks.iter().skip(1) {
        let [instr] = block.instructions.as_slice() else {
            continue;
        };
        let SsaOp::Phi { branches } = &instr.op else {
            continue;
        };
        match &block.terminator {
            SsaTerminator::Return(SsaValue::Reg(reg)) if *reg == instr.dest => {}
            _ => continue,
        }
        // Every block that targets this join must do so via an unconditional
        // `Jump` listed in the phi. A conditional branch or an unlisted edge
        // would dangle once the block is removed, so bail on those joins.
        for other in &func.blocks {
            let targets_join = match &other.terminator {
                SsaTerminator::Jump(target) => target == &block.label,
                SsaTerminator::Branch {
                    then_label,
                    else_label,
                    ..
                } => then_label == &block.label || else_label == &block.label,
                SsaTerminator::Return(_) => false,
            };
            if !targets_join {
                continue;
            }
            let listed_as_jump = branches.iter().any(|(pred, _)| {
                pred == &other.label && matches!(&other.terminator, SsaTerminator::Jump(_))
            });
            if !listed_as_jump {
                continue 'block;
            }
        }
        return Some((block.label.clone(), branches.clone()));
    }
    None
}

/// Mark every return-position `ApplyClosure` as a tail call. A call qualifies
/// when it is the last instruction of its block and the block returns exactly
/// that instruction's result.
///
/// LLVM `musttail` requires the caller and callee parameter lists to match.
/// Every closure-code function is `i64(i64, i64)` (`__self`, arg), matching the
/// indirect callee type, but zero-parameter thunks and the entry point do not —
/// so only two-parameter functions may carry tail calls. The single
/// entry-to-function call this skips costs one stack frame, not unbounded depth.
fn mark_tail_calls(func: &mut SsaFunc) {
    if func.params.len() != 2 {
        return;
    }
    for block in &mut func.blocks {
        let SsaTerminator::Return(SsaValue::Reg(ret)) = &block.terminator else {
            continue;
        };
        let ret = ret.clone();
        if let Some(last) = block.instructions.last_mut()
            && last.dest == ret
            && let SsaOp::ApplyClosure { tail, .. } = &mut last.op
        {
            *tail = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(dest: &str, closure: &str, arg: i64) -> SsaInstr {
        SsaInstr {
            dest: dest.to_string(),
            op: SsaOp::ApplyClosure {
                closure: SsaValue::GlobalClosure(closure.to_string()),
                arg: SsaValue::Lit(DfLit::Int(arg)),
                tail: false,
            },
        }
    }

    fn alias(dest: &str, value: i64) -> SsaInstr {
        SsaInstr {
            dest: dest.to_string(),
            op: SsaOp::Alias {
                value: SsaValue::Lit(DfLit::Int(value)),
            },
        }
    }

    fn is_tail(func: &SsaFunc, block: &str, dest: &str) -> bool {
        func.blocks
            .iter()
            .find(|b| b.label == block)
            .and_then(|b| b.instructions.iter().find(|i| i.dest == dest))
            .map(|i| matches!(&i.op, SsaOp::ApplyClosure { tail: true, .. }))
            .unwrap_or(false)
    }

    /// A straight-line tail call (no match) is marked directly.
    #[test]
    fn marks_straight_line_tail_call() {
        let mut func = SsaFunc {
            name: "f".into(),
            params: vec!["__self".into(), "n".into()],
            blocks: vec![SsaBlock {
                label: "entry".into(),
                instructions: vec![apply("r", "g", 1)],
                terminator: SsaTerminator::Return(SsaValue::Reg("r".into())),
            }],
        };
        optimize_func(&mut func);
        assert!(is_tail(&func, "entry", "r"));
    }

    /// A non-tail call (its result is used, not returned) is left untouched.
    #[test]
    fn leaves_non_tail_call_alone() {
        let mut func = SsaFunc {
            name: "f".into(),
            params: vec!["__self".into(), "n".into()],
            blocks: vec![SsaBlock {
                label: "entry".into(),
                instructions: vec![
                    apply("r", "g", 1),
                    SsaInstr {
                        dest: "s".into(),
                        op: SsaOp::Builtin {
                            op: DfBuiltinOp::Add,
                            lhs: SsaValue::Reg("r".into()),
                            rhs: SsaValue::Lit(DfLit::Int(1)),
                        },
                    },
                ],
                terminator: SsaTerminator::Return(SsaValue::Reg("s".into())),
            }],
        };
        optimize_func(&mut func);
        assert!(!is_tail(&func, "entry", "r"));
    }

    /// A match in tail position: the recursive arm's call sits before a jump to
    /// a `phi; ret phi` join. Sinking turns the arm into a direct return, after
    /// which the call is marked tail and the join is deleted.
    #[test]
    fn sinks_join_and_marks_recursive_arm() {
        let mut func = SsaFunc {
            name: "loop".into(),
            params: vec!["__self".into(), "n".into()],
            blocks: vec![
                SsaBlock {
                    label: "entry".into(),
                    instructions: vec![],
                    terminator: SsaTerminator::Branch {
                        cond: SsaValue::Reg("n".into()),
                        then_label: "base".into(),
                        else_label: "rec".into(),
                    },
                },
                SsaBlock {
                    label: "base".into(),
                    instructions: vec![alias("z", 0)],
                    terminator: SsaTerminator::Jump("join".into()),
                },
                SsaBlock {
                    label: "rec".into(),
                    instructions: vec![apply("rc", "loop", 1)],
                    terminator: SsaTerminator::Jump("join".into()),
                },
                SsaBlock {
                    label: "join".into(),
                    instructions: vec![SsaInstr {
                        dest: "r".into(),
                        op: SsaOp::Phi {
                            branches: vec![
                                ("base".into(), SsaValue::Reg("z".into())),
                                ("rec".into(), SsaValue::Reg("rc".into())),
                            ],
                        },
                    }],
                    terminator: SsaTerminator::Return(SsaValue::Reg("r".into())),
                },
            ],
        };
        optimize_func(&mut func);
        // Join collapsed away.
        assert!(!func.blocks.iter().any(|b| b.label == "join"));
        // The base arm now returns its literal directly.
        let base = func.blocks.iter().find(|b| b.label == "base").unwrap();
        assert_eq!(
            base.terminator,
            SsaTerminator::Return(SsaValue::Reg("z".into()))
        );
        // The recursive arm now returns the call result, and the call is tail.
        let rec = func.blocks.iter().find(|b| b.label == "rec").unwrap();
        assert_eq!(
            rec.terminator,
            SsaTerminator::Return(SsaValue::Reg("rc".into()))
        );
        assert!(is_tail(&func, "rec", "rc"));
    }

    /// Nested tail matches peel from the outside in: sinking the outer join
    /// exposes the inner join, whose recursive arm then becomes a tail call.
    #[test]
    fn sinks_nested_joins_to_fixpoint() {
        let mut func = SsaFunc {
            name: "loop".into(),
            params: vec!["__self".into(), "n".into()],
            blocks: vec![
                SsaBlock {
                    label: "entry".into(),
                    instructions: vec![],
                    terminator: SsaTerminator::Branch {
                        cond: SsaValue::Reg("n".into()),
                        then_label: "a".into(),
                        else_label: "inner".into(),
                    },
                },
                SsaBlock {
                    label: "a".into(),
                    instructions: vec![alias("av", 1)],
                    terminator: SsaTerminator::Jump("outer_join".into()),
                },
                SsaBlock {
                    label: "inner".into(),
                    instructions: vec![],
                    terminator: SsaTerminator::Branch {
                        cond: SsaValue::Reg("n".into()),
                        then_label: "b".into(),
                        else_label: "rec".into(),
                    },
                },
                SsaBlock {
                    label: "b".into(),
                    instructions: vec![alias("bv", 2)],
                    terminator: SsaTerminator::Jump("inner_join".into()),
                },
                SsaBlock {
                    label: "rec".into(),
                    instructions: vec![apply("rc", "loop", 1)],
                    terminator: SsaTerminator::Jump("inner_join".into()),
                },
                SsaBlock {
                    label: "inner_join".into(),
                    instructions: vec![SsaInstr {
                        dest: "ir".into(),
                        op: SsaOp::Phi {
                            branches: vec![
                                ("b".into(), SsaValue::Reg("bv".into())),
                                ("rec".into(), SsaValue::Reg("rc".into())),
                            ],
                        },
                    }],
                    terminator: SsaTerminator::Jump("outer_join".into()),
                },
                SsaBlock {
                    label: "outer_join".into(),
                    instructions: vec![SsaInstr {
                        dest: "r".into(),
                        op: SsaOp::Phi {
                            branches: vec![
                                ("a".into(), SsaValue::Reg("av".into())),
                                ("inner_join".into(), SsaValue::Reg("ir".into())),
                            ],
                        },
                    }],
                    terminator: SsaTerminator::Return(SsaValue::Reg("r".into())),
                },
            ],
        };
        optimize_func(&mut func);
        assert!(!func.blocks.iter().any(|b| b.label == "outer_join"));
        assert!(!func.blocks.iter().any(|b| b.label == "inner_join"));
        assert!(is_tail(&func, "rec", "rc"));
    }
}
