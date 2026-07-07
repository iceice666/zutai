use crate::*;
use la_arena::RawIdx;

/// Build a DataflowGraph from a source string. Panics on any diagnostic.
fn dc_of(src: &str) -> DataflowGraph {
    let parsed = zutai_syntax::parse(src);
    assert!(
        !parsed.has_errors(),
        "parse errors: {:?}",
        parsed.diagnostics()
    );
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "HIR errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "THIR errors: {:?}",
        thir.diagnostics
    );
    let tlc = zutai_tlc::lower_thir(thir.file.as_ref().expect("THIR file should be complete"));
    lower_tlc(&tlc, &hir.file.bindings)
}

#[test]
fn residual_non_host_effects_do_not_enter_dataflow_core() {
    let parsed = zutai_syntax::parse(
        r#"
parse :: Text -> Text ! { fail Text; }
  = text => perform fail text;
parse
"#,
    );
    assert!(!parsed.has_errors());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "HIR errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "THIR errors: {:?}",
        thir.diagnostics
    );
    let tlc = zutai_tlc::lower_thir(thir.file.as_ref().expect("THIR file should be complete"));
    let reason = try_lower_tlc(&tlc, &hir.file.bindings).expect_err("unhandled TLC must be gated");
    assert!(reason.contains("effect"), "{reason}");
}

#[test]
fn granted_standard_host_effect_lowers_to_host_op() {
    let parsed = zutai_syntax::parse(
        r#"
readFile :: Path -> Text ! { fs.read : Path -> Text; }
  = path => perform fs.read path;
readFile "Cargo.toml"
"#,
    );
    assert!(
        !parsed.has_errors(),
        "parse errors: {:?}",
        parsed.diagnostics()
    );
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "HIR errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "THIR errors: {:?}",
        thir.diagnostics
    );
    let tlc = zutai_tlc::lower_thir(thir.file.as_ref().expect("THIR file should be complete"));
    try_lower_tlc(&tlc, &hir.file.bindings).expect_err("fs.read needs an explicit host grant");
    let grants = zutai_tlc::HostEffectSet::AMBIENT.with(zutai_tlc::HostOp::FsRead);
    let graph = try_lower_tlc_with_host_grants(&tlc, &hir.file.bindings, grants)
        .expect("granted fs.read should lower");
    assert!(
        graph.nodes.iter().any(|(_, node)| {
            matches!(
                node.kind,
                DfNodeKind::HostOp {
                    op: zutai_tlc::HostOp::FsRead,
                    ..
                }
            )
        }),
        "granted fs.read should lower to a HostOp: {graph:#?}"
    );
    validate(&graph).expect("granted host op graph should validate");
}

#[test]
fn granted_scoped_fs_host_effects_lower_to_host_ops() {
    let parsed = zutai_syntax::parse(
        r#"
WriteTextRequest :: type { contents : Text; writer : Writer; };
writeTextRequest :: Writer -> Text -> WriteTextRequest
  = writer contents => { contents = contents; writer = writer; };
roundTrip :: Path -> Text? ! { fs.openWrite : Path -> Writer; fs.writeText : WriteTextRequest -> Unit; fs.flush : Writer -> Unit; fs.closeWrite : Writer -> Unit; fs.openRead : Path -> Reader; fs.readLine : Reader -> Text?; fs.closeRead : Reader -> Unit; }
  = path => [
    writer := perform fs.openWrite path;
    wrote := perform fs.writeText (writeTextRequest writer "alpha\n");
    flushed := perform fs.flush writer;
    closedWriter := perform fs.closeWrite writer;
    reader := perform fs.openRead path;
    line := perform fs.readLine reader;
    closedReader := perform fs.closeRead reader;
    line
  ];
roundTrip "target/zutai-dataflow-scoped-fs.txt"
"#,
    );
    assert!(
        !parsed.has_errors(),
        "parse errors: {:?}",
        parsed.diagnostics()
    );
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "HIR errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "THIR errors: {:?}",
        thir.diagnostics
    );
    let tlc = zutai_tlc::lower_thir(thir.file.as_ref().expect("THIR file should be complete"));
    try_lower_tlc(&tlc, &hir.file.bindings).expect_err("scoped fs ops need explicit host grants");
    let grants = zutai_tlc::HostEffectSet::AMBIENT
        .with(zutai_tlc::HostOp::FsOpenWrite)
        .with(zutai_tlc::HostOp::FsWriteText)
        .with(zutai_tlc::HostOp::FsFlush)
        .with(zutai_tlc::HostOp::FsCloseWrite)
        .with(zutai_tlc::HostOp::FsOpenRead)
        .with(zutai_tlc::HostOp::FsReadLine)
        .with(zutai_tlc::HostOp::FsCloseRead);
    let graph = try_lower_tlc_with_host_grants(&tlc, &hir.file.bindings, grants)
        .expect("granted scoped fs ops should lower");
    for op in [
        zutai_tlc::HostOp::FsOpenWrite,
        zutai_tlc::HostOp::FsWriteText,
        zutai_tlc::HostOp::FsFlush,
        zutai_tlc::HostOp::FsCloseWrite,
        zutai_tlc::HostOp::FsOpenRead,
        zutai_tlc::HostOp::FsReadLine,
        zutai_tlc::HostOp::FsCloseRead,
    ] {
        assert!(
            graph.nodes.iter().any(|(_, node)| {
                matches!(
                    node.kind,
                    DfNodeKind::HostOp {
                        op: found,
                        ..
                    } if found == op
                )
            }),
            "expected {op:?} HostOp in graph: {graph:#?}"
        );
    }
    validate(&graph).expect("granted scoped fs graph should validate");
}

#[test]
fn ambient_io_print_lowers_to_runtime_host_print() {
    let g = dc_of(r#"print "x""#);
    assert!(
        g.nodes
            .iter()
            .any(|(_, node)| matches!(node.kind, DfNodeKind::HostPrint { .. })),
        "ambient io.print should lower to a runtime HostPrint node: {g:#?}"
    );
}

#[test]
fn handled_single_op_effect_lowers_to_dataflow_core() {
    let parsed = zutai_syntax::parse(
        r#"
result ::= handle [ perform warn "diag"; "ok" ] with { warn = \d. resume (); };
result
"#,
    );
    assert!(!parsed.has_errors());
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "HIR errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "THIR errors: {:?}",
        thir.diagnostics
    );
    let tlc = zutai_tlc::lower_thir(thir.file.as_ref().expect("THIR file should be complete"));
    try_lower_tlc(&tlc, &hir.file.bindings).expect("handled effect must lower through Dataflow");
}

#[test]
fn repeated_handled_performs_do_not_emit_error_nodes() {
    let graph = dc_of(
        r#"
result ::= handle (perform query 1) + (perform query 2) with { query = \n. resume n; };
result
"#,
    );
    assert!(
        graph
            .nodes
            .iter()
            .all(|(_, node)| !matches!(node.kind, DfNodeKind::Error)),
        "fresh handler instantiations must not lower captured params to Error nodes"
    );
}

#[test]
fn handler_param_inside_tuple_does_not_emit_error_nodes() {
    let graph = dc_of(
        r#"
result ::= handle perform query 1 with { query = \n. resume (n, n); };
result
"#,
    );
    assert!(
        graph
            .nodes
            .iter()
            .all(|(_, node)| !matches!(node.kind, DfNodeKind::Error)),
        "handler alpha-renaming must update parameter refs inside aggregate nodes"
    );
}

// ── Span invariant ────────────────────────────────────────────────────────────

#[test]
fn span_table_same_length_as_nodes() {
    let g = dc_of("42");
    assert_eq!(
        g.spans.len(),
        g.nodes.len(),
        "spans.len() must equal nodes.len()"
    );
}

// ── Literal leaves ────────────────────────────────────────────────────────────

#[test]
fn int_literal_produces_lit_node() {
    let g = dc_of("42");
    let has_lit = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Lit(DfLit::Int(42))));
    assert!(has_lit, "expected Lit(Int(42)) node");
}

#[test]
fn bool_literal_produces_lit_node() {
    let g = dc_of("true");
    let has_lit = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Lit(DfLit::Bool(true))));
    assert!(has_lit, "expected Lit(Bool(true)) node");
}

#[test]
fn string_literal_produces_text_lit_node() {
    let g = dc_of("\"hello\"");
    let has_lit = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Lit(DfLit::Text(_))));
    assert!(has_lit, "expected Lit(Text(...)) node");
}

#[test]
fn posit_literal_produces_posit_lit_and_type() {
    let g = dc_of("1p64e5");
    let has_lit = g.nodes.iter().any(|(_, n)| {
        matches!(
            &n.kind,
            DfNodeKind::Lit(DfLit::Posit(lit))
                if lit.spec.nbits == 64 && lit.spec.es == 5
        )
    });
    assert!(has_lit, "expected Lit(Posit64e5) node");
    assert!(
        matches!(g.types[g.nodes[g.root].ty], DfTy::Posit(spec) if spec.nbits == 64 && spec.es == 5),
        "expected root type DfTy::Posit(Posit64e5)"
    );
}

#[test]
fn decimal_posit_add_lowers_to_posit_builtin() {
    let g = dc_of("1.5p32e3 + 2.25p32e3");
    let has_posit_add = g.nodes.iter().any(|(_, n)| {
        matches!(
            &n.kind,
            DfNodeKind::Builtin(
                DfBuiltinOp::Posit {
                    op: DfPositOp::Add,
                    spec
                },
                _,
                _
            ) if spec.nbits == 32 && spec.es == 3
        )
    });
    assert!(
        has_posit_add,
        "expected decimal posit addition to lower to a Posit32e3 add builtin"
    );
    assert!(
        matches!(g.types[g.nodes[g.root].ty], DfTy::Posit(spec) if spec.nbits == 32 && spec.es == 3),
        "expected root type DfTy::Posit(Posit32e3)"
    );
}

// ── Global declarations ───────────────────────────────────────────────────────

#[test]
fn global_value_appears_in_globals_map() {
    let g = dc_of("x ::= 42;\nx");
    assert!(g.globals.contains_key("x"), "expected 'x' in globals map");
}

#[test]
fn function_decl_appears_in_globals_map() {
    let g = dc_of("add a b = a + b;\nadd 1 2");
    assert!(
        g.globals.contains_key("add"),
        "expected 'add' in globals map"
    );
}

// ── Sharing (tree-to-graph) ───────────────────────────────────────────────────

#[test]
fn shared_binding_produces_single_node() {
    // `x` is used twice in `f`; both uses should resolve to the same NodeId.
    let g = dc_of("x ::= 42;\nf a = a + x;\nx");
    // Verify x is in globals exactly once.
    assert_eq!(
        g.globals.values().filter(|&&n| n == g.globals["x"]).count(),
        1,
        "globals should have x exactly once"
    );
    // Verify there is at least one Lit(Int(42)) node (x's value).
    let lit_count = g
        .nodes
        .iter()
        .filter(|(_, n)| matches!(&n.kind, DfNodeKind::Lit(DfLit::Int(42))))
        .count();
    assert!(lit_count >= 1, "expected at least one Lit(Int(42)) node");
}

// ── Lambda and Apply ──────────────────────────────────────────────────────────

#[test]
fn lambda_and_bind_nodes_present_for_function() {
    let g = dc_of("id x = x;\nid 42");
    let has_lambda = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Lambda { .. }));
    assert!(has_lambda, "expected Lambda node for 'id'");

    let has_bind = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Bind));
    assert!(has_bind, "expected Bind node for parameter 'x'");
}

#[test]
fn apply_node_present_for_function_call() {
    let g = dc_of("id x = x;\nid 42");
    let has_apply = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Apply { .. }));
    assert!(has_apply, "expected Apply node for 'id 42'");
}

// ── Type polymorphism ─────────────────────────────────────────────────────────

#[test]
fn polymorphic_function_has_tylam_node() {
    let g = dc_of("id x = x;\nid 42");
    let has_tylam = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::TyLam { .. }));
    assert!(has_tylam, "expected TyLam node for polymorphic 'id'");
}

#[test]
fn call_to_polymorphic_function_has_tyapp_node() {
    let g = dc_of("id x = x;\nresult :: Int = id 42;\nresult");
    let has_tyapp = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::TyApp { .. }));
    assert!(has_tyapp, "expected TyApp node at call site of 'id'");
}

// ── Binary operators ──────────────────────────────────────────────────────────

#[test]
fn binary_add_produces_builtin_node() {
    let g = dc_of("f a b = a + b;\nf 1 2");
    let has_add = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Builtin(DfBuiltinOp::Add, _, _)));
    assert!(has_add, "expected Builtin(Add) node for '+'");
}

#[test]
fn operator_witness_equality_lowers_to_select_apply_not_builtin() {
    let g = dc_of(
        r#"
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \a b. false; }
result :: Bool = 1 == 1;
result
"#,
    );

    let has_select = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Select { field, .. } if field == "=="));
    assert!(has_select, "expected Select(\"==\") for witnessed equality");

    let has_apply = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Apply { .. }));
    assert!(has_apply, "expected Apply nodes for witnessed equality");

    let builtin_eq_count = g
        .nodes
        .iter()
        .filter(|(_, n)| matches!(&n.kind, DfNodeKind::Builtin(DfBuiltinOp::Eq, _, _)))
        .count();
    assert_eq!(
        builtin_eq_count, 0,
        "witnessed equality must not lower to Builtin(Eq)"
    );
}

// ── Record ────────────────────────────────────────────────────────────────────

#[test]
fn record_literal_produces_record_node() {
    let g = dc_of("{ x = 1; y = 2; }");
    let has_record = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Record(_)));
    assert!(has_record, "expected Record node");
}

#[test]
fn record_update_produces_record_update_node() {
    let g = dc_of("r ::= { x = 1; y = 2; };\nr with { x = 3; }");
    let has_update = g.nodes.iter().any(|(_, n)| {
        matches!(
            &n.kind,
            DfNodeKind::RecordUpdate { updates, .. }
                if updates.iter().any(|(name, slot, _)| name == "x" && *slot == 0)
        )
    });
    assert!(has_update, "expected RecordUpdate node");
}

#[test]
fn record_field_access_produces_select_node() {
    let g = dc_of("r ::= { x = 1; };\nr.x");
    let has_select = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Select { .. }));
    assert!(has_select, "expected Select node for 'r.x'");
}

#[test]
fn optional_field_access_lowers_to_maybe_type() {
    let g = dc_of("S :: type { p? : Int; };\ns :: S = {};\ns.p");
    assert!(
        matches!(g.types[g.nodes[g.root].ty], DfTy::Maybe(_)),
        "expected optional field access to lower to DfTy::Maybe"
    );
}

#[test]
fn optional_record_literal_stores_maybe_slots() {
    let g = dc_of("S :: type { p? : Int; q? : Int; };\ns :: S = { p = 9; };\ns");
    let record_fields = g
        .nodes
        .iter()
        .find_map(|(_, node)| match &node.kind {
            DfNodeKind::Record(fields)
                if fields.iter().any(|(name, _)| name == "p")
                    && fields.iter().any(|(name, _)| name == "q") =>
            {
                Some(fields)
            }
            _ => None,
        })
        .expect("record literal");
    assert_eq!(record_fields.len(), 2);

    let p = record_fields
        .iter()
        .find(|(name, _)| name == "p")
        .map(|(_, node)| *node)
        .expect("p slot");
    let q = record_fields
        .iter()
        .find(|(name, _)| name == "q")
        .map(|(_, node)| *node)
        .expect("q slot");
    assert!(matches!(g.types[g.nodes[p].ty], DfTy::Maybe(_)));
    assert!(matches!(
        &g.nodes[p].kind,
        DfNodeKind::Variant { tag, .. } if tag == "present"
    ));
    assert!(matches!(g.types[g.nodes[q].ty], DfTy::Maybe(_)));
    assert!(matches!(
        &g.nodes[q].kind,
        DfNodeKind::Lit(DfLit::Atom(name)) if name == "absent"
    ));
}

#[test]
fn optional_access_lowers_to_match_node() {
    let g = dc_of(
        "Config :: type { port : Int; };
cfg :: Config? = #none;
cfg?.port",
    );
    let has_match = g
        .nodes
        .iter()
        .any(|(_, node)| matches!(&node.kind, DfNodeKind::Match { .. }));
    assert!(has_match, "optional access should lower to Match");
}

#[test]
fn optional_value_lowers_to_optional_type() {
    let g = dc_of("x :: Int? = #none;\nx");
    assert!(
        matches!(g.types[g.nodes[g.root].ty], DfTy::Optional(_)),
        "expected Int? to lower to DfTy::Optional"
    );
}

#[test]
fn recursive_union_alias_value_lowers_and_validates() {
    let g = dc_of(
        r#"
Tree :: type {
  #leaf;
  #node : { value : Int; left : Tree; right : Tree; };
};
example :: Tree =
  #node {
    value = 1;
    left = #leaf;
    right = #node { value = 2; left = #leaf; right = #leaf; };
  };
example == example
"#,
    );
    validate(&g).expect("recursive union aliases should produce valid Dataflow Core");
}

#[test]
fn generic_recursive_union_alias_value_lowers_and_validates() {
    let g = dc_of(
        r#"
Tree :: <A> type {
  #leaf;
  #node : { value : A; left : Tree A; right : Tree A; };
};
example :: Tree Int =
  #node { value = 1; left = #leaf; right = #leaf; };
example == example
"#,
    );
    validate(&g).expect("generic recursive union aliases should produce valid Dataflow Core");
}

#[test]
fn generic_recursive_union_alias_instantiates_entry_type() {
    let g = dc_of(
        r#"
Tree :: <A> type {
  #leaf;
  #node : { value : A; left : Tree A; right : Tree A; };
};
example :: Tree Int =
  #node { value = 1; left = #leaf; right = #leaf; };
example
"#,
    );
    let root_ty = g.nodes[g.root].ty;
    let DfTy::Union(variants) = &g.types[root_ty] else {
        panic!(
            "expected Tree Int root to instantiate to Union, got {:?}",
            g.types[root_ty]
        );
    };
    let node_record = variants
        .iter()
        .find(|variant| variant.tag == "node")
        .expect("node variant")
        .ty;
    let DfTy::Record(fields) = &g.types[node_record] else {
        panic!("expected node payload to instantiate to Record");
    };
    let recursive_fields: Vec<_> = fields
        .iter()
        .filter(|field| field.name == "left" || field.name == "right")
        .collect();
    assert_eq!(
        recursive_fields.len(),
        2,
        "expected exactly two recursive fields (left, right) in node payload"
    );
    assert!(
        recursive_fields
            .iter()
            .all(|field| matches!(g.types[field.ty], DfTy::Union(_))),
        "recursive fields should instantiate to Tree Int union types"
    );
}

#[test]
fn generic_alias_instantiation_preserves_nested_recursive_alias() {
    let g = dc_of(
        r#"
List_ :: type {
  #nil;
  #cons : { head : Int; tail : List_; };
};
Wrap :: <A> type { item : A; rest : List_; };
x :: Wrap Int = { item = 1; rest = #nil; };
x
"#,
    );
    validate(&g).expect("generic alias containing recursive alias should validate");
    // Assert the cons.tail back-edge: Wrap Int root has a `rest` field of type
    // List_ (Union); the `cons` variant's `tail` field must also be Union.
    let root_ty = g.nodes[g.root].ty;
    let DfTy::Record(root_fields) = &g.types[root_ty] else {
        panic!("expected Wrap Int root to be DfTy::Record");
    };
    let rest_ty = root_fields
        .iter()
        .find(|f| f.name == "rest")
        .expect("rest field")
        .ty;
    let DfTy::Union(list_variants) = &g.types[rest_ty] else {
        panic!("expected rest to be DfTy::Union (List_)");
    };
    let cons_payload_ty = list_variants
        .iter()
        .find(|v| v.tag == "cons")
        .expect("cons variant")
        .ty;
    let DfTy::Record(cons_fields) = &g.types[cons_payload_ty] else {
        panic!("expected cons payload to be DfTy::Record");
    };
    let tail_ty = cons_fields
        .iter()
        .find(|f| f.name == "tail")
        .expect("tail field")
        .ty;
    assert!(
        matches!(g.types[tail_ty], DfTy::Union(_)),
        "cons.tail should be a DfTy::Union (equirecursive List_ back-edge)"
    );
}

#[test]
fn mutual_recursive_aliases_lower_without_hang() {
    // Expr and Args are mutually recursive (non-generic); neither has params.
    // Nullary `#lit` construction mirrors the proven `#leaf` pattern.
    let g = dc_of(
        r#"
Expr :: type { #lit; #call : { args : Args; }; };
Args :: type { #none; #cons : { head : Expr; tail : Args; }; };
example :: Expr = #lit;
example
"#,
    );
    validate(&g).expect("mutually-referencing aliases should validate");
    let root_ty = g.nodes[g.root].ty;
    assert!(
        matches!(g.types[root_ty], DfTy::Union(_)),
        "Expr should lower to DfTy::Union"
    );
}

#[test]
fn instantiate_df_type_covers_container_arms() {
    let g = dc_of(
        r#"
Holder :: <A> type { items : List A; opt : A?; pair : (A, Int); };
h :: Holder Int = { items = {1; 2;}; opt = #none; pair = (1, 2); };
h
"#,
    );
    validate(&g).expect("generic alias with container fields should validate");
    let DfTy::Record(fields) = &g.types[g.nodes[g.root].ty] else {
        panic!(
            "expected Holder Int root to be DfTy::Record, got {:?}",
            g.types[g.nodes[g.root].ty]
        );
    };
    let field_ty = |name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("missing field {name}"))
            .ty
    };
    assert!(
        matches!(g.types[field_ty("items")], DfTy::List(_)),
        "items should instantiate to DfTy::List"
    );
    assert!(
        matches!(g.types[field_ty("opt")], DfTy::Optional(_)),
        "opt should instantiate to DfTy::Optional"
    );
    assert!(
        matches!(g.types[field_ty("pair")], DfTy::Tuple(_)),
        "pair should instantiate to DfTy::Tuple"
    );
}

// ── Tuple ─────────────────────────────────────────────────────────────────────

#[test]
fn tuple_literal_produces_tuple_node() {
    let g = dc_of("(1, 2, 3)");
    let has_tuple = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Tuple(_)));
    assert!(has_tuple, "expected Tuple node");
}

// ── List ──────────────────────────────────────────────────────────────────────

#[test]
fn list_literal_produces_list_node() {
    let g = dc_of("{1; 2; 3;}");
    let has_list = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::List(_)));
    assert!(has_list, "expected List node");
}

// ── Match / Case ──────────────────────────────────────────────────────────────

#[test]
fn match_expression_produces_match_node() {
    let g = dc_of("f x = match x { | 1 => true; | _ => false; };\nf 1");
    let has_match = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Match { .. }));
    assert!(has_match, "expected Match node");
}

#[test]
fn if_expression_produces_match_node() {
    let g = dc_of("f x = if x then 1 else 2;\nf true");
    let has_match = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Match { .. }));
    assert!(has_match, "expected Match node from if desugaring");
}

#[test]
fn logical_short_circuit_produces_match_node() {
    let g = dc_of("f x y = x && y;\nf false true");
    let has_match = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Match { .. }));
    let has_eager_and = g.nodes.iter().any(|(_, n)| {
        matches!(
            &n.kind,
            DfNodeKind::Builtin(DfBuiltinOp::And | DfBuiltinOp::Or, _, _)
        )
    });
    assert!(has_match, "expected Match node from logical short-circuit");
    assert!(!has_eager_and, "logical short-circuit must not be eager");
}

#[test]
fn coalesce_produces_match_node() {
    let g = dc_of("x :: Int? = #some (9);\nx ?? 0");
    let has_match = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Match { .. }));
    let has_eager_coalesce = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Coalesce { .. }));
    assert!(has_match, "expected Match node from coalesce");
    assert!(!has_eager_coalesce, "coalesce fallback must not be eager");
}

// ── Recursive back-edge ───────────────────────────────────────────────────────

#[test]
fn recursive_function_has_global_ref_back_edge() {
    let g = dc_of("factorial n = if n < 1 then 1 else n * factorial (n - 1);\nfactorial 5");
    // factorial should be in globals.
    assert!(
        g.globals.contains_key("factorial"),
        "expected 'factorial' in globals"
    );
    // The body of factorial must reference GlobalRef("factorial").
    let has_back_edge = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::GlobalRef(name) if name == "factorial"));
    assert!(
        has_back_edge,
        "expected GlobalRef('factorial') back-edge for recursive call"
    );
}

// ── GlobalRef validity ────────────────────────────────────────────────────────

fn invalid_node_id(g: &DataflowGraph) -> NodeId {
    NodeId::from_raw(RawIdx::from_u32(g.nodes.len() as u32))
}

fn invalid_ty_id(g: &DataflowGraph) -> DfTyId {
    DfTyId::from_raw(RawIdx::from_u32(g.types.len() as u32))
}

fn first_bind(g: &DataflowGraph) -> NodeId {
    g.nodes
        .iter()
        .find_map(|(id, node)| matches!(&node.kind, DfNodeKind::Bind).then_some(id))
        .expect("expected at least one Bind node")
}

fn first_builtin(g: &DataflowGraph) -> NodeId {
    g.nodes
        .iter()
        .find_map(|(id, node)| matches!(&node.kind, DfNodeKind::Builtin(..)).then_some(id))
        .expect("expected at least one Builtin node")
}

#[test]
fn validation_rejects_invalid_root_node_ref() {
    let mut g = dc_of("42");
    let target = invalid_node_id(&g);
    g.root = target;

    let errors = validate(&g).expect_err("invalid root must fail validation");
    assert!(errors.contains(&ValidationError::InvalidRootNode { target }));
}

#[test]
fn validation_rejects_invalid_node_type_ref() {
    let mut g = dc_of("42");
    let node = g.nodes.iter().next().expect("expected a node").0;
    let ty = invalid_ty_id(&g);
    g.nodes[node].ty = ty;

    let errors = validate(&g).expect_err("invalid node type must fail validation");
    assert!(errors.contains(&ValidationError::InvalidNodeType { node, ty }));
}

#[test]
fn validation_rejects_invalid_nested_type_ref() {
    let mut g = dc_of("42");
    let owner = g.types.alloc(DfTy::Error);
    let target = invalid_ty_id(&g);
    g.types[owner] = DfTy::List(target);
    g.nodes[g.root].ty = owner;

    let errors = validate(&g).expect_err("invalid nested type must fail validation");
    assert!(errors.contains(&ValidationError::InvalidTypeRef {
        owner,
        field: "element",
        target,
    }));
}

#[test]
fn validation_rejects_builtin_result_type_mismatch() {
    let mut g = dc_of("1 + 2");
    let node = first_builtin(&g);
    let ty = g.types.alloc(DfTy::Bool);
    g.nodes[node].ty = ty;

    let errors = validate(&g).expect_err("builtin result mismatch must fail validation");
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::TypeMismatch {
            owner,
            field: "type",
            actual,
            ..
        } if *owner == node && *actual == ty
    )));
}

#[test]
fn validation_rejects_lambda_bind_used_outside_owner() {
    let mut g = dc_of("id x = x;\nid 1");
    let bind = first_bind(&g);
    g.root = bind;

    let errors = validate(&g).expect_err("lambda bind outside owner must fail validation");
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::LambdaCaptureViolation {
            bind: error_bind,
            use_site,
            ..
        } if *error_bind == bind && *use_site == bind
    )));
}

#[test]
fn validation_rejects_arm_bind_used_outside_arm() {
    let mut g = dc_of("match 1 { | x => x; }");
    let bind = first_bind(&g);
    g.root = bind;

    let errors = validate(&g).expect_err("arm bind outside arm must fail validation");
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::ArmBindScopeViolation {
            bind: error_bind,
            use_site,
            ..
        } if *error_bind == bind && *use_site == bind
    )));
}

#[test]
fn validation_allows_nested_lambda_to_capture_outer_bind() {
    validate(&dc_of("make :: Int -> Int -> Int\n  = x y => x;\nmake 1 2"))
        .expect("nested lambda capture must validate");
}

#[test]
fn no_stray_global_refs() {
    let g = dc_of("f x = x + 1;\nf 10");
    validate(&g).expect("DataflowGraph should pass validation");
}

#[test]
fn validation_passes_for_polymorphic_program() {
    let g = dc_of("id x = x;\nid 42");
    validate(&g).expect("DataflowGraph should pass validation");
}

#[test]
fn validation_passes_for_recursive_program() {
    let g = dc_of("factorial n = if n < 1 then 1 else n * factorial (n - 1);\nfactorial 5");
    validate(&g).expect("DataflowGraph should pass validation");
}

// ── Root node ─────────────────────────────────────────────────────────────────

#[test]
fn root_node_is_valid_arena_index() {
    let g = dc_of("42");
    // Accessing root should not panic.
    let _ = &g.nodes[g.root];
}

// ── Block / let sharing ───────────────────────────────────────────────────────

#[test]
fn block_local_sharing_no_duplicate_value_nodes() {
    // `n` is used twice in the block; the value node (Lit(42)) should appear once.
    let g = dc_of("[ n := 42; n + n ]");
    let lit_count = g
        .nodes
        .iter()
        .filter(|(_, n)| matches!(&n.kind, DfNodeKind::Lit(DfLit::Int(42))))
        .count();
    // There should be exactly 1 Lit(Int(42)) node (shared via graph edges).
    assert_eq!(
        lit_count, 1,
        "expected exactly one Lit(42) node (shared); got {lit_count}"
    );
}

// ── Constraint witnesses (dict records) ──────────────────────────────────────

#[test]
fn witness_dict_record_present_in_globals() {
    let g = dc_of(
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. true; }
1
"#,
    );
    // The witness decl should appear as a Value in globals (dict record).
    let record_fields = g
        .globals
        .values()
        .find_map(|&node_id| match &g.nodes[node_id].kind {
            DfNodeKind::Record(fields) => Some(fields),
            _ => None,
        })
        .expect("expected a Record node among globals for the witness dict");
    assert_eq!(
        record_fields.len(),
        1,
        "witness dict record must retain its method field"
    );
}

// ── validate_structural ───────────────────────────────────────────────────────

#[test]
fn validate_structural_rejects_invalid_root_node_ref() {
    let mut g = dc_of("42");
    let target = invalid_node_id(&g);
    g.root = target;

    let errors = validate_structural(&g).expect_err("invalid root must fail structural validation");
    assert!(errors.contains(&ValidationError::InvalidRootNode { target }));
}

#[test]
fn validate_structural_rejects_builtin_result_type_mismatch() {
    let mut g = dc_of("1 + 2");
    let node = first_builtin(&g);
    let ty = g.types.alloc(DfTy::Bool);
    g.nodes[node].ty = ty;

    let errors =
        validate_structural(&g).expect_err("type-shape mismatch must fail structural validation");
    assert!(errors.iter().any(|error| matches!(
        error,
        ValidationError::TypeMismatch {
            owner,
            field: "type",
            actual,
            ..
        } if *owner == node && *actual == ty
    )));
}

#[test]
fn validate_structural_rejects_invalid_nested_type_ref() {
    let mut g = dc_of("42");
    let owner = g.types.alloc(DfTy::Error);
    let target = invalid_ty_id(&g);
    g.types[owner] = DfTy::List(target);
    g.nodes[g.root].ty = owner;

    let errors = validate_structural(&g)
        .expect_err("invalid nested type ref must fail structural validation");
    assert!(errors.contains(&ValidationError::InvalidTypeRef {
        owner,
        field: "element",
        target,
    }));
}

#[test]
fn validate_structural_skips_scope_walk() {
    // A lambda-bind exposed as the root violates the capture-walk invariants (3, 4)
    // but is structurally valid: the Bind node exists, has a valid type, etc.
    // validate_structural must pass; validate must still fail.
    let mut g = dc_of("id x = x;\nid 1");
    let bind = first_bind(&g);
    g.root = bind;

    // Structural subset: no violation — Bind is a valid node with a valid type.
    validate_structural(&g).expect("structural check must pass for a bind-as-root graph");

    // Full validation catches the capture violation.
    let errors = validate(&g).expect_err("full validation must catch the scope violation");
    assert!(errors.iter().any(|e| matches!(
        e,
        ValidationError::LambdaCaptureViolation { bind: error_bind, .. }
            if *error_bind == bind
    )));
}

#[test]
fn validate_structural_accepts_well_formed_graph() {
    // A nested-capture program with a valid graph must not produce false positives.
    validate_structural(&dc_of("make :: Int -> Int -> Int\n  = x y => x;\nmake 1 2"))
        .expect("well-formed graph must pass structural validation");
}

#[test]
fn curried_polymorphic_lambdas_get_per_layer_function_types() {
    // `constFn` curries over two distinct type variables (`A -> B -> A`). TLC used
    // to type every curried lambda layer with the full signature, so the inner
    // lambda's declared param type (`A`) disagreed with its bind type (`B`), and the
    // structural validator rejected the graph (a hard panic during lowering). With
    // per-layer peeled function types the graph is well-formed. The monomorphic
    // `validate_structural_accepts_well_formed_graph` above never exercised this
    // because all parameters shared one type.
    validate_structural(&dc_of(
        "constFn :: <A, B> A -> B -> A = a b => a;\nconstFn 7 5",
    ))
    .expect("curried polymorphic lowering must validate");
}

// ── Open-row select gate ──────────────────────────────────────────────────────

fn tlc_of(src: &str) -> (zutai_tlc::TlcModule, Vec<zutai_hir::Binding>) {
    let parsed = zutai_syntax::parse(src);
    assert!(
        !parsed.has_errors(),
        "parse errors: {:?}",
        parsed.diagnostics()
    );
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "HIR errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "THIR errors: {:?}",
        thir.diagnostics
    );
    let tlc = zutai_tlc::lower_thir(thir.file.as_ref().expect("THIR file should be complete"));
    let bindings = hir.file.bindings.clone();
    (tlc, bindings)
}

#[test]
fn open_row_select_lowers_after_monomorphization() {
    // Phase C: a polymorphic open-row select is monomorphized at the concrete call
    // site (the field slot is recomputed for the concrete record), so it lowers to
    // Dataflow Core instead of being gated.
    let (mut tlc, bindings) = tlc_of(
        r#"
getN :: { n : Int; ...; } -> Int = x => x.n;
getN { extra = 7; n = 5; }
"#,
    );
    zutai_tlc::monomorphize_open_row_selects(&mut tlc);
    try_lower_tlc(&tlc, &bindings).expect("monomorphized open-row select must lower to DC");
}

#[test]
fn closed_record_select_lowers_without_error() {
    // Concrete/closed record field access must still work.
    let (tlc, bindings) = tlc_of(
        r#"
r :: { a : Int; b : Int; } = { a = 3; b = 9; };
r.b
"#,
    );
    try_lower_tlc(&tlc, &bindings).expect("closed-record select must lower to DC");
}

#[test]
fn open_row_passthrough_without_select_lowers_without_error() {
    // Passing an open record through unchanged (no GetField) is sound: the caller
    // provides a concrete record; the function never reads a field by slot.
    let (tlc, bindings) = tlc_of(
        r#"
idRec :: { n : Int; ...; } -> { n : Int; ...; } = x => x;
idRec { extra = 7; n = 5; }
"#,
    );
    try_lower_tlc(&tlc, &bindings)
        .expect("open-row passthrough without field access must lower to DC");
}

#[test]
fn open_row_select_lowers_under_host_grants_after_monomorphization() {
    // The host-grant lowering path runs the same gate; after monomorphization a
    // concrete open-row select lowers there too.
    let (mut tlc, bindings) = tlc_of(
        r#"
getN :: { n : Int; ...; } -> Int = x => x.n;
getN { extra = 7; n = 5; }
"#,
    );
    zutai_tlc::monomorphize_open_row_selects(&mut tlc);
    try_lower_tlc_with_host_grants(&tlc, &bindings, zutai_tlc::HostEffectSet::ALL)
        .expect("monomorphized open-row select must lower under host grants");
}

// ── Module-import backend gate ────────────────────────────────────────────────

#[test]
fn module_import_is_rejected_before_dataflow_core() {
    // TLC→DC lowers `TlcExpr::Import` to `DfNodeKind::Import`, which ANF treats
    // as an `AnfExpr::Error` leaf — imported modules are never linked, so a
    // compiled program that imports crashes at runtime. `try_lower_tlc` must
    // reject any module containing an import. Build a minimal module by hand
    // because the `tlc_of` helper cannot resolve a real import without a base
    // directory.
    use la_arena::Arena;
    use rustc_hash::FxHashMap;
    use zutai_hir::HirImportSource;
    use zutai_tlc::{Row, TlcExpr, TlcModule, TlcType};

    let mut type_arena = Arena::new();
    let unit_ty = type_arena.alloc(TlcType::Record(Row::REmpty));
    let mut expr_arena = Arena::new();
    let import_expr = expr_arena.alloc(TlcExpr::Import(HirImportSource::String(
        "data_module.zt".into(),
    )));
    let mut expr_types = FxHashMap::default();
    expr_types.insert(import_expr, unit_ty);

    let module = TlcModule {
        decls: Vec::new(),
        decl_arena: Arena::new(),
        expr_arena,
        type_arena,
        expr_types,
        dict_field_slots: FxHashMap::default(),
        dict_dispatch_keys: FxHashMap::default(),
        spans: FxHashMap::default(),
        final_expr: Some(import_expr),
        extern_global_bindings: rustc_hash::FxHashMap::default(),
    };

    let reason = try_lower_tlc(&module, &[]).expect_err("module import must be gated before DC");
    assert!(
        reason.contains("import"),
        "reason should mention imports, got: {reason}"
    );
    let reason = try_lower_tlc_with_host_grants(&module, &[], zutai_tlc::HostEffectSet::ALL)
        .expect_err("module import must be gated under host grants");
    assert!(reason.contains("import"), "{reason}");
}
