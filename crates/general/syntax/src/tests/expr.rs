use super::*;
use crate::numlit::NumberType;

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
fn parse_numeric_type_postfixes() {
    match parse_expr_str("255u8") {
        Expr::Integer {
            value,
            postfix: Some(NumberType::U8),
            ..
        } => assert_eq!(value, 255),
        other => panic!("expected u8 integer literal, got {other:?}"),
    }

    match parse_expr_str("1f32") {
        Expr::Float {
            value,
            postfix: Some(NumberType::F32),
            ..
        } => assert_eq!(value, 1.0),
        other => panic!("expected f32 float literal, got {other:?}"),
    }

    match parse_expr_str("42i64") {
        Expr::Integer {
            value,
            postfix: Some(NumberType::I64),
            ..
        } => assert_eq!(value, 42),
        other => panic!("expected i64 integer literal, got {other:?}"),
    }
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
fn parse_record_update() {
    let e = parse_expr_str("cfg with { port = 8080; }");
    let (receiver, fields) = as_record_update(&e);
    assert_eq!(as_ident(receiver), "cfg");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "port");
    assert_eq!(as_int(&fields[0].value), 8080);
}

#[test]
fn parse_access_over_grouped_record_update() {
    let e = parse_expr_str("(cfg with { port = 8080; }).port");
    let (receiver, field) = as_access(&e);
    assert_eq!(field, "port");
    let (update_receiver, fields) = as_record_update(receiver);
    assert_eq!(as_ident(update_receiver), "cfg");
    assert_eq!(fields[0].name, "port");
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
fn parse_stream_generator_yields_values() {
    let e = parse_expr_str("stream { yield 1; yield 2; }");
    let Expr::Generator { yields, .. } = e else {
        panic!("expected generator, got {e:?}");
    };
    assert_eq!(yields.len(), 2);
    assert_eq!(as_int(&yields[0]), 1);
    assert_eq!(as_int(&yields[1]), 2);
}

#[test]
fn stream_without_generator_block_remains_identifier() {
    assert_eq!(as_ident(&parse_expr_str("stream")), "stream");
}

#[test]
fn stream_record_application_is_not_generator() {
    let e = parse_expr_str("stream { value = 1; }");
    let (func, arg) = as_apply(&e);
    assert_eq!(as_ident(func), "stream");
    assert_eq!(as_int(&as_record(arg)[0].value), 1);
}

#[test]
fn stream_record_with_yield_field_is_not_generator() {
    let e = parse_expr_str("stream { yield = 1; }");
    let (func, arg) = as_apply(&e);
    assert_eq!(as_ident(func), "stream");
    assert_eq!(as_int(&as_record(arg)[0].value), 1);
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
fn tight_minus_after_field_access_is_subtraction() {
    let e = parse_expr_str("cfg.target-triple");
    let (op, lhs, rhs) = as_binary(&e);
    assert_eq!(op, BinOp::Sub);
    let (recv, field) = as_access(lhs);
    assert_eq!(as_ident(recv), "cfg");
    assert_eq!(field, "target");
    assert_eq!(as_ident(rhs), "triple");
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

#[test]
fn parse_block_typed_local_binding() {
    let e = parse_expr_str("{ x : Int = 1; x }");
    match &e {
        Expr::Block { bindings, .. } => {
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].name, "x");
            assert!(matches!(
                &bindings[0].annotation,
                Some(TypeExpr::Ident { name, .. }) if name == "Int"
            ));
        }
        other => panic!("expected Block, got {other:?}"),
    }
}

#[test]
fn deeply_nested_parens_parse_without_exponential_blowup() {
    // Regression: group-vs-tuple disambiguation used to try `parse_tuple`, fail,
    // backtrack, and re-parse the inner expression as a group — re-parsing it
    // twice per nesting level, i.e. O(2^n). 40 levels is ~2^40 re-parses under the
    // old code (it would hang for hours) but parses instantly now. Kept modest so
    // the O(depth) recursive-descent stack stays well within the test thread.
    let depth = 40;
    let src = format!("{}42{}", "(".repeat(depth), ")".repeat(depth));
    assert_eq!(as_int(&parse_expr_str(&src)), 42);
}

// ---------------------------------------------------------------------------
// Fix #02: u64 literals above i64::MAX
// ---------------------------------------------------------------------------

/// u64::MAX (18446744073709551615) must parse successfully with postfix U64.
/// The stored `value` is the two's-complement bit pattern, i.e. u64::MAX as i64 == -1.
#[test]
fn parse_u64_max_literal() {
    match parse_expr_str("18446744073709551615u64") {
        Expr::Integer {
            value,
            postfix: Some(NumberType::U64),
            ..
        } => assert_eq!(
            value,
            u64::MAX as i64,
            "expected u64::MAX as i64 (= -1), got {value}"
        ),
        other => panic!("expected U64 integer literal, got {other:?}"),
    }
}

/// 9223372036854775808u64 (i64::MIN magnitude) stores as i64::MIN bit pattern.
#[test]
fn parse_u64_above_i64_max() {
    match parse_expr_str("9223372036854775808u64") {
        Expr::Integer {
            value,
            postfix: Some(NumberType::U64),
            ..
        } => assert_eq!(
            value,
            i64::MIN,
            "expected i64::MIN bit pattern, got {value}"
        ),
        other => panic!("expected U64 integer literal, got {other:?}"),
    }
}

/// Bare integer overflow (no postfix) is still a parse error.
#[test]
fn parse_bare_i64_overflow_still_fails() {
    assert!(
        !parse_kinds("9223372036854775808").is_empty(),
        "expected parse error for bare i64 overflow"
    );
}

/// u64::MAX + 1 (= 2^64) still fails even with u64 postfix.
#[test]
fn parse_u64_overflow_still_fails() {
    assert!(
        !parse_kinds("18446744073709551616u64").is_empty(),
        "expected parse error for u64 overflow"
    );
}

/// Smaller unsigned widths still use signed parsing; huge overshoots remain
/// parse errors instead of wrapping into negative THIR range diagnostics.
#[test]
fn parse_huge_u8_overflow_still_fails() {
    assert!(
        !parse_kinds("10000000000000000000u8").is_empty(),
        "expected parse error for huge u8 overflow"
    );
}

// ---------------------------------------------------------------------------
// Fix #04: then/else-prefixed identifiers as application arguments
// ---------------------------------------------------------------------------

/// `f thenResult` must parse as application Apply(Ident("f"), Ident("thenResult")).
#[test]
fn parse_application_then_prefixed_identifier_arg() {
    let e = parse_expr_str("f thenResult");
    let (func, arg) = as_apply(&e);
    assert_eq!(as_ident(func), "f");
    assert_eq!(as_ident(arg), "thenResult");
}

/// `f elseValue` must parse as application.
#[test]
fn parse_application_else_prefixed_identifier_arg() {
    let e = parse_expr_str("f elseValue");
    let (func, arg) = as_apply(&e);
    assert_eq!(as_ident(func), "f");
    assert_eq!(as_ident(arg), "elseValue");
}

/// Identifiers sharing only the prefix of `then`/`else` must all parse as
/// application arguments: `then_x` (underscore suffix), `thenable` (letter
/// suffix), `elsewhere` (else + letter).
#[test]
fn parse_application_then_else_edge_identifiers() {
    for (src, func_name, arg_name) in [
        ("f then_x", "f", "then_x"),
        ("f thenable", "f", "thenable"),
        ("f elsewhere", "f", "elsewhere"),
    ] {
        let e = parse_expr_str(src);
        let (func, arg) = as_apply(&e);
        assert_eq!(as_ident(func), func_name, "src = {src:?}");
        assert_eq!(as_ident(arg), arg_name, "src = {src:?}");
    }
}

/// Real `then`/`else` keywords still stop application: `if c then x else y`
/// parses as `Expr::If`, not as application of `if` to multiple args.
#[test]
fn parse_real_then_else_keywords_stop_application() {
    let e = parse_expr_str("if true then 1 else 2");
    assert!(
        matches!(e, Expr::If { .. }),
        "expected If expression, got {e:?}"
    );
}

/// `if f x then 1 else 2` — `can_start_atom` stops application at the real
/// `then` so the condition is `Apply(f, x)` and branches are 1 / 2.
#[test]
fn parse_if_application_in_cond_then_real_keywords() {
    let e = parse_expr_str("if f x then 1 else 2");
    match &e {
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            let (func, arg) = as_apply(cond);
            assert_eq!(as_ident(func), "f");
            assert_eq!(as_ident(arg), "x");
            assert_eq!(as_int(then_branch), 1);
            assert_eq!(as_int(else_branch), 2);
        }
        other => panic!("expected If, got {other:?}"),
    }
}
