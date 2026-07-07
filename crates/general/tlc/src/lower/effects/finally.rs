//! Desugar `finally` handler clauses before effect elaboration.
//!
//! A `handle expr with { value = v; finally = t; op = … }` runs the teardown `t`
//! once, in the outer effect row, after the handled computation reduces to its
//! final value — on normal completion *and* on handler abort. The reference
//! interpreter implements this with `run_finally` (`eval_tlc::effects`); natively
//! it desugars to
//!
//! ```text
//! let r = (handle expr with { value = v; op = … }) in [ r; t; r ]
//! ```
//!
//! The inner `handle` (now without `finally`) is elaborated/reified by the normal
//! pipeline; the left-to-right `Sequence` runs `t` for its effects (result
//! discarded) and yields `r`. Because the binding sits *outside* the inner
//! handle, `t`'s own effects are charged to the enclosing (outer) handler, exactly
//! as the interpreter forwards them.

use rustc_hash::FxHashSet;
use zutai_hir::BindingId;

use crate::ir::{TlcDecl, TlcExpr, TlcModule};
use crate::monomorphize::reachable_exprs;

impl TlcModule {
    /// Rewrite every reachable `handle … with { finally = t; … }` into the
    /// teardown-sequencing desugaring above, clearing the `finally` clause so the
    /// residual-effect gate and the elaborator see an ordinary handle.
    pub fn desugar_finally(&mut self) {
        let mut next_fresh = u32::MAX;
        let mut used: FxHashSet<BindingId> = FxHashSet::default();
        for (_, decl) in self.decl_arena.iter() {
            match decl {
                TlcDecl::Value { binding, .. } | TlcDecl::TypeAlias { binding, .. } => {
                    used.insert(*binding);
                }
            }
        }
        // Iterate to a fixpoint: desugaring a handle exposes its inner handle,
        // and nested `finally`s are rewritten on subsequent passes.
        loop {
            let target = reachable_exprs(self).into_iter().find(|id| {
                matches!(
                    self.expr_arena[*id],
                    TlcExpr::Handle {
                        finally: Some(_),
                        ..
                    }
                )
            });
            let Some(handle_id) = target else { break };
            let TlcExpr::Handle {
                expr,
                value,
                finally: Some(teardown),
                ops,
            } = self.expr_arena[handle_id].clone()
            else {
                break;
            };

            let result_ty = self.expr_types[&handle_id];
            let span = self.spans.get(&handle_id).copied().unwrap_or_default();

            // Inner computation without the finally clause. A finally-only
            // handle has no operation or value behavior, so avoid creating an
            // empty residual `Handle` node that the backend would later reject.
            let inner = if ops.is_empty() && value.is_none() {
                expr
            } else {
                let inner = self.expr_arena.alloc(TlcExpr::Handle {
                    expr,
                    value,
                    finally: None,
                    ops,
                });
                self.expr_types.insert(inner, result_ty);
                self.spans.insert(inner, span);
                inner
            };

            // r : result; body = [ r; teardown; r ]. The leading `r` forces
            // the handled callback value to settle before teardown. The final
            // `r` returns the memoized result after cleanup.
            let r = loop {
                let b = BindingId(next_fresh);
                next_fresh = next_fresh.saturating_sub(1);
                if used.insert(b) {
                    break b;
                }
            };
            let force_r_var = self.expr_arena.alloc(TlcExpr::Var(r));
            self.expr_types.insert(force_r_var, result_ty);
            self.spans.insert(force_r_var, span);
            let result_r_var = self.expr_arena.alloc(TlcExpr::Var(r));
            self.expr_types.insert(result_r_var, result_ty);
            self.spans.insert(result_r_var, span);
            let seq =
                self.expr_arena
                    .alloc(TlcExpr::Sequence(vec![force_r_var, teardown, result_r_var]));
            self.expr_types.insert(seq, result_ty);
            self.spans.insert(seq, span);

            // Overwrite the original handle node with the let so its parents see
            // the desugared form.
            self.expr_arena[handle_id] = TlcExpr::Let {
                binding: r,
                ty: result_ty,
                value: inner,
                body: seq,
            };
        }
    }
}
