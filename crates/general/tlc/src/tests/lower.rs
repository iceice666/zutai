// ── Additional lower/types.rs + lower/expr.rs + binop coverage ───────────────

use super::tlc_of;
use crate::*;
use zutai_syntax::posit::PositSpec;
use zutai_thir::FixedWidth;

#[test]
fn float_literal_lowers_to_prim_float_type() {
    let m = tlc_of("f :: Float = 1.5\nf");
    // lower_types.rs: TypeKind::Float → TlcType::Prim(PrimTy::Float)
    let has_float = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Prim(PrimTy::Float)));
    assert!(has_float, "expected Prim(Float) for Float type");
    // lower_expr.rs: ThirExprKind::Float → TlcExpr::Lit(Literal::Float)
    let has_lit = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Lit(Literal::Float(_))));
    assert!(has_lit, "expected Lit(Float) for float literal expression");
}

#[test]
fn fixed_width_literal_lowers_to_prim_fixed_num_type() {
    let m = tlc_of("255u8");
    let has_fixed = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Prim(PrimTy::FixedNum(FixedWidth::U8))));
    assert!(has_fixed, "expected Prim(FixedNum(U8)) for u8 literal");

    let has_lit = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Lit(Literal::Int(255))));
    assert!(has_lit, "expected Int literal payload for u8 literal");
}

#[test]
fn posit_literal_lowers_to_prim_posit_type() {
    let m = tlc_of("p :: Posit32e3 = 1.5p32e3\np");
    let has_posit_ty = m.type_arena.iter().any(|(_, ty)| {
        matches!(
            ty,
            TlcType::Prim(PrimTy::Posit(spec))
                if *spec == (PositSpec { nbits: 32, es: 3 })
        )
    });
    assert!(has_posit_ty, "expected Prim(Posit32e3) for posit type");

    let has_posit_lit = m.expr_arena.iter().any(|(_, e)| {
        matches!(
            e,
            TlcExpr::Lit(Literal::Posit(lit))
                if lit.spec == (PositSpec { nbits: 32, es: 3 })
        )
    });
    assert!(has_posit_lit, "expected Lit(Posit32e3) for posit literal");
}

#[test]
fn string_literal_lowers_to_prim_str_type() {
    let m = tlc_of(
        r#"s :: Text = "hello"
s"#,
    );
    // lower_types.rs: TypeKind::Text → TlcType::Prim(PrimTy::Str)
    let has_str_ty = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Prim(PrimTy::Str)));
    assert!(has_str_ty, "expected Prim(Str) for Text type");
    // lower_expr.rs: ThirExprKind::String → TlcExpr::Lit(Literal::Str)
    let has_lit = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Lit(Literal::Str(_))));
    assert!(has_lit, "expected Lit(Str) for string literal expression");
}

#[test]
fn bool_type_annotation_lowers_to_prim_bool() {
    let m = tlc_of("b :: Bool = true\nb");
    // lower_types.rs: TypeKind::Bool → TlcType::Prim(PrimTy::Bool)
    let has_bool = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Prim(PrimTy::Bool)));
    assert!(has_bool, "expected Prim(Bool) for Bool annotation");
}

#[test]
fn list_type_lowers_to_tlc_list() {
    let m = tlc_of("xs :: List Int = [1; 2; 3;]\nxs");
    // lower_types.rs: TypeKind::List(inner) → TlcType::List(inner_tlc)
    let has_list = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::List(_)));
    assert!(has_list, "expected TlcType::List for List Int");
}

#[test]
fn optional_type_lowers_to_tlc_optional() {
    let m = tlc_of("x :: Int? = #none\nx");
    // lower_types.rs: TypeKind::Optional(inner) → TlcType::Optional(inner_tlc)
    let has_opt = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Optional(_)));
    assert!(has_opt, "expected TlcType::Optional for Int?");
}

#[test]
fn positional_tuple_type_lowers_to_tlc_tuple() {
    let m = tlc_of(
        r#"p :: (Int, Text) = (1, "hi")
p"#,
    );
    // lower_types.rs: TypeKind::Tuple with Positional → TlcType::Tuple with TlcTupleField::Positional
    let has_tuple = m
        .type_arena
        .iter()
        .any(|(_, ty)| matches!(ty, TlcType::Tuple(_)));
    assert!(
        has_tuple,
        "expected TlcType::Tuple for positional tuple type"
    );
    let has_positional_field = m.type_arena.iter().any(|(_, ty)| {
        if let TlcType::Tuple(items) = ty {
            items
                .iter()
                .any(|i| matches!(i, TlcTupleField::Positional(_)))
        } else {
            false
        }
    });
    assert!(
        has_positional_field,
        "expected TlcTupleField::Positional inside Tuple"
    );
}

#[test]
fn named_tuple_type_lowers_to_tlc_tuple_with_named_fields() {
    let m = tlc_of("p :: (x : Int, y : Int) = (x = 1, y = 2)\np");
    // lower_types.rs: TypeKind::Tuple with Named → TlcType::Tuple with TlcTupleField::Named
    let has_named_field = m.type_arena.iter().any(|(_, ty)| {
        if let TlcType::Tuple(items) = ty {
            items
                .iter()
                .any(|i| matches!(i, TlcTupleField::Named { .. }))
        } else {
            false
        }
    });
    assert!(
        has_named_field,
        "expected TlcTupleField::Named inside Tuple for named tuple type"
    );
}

#[test]
fn float_pattern_lowers_to_lit_float_pat() {
    let m = tlc_of(
        r#"classify :: Float -> Text
  = 0.0 => "zero";
  = _ => "other";
classify 1.0"#,
    );
    // lower_expr.rs: ThirPatKind::Float(f) → TlcPat::Lit(Literal::Float(f))
    let has_float_pat = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Case(_, alts) = e {
            alts.iter()
                .any(|a| matches!(&a.pat, TlcPat::Lit(Literal::Float(_))))
        } else {
            false
        }
    });
    assert!(has_float_pat, "expected Lit(Float) pattern in Case alts");
}

#[test]
fn posit_pattern_lowers_to_lit_posit_pat() {
    let m = tlc_of(
        r#"classify :: Posit32e3 -> Text
  = 0p32e3 => "zero";
  = _ => "other";
classify 1p32e3"#,
    );
    let has_posit_pat = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Case(_, alts) = e {
            alts.iter().any(|a| {
                matches!(
                    &a.pat,
                    TlcPat::Lit(Literal::Posit(lit))
                        if lit.spec == (PositSpec { nbits: 32, es: 3 })
                )
            })
        } else {
            false
        }
    });
    assert!(
        has_posit_pat,
        "expected Lit(Posit32e3) pattern in Case alts"
    );
}

#[test]
fn string_pattern_lowers_to_lit_str_pat() {
    let m = tlc_of(
        r#"greet :: Text -> Int
  = "hello" => 1;
  = _ => 0;
greet "hi""#,
    );
    // lower_expr.rs: ThirPatKind::String(s) → TlcPat::Lit(Literal::Str(s))
    let has_str_pat = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Case(_, alts) = e {
            alts.iter()
                .any(|a| matches!(&a.pat, TlcPat::Lit(Literal::Str(_))))
        } else {
            false
        }
    });
    assert!(has_str_pat, "expected Lit(Str) pattern in Case alts");
}

#[test]
fn atom_pattern_bare_union_lowers_to_atom_pat() {
    // Bare union arm `#dev` / `#prod` (no payload) → ThirPatKind::Atom → TlcPat::Atom
    let m = tlc_of(
        r#"Profile :: type { #dev; #prod; }
isProd :: Profile -> Bool
  = #prod => true;
  = #dev => false;
isProd #prod"#,
    );
    // lower_expr.rs: ThirPatKind::Atom(s) → TlcPat::Atom(s)
    let has_atom_pat = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Case(_, alts) = e {
            alts.iter().any(|a| matches!(&a.pat, TlcPat::Atom(_)))
        } else {
            false
        }
    });
    assert!(
        has_atom_pat,
        "expected TlcPat::Atom for bare union arm patterns"
    );
}

#[test]
fn wildcard_lambda_param_uses_fresh_synthetic_binding() {
    // `\\ _ . body` — the `_` wildcard is ThirPatKind::Wildcard (non-Bind)
    // lower_lambda's else branch creates a fresh synthetic binding.
    let m = tlc_of("const42 :: Int -> Int = \\_ . 42\nconst42 1");
    let has_lam = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::Lam(_, _, _)));
    assert!(has_lam, "expected TlcExpr::Lam from wildcard-param lambda");
}

#[test]
fn optional_access_lowers_to_get_field() {
    // `cfg?.port` where cfg :: Config? → ThirExprKind::OptionalAccess → TlcExpr::GetField
    let m = tlc_of(
        "Config :: type { port : Int; }
cfg :: Config? = #none
n :: Int? = cfg?.port
n",
    );
    let has_get_field = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, TlcExpr::GetField(_, _)));
    assert!(
        has_get_field,
        "expected TlcExpr::GetField from OptionalAccess"
    );
}

#[test]
fn sub_mul_div_binops_lower_to_builtin() {
    let m = tlc_of("f x y = x - y\nf 5 3");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Sub, _, _))),
        "expected Builtin(Sub)"
    );
    let m = tlc_of("f x y = x * y\nf 2 3");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Mul, _, _))),
        "expected Builtin(Mul)"
    );
    let m = tlc_of("f x y = x / y\nf 6 2");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Div, _, _))),
        "expected Builtin(Div)"
    );
}

#[test]
fn comparison_binops_lower_to_builtin() {
    let m = tlc_of("f x y = x == y\nf 1 1");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Eq, _, _))),
        "expected Builtin(Eq)"
    );
    let m = tlc_of("f x y = x != y\nf 1 2");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Ne, _, _))),
        "expected Builtin(Ne)"
    );
    let m = tlc_of("f x y = x < y\nf 1 2");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Lt, _, _))),
        "expected Builtin(Lt)"
    );
    let m = tlc_of("f x y = x <= y\nf 1 2");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Le, _, _))),
        "expected Builtin(Le)"
    );
    let m = tlc_of("f x y = x > y\nf 2 1");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Gt, _, _))),
        "expected Builtin(Gt)"
    );
    let m = tlc_of("f x y = x >= y\nf 2 1");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Ge, _, _))),
        "expected Builtin(Ge)"
    );
}

#[test]
fn logical_and_or_coalesce_lower_to_builtin() {
    let m = tlc_of("f x y = x && y\nf true false");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::And, _, _))),
        "expected Builtin(And)"
    );
    let m = tlc_of("f x y = x || y\nf true false");
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Or, _, _))),
        "expected Builtin(Or)"
    );
    // Coalesce (??) on an Optional record field — placed in a declaration body so
    // TLC lowers it (the `final_expr` slot is not visited by the TLC lowerer).
    let m = tlc_of(
        "Server :: type { port? : Int; }\nget :: Server -> Int = \\s. s.port ?? 8080\nget {}",
    );
    assert!(
        m.expr_arena
            .iter()
            .any(|(_, e)| matches!(e, TlcExpr::Builtin(BuiltinOp::Coalesce, _, _))),
        "expected Builtin(Coalesce)"
    );
}

// ── Phase 10: THIR→TLC row elaboration ───────────────────────────────────────

/// Walk to the tail of a row, returning its row variable if the row is open.
fn row_tail_var(row: &Row) -> Option<TlcTypeVar> {
    match row {
        Row::RVar(v) => Some(*v),
        Row::RExtend { tail, .. } => row_tail_var(tail),
        Row::REmpty => None,
    }
}

#[test]
fn named_row_tail_emits_rvar_quantified_by_row_kind() {
    let m = tlc_of(
        "idHost :: <Rest> { host : Text; ...Rest; } -> { host : Text; ...Rest; }\n  = x => x;\nidHost",
    );
    let TlcDecl::Value { ty, .. } = &m.decl_arena[m.decls[0]] else {
        panic!("expected Value decl");
    };
    let TlcType::ForAll(_, kind, _) = &m.type_arena[*ty] else {
        panic!(
            "expected ForAll for a row-polymorphic function, got {:?}",
            m.type_arena[*ty]
        );
    };
    assert!(
        matches!(kind, Kind::Row(_)),
        "named row tail must quantify with Kind::Row, got {kind:?}"
    );
    let has_named_rvar = m.type_arena.iter().any(|(_, t)| {
        matches!(t, TlcType::Record(row) if matches!(row_tail_var(row), Some(TlcTypeVar::Named(_))))
    });
    assert!(
        has_named_rvar,
        "expected a record row ending in RVar(Named)"
    );
}

#[test]
fn anonymous_open_record_emits_rvar_tail() {
    let m = tlc_of(
        "getHost :: { host : Text; ...; } -> Text\n  = x => x.host;\ngetHost { host = \"h\"; }",
    );
    let has_rvar = m
        .type_arena
        .iter()
        .any(|(_, t)| matches!(t, TlcType::Record(row) if row_tail_var(row).is_some()));
    assert!(
        has_rvar,
        "anonymous open record must lower to a row with an RVar tail"
    );
}

#[test]
fn closed_records_have_no_row_variable() {
    let m = tlc_of("s :: { host : Text; port : Int; } = { host = \"h\"; port = 1; }\ns");
    for (_, t) in m.type_arena.iter() {
        if let TlcType::Record(row) = t {
            assert!(
                row_tail_var(row).is_none(),
                "closed record must not carry an RVar tail: {row:?}"
            );
        }
    }
}

#[test]
fn value_select_preserves_field_order_in_tlc() {
    let m = tlc_of("s ::= { host = \"h\"; port = 8080; name = \"n\"; }\nselect s { port; host; }");
    let has_ordered = m.expr_arena.iter().any(|(_, e)| {
        if let TlcExpr::Record(fields) = e {
            fields.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>() == ["port", "host"]
        } else {
            false
        }
    });
    assert!(
        has_ordered,
        "value select must lower to a record with fields in requested order [port, host]"
    );
}

fn row_field(row: &Row, name: &str) -> Option<(bool, TlcTypeId)> {
    match row {
        Row::RExtend {
            label,
            ty,
            optional,
            tail,
        } if label == name => Some((*optional, *ty)),
        Row::RExtend { tail, .. } => row_field(tail, name),
        Row::REmpty | Row::RVar(_) => None,
    }
}

#[test]
fn recursive_union_alias_lowers_recursive_fields_to_alias_tyvars() {
    let m = tlc_of(
        r#"
Tree :: type {
  #leaf;
  #node : { value : Int; left : Tree; right : Tree; };
}

example :: Tree =
  #node {
    value = 1;
    left  = #leaf;
    right = #node { value = 2; left = #leaf; right = #leaf; };
  }

example
"#,
    );

    let (tree_binding, body) = m
        .decls
        .iter()
        .find_map(|&id| match &m.decl_arena[id] {
            TlcDecl::TypeAlias {
                binding,
                params,
                body,
            } if params.is_empty() => Some((*binding, *body)),
            _ => None,
        })
        .expect("Tree type alias");

    let row = match &m.type_arena[body] {
        TlcType::VariantT(row) => row,
        other => panic!("expected Tree to lower to VariantT, got {other:?}"),
    };

    let (_, leaf_ty) = row_field(row, "leaf").expect("leaf variant");
    assert!(matches!(
        &m.type_arena[leaf_ty],
        TlcType::Singleton(Literal::Atom(name)) if name == "leaf"
    ));

    let (_, node_ty) = row_field(row, "node").expect("node variant");
    let node_row = match &m.type_arena[node_ty] {
        TlcType::Record(node_row) => node_row,
        other => panic!("expected node payload to lower to Record, got {other:?}"),
    };

    let (_, value_ty) = row_field(node_row, "value").expect("value field");
    assert!(matches!(
        &m.type_arena[value_ty],
        TlcType::Prim(PrimTy::Int)
    ));

    let (_, left_ty) = row_field(node_row, "left").expect("left field");
    assert!(matches!(
        &m.type_arena[left_ty],
        TlcType::TyVar(TlcTypeVar::Named(id), kind) if *id == tree_binding.0 && *kind == Kind::ground()
    ));

    let (_, right_ty) = row_field(node_row, "right").expect("right field");
    assert!(matches!(
        &m.type_arena[right_ty],
        TlcType::TyVar(TlcTypeVar::Named(id), kind) if *id == tree_binding.0 && *kind == Kind::ground()
    ));
}

#[test]
fn unsupported_effect_alias_application_keeps_row_for_dataflow_gate() {
    let m = tlc_of(
        r#"
Failing :: <A> type A ! { fail A }
f :: Text -> Failing Int
  = _ => perform fail 1;
f
"#,
    );
    let f_ty = m
        .decls
        .iter()
        .find_map(|&decl_id| match &m.decl_arena[decl_id] {
            TlcDecl::Value { ty, .. } => Some(*ty),
            _ => None,
        })
        .expect("value decl");
    let TlcType::Fun(_, _, eff) = &m.type_arena[f_ty] else {
        panic!("expected function type");
    };
    assert!(
        !matches!(eff, Row::REmpty),
        "unsupported residual effects must keep their row until the gate rejects them"
    );
    assert!(
        crate::residual_effect_reason(&m).is_some(),
        "unhandled residual perform still gates Dataflow lowering"
    );
}

#[test]
fn single_op_handle_resume_elaborates_to_effect_free_tlc() {
    let m = tlc_of(
        r#"
result ::= handle { perform warn "diag"; "ok" } with { warn = \d. resume (); }
result
"#,
    );
    assert!(
        crate::residual_effect_reason(&m).is_none(),
        "handled single-op program must leave no reachable effect markers or effect rows"
    );
}

#[test]
fn multi_op_handle_elaborates_to_effect_free_tlc() {
    let m = tlc_of(
        r#"
result ::= handle { perform warn "diag"; perform note "seen"; "ok" } with {
  warn = \d. resume ();
  note = \d. resume ();
}
result
"#,
    );
    assert!(
        crate::residual_effect_reason(&m).is_none(),
        "handled multi-op program must leave no reachable effect markers or effect rows"
    );
}

#[test]
fn nested_handlers_forward_to_outer_scope() {
    let m = tlc_of(
        r#"
result ::= handle {
  handle { perform inner "x"; perform outer "y"; "ok" } with {
    inner = \d. resume ();
  }
} with {
  outer = \d. resume ();
}
result
"#,
    );
    assert!(
        crate::residual_effect_reason(&m).is_none(),
        "nested handlers must forward unmatched operations to the enclosing handler"
    );
}

#[test]
fn deeply_nested_handlers_keep_all_enclosing_scopes() {
    let m = tlc_of(
        r#"
result ::= handle {
  handle {
    handle { perform outer "y"; "ok" } with {}
  } with {}
} with {
  outer = \d. resume ();
}
result
"#,
    );
    assert!(
        crate::residual_effect_reason(&m).is_none(),
        "deeply nested handlers must retain all enclosing forwarding scopes"
    );
}

#[test]
fn handler_clause_may_return_without_resume() {
    let m = tlc_of(
        r#"
result ::= handle { perform fail "bad"; "unreachable" } with {
  fail = \e. "fallback";
}
result
"#,
    );
    assert!(
        crate::residual_effect_reason(&m).is_none(),
        "handler clauses that return directly must erase the handled operation"
    );
}

#[test]
fn handler_clause_perform_forwards_to_outer_scope() {
    let m = tlc_of(
        r#"
result ::= handle {
  handle { perform fail "bad"; "unreachable" } with {
    fail = \e. { perform log e; "fallback" };
  }
} with {
  log = \d. resume ();
}
result
"#,
    );
    assert!(
        crate::residual_effect_reason(&m).is_none(),
        "handler-clause performs must be forwarded to the enclosing handler"
    );
}

#[test]
fn deep_patch_lowers_nested_record_fields_as_optional() {
    let m = tlc_of(
        r#"
Config :: type { server : { host : Text; port : Int; }; enabled : Bool; }
patch :: DeepPatch Config = { server = { port = 8080; }; }
patch
"#,
    );
    let patch_ty = m
        .decls
        .iter()
        .find_map(|&decl_id| match &m.decl_arena[decl_id] {
            TlcDecl::Value { ty, .. } => Some(*ty),
            _ => None,
        })
        .expect("patch decl");
    let TlcType::Record(row) = &m.type_arena[patch_ty] else {
        panic!("expected patch record type");
    };
    let (server_optional, server_ty) = row_field(row, "server").expect("server field");
    let (enabled_optional, _) = row_field(row, "enabled").expect("enabled field");
    assert!(server_optional);
    assert!(enabled_optional);
    let TlcType::Record(server_row) = &m.type_arena[server_ty] else {
        panic!("expected nested server record");
    };
    assert!(row_field(server_row, "host").expect("host field").0);
    assert!(row_field(server_row, "port").expect("port field").0);
}
