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
fn comparisons_around_logical_ops_are_not_chained() {
    // Regression: comparisons on both sides of a lower-precedence operator
    // (`&&`, `||`, `??`, pipelines) are independent operands, not a chain.
    // The diagnostic scanner used to flag `a < b && c < d` as a false chain.
    for src in [
        "1 < 2 && 3 < 4",
        "1 < 2 || 3 < 4",
        "1 < 2 && 3 < 4 && 5 < 6",
        "1 > 2 || 3 >= 4",
        "1 <= 2 && 3 != 4",
    ] {
        assert!(
            !parse_kinds(src).contains(&ParseErrorKind::ChainedComparison),
            "{src:?} should not be flagged as a chained comparison"
        );
    }
}

#[test]
fn genuine_chained_comparison_still_rejected() {
    // A comparison directly followed by another comparison (no looser-binding
    // operator between them) is still the non-associative error.
    for src in ["1 < 2 < 3", "1 > 2 > 3", "1 <= 2 <= 3", "1 != 2 != 3"] {
        assert!(
            parse_kinds(src).contains(&ParseErrorKind::ChainedComparison),
            "{src:?} should be a chained comparison error"
        );
    }
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
    let e = parse_expr_str("type {#a; #b; #c;}");
    match &e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Union { variants, .. } => assert_eq!(variants.len(), 3),
            other => panic!("expected TyUnion, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }
}

#[test]
fn parse_type_form_brackets_are_not_union() {
    let e = parse_expr_str("type [#a;]");
    match &e {
        Expr::TypeForm { ty, .. } => assert!(
            !matches!(ty.as_ref(), TypeExpr::Union { .. }),
            "bracketed type expressions must not parse as union syntax"
        ),
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

#[test]
fn parse_type_union_in_record_field() {
    parse_str(r#"{ type-union = type {#a; #b; #c;}; }"#);
}

#[test]
fn parse_type_union_in_file() {
    parse_str(
        r#"
Foo :: type {#a; #b; #c;}
Foo
"#,
    );
}

#[test]
fn parse_type_forms_section() {
    parse_str(
        r#"{
  type-rec       = type { host : Text; port? : Int; };
  type-union     = type {#a; #b; #c;};
  type-tup       = type (#circle, radius : Float);
  type-arrow     = type Int -> Int -> Int;
  type-opt       = type Int?;
}"#,
    );
}

#[test]
fn parse_match_section() {
    parse_str(
        r#"{
  match-expr = match #prod {
    | #dev  => 0;
    | #prod => 1;
    | _     => -1;
  };
}"#,
    );
}

#[test]
fn parse_match_in_record_minimal() {
    parse_str(r#"{ x = match #a { | #a => 1; }; }"#);
}

#[test]
fn parse_match_with_hyphen_field() {
    parse_str(r#"{ match-expr = match #a { | #a => 1; }; }"#);
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
fn parse_typed_decl_lambda_value() {
    let src = "\ndouble :: Int -> Int = \\x. x * 2\n\ndouble 5\n";
    let parsed = parse(src);
    assert!(
        !parsed.has_errors(),
        "parse errors: {:?}",
        parsed.diagnostics()
    );
    let f = parsed.into_ast().expect("should have AST");
    assert_eq!(f.decls.len(), 1);
}

#[test]
fn parse_type_application_in_typed_decl() {
    let f = parse_str("items :: List Int = []\nitems");
    let (_name, ty, _val) = as_typed(decl_by(&f, "items"));
    assert!(matches!(ty, TypeExpr::Apply { .. }));
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
// Type-grouping / parenthesized-type tests (Fix A)
// ---------------------------------------------------------------------------

#[test]
fn single_positional_type_paren_is_arrow_not_tuple() {
    // `(Int -> Int) -> Int -> Int` — the `(Int -> Int)` in the first position
    // should be an Arrow type, not a 1-element Tuple.
    let file = parse_str("f :: (Int -> Int) -> Int -> Int { | x => x; }\nf");
    let decl = decl_by(&file, "f");
    let (_, _, sig, _) = as_function(decl);
    // Top-level sig is `Arrow { from: (Int -> Int), to: (Int -> Int) }`.
    // After the fix, `from` must be TypeExpr::Arrow, never TypeExpr::Tuple.
    let TypeExpr::Arrow { from, .. } = sig else {
        panic!("expected Arrow sig, got {sig:?}");
    };
    assert!(
        matches!(from.as_ref(), TypeExpr::Arrow { .. }),
        "expected from-type to be Arrow (grouped type), got {:?}",
        from
    );
}

#[test]
fn optional_of_grouped_arrow_type_is_optional_arrow() {
    // `(Int -> Int)?` — the inner type should be Arrow, not a 1-element Tuple.
    let file = parse_str("T :: type { fn? : (Int -> Int)?; }\nT");
    let decl = decl_by(&file, "T");
    let (_, _, ty) = as_alias(decl);
    // Find the field type inside the record.
    let TypeExpr::Record { fields, .. } = ty else {
        panic!("expected Record alias, got {ty:?}");
    };
    let field = fields
        .iter()
        .find(|f| f.name == "fn")
        .expect("field `fn` not found");
    // Field type is `(Int -> Int)?` = Optional(Arrow(..))
    let TypeExpr::Optional { inner, .. } = &field.ty else {
        panic!("expected Optional field type, got {:?}", field.ty);
    };
    assert!(
        matches!(inner.as_ref(), TypeExpr::Arrow { .. }),
        "expected Arrow inside Optional, got {:?}",
        inner
    );
}

#[test]
fn two_element_paren_type_is_tuple() {
    // `(Int, Text)` must still be a 2-element Tuple.
    let file = parse_str("f :: (Int, Text) -> Int { | _ => 0; }\nf");
    let decl = decl_by(&file, "f");
    let (_, _, sig, _) = as_function(decl);
    let TypeExpr::Arrow { from, .. } = sig else {
        panic!("expected Arrow sig, got {sig:?}");
    };
    assert!(
        matches!(from.as_ref(), TypeExpr::Tuple { items, .. } if items.len() == 2),
        "expected 2-element Tuple, got {:?}",
        from
    );
}

#[test]
fn empty_type_paren_is_empty_tuple() {
    // `()` must still be an empty Tuple (unit type).
    let file = parse_str("f :: () -> Int { | _ => 0; }\nf");
    let decl = decl_by(&file, "f");
    let (_, _, sig, _) = as_function(decl);
    let TypeExpr::Arrow { from, .. } = sig else {
        panic!("expected Arrow sig, got {sig:?}");
    };
    assert!(
        matches!(from.as_ref(), TypeExpr::Tuple { items, .. } if items.is_empty()),
        "expected empty Tuple, got {:?}",
        from
    );
}

// ---------------------------------------------------------------------------
// Fixture smoke test (M1)
// ---------------------------------------------------------------------------

const EXPR_CORE: &str = include_str!("../../fixtures/expr_core.zt");
const VALID_CURSED_DISAMBIGUATION: &str =
    include_str!("../../fixtures/valid/cursed_disambiguation.zt");
const VALID_CURSED_OPERATORS: &str = include_str!("../../fixtures/valid/cursed_operators.zt");
const VALID_CURSED_PATTERNS: &str = include_str!("../../fixtures/valid/cursed_patterns.zt");
const VALID_HIGHER_ORDER_FUNCTIONS: &str =
    include_str!("../../fixtures/valid/higher_order_functions.zt");
const VALID_DEEP_OPTIONALS: &str = include_str!("../../fixtures/valid/deep_optionals.zt");
const VALID_GENERIC_ALIASES: &str = include_str!("../../fixtures/valid/generic_aliases.zt");
const VALID_DOC_STALE_SYNTAX: &str = include_str!("../../fixtures/valid/doc_stale_syntax.zt");
const VALID_NESTED_MATCH: &str = include_str!("../../fixtures/valid/nested_match.zt");
const VALID_LARGE_PROGRAM: &str = include_str!("../../fixtures/valid/large_program.zt");
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
const INVALID_RECORD_PATTERN_MISSING_SEMICOLON: &str =
    include_str!("../../fixtures/invalid/record_pattern_missing_semicolon.zt");
const INVALID_TOP_LEVEL_SINGLE_COLON: &str =
    include_str!("../../fixtures/invalid/top_level_single_colon.zt");
const INVALID_TYPE_FIELD_EQUALS: &str = include_str!("../../fixtures/invalid/type_field_equals.zt");
const INVALID_UNCLOSED_RECORD: &str = include_str!("../../fixtures/invalid/unclosed_record.zt");
const INVALID_UNCLOSED_LIST: &str = include_str!("../../fixtures/invalid/unclosed_list.zt");
const INVALID_TRAILING_OPERATOR: &str = include_str!("../../fixtures/invalid/trailing_operator.zt");

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
        (
            "valid/higher_order_functions.zt",
            VALID_HIGHER_ORDER_FUNCTIONS,
        ),
        ("valid/deep_optionals.zt", VALID_DEEP_OPTIONALS),
        ("valid/generic_aliases.zt", VALID_GENERIC_ALIASES),
        ("valid/doc_stale_syntax.zt", VALID_DOC_STALE_SYNTAX),
        ("valid/nested_match.zt", VALID_NESTED_MATCH),
        ("valid/large_program.zt", VALID_LARGE_PROGRAM),
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
            "invalid/record_pattern_missing_semicolon.zt",
            INVALID_RECORD_PATTERN_MISSING_SEMICOLON,
        ),
        (
            "invalid/top_level_single_colon.zt",
            INVALID_TOP_LEVEL_SINGLE_COLON,
        ),
        ("invalid/type_field_equals.zt", INVALID_TYPE_FIELD_EQUALS),
        ("invalid/unclosed_record.zt", INVALID_UNCLOSED_RECORD),
        ("invalid/unclosed_list.zt", INVALID_UNCLOSED_LIST),
        ("invalid/trailing_operator.zt", INVALID_TRAILING_OPERATOR),
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
        (
            "invalid/unclosed_record.zt",
            INVALID_UNCLOSED_RECORD,
            ParseErrorKind::UnclosedDelimiter('{'),
        ),
        (
            "invalid/unclosed_list.zt",
            INVALID_UNCLOSED_LIST,
            ParseErrorKind::UnclosedDelimiter('['),
        ),
    ] {
        let kinds = parse_kinds(src);
        assert_eq!(kinds.first(), Some(&kind), "{name}: {kinds:?}");
    }
}

#[test]
fn ast_only_parse_matches_parse_diagnostics() {
    assert!(parse_ast_only("x := 1\nx").ast().is_some());
    assert_eq!(
        parse_ast_only_kinds(INVALID_MIXED_PIPELINE),
        parse_kinds(INVALID_MIXED_PIPELINE)
    );
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

// ---------------------------------------------------------------------------
// Constraint / witness parser tests (v1 syntax)
// ---------------------------------------------------------------------------

fn as_constraint(
    d: &Decl,
) -> (
    &str,
    &Vec<TypeParam>,
    &TypeExpr,
    &Vec<ConstraintMethod>,
    bool,
) {
    match d {
        Decl::Constraint {
            name,
            params,
            target,
            methods,
            derivable,
            ..
        } => (name, params, target, methods, *derivable),
        other => panic!("expected Constraint, got {other:?}"),
    }
}

fn as_witness(d: &Decl) -> (&str, &TypeExpr, &Vec<TypeParam>, &WitnessBody) {
    match d {
        Decl::Witness {
            constraint,
            target,
            params,
            body,
            ..
        } => (constraint, target, params, body),
        other => panic!("expected Witness, got {other:?}"),
    }
}

/// P1: basic constraint definition with one method
#[test]
fn p1_constraint_def_basic() {
    let f = parse_str("Eq :: <A> @A { eq :: A -> A -> Bool; }\n1");
    let d = decl_by(&f, "Eq");
    let (name, params, _target, methods, derivable) = as_constraint(d);
    assert_eq!(name, "Eq");
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "A");
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name.as_str(), "eq");
    assert!(!derivable);
}

/// P2: constraint with single bound `<A: Eq>`
#[test]
fn p2_single_bound() {
    let f = parse_str("Ord :: <A: Eq> @A { lt :: A -> A -> Bool; }\n1");
    let d = decl_by(&f, "Ord");
    let (_, params, _, _, _) = as_constraint(d);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "A");
    assert_eq!(params[0].bounds.len(), 1);
    assert_eq!(params[0].bounds[0].name, "Eq");
    // No spurious TopLevelSingleColon diagnostic
    assert!(parse_kinds("Ord :: <A: Eq> @A { lt :: A -> A -> Bool; }\n1").is_empty());
}

/// P3: multi-bound `<A: Eq + Show>`
#[test]
fn p3_multi_bound() {
    let f = parse_str("Hash :: <A: Eq + Show> @A { hash :: A -> Int; }\n1");
    let d = decl_by(&f, "Hash");
    let (_, params, _, _, _) = as_constraint(d);
    assert_eq!(params[0].bounds.len(), 2);
    assert_eq!(params[0].bounds[0].name, "Eq");
    assert_eq!(params[0].bounds[1].name, "Show");
}

/// P4: HKT kind annotation `<F :: Type -> Type>`
#[test]
fn p4_hkt_kind() {
    let f = parse_str("Functor :: <F :: Type -> Type> @F { map :: Int -> F Int; }\n1");
    let d = decl_by(&f, "Functor");
    let (_, params, _, _, _) = as_constraint(d);
    assert_eq!(params[0].name, "F");
    assert_eq!(params[0].bounds.len(), 0);
    assert!(params[0].kind.is_some());
}

/// P5: method with method-level type params `<A, B>`
#[test]
fn p5_method_level_params() {
    let f = parse_str("Conv :: <F> @F { convert :: <A, B> A -> F B; }\n1");
    let d = decl_by(&f, "Conv");
    let (_, _, _, methods, _) = as_constraint(d);
    assert_eq!(methods[0].params.len(), 2);
    assert_eq!(methods[0].params[0].name, "A");
    assert_eq!(methods[0].params[1].name, "B");
}

/// P6: operator method name `(<)`
#[test]
fn p6_operator_method() {
    let f = parse_str("Ord :: <A> @A { (<) :: A -> A -> Bool; }\n1");
    let d = decl_by(&f, "Ord");
    let (_, _, _, methods, _) = as_constraint(d);
    assert!(matches!(&methods[0].name, MethodName::Operator(s) if s == "<"));
}

/// P7: optional method `max?`
#[test]
fn p7_optional_method() {
    let f = parse_str("Ord :: <A> @A { lt :: A -> A -> Bool; max? :: A -> A -> A; }\n1");
    let d = decl_by(&f, "Ord");
    let (_, _, _, methods, _) = as_constraint(d);
    assert!(!methods[0].optional);
    assert!(methods[1].optional);
}

/// P8: trailing `derive` marker on constraint def
#[test]
fn p8_constraint_derivable() {
    let f = parse_str("Eq :: <A> @A { eq :: A -> A -> Bool; } derive\n1");
    let d = decl_by(&f, "Eq");
    let (_, _, _, _, derivable) = as_constraint(d);
    assert!(derivable);
}

/// P9: basic witness with field body
#[test]
fn p9_witness_basic() {
    let f = parse_str("Eq @Int :: { eq = \\ a b. a == b; }\n1");
    let d = decl_by(&f, "Eq");
    let (constraint, _target, params, body) = as_witness(d);
    assert_eq!(constraint, "Eq");
    assert!(params.is_empty());
    assert!(matches!(body, WitnessBody::Fields(fields) if fields.len() == 1));
    if let WitnessBody::Fields(fields) = body {
        assert_eq!(fields[0].name.as_str(), "eq");
    }
}

/// P10: conditional witness `<A: Eq>`
#[test]
fn p10_conditional_witness() {
    let f = parse_str("Eq @List :: <A: Eq> { eq = eqList; }\n1");
    let d = decl_by(&f, "Eq");
    let (_, _, params, _) = as_witness(d);
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "A");
    assert_eq!(params[0].bounds[0].name, "Eq");
    // No spurious TopLevelSingleColon diagnostic
    assert!(parse_kinds("Eq @List :: <A: Eq> { eq = eqList; }\n1").is_empty());
}

/// P11: derive body witness `:: derive`
#[test]
fn p11_derive_witness() {
    let f = parse_str("Eq @Server :: derive\n1");
    let d = decl_by(&f, "Eq");
    let (_, _, _, body) = as_witness(d);
    assert!(matches!(body, WitnessBody::Derive));
}

/// P12: operator witness field `(<) = ...`
#[test]
fn p12_operator_witness_field() {
    let f = parse_str("Ord @Int :: { (<) = intLt; }\n1");
    let d = decl_by(&f, "Ord");
    let (_, _, _, body) = as_witness(d);
    if let WitnessBody::Fields(fields) = body {
        assert!(matches!(&fields[0].name, MethodName::Operator(s) if s == "<"));
    } else {
        panic!("expected Fields body");
    }
}

/// P13: partial-application target `@(List A)` — paren-grouped
#[test]
fn p13_partial_app_target() {
    let f = parse_str("Eq @(List A) :: <A: Eq> { eq = eqList; }\n1");
    let d = decl_by(&f, "Eq");
    let (_, target, _, _) = as_witness(d);
    // Target should be Apply(List, A)
    assert!(matches!(target, TypeExpr::Apply { .. }));
}

/// P14: plain function with bound `contains :: <A: Eq>`  — zero pre-pass diagnostics
#[test]
fn p14_plain_fn_bound_no_diagnostic() {
    let kinds = parse_kinds("contains :: <A: Eq> List -> A -> Bool\n{ | xs x => false; }\n1");
    assert!(kinds.is_empty(), "expected no diagnostics, got {kinds:?}");
}

/// P15: `derive := 1` is still a normal inferred binding (D4 guard)
#[test]
fn p15_derive_as_normal_binding() {
    let f = parse_str("derive := 1\n1");
    let d = decl_by(&f, "derive");
    assert!(matches!(d, Decl::Inferred { .. }));
}

/// P16: every new form produces zero pre-pass diagnostics
#[test]
fn p16_no_prepass_diagnostics() {
    let forms = [
        "Eq :: <A> @A { eq :: A -> A -> Bool; }\n1",
        "Ord :: <A: Eq> @A { lt :: A -> A -> Bool; }\n1",
        "Hash :: <A: Eq + Show> @A { hash :: A -> Int; }\n1",
        "Functor :: <F :: Type -> Type> @F { map :: Int -> F Int; }\n1",
        "Eq @Int :: { eq = intEq; }\n1",
        "Eq @List :: <A: Eq> { eq = eqList; }\n1",
        "Eq @Server :: derive\n1",
        "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\n1",
    ];
    for src in &forms {
        let kinds = parse_kinds(src);
        assert!(
            kinds.is_empty(),
            "unexpected diagnostics for {src:?}: {kinds:?}"
        );
    }
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

// ============================================================================
// Display (display.rs) – exhaust every branch of write_decl / write_clause /
// write_expr / write_pattern / write_type_expr.
// ============================================================================

// ─── File header ─────────────────────────────────────────────────────────────

#[test]
fn display_file_starts_with_file_header() {
    let s = parse_str("42").to_string();
    assert!(s.starts_with("File\n"), "output must start with 'File\\n'");
}

// ─── Decl variants ───────────────────────────────────────────────────────────

#[test]
fn display_decl_inferred() {
    let s = parse_str("x := 42\nx").to_string();
    assert!(s.contains("Inferred \"x\""), "inferred decl name");
    assert!(s.contains("Int(42)"), "inferred decl value");
}

#[test]
fn display_decl_typed() {
    let s = parse_str("x :: Int = 99\nx").to_string();
    assert!(s.contains("Typed \"x\""), "typed decl name");
    assert!(s.contains("TyIdent(Int)"), "typed decl type annotation");
    assert!(s.contains("Int(99)"), "typed decl value");
}

#[test]
fn display_decl_type_alias_no_params() {
    let s = parse_str("MyInt :: type Int\nMyInt").to_string();
    assert!(
        s.contains("TypeAlias \"MyInt\" <>"),
        "alias name and empty params"
    );
    assert!(s.contains("TyIdent(Int)"), "alias body");
}

#[test]
fn display_decl_type_alias_with_params() {
    let s = parse_str("Pair :: <A, B> type (A, B)\n1").to_string();
    assert!(
        s.contains("TypeAlias \"Pair\" <A, B>"),
        "alias with type params"
    );
}

#[test]
fn display_decl_function() {
    let s = parse_str("id :: Int -> Int {\n  | x => x;\n}\nid 1").to_string();
    assert!(s.contains("Function \"id\" <>"), "function decl name");
    assert!(s.contains("TyArrow"), "function signature");
    assert!(s.contains("Clause"), "function clause");
}

#[test]
fn display_decl_function_clause_with_guard() {
    let s =
        parse_str("pos :: Int -> Int {\n  | x if x > 0 => x;\n  | _ => 0;\n}\npos 3").to_string();
    assert!(s.contains("guard:"), "clause guard label");
    assert!(s.contains("Binary("), "guard binary expression");
}

#[test]
fn display_decl_nosig_fn() {
    let s = parse_str("f x = x\nf 1").to_string();
    assert!(s.contains("NoSigFn \"f\""), "no-sig fn name");
    assert!(s.contains("pat: Ident(x)"), "no-sig fn pattern");
    assert!(s.contains("body: Ident(x)"), "no-sig fn body");
}

#[test]
fn display_decl_constraint() {
    let s = parse_str("Eq :: <A> @A { eq :: A -> A -> Bool; }\n1").to_string();
    assert!(s.contains("Constraint \"Eq\""), "constraint name");
}

#[test]
fn display_decl_witness() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. true; }\n1";
    let s = parse_str(src).to_string();
    assert!(s.contains("Witness for \"Eq\""), "witness header");
}

// ─── Expr variants ───────────────────────────────────────────────────────────

#[test]
fn display_expr_true() {
    let s = parse_str("true").to_string();
    assert!(s.contains("final: true"), "true literal in final expr");
}

#[test]
fn display_expr_false() {
    let s = parse_str("false").to_string();
    assert!(s.contains("final: false"), "false literal in final expr");
}

#[test]
fn display_expr_integer() {
    let s = parse_str("123").to_string();
    assert!(s.contains("Int(123)"), "integer literal");
}

#[test]
fn display_expr_float() {
    let s = parse_str("3.14").to_string();
    assert!(s.contains("Float(3.14)"), "float literal");
}

#[test]
fn display_expr_string() {
    let s = parse_str("\"hello\"").to_string();
    assert!(s.contains("Str(\"hello\")"), "string literal");
}

#[test]
fn display_expr_atom() {
    let s = parse_str("#foo").to_string();
    assert!(s.contains("Atom(#foo)"), "atom literal");
}

#[test]
fn display_expr_tagged_value() {
    let s = parse_str("#some { val = 1; }").to_string();
    assert!(s.contains("TaggedValue(#some)"), "tagged value tag");
    assert!(s.contains("Record"), "tagged value payload is record");
}

#[test]
fn display_expr_tagged_tuple_payload() {
    let s = parse_str("#pair (1, \"x\")").to_string();
    assert!(s.contains("TaggedValue(#pair)"), "tagged value tag");
    assert!(s.contains("Tuple"), "tagged value payload is tuple");
    assert!(s.contains("Int(1)"), "first tuple payload element");
}

#[test]
fn display_expr_ident() {
    let s = parse_str("x := 1\nx").to_string();
    assert!(s.contains("Ident(x)"), "identifier expression");
}

#[test]
fn display_expr_record() {
    let s = parse_str("{ a = 1; b = 2; }").to_string();
    assert!(s.contains("Record"), "record expression");
    assert!(s.contains("a:"), "record field a");
    assert!(s.contains("b:"), "record field b");
}

#[test]
fn display_expr_tuple_positional() {
    let s = parse_str("(1, 2)").to_string();
    assert!(s.contains("Tuple"), "tuple expression");
    assert!(s.contains("Int(1)"), "first tuple element");
    assert!(s.contains("Int(2)"), "second tuple element");
}

#[test]
fn display_expr_tuple_named() {
    let s = parse_str("(x=1, y=2)").to_string();
    assert!(s.contains("Tuple"), "named tuple");
    assert!(s.contains("x="), "named tuple field x");
    assert!(s.contains("y="), "named tuple field y");
}

#[test]
fn display_expr_list() {
    let s = parse_str("[1; 2; 3;]").to_string();
    assert!(s.contains("List"), "list expression");
    assert!(s.contains("Int(1)"), "first list element");
}

#[test]
fn display_expr_block() {
    let s = parse_str("{ x := 1; x }").to_string();
    assert!(s.contains("Block"), "block expression");
    assert!(s.contains("x:"), "block binding");
    assert!(s.contains("result:"), "block result");
}

#[test]
fn display_expr_lambda() {
    let s = parse_str(r"\x. x").to_string();
    assert!(s.contains("Lambda"), "lambda expression");
    assert!(s.contains("param:"), "lambda param");
    assert!(s.contains("body:"), "lambda body");
}

#[test]
fn display_expr_if() {
    let s = parse_str("if true then 1 else 2").to_string();
    assert!(s.contains("If"), "if expression");
    assert!(s.contains("cond:"), "if condition");
    assert!(s.contains("then:"), "if then branch");
    assert!(s.contains("else:"), "if else branch");
}

#[test]
fn display_expr_match() {
    let s = parse_str("match 1 { | 1 => true; | _ => false; }").to_string();
    assert!(s.contains("Match"), "match expression");
    assert!(s.contains("on:"), "match scrutinee");
    assert!(s.contains("Clause"), "match arm");
}

#[test]
fn display_expr_import_string() {
    let s = parse_str("import \"data.zti\"").to_string();
    assert!(s.contains("Import(\"data.zti\")"), "string import");
}

#[test]
fn display_expr_import_path() {
    let s = parse_str("import foo.bar").to_string();
    assert!(s.contains("Import(foo.bar)"), "path import");
}

#[test]
fn display_expr_type_form() {
    let s = parse_str("type Int?").to_string();
    assert!(s.contains("TypeForm"), "type form expression");
    assert!(s.contains("TyOptional"), "type form contains optional type");
}

#[test]
fn display_expr_apply() {
    let s = parse_str("f 1").to_string();
    assert!(s.contains("Apply"), "application");
    assert!(s.contains("fn:"), "apply function");
    assert!(s.contains("arg:"), "apply argument");
}

#[test]
fn display_expr_access() {
    let s = parse_str("r := { a = 1; }\nr.a").to_string();
    assert!(s.contains("Access .a"), "field access");
}

#[test]
fn display_expr_opt_access() {
    let s = parse_str("r?.field").to_string();
    assert!(s.contains("OptAccess ?.field"), "optional field access");
}

#[test]
fn display_expr_binary() {
    let s = parse_str("1 + 2").to_string();
    assert!(s.contains("Binary("), "binary expression");
}

#[test]
fn display_expr_pipeline_forward() {
    let s = parse_str("1 |> f").to_string();
    assert!(s.contains("Pipeline(|>)"), "forward pipeline");
}

#[test]
fn display_expr_pipeline_backward() {
    let s = parse_str("f <| 1").to_string();
    assert!(s.contains("Pipeline(<|)"), "backward pipeline");
}

// ─── Pattern variants ─────────────────────────────────────────────────────────

#[test]
fn display_pattern_wildcard() {
    let s = parse_str("match 1 { | _ => 0; }").to_string();
    assert!(s.contains("pat: _"), "wildcard pattern");
}

#[test]
fn display_pattern_ident() {
    let s = parse_str("match 1 { | x => x; }").to_string();
    assert!(s.contains("pat: Ident(x)"), "ident pattern");
}

#[test]
fn display_pattern_true() {
    let s = parse_str("match true { | true => 1; | _ => 0; }").to_string();
    assert!(s.contains("pat: true"), "true pattern");
}

#[test]
fn display_pattern_false() {
    let s = parse_str("match false { | false => 0; | _ => 1; }").to_string();
    assert!(s.contains("pat: false"), "false pattern");
}

#[test]
fn display_pattern_integer() {
    let s = parse_str("match 42 { | 42 => true; | _ => false; }").to_string();
    assert!(s.contains("pat: Int(42)"), "integer pattern");
}

#[test]
fn display_pattern_float() {
    let s = parse_str("match 1.5 { | 1.5 => true; | _ => false; }").to_string();
    assert!(s.contains("pat: Float("), "float pattern");
}

#[test]
fn display_pattern_string() {
    let s = parse_str("match \"hi\" { | \"hi\" => 1; | _ => 0; }").to_string();
    assert!(s.contains("pat: Str("), "string pattern");
}

#[test]
fn display_pattern_atom() {
    let s = parse_str("match #ok { | #ok => 1; | _ => 0; }").to_string();
    assert!(s.contains("pat: Atom(#ok)"), "atom pattern");
}

#[test]
fn display_pattern_tagged_value() {
    let s = parse_str("match #some { v = 1; } { | #some { v = x; } => x; | _ => 0; }").to_string();
    assert!(s.contains("TaggedPat(#some)"), "tagged value pattern tag");
    assert!(s.contains("v="), "tagged pattern field");
}

#[test]
fn display_pattern_tagged_tuple_payload() {
    let s = parse_str("match #pair (1, 2) { | #pair (x, y) => x; | _ => 0; }").to_string();
    assert!(s.contains("TaggedPat(#pair)"), "tagged pattern tag");
    assert!(s.contains("0="), "first positional payload slot");
    assert!(s.contains("1="), "second positional payload slot");
}

#[test]
fn display_pattern_tuple_positional() {
    let s = parse_str("match (1, 2) { | (a, b) => a; }").to_string();
    assert!(s.contains("TuplePat"), "positional tuple pattern");
}

#[test]
fn display_pattern_tuple_named() {
    let s = parse_str("match (x=1, y=2) { | (x=a, y=b) => a; }").to_string();
    assert!(s.contains("TuplePat"), "named tuple pattern");
    assert!(s.contains("x="), "named tuple pattern field x");
    assert!(s.contains("y="), "named tuple pattern field y");
}

#[test]
fn display_pattern_record() {
    let s = parse_str("match { a = 1; } { | { a = x; } => x; }").to_string();
    assert!(s.contains("RecordPat"), "record pattern");
    assert!(s.contains("a="), "record pattern field");
}

// ─── TypeExpr variants ───────────────────────────────────────────────────────

#[test]
fn display_type_expr_ident() {
    let s = parse_str("x :: Int = 1\nx").to_string();
    assert!(s.contains("TyIdent(Int)"), "type ident");
}

#[test]
fn display_type_expr_atom() {
    let s = parse_str("x :: #ok = #ok\nx").to_string();
    assert!(s.contains("TyAtom(#ok)"), "type atom");
}

#[test]
fn display_type_expr_true() {
    let s = parse_str("x :: true = true\nx").to_string();
    assert!(s.contains("TyTrue"), "type literal true");
}

#[test]
fn display_type_expr_false() {
    let s = parse_str("x :: false = false\nx").to_string();
    assert!(s.contains("TyFalse"), "type literal false");
}

#[test]
fn display_type_expr_record_with_optional_field() {
    let s = parse_str("Point :: type { x : Int; y? : Text; }\n1").to_string();
    assert!(s.contains("TyRecord"), "type record");
    assert!(s.contains("x:"), "required field");
    assert!(s.contains("y?:"), "optional field");
}

#[test]
fn display_type_expr_union_with_and_without_payload() {
    let s = parse_str("Shape :: type {#circle; #rect: { w : Int; h : Int; };}\n1").to_string();
    assert!(s.contains("TyUnion"), "type union");
    assert!(s.contains("circle"), "bare union variant");
    assert!(s.contains("rect:"), "payload union variant");
}

#[test]
fn display_type_expr_tuple_positional() {
    let s = parse_str("f :: (Int, Text) -> Int { | _ => 0; }\nf").to_string();
    assert!(s.contains("TyTuple"), "positional type tuple");
}

#[test]
fn display_type_expr_tuple_named() {
    let s = parse_str("T :: type (x : Int, y : Text)\n1").to_string();
    assert!(s.contains("TyTuple"), "named type tuple");
    assert!(s.contains("x:"), "named tuple type field x");
}

#[test]
fn display_type_expr_optional() {
    let s = parse_str("x :: Int? = #none\nx").to_string();
    assert!(s.contains("TyOptional"), "optional type");
    assert!(s.contains("TyIdent(Int)"), "optional inner type");
}

#[test]
fn display_type_expr_arrow() {
    let s = parse_str("f :: Int -> Text { | _ => \"x\"; }\nf").to_string();
    assert!(s.contains("TyArrow"), "arrow type");
    assert!(s.contains("from:"), "arrow from");
    assert!(s.contains("to:"), "arrow to");
}

#[test]
fn display_type_expr_apply() {
    let s = parse_str("xs :: List Int = [1;]\nxs").to_string();
    assert!(s.contains("TyApply"), "type application");
}

#[test]
fn display_type_expr_access() {
    let s = parse_str("x :: Foo.Bar = x\nx").to_string();
    assert!(s.contains("TyAccess .Bar"), "type field access");
}

#[test]
fn display_type_expr_expr_escape() {
    // A numeric literal in type position falls through to ExprEscape.
    let s = parse_str("x :: 1 = 1\nx").to_string();
    assert!(s.contains("TyExprEscape"), "type expr escape");
    assert!(s.contains("Int(1)"), "escaped expression value");
}

// ── V1 parser frontend surface syntax ─────────────────────────────────────────

#[test]
fn v1_record_row_tails_parse() {
    let e = parse_expr_str("type { host : Text; ...; }");
    match e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Record { fields, tail, .. } => {
                assert_eq!(fields[0].name, "host");
                assert!(matches!(tail, Some(RowTail::Anonymous { .. })));
            }
            other => panic!("expected TyRecord, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }

    let e = parse_expr_str("type { host : Text; ...Rest; }");
    match e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Record { tail, .. } => {
                assert!(matches!(tail, Some(RowTail::Named { name, .. }) if name == "Rest"));
            }
            other => panic!("expected TyRecord, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }
}

#[test]
fn v1_row_tail_overlapping_record_field_rejected() {
    assert!(parse("T :: type { host : Text; ...host; }\n1").has_errors());
}

#[test]
fn v1_record_row_tail_must_be_last_and_unique() {
    assert!(parse("T :: type { ...Rest; host : Text; }\n1").has_errors());
    assert!(parse("T :: type { ...A; ...B; }\n1").has_errors());
}

#[test]
fn v1_union_payload_row_tail_rejected() {
    assert!(parse("T :: type { #ok: { value : Int; ...Rest; }; }\n1").has_errors());
}

#[test]
fn v1_union_row_tails_and_spreads_parse() {
    let e = parse_expr_str("type { #dev; #test; ...Rest; }");
    match e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Union { variants, tail, .. } => {
                assert_eq!(
                    variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
                    ["dev", "test"]
                );
                assert!(matches!(tail, Some(RowTail::Named { name, .. }) if name == "Rest"));
            }
            other => panic!("expected TyUnion, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }

    let e = parse_expr_str("type { ...Shape; #sphere: { radius : Float; }; }");
    match e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Union { variants, tail, .. } => {
                assert!(matches!(tail, Some(RowTail::Named { name, .. }) if name == "Shape"));
                assert_eq!(variants[0].name, "sphere");
                let payload = variants[0].payload.as_deref().expect("payload");
                assert!(
                    matches!(payload, TypeExpr::Record { fields, .. } if fields[0].name == "radius")
                );
            }
            other => panic!("expected TyUnion, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }

    let e = parse_expr_str("type { #point: (Int, Int); }");
    match e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Union { variants, .. } => {
                assert_eq!(variants[0].name, "point");
                let payload = variants[0].payload.as_deref().expect("payload");
                assert!(matches!(payload, TypeExpr::Tuple { items, .. } if items.len() == 2));
            }
            other => panic!("expected TyUnion, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }
}

#[test]
fn v1_value_select_preserves_field_order() {
    let e = parse_expr_str("select server { host; port; }");
    match e {
        Expr::Select {
            receiver, fields, ..
        } => {
            assert_eq!(as_ident(&receiver), "server");
            assert_eq!(
                fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
                ["host", "port"]
            );
        }
        other => panic!("expected Select, got {other:?}"),
    }
}

#[test]
fn v1_type_select_preserves_field_order() {
    let e = parse_expr_str("type select Server { host; port; }");
    match e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Select {
                receiver, fields, ..
            } => {
                assert!(
                    matches!(receiver.as_ref(), TypeExpr::Ident { name, .. } if name == "Server")
                );
                assert_eq!(
                    fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
                    ["host", "port"]
                );
            }
            other => panic!("expected TySelect, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }
}

#[test]
fn v1_effect_row_syntax_parses() {
    let f = parse_str("parse :: Text -> Config ! { fail ParseError } { | text => text; }\nparse");
    let (_, _, sig, _) = as_function(decl_by(&f, "parse"));
    let TypeExpr::Arrow { to, .. } = sig else {
        panic!("expected Arrow, got {sig:?}");
    };
    match to.as_ref() {
        TypeExpr::Effect { effects, .. } => {
            assert_eq!(effects.ops[0].path, vec!["fail"]);
            assert!(effects.ops[0].payload.is_some());
        }
        other => panic!("expected TyEffect, got {other:?}"),
    }

    let f = parse_str(
        "load :: FsRead -> Path -> Text ! { fs.read : Path -> Text, fail IOError } { | fs path => path; }\nload",
    );
    let (_, _, sig, _) = as_function(decl_by(&f, "load"));
    assert!(format!("{sig:?}").contains("fs"));
}

#[test]
fn v1_effect_row_requires_operation_separators() {
    assert!(parse("parse :: Text -> Config ! { fail ParseError warn Diagnostic } { | text => text; }\nparse").has_errors());
}

#[test]
fn v1_perform_handle_resume_parse() {
    let e = parse_expr_str("perform fail err");
    match e {
        Expr::Perform { op, arg, .. } => {
            assert_eq!(op, vec!["fail"]);
            assert_eq!(as_ident(&arg), "err");
        }
        other => panic!("expected Perform, got {other:?}"),
    }

    let e = parse_expr_str(
        "handle check cfg with { warn = \\diagnostic => { perform log diagnostic; resume (); }; }",
    );
    match e {
        Expr::Handle { clauses, .. } => {
            assert_eq!(clauses[0].op, vec!["warn"]);
            assert!(format!("{:?}", clauses[0].body).contains("Resume"));
        }
        other => panic!("expected Handle, got {other:?}"),
    }
}

#[test]
fn v1_reflection_builtins_parse_as_application() {
    let fields_expr = parse_expr_str("fields Server");
    let (func, arg) = as_apply(&fields_expr);
    assert_eq!(as_ident(func), "fields");
    assert_eq!(as_ident(arg), "Server");

    let schema_expr = parse_expr_str("schema Server");
    let (func, arg) = as_apply(&schema_expr);
    assert_eq!(as_ident(func), "schema");
    assert_eq!(as_ident(arg), "Server");
}

// ── Lexer coverage: v1 keywords, @, scientific notation, unknown token ────────

/// V1 future-reserved keywords produce their own SyntaxKind variants.
/// Tokenizing them exercises `consume_word` arms 21-25 and `from_raw` arms 21-25.
#[test]
fn tokenize_v1_keywords() {
    let tokens = tokenize("select perform handle with resume");
    let kinds: Vec<_> = tokens.iter().map(|t| t.kind).collect();
    assert!(kinds.contains(&SyntaxKind::KeywordSelect), "select");
    assert!(kinds.contains(&SyntaxKind::KeywordPerform), "perform");
    assert!(kinds.contains(&SyntaxKind::KeywordHandle), "handle");
    assert!(kinds.contains(&SyntaxKind::KeywordWith), "with");
    assert!(kinds.contains(&SyntaxKind::KeywordResume), "resume");
}

/// `@` tokenises as `SyntaxKind::At` — used in constraint/witness declarations.
/// Parsing a real witness program ensures `from_raw` arm 61 is also covered.
#[test]
fn tokenize_at_sign() {
    let tokens = tokenize("@");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, SyntaxKind::At);
}

/// Characters not in the lexer's known set produce `SyntaxKind::Unknown`.
/// `$` is not a valid Zutai token — exercises the `_ =>` arm in `next_kind`
/// and `from_raw` arm 60.
#[test]
fn tokenize_unknown_character() {
    let tokens = tokenize("$");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, SyntaxKind::Unknown);
}

/// Scientific-notation integers and floats — exercises the `e`/`E` branch
/// inside `consume_number` (lines 411-424 in syntax.rs).
#[test]
fn tokenize_scientific_notation() {
    // Integer with exponent → Float
    let tokens = tokenize("1e3");
    assert_eq!(tokens[0].kind, SyntaxKind::Float, "1e3 should be Float");

    // Float with negative exponent
    let tokens = tokenize("1.5e-2");
    assert_eq!(tokens[0].kind, SyntaxKind::Float, "1.5e-2 should be Float");

    // Positive exponent sign
    let tokens = tokenize("2e+4");
    assert_eq!(tokens[0].kind, SyntaxKind::Float, "2e+4 should be Float");
}

// ── parse_lossless: covers SyntaxKind::from_raw and kind_from_raw ─────────────

/// Calling `parse_lossless` and then iterating children with `.kind()` triggers
/// `Language::kind_from_raw` → `SyntaxKind::from_raw` for every token in the
/// input — the only way to cover those arms (the winnow AST path never calls them).
#[test]
fn parse_lossless_traversal_covers_from_raw() {
    // Source containing every token kind the lexer can produce.
    // Carefully ordered so operators aren't accidentally merged:
    //   - `::` before `:=` before bare `:`
    //   - `==` before `=>` before bare `=`
    //   - `??` before `?.` before bare `?`
    //   - `->` before bare `-`
    //   - `|>` `||` before bare `|`
    //   - `<|` `<=` before bare `<`
    //   - `>=` before bare `>`
    let src = concat!(
        // Keywords (13-25)
        "type match if then else import true false select perform handle with resume\n",
        // Punctuation and multi-char operators
        "{ } [ ] ( ) ; , . :: := : == => = |> || | <| <= < >= > -> ?? ?. ? + - * / && != @ $\n",
        // Comments (on their own line so the lexer doesn't swallow the operators above)
        "--[ block comment ]--\n",
        "--|  doc comment\n",
        "-- line comment\n",
        // Literals
        "42 1.5 \"hello\" #atom ident\n",
        // Whitespace + newlines are already implicit in the concat
    );
    let root = parse_lossless(src);
    // .kind() on the root triggers from_raw(0) = SourceFile
    let root_kind = root.kind();
    assert_eq!(root_kind, SyntaxKind::SourceFile);
    // Iterating children triggers from_raw for each token kind present in src
    let kinds: Vec<_> = root.children_with_tokens().map(|e| e.kind()).collect();
    assert!(!kinds.is_empty(), "expected tokens from parse_lossless");
    // Spot-check a few expected kinds
    assert!(kinds.contains(&SyntaxKind::KeywordType), "type keyword");
    assert!(kinds.contains(&SyntaxKind::Integer), "integer literal");
    assert!(kinds.contains(&SyntaxKind::At), "@ token");
    assert!(kinds.contains(&SyntaxKind::ColonColon), "::");
    assert!(kinds.contains(&SyntaxKind::KeywordSelect), "select keyword");
}

// ── Unicode escape coverage ───────────────────────────────────────────────────

/// A string with `\uXXXX` BMP escape exercises `parse_unicode_escape` and
/// `parse_u16_hex_escape` for the basic plane (U+0000–U+D7FF, U+E000–U+FFFF).
#[test]
fn parse_string_bmp_unicode_escape() {
    // A = 'A', a normal BMP codepoint (not a surrogate).
    // This exercises parse_unicode_escape's `other =>` arm and parse_u16_hex_escape.
    let file = parse_str("x := \"\\u0041\"\nx");
    let _ = file;
}

/// A surrogate-pair escape (`𐀀`) decodes to U+10000, exercising the
/// high-surrogate branch (0xD800..=0xDBFF) in `parse_unicode_escape`.
#[test]
fn parse_string_surrogate_pair_escape() {
    // \uD800 is a high surrogate; \uDC00 is a low surrogate.
    // Together they encode U+10000 via the surrogate-pair algorithm.
    let file = parse_str("x := \"\\uD800\\uDC00\"\nx");
    let _ = file;
}

/// A lone low surrogate (`\uDC00`) is invalid UTF-16 and causes
/// `parse_unicode_escape` to return `fail` — exercising the `0xDC00..=0xDFFF`
/// error arm. The surrounding string literal fails to parse.
#[test]
fn parse_string_lone_low_surrogate_is_parse_error() {
    // The lexer sees `\uDC00` inside a string, calls parse_unicode_escape which
    // hits the `0xDC00..=0xDFFF => fail` arm, causing a parse diagnostic.
    let diags = parse_kinds(r#""\uDC00""#);
    assert!(
        !diags.is_empty(),
        "expected parse error for lone low surrogate"
    );
}

/// An integer literal too large for i64 causes `parse_number_value` to fail
/// and backtrack (covers the `Err(_) => { *input = start; fail }` arm).
#[test]
fn parse_number_int_overflow_is_parse_error() {
    // 2^63 cannot be stored in i64 → parse_number_value backtracks.
    let diags = parse_kinds("9223372036854775808");
    // The parser fails to parse the oversized literal — a diagnostic is emitted.
    assert!(!diags.is_empty(), "expected parse error for i64 overflow");
}

/// An unclosed block comment reaching end-of-input triggers the
/// `if input.is_empty() { return fail }` branch inside `skip_block_comment`.
#[test]
fn parse_unclosed_block_comment_is_parse_error() {
    // After parsing `1`, the whitespace skipper tries to consume `--[…` but
    // never finds `]--`, hits EOF, and returns `fail`.
    let diags = parse_kinds("1 --[ this comment is never closed");
    assert!(
        !diags.is_empty(),
        "expected parse error for unclosed block comment"
    );
}
