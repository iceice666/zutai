//! Typed high-level IR for Zutai general mode (`.zt`).
//!
//! THIR is the planned output of type checking and elaboration. It is distinct
//! from HIR so that HIR remains useful when type checking fails and THIR can
//! lower type-dependent sugar such as optional access and defaulting.

use zutai_hir::{BindingId, HirDeclId, HirExprId, HirImportSource, HirPatId};
use zutai_syntax::Span;
use zutai_syntax::ast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThirDeclId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThirExprId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThirPatId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub struct ThirFile {
    pub decls: Vec<ThirDeclId>,
    pub final_expr: ThirExprId,
    pub decl_arena: Vec<ThirDecl>,
    pub expr_arena: Vec<ThirExpr>,
    pub pat_arena: Vec<ThirPat>,
    pub type_arena: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirDecl {
    pub source: HirDeclId,
    pub binding: BindingId,
    pub kind: ThirDeclKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirDeclKind {
    Value {
        ty: TypeId,
        value: ThirExprId,
    },
    TypeAlias {
        params: Vec<BindingId>,
        ty: TypeId,
    },
    Function {
        params: Vec<BindingId>,
        sig: TypeId,
        clauses: Vec<ThirClause>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirClause {
    pub patterns: Vec<ThirPatId>,
    pub guard: Option<ThirExprId>,
    pub body: ThirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirExpr {
    pub source: HirExprId,
    pub ty: TypeId,
    pub kind: ThirExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirExprKind {
    True,
    False,
    Integer(i64),
    Float(f64),
    String(String),
    Atom(String),
    BindingRef(BindingId),
    Record(Vec<ThirRecordField>),
    Tuple(Vec<ThirTupleItem>),
    List(Vec<ThirExprId>),
    Block {
        bindings: Vec<ThirLocalBinding>,
        result: ThirExprId,
    },
    Lambda {
        params: Vec<ThirPatId>,
        body: ThirExprId,
    },
    If {
        cond: ThirExprId,
        then_branch: ThirExprId,
        else_branch: ThirExprId,
    },
    Match {
        scrutinee: ThirExprId,
        arms: Vec<ThirClause>,
    },
    Import(HirImportSource),
    TypeValue(TypeId),
    Apply {
        func: ThirExprId,
        arg: ThirExprId,
        instantiation: Vec<TypeId>,
    },
    Access {
        receiver: ThirExprId,
        field: String,
    },
    OptionalAccess {
        receiver: ThirExprId,
        field: String,
    },
    Binary {
        op: ast::BinOp,
        lhs: ThirExprId,
        rhs: ThirExprId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirRecordField {
    pub name: String,
    pub value: ThirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirTupleItem {
    Named {
        name: String,
        value: ThirExprId,
        span: Span,
    },
    Positional(ThirExprId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirLocalBinding {
    pub binding: BindingId,
    pub ty: TypeId,
    pub value: ThirExprId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirPat {
    pub source: HirPatId,
    pub ty: TypeId,
    pub kind: ThirPatKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirPatKind {
    Wildcard,
    Bind(BindingId),
    True,
    False,
    Integer(i64),
    Float(f64),
    String(String),
    Atom(String),
    Tuple(Vec<ThirTuplePatItem>),
    Record(Vec<ThirRecordPatField>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThirTuplePatItem {
    Named {
        name: String,
        pattern: ThirPatId,
        span: Span,
    },
    Positional(ThirPatId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThirRecordPatField {
    pub name: String,
    pub pattern: ThirPatId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    Type,
    Bool,
    Text,
    Int,
    Float,
    Atom(String),
    True,
    False,
    List(TypeId),
    Optional(TypeId),
    Record(Vec<TypeRecordField>),
    Union(Vec<TypeId>),
    Tuple(Vec<TypeTupleItem>),
    Function { from: TypeId, to: TypeId },
    TypeVar(BindingId),
    Alias(BindingId),
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeRecordField {
    pub name: String,
    pub optional: bool,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeTupleItem {
    Named {
        name: String,
        ty: TypeId,
        span: Span,
    },
    Positional(TypeId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirDiagnostic {
    pub kind: ThirDiagnosticKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThirDiagnosticKind {
    TypeCheckerNotImplemented,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoweredThir {
    pub file: Option<ThirFile>,
    pub diagnostics: Vec<ThirDiagnostic>,
    pub pass_reports: Vec<ThirPassReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThirLowerOptions {
    pub run_passes: bool,
}

impl Default for ThirLowerOptions {
    fn default() -> Self {
        Self { run_passes: true }
    }
}

pub fn lower_hir(_file: &zutai_hir::HirFile) -> LoweredThir {
    lower_hir_with_options(_file, ThirLowerOptions::default())
}

pub fn lower_hir_with_options(
    _file: &zutai_hir::HirFile,
    options: ThirLowerOptions,
) -> LoweredThir {
    let mut lowered = LoweredThir {
        file: None,
        diagnostics: vec![ThirDiagnostic {
            kind: ThirDiagnosticKind::TypeCheckerNotImplemented,
            span: Span::default(),
        }],
        pass_reports: Vec::new(),
    };
    if options.run_passes {
        lowered.pass_reports = run_default_passes(&mut lowered.file, &mut lowered.diagnostics);
    }
    lowered
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirPassReport {
    pub name: &'static str,
}

pub trait ThirPass {
    fn name(&self) -> &'static str;
    fn run(&mut self, file: &mut ThirFile, diagnostics: &mut Vec<ThirDiagnostic>);
}

pub fn run_passes(
    file: &mut ThirFile,
    diagnostics: &mut Vec<ThirDiagnostic>,
    passes: &mut [&mut dyn ThirPass],
) -> Vec<ThirPassReport> {
    passes
        .iter_mut()
        .map(|pass| {
            pass.run(file, diagnostics);
            ThirPassReport { name: pass.name() }
        })
        .collect()
}

pub fn run_default_passes(
    file: &mut Option<ThirFile>,
    diagnostics: &mut Vec<ThirDiagnostic>,
) -> Vec<ThirPassReport> {
    let Some(file) = file.as_mut() else {
        return Vec::new();
    };
    let mut passes: [&mut dyn ThirPass; 0] = [];
    run_passes(file, diagnostics, &mut passes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_thir_passes_in_order() {
        struct MarkerPass(&'static str);

        impl ThirPass for MarkerPass {
            fn name(&self) -> &'static str {
                self.0
            }

            fn run(&mut self, file: &mut ThirFile, _diagnostics: &mut Vec<ThirDiagnostic>) {
                file.decls.clear();
            }
        }

        let mut file = ThirFile {
            decls: Vec::new(),
            final_expr: ThirExprId(0),
            decl_arena: Vec::new(),
            expr_arena: Vec::new(),
            pat_arena: Vec::new(),
            type_arena: Vec::new(),
        };
        let mut diagnostics = Vec::new();
        let mut first = MarkerPass("first");
        let mut second = MarkerPass("second");
        let mut passes: [&mut dyn ThirPass; 2] = [&mut first, &mut second];

        let reports = run_passes(&mut file, &mut diagnostics, &mut passes);

        assert_eq!(
            reports,
            vec![
                ThirPassReport { name: "first" },
                ThirPassReport { name: "second" }
            ]
        );
        assert!(diagnostics.is_empty());
    }
}
