//! ANF → SSA lowering.
//!
//! Converts flat ANF bindings into basic blocks with phi nodes at join points.

use crate::*;
use std::collections::HashSet;
use zutai_anf::{
    AnfArm, AnfAtom, AnfBody, AnfDecl, AnfExpr, AnfModule, AnfPattern, AnfTupleItem,
    AnfTuplePatItem,
};

// ── Counter for fresh names ────────────────────────────────────────────────────

/// A simple counter for generating fresh register/block names.
#[derive(Default)]
struct Fresh {
    counter: usize,
}

impl Fresh {
    fn next_label(&mut self, hint: &str) -> String {
        let id = self.counter;
        self.counter += 1;
        format!("__{hint}_{id}")
    }

    fn next_lambda(&mut self) -> String {
        let id = self.counter;
        self.counter += 1;
        format!("__lambda_{id}")
    }
}

// ── Function builder ───────────────────────────────────────────────────────────

/// Builds one SSA function at a time. Completed blocks are accumulated;
/// the active block is the one currently accepting instructions.
struct FuncBuilder {
    /// Completed basic blocks (not including the active block).
    completed: Vec<SsaBlock>,
    /// The block currently being built.
    active: BlockBuilder,
}

impl FuncBuilder {
    fn new(entry_label: String) -> Self {
        FuncBuilder {
            completed: Vec::new(),
            active: BlockBuilder::new(entry_label),
        }
    }

    /// Finish the active block with `terminator`, push it to completed,
    /// and start a new active block with `label`.
    fn finish_and_start(&mut self, terminator: SsaTerminator, label: String) {
        self.active.terminator = Some(terminator);
        let old = std::mem::replace(&mut self.active, BlockBuilder::new(label));
        self.completed.push(old.finish());
    }

    /// Emit an instruction into the active block.
    fn push(&mut self, instr: SsaInstr) {
        self.active.instrs.push(instr);
    }

    /// Consume the builder, finishing the active block with `terminator`,
    /// and return all blocks (entry block first).
    fn finish(mut self, terminator: SsaTerminator) -> Vec<SsaBlock> {
        self.active.terminator = Some(terminator);
        let last = std::mem::replace(&mut self.active, BlockBuilder::new(String::new()));
        self.completed.push(last.finish());
        self.completed
    }
}

// ── Block builder ──────────────────────────────────────────────────────────────

struct BlockBuilder {
    label: String,
    instrs: Vec<SsaInstr>,
    terminator: Option<SsaTerminator>,
}

impl BlockBuilder {
    fn new(label: String) -> Self {
        BlockBuilder {
            label,
            instrs: Vec::new(),
            terminator: None,
        }
    }

    fn finish(self) -> SsaBlock {
        SsaBlock {
            label: self.label,
            instructions: self.instrs,
            terminator: self.terminator.expect("block missing terminator"),
        }
    }
}

// ── Lowering context ────────────────────────────────────────────────────────────

/// Context shared across all function lowerings in the module.
struct Ctx {
    fresh: Fresh,
    /// Lambda functions lifted out during lowering.
    lambdas: Vec<SsaFunc>,
}

impl Ctx {
    fn new() -> Self {
        Ctx {
            fresh: Fresh::default(),
            lambdas: Vec::new(),
        }
    }
}

// ── Converting ANF atoms to SSA values ─────────────────────────────────────────

fn atom_to_value(atom: &AnfAtom) -> SsaValue {
    match atom {
        AnfAtom::Var(name) => SsaValue::Reg(name.clone()),
        AnfAtom::Lit(lit) => SsaValue::Lit(lit.clone()),
        AnfAtom::Global(name) => SsaValue::Global(name.clone()),
    }
}

// ── Converting ANF tuple items ─────────────────────────────────────────────────

fn tuple_item_to_ssa(item: &AnfTupleItem) -> SsaTupleItem {
    match item {
        AnfTupleItem::Named { name, value } => SsaTupleItem::Named {
            name: name.clone(),
            value: atom_to_value(value),
        },
        AnfTupleItem::Positional(atom) => SsaTupleItem::Positional(atom_to_value(atom)),
    }
}

// ── Body lowering ──────────────────────────────────────────────────────────────

/// Lower an ANF body into the current function builder, emitting instructions
/// for each binding. Returns the SSA value of the body's result.
fn lower_body(body: &AnfBody, fb: &mut FuncBuilder, ctx: &mut Ctx) -> SsaValue {
    for (name, expr) in &body.bindings {
        lower_expr(name, expr, fb, ctx);
    }
    atom_to_value(&body.result)
}

// ── Expression lowering ────────────────────────────────────────────────────────

/// Lower an ANF expression, writing the result into register `dest`.
fn lower_expr(dest: &str, expr: &AnfExpr, fb: &mut FuncBuilder, ctx: &mut Ctx) {
    let op = match expr {
        AnfExpr::Atom(atom) => {
            // "let x = y" in ANF → in SSA, x is just an alias for y.
            // Emit a single-branch phi so dest is bound.
            let val = atom_to_value(atom);
            fb.push(SsaInstr {
                dest: dest.to_string(),
                op: SsaOp::Phi {
                    branches: vec![(fb.active.label.clone(), val)],
                },
            });
            return;
        }

        AnfExpr::Apply { func, arg } => SsaOp::Call {
            func: atom_to_value(func),
            arg: atom_to_value(arg),
        },

        AnfExpr::TyApp { poly, ty_args } => SsaOp::TyApp {
            poly: atom_to_value(poly),
            ty_args: ty_args.clone(),
        },

        AnfExpr::Lambda { param, body } => {
            // Closure conversion: compute free vars, generate a top-level function,
            // and create a closure record.
            let mut free = free_vars_body(body);
            free.remove(param);
            let mut captures: Vec<String> = free.into_iter().collect();
            captures.sort(); // deterministic order

            let func_name = ctx.fresh.next_lambda();

            // Build the lambda function: captures + param.
            let mut params = captures.clone();
            params.push(param.clone());
            let mut lambda_fb = FuncBuilder::new(format!("{func_name}_entry"));
            let result = lower_body(body, &mut lambda_fb, ctx);
            let lambda_blocks = lambda_fb.finish(SsaTerminator::Return(result));
            let lambda_func = SsaFunc {
                name: func_name.clone(),
                params,
                blocks: lambda_blocks,
            };
            ctx.lambdas.push(lambda_func);

            // Closure record: { __fn = global, captured_var = reg, ... }
            let mut fields: Vec<(String, SsaValue)> = Vec::new();
            fields.push(("__fn".to_string(), SsaValue::Global(func_name)));
            for cap in &captures {
                fields.push((cap.clone(), SsaValue::Reg(cap.clone())));
            }
            SsaOp::Record { fields }
        }

        AnfExpr::TyLam { ty_params: _, body } => {
            // Type erasure for v0: inline the body.
            let result = lower_body(body, fb, ctx);
            fb.push(SsaInstr {
                dest: dest.to_string(),
                op: SsaOp::Phi {
                    branches: vec![(fb.active.label.clone(), result)],
                },
            });
            return;
        }

        AnfExpr::Record(fields) => SsaOp::Record {
            fields: fields
                .iter()
                .map(|(name, atom)| (name.clone(), atom_to_value(atom)))
                .collect(),
        },

        AnfExpr::Tuple(items) => SsaOp::Tuple {
            items: items.iter().map(tuple_item_to_ssa).collect(),
        },

        AnfExpr::List(elems) => SsaOp::List {
            elems: elems.iter().map(atom_to_value).collect(),
        },

        AnfExpr::Select { base, field } => SsaOp::Select {
            base: atom_to_value(base),
            field: field.clone(),
        },

        AnfExpr::Match { scrutinee, arms } => {
            lower_match(dest, scrutinee, arms, fb, ctx);
            return;
        }

        AnfExpr::Coalesce { value, fallback } => SsaOp::Coalesce {
            value: atom_to_value(value),
            fallback: atom_to_value(fallback),
        },

        AnfExpr::Builtin { op, lhs, rhs } => SsaOp::Builtin {
            op: *op,
            lhs: atom_to_value(lhs),
            rhs: atom_to_value(rhs),
        },

        AnfExpr::Variant { tag, value } => SsaOp::Variant {
            tag: tag.clone(),
            value: atom_to_value(value),
        },

        AnfExpr::Error => SsaOp::Error,
    };

    fb.push(SsaInstr {
        dest: dest.to_string(),
        op,
    });
}

// ── Match lowering ─────────────────────────────────────────────────────────────

/// Lower a match expression. Creates arm blocks and a join block with a phi node.
fn lower_match(
    dest: &str,
    scrutinee: &AnfAtom,
    arms: &[AnfArm],
    fb: &mut FuncBuilder,
    ctx: &mut Ctx,
) {
    // Emit a MatchDiscriminant instruction in the current block.
    let scrut_reg = ctx.fresh.next_label("match_scrut");
    fb.push(SsaInstr {
        dest: scrut_reg.clone(),
        op: SsaOp::MatchDiscriminant {
            scrutinee: atom_to_value(scrutinee),
        },
    });

    // Plan: current block jumps to arm0, arm0→join, arm1→join, ..., join has phi.
    let join_label = ctx.fresh.next_label("join");

    // Labels for each arm block.
    let arm_labels: Vec<String> = (0..arms.len())
        .map(|_| ctx.fresh.next_label("arm"))
        .collect();

    // Jump from current block to the first arm.
    if let Some(first) = arm_labels.first() {
        fb.finish_and_start(SsaTerminator::Jump(first.clone()), arm_labels[0].clone());
        // We just started a new active block labeled arm_labels[0], but we'll
        // immediately replace it. The finish_and_start pushed the pre-match block
        // (which now has the MatchDiscriminant + Jump to first arm) and created a
        // new active block with label = arm_labels[0].
    } else {
        // No arms — unreachable, return error.
        fb.finish_and_start(
            SsaTerminator::Return(SsaValue::Reg("__error".to_string())),
            join_label.clone(),
        );
        return;
    }

    let mut phi_branches: Vec<(String, SsaValue)> = Vec::new();

    for (i, arm) in arms.iter().enumerate() {
        // Set the active block label to the correct arm label.
        // (After the first iteration we start a fresh block for the next arm.)
        let arm_label = arm_labels[i].clone();

        // If this is not the arm we started after the jump, start a new block.
        // Actually, after the first arm, we need to finish the previous arm's block
        // and start the current arm's block. But for the first arm, the active
        // block was already created by finish_and_start above.

        // Rename active block to match arm label.
        fb.active.label = arm_label;

        let mut arm_fb = FuncBuilder::new(fb.active.label.clone());
        // Take the instrs/label from fb's active block.
        arm_fb.active = std::mem::replace(&mut fb.active, BlockBuilder::new(String::new()));
        // arm_fb.active now has the arm label and any instrs from fb.

        // Bind pattern variables from the scrutinee register.
        bind_pattern(
            &arm.pattern,
            &SsaValue::Reg(scrut_reg.clone()),
            &mut arm_fb.active,
        );

        // Lower the arm body.
        let arm_result = lower_body(&arm.body, &mut arm_fb, ctx);

        // Collect arm blocks + finish arm's active block with Jump to join.
        // But first, move any completed blocks from arm_fb into fb.
        fb.completed.extend(arm_fb.completed);

        // The arm's active block jumps to join.
        let arm_block_label = arm_fb.active.label.clone();

        // Store phi branch: from this arm's block, the result is arm_result.
        phi_branches.push((arm_block_label.clone(), arm_result));

        // Finish the arm block and start the join block.
        // (If there are more arms, we'd need blocks for them, but in this
        // simplified model, all arms are in sequence and all jump to join.)
        fb.active = arm_fb.active;
        fb.active.terminator = Some(SsaTerminator::Jump(join_label.clone()));
        let done = std::mem::replace(&mut fb.active, BlockBuilder::new(String::new()));
        fb.completed.push(done.finish());
    }

    // Start the join block.
    fb.active = BlockBuilder::new(join_label.clone());
    fb.push(SsaInstr {
        dest: dest.to_string(),
        op: SsaOp::Phi {
            branches: phi_branches,
        },
    });
    // The join block's terminator will be set by subsequent lowering
    // (or by the function's Return terminator at the end).
}

// ── Pattern binding ────────────────────────────────────────────────────────────

/// Emit instructions to bind pattern variables from a scrutinee value.
fn bind_pattern(pattern: &AnfPattern, scrutinee: &SsaValue, bb: &mut BlockBuilder) {
    match pattern {
        AnfPattern::Wildcard | AnfPattern::Lit(_) | AnfPattern::Atom(_) => {}
        AnfPattern::Bind(name) => {
            bb.instrs.push(SsaInstr {
                dest: name.clone(),
                op: SsaOp::Phi {
                    branches: vec![(bb.label.clone(), scrutinee.clone())],
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
                        let tmp = format!("__tup_{i}_{name}");
                        bb.instrs.push(SsaInstr {
                            dest: tmp.clone(),
                            op: SsaOp::Select {
                                base: scrutinee.clone(),
                                field: name.clone(),
                            },
                        });
                        bind_pattern(inner, &SsaValue::Reg(tmp), bb);
                    }
                    AnfTuplePatItem::Positional(inner) => {
                        let tmp = format!("__tup_{i}");
                        bb.instrs.push(SsaInstr {
                            dest: tmp.clone(),
                            op: SsaOp::Select {
                                base: scrutinee.clone(),
                                field: format!("{i}"),
                            },
                        });
                        bind_pattern(inner, &SsaValue::Reg(tmp), bb);
                    }
                }
            }
        }
        AnfPattern::Record(fields) => {
            for (field_name, inner) in fields {
                let tmp = format!("__rec_{field_name}");
                bb.instrs.push(SsaInstr {
                    dest: tmp.clone(),
                    op: SsaOp::Select {
                        base: scrutinee.clone(),
                        field: field_name.clone(),
                    },
                });
                bind_pattern(inner, &SsaValue::Reg(tmp), bb);
            }
        }
        AnfPattern::Variant(tag, inner) => {
            let tmp = format!("__var_{tag}");
            bb.instrs.push(SsaInstr {
                dest: tmp.clone(),
                op: SsaOp::Select {
                    base: scrutinee.clone(),
                    field: format!("__{tag}_value"),
                },
            });
            bind_pattern(inner, &SsaValue::Reg(tmp), bb);
        }
    }
}

// ── Free variable analysis ─────────────────────────────────────────────────────

fn free_vars_atom(atom: &AnfAtom) -> HashSet<String> {
    match atom {
        AnfAtom::Var(name) => {
            let mut s = HashSet::new();
            s.insert(name.clone());
            s
        }
        AnfAtom::Lit(_) | AnfAtom::Global(_) => HashSet::new(),
    }
}

fn free_vars_expr(expr: &AnfExpr) -> HashSet<String> {
    match expr {
        AnfExpr::Atom(atom) => free_vars_atom(atom),
        AnfExpr::Apply { func, arg } => free_vars_atom(func)
            .union(&free_vars_atom(arg))
            .cloned()
            .collect(),
        AnfExpr::TyApp { poly, ty_args: _ } => free_vars_atom(poly),
        AnfExpr::Lambda { param, body } => {
            let mut fv = free_vars_body(body);
            fv.remove(param);
            fv
        }
        AnfExpr::TyLam { ty_params: _, body } => free_vars_body(body),
        AnfExpr::Record(fields) => fields.iter().flat_map(|(_, a)| free_vars_atom(a)).collect(),
        AnfExpr::Tuple(items) => items
            .iter()
            .flat_map(|i| match i {
                AnfTupleItem::Named { name: _, value } => free_vars_atom(value),
                AnfTupleItem::Positional(a) => free_vars_atom(a),
            })
            .collect(),
        AnfExpr::List(elems) => elems.iter().flat_map(free_vars_atom).collect(),
        AnfExpr::Select { base, field: _ } => free_vars_atom(base),
        AnfExpr::Match { scrutinee, arms } => {
            let mut fv = free_vars_atom(scrutinee);
            for arm in arms {
                fv.extend(free_vars_arm(arm));
            }
            fv
        }
        AnfExpr::Coalesce { value, fallback } => free_vars_atom(value)
            .union(&free_vars_atom(fallback))
            .cloned()
            .collect(),
        AnfExpr::Builtin { op: _, lhs, rhs } => free_vars_atom(lhs)
            .union(&free_vars_atom(rhs))
            .cloned()
            .collect(),
        AnfExpr::Variant { tag: _, value } => free_vars_atom(value),
        AnfExpr::Error => HashSet::new(),
    }
}

fn free_vars_arm(arm: &AnfArm) -> HashSet<String> {
    let mut fv = free_vars_body(&arm.body);
    if let Some(guard) = &arm.guard {
        fv.extend(free_vars_body(guard));
    }
    let bound = pattern_bindings(&arm.pattern);
    for b in &bound {
        fv.remove(b);
    }
    fv
}

fn pattern_bindings(pat: &AnfPattern) -> Vec<String> {
    match pat {
        AnfPattern::Wildcard | AnfPattern::Lit(_) | AnfPattern::Atom(_) => vec![],
        AnfPattern::Bind(name) => vec![name.clone()],
        AnfPattern::Tuple(items) => items
            .iter()
            .flat_map(|i| match i {
                AnfTuplePatItem::Named { name: _, pattern } => pattern_bindings(pattern),
                AnfTuplePatItem::Positional(p) => pattern_bindings(p),
            })
            .collect(),
        AnfPattern::Record(fields) => fields
            .iter()
            .flat_map(|(_, p)| pattern_bindings(p))
            .collect(),
        AnfPattern::Variant(_, inner) => pattern_bindings(inner),
    }
}

fn free_vars_body(body: &AnfBody) -> HashSet<String> {
    let mut fv = HashSet::new();
    let mut bound = HashSet::new();
    for (name, expr) in &body.bindings {
        for v in free_vars_expr(expr) {
            if !bound.contains(&v) {
                fv.insert(v);
            }
        }
        bound.insert(name.clone());
    }
    for v in free_vars_atom(&body.result) {
        if !bound.contains(&v) {
            fv.insert(v);
        }
    }
    fv
}

// ── Module-level lowering ──────────────────────────────────────────────────────

/// Lower a complete ANF module into SSA form.
pub fn lower_anf(module: &AnfModule) -> SsaModule {
    let mut ctx = Ctx::new();
    let mut decls = Vec::new();

    for decl in &module.decls {
        match decl {
            AnfDecl::Let { name, body } => {
                let func = lower_top_let(name, body, &mut ctx);
                // Any lambdas lifted during this body become separate Func decls.
                let lifted = std::mem::take(&mut ctx.lambdas);
                for lf in lifted {
                    decls.push(SsaDecl::Func(lf));
                }
                decls.push(SsaDecl::Func(func));
            }
            AnfDecl::Letrec { bindings } => {
                let funcs: Vec<SsaFunc> = bindings
                    .iter()
                    .map(|(name, body)| lower_top_let(name, body, &mut ctx))
                    .collect();
                // Lambdas lifted from letrec bodies become separate decls.
                let lifted = std::mem::take(&mut ctx.lambdas);
                for lf in lifted {
                    decls.push(SsaDecl::Func(lf));
                }
                decls.push(SsaDecl::RecGroup(funcs));
            }
        }
    }

    // Lower the root body into the entry function.
    let entry = lower_entry(&module.root, &mut ctx);
    let lifted = std::mem::take(&mut ctx.lambdas);
    for lf in lifted {
        decls.push(SsaDecl::Func(lf));
    }

    SsaModule { decls, entry }
}

/// Lower a top-level `let` declaration into an SSA function.
fn lower_top_let(name: &str, body: &AnfBody, ctx: &mut Ctx) -> SsaFunc {
    let mut fb = FuncBuilder::new(format!("{name}_entry"));
    let result = lower_body(body, &mut fb, ctx);
    let blocks = fb.finish(SsaTerminator::Return(result));
    SsaFunc {
        name: name.to_string(),
        params: vec![],
        blocks,
    }
}

/// Lower the module's root expression into an entry-point function.
fn lower_entry(body: &AnfBody, ctx: &mut Ctx) -> SsaFunc {
    let mut fb = FuncBuilder::new("__entry".to_string());
    let result = lower_body(body, &mut fb, ctx);
    let blocks = fb.finish(SsaTerminator::Return(result));
    SsaFunc {
        name: "__entry".to_string(),
        params: vec![],
        blocks,
    }
}
