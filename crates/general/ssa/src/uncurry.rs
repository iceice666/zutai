//! Uncurrying / known-call optimization for SSA.
//!
//! A curried `N`-argument call lowers to `N` sequential [`SsaOp::ApplyClosure`]
//! instructions; each non-final application allocates a closure object (and its
//! captured-argument storage), so an accumulator loop pays one closure + one
//! arg-tuple per iteration of pure calling-convention churn.
//!
//! This pass collapses a *saturated* call to a *known* top-level function (a
//! chain of exactly `arity` consecutive `ApplyClosure`s whose head is a
//! [`SsaValue::GlobalClosure`] and whose intermediate results are single-use)
//! into one direct [`SsaOp::CallKnown`] to a generated multi-parameter *worker*
//! function. The worker runs the fully-applied body with every argument bound
//! as a direct SSA parameter — no intermediate closures, no arg-tuples. A
//! self-recursive worker call sits in tail position, so the later TCO pass marks
//! it `musttail` and the loop runs in constant stack with no per-iteration heap
//! churn.
//!
//! The original curried function is kept: it is still needed wherever the
//! function is used as a value or applied to fewer than `arity` arguments.

use rustc_hash::{FxHashMap, FxHashSet};

use zutai_anf::{AnfBody, AnfDecl, AnfModule};

use crate::*;

/// Suffix for a generated uncurried worker's global name.
fn worker_name(func: &str) -> String {
    format!("{func}$uncurried")
}

/// Run the uncurrying pass over `ssa` in place, using `anf` to recover each
/// curried function's arity and fully-applied body. `fresh_start` seeds worker
/// lambda-name generation so it never collides with the main lowering.
pub fn uncurry(ssa: &mut SsaModule, anf: &AnfModule, fresh_start: usize) {
    // Arity (≥2) and the fully-applied body for every top-level curried function.
    let mut bodies: FxHashMap<String, (Vec<String>, &AnfBody)> = FxHashMap::default();
    for decl in &anf.decls {
        let funcs: Vec<(&String, &AnfBody)> = match decl {
            AnfDecl::Let { name, body } => vec![(name, body)],
            AnfDecl::Letrec { bindings } => bindings.iter().map(|(n, b)| (n, b)).collect(),
        };
        for (name, body) in funcs {
            let (params, final_body) = peel_lambdas(body);
            if params.len() < 2 {
                continue;
            }
            // A top-level function is closure-free; confirm the worker body
            // references only its own parameters before relying on direct params.
            let free = crate::lower::body_free_vars(final_body);
            let param_set: FxHashSet<&String> = params.iter().collect();
            if free.iter().any(|v| !param_set.contains(v)) {
                continue;
            }
            bodies.insert(name.clone(), (params, final_body));
        }
    }
    if bodies.is_empty() {
        return;
    }
    let arity: FxHashMap<String, usize> = bodies
        .iter()
        .map(|(n, (p, _))| (n.clone(), p.len()))
        .collect();

    // Targets: functions that have at least one saturated known-call site.
    let mut targets: FxHashSet<String> = FxHashSet::default();
    for func in all_funcs(ssa) {
        scan_targets(func, &arity, &mut targets);
    }
    targets.retain(|t| bodies.contains_key(t));
    if targets.is_empty() {
        return;
    }
    // Generate a worker for each target, sharing one counter so lifted lambda
    // names stay unique across workers and disjoint from the main lowering.
    let global_closures: FxHashSet<String> = ssa.closure_exports.iter().cloned().collect();
    let mut counter = fresh_start;
    let mut new_decls: Vec<SsaDecl> = Vec::new();
    let mut sorted_targets: Vec<&String> = targets.iter().collect();
    sorted_targets.sort();
    for target in sorted_targets {
        let (params, body) = (&bodies[target].0, bodies[target].1);
        let (worker, lifted, next) = crate::lower::lower_worker(
            worker_name(target),
            params,
            body,
            &global_closures,
            counter,
        );
        counter = next;
        for lf in lifted {
            new_decls.push(SsaDecl::Func(lf));
        }
        new_decls.push(SsaDecl::Func(worker));
    }
    ssa.decls.extend(new_decls);

    // Rewrite every saturated chain to a target (including inside the freshly
    // generated workers, so a worker's self-recursion becomes a direct call).
    for func in all_funcs(ssa) {
        rewrite_func(func, &arity, &targets);
    }
}

/// Peel nested lambdas (skipping erased type lambdas) from a top-level function
/// body, returning the parameter names in application order and the
/// fully-applied body.
fn peel_lambdas(body: &AnfBody) -> (Vec<String>, &AnfBody) {
    let mut params = Vec::new();
    let mut cur = body;
    while let Some((param, inner)) = crate::lower::top_level_lambda(cur) {
        params.push(param.clone());
        cur = inner;
    }
    (params, cur)
}

/// Mutable iterator over every function in the module (top-level decls and the
/// entry point), flattening rec groups.
fn all_funcs(ssa: &mut SsaModule) -> Vec<&mut SsaFunc> {
    let mut out: Vec<&mut SsaFunc> = Vec::new();
    for decl in &mut ssa.decls {
        match decl {
            SsaDecl::Func(f) => out.push(f),
            SsaDecl::RecGroup(fs) => out.extend(fs.iter_mut()),
        }
    }
    out.push(&mut ssa.entry);
    out
}

/// Count how many times each register is *used* across a function (operands of
/// instructions, terminators, and phi branches). A binding's own `dest` is not
/// a use.
fn use_counts(func: &SsaFunc) -> FxHashMap<String, usize> {
    let mut counts: FxHashMap<String, usize> = FxHashMap::default();
    let mut bump = |v: &SsaValue| {
        if let SsaValue::Reg(r) = v {
            *counts.entry(r.clone()).or_insert(0) += 1;
        }
    };
    for block in &func.blocks {
        for instr in &block.instructions {
            for v in op_values(&instr.op) {
                bump(v);
            }
        }
        match &block.terminator {
            SsaTerminator::Return(v) => bump(v),
            SsaTerminator::Branch { cond, .. } => bump(cond),
            SsaTerminator::Jump(_) => {}
        }
    }
    counts
}

/// Every `SsaValue` operand referenced by an op.
fn op_values(op: &SsaOp) -> Vec<&SsaValue> {
    match op {
        SsaOp::ApplyClosure { closure, arg, .. } => vec![closure, arg],
        SsaOp::CallKnown { args, .. } => args.iter().collect(),
        SsaOp::HostPrint { value } | SsaOp::HostOp { value, .. } => vec![value],
        SsaOp::MakeClosure { captures, .. } => captures.iter().collect(),
        SsaOp::LoadCapture { closure, .. } => vec![closure],
        SsaOp::CallGlobal { .. } | SsaOp::Error => vec![],
        SsaOp::TyApp { poly, .. } => vec![poly],
        SsaOp::Record { fields } => fields.iter().collect(),
        SsaOp::RecordUpdate { base, updates } => {
            let mut v = vec![base];
            v.extend(updates.iter().map(|(_, val)| val));
            v
        }
        SsaOp::Tuple { items } => items
            .iter()
            .map(|i| match i {
                SsaTupleItem::Named { value, .. } | SsaTupleItem::Positional(value) => value,
            })
            .collect(),
        SsaOp::List { elems } => elems.iter().collect(),
        SsaOp::Select { base, .. } => vec![base],
        SsaOp::Variant { value, .. } => vec![value],
        SsaOp::VariantValue { scrutinee } => vec![scrutinee],
        SsaOp::Builtin { lhs, rhs, .. } => vec![lhs, rhs],
        SsaOp::ValueEq { lhs, rhs, .. } => vec![lhs, rhs],
        SsaOp::ListPrim { args, .. }
        | SsaOp::NumPrim { args, .. }
        | SsaOp::TextPrim { args, .. } => args.iter().collect(),
        SsaOp::Coalesce { value, fallback } => vec![value, fallback],
        SsaOp::Alias { value } => vec![value],
        SsaOp::Phi { branches } => branches.iter().map(|(_, v)| v).collect(),
        SsaOp::MatchDiscriminant { scrutinee } => vec![scrutinee],
    }
}

/// `Alias { value: GlobalClosure(f) }` destinations, mapping the materialized
/// register back to the function `f`. A `GlobalClosure` operand is materialized
/// into such an alias before an `ApplyClosure`, so this recovers the known
/// callee at the head of a chain.
fn static_closures(func: &SsaFunc) -> FxHashMap<String, String> {
    let mut map = FxHashMap::default();
    for block in &func.blocks {
        for instr in &block.instructions {
            if let SsaOp::Alias {
                value: SsaValue::GlobalClosure(f),
            } = &instr.op
            {
                map.insert(instr.dest.clone(), f.clone());
            }
        }
    }
    map
}

/// Resolve a closure operand to a known top-level function name, following a
/// materialized static-closure alias.
fn resolve_head<'a>(
    closure: &'a SsaValue,
    statics: &'a FxHashMap<String, String>,
) -> Option<&'a str> {
    match closure {
        SsaValue::GlobalClosure(f) => Some(f),
        SsaValue::Reg(r) => statics.get(r).map(String::as_str),
        _ => None,
    }
}

/// A saturated known-call chain within one block.
struct Chain {
    head: String,
    /// Indices of the intermediate (non-final) applications, to delete.
    intermediate: Vec<usize>,
    /// Index of the final application, replaced by the `CallKnown`.
    final_idx: usize,
    /// Arguments in application order.
    args: Vec<SsaValue>,
}

/// Find every saturated known-call chain in a block. A chain is a root
/// `ApplyClosure` whose closure resolves to a known `f` (arity `N ≥ 2`), followed
/// — through single-use result-to-closure links — by `N` total applications. The
/// links need not be consecutive: ANF computes the next argument between two
/// applications, so the chain is followed by def-use, not position.
fn block_chains(
    block: &SsaBlock,
    arity: &FxHashMap<String, usize>,
    statics: &FxHashMap<String, String>,
    uses: &FxHashMap<String, usize>,
) -> Vec<Chain> {
    // reg -> index of the apply whose closure is that reg.
    let mut closure_use: FxHashMap<&str, usize> = FxHashMap::default();
    for (idx, instr) in block.instructions.iter().enumerate() {
        if let SsaOp::ApplyClosure {
            closure: SsaValue::Reg(r),
            ..
        } = &instr.op
        {
            closure_use.insert(r.as_str(), idx);
        }
    }
    let mut chains = Vec::new();
    for (idx, instr) in block.instructions.iter().enumerate() {
        let SsaOp::ApplyClosure { closure, arg, .. } = &instr.op else {
            continue;
        };
        let Some(head) = resolve_head(closure, statics) else {
            continue;
        };
        let Some(&n) = arity.get(head) else {
            continue;
        };
        let mut args = vec![arg.clone()];
        let mut cur_dest = instr.dest.clone();
        let mut indices = vec![idx];
        let mut ok = true;
        while args.len() < n {
            // The intermediate result must flow only into the next application.
            if uses.get(&cur_dest).copied().unwrap_or(0) != 1 {
                ok = false;
                break;
            }
            let Some(&use_idx) = closure_use.get(cur_dest.as_str()) else {
                ok = false;
                break;
            };
            let SsaOp::ApplyClosure { arg: next_arg, .. } = &block.instructions[use_idx].op else {
                ok = false;
                break;
            };
            args.push(next_arg.clone());
            cur_dest = block.instructions[use_idx].dest.clone();
            indices.push(use_idx);
        }
        if ok && args.len() == n {
            let final_idx = *indices.last().unwrap();
            indices.pop();
            chains.push(Chain {
                head: head.to_string(),
                intermediate: indices,
                final_idx,
                args,
            });
        }
    }
    chains
}

/// Record every function that has a saturated known-call chain.
fn scan_targets(func: &SsaFunc, arity: &FxHashMap<String, usize>, targets: &mut FxHashSet<String>) {
    let uses = use_counts(func);
    let statics = static_closures(func);
    for block in &func.blocks {
        for chain in block_chains(block, arity, &statics, &uses) {
            targets.insert(chain.head);
        }
    }
}

/// Rewrite every saturated chain to a `targets` callee into a `CallKnown` of the
/// callee's worker, deleting the now-dead intermediate applications.
fn rewrite_func(func: &mut SsaFunc, arity: &FxHashMap<String, usize>, targets: &FxHashSet<String>) {
    let uses = use_counts(func);
    let statics = static_closures(func);
    for block in &mut func.blocks {
        let chains: Vec<Chain> = block_chains(block, arity, &statics, &uses)
            .into_iter()
            .filter(|c| targets.contains(&c.head))
            .collect();
        if chains.is_empty() {
            continue;
        }
        // Commit chains greedily, skipping any whose indices a prior chain has
        // already claimed, so an index is never both deleted (intermediate) and
        // replaced (final).
        let mut claimed: FxHashSet<usize> = FxHashSet::default();
        let mut delete: FxHashSet<usize> = FxHashSet::default();
        let mut replace: FxHashMap<usize, (String, Vec<SsaValue>)> = FxHashMap::default();
        for chain in chains {
            let indices: Vec<usize> = chain
                .intermediate
                .iter()
                .copied()
                .chain(std::iter::once(chain.final_idx))
                .collect();
            if indices.iter().any(|i| claimed.contains(i)) {
                continue;
            }
            claimed.extend(indices);
            for idx in chain.intermediate {
                delete.insert(idx);
            }
            replace.insert(chain.final_idx, (worker_name(&chain.head), chain.args));
        }
        let mut out: Vec<SsaInstr> = Vec::with_capacity(block.instructions.len());
        for (idx, instr) in block.instructions.iter().enumerate() {
            if delete.contains(&idx) {
                continue;
            }
            if let Some((func_name, args)) = replace.remove(&idx) {
                out.push(SsaInstr {
                    dest: instr.dest.clone(),
                    op: SsaOp::CallKnown {
                        func: func_name,
                        args,
                        tail: false,
                    },
                });
            } else {
                out.push(instr.clone());
            }
        }
        block.instructions = out;
    }
}

/// Scalar-replace tuples that never escape: a `Tuple` whose result is used only
/// as the base of constant-slot `Select`s is built and immediately destructured
/// (the multi-parameter clause `n acc => …` desugars to `match (n, acc) { … }`).
/// Replace each such `Select` with a direct alias to the tuple element and drop
/// the dead `Tuple`, eliminating the per-call arg-tuple allocation that survives
/// uncurrying inside the worker body.
pub fn scalar_replace_tuples(ssa: &mut SsaModule) {
    for func in all_funcs(ssa) {
        scalar_replace_tuples_func(func);
    }
}

fn scalar_replace_tuples_func(func: &mut SsaFunc) {
    // Tuple definitions: reg -> the element values (by canonical slot).
    let mut tuple_defs: FxHashMap<String, Vec<SsaValue>> = FxHashMap::default();
    for block in &func.blocks {
        for instr in &block.instructions {
            if let SsaOp::Tuple { items } = &instr.op {
                let elems = items
                    .iter()
                    .map(|i| match i {
                        SsaTupleItem::Named { value, .. } | SsaTupleItem::Positional(value) => {
                            value.clone()
                        }
                    })
                    .collect();
                tuple_defs.insert(instr.dest.clone(), elems);
            }
        }
    }
    if tuple_defs.is_empty() {
        return;
    }

    // Disqualify any tuple used as anything other than a valid constant-slot
    // `Select` base (escapes through another op, a terminator, or a phi).
    let mut disqualified: FxHashSet<String> = FxHashSet::default();
    let mark = |v: &SsaValue, disq: &mut FxHashSet<String>| {
        if let SsaValue::Reg(r) = v
            && tuple_defs.contains_key(r)
        {
            disq.insert(r.clone());
        }
    };
    for block in &func.blocks {
        for instr in &block.instructions {
            match &instr.op {
                SsaOp::Select {
                    base: SsaValue::Reg(r),
                    slot,
                } if tuple_defs.contains_key(r) => {
                    if *slot >= tuple_defs[r].len() {
                        disqualified.insert(r.clone());
                    }
                }
                other => {
                    for v in op_values(other) {
                        mark(v, &mut disqualified);
                    }
                }
            }
        }
        match &block.terminator {
            SsaTerminator::Return(v) => mark(v, &mut disqualified),
            SsaTerminator::Branch { cond, .. } => mark(cond, &mut disqualified),
            SsaTerminator::Jump(_) => {}
        }
    }
    tuple_defs.retain(|r, _| !disqualified.contains(r));
    if tuple_defs.is_empty() {
        return;
    }

    for block in &mut func.blocks {
        let mut out: Vec<SsaInstr> = Vec::with_capacity(block.instructions.len());
        for instr in block.instructions.drain(..) {
            match &instr.op {
                SsaOp::Tuple { .. } if tuple_defs.contains_key(&instr.dest) => {
                    // Dead after scalar replacement; drop.
                }
                SsaOp::Select {
                    base: SsaValue::Reg(r),
                    slot,
                } if tuple_defs.contains_key(r) => {
                    let value = tuple_defs[r][*slot].clone();
                    out.push(SsaInstr {
                        dest: instr.dest,
                        op: SsaOp::Alias { value },
                    });
                }
                _ => out.push(instr),
            }
        }
        block.instructions = out;
    }
}
