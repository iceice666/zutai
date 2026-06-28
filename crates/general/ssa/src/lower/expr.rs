//! ANF → SSA lowering.
//!
//! Converts flat ANF bindings into basic blocks with phi nodes at join points.

use crate::*;
use rustc_hash::FxHashSet;
use zutai_anf::{AnfAtom, AnfBody, AnfExpr, AnfTupleItem};

use super::*;

use super::freevars::*;
use super::match_::*;

// ── Converting ANF atoms to SSA values ─────────────────────────────────────────

pub(super) fn atom_to_value(atom: &AnfAtom, globals: &FxHashSet<String>) -> SsaValue {
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

pub(super) fn materialize_value(val: SsaValue, fb: &mut FuncBuilder, ctx: &mut Ctx) -> SsaValue {
    match val {
        SsaValue::Global(name) => {
            let dest = ctx.fresh.next_label("global");
            fb.push(SsaInstr {
                dest: dest.clone(),
                op: SsaOp::CallGlobal { name },
            });
            SsaValue::Reg(dest)
        }
        value @ (SsaValue::GlobalClosure(_) | SsaValue::Lit(DfLit::Text(_) | DfLit::Atom(_))) => {
            let dest = ctx.fresh.next_label("static");
            fb.push(SsaInstr {
                dest: dest.clone(),
                op: SsaOp::Alias { value },
            });
            SsaValue::Reg(dest)
        }
        other => other,
    }
}

pub(super) fn lower_atom_value(atom: &AnfAtom, fb: &mut FuncBuilder, ctx: &mut Ctx) -> SsaValue {
    materialize_value(atom_to_value(atom, &ctx.global_closures), fb, ctx)
}

// ── Converting ANF tuple items ─────────────────────────────────────────────────

pub(super) fn tuple_item_to_ssa(
    item: &AnfTupleItem,
    fb: &mut FuncBuilder,
    ctx: &mut Ctx,
) -> SsaTupleItem {
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
pub(super) fn lower_body(body: &AnfBody, fb: &mut FuncBuilder, ctx: &mut Ctx) -> SsaValue {
    for (name, expr) in &body.bindings {
        lower_expr(name, expr, fb, ctx);
    }
    lower_atom_value(&body.result, fb, ctx)
}

// ── Expression lowering ────────────────────────────────────────────────────────

/// Lower an ANF expression, writing the result into register `dest`.
pub(super) fn lower_expr(dest: &str, expr: &AnfExpr, fb: &mut FuncBuilder, ctx: &mut Ctx) {
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
            tail: false,
        },
        AnfExpr::HostPrint { value } => SsaOp::HostPrint {
            value: lower_atom_value(value, fb, ctx),
        },
        AnfExpr::HostOp { op, value } => SsaOp::HostOp {
            op: *op,
            value: lower_atom_value(value, fb, ctx),
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

        AnfExpr::ListPrim { op, args } => SsaOp::ListPrim {
            op: *op,
            args: args.iter().map(|a| lower_atom_value(a, fb, ctx)).collect(),
        },
        AnfExpr::NumPrim { op, args } => SsaOp::NumPrim {
            op: *op,
            args: args.iter().map(|a| lower_atom_value(a, fb, ctx)).collect(),
        },
        AnfExpr::TextPrim { op, args } => SsaOp::TextPrim {
            op: *op,
            args: args.iter().map(|a| lower_atom_value(a, fb, ctx)).collect(),
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
pub(super) fn lower_lambda_function(
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
