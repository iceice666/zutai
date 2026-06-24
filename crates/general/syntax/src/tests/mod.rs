use crate::ast::*;
use crate::error::ParseErrorKind;
use crate::parser::expr::parse_expr;
use crate::{LineIndex, SyntaxKind, parse, parse_ast_only, parse_lossless, tokenize};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_expr_str(s: &str) -> Expr {
    crate::parser::lex::BASE_PTR.with(|c| c.set(s.as_ptr() as usize));
    let mut input = s;
    parse_expr(&mut input).unwrap_or_else(|e| panic!("parse_expr({s:?}) failed: {e}"))
}

fn parse_str(s: &str) -> File {
    let parsed = parse(s);
    if parsed.ast().is_none() {
        let msgs: Vec<_> = parsed
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        panic!("parse({s:?}) failed:\n{}", msgs.join("\n"));
    }
    parsed.into_ast().expect("checked above")
}

fn parse_kinds(s: &str) -> Vec<ParseErrorKind> {
    parse(s)
        .diagnostics()
        .iter()
        .map(|err| err.kind.clone())
        .collect()
}

fn parse_ast_only_kinds(s: &str) -> Vec<ParseErrorKind> {
    parse_ast_only(s)
        .diagnostics()
        .iter()
        .map(|err| err.kind.clone())
        .collect()
}

fn as_int(e: &Expr) -> i64 {
    match e {
        Expr::Integer { value, .. } => *value,
        other => panic!("expected Int, got {other:?}"),
    }
}

fn as_float(e: &Expr) -> f64 {
    match e {
        Expr::Float { value, .. } => *value,
        other => panic!("expected Float, got {other:?}"),
    }
}

fn as_str_val(e: &Expr) -> &str {
    match e {
        Expr::String { value, .. } => value,
        other => panic!("expected Str, got {other:?}"),
    }
}

fn as_atom(e: &Expr) -> &str {
    match e {
        Expr::Atom { name, .. } => name,
        other => panic!("expected Atom, got {other:?}"),
    }
}

fn as_ident(e: &Expr) -> &str {
    match e {
        Expr::Ident { name, .. } => name,
        other => panic!("expected Ident, got {other:?}"),
    }
}

fn as_record(e: &Expr) -> &Vec<RecordField> {
    match e {
        Expr::Record { fields, .. } => fields,
        other => panic!("expected Record, got {other:?}"),
    }
}

fn as_record_update(e: &Expr) -> (&Expr, &Vec<RecordField>) {
    match e {
        Expr::RecordUpdate {
            receiver, fields, ..
        } => (receiver, fields),
        other => panic!("expected RecordUpdate, got {other:?}"),
    }
}

fn as_list(e: &Expr) -> &Vec<Expr> {
    match e {
        Expr::List { items, .. } => items,
        other => panic!("expected List, got {other:?}"),
    }
}

fn as_tuple(e: &Expr) -> &Vec<TupleItem> {
    match e {
        Expr::Tuple { items, .. } => items,
        other => panic!("expected Tuple, got {other:?}"),
    }
}

fn as_binary(e: &Expr) -> (BinOp, &Expr, &Expr) {
    match e {
        Expr::Binary { op, lhs, rhs, .. } => (*op, lhs, rhs),
        other => panic!("expected Binary, got {other:?}"),
    }
}

fn as_apply(e: &Expr) -> (&Expr, &Expr) {
    match e {
        Expr::Apply { func, arg, .. } => (func, arg),
        other => panic!("expected Apply, got {other:?}"),
    }
}

fn as_pipeline(e: &Expr) -> (PipelineDir, &Expr, &Expr) {
    match e {
        Expr::Pipeline { dir, lhs, rhs, .. } => (*dir, lhs, rhs),
        other => panic!("expected Pipeline, got {other:?}"),
    }
}

fn as_access(e: &Expr) -> (&Expr, &str) {
    match e {
        Expr::Access {
            receiver, field, ..
        } => (receiver, field),
        other => panic!("expected Access, got {other:?}"),
    }
}

fn field_val<'a>(rec: &'a [RecordField], name: &str) -> &'a Expr {
    rec.iter()
        .find(|f| f.name == name)
        .map(|f| &f.value)
        .unwrap_or_else(|| panic!("field {name:?} not found"))
}

fn decl_by<'a>(file: &'a File, name: &str) -> &'a Decl {
    file.decls
        .iter()
        .find(|d| d.name() == name)
        .unwrap_or_else(|| panic!("decl {name:?} not found"))
}

fn as_inferred(d: &Decl) -> (&str, &Expr) {
    match d {
        Decl::Inferred { name, value, .. } => (name, value),
        other => panic!("expected Inferred, got {other:?}"),
    }
}

fn as_typed(d: &Decl) -> (&str, &TypeExpr, &Expr) {
    match d {
        Decl::Typed {
            name, ty, value, ..
        } => (name, ty, value),
        other => panic!("expected Typed, got {other:?}"),
    }
}

fn as_function(d: &Decl) -> (&str, &Vec<TypeParam>, &TypeExpr, &Vec<FuncClause>) {
    match d {
        Decl::Function {
            name,
            params,
            sig,
            clauses,
            ..
        } => (name, params, sig, clauses),
        other => panic!("expected Function, got {other:?}"),
    }
}

fn as_alias(d: &Decl) -> (&str, &Vec<TypeParam>, &TypeExpr) {
    match d {
        Decl::TypeAlias {
            name, params, ty, ..
        } => (name, params, ty),
        other => panic!("expected TypeAlias, got {other:?}"),
    }
}

mod constraints;
mod display;
mod expr;
mod fixtures_and_diagnostics;
mod types_and_decls;
mod universe_levels;
mod v1_and_lexer;
