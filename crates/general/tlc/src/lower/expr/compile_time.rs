use rustc_hash::FxHashMap;

use zutai_hir::BindingId;
use zutai_thir::{
    ThirDeclKind, ThirExprId, ThirExprKind, ThirPatId, ThirPatKind, ThirRecordField,
    ThirRecordPatField, ThirTupleItem, ThirTuplePatItem,
};

use crate::ir::TlcExprId;

use super::*;
use crate::lower::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn code_substitution(&self, binding: BindingId) -> Option<ThirExprId> {
        self.code_frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(&binding).copied())
    }

    pub(super) fn lower_code_expansion(&mut self, expansion: CodeExpansion) -> TlcExprId {
        let saved = std::mem::replace(&mut self.code_frames, expansion.frames);
        let value = self.lower_expr(expansion.value);
        self.code_frames = saved;
        value
    }

    pub(super) fn resolve_code_expr(&self, id: ThirExprId) -> Option<CodeExpansion> {
        match self.eval_compile_time(id, self.code_frames.clone(), CODE_EXPANSION_FUEL)? {
            CompileTimeValue::Code(code) => Some(code),
            CompileTimeValue::Closure(_) | CompileTimeValue::Expr(_) => None,
        }
    }

    pub(super) fn eval_compile_time(
        &self,
        id: ThirExprId,
        frames: Vec<FxHashMap<BindingId, ThirExprId>>,
        fuel: u16,
    ) -> Option<CompileTimeValue> {
        let Some(fuel) = fuel.checked_sub(1) else {
            self.recipe_fuel_exhausted.set(true);
            return None;
        };
        match self.thir.expr_arena[id].kind.clone() {
            ThirExprKind::Quote(value) => {
                Some(CompileTimeValue::Code(CodeExpansion { value, frames }))
            }
            ThirExprKind::BindingRef { binding, .. } => {
                if let Some(value) = frames
                    .iter()
                    .rev()
                    .find_map(|frame| frame.get(&binding).copied())
                {
                    return self.eval_compile_time(value, frames, fuel);
                }
                self.thir.decls.iter().find_map(|&decl_id| {
                    let decl = &self.thir.decl_arena[decl_id];
                    if decl.binding != binding {
                        return None;
                    }
                    match &decl.kind {
                        ThirDeclKind::Value { value, .. } => {
                            self.eval_compile_time(*value, frames.clone(), fuel)
                        }
                        ThirDeclKind::Function { clauses, .. } if clauses.len() == 1 => {
                            let clause = &clauses[0];
                            Some(CompileTimeValue::Closure(CodeClosure {
                                params: clause.patterns.clone(),
                                body: clause.body,
                                frames: frames.clone(),
                            }))
                        }
                        _ => None,
                    }
                })
            }
            ThirExprKind::Lambda { params, body } => Some(CompileTimeValue::Closure(CodeClosure {
                params,
                body,
                frames,
            })),
            ThirExprKind::Apply { func, arg, .. } => {
                let CompileTimeValue::Closure(mut closure) =
                    self.eval_compile_time(func, frames, fuel)?
                else {
                    return None;
                };
                let pattern = closure.params.first().copied()?;
                let mut frame = FxHashMap::default();
                if !self.bind_compile_time_pattern(pattern, arg, &mut frame) {
                    return None;
                }
                closure.frames.push(frame);
                closure.params.remove(0);
                if closure.params.is_empty() {
                    self.eval_compile_time(closure.body, closure.frames, fuel)
                } else {
                    Some(CompileTimeValue::Closure(closure))
                }
            }
            ThirExprKind::Block { bindings, result } => {
                let mut frame = FxHashMap::default();
                for binding in bindings {
                    frame.insert(binding.binding, binding.value);
                }
                let mut frames = frames;
                frames.push(frame);
                self.eval_compile_time(result, frames, fuel)
            }
            ThirExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond = self.eval_compile_time(cond, frames.clone(), fuel)?;
                match self.compile_time_bool(cond) {
                    Some(true) => self.eval_compile_time(then_branch, frames, fuel),
                    Some(false) => self.eval_compile_time(else_branch, frames, fuel),
                    None => None,
                }
            }
            ThirExprKind::Match { scrutinee, arms } => {
                // Try each arm in order; the matcher reduces the scrutinee (and
                // its sub-expressions) structurally. A matched arm's body is
                // evaluated in the match's own frames extended with the pattern
                // bindings — pattern-leaf sub-exprs are bound raw and resolved
                // lazily, mirroring the `Apply` binding convention. Recipes match
                // on closed literal configs, so this frame flattening is exact.
                for arm in &arms {
                    if arm.guard.is_some() {
                        return None;
                    }
                    let pattern = *arm.patterns.first()?;
                    let mut frame = FxHashMap::default();
                    if self
                        .match_compile_time_pattern(pattern, scrutinee, &frames, &mut frame, fuel)?
                    {
                        let mut arm_frames = frames.clone();
                        arm_frames.push(frame);
                        return self.eval_compile_time(arm.body, arm_frames, fuel);
                    }
                }
                None
            }
            ThirExprKind::Perform { .. }
            | ThirExprKind::Handle { .. }
            | ThirExprKind::Resume { .. }
            | ThirExprKind::Import(_) => None,
            _ => Some(CompileTimeValue::Expr(CodeExpansion { value: id, frames })),
        }
    }

    pub(super) fn bind_compile_time_pattern(
        &self,
        pattern: ThirPatId,
        value: ThirExprId,
        frame: &mut FxHashMap<BindingId, ThirExprId>,
    ) -> bool {
        match self.thir.pat_arena[pattern].kind {
            ThirPatKind::Bind(binding) => {
                frame.insert(binding, value);
                true
            }
            ThirPatKind::Wildcard => true,
            _ => false,
        }
    }

    /// Structurally match a THIR pattern against a compile-time expression for
    /// the recipe reducer. Returns `Some(true)` on a match (binding sub-exprs
    /// into `frame`), `Some(false)` on a decisive non-match (try the next arm),
    /// and `None` when the match is undecidable at compile time (abort the whole
    /// reduction and fall back to the structural synthesizers). Bound leaves
    /// capture the raw scrutinee sub-expr id, resolved lazily in the arm frames
    /// — exact for the closed literal configs recipes match on.
    pub(super) fn match_compile_time_pattern(
        &self,
        pattern: ThirPatId,
        expr_id: ThirExprId,
        frames: &[FxHashMap<BindingId, ThirExprId>],
        frame: &mut FxHashMap<BindingId, ThirExprId>,
        fuel: u16,
    ) -> Option<bool> {
        let pat_kind = self.thir.pat_arena[pattern].kind.clone();
        // Irrefutable leaves need no scrutinee reduction.
        match &pat_kind {
            ThirPatKind::Error => return None,
            ThirPatKind::Wildcard => return Some(true),
            ThirPatKind::Bind(binding) => {
                frame.insert(*binding, expr_id);
                return Some(true);
            }
            _ => {}
        }
        // Refutable patterns require the scrutinee's structural head.
        let CompileTimeValue::Expr(scrut) =
            self.eval_compile_time(expr_id, frames.to_vec(), fuel)?
        else {
            return None;
        };
        let value = self.thir.expr_arena[scrut.value].kind.clone();
        let sub = scrut.frames.as_slice();
        match (pat_kind, value) {
            (ThirPatKind::True, ThirExprKind::True) => Some(true),
            (ThirPatKind::True, ThirExprKind::False) => Some(false),
            (ThirPatKind::False, ThirExprKind::False) => Some(true),
            (ThirPatKind::False, ThirExprKind::True) => Some(false),
            (ThirPatKind::Integer(p), ThirExprKind::Integer(v)) => Some(p == v),
            (ThirPatKind::Float(p), ThirExprKind::Float(v)) => Some(p.to_bits() == v.to_bits()),
            (ThirPatKind::Posit(p), ThirExprKind::Posit(v)) => Some(p == v),
            (ThirPatKind::String(p), ThirExprKind::String(v)) => Some(p == v),
            (ThirPatKind::Atom(p), ThirExprKind::Atom(v)) => Some(p == v),
            (ThirPatKind::ListNil, ThirExprKind::List(items)) => Some(items.is_empty()),
            (ThirPatKind::Record(pat_fields), ThirExprKind::Record(val_fields)) => {
                self.match_compile_time_record(&pat_fields, &val_fields, sub, frame, fuel)
            }
            (
                ThirPatKind::TaggedValue {
                    tag: ptag,
                    payload: pat_fields,
                },
                ThirExprKind::TaggedValue { tag: vtag, payload },
            ) => {
                if ptag != vtag {
                    return Some(false);
                }
                // A tagged value's payload is a single expr; recipes tag record
                // payloads, so resolve it as a record to match the field patterns.
                let CompileTimeValue::Expr(pl) =
                    self.eval_compile_time(payload, sub.to_vec(), fuel)?
                else {
                    return None;
                };
                let ThirExprKind::Record(val_fields) = self.thir.expr_arena[pl.value].kind.clone()
                else {
                    return None;
                };
                self.match_compile_time_record(&pat_fields, &val_fields, &pl.frames, frame, fuel)
            }
            (ThirPatKind::Tuple(pat_items), ThirExprKind::Tuple(val_items)) => {
                self.match_compile_time_tuple(&pat_items, &val_items, sub, frame, fuel)
            }
            // A nullary-variant (atom) pattern and a payload-carrying variant
            // value — or vice versa — are distinct constructors of the same
            // union. Both scrutinee and pattern have decided structural heads, so
            // this is a decisive non-match: try the next arm rather than stalling
            // the whole reduction (which would strand a structurally recursive
            // recipe and fall through to a broken witness).
            (ThirPatKind::Atom(_), ThirExprKind::TaggedValue { .. })
            | (ThirPatKind::TaggedValue { .. }, ThirExprKind::Atom(_)) => Some(false),
            _ => None,
        }
    }

    pub(super) fn match_compile_time_record(
        &self,
        pat_fields: &[ThirRecordPatField],
        val_fields: &[ThirRecordField],
        frames: &[FxHashMap<BindingId, ThirExprId>],
        frame: &mut FxHashMap<BindingId, ThirExprId>,
        fuel: u16,
    ) -> Option<bool> {
        for pf in pat_fields {
            let vf = val_fields.iter().find(|f| f.name == pf.name)?;
            if !self.match_compile_time_pattern(pf.pattern, vf.value, frames, frame, fuel)? {
                return Some(false);
            }
        }
        Some(true)
    }

    pub(super) fn match_compile_time_tuple(
        &self,
        pat_items: &[ThirTuplePatItem],
        val_items: &[ThirTupleItem],
        frames: &[FxHashMap<BindingId, ThirExprId>],
        frame: &mut FxHashMap<BindingId, ThirExprId>,
        fuel: u16,
    ) -> Option<bool> {
        if pat_items.len() != val_items.len() {
            return Some(false);
        }
        for (pi, vi) in pat_items.iter().zip(val_items) {
            let (sub_pat, sub_expr) = match (pi, vi) {
                (ThirTuplePatItem::Positional(p), ThirTupleItem::Positional(v)) => (*p, *v),
                (
                    ThirTuplePatItem::Named {
                        name: pn,
                        pattern: p,
                        ..
                    },
                    ThirTupleItem::Named {
                        name: vn, value: v, ..
                    },
                ) if pn == vn => (*p, *v),
                _ => return None,
            };
            if !self.match_compile_time_pattern(sub_pat, sub_expr, frames, frame, fuel)? {
                return Some(false);
            }
        }
        Some(true)
    }

    pub(super) fn compile_time_bool(&self, value: CompileTimeValue) -> Option<bool> {
        let CompileTimeValue::Expr(value) = value else {
            return None;
        };
        match self.thir.expr_arena[value.value].kind {
            ThirExprKind::True => Some(true),
            ThirExprKind::False => Some(false),
            _ => None,
        }
    }
}
