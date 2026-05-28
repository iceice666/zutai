use crate::{SyntaxKind, SyntaxNode};

use super::{AstNode, support};

// ── Macro to define simple AstNode newtypes ───────────────────────────────────

macro_rules! ast_node {
    ($Name:ident, $kind:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $Name(SyntaxNode);

        impl AstNode for $Name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == $kind
            }
            fn cast(node: SyntaxNode) -> Option<Self> {
                if node.kind() == $kind {
                    Some(Self(node))
                } else {
                    None
                }
            }
            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

// ── File ──────────────────────────────────────────────────────────────────────

ast_node!(File, SyntaxKind::FILE);

impl File {
    pub fn decls(&self) -> impl Iterator<Item = TopDecl> + '_ {
        support::children(self.syntax())
    }
}

// ── Top-level declarations ────────────────────────────────────────────────────

/// Any top-level declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopDecl {
    Inferred(InferredBinding),
    Annotated(AnnotatedBinding),
    Func(FuncDecl),
}

impl AstNode for TopDecl {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::INFERRED_BINDING | SyntaxKind::ANNOTATED_BINDING | SyntaxKind::FUNC_DECL
        )
    }
    fn cast(node: SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::INFERRED_BINDING => Self::Inferred(InferredBinding(node)),
            SyntaxKind::ANNOTATED_BINDING => Self::Annotated(AnnotatedBinding(node)),
            SyntaxKind::FUNC_DECL => Self::Func(FuncDecl(node)),
            _ => return None,
        })
    }
    fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Inferred(n) => &n.0,
            Self::Annotated(n) => &n.0,
            Self::Func(n) => &n.0,
        }
    }
}

/// Return the binding name (the leading `IDENT` token) of any top-level decl.
pub fn decl_name(decl: &TopDecl) -> Option<crate::SyntaxToken> {
    use crate::SyntaxKind::IDENT;
    super::support::token(decl.syntax(), IDENT)
}

// ── IDENT := Expr ─────────────────────────────────────────────────────────────

ast_node!(InferredBinding, SyntaxKind::INFERRED_BINDING);

impl InferredBinding {
    pub fn name_token(&self) -> Option<crate::SyntaxToken> {
        support::token(self.syntax(), SyntaxKind::IDENT)
    }
    pub fn name(&self) -> Option<String> {
        self.name_token().map(|t| t.text().to_owned())
    }
    pub fn value(&self) -> Option<Expr> {
        support::child(self.syntax())
    }
}

// ── IDENT : TypeExpr = Expr ───────────────────────────────────────────────────

ast_node!(AnnotatedBinding, SyntaxKind::ANNOTATED_BINDING);

impl AnnotatedBinding {
    pub fn name_token(&self) -> Option<crate::SyntaxToken> {
        support::token(self.syntax(), SyntaxKind::IDENT)
    }
    pub fn name(&self) -> Option<String> {
        self.name_token().map(|t| t.text().to_owned())
    }
    pub fn value(&self) -> Option<Expr> {
        support::child(self.syntax())
    }
}

// ── IDENT :: … (function / type declaration) ─────────────────────────────────

ast_node!(FuncDecl, SyntaxKind::FUNC_DECL);

impl FuncDecl {
    pub fn name_token(&self) -> Option<crate::SyntaxToken> {
        support::token(self.syntax(), SyntaxKind::IDENT)
    }
    pub fn name(&self) -> Option<String> {
        self.name_token().map(|t| t.text().to_owned())
    }
    pub fn type_params(&self) -> Option<TypeParamList> {
        support::child(self.syntax())
    }
    pub fn clauses(&self) -> impl Iterator<Item = Clause> + '_ {
        support::children(self.syntax())
    }
}

// ── Type parameter list ───────────────────────────────────────────────────────

ast_node!(TypeParamList, SyntaxKind::TYPE_PARAM_LIST);

impl TypeParamList {
    pub fn params(&self) -> impl Iterator<Item = crate::SyntaxToken> + '_ {
        self.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::IDENT)
    }
}

// ── Clause ────────────────────────────────────────────────────────────────────

ast_node!(Clause, SyntaxKind::CLAUSE);

impl Clause {
    pub fn patterns(&self) -> impl Iterator<Item = Pattern> + '_ {
        support::children(self.syntax())
    }
    pub fn guard(&self) -> Option<Guard> {
        support::child(self.syntax())
    }
    pub fn body(&self) -> Option<Block> {
        support::child(self.syntax())
    }
}

ast_node!(Guard, SyntaxKind::GUARD);

impl Guard {
    pub fn condition(&self) -> Option<Expr> {
        support::child(self.syntax())
    }
}

// ── Block ─────────────────────────────────────────────────────────────────────

ast_node!(Block, SyntaxKind::BLOCK);

impl Block {
    pub fn local_bindings(&self) -> impl Iterator<Item = LocalBinding> + '_ {
        support::children(self.syntax())
    }
    pub fn output(&self) -> Option<Expr> {
        support::child(self.syntax())
    }
}

ast_node!(LocalBinding, SyntaxKind::LOCAL_BINDING);

impl LocalBinding {
    pub fn name_token(&self) -> Option<crate::SyntaxToken> {
        support::token(self.syntax(), SyntaxKind::IDENT)
    }
    pub fn name(&self) -> Option<String> {
        self.name_token().map(|t| t.text().to_owned())
    }
    pub fn value(&self) -> Option<Expr> {
        support::child(self.syntax())
    }
}

// ── Expressions ───────────────────────────────────────────────────────────────

/// Any expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Literal(Literal),
    Paren(ParenExpr),
    Tuple(TupleExpr),
    Record(RecordExpr),
    List(ListExpr),
    Lambda(LambdaExpr),
    Match(MatchExpr),
    If(IfExpr),
    Import(ImportExpr),
    Call(CallExpr),
    Access(AccessExpr),
    OptionalAccess(OptionalAccessExpr),
    Binary(BinaryExpr),
    Pipeline(PipelineExpr),
    Block(Block),
    TypeForm(TypeForm),
}

impl AstNode for Expr {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::LITERAL
                | SyntaxKind::PAREN_EXPR
                | SyntaxKind::TUPLE_EXPR
                | SyntaxKind::RECORD_EXPR
                | SyntaxKind::LIST_EXPR
                | SyntaxKind::LAMBDA_EXPR
                | SyntaxKind::MATCH_EXPR
                | SyntaxKind::IF_EXPR
                | SyntaxKind::IMPORT_EXPR
                | SyntaxKind::CALL_EXPR
                | SyntaxKind::ACCESS_EXPR
                | SyntaxKind::OPTIONAL_ACCESS_EXPR
                | SyntaxKind::BINARY_EXPR
                | SyntaxKind::PIPELINE_EXPR
                | SyntaxKind::BLOCK
                | SyntaxKind::TYPE_FORM
        )
    }
    fn cast(node: SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::LITERAL => Self::Literal(Literal(node)),
            SyntaxKind::PAREN_EXPR => Self::Paren(ParenExpr(node)),
            SyntaxKind::TUPLE_EXPR => Self::Tuple(TupleExpr(node)),
            SyntaxKind::RECORD_EXPR => Self::Record(RecordExpr(node)),
            SyntaxKind::LIST_EXPR => Self::List(ListExpr(node)),
            SyntaxKind::LAMBDA_EXPR => Self::Lambda(LambdaExpr(node)),
            SyntaxKind::MATCH_EXPR => Self::Match(MatchExpr(node)),
            SyntaxKind::IF_EXPR => Self::If(IfExpr(node)),
            SyntaxKind::IMPORT_EXPR => Self::Import(ImportExpr(node)),
            SyntaxKind::CALL_EXPR => Self::Call(CallExpr(node)),
            SyntaxKind::ACCESS_EXPR => Self::Access(AccessExpr(node)),
            SyntaxKind::OPTIONAL_ACCESS_EXPR => Self::OptionalAccess(OptionalAccessExpr(node)),
            SyntaxKind::BINARY_EXPR => Self::Binary(BinaryExpr(node)),
            SyntaxKind::PIPELINE_EXPR => Self::Pipeline(PipelineExpr(node)),
            SyntaxKind::BLOCK => Self::Block(Block(node)),
            SyntaxKind::TYPE_FORM => Self::TypeForm(TypeForm(node)),
            _ => return None,
        })
    }
    fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Literal(n) => &n.0,
            Self::Paren(n) => &n.0,
            Self::Tuple(n) => &n.0,
            Self::Record(n) => &n.0,
            Self::List(n) => &n.0,
            Self::Lambda(n) => &n.0,
            Self::Match(n) => &n.0,
            Self::If(n) => &n.0,
            Self::Import(n) => &n.0,
            Self::Call(n) => &n.0,
            Self::Access(n) => &n.0,
            Self::OptionalAccess(n) => &n.0,
            Self::Binary(n) => &n.0,
            Self::Pipeline(n) => &n.0,
            Self::Block(n) => &n.0,
            Self::TypeForm(n) => &n.0,
        }
    }
}

ast_node!(Literal, SyntaxKind::LITERAL);
ast_node!(ParenExpr, SyntaxKind::PAREN_EXPR);
ast_node!(TupleExpr, SyntaxKind::TUPLE_EXPR);
ast_node!(RecordExpr, SyntaxKind::RECORD_EXPR);
ast_node!(ListExpr, SyntaxKind::LIST_EXPR);
ast_node!(LambdaExpr, SyntaxKind::LAMBDA_EXPR);
ast_node!(MatchExpr, SyntaxKind::MATCH_EXPR);
ast_node!(IfExpr, SyntaxKind::IF_EXPR);
ast_node!(ImportExpr, SyntaxKind::IMPORT_EXPR);
ast_node!(CallExpr, SyntaxKind::CALL_EXPR);
ast_node!(AccessExpr, SyntaxKind::ACCESS_EXPR);
ast_node!(OptionalAccessExpr, SyntaxKind::OPTIONAL_ACCESS_EXPR);

ast_node!(BinaryExpr, SyntaxKind::BINARY_EXPR);

impl BinaryExpr {
    /// The binary operator token kind.
    pub fn op_kind(&self) -> Option<SyntaxKind> {
        use crate::SyntaxKind::*;
        self.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| {
                matches!(
                    t.kind(),
                    PLUS | MINUS
                        | STAR
                        | SLASH
                        | EQ_EQ
                        | BANG_EQ
                        | LT
                        | LT_EQ
                        | GT
                        | GT_EQ
                        | AMP_AMP
                        | PIPE_PIPE
                        | QUESTION_QUESTION
                )
            })
            .map(|t| t.kind())
    }

    pub fn op(&self) -> Option<super::operators::BinaryOp> {
        self.op_kind()
            .and_then(super::operators::BinaryOp::from_kind)
    }

    pub fn lhs(&self) -> Option<Expr> {
        support::child(self.syntax())
    }
}

ast_node!(PipelineExpr, SyntaxKind::PIPELINE_EXPR);

impl PipelineExpr {
    pub fn direction(&self) -> Option<super::operators::PipelineDir> {
        use crate::SyntaxKind::{ARROW_PIPE, PIPE_ARROW};
        self.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| matches!(t.kind(), PIPE_ARROW | ARROW_PIPE))
            .map(|t| {
                if t.kind() == PIPE_ARROW {
                    super::operators::PipelineDir::Forward
                } else {
                    super::operators::PipelineDir::Backward
                }
            })
    }
}

ast_node!(MatchCase, SyntaxKind::MATCH_CASE);

impl MatchExpr {
    pub fn cases(&self) -> impl Iterator<Item = MatchCase> + '_ {
        support::children(self.syntax())
    }
}

ast_node!(TypeForm, SyntaxKind::TYPE_FORM);

// ── Patterns ──────────────────────────────────────────────────────────────────

/// Any pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Wildcard(WildcardPattern),
    Literal(Literal),
    Tuple(TuplePattern),
    Record(RecordPattern),
}

impl AstNode for Pattern {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            SyntaxKind::WILDCARD_PATTERN
                | SyntaxKind::LITERAL
                | SyntaxKind::TUPLE_PATTERN
                | SyntaxKind::RECORD_PATTERN
        )
    }
    fn cast(node: SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            SyntaxKind::WILDCARD_PATTERN => Self::Wildcard(WildcardPattern(node)),
            SyntaxKind::LITERAL => Self::Literal(Literal(node)),
            SyntaxKind::TUPLE_PATTERN => Self::Tuple(TuplePattern(node)),
            SyntaxKind::RECORD_PATTERN => Self::Record(RecordPattern(node)),
            _ => return None,
        })
    }
    fn syntax(&self) -> &SyntaxNode {
        match self {
            Self::Wildcard(n) => &n.0,
            Self::Literal(n) => &n.0,
            Self::Tuple(n) => &n.0,
            Self::Record(n) => &n.0,
        }
    }
}

ast_node!(WildcardPattern, SyntaxKind::WILDCARD_PATTERN);
ast_node!(TuplePattern, SyntaxKind::TUPLE_PATTERN);
ast_node!(RecordPattern, SyntaxKind::RECORD_PATTERN);
