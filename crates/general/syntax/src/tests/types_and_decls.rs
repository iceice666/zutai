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
fn type_access_span_reaches_selected_field() {
    let source = "type module.Config";
    let e = parse_expr_str(source);
    let Expr::TypeForm { ty, .. } = e else {
        panic!("expected TypeForm");
    };
    assert_eq!(ty.span(), crate::Span::new(5, source.len()));
}

#[test]
fn question_mark_value_suffix_does_not_steal_optional_type() {
    let e = parse_expr_str("type Int?");
    match &e {
        Expr::TypeForm { ty, .. } => assert!(matches!(ty.as_ref(), TypeExpr::Optional { .. })),
        other => panic!("expected TypeForm, got {other:?}"),
    }
}

#[test]
fn question_mark_value_suffix_does_not_steal_optional_record_field() {
    let e = parse_expr_str("type { port? : Int; }");
    match &e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Record { fields, .. } => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "port");
                assert!(fields[0].optional);
            }
            other => panic!("expected TyRecord, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }
}
#[test]
fn parse_type_union_in_record_field() {
    parse_str(r#"{ type_union = type {#a; #b; #c;}; }"#);
}

#[test]
fn reject_hyphenated_record_field() {
    assert!(!parse_kinds(r#"{ bad-name = 1; }"#).is_empty());
}

#[test]
fn parse_type_union_in_file() {
    parse_str(
        r#"
Foo :: type {#a; #b; #c;};
Foo
"#,
    );
}

#[test]
fn parse_recursive_union_alias_keeps_recursive_fields() {
    let file = parse_str(
        r#"
Tree :: type {
  #leaf;
  #node : { value : Int; left : Tree; right : Tree; };
};
Tree
"#,
    );

    let (name, params, ty) = as_alias(decl_by(&file, "Tree"));
    assert_eq!(name, "Tree");
    assert!(params.is_empty());

    let variants = match ty {
        TypeExpr::Union { variants, tail, .. } => {
            assert!(tail.is_none());
            assert_eq!(variants.len(), 2);
            variants
        }
        other => panic!("expected union alias body, got {other:?}"),
    };

    assert_eq!(variants[0].name, "leaf");
    assert!(variants[0].payload.is_none());

    assert_eq!(variants[1].name, "node");
    let fields = match variants[1].payload.as_deref() {
        Some(TypeExpr::Record { fields, tail, .. }) => {
            assert!(tail.is_none());
            assert_eq!(
                fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>(),
                ["value", "left", "right"]
            );
            fields
        }
        other => panic!("expected node record payload, got {other:?}"),
    };

    assert!(matches!(
        &fields[0].ty,
        TypeExpr::Ident { name, .. } if name == "Int"
    ));
    assert!(matches!(
        &fields[1].ty,
        TypeExpr::Ident { name, .. } if name == "Tree"
    ));
    assert!(matches!(
        &fields[2].ty,
        TypeExpr::Ident { name, .. } if name == "Tree"
    ));
}

#[test]
fn parse_type_forms_section() {
    parse_str(
        r#"{
  type_rec       = type { host : Text; port? : Int; };
  type_union     = type {#a; #b; #c;};
  type_tup       = type (#circle, radius : Float);
  type_arrow     = type Int -> Int -> Int;
  type_opt       = type Int?;
}"#,
    );
}

#[test]
fn parse_match_section() {
    parse_str(
        r#"{
  match_expr = match #prod {
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
fn parse_match_with_underscore_field() {
    parse_str(r#"{ match_expr = match #a { | #a => 1; }; }"#);
}

#[test]
fn parse_list_patterns_in_match() {
    parse_str(
        r#"
f :: List Int -> Int
  = {;} => 0;
  = {h; ...t} => h;
f
"#,
    );
}

#[test]
fn reject_invalid_list_pattern_shapes() {
    assert!(!parse_kinds("f {h; t} = 0;\nf").is_empty());
    assert!(!parse_kinds("f {...t} = 0;\nf").is_empty());
    assert!(!parse_kinds("f {h; ...t; extra} = 0;\nf").is_empty());
}

#[test]
fn parse_question_mark_destructure_binding() {
    let f = parse_str("{ head?; } ::= import stdlib.prelude;\nhead?");
    assert_eq!(f.decls.len(), 1);
    match &f.decls[0] {
        Decl::Destructure { fields, .. } => {
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].name, "head?");
        }
        other => panic!("expected destructure, got {other:?}"),
    }
}
// ---------------------------------------------------------------------------
// M2: top-level declarations
// ---------------------------------------------------------------------------

#[test]
fn parse_inferred_decl() {
    let f = parse_str("x ::= 42;\n42");
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
    let f = parse_str("port :: Int = 8080;\n8080");
    let (name, _ty, val) = as_typed(decl_by(&f, "port"));
    assert_eq!(name, "port");
    assert_eq!(as_int(val), 8080);
}

#[test]
fn parse_import_binding_string() {
    // `import` is an expression; a plain import binding is an inferred binding
    // whose value is an `Expr::Import`.
    let f = parse_str("lib ::= import \"lib.zt\";\nlib");
    assert_eq!(f.decls.len(), 1);
    match decl_by(&f, "lib") {
        Decl::Inferred { name, value, .. } => {
            assert_eq!(name, "lib");
            match value {
                Expr::Import { source, .. } => {
                    assert!(matches!(source, ImportSource::String(s) if s == "lib.zt"));
                }
                other => panic!("expected Import expr, got {other:?}"),
            }
        }
        other => panic!("expected Inferred, got {other:?}"),
    }
}

#[test]
fn parse_import_binding_path() {
    let f = parse_str("lib ::= import lib.zt;\nlib");
    assert_eq!(f.decls.len(), 1);
    match decl_by(&f, "lib") {
        Decl::Inferred { value, .. } => match value {
            Expr::Import {
                source: ImportSource::Path(parts),
                ..
            } => {
                assert_eq!(parts, &["lib", "zt"]);
            }
            other => panic!("expected path import expr, got {other:?}"),
        },
        other => panic!("expected Inferred, got {other:?}"),
    }
}

#[test]
fn import_destructures_in_one_binding() {
    // The unified form: destructure straight off an `import` expression.
    let f = parse_str("{ map; fold; } ::= import stdlib.stream;\nmap");
    assert_eq!(f.decls.len(), 1);
    match &f.decls[0] {
        Decl::Destructure { fields, value, .. } => {
            let names: Vec<_> = fields.iter().map(|field| field.name.as_str()).collect();
            assert_eq!(names, ["map", "fold"]);
            assert!(matches!(value, Expr::Import { .. }));
        }
        other => panic!("expected Destructure, got {other:?}"),
    }
}

#[test]
fn use_is_an_ordinary_identifier() {
    let f = parse_str("use ::= 1;\nuse");
    assert_eq!(f.decls.len(), 1);
    match decl_by(&f, "use") {
        Decl::Inferred { name, value, .. } => {
            assert_eq!(name, "use");
            assert_eq!(as_int(value), 1);
        }
        other => panic!("expected Inferred, got {other:?}"),
    }
}

#[test]
fn old_import_decl_form_is_rejected() {
    // `name :: import …` no longer exists — import is purely an expression.
    assert!(parse("lib :: import \"lib.zt\";\nlib").has_errors());
}

#[test]
fn parse_destructure_binding() {
    let f = parse_str("{ map; fold; filter; } ::= s;\ns");
    assert_eq!(f.decls.len(), 1);
    match &f.decls[0] {
        Decl::Destructure { fields, value, .. } => {
            let names: Vec<_> = fields.iter().map(|field| field.name.as_str()).collect();
            assert_eq!(names, ["map", "fold", "filter"]);
            assert!(matches!(value, Expr::Ident { .. }));
        }
        other => panic!("expected Destructure, got {other:?}"),
    }
}

#[test]
fn trailing_record_is_not_a_destructure() {
    // A `{ … }` record final-expression has no `::=`, so it stays the file's
    // value rather than being parsed as a destructuring binding.
    let f = parse_str("x ::= 5;\n{ a = x; b = x; }");
    assert_eq!(f.decls.len(), 1);
    assert!(matches!(f.final_expr, Expr::Record { .. }));
}

#[test]
fn parse_typed_decl_lambda_value() {
    let src = "\ndouble :: Int -> Int = \\x. x * 2;\n\ndouble 5\n";
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
    let f = parse_str("items :: List Int = {;};\nitems");
    let (_name, ty, _val) = as_typed(decl_by(&f, "items"));
    assert!(matches!(ty, TypeExpr::Apply { .. }));
}

#[test]
fn parse_type_alias() {
    let f = parse_str("Server :: type { host : Text; };\n#unit");
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
    let file = parse_str("T :: type { fn? : (Int -> Int)?; };\nT");
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
