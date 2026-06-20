//! Exhaustiveness and reachability checking for `match` expressions and
//! multi-clause functions.
//!
//! Both checks are driven by Maranget's usefulness algorithm (see "Warnings for
//! pattern matching", JFP 2007) over a *pattern matrix*. A row is one
//! clause/arm; a column is one scrutinee position (one column for `match`, N for
//! an N-ary function). The algorithm is parameterized by a tag-based
//! constructor model derived from each column's type:
//!
//! * **Exhaustiveness** — the all-wildcard row is *useful* against the matrix of
//!   unguarded rows exactly when some value is matched by no clause. The
//!   usefulness witness is a rendered example of that unmatched value.
//! * **Reachability** — clause `i` is unreachable when its own row is *not*
//!   useful against the preceding unguarded rows.
//!
//! Guarded clauses never enter a "covering" matrix (a guard may fail, so the
//! clause cannot be assumed to match), but a guarded clause is still tested for
//! its own reachability.

use std::collections::{HashMap, HashSet};

use zutai_syntax::Span;

use crate::diagnostic::{ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    RowTail, ThirClause, ThirPatId, ThirPatKind, ThirRecordPatField, ThirTuplePatItem, TypeId,
    TypeKind, TypeTupleItem, UnionVariant,
};

use super::Lowerer;

/// A constructor identity used to group and compare patterns. Two patterns
/// "test the same constructor" iff their `Ctor`s are equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Ctor {
    Bool(bool),
    /// A bare atom, stored without the leading `#` (matching `TypeKind::Atom`).
    Atom(String),
    IntLit(i64),
    /// Float literal compared by bit pattern so `Ctor` can be `Eq + Hash`.
    FloatLit(u64),
    StrLit(String),
    /// The single constructor of a plain tuple or record (a product type).
    Struct,
    /// A tagged union member: a tuple led by `#tag`, carrying the remaining
    /// fields as payload. Stored without the leading `#`.
    Tagged(String),
    /// A tagged union member with tuple-shaped positional payload.
    TaggedTuple(String),
    /// `#none` of an `Optional`.
    OptNone,
    /// `#some` of an `Optional`, carrying the wrapped value.
    OptSome,
    /// `#absent` of a `Maybe`.
    MaybeAbsent,
    /// `#present` of a `Maybe`, carrying the present value.
    MaybePresent,
}

/// A pattern deconstructed into the matrix algebra: either a catch-all, or a
/// constructor applied to already-ordered sub-patterns.
#[derive(Debug, Clone)]
enum DeconPat {
    Wild,
    Ctor { tag: Ctor, fields: Vec<DeconPat> },
}

impl DeconPat {
    fn nullary(tag: Ctor) -> Self {
        DeconPat::Ctor {
            tag,
            fields: Vec::new(),
        }
    }

    fn is_wild(&self) -> bool {
        matches!(self, DeconPat::Wild)
    }
}

/// The set of constructors of a column type, with each constructor's field
/// types. `ctors == None` marks an infinite/opaque column (`Int`, `Text`,
/// functions, type variables, …) where only a wildcard completes coverage.
struct Signature {
    ctors: Option<Vec<SigCtor>>,
}

struct SigCtor {
    ctor: Ctor,
    /// Field (sub-column) types, in canonical order; `len()` is the arity.
    fields: Vec<TypeId>,
}

impl SigCtor {
    fn nullary(ctor: Ctor) -> Self {
        SigCtor {
            ctor,
            fields: Vec::new(),
        }
    }
}

impl<'hir> Lowerer<'hir> {
    /// Check a set of clauses (a `match` or a multi-clause function) for
    /// exhaustiveness and unreachable arms. `col_tys` is one type per scrutinee
    /// position; `span` locates the whole construct for a non-exhaustive report.
    pub(super) fn check_match_exhaustiveness(
        &mut self,
        clauses: &[ThirClause],
        col_tys: &[TypeId],
        span: Span,
    ) {
        let width = col_tys.len();
        if width == 0 || clauses.is_empty() {
            return;
        }

        // Resolve every column up front so the matrix algebra sees concrete kinds.
        let mut cols = Vec::with_capacity(width);
        for &ty in col_tys {
            cols.push(self.resolve_alias(ty, &mut HashSet::new(), span));
        }

        // Don't pile non-exhaustive errors on top of an existing type error.
        if cols
            .iter()
            .any(|&c| matches!(self.ty(c).kind, TypeKind::Error))
        {
            return;
        }

        // Deconstruct every clause into a row. Bail entirely if any clause has
        // the wrong arity (a FunctionClauseArityMismatch / MatchArmPatternCountMismatch
        // was already emitted; the matrix would be malformed).
        let mut rows: Vec<Vec<DeconPat>> = Vec::with_capacity(clauses.len());
        let mut guarded: Vec<bool> = Vec::with_capacity(clauses.len());
        let mut spans: Vec<Span> = Vec::with_capacity(clauses.len());
        for clause in clauses {
            if clause.patterns.len() != width {
                return;
            }
            let row = clause
                .patterns
                .iter()
                .zip(&cols)
                .map(|(&pat, &ty)| self.decon_pattern(pat, ty))
                .collect();
            rows.push(row);
            guarded.push(clause.guard.is_some());
            spans.push(clause.span);
        }

        // Reachability: clause i is unreachable if its row is not useful against
        // the preceding *unguarded* rows.
        for i in 0..rows.len() {
            let prefix: Vec<Vec<DeconPat>> = (0..i)
                .filter(|&j| !guarded[j])
                .map(|j| rows[j].clone())
                .collect();
            if self.is_useful(&prefix, &rows[i], &cols, span).is_none() {
                self.diagnostics.push(ThirDiagnostic {
                    kind: ThirDiagnosticKind::UnreachableMatchArm,
                    span: spans[i],
                });
            }
        }

        // Exhaustiveness: is the all-wildcard row useful against the unguarded rows?
        let covering: Vec<Vec<DeconPat>> = rows
            .iter()
            .zip(&guarded)
            .filter(|&(_, &g)| !g)
            .map(|(row, _)| row.clone())
            .collect();
        let wild = vec![DeconPat::Wild; width];
        if let Some(witness) = self.is_useful(&covering, &wild, &cols, span) {
            self.diagnostics.push(ThirDiagnostic {
                kind: ThirDiagnosticKind::NonExhaustiveMatch {
                    witness: render(&witness),
                },
                span,
            });
        }
    }

    // ── Maranget usefulness ──────────────────────────────────────────────────

    /// Is vector `q` useful with respect to `matrix` (do they have N columns and
    /// is there a value matched by `q` but no row of `matrix`)? Returns a witness
    /// of such a value when useful, `None` otherwise.
    fn is_useful(
        &mut self,
        matrix: &[Vec<DeconPat>],
        q: &[DeconPat],
        col_tys: &[TypeId],
        span: Span,
    ) -> Option<Vec<DeconPat>> {
        if q.is_empty() {
            return matrix.is_empty().then(Vec::new);
        }

        match &q[0] {
            DeconPat::Ctor { tag, fields } => {
                let tag = tag.clone();
                let arity = fields.len();
                let sub_tys = self.ctor_field_tys(col_tys[0], &tag, arity, span);
                let specialized = specialize(&tag, arity, matrix);
                let mut q2 = fields.clone();
                q2.extend_from_slice(&q[1..]);
                let mut col2 = sub_tys;
                col2.extend_from_slice(&col_tys[1..]);
                self.is_useful(&specialized, &q2, &col2, span)
                    .map(|w| rewrap(&tag, arity, w))
            }
            DeconPat::Wild => {
                let sig = self.column_signature(col_tys[0], span);
                let present = present_ctors(matrix);
                let complete = match &sig.ctors {
                    Some(ctors) => {
                        !ctors.is_empty() && ctors.iter().all(|c| present.contains(&c.ctor))
                    }
                    None => false,
                };

                if complete {
                    let ctors = sig.ctors.unwrap();
                    for sc in &ctors {
                        let arity = sc.fields.len();
                        let specialized = specialize(&sc.ctor, arity, matrix);
                        let mut q2 = vec![DeconPat::Wild; arity];
                        q2.extend_from_slice(&q[1..]);
                        let mut col2 = sc.fields.clone();
                        col2.extend_from_slice(&col_tys[1..]);
                        if let Some(w) = self.is_useful(&specialized, &q2, &col2, span) {
                            return Some(rewrap(&sc.ctor, arity, w));
                        }
                    }
                    None
                } else {
                    let defaulted = default_matrix(matrix);
                    let col2 = col_tys[1..].to_vec();
                    self.is_useful(&defaulted, &q[1..], &col2, span).map(|w| {
                        let mut out = vec![missing_witness(&sig, &present)];
                        out.extend(w);
                        out
                    })
                }
            }
        }
    }

    /// Sub-column types for `tag` in column `col_ty`, padding with the error type
    /// if the signature does not describe it (e.g. a literal over an infinite type).
    fn ctor_field_tys(
        &mut self,
        col_ty: TypeId,
        tag: &Ctor,
        arity: usize,
        span: Span,
    ) -> Vec<TypeId> {
        self.column_signature(col_ty, span)
            .ctors
            .and_then(|ctors| ctors.into_iter().find(|c| &c.ctor == tag))
            .map(|sc| sc.fields)
            .unwrap_or_else(|| vec![self.error_type; arity])
    }

    // ── Constructor model ────────────────────────────────────────────────────

    /// The constructor signature of a column type.
    fn column_signature(&mut self, col_ty: TypeId, span: Span) -> Signature {
        let resolved = self.resolve_alias(col_ty, &mut HashSet::new(), span);
        let ctors = match self.ty(resolved).kind.clone() {
            TypeKind::Bool => Some(vec![
                SigCtor::nullary(Ctor::Bool(true)),
                SigCtor::nullary(Ctor::Bool(false)),
            ]),
            TypeKind::True => Some(vec![SigCtor::nullary(Ctor::Bool(true))]),
            TypeKind::False => Some(vec![SigCtor::nullary(Ctor::Bool(false))]),
            TypeKind::Atom(name) => Some(vec![SigCtor::nullary(Ctor::Atom(name))]),
            TypeKind::Optional(inner) => Some(vec![
                SigCtor::nullary(Ctor::OptNone),
                SigCtor {
                    ctor: Ctor::OptSome,
                    fields: vec![inner],
                },
            ]),
            TypeKind::Maybe(inner) => Some(vec![
                SigCtor::nullary(Ctor::MaybeAbsent),
                SigCtor {
                    ctor: Ctor::MaybePresent,
                    fields: vec![inner],
                },
            ]),
            TypeKind::Tuple(items) => Some(vec![SigCtor {
                ctor: Ctor::Struct,
                fields: items.iter().map(tuple_item_ty).collect(),
            }]),
            TypeKind::Record(fields, _) => Some(vec![SigCtor {
                ctor: Ctor::Struct,
                fields: fields.iter().map(|f| f.ty).collect(),
            }]),
            TypeKind::Union(members, RowTail::Closed) => self.union_signature(&members, span),
            // An open or row-polymorphic union has unknown extra members, so only
            // a wildcard can complete coverage.
            TypeKind::Union(_, _) => None,
            // Infinite or opaque: only a wildcard completes coverage.
            _ => None,
        };
        Signature { ctors }
    }

    /// Build the constructor set of a union from its variant list. Each variant
    /// becomes a tagged constructor; variants with record or tuple payloads expand
    /// to their field types as sub-columns.
    fn union_signature(&mut self, variants: &[UnionVariant], span: Span) -> Option<Vec<SigCtor>> {
        let mut ctors: Vec<SigCtor> = Vec::new();
        for variant in variants {
            let (ctor, fields) = match variant.payload {
                None => (Ctor::Tagged(variant.name.clone()), vec![]),
                Some(payload_ty) => {
                    let resolved = self.resolve_alias(payload_ty, &mut HashSet::new(), span);
                    match self.ty(resolved).kind.clone() {
                        TypeKind::Record(fields, _) => (
                            Ctor::Tagged(variant.name.clone()),
                            fields.iter().map(|f| f.ty).collect(),
                        ),
                        TypeKind::Tuple(items) => (
                            Ctor::TaggedTuple(variant.name.clone()),
                            items.iter().map(tuple_item_ty).collect(),
                        ),
                        _ => return None,
                    }
                }
            };
            push_unique(&mut ctors, SigCtor { ctor, fields });
        }
        Some(ctors)
    }

    // ── Pattern deconstruction ───────────────────────────────────────────────

    /// Deconstruct a THIR pattern into the matrix algebra, reordering tuple/record
    /// sub-patterns into the column type's canonical order.
    fn decon_pattern(&mut self, pat: ThirPatId, col_ty: TypeId) -> DeconPat {
        let pat = self.pat_arena[pat].clone();
        let col_ty = self.resolve_alias(col_ty, &mut HashSet::new(), pat.span);
        match pat.kind {
            ThirPatKind::Error | ThirPatKind::Wildcard | ThirPatKind::Bind(_) => DeconPat::Wild,
            ThirPatKind::True => DeconPat::nullary(Ctor::Bool(true)),
            ThirPatKind::False => DeconPat::nullary(Ctor::Bool(false)),
            ThirPatKind::Integer(value) => DeconPat::nullary(Ctor::IntLit(value)),
            ThirPatKind::Float(value) => DeconPat::nullary(Ctor::FloatLit(value.to_bits())),
            ThirPatKind::String(value) => DeconPat::nullary(Ctor::StrLit(value)),
            ThirPatKind::Atom(name) => {
                if name == "none" && matches!(self.ty(col_ty).kind, TypeKind::Optional(_)) {
                    DeconPat::nullary(Ctor::OptNone)
                } else if name == "absent" && matches!(self.ty(col_ty).kind, TypeKind::Maybe(_)) {
                    DeconPat::nullary(Ctor::MaybeAbsent)
                } else if matches!(self.ty(col_ty).kind, TypeKind::Union(_, _)) {
                    // Atom pattern against a union: pure enum variant with no payload.
                    DeconPat::nullary(Ctor::Tagged(name))
                } else {
                    DeconPat::nullary(Ctor::Atom(name))
                }
            }
            ThirPatKind::Tuple(items) => self.decon_tuple_pattern(&items, col_ty, pat.span),
            ThirPatKind::Record(fields) => self.decon_record_pattern(&fields, col_ty),
            ThirPatKind::TaggedValue { tag, payload } => {
                self.decon_tagged_value_pattern(tag, &payload, col_ty, pat.span)
            }
        }
    }

    fn decon_tuple_pattern(
        &mut self,
        items: &[ThirTuplePatItem],
        col_ty: TypeId,
        _span: Span,
    ) -> DeconPat {
        // An ill-typed pattern whose field count disagrees with its expected
        // shape already produced a diagnostic; treat it as a wildcard so it can
        // never corrupt the constructor arity the matrix algorithm relies on.
        match self.ty(col_ty).kind.clone() {
            TypeKind::Tuple(type_items) if type_items.len() == items.len() => {
                let field_tys: Vec<TypeId> = type_items.iter().map(tuple_item_ty).collect();
                DeconPat::Ctor {
                    tag: Ctor::Struct,
                    fields: self.decon_items_indexed(items, &field_tys),
                }
            }
            // A mismatch here was already reported by pattern checking.
            _ => DeconPat::Wild,
        }
    }

    fn decon_record_pattern(&mut self, fields: &[ThirRecordPatField], col_ty: TypeId) -> DeconPat {
        let TypeKind::Record(type_fields, _) = self.ty(col_ty).kind.clone() else {
            return DeconPat::Wild;
        };
        let by_name: HashMap<&str, ThirPatId> = fields
            .iter()
            .map(|f| (f.name.as_str(), f.pattern))
            .collect();
        let decon_fields = type_fields
            .iter()
            .map(|tf| match by_name.get(tf.name.as_str()) {
                Some(&pat) => self.decon_pattern(pat, tf.ty),
                None => DeconPat::Wild,
            })
            .collect();
        DeconPat::Ctor {
            tag: Ctor::Struct,
            fields: decon_fields,
        }
    }

    /// Deconstruct tuple items positionally against the given field types.
    fn decon_items_indexed(
        &mut self,
        items: &[ThirTuplePatItem],
        field_tys: &[TypeId],
    ) -> Vec<DeconPat> {
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let pat = match item {
                    ThirTuplePatItem::Named { pattern, .. } => *pattern,
                    ThirTuplePatItem::Positional(pattern) => *pattern,
                };
                let ty = field_tys.get(i).copied().unwrap_or(self.error_type);
                self.decon_pattern(pat, ty)
            })
            .collect()
    }

    /// Deconstruct a `ThirPatKind::TaggedValue` pattern for the Maranget matrix.
    fn decon_tagged_value_pattern(
        &mut self,
        tag: String,
        payload: &[ThirRecordPatField],
        col_ty: TypeId,
        span: Span,
    ) -> DeconPat {
        use std::collections::HashMap;
        match self.ty(col_ty).kind.clone() {
            TypeKind::Optional(_) if tag == "none" => DeconPat::nullary(Ctor::OptNone),
            TypeKind::Optional(inner) if tag == "some" => {
                let pat = payload
                    .iter()
                    .find(|f| f.name == "0")
                    .map(|f| self.decon_pattern(f.pattern, inner))
                    .unwrap_or(DeconPat::Wild);
                DeconPat::Ctor {
                    tag: Ctor::OptSome,
                    fields: vec![pat],
                }
            }
            TypeKind::Maybe(_) if tag == "absent" => DeconPat::nullary(Ctor::MaybeAbsent),
            TypeKind::Maybe(inner) if tag == "present" => {
                let pat = payload
                    .iter()
                    .find(|f| f.name == "0")
                    .map(|f| self.decon_pattern(f.pattern, inner))
                    .unwrap_or(DeconPat::Wild);
                DeconPat::Ctor {
                    tag: Ctor::MaybePresent,
                    fields: vec![pat],
                }
            }
            TypeKind::Union(variants, _) => {
                let variant = variants.iter().find(|v| v.name == tag).cloned();
                match variant {
                    None => DeconPat::Wild,
                    Some(v) => match v.payload {
                        None => DeconPat::nullary(Ctor::Tagged(tag)),
                        Some(payload_ty) => {
                            let resolved =
                                self.resolve_alias(payload_ty, &mut HashSet::new(), span);
                            match self.ty(resolved).kind.clone() {
                                TypeKind::Record(type_fields, _) => {
                                    let by_name: HashMap<&str, ThirPatId> = payload
                                        .iter()
                                        .map(|f| (f.name.as_str(), f.pattern))
                                        .collect();
                                    let fields = type_fields
                                        .iter()
                                        .map(|tf| match by_name.get(tf.name.as_str()) {
                                            Some(&pat) => self.decon_pattern(pat, tf.ty),
                                            None => DeconPat::Wild,
                                        })
                                        .collect();
                                    DeconPat::Ctor {
                                        tag: Ctor::Tagged(tag),
                                        fields,
                                    }
                                }
                                TypeKind::Tuple(type_items) => {
                                    let by_name: HashMap<&str, ThirPatId> = payload
                                        .iter()
                                        .map(|f| (f.name.as_str(), f.pattern))
                                        .collect();
                                    let fields = type_items
                                        .iter()
                                        .enumerate()
                                        .map(|(index, item)| {
                                            let (name, ty) = match item {
                                                TypeTupleItem::Named { name, ty, .. } => {
                                                    (name.as_str(), *ty)
                                                }
                                                TypeTupleItem::Positional(ty) => ("", *ty),
                                            };
                                            let index_name;
                                            let key = if name.is_empty() {
                                                index_name = index.to_string();
                                                index_name.as_str()
                                            } else {
                                                name
                                            };
                                            match by_name.get(key) {
                                                Some(&pat) => self.decon_pattern(pat, ty),
                                                None => DeconPat::Wild,
                                            }
                                        })
                                        .collect();
                                    DeconPat::Ctor {
                                        tag: Ctor::TaggedTuple(tag),
                                        fields,
                                    }
                                }
                                _ => DeconPat::Wild,
                            }
                        }
                    },
                }
            }
            _ => DeconPat::Wild,
        }
    }
}

// ── Matrix operations (no `self`) ────────────────────────────────────────────

/// S(c, P): keep rows whose head matches `tag` (expanding wildcards to `arity`
/// wildcards), exposing the constructor's fields as new leading columns.
fn specialize(tag: &Ctor, arity: usize, matrix: &[Vec<DeconPat>]) -> Vec<Vec<DeconPat>> {
    let mut out = Vec::new();
    for row in matrix {
        match &row[0] {
            DeconPat::Ctor { tag: t, fields } if t == tag => {
                let mut new_row = fields.clone();
                new_row.extend_from_slice(&row[1..]);
                out.push(new_row);
            }
            DeconPat::Wild => {
                let mut new_row = vec![DeconPat::Wild; arity];
                new_row.extend_from_slice(&row[1..]);
                out.push(new_row);
            }
            DeconPat::Ctor { .. } => {}
        }
    }
    out
}

/// D(P): keep only rows whose head is a wildcard, dropping the first column.
fn default_matrix(matrix: &[Vec<DeconPat>]) -> Vec<Vec<DeconPat>> {
    matrix
        .iter()
        .filter(|row| row[0].is_wild())
        .map(|row| row[1..].to_vec())
        .collect()
}

/// The set of head constructors appearing in column 0 of the matrix.
fn present_ctors(matrix: &[Vec<DeconPat>]) -> HashSet<Ctor> {
    matrix
        .iter()
        .filter_map(|row| match &row[0] {
            DeconPat::Ctor { tag, .. } => Some(tag.clone()),
            DeconPat::Wild => None,
        })
        .collect()
}

/// Re-wrap a witness whose first `arity` columns are `tag`'s fields back into a
/// single constructor column.
fn rewrap(tag: &Ctor, arity: usize, mut witness: Vec<DeconPat>) -> Vec<DeconPat> {
    let rest = witness.split_off(arity.min(witness.len()));
    let mut out = vec![DeconPat::Ctor {
        tag: tag.clone(),
        fields: witness,
    }];
    out.extend(rest);
    out
}

/// A witness value for an uncovered first column: a named missing constructor
/// when one exists, otherwise a wildcard.
fn missing_witness(sig: &Signature, present: &HashSet<Ctor>) -> DeconPat {
    if present.is_empty() {
        return DeconPat::Wild;
    }
    let Some(ctors) = &sig.ctors else {
        return DeconPat::Wild;
    };
    match ctors.iter().find(|c| !present.contains(&c.ctor)) {
        Some(sc) => DeconPat::Ctor {
            tag: sc.ctor.clone(),
            fields: vec![DeconPat::Wild; sc.fields.len()],
        },
        None => DeconPat::Wild,
    }
}

fn push_unique(ctors: &mut Vec<SigCtor>, ctor: SigCtor) {
    if !ctors.iter().any(|c| c.ctor == ctor.ctor) {
        ctors.push(ctor);
    }
}

fn tuple_item_ty(item: &TypeTupleItem) -> TypeId {
    match item {
        TypeTupleItem::Named { ty, .. } => *ty,
        TypeTupleItem::Positional(ty) => *ty,
    }
}

// ── Witness rendering ────────────────────────────────────────────────────────

fn render(witness: &[DeconPat]) -> String {
    witness.iter().map(render_one).collect::<Vec<_>>().join(" ")
}

fn render_one(pat: &DeconPat) -> String {
    match pat {
        DeconPat::Wild => "_".to_string(),
        DeconPat::Ctor { tag, fields } => match tag {
            Ctor::Bool(true) => "true".to_string(),
            Ctor::Bool(false) => "false".to_string(),
            Ctor::Atom(name) => format!("#{name}"),
            Ctor::IntLit(value) => value.to_string(),
            Ctor::FloatLit(bits) => f64::from_bits(*bits).to_string(),
            Ctor::StrLit(value) => format!("{value:?}"),
            Ctor::OptNone => "#none".to_string(),
            Ctor::OptSome => format!("#some ({})", render_payload(fields)),
            Ctor::MaybeAbsent => "#absent".to_string(),
            Ctor::MaybePresent => format!("#present ({})", render_payload(fields)),
            Ctor::Tagged(tag) => {
                if fields.is_empty() {
                    format!("#{tag}")
                } else {
                    format!("#{tag} {{ {} }}", render_payload(fields))
                }
            }
            Ctor::TaggedTuple(tag) => format!("#{tag} ({})", render_payload(fields)),
            Ctor::Struct => format!("({})", render_payload(fields)),
        },
    }
}

/// Render constructor payload, collapsing an all-wildcard payload to `...`.
fn render_payload(fields: &[DeconPat]) -> String {
    if fields.iter().all(DeconPat::is_wild) {
        "...".to_string()
    } else {
        fields.iter().map(render_one).collect::<Vec<_>>().join(", ")
    }
}
