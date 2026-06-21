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
    /// Names of top-level functions represented by static closure objects.
    global_closures: HashSet<String>,
}

impl Ctx {
    fn new(global_closures: HashSet<String>) -> Self {
        Ctx {
            fresh: Fresh::default(),
            lambdas: Vec::new(),
            global_closures,
        }
    }
}

// ── Converting ANF atoms to SSA values ─────────────────────────────────────────

fn atom_to_value(atom: &AnfAtom, globals: &HashSet<String>) -> SsaValue {
    match atom {
        AnfAtom::Var(name) => SsaValue::Reg(name.clone()),
        AnfAtom::Lit(lit) => SsaValue::Lit(lit.clone()),
        AnfAtom::Global(name) => {
            if globals.contains(name) {
                SsaValue::GlobalClosure(name.clone())
            } else {
                SsaValue::Global(name.clone())
            }
        }
    }
}

fn materialize_value(val: SsaValue, fb: &mut FuncBuilder, ctx: &mut Ctx) -> SsaValue {
    if let SsaValue::Global(name) = val {
        let dest = ctx.fresh.next_label("global");
        fb.push(SsaInstr {
            dest: dest.clone(),
            op: SsaOp::CallGlobal { name },
        });
        SsaValue::Reg(dest)
    } else {
        val
    }
}

fn lower_atom_value(atom: &AnfAtom, fb: &mut FuncBuilder, ctx: &mut Ctx) -> SsaValue {
    materialize_value(atom_to_value(atom, &ctx.global_closures), fb, ctx)
}

// ── Converting ANF tuple items ─────────────────────────────────────────────────

fn tuple_item_to_ssa(item: &AnfTupleItem, fb: &mut FuncBuilder, ctx: &mut Ctx) -> SsaTupleItem {
    match item {
        AnfTupleItem::Named { name, value } => SsaTupleItem::Named {
            name: name.clone(),
            value: lower_atom_value(value, fb, ctx),
        },
        AnfTupleItem::Positional(atom) => SsaTupleItem::Positional(lower_atom_value(atom, fb, ctx)),
    }
}

// ── Body lowering ──────────────────────────────────────────────────────────────

/// Lower an ANF body into the current function builder, emitting instructions
/// for each binding. Returns the SSA value of the body's result.
fn lower_body(body: &AnfBody, fb: &mut FuncBuilder, ctx: &mut Ctx) -> SsaValue {
    for (name, expr) in &body.bindings {
        lower_expr(name, expr, fb, ctx);
    }
    lower_atom_value(&body.result, fb, ctx)
}

// ── Expression lowering ────────────────────────────────────────────────────────

/// Lower an ANF expression, writing the result into register `dest`.
fn lower_expr(dest: &str, expr: &AnfExpr, fb: &mut FuncBuilder, ctx: &mut Ctx) {
    let op = match expr {
        AnfExpr::Atom(atom) => {
            // "let x = y" in ANF is a plain value alias. A phi node would be
            // invalid here because the current block has not necessarily been
            // reached through an edge named in the phi's predecessor list.
            let val = lower_atom_value(atom, fb, ctx);
            fb.push(SsaInstr {
                dest: dest.to_string(),
                op: SsaOp::Alias { value: val },
            });
            return;
        }

        AnfExpr::Apply { func, arg } => SsaOp::ApplyClosure {
            closure: lower_atom_value(func, fb, ctx),
            arg: lower_atom_value(arg, fb, ctx),
        },

        AnfExpr::TyApp { poly, ty_args } => SsaOp::TyApp {
            poly: lower_atom_value(poly, fb, ctx),
            ty_args: ty_args.clone(),
        },

        AnfExpr::Lambda { param, body } => {
            // Closure conversion: capture the lambda's free variables, lift its
            // body into a closure-code function, and allocate a closure object.
            let mut free = free_vars_body(body);
            free.remove(param);
            let mut captures: Vec<String> = free.into_iter().collect();
            captures.sort(); // deterministic capture order

            let func_name = ctx.fresh.next_lambda();
            let lambda_func = lower_lambda_function(func_name.clone(), param, body, &captures, ctx);
            ctx.lambdas.push(lambda_func);

            SsaOp::MakeClosure {
                code: func_name,
                captures: captures.iter().map(|c| SsaValue::Reg(c.clone())).collect(),
            }
        }

        AnfExpr::TyLam { ty_params: _, body } => {
            // Type erasure for v0: inline the body.
            let result = lower_body(body, fb, ctx);
            fb.push(SsaInstr {
                dest: dest.to_string(),
                op: SsaOp::Alias { value: result },
            });
            return;
        }

        AnfExpr::Record(fields) => {
            let mut values = Vec::with_capacity(fields.len());
            for atom in fields {
                values.push(lower_atom_value(atom, fb, ctx));
            }
            SsaOp::Record { fields: values }
        }

        AnfExpr::RecordUpdate { base, updates } => {
            let base = lower_atom_value(base, fb, ctx);
            let mut lowered = Vec::with_capacity(updates.len());
            for (slot, value) in updates {
                lowered.push((*slot, lower_atom_value(value, fb, ctx)));
            }
            SsaOp::RecordUpdate {
                base,
                updates: lowered,
            }
        }

        AnfExpr::Tuple(items) => {
            let mut lowered = Vec::with_capacity(items.len());
            for item in items {
                lowered.push(tuple_item_to_ssa(item, fb, ctx));
            }
            SsaOp::Tuple { items: lowered }
        }

        AnfExpr::List(elems) => {
            let mut lowered = Vec::with_capacity(elems.len());
            for elem in elems {
                lowered.push(lower_atom_value(elem, fb, ctx));
            }
            SsaOp::List { elems: lowered }
        }

        AnfExpr::Select { base, slot } => SsaOp::Select {
            base: lower_atom_value(base, fb, ctx),
            slot: *slot,
        },

        AnfExpr::Match { scrutinee, arms } => {
            lower_match(dest, scrutinee, arms, fb, ctx);
            return;
        }

        AnfExpr::Coalesce { value, fallback } => SsaOp::Coalesce {
            value: lower_atom_value(value, fb, ctx),
            fallback: lower_atom_value(fallback, fb, ctx),
        },

        AnfExpr::Builtin { op, lhs, rhs } => SsaOp::Builtin {
            op: *op,
            lhs: lower_atom_value(lhs, fb, ctx),
            rhs: lower_atom_value(rhs, fb, ctx),
        },

        AnfExpr::Variant {
            tag,
            tag_index,
            value,
        } => SsaOp::Variant {
            tag: tag.clone(),
            tag_index: *tag_index,
            value: lower_atom_value(value, fb, ctx),
        },

        AnfExpr::Error => SsaOp::Error,
    };

    fb.push(SsaInstr {
        dest: dest.to_string(),
        op,
    });
}

/// Build a closure-code function for a lambda: parameters are `[__self, param]`,
/// and each capture is loaded from the closure object before the body runs.
fn lower_lambda_function(
    func_name: String,
    param: &str,
    body: &AnfBody,
    captures: &[String],
    ctx: &mut Ctx,
) -> SsaFunc {
    let mut lambda_fb = FuncBuilder::new(format!("{func_name}_entry"));
    // Load every capture from the closure (`__self`) before lowering the body.
    for (index, cap) in captures.iter().enumerate() {
        lambda_fb.push(SsaInstr {
            dest: cap.clone(),
            op: SsaOp::LoadCapture {
                closure: SsaValue::Reg("__self".to_string()),
                index,
            },
        });
    }
    let result = lower_body(body, &mut lambda_fb, ctx);
    let blocks = lambda_fb.finish(SsaTerminator::Return(result));
    SsaFunc {
        name: func_name,
        params: vec!["__self".to_string(), param.to_string()],
        blocks,
    }
}

// ── Match lowering ─────────────────────────────────────────────────────────────

/// Lower a match expression into explicit tests, arm blocks, and a join phi.
fn lower_match(
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
            bind_pattern(&arm.pattern, &scrutinee_value, &mut fb.active);
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

fn lower_match_arm_test(
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
        bind_pattern(&arm.pattern, scrutinee, &mut fb.active);
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

fn emit_pattern_test(
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

fn emit_select_for_pattern(
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

fn emit_value_eq(lhs: SsaValue, rhs: SsaValue, fb: &mut FuncBuilder, ctx: &mut Ctx) -> SsaValue {
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

fn combine_optional_conditions(
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
fn bind_pattern(pattern: &AnfPattern, scrutinee: &SsaValue, bb: &mut BlockBuilder) {
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
                        let tmp = format!("__tup_{i}_{name}");
                        bb.instrs.push(SsaInstr {
                            dest: tmp.clone(),
                            op: SsaOp::Select {
                                base: scrutinee.clone(),
                                slot: i,
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
                                slot: i,
                            },
                        });
                        bind_pattern(inner, &SsaValue::Reg(tmp), bb);
                    }
                }
            }
        }
        AnfPattern::Record(fields) => {
            for (slot, inner) in fields {
                let tmp = format!("__rec_{slot}");
                bb.instrs.push(SsaInstr {
                    dest: tmp.clone(),
                    op: SsaOp::Select {
                        base: scrutinee.clone(),
                        slot: *slot,
                    },
                });
                bind_pattern(inner, &SsaValue::Reg(tmp), bb);
            }
        }
        AnfPattern::Variant { tag, pattern, .. } => {
            let tmp = format!("__var_{tag}");
            bb.instrs.push(SsaInstr {
                dest: tmp.clone(),
                op: SsaOp::VariantValue {
                    scrutinee: scrutinee.clone(),
                },
            });
            bind_pattern(pattern, &SsaValue::Reg(tmp), bb);
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
        AnfExpr::Record(fields) => fields.iter().flat_map(free_vars_atom).collect(),
        AnfExpr::RecordUpdate { base, updates } => {
            let mut fv = free_vars_atom(base);
            for (_, value) in updates {
                fv.extend(free_vars_atom(value));
            }
            fv
        }
        AnfExpr::Tuple(items) => items
            .iter()
            .flat_map(|i| match i {
                AnfTupleItem::Named { name: _, value } => free_vars_atom(value),
                AnfTupleItem::Positional(a) => free_vars_atom(a),
            })
            .collect(),
        AnfExpr::List(elems) => elems.iter().flat_map(free_vars_atom).collect(),
        AnfExpr::Select { base, slot: _ } => free_vars_atom(base),
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
        AnfExpr::Variant { value, .. } => free_vars_atom(value),
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
        AnfPattern::Variant { pattern, .. } => pattern_bindings(pattern),
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
    let closure_exports = collect_closure_exports(module);
    let global_closures: HashSet<String> = closure_exports.iter().cloned().collect();
    let mut ctx = Ctx::new(global_closures);
    let mut decls = Vec::new();

    for decl in &module.decls {
        match decl {
            AnfDecl::Let { name, body } => {
                let func = lower_top_decl(name, body, &mut ctx);
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
                    .map(|(name, body)| lower_top_decl(name, body, &mut ctx))
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

    SsaModule {
        decls,
        entry,
        entry_ty: module.root_ty.clone(),
        entry_ty_id: module.root_ty_id,
        types: module.types.clone(),
        closure_exports,
    }
}

/// Recognize a top-level binding whose value is a single lambda. Returns the
/// lambda's `(param, body)` when `body` consists of exactly one binding `v = λ`
/// (optionally wrapped in erased type lambdas) and `body.result` names `v`.
/// Any sibling binding yields `None`, so such a declaration lowers as a thunk.
fn top_level_lambda(body: &AnfBody) -> Option<(&String, &AnfBody)> {
    if body.bindings.len() != 1 {
        return None;
    }
    let (bind_name, expr) = &body.bindings[0];
    match &body.result {
        AnfAtom::Var(name) if name == bind_name => {}
        _ => return None,
    }
    match expr {
        AnfExpr::Lambda { param, body } => Some((param, body)),
        AnfExpr::TyLam { body, .. } => top_level_lambda(body),
        _ => None,
    }
}

/// Collect the names of top-level functions that become static empty-capture
/// closure objects, in declaration order, deduplicated by first occurrence.
fn collect_closure_exports(module: &AnfModule) -> Vec<String> {
    let mut exports = Vec::new();
    let mut seen = HashSet::new();
    for decl in &module.decls {
        match decl {
            AnfDecl::Let { name, body } => {
                if top_level_lambda(body).is_some() && seen.insert(name.clone()) {
                    exports.push(name.clone());
                }
            }
            AnfDecl::Letrec { bindings } => {
                for (name, body) in bindings {
                    if top_level_lambda(body).is_some() && seen.insert(name.clone()) {
                        exports.push(name.clone());
                    }
                }
            }
        }
    }
    exports
}

/// Lower a top-level declaration. Function-valued declarations become closure-
/// code functions named after the binding (`i64 @name(i64 __self, i64 arg)`);
/// every other declaration stays a zero-argument thunk over its body.
fn lower_top_decl(name: &str, body: &AnfBody, ctx: &mut Ctx) -> SsaFunc {
    if let Some((param, lambda_body)) = top_level_lambda(body) {
        let mut free = free_vars_body(lambda_body);
        free.remove(param);
        let mut captures: Vec<String> = free.into_iter().collect();
        captures.sort();
        lower_lambda_function(name.to_string(), param, lambda_body, &captures, ctx)
    } else {
        lower_top_let_thunk(name, body, ctx)
    }
}

/// Lower a non-function top-level declaration into a zero-argument thunk.
fn lower_top_let_thunk(name: &str, body: &AnfBody, ctx: &mut Ctx) -> SsaFunc {
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
