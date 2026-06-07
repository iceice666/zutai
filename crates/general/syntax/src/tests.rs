use crate::ast::*;
use crate::error::ParseErrorKind;
use crate::parser::expr::parse_expr;
use crate::{LineIndex, SyntaxKind, parse, tokenize};

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

// ---------------------------------------------------------------------------
// Lexer / literals
// ---------------------------------------------------------------------------

#[test]
fn parse_integer() {
    assert_eq!(as_int(&parse_expr_str("42")), 42);
    assert_eq!(as_int(&parse_expr_str("-10")), -10);
    assert_eq!(as_int(&parse_expr_str("0")), 0);
}

#[test]
fn parse_float() {
    let f = as_float(&parse_expr_str("2.71"));
    assert!((f - 2.71_f64).abs() < 1e-10);
    let f2 = as_float(&parse_expr_str("1e9"));
    assert!((f2 - 1e9).abs() < 1.0);
    let f3 = as_float(&parse_expr_str("-2.5e-3"));
    assert!((f3 - (-2.5e-3_f64)).abs() < 1e-10);
}

#[test]
fn parse_string_simple() {
    assert_eq!(as_str_val(&parse_expr_str(r#""hello""#)), "hello");
    assert_eq!(
        as_str_val(&parse_expr_str(r#""line1\nline2""#)),
        "line1\nline2"
    );
    assert_eq!(as_str_val(&parse_expr_str(r#""tab\there""#)), "tab\there");
    assert_eq!(as_str_val(&parse_expr_str(r#"""""#)), "");
}

#[test]
fn parse_bool_literals() {
    assert!(matches!(parse_expr_str("true"), Expr::True(_)));
    assert!(matches!(parse_expr_str("false"), Expr::False(_)));
}

#[test]
fn parse_atom_hash() {
    assert_eq!(as_atom(&parse_expr_str("#prod")), "prod");
    assert_eq!(as_atom(&parse_expr_str("#x86_64-linux")), "x86_64-linux");
}

#[test]
fn parse_ident_simple() {
    assert_eq!(as_ident(&parse_expr_str("x")), "x");
    assert_eq!(as_ident(&parse_expr_str("someVar")), "someVar");
    assert_eq!(as_ident(&parse_expr_str("_private")), "_private");
}

#[test]
fn parse_ident_rejects_keywords() {
    crate::parser::lex::BASE_PTR.with(|c| c.set("type".as_ptr() as usize));
    let mut input = "type";
    use crate::parser::lex::parse_ident;
    assert!(parse_ident(&mut input).is_err());
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[test]
fn parse_line_comment() {
    // Line comment is stripped, integer follows
    let e = parse_expr_str("-- this is a comment\n42");
    assert_eq!(as_int(&e), 42);
}

#[test]
fn parse_block_comment() {
    let e = parse_expr_str("--[ nested --[ inner ]-- ]-- 99");
    assert_eq!(as_int(&e), 99);
}

// ---------------------------------------------------------------------------
// Record, list, tuple, group
// ---------------------------------------------------------------------------

#[test]
fn parse_empty_record() {
    let e = parse_expr_str("{}");
    let fields = as_record(&e);
    assert!(fields.is_empty());
}

#[test]
fn parse_record_value() {
    let e = parse_expr_str("{ host = \"localhost\"; port = 8080; }");
    let fields = as_record(&e);
    assert_eq!(fields.len(), 2);
    assert_eq!(as_str_val(field_val(fields, "host")), "localhost");
    assert_eq!(as_int(field_val(fields, "port")), 8080);
}

#[test]
fn parse_list_value() {
    let e = parse_expr_str("[1; 2; 3;]");
    let items = as_list(&e);
    assert_eq!(items.len(), 3);
    assert_eq!(as_int(&items[0]), 1);
    assert_eq!(as_int(&items[2]), 3);
}

#[test]
fn parse_empty_list() {
    let e = parse_expr_str("[]");
    let items = as_list(&e);
    assert!(items.is_empty());
}

#[test]
fn parse_empty_tuple() {
    let e = parse_expr_str("()");
    let items = as_tuple(&e);
    assert!(items.is_empty());
}

#[test]
fn parse_group_is_not_tuple() {
    // (expr) without comma is a group — unwraps to the inner expr
    let e = parse_expr_str("(42)");
    assert_eq!(as_int(&e), 42);
}

#[test]
fn parse_named_tuple() {
    let e = parse_expr_str("(#circle, radius = 5.0)");
    let items = as_tuple(&e);
    assert_eq!(items.len(), 2);
    match &items[0] {
        TupleItem::Positional(e) => assert_eq!(as_atom(e), "circle"),
        other => panic!("expected positional, got {other:?}"),
    }
    match &items[1] {
        TupleItem::Named { name, value, .. } => {
            assert_eq!(name, "radius");
            let f = as_float(value);
            assert!((f - 5.0).abs() < 1e-10);
        }
        other => panic!("expected named, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Field access
// ---------------------------------------------------------------------------

#[test]
fn parse_field_access() {
    let e = parse_expr_str("cfg.host");
    let (recv, field) = as_access(&e);
    assert_eq!(as_ident(recv), "cfg");
    assert_eq!(field, "host");
}

#[test]
fn parse_hyphenated_field_access() {
    let e = parse_expr_str("cfg.target-triple");
    let (recv, field) = as_access(&e);
    assert_eq!(as_ident(recv), "cfg");
    assert_eq!(field, "target-triple");
}

#[test]
fn parse_optional_chain() {
    let e = parse_expr_str("cfg?.port");
    match &e {
        Expr::OptAccess {
            receiver, field, ..
        } => {
            assert_eq!(as_ident(receiver), "cfg");
            assert_eq!(field, "port");
        }
        other => panic!("expected OptAccess, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Arithmetic / operator precedence
// ---------------------------------------------------------------------------

#[test]
fn parse_mul_binds_tighter_than_add() {
    // 1 + 2 * 3 → 1 + (2 * 3)
    let e = parse_expr_str("1 + 2 * 3");
    let (op, lhs, rhs) = as_binary(&e);
    assert_eq!(op, BinOp::Add);
    assert_eq!(as_int(lhs), 1);
    let (op2, l2, r2) = as_binary(rhs);
    assert_eq!(op2, BinOp::Mul);
    assert_eq!(as_int(l2), 2);
    assert_eq!(as_int(r2), 3);
}

#[test]
fn parse_left_assoc_add() {
    // 1 + 2 + 3 → (1 + 2) + 3
    let e = parse_expr_str("1 + 2 + 3");
    let (_, lhs, rhs) = as_binary(&e);
    assert_eq!(as_int(rhs), 3);
    let (_, l2, r2) = as_binary(lhs);
    assert_eq!(as_int(l2), 1);
    assert_eq!(as_int(r2), 2);
}

#[test]
fn parse_coalesce_right_assoc() {
    // a ?? b ?? c → a ?? (b ?? c)
    let e = parse_expr_str("a ?? b ?? c");
    let (op, _, rhs) = as_binary(&e);
    assert_eq!(op, BinOp::Coalesce);
    let (op2, _, _) = as_binary(rhs);
    assert_eq!(op2, BinOp::Coalesce);
}

#[test]
fn parse_comparison_non_assoc_error() {
    let mut input = "1 < 2 < 3";
    crate::parser::lex::BASE_PTR.with(|c| c.set(input.as_ptr() as usize));
    // Should fail — chained comparison is non-associative
    assert!(parse_expr(&mut input).is_err());
}

#[test]
fn parse_pipeline_forward() {
    let e = parse_expr_str("x |> f");
    let (dir, lhs, rhs) = as_pipeline(&e);
    assert_eq!(dir, PipelineDir::Forward);
    assert_eq!(as_ident(lhs), "x");
    assert_eq!(as_ident(rhs), "f");
}

#[test]
fn parse_pipeline_backward() {
    let e = parse_expr_str("f <| x");
    let (dir, _, _) = as_pipeline(&e);
    assert_eq!(dir, PipelineDir::Backward);
}

#[test]
fn parse_pipeline_mixed_rejected() {
    let mut input = "x |> f <| g";
    crate::parser::lex::BASE_PTR.with(|c| c.set(input.as_ptr() as usize));
    assert!(parse_expr(&mut input).is_err());
}

// ---------------------------------------------------------------------------
// Function application
// ---------------------------------------------------------------------------

#[test]
fn parse_application_left_assoc() {
    // f x y → (f x) y
    let e = parse_expr_str("f x y");
    let (func, arg) = as_apply(&e);
    let (func2, arg2) = as_apply(func);
    assert_eq!(as_ident(func2), "f");
    assert_eq!(as_ident(arg2), "x");
    assert_eq!(as_ident(arg), "y");
}

#[test]
fn parse_application_negative_int_argument() {
    let e = parse_expr_str("f -1");
    let (func, arg) = as_apply(&e);
    assert_eq!(as_ident(func), "f");
    assert_eq!(as_int(arg), -1);
}

#[test]
fn parse_application_negative_float_argument() {
    let e = parse_expr_str("f -2.5e-3");
    let (func, arg) = as_apply(&e);
    assert_eq!(as_ident(func), "f");
    assert!((as_float(arg) - (-2.5e-3_f64)).abs() < 1e-10);
}

#[test]
fn parse_spaced_minus_remains_subtraction() {
    let e = parse_expr_str("f - 1");
    let (op, lhs, rhs) = as_binary(&e);
    assert_eq!(op, BinOp::Sub);
    assert_eq!(as_ident(lhs), "f");
    assert_eq!(as_int(rhs), 1);
}

#[test]
fn parse_tight_minus_remains_subtraction() {
    let e = parse_expr_str("f-1");
    let (op, lhs, rhs) = as_binary(&e);
    assert_eq!(op, BinOp::Sub);
    assert_eq!(as_ident(lhs), "f");
    assert_eq!(as_int(rhs), 1);

    let e = parse_expr_str("1-2");
    let (op, lhs, rhs) = as_binary(&e);
    assert_eq!(op, BinOp::Sub);
    assert_eq!(as_int(lhs), 1);
    assert_eq!(as_int(rhs), 2);
}

#[test]
fn parse_multiply_by_negative_literal() {
    let e = parse_expr_str("x * -1");
    let (op, lhs, rhs) = as_binary(&e);
    assert_eq!(op, BinOp::Mul);
    assert_eq!(as_ident(lhs), "x");
    assert_eq!(as_int(rhs), -1);
}

// ---------------------------------------------------------------------------
// Lambda
// ---------------------------------------------------------------------------

#[test]
fn parse_lambda_simple() {
    let e = parse_expr_str(r"\x. x");
    match &e {
        Expr::Lambda { params, body, .. } => {
            assert_eq!(params.len(), 1);
            assert!(matches!(&params[0], Pattern::Ident { name, .. } if name == "x"));
            assert_eq!(as_ident(body), "x");
        }
        other => panic!("expected Lambda, got {other:?}"),
    }
}

#[test]
fn parse_lambda_multi_param() {
    let e = parse_expr_str(r"\x y. x");
    match &e {
        Expr::Lambda { params, .. } => assert_eq!(params.len(), 2),
        other => panic!("expected Lambda, got {other:?}"),
    }
}

#[test]
fn parse_lambda_no_space_before_dot_rejected() {
    // `\x.y` — no space before dot
    crate::parser::lex::BASE_PTR.with(|c| c.set(r"\x.y".as_ptr() as usize));
    let mut input = r"\x.y";
    assert!(parse_expr(&mut input).is_err());
}

// ---------------------------------------------------------------------------
// If / match
// ---------------------------------------------------------------------------

#[test]
fn parse_if_then_else() {
    let e = parse_expr_str("if true then 1 else 2");
    match &e {
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            assert!(matches!(cond.as_ref(), Expr::True(_)));
            assert_eq!(as_int(then_branch), 1);
            assert_eq!(as_int(else_branch), 2);
        }
        other => panic!("expected If, got {other:?}"),
    }
}

#[test]
fn parse_match_basic() {
    let e = parse_expr_str("match n { | 0 => #zero; | _ => #nonzero; }");
    match &e {
        Expr::Match { arms, .. } => {
            assert_eq!(arms.len(), 2);
            assert_eq!(arms[0].patterns.len(), 1);
            assert!(matches!(
                &arms[0].patterns[0],
                Pattern::Integer { value: 0, .. }
            ));
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

#[test]
fn parse_match_with_guard() {
    let e = parse_expr_str("match n { | x if x > 0 => #pos; | _ => #nonpos; }");
    match &e {
        Expr::Match { arms, .. } => {
            assert!(arms[0].guard.is_some());
            assert!(arms[1].guard.is_none());
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Block expression
// ---------------------------------------------------------------------------

#[test]
fn parse_block_expr() {
    let e = parse_expr_str("{ x := 1; x }");
    match &e {
        Expr::Block {
            bindings, result, ..
        } => {
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].name, "x");
            assert_eq!(as_ident(result), "x");
        }
        other => panic!("expected Block, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Type forms
// ---------------------------------------------------------------------------

#[test]
fn parse_type_form_record() {
    let e = parse_expr_str("type { host : Text; port? : Int; }");
    match &e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Record { fields, .. } => {
                assert_eq!(fields.len(), 2);
                assert!(!fields[0].optional);
                assert!(fields[1].optional);
            }
            other => panic!("expected TyRecord, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }
}

#[test]
fn parse_type_form_union() {
    let e = parse_expr_str("type [ #a; #b; #c; ]");
    match &e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Union { items, .. } => assert_eq!(items.len(), 3),
            other => panic!("expected TyUnion, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }
}

#[test]
fn parse_type_optional_postfix() {
    let e = parse_expr_str("type Int?");
    match &e {
        Expr::TypeForm { ty, .. } => assert!(matches!(ty.as_ref(), TypeExpr::Optional { .. })),
        other => panic!("expected TypeForm, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// M2: top-level declarations
// ---------------------------------------------------------------------------

#[test]
fn parse_inferred_decl() {
    let f = parse_str("x := 42\n42");
    assert_eq!(f.decls.len(), 1);
    let (name, val) = as_inferred(decl_by(&f, "x"));
    assert_eq!(name, "x");
    assert_eq!(as_int(val), 42);
}

#[test]
fn parse_typed_decl() {
    let f = parse_str("port :: Int = 8080\n8080");
    let (name, _ty, val) = as_typed(decl_by(&f, "port"));
    assert_eq!(name, "port");
    assert_eq!(as_int(val), 8080);
}

#[test]
fn parse_type_alias() {
    let f = parse_str("Server :: type { host : Text; }\n#unit");
    let (name, params, _ty) = as_alias(decl_by(&f, "Server"));
    assert_eq!(name, "Server");
    assert!(params.is_empty());
}

#[test]
fn parse_function_decl() {
    let src = "id :: Int -> Int {\n  | x => x;\n}\n#unit";
    let f = parse_str(src);
    let (name, _params, _sig, clauses) = as_function(decl_by(&f, "id"));
    assert_eq!(name, "id");
    assert_eq!(clauses.len(), 1);
    assert_eq!(clauses[0].patterns.len(), 1);
}

#[test]
fn parse_polymorphic_function_decl() {
    let src = "id :: <A> A -> A {\n  | x => x;\n}\n#unit";
    let f = parse_str(src);
    let (_, params, _, _) = as_function(decl_by(&f, "id"));
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "A");
}

#[test]
fn parse_final_only_expr() {
    let f = parse_str("42");
    assert!(f.decls.is_empty());
    assert_eq!(as_int(&f.final_expr), 42);
}

#[test]
fn parse_single_colon_binding_rejected() {
    // `name : Type = expr` with single colon should fail
    assert!(parse("x : Int = 5\n5").has_errors());
}

// ---------------------------------------------------------------------------
// Fixture smoke test (M1)
// ---------------------------------------------------------------------------

const EXPR_CORE: &str = include_str!("../../fixtures/expr_core.zt");
const VALID_CURSED_DISAMBIGUATION: &str =
    include_str!("../../fixtures/valid/cursed_disambiguation.zt");
const VALID_CURSED_OPERATORS: &str = include_str!("../../fixtures/valid/cursed_operators.zt");
const VALID_CURSED_PATTERNS: &str = include_str!("../../fixtures/valid/cursed_patterns.zt");
const INVALID_CHAINED_COMPARISON: &str =
    include_str!("../../fixtures/invalid/chained_comparison.zt");
const INVALID_LAMBDA_ARROW: &str = include_str!("../../fixtures/invalid/lambda_arrow.zt");
const INVALID_LAMBDA_TIGHT_DOT: &str = include_str!("../../fixtures/invalid/lambda_tight_dot.zt");
const INVALID_LIST_MISSING_SEMICOLON: &str =
    include_str!("../../fixtures/invalid/list_missing_semicolon.zt");
const INVALID_LOCAL_BINDING_MISSING_RESULT: &str =
    include_str!("../../fixtures/invalid/local_binding_missing_result.zt");
const INVALID_MIXED_PIPELINE: &str = include_str!("../../fixtures/invalid/mixed_pipeline.zt");
const INVALID_RECORD_FIELD_COLON: &str =
    include_str!("../../fixtures/invalid/record_field_colon.zt");
const INVALID_TOP_LEVEL_SINGLE_COLON: &str =
    include_str!("../../fixtures/invalid/top_level_single_colon.zt");
const INVALID_TYPE_FIELD_EQUALS: &str = include_str!("../../fixtures/invalid/type_field_equals.zt");

#[test]
fn parse_expr_core_fixture() {
    parse_str(EXPR_CORE);
}

#[test]
fn parse_cursed_fixture_variants() {
    for (name, src) in [
        (
            "valid/cursed_disambiguation.zt",
            VALID_CURSED_DISAMBIGUATION,
        ),
        ("valid/cursed_operators.zt", VALID_CURSED_OPERATORS),
        ("valid/cursed_patterns.zt", VALID_CURSED_PATTERNS),
    ] {
        let parsed = parse(src);
        if parsed.ast().is_none() {
            let msgs: Vec<_> = parsed
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect();
            panic!("parse({name}) failed:\n{}", msgs.join("\n"))
        }
    }
}

#[test]
fn reject_invalid_fixture_variants() {
    for (name, src) in [
        ("invalid/chained_comparison.zt", INVALID_CHAINED_COMPARISON),
        ("invalid/lambda_arrow.zt", INVALID_LAMBDA_ARROW),
        ("invalid/lambda_tight_dot.zt", INVALID_LAMBDA_TIGHT_DOT),
        (
            "invalid/list_missing_semicolon.zt",
            INVALID_LIST_MISSING_SEMICOLON,
        ),
        (
            "invalid/local_binding_missing_result.zt",
            INVALID_LOCAL_BINDING_MISSING_RESULT,
        ),
        ("invalid/mixed_pipeline.zt", INVALID_MIXED_PIPELINE),
        ("invalid/record_field_colon.zt", INVALID_RECORD_FIELD_COLON),
        (
            "invalid/top_level_single_colon.zt",
            INVALID_TOP_LEVEL_SINGLE_COLON,
        ),
        ("invalid/type_field_equals.zt", INVALID_TYPE_FIELD_EQUALS),
    ] {
        assert!(parse(src).has_errors(), "{name} parsed successfully");
    }
}

#[test]
fn invalid_fixtures_report_specific_error_kinds() {
    for (name, src, kind) in [
        (
            "invalid/chained_comparison.zt",
            INVALID_CHAINED_COMPARISON,
            ParseErrorKind::ChainedComparison,
        ),
        (
            "invalid/lambda_arrow.zt",
            INVALID_LAMBDA_ARROW,
            ParseErrorKind::LambdaArrow,
        ),
        (
            "invalid/lambda_tight_dot.zt",
            INVALID_LAMBDA_TIGHT_DOT,
            ParseErrorKind::LambdaDotNeedsWhitespace,
        ),
        (
            "invalid/list_missing_semicolon.zt",
            INVALID_LIST_MISSING_SEMICOLON,
            ParseErrorKind::MissingListItemSemicolon,
        ),
        (
            "invalid/local_binding_missing_result.zt",
            INVALID_LOCAL_BINDING_MISSING_RESULT,
            ParseErrorKind::MissingBlockResult,
        ),
        (
            "invalid/mixed_pipeline.zt",
            INVALID_MIXED_PIPELINE,
            ParseErrorKind::MixedPipeline,
        ),
        (
            "invalid/record_field_colon.zt",
            INVALID_RECORD_FIELD_COLON,
            ParseErrorKind::ValueRecordFieldUsesColon,
        ),
        (
            "invalid/top_level_single_colon.zt",
            INVALID_TOP_LEVEL_SINGLE_COLON,
            ParseErrorKind::TopLevelSingleColon,
        ),
        (
            "invalid/type_field_equals.zt",
            INVALID_TYPE_FIELD_EQUALS,
            ParseErrorKind::TypeRecordFieldUsesEquals,
        ),
    ] {
        let kinds = parse_kinds(src);
        assert_eq!(kinds.first(), Some(&kind), "{name}: {kinds:?}");
    }
}

#[test]
fn reports_multiple_common_diagnostics_in_source_order() {
    let parsed = parse(
        r#"
{
  a = 1 < 2 < 3;
  b = \x => x;
  c = [1; 2]
}
"#,
    );
    assert!(parsed.has_errors(), "source should fail");

    let kinds: Vec<_> = parsed
        .diagnostics()
        .iter()
        .map(|err| err.kind.clone())
        .collect();
    assert_eq!(
        kinds,
        vec![
            ParseErrorKind::ChainedComparison,
            ParseErrorKind::LambdaArrow,
            ParseErrorKind::MissingListItemSemicolon,
        ]
    );
    assert!(
        parsed
            .diagnostics()
            .windows(2)
            .all(|pair| pair[0].primary_span().start <= pair[1].primary_span().start)
    );
}

#[test]
fn lossless_cst_round_trips_source_text() {
    let src = "--| doc\nanswer := --[ nested --[ inner ]-- ]-- 42\nanswer";
    let parsed = parse(src);
    assert_eq!(parsed.syntax().to_string(), src);
}

#[test]
fn tokenizer_preserves_comments_and_keywords() {
    let tokens = tokenize("-- hi\nif true then #ok else #no");
    let kinds: Vec<_> = tokens.iter().map(|token| token.kind).collect();
    assert!(kinds.contains(&SyntaxKind::LineComment));
    assert!(kinds.contains(&SyntaxKind::KeywordIf));
    assert!(kinds.contains(&SyntaxKind::KeywordTrue));
    assert!(kinds.contains(&SyntaxKind::KeywordThen));
    assert!(kinds.contains(&SyntaxKind::KeywordElse));
    assert!(kinds.contains(&SyntaxKind::Atom));
}

#[test]
fn diagnostic_exposes_structured_fix() {
    let parsed = parse("x : Int = 5\n5");
    let diagnostic = parsed.diagnostics().first().expect("expected diagnostic");
    assert_eq!(diagnostic.kind, ParseErrorKind::TopLevelSingleColon);
    assert_eq!(diagnostic.code, "zutai::parse::top_level_single_colon");
    assert_eq!(diagnostic.fixes.len(), 1);
    assert_eq!(diagnostic.fixes[0].edits[0].replacement, "::");
}

#[test]
fn line_index_converts_byte_and_utf16_positions() {
    let index = LineIndex::new("a\né😀z");
    assert_eq!(index.line_col(0).line, 0);
    assert_eq!(index.line_col(2).line, 1);
    assert_eq!(index.line_col(2).col, 0);
    let offset = "a\né😀".len();
    let utf16 = index.utf16_line_col(offset);
    assert_eq!(utf16.line, 1);
    assert_eq!(utf16.col, 3);
}
