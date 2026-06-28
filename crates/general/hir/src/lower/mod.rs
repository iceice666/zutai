use rustc_hash::{FxHashMap, FxHashSet};

use la_arena::Arena;
use zutai_syntax::Span;
use zutai_syntax::ast;

use crate::diagnostic::{HirDiagnostic, HirDiagnosticKind};
use crate::ir::{
    Binding, BindingId, BindingKind, HirClause, HirConstraintMethod, HirDecl, HirDeclId,
    HirDeclKind, HirDeriveRecipe, HirEffectOp, HirEffectRow, HirExpr, HirExprId, HirExprKind,
    HirFile, HirHandleClause, HirHandleOp, HirImportSource, HirLevel, HirLocalBinding, HirPat,
    HirPatId, HirPatKind, HirRecordField, HirRecordPatField, HirRowTail, HirRowTailKind,
    HirSelectField, HirTupleItem, HirTuplePatItem, HirTypeExpr, HirTypeId, HirTypeKind,
    HirTypeParam, HirTypeRecordField, HirTypeTupleItem, HirUnionVariant, HirWitnessField,
};
use crate::pass::{HirPassReport, run_default_passes};

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredHir {
    pub file: HirFile,
    pub diagnostics: Vec<HirDiagnostic>,
    pub pass_reports: Vec<HirPassReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirLowerOptions {
    pub run_passes: bool,
}

impl Default for HirLowerOptions {
    fn default() -> Self {
        Self { run_passes: true }
    }
}

/// Canonical source for the codata `Stream` type and its combinators
/// (`cons`/`singleton`/`map`/`filter`/`take`/`drop`/`fold`/`uncons`).
///
/// Single source of truth for two surfaces (V3-G6):
/// - **Ambient prelude.** [`lower_file`] injects these *declarations* into every
///   module as a fallback (user and constraint-method names of the same spelling
///   win); each is lowered into a module only when that module references it. The
///   final record expression is the export for the import surface and is ignored
///   on this path.
/// - **Importable module.** `s ::= import "stream.zt"` exports the final record, so
///   a program can use `s.map`, `s.fold`, … qualified. The same constant backs a
///   user-importable copy of the file in tests, keeping one source of truth.
pub const STREAM_MODULE_SRC: &str = include_str!("prelude/stream.zt");
///
/// Canonical source for the small function prelude — the ordinary polymorphic
/// helpers `id`/`const`/`compose`/`flip` (stdlib slice B).
///
/// Same two-surface model as `STREAM_MODULE_SRC`: [`lower_file`] injects the
/// declarations as an ambient fallback (user/constraint names of the same
/// spelling win; lowered only when referenced), and `import stdlib.prelude`
/// exports the final record. Pure source-level stdlib — no intrinsics, no new
/// syntax, no backend IR node.
pub const PRELUDE_MODULE_SRC: &str = include_str!("prelude/prelude.zt");

pub fn lower_file(file: &ast::File) -> LoweredHir {
    lower_file_with_options(file, HirLowerOptions::default())
}

pub fn lower_file_with_options(file: &ast::File, options: HirLowerOptions) -> LoweredHir {
    let mut lowerer = Lowerer::new(file.span);
    let mut lowered = lowerer.lower_file(file);
    if options.run_passes {
        lowered.pass_reports = run_default_passes(&mut lowered.file, &mut lowered.diagnostics);
    }
    lowered
}

#[derive(Default)]
struct Scope {
    names: FxHashMap<String, BindingId>,
}

/// Tracks the lexically-nearest `handle` clause body during lowering so that
/// `resume` can be validated: it is legal only inside an *operation* clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandlerClauseKind {
    Value,
    Finally,
    Operation,
}

struct Lowerer {
    bindings: Vec<Binding>,
    decl_arena: Arena<HirDecl>,
    expr_arena: Arena<HirExpr>,
    pat_arena: Arena<HirPat>,
    type_arena: Arena<HirTypeExpr>,
    scopes: Vec<Scope>,
    diagnostics: Vec<HirDiagnostic>,
    /// Maps each constraint's `BindingId` to the index-aligned vector of
    /// per-method bindings allocated in Pass 1.  `None` entries are operator methods.
    constraint_method_bindings: FxHashMap<BindingId, Vec<Option<BindingId>>>,
    /// Maps a destructuring binding's synthetic receiver `BindingId` to the
    /// per-field `(binding, field_name)` pairs allocated in Pass 1, so Pass 2 can
    /// emit one `field ::= receiver.field` value decl per name.
    destructure_fields: FxHashMap<BindingId, Vec<(BindingId, String)>>,
    /// The lexically-nearest enclosing `handle` clause body, if any. `resume`
    /// is only valid when this is `Some(HandlerClauseKind::Operation)`.
    handler_clause: Option<HandlerClauseKind>,
    /// Level-parameter bindings (`<$l>`) referenced by a `$…` level use, used to
    /// report declared-but-unused level variables.
    used_level_params: FxHashSet<BindingId>,
}

mod decl;
mod expr;
mod types;

impl Lowerer {
    fn new(file_span: Span) -> Self {
        let mut lowerer = Self {
            bindings: Vec::new(),
            decl_arena: Arena::new(),
            expr_arena: Arena::new(),
            pat_arena: Arena::new(),
            type_arena: Arena::new(),
            scopes: vec![Scope::default()],
            diagnostics: Vec::new(),
            constraint_method_bindings: FxHashMap::default(),
            destructure_fields: FxHashMap::default(),
            handler_clause: None,
            used_level_params: FxHashSet::default(),
        };
        for name in [
            "Type",
            "Text",
            "Bool",
            "Int",
            "Float",
            "i8",
            "i16",
            "i32",
            "i64",
            "u8",
            "u16",
            "u32",
            "u64",
            "f32",
            "f64",
            "List",
            "Optional",
            "Maybe",
            "Patch",
            "DeepPatch",
        ] {
            lowerer.define_current(name.to_string(), BindingKind::BuiltinType, file_span);
        }
        for name in crate::ir::HOST_CAPABILITY_TYPE_NAMES
            .iter()
            .chain(crate::ir::HOST_SUPPORT_TYPE_NAMES)
        {
            lowerer.define_current((*name).to_string(), BindingKind::BuiltinType, file_span);
        }
        for nbits in [32u8, 64] {
            lowerer.define_current(format!("Posit{nbits}"), BindingKind::BuiltinType, file_span);
            for es in 0..nbits {
                lowerer.define_current(
                    format!("Posit{nbits}e{es}"),
                    BindingKind::BuiltinType,
                    file_span,
                );
            }
        }
        for name in crate::ir::BUILTIN_VALUE_NAMES {
            lowerer.define_current((*name).to_string(), BindingKind::BuiltinValue, file_span);
        }
        lowerer
    }

    fn lower_file(&mut self, file: &ast::File) -> LoweredHir {
        // Define user top-level names first, then the source preludes (the codata
        // `Stream` type + combinators, and the small function prelude). All names
        // are in scope before any body is lowered, so user code can reference
        // prelude names; defining user decls first keeps their binding ids and
        // decl positions stable.
        let user_bindings: Vec<_> = file
            .decls
            .iter()
            .map(|decl| self.define_top_decl(decl))
            .collect();
        // Each source prelude is a fallback: define only names not already owned by
        // user code or constraint methods (all share this scope), so user
        // definitions always win and a colliding name raises no spurious
        // duplicate-binding diagnostic. The decl index is kept so the body can be
        // lowered later if the name is actually used.
        let (stream_prelude_ast, stream_prelude_decls) =
            self.define_prelude_fallback(STREAM_MODULE_SRC);
        let (fn_prelude_ast, fn_prelude_decls) = self.define_prelude_fallback(PRELUDE_MODULE_SRC);

        let mut decls: Vec<HirDeclId> = Vec::new();
        for (decl, binding) in file.decls.iter().zip(user_bindings) {
            match decl {
                // A destructuring binding expands to a synthetic receiver decl
                // plus one `field ::= receiver.field` decl per name.
                ast::Decl::Destructure { value, .. } => {
                    self.lower_destructure_decl(binding, value, &mut decls);
                }
                _ => decls.push(self.lower_decl(decl, binding)),
            }
        }
        let final_expr = self.lower_expr(&file.final_expr);

        // Include each prelude (all of it) only when the program references any of
        // its names — a type (e.g. an annotation mentions `Stream`) or a value
        // (e.g. a call to `map`/`id`). All-or-nothing is sufficient because each
        // prelude's declarations depend only on their own shared type
        // (`Stream`/`Step` for the stream prelude) and self-recursion, never on
        // each other, so any single reference co-satisfies every dependency; the
        // function prelude's helpers are mutually independent. A program that
        // touches no prelude name keeps it out of THIR/TLC/codegen entirely. Both
        // preludes are valid and never diagnose.
        //
        // The stream prelude is also force-included when the program uses the
        // `loadZti`/`loadZt` builtins (or their `perform load.zti`/`load.zt`
        // forms): those return `Data`/`DataField`, whose type declarations live in
        // the stream prelude even though no prelude binding is textually named.
        let references_dynamic_load_builtin = self.expr_arena.iter().any(|(_, e)| {
            matches!(e.kind, HirExprKind::BindingRef(b) if matches!(self.bindings.get(b.0 as usize).map(|binding| binding.name.as_str()), Some("loadZti" | "loadZt")))
                || matches!(&e.kind, HirExprKind::Perform { op, .. } if matches!(op.as_slice(), [namespace, ext] if namespace == "load" && (ext == "zti" || ext == "zt")))
        });
        if let Some(p) = stream_prelude_ast.as_ref() {
            self.lower_prelude_if_referenced(
                p,
                &stream_prelude_decls,
                references_dynamic_load_builtin,
                &mut decls,
            );
        }
        if let Some(p) = fn_prelude_ast.as_ref() {
            self.lower_prelude_if_referenced(p, &fn_prelude_decls, false, &mut decls);
        }

        LoweredHir {
            file: HirFile {
                decls,
                final_expr,
                span: file.span,
                bindings: std::mem::take(&mut self.bindings),
                decl_arena: std::mem::take(&mut self.decl_arena),
                expr_arena: std::mem::take(&mut self.expr_arena),
                pat_arena: std::mem::take(&mut self.pat_arena),
                type_arena: std::mem::take(&mut self.type_arena),
            },
            diagnostics: std::mem::take(&mut self.diagnostics),
            pass_reports: Vec::new(),
        }
    }

    /// Parse a source prelude (`src`) and define its declarations as fallback
    /// bindings — only names not already owned by user code or constraint
    /// methods. Returns the parsed AST (for later body lowering) and the
    /// `(decl index, binding)` pairs that survived the fallback filter. The
    /// prelude is a fixed constant; a parse failure is an internal bug, so fail
    /// fast in dev builds.
    fn define_prelude_fallback(
        &mut self,
        src: &str,
    ) -> (Option<ast::File>, Vec<(usize, BindingId)>) {
        let prelude_ast = zutai_syntax::parse_ast_only(src).into_ast();
        debug_assert!(prelude_ast.is_some(), "builtin prelude must parse");
        let prelude_decls = prelude_ast
            .as_ref()
            .map(|p| {
                p.decls
                    .iter()
                    .enumerate()
                    .filter_map(|(i, d)| {
                        let taken = self
                            .scopes
                            .last()
                            .is_some_and(|s| s.names.contains_key(d.name()));
                        (!taken).then(|| (i, self.define_top_decl(d)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        (prelude_ast, prelude_decls)
    }

    /// Lower a prelude's declaration bodies into `decls` iff the program
    /// references any of its fallback bindings. `force_include` covers
    /// prelude-internal triggers that need the prelude present even though no
    /// fallback binding is textually named — the stream prelude's `Data`/
    /// `DataField` types, required by the `loadZti`/`loadZt` builtins.
    /// All-or-nothing: every surviving decl is lowered together once any is
    /// referenced, since each prelude's decls share a single dependency cluster.
    fn lower_prelude_if_referenced(
        &mut self,
        prelude_ast: &ast::File,
        prelude_decls: &[(usize, BindingId)],
        force_include: bool,
        decls: &mut Vec<HirDeclId>,
    ) {
        let set: Vec<BindingId> = prelude_decls.iter().map(|(_, b)| *b).collect();
        let referenced = force_include
            || self
                .type_arena
                .iter()
                .any(|(_, ty)| matches!(ty.kind, HirTypeKind::BindingRef(b) if set.contains(&b)))
            || self
                .expr_arena
                .iter()
                .any(|(_, e)| matches!(e.kind, HirExprKind::BindingRef(b) if set.contains(&b)));
        if referenced {
            for (i, binding) in prelude_decls {
                decls.push(self.lower_decl(&prelude_ast.decls[*i], *binding));
            }
        }
    }

    pub(super) fn define_current(
        &mut self,
        name: String,
        kind: BindingKind,
        span: Span,
    ) -> BindingId {
        let id = BindingId(self.bindings.len() as u32);
        let scope = self.scopes.last_mut().expect("scope stack is never empty");
        if let Some(first) = scope.names.get(&name).copied() {
            self.diagnostics.push(HirDiagnostic {
                kind: HirDiagnosticKind::DuplicateBinding {
                    name: name.clone(),
                    first_span: self.bindings[first.0 as usize].span,
                },
                span,
            });
        } else {
            scope.names.insert(name.clone(), id);
        }
        self.bindings.push(Binding { name, kind, span });
        id
    }

    pub(super) fn resolve(&self, name: &str) -> Option<BindingId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.names.get(name).copied())
    }

    pub(super) fn push_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
        debug_assert!(!self.scopes.is_empty());
    }

    pub(super) fn alloc_decl(&mut self, decl: HirDecl) -> HirDeclId {
        self.decl_arena.alloc(decl)
    }

    pub(super) fn alloc_expr(&mut self, expr: HirExpr) -> HirExprId {
        self.expr_arena.alloc(expr)
    }

    pub(super) fn alloc_pat(&mut self, pat: HirPat) -> HirPatId {
        self.pat_arena.alloc(pat)
    }

    pub(super) fn alloc_type(&mut self, ty: HirTypeExpr) -> HirTypeId {
        self.type_arena.alloc(ty)
    }
}
