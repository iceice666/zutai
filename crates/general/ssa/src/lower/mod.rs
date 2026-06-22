//! ANF → SSA lowering.
//!
//! Converts flat ANF bindings into basic blocks with phi nodes at join points.

use crate::*;
use rustc_hash::FxHashSet;
use zutai_anf::{AnfAtom, AnfBody, AnfDecl, AnfExpr, AnfModule};

mod expr;
mod freevars;
mod match_;

use self::expr::*;
use self::freevars::*;

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
    global_closures: FxHashSet<String>,
}

impl Ctx {
    fn new(global_closures: FxHashSet<String>) -> Self {
        Ctx {
            fresh: Fresh::default(),
            lambdas: Vec::new(),
            global_closures,
        }
    }
}

// ── Module-level lowering ──────────────────────────────────────────────────────

/// Lower a complete ANF module into SSA form.
pub fn lower_anf(module: &AnfModule) -> SsaModule {
    let closure_exports = collect_closure_exports(module);
    let global_closures: FxHashSet<String> = closure_exports.iter().cloned().collect();
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
    let mut seen = FxHashSet::default();
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
