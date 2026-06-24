//! Host-capability entry boundary (v2 `docs/v2_spec/02-host-capabilities.md`).
//!
//! The idiomatic entry shape declares the capabilities a program needs as its
//! entry parameter and lets the host supply them:
//!
//! ```text
//! main :: { fs : FsRead; } -> Text ! { fs.read : Path -> Text; }
//!   = caps => readConfig caps.fs;
//! main
//! ```
//!
//! Capability values are opaque and *advisory* — obtained only at this boundary,
//! never constructed by source, and never inspected by `perform` (the effect
//! row, not the value, authorizes the operation). So the host boundary
//! synthesizes a placeholder token for each requested capability and applies the
//! entry to it, turning a `Caps -> Result` entry into the `Result` the backend
//! can render. The granted host operations in `Result` then lower to Dataflow
//! Core `HostOp` nodes as usual. A non-capability entry function is left
//! untouched and stays rejected as "entry returns a function".

use zutai_hir::ir::HOST_CAPABILITY_TYPE_NAMES;

use crate::ir::{Literal, Row, TlcExpr, TlcExprId, TlcModule, TlcType, TlcTypeId};

impl TlcModule {
    /// Apply the entry to synthesized advisory capability tokens while its
    /// leading parameters are host capabilities (an `Opaque` capability type, or
    /// a closed record whose every field is one). Iterates so curried capability
    /// parameters (`FsRead -> Env -> …`) are all supplied; stops at the first
    /// non-capability parameter, leaving any genuinely-unsuppliable entry as the
    /// function the backend already rejects.
    pub fn apply_entry_capabilities(&mut self) {
        let Some(mut entry) = self.final_expr else {
            return;
        };
        while let Some(&entry_ty) = self.expr_types.get(&entry) {
            let TlcType::Fun(param_ty, result_ty, _) = self.type_arena[entry_ty] else {
                break;
            };
            let Some(caps) = self.synthesize_capabilities(param_ty) else {
                break;
            };
            let app = self.expr_arena.alloc(TlcExpr::App(entry, caps));
            self.expr_types.insert(app, result_ty);
            if let Some(span) = self.spans.get(&entry).copied() {
                self.spans.insert(app, span);
            }
            entry = app;
        }
        self.final_expr = Some(entry);
    }

    /// Synthesize a value for a capability parameter: a token for a single
    /// `Opaque` capability, or a record of tokens for a closed record whose every
    /// field is a capability. Returns `None` for anything else.
    fn synthesize_capabilities(&mut self, param_ty: TlcTypeId) -> Option<TlcExprId> {
        match self.type_arena[param_ty].clone() {
            TlcType::Opaque(name) if is_capability(&name) => Some(self.capability_token(param_ty)),
            TlcType::Record(row) => {
                let fields = closed_record_fields(&row)?;
                if fields.is_empty() {
                    return None;
                }
                let mut token_fields = Vec::with_capacity(fields.len());
                for (label, field_ty) in &fields {
                    match &self.type_arena[*field_ty] {
                        TlcType::Opaque(name) if is_capability(name) => {}
                        _ => return None,
                    }
                    token_fields.push((label.clone(), self.capability_token(*field_ty)));
                }
                let record = self.expr_arena.alloc(TlcExpr::Record(token_fields));
                self.expr_types.insert(record, param_ty);
                Some(record)
            }
            _ => None,
        }
    }

    /// An advisory, never-inspected capability token: a `0` literal stamped with
    /// the capability's opaque type. DC validation is a no-op for `Lit` nodes, so
    /// the literal kind need not match the opaque type; codegen lowers it to an
    /// `i64 0` the program never reads.
    fn capability_token(&mut self, cap_ty: TlcTypeId) -> TlcExprId {
        let token = self.expr_arena.alloc(TlcExpr::Lit(Literal::Int(0)));
        self.expr_types.insert(token, cap_ty);
        token
    }
}

fn is_capability(name: &str) -> bool {
    HOST_CAPABILITY_TYPE_NAMES.contains(&name)
}

/// The `(label, ty)` fields of a closed record row, or `None` if the row is open
/// (`RVar` tail) — an open capability record cannot be fully supplied.
fn closed_record_fields(row: &Row) -> Option<Vec<(String, TlcTypeId)>> {
    let mut fields = Vec::new();
    let mut cur = row;
    loop {
        match cur {
            Row::REmpty => return Some(fields),
            Row::RExtend {
                label, ty, tail, ..
            } => {
                fields.push((label.clone(), *ty));
                cur = tail;
            }
            Row::RVar(_) => return None,
        }
    }
}
