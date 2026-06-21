use super::*;

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
    let f = parse_str("x ::= 42\n42");
    assert_eq!(f.decls.len(), 1);
    let (name, val) = as_inferred(decl_by(&f, "x"));
    assert_eq!(name, "x");
    assert_eq!(as_int(val), 42);
}

#[test]
fn parse_top_level_colon_equal_rejected() {
    assert!(parse("x := 42\nx").has_errors());
}

#[test]
fn parse_typed_decl() {
    let f = parse_str("port :: Int = 8080\n8080");
    let (name, _ty, val) = as_typed(decl_by(&f, "port"));
    assert_eq!(name, "port");
    assert_eq!(as_int(val), 8080);
}

#[test]
fn parse_import_decl_string() {
    let f = parse_str("lib :: import \"lib.zt\"\nlib");
    assert_eq!(f.decls.len(), 1);
    match decl_by(&f, "lib") {
        Decl::Import { name, source, .. } => {
            assert_eq!(name, "lib");
            assert!(matches!(source, ImportSource::String(s) if s == "lib.zt"));
        }
        other => panic!("expected Import, got {other:?}"),
    }
}

#[test]
fn parse_import_decl_path() {
    let f = parse_str("lib :: import lib.zt\nlib");
    assert_eq!(f.decls.len(), 1);
    match decl_by(&f, "lib") {
        Decl::Import { source, .. } => match source {
            ImportSource::Path(parts) => {
                assert_eq!(parts.len(), 2);
                assert_eq!(parts[0], "lib");
                assert_eq!(parts[1], "zt");
            }
            other => panic!("expected path import source, got {other:?}"),
        },
        other => panic!("expected Import, got {other:?}"),
    }
}

#[test]
fn expression_import_is_rejected() {
    assert!(parse("{ cfg := import \"config.zti\"; cfg }").has_errors());
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
    let src = "id :: Int -> Int\n  = x => x\n#unit";
    let f = parse_str(src);
    let (name, _params, _sig, clauses) = as_function(decl_by(&f, "id"));
    assert_eq!(name, "id");
    assert_eq!(clauses.len(), 1);
    assert_eq!(clauses[0].patterns.len(), 1);
}

#[test]
fn parse_polymorphic_function_decl() {
    let src = "id :: <A> A -> A\n  = x => x;\n#unit";
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
    let file = parse_str("f :: (Int -> Int) -> Int -> Int\n  = x => x;\nf");
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
    let file = parse_str("f :: (Int, Text) -> Int\n  = _ => 0;\nf");
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
    let file = parse_str("f :: () -> Int\n  = _ => 0;\nf");
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
