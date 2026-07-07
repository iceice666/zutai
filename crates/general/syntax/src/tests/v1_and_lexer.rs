use super::*;

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
fn v1_explicit_row_spreads_parse() {
    let e = parse_expr_str("type { * Base; host : Text; }");
    match e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Record {
                fields,
                spreads,
                tail,
                ..
            } => {
                assert_eq!(fields[0].name, "host");
                assert!(tail.is_none());
                assert!(matches!(&spreads[0], RowSpread::Named { name, .. } if name == "Base"));
            }
            other => panic!("expected TyRecord, got {other:?}"),
        },
        other => panic!("expected TypeForm, got {other:?}"),
    }

    let e = parse_expr_str("type { * m.Base; #extra; }");
    match e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Union {
                variants,
                spreads,
                tail,
                ..
            } => {
                assert_eq!(variants[0].name, "extra");
                assert!(tail.is_none());
                let RowSpread::Qualified { path, .. } = &spreads[0] else {
                    panic!("expected qualified spread");
                };
                assert_eq!(
                    path.iter().map(String::as_str).collect::<Vec<_>>(),
                    ["m", "Base"]
                );
            }
            other => panic!("expected TyUnion, got {other:?}"),
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
    assert!(parse("T :: type { ...m.Base; }\n1").has_errors());
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

    let e = parse_expr_str("type { * Shape; #sphere: { radius : Float; }; }");
    match e {
        Expr::TypeForm { ty, .. } => match ty.as_ref() {
            TypeExpr::Union {
                variants,
                spreads,
                tail,
                ..
            } => {
                assert!(tail.is_none());
                assert!(matches!(&spreads[0], RowSpread::Named { name, .. } if name == "Shape"));
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
fn v1_value_spreads_parse() {
    let e = parse_expr_str("{ * base; port = 8080; }");
    let Expr::Record { items, .. } = e else {
        panic!("expected record spread literal");
    };
    assert!(matches!(items[0], RecordItem::Spread(_)));
    assert!(matches!(&items[1], RecordItem::Field(field) if field.name == "port"));

    let e = parse_expr_str("{ 1; * xs; 4; }");
    let Expr::List { items, .. } = e else {
        panic!("expected list spread literal");
    };
    assert!(matches!(items[0], ListItem::Item(_)));
    assert!(matches!(items[1], ListItem::Spread(_)));
    assert!(matches!(items[2], ListItem::Item(_)));

    let e = parse_expr_str("{ * x; }");
    assert!(matches!(e, Expr::SpreadOnly { .. }));

    assert!(parse("x ::= { a = 1; 2; };\nx").has_errors());
    assert!(parse("x ::= { 1; a = 2; };\nx").has_errors());
}

#[test]
fn v1_value_select_operator_preserves_field_order() {
    let e = parse_expr_str("server >>= { host; port; }");
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
fn v1_type_select_operator_preserves_field_order() {
    let e = parse_expr_str("type Server >>= { host; port; }");
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
    let f = parse_str("parse :: Text -> Config ! { fail ParseError }\n  = text => text;\nparse");
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
        "load :: FsRead -> Path -> Text ! { fs.read : Path -> Text, fail IOError }\n  = fs path => path;\nload",
    );
    let (_, _, sig, _) = as_function(decl_by(&f, "load"));
    assert!(format!("{sig:?}").contains("fs"));
}

#[test]
fn v1_effect_row_no_payload_and_signature_shapes() {
    let f = parse_str(
        r#"
Eff :: type Unit ! { tick; fs.read : Path -> Text; fail Error };
1
"#,
    );
    let Decl::TypeAlias { ty, .. } = decl_by(&f, "Eff") else {
        panic!("expected type alias");
    };
    let TypeExpr::Effect { effects, .. } = ty else {
        panic!("expected effect type");
    };
    assert_eq!(effects.ops[0].path, vec!["tick"]);
    assert_eq!(effects.ops[1].path, vec!["fs", "read"]);
    assert_eq!(effects.ops[2].path, vec!["fail"]);
    assert!(effects.ops[0].signature.is_none());
    assert!(effects.ops[0].payload.is_none());
    assert!(effects.ops[1].signature.is_some());
    assert!(effects.ops[2].payload.is_some());
}

#[test]
fn v1_effect_row_named_spread_before_tail_parses() {
    let f = parse_str(
        r#"
Eff :: <e> type Unit ! { * FsRead; ...e; };
1
"#,
    );
    let Decl::TypeAlias { ty, .. } = decl_by(&f, "Eff") else {
        panic!("expected type alias");
    };
    let TypeExpr::Effect { effects, .. } = ty else {
        panic!("expected effect type");
    };
    assert_eq!(effects.spreads.len(), 1);
    assert!(matches!(
        &effects.spreads[0],
        RowSpread::Named { name, .. } if name == "FsRead"
    ));
    assert!(matches!(
        effects.tail.as_ref(),
        Some(RowTail::Named { name, .. }) if name == "e"
    ));
}

#[test]
fn v1_effect_row_qualified_spread_before_tail_parses() {
    let f = parse_str(
        r#"
Eff :: <e> type Unit ! { * fs.ReadEffects; ...e; };
1
"#,
    );
    let Decl::TypeAlias { ty, .. } = decl_by(&f, "Eff") else {
        panic!("expected type alias");
    };
    let TypeExpr::Effect { effects, .. } = ty else {
        panic!("expected effect type");
    };
    assert_eq!(effects.spreads.len(), 1);
    let RowSpread::Qualified { path, .. } = &effects.spreads[0] else {
        panic!("expected qualified spread, got {:?}", effects.spreads[0]);
    };
    assert_eq!(
        path.iter().map(String::as_str).collect::<Vec<_>>(),
        ["fs", "ReadEffects"]
    );
    assert!(matches!(
        effects.tail.as_ref(),
        Some(RowTail::Named { name, .. }) if name == "e"
    ));
}

#[test]
fn v1_effect_row_requires_operation_separators() {
    assert!(parse("parse :: Text -> Config ! { fail ParseError warn Diagnostic }\n  = text => text;\nparse").has_errors());
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

    let e = parse_expr_str("! fail err");
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

    let e = parse_expr_str(
        "handle check cfg with { warn = \\diagnostic => { ! log diagnostic; ^ (); }; }",
    );
    match e {
        Expr::Handle { clauses, .. } => {
            assert_eq!(clauses[0].op, vec!["warn"]);
            assert!(format!("{:?}", clauses[0].body).contains("Resume"));
            assert!(format!("{:?}", clauses[0].body).contains("Perform"));
        }
        other => panic!("expected Handle, got {other:?}"),
    }
}

#[test]
fn v1_handle_with_clause_is_not_record_update() {
    let e = parse_expr_str(
        "handle check cfg with { warn = \\diagnostic => { perform log diagnostic; resume (); }; }",
    );
    match e {
        Expr::Handle { expr, clauses, .. } => {
            let (func, arg) = as_apply(&expr);
            assert_eq!(as_ident(func), "check");
            assert_eq!(as_ident(arg), "cfg");
            assert_eq!(clauses[0].op, vec!["warn"]);
        }
        other => panic!("expected Handle, got {other:?}"),
    }
}

#[test]
fn v1_config_names_parse_as_identifiers() {
    for name in ["Patch", "DeepPatch", "overlay", "overlayDeep"] {
        let e = parse_expr_str(name);
        assert_eq!(as_ident(&e), name);
    }

    let ty_source = "PatchAlias :: type Patch;\nDeepPatchAlias :: type DeepPatch;\nPatch";
    let f = parse_str(ty_source);
    let (_, _, ty) = as_alias(decl_by(&f, "PatchAlias"));
    assert!(matches!(ty, TypeExpr::Ident { name, .. } if name == "Patch"));
    let (_, _, ty) = as_alias(decl_by(&f, "DeepPatchAlias"));
    assert!(matches!(ty, TypeExpr::Ident { name, .. } if name == "DeepPatch"));
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

#[test]
fn tokenize_numeric_type_postfixes() {
    let tokens = tokenize("255u8");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, SyntaxKind::PostfixedNumber);
    assert_eq!(tokens[0].text, "255u8");

    let tokens = tokenize("1.5f32");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, SyntaxKind::PostfixedNumber);
    assert_eq!(tokens[0].text, "1.5f32");

    let tokens = tokenize("1foo");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, SyntaxKind::Error);
}

#[test]
fn tokenize_posit_type_postfixes() {
    for src in ["1p32", "1p64", "1.5e-2p32e3"] {
        let tokens = tokenize(src);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, SyntaxKind::PostfixedNumber, "{src}");
        assert_eq!(tokens[0].text, src);
    }
}

#[test]
fn posit_type_postfix_diagnostics() {
    for src in ["1p32", "1p64", "1.5e-2p32e3", "-1p32"] {
        let kinds = parse_kinds(src);
        assert!(
            kinds.is_empty(),
            "unexpected diagnostics for {src:?}: {kinds:?}"
        );
    }

    for src in ["1p32e32", "1p64e64", "1p32e01", "1p16"] {
        assert!(parse(src).has_errors(), "expected parse error for {src:?}");
    }
}

#[test]
fn number_type_name_round_trips_through_parse() {
    use crate::numlit::NumberType;
    // `name()` is the inverse of `parse()` for every canonical postfix run.
    for run in [
        "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "p32", "p64", "p32e3",
        "p64e5",
    ] {
        let ty = NumberType::parse(run).unwrap_or_else(|| panic!("postfix {run:?} should parse"));
        assert_eq!(ty.name(), run, "name() must round-trip {run:?}");
    }
    assert!(NumberType::parse("i128").is_none(), "i128 is not a postfix");
}

#[test]
fn parse_posit_number_type_postfix_edge_cases() {
    use crate::posit::{PositSpec, parse_posit_number_type_postfix as parse};
    // `e` exponent overflowing u8 is rejected.
    assert!(parse("p32e999").is_none(), "es 999 overflows u8");
    // `e` followed by a non-digit is rejected.
    assert!(parse("p32ex").is_none(), "es must start with a digit");
    // A non-32/64 width has no posit spec.
    assert!(parse("p16").is_none(), "p16 is not a supported width");
    // Missing the `p` prefix entirely.
    assert!(parse("q32").is_none(), "missing `p` prefix");
    // Default es (2) when no exponent suffix; consumes the whole run.
    assert_eq!(parse("p32"), Some((PositSpec::new(32, 2).unwrap(), 3)));
    // Digits stop at the first non-digit: `p32e3` is consumed, `x` is left.
    assert_eq!(parse("p32e3x"), Some((PositSpec::new(32, 3).unwrap(), 5)));
}

#[test]
fn parse_posit_type_name_edge_cases() {
    use crate::posit::{PositSpec, parse_posit_type_name as parse};
    // Non-32/64 width rejected.
    assert!(
        parse("Posit16").is_none(),
        "Posit16 is not a supported width"
    );
    // Non-`Posit` names rejected.
    assert!(parse("Int").is_none(), "Int is not a posit type name");
    // Trailing junk after a valid spec is rejected (must consume the whole name).
    assert!(parse("Posit32x").is_none(), "trailing junk rejected");
    assert_eq!(parse("Posit32"), PositSpec::new(32, 2));
    assert_eq!(parse("Posit64e5"), PositSpec::new(64, 5));
}

#[test]
fn numeric_field_access_is_not_a_postfix() {
    assert!(
        !parse("1.foo").has_errors(),
        "field access after an integer should parse"
    );
    assert!(
        parse("1.0foo").has_errors(),
        "float-looking unknown postfix should be rejected"
    );
}

#[test]
fn numeric_type_postfix_diagnostics() {
    for src in ["255u8", "3.14f64", "1e9f64"] {
        let kinds = parse_kinds(src);
        assert!(
            kinds.is_empty(),
            "unexpected diagnostics for {src:?}: {kinds:?}"
        );
    }

    for (src, expected) in [
        ("-1u8", ParseErrorKind::UnsignedPostfixOnNegative),
        ("1.0i64", ParseErrorKind::IntegerPostfixOnFloatLiteral),
        ("1e3u32", ParseErrorKind::IntegerPostfixOnFloatLiteral),
        ("1foo", ParseErrorKind::UnknownNumberPostfix),
        ("1_u8", ParseErrorKind::UnknownNumberPostfix),
        ("1i128", ParseErrorKind::UnknownNumberPostfix),
    ] {
        let kinds = parse_kinds(src);
        assert!(
            kinds.contains(&expected),
            "expected {expected:?} for {src:?}, got {kinds:?}"
        );
    }
}

#[test]
fn posit_literal_rounds_decimal_directly() {
    let expr = parse_expr_str("1.0000000000000001p64");
    let Expr::Posit { literal, .. } = expr else {
        panic!("expected posit literal");
    };
    assert_eq!(literal.bits, 0x4000_0000_0000_003a);
}

#[test]
fn posit_literal_decimal_edge_cases() {
    use crate::posit::{PositSpec, parse_posit_literal};

    let spec = PositSpec::new(32, 2).unwrap();
    assert_eq!(parse_posit_literal(spec, "0").unwrap().bits, 0);
    assert_eq!(parse_posit_literal(spec, "-0").unwrap().bits, 0);
    assert!(parse_posit_literal(spec, ".1").is_none());
    assert!(parse_posit_literal(spec, "1.").is_none());
    assert!(parse_posit_literal(spec, "1e").is_none());
    assert!(parse_posit_literal(spec, "-").is_none());
}

#[test]
fn posit_literal_extreme_scale_saturates_or_minpos() {
    use crate::posit::{PositSpec, parse_posit_literal};

    let spec = PositSpec::new(32, 2).unwrap();
    let huge_positive = parse_posit_literal(spec, "1e1000001").unwrap();
    let huge_negative = parse_posit_literal(spec, "1e-1000001").unwrap();
    assert!(huge_positive.bits > huge_negative.bits);
    assert_eq!(huge_negative.bits, 1);
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
        "{ } [ ] ( ) ; , . :: := : == => = |> || | <| <= < >= > >>= -> ?? ?. ? + - * / % && != ! ^ @ $\n",
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
    assert!(kinds.contains(&SyntaxKind::Percent), "% token");
    assert!(kinds.contains(&SyntaxKind::ColonColon), "::");
    assert!(kinds.contains(&SyntaxKind::KeywordSelect), "select keyword");
    assert!(kinds.contains(&SyntaxKind::SelectOperator), ">>=");
    assert!(kinds.contains(&SyntaxKind::Bang), "!");
    assert!(kinds.contains(&SyntaxKind::Caret), "^");
}

// ── Unicode escape coverage ───────────────────────────────────────────────────

/// A string with `\uXXXX` BMP escape exercises `parse_unicode_escape` and
/// `parse_u16_hex_escape` for the basic plane (U+0000–U+D7FF, U+E000–U+FFFF).
#[test]
fn parse_string_bmp_unicode_escape() {
    // A = 'A', a normal BMP codepoint (not a surrogate).
    // This exercises parse_unicode_escape's `other =>` arm and parse_u16_hex_escape.
    let file = parse_str("x ::= \"\\u0041\";\nx");
    let _ = file;
}

/// A surrogate-pair escape (`𐀀`) decodes to U+10000, exercising the
/// high-surrogate branch (0xD800..=0xDBFF) in `parse_unicode_escape`.
#[test]
fn parse_string_surrogate_pair_escape() {
    // \uD800 is a high surrogate; \uDC00 is a low surrogate.
    // Together they encode U+10000 via the surrogate-pair algorithm.
    let file = parse_str("x ::= \"\\uD800\\uDC00\";\nx");
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

// ── Fix #01: Unicode whitespace in lookahead heuristics ───────────────────────

/// U+00A0 (non-breaking space, 2 bytes) in the record-vs-block lookahead must
/// not panic. Pre-fix the `&tmp[1..]` slice cut at byte 1 mid-character.
#[test]
fn parse_record_lookahead_unicode_ws_no_panic() {
    let parsed = parse_ast_only("{ foo\u{00A0}= 1; }");
    assert!(
        parsed.diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        parsed.diagnostics()
    );
    let f = parsed.into_ast().expect("should parse without panic");
    let fields = as_record(&f.final_expr);
    assert_eq!(fields[0].name, "foo");
}

/// U+00A0 in the block let-binding lookahead must not panic.
#[test]
fn parse_block_let_lookahead_unicode_ws_no_panic() {
    let f = parse_ast_only("[ x\u{00A0}:= 1; x ]")
        .into_ast()
        .expect("should parse without panic");
    assert!(
        matches!(f.final_expr, Expr::Block { .. }),
        "expected Block, got {:?}",
        f.final_expr
    );
}

/// U+00A0 after an identifier in the decl-start lookahead must not panic.
#[test]
fn parse_decl_lookahead_unicode_ws_no_panic() {
    let f = parse_ast_only("foo\u{00A0}:: Int = 42;\n42")
        .into_ast()
        .expect("should parse without panic");
    let (name, _ty, val) = as_typed(decl_by(&f, "foo"));
    assert_eq!(name, "foo");
    assert_eq!(as_int(val), 42);
}

/// 2-byte (U+00A0) and 3-byte (U+2003, U+3000) Unicode whitespace — every
/// iteration would panic pre-fix; the 3-byte cases prove the skip advances
/// by the full char width (not a fixed 2 bytes).
#[test]
fn parse_unicode_ws_multi_byte_no_panic() {
    for ws in ['\u{00A0}', '\u{2003}', '\u{3000}'] {
        assert!(
            parse_ast_only(&format!("{{ foo{ws}= 1; }}"))
                .ast()
                .is_some(),
            "record with U+{:04X} produced no AST",
            ws as u32
        );
        assert!(
            parse_ast_only(&format!("[ x{ws}:= 1; x ]")).ast().is_some(),
            "block with U+{:04X} produced no AST",
            ws as u32
        );
        assert!(
            parse_ast_only(&format!("foo{ws}:: Int = 42;\n42"))
                .ast()
                .is_some(),
            "decl with U+{:04X} produced no AST",
            ws as u32
        );
    }
}

#[test]
fn parse_unicode_inline_ws_separates_top_level_application() {
    let f = parse_ast_only("id ::= \\x. x;\nid\u{00A0}1")
        .into_ast()
        .expect("NBSP should separate application");
    assert!(
        matches!(f.final_expr, Expr::Apply { .. }),
        "expected application, got {:?}",
        f.final_expr
    );
}

#[test]
fn parse_lambda_dot_allows_unicode_inline_whitespace() {
    let parsed = parse("id ::= \\x.\u{00A0}x;\nid 1");
    assert!(
        parsed.diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        parsed.diagnostics()
    );
}

#[test]
fn parse_unicode_comment_diagnostics_stay_on_char_boundaries() {
    let src = "-- 註解 —\n{1; 2}";
    let parsed = parse(src);
    let diag = parsed
        .diagnostics()
        .first()
        .expect("fixture should produce a parse diagnostic");
    assert_eq!(diag.kind, ParseErrorKind::MissingListItemSemicolon);
    let span = diag.primary_span();
    assert!(src.is_char_boundary(span.start as usize), "{span:?}");
    assert!(src.is_char_boundary(span.end as usize), "{span:?}");
}

#[test]
fn parse_unicode_comments_as_trivia() {
    let src = concat!(
        "-- 行註解 café 🚀\n",
        "--| 文件註解 日本語\n",
        "--[ 外層 🧪 --[ 內層 한글 ]-- 結束 ]--\n",
        "名前 ::= 41;\n",
        "名前 + 1\n",
    );
    let parsed = parse_ast_only(src);
    assert!(
        parsed.diagnostics().is_empty(),
        "{:?}",
        parsed.diagnostics()
    );
    assert!(parsed.ast().is_some());
}

#[test]
fn tokenize_unicode_whitespace_as_whitespace() {
    let tokens = tokenize("foo\u{3000}bar");
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == SyntaxKind::Whitespace && token.text == "\u{3000}"),
        "tokens: {tokens:?}"
    );
    assert!(
        tokens.iter().all(|token| token.kind != SyntaxKind::Unknown),
        "tokens: {tokens:?}"
    );
}

#[test]
fn parse_unicode_ident_atom_and_field_name() {
    let parsed = parse_ast_only("café ::= #café;\n{ café = café; }");
    assert!(
        parsed.diagnostics().is_empty(),
        "{:?}",
        parsed.diagnostics()
    );
    let ast = parsed.ast().expect("unicode program should parse");
    let rendered = format!("{ast:?}");
    assert!(rendered.contains("café"), "{rendered}");
}

#[test]
fn parse_cjk_identifier_without_case() {
    let parsed = parse_ast_only("名前 ::= 1;\n名前");
    assert!(
        parsed.diagnostics().is_empty(),
        "{:?}",
        parsed.diagnostics()
    );
    assert!(parsed.ast().is_some());
}
