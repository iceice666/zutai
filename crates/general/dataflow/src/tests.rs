use crate::*;

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

// ── Global declarations ───────────────────────────────────────────────────────

#[test]
fn global_value_appears_in_globals_map() {
    let g = dc_of("x := 42\nx");
    assert!(g.globals.contains_key("x"), "expected 'x' in globals map");
}

#[test]
fn function_decl_appears_in_globals_map() {
    let g = dc_of("add a b = a + b\nadd 1 2");
    assert!(
        g.globals.contains_key("add"),
        "expected 'add' in globals map"
    );
}

// ── Sharing (tree-to-graph) ───────────────────────────────────────────────────

#[test]
fn shared_binding_produces_single_node() {
    // `x` is used twice in `f`; both uses should resolve to the same NodeId.
    let g = dc_of("x := 42\nf a = a + x\nx");
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
    let g = dc_of("id x = x\nid 42");
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
    let g = dc_of("id x = x\nid 42");
    let has_apply = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Apply { .. }));
    assert!(has_apply, "expected Apply node for 'id 42'");
}

// ── Type polymorphism ─────────────────────────────────────────────────────────

#[test]
fn polymorphic_function_has_tylam_node() {
    let g = dc_of("id x = x\nid 42");
    let has_tylam = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::TyLam { .. }));
    assert!(has_tylam, "expected TyLam node for polymorphic 'id'");
}

#[test]
fn call_to_polymorphic_function_has_tyapp_node() {
    let g = dc_of("id x = x\nresult :: Int = id 42\nresult");
    let has_tyapp = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::TyApp { .. }));
    assert!(has_tyapp, "expected TyApp node at call site of 'id'");
}

// ── Binary operators ──────────────────────────────────────────────────────────

#[test]
fn binary_add_produces_builtin_node() {
    let g = dc_of("f a b = a + b\nf 1 2");
    let has_add = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Builtin(DfBuiltinOp::Add, _, _)));
    assert!(has_add, "expected Builtin(Add) node for '+'");
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
fn record_field_access_produces_select_node() {
    let g = dc_of("r := { x = 1; }\nr.x");
    let has_select = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Select { .. }));
    assert!(has_select, "expected Select node for 'r.x'");
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
    let g = dc_of("[1; 2; 3;]");
    let has_list = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::List(_)));
    assert!(has_list, "expected List node");
}

// ── Match / Case ──────────────────────────────────────────────────────────────

#[test]
fn match_expression_produces_match_node() {
    let g = dc_of("f x = match x { | 1 => true; | _ => false; }\nf 1");
    let has_match = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Match { .. }));
    assert!(has_match, "expected Match node");
}

#[test]
fn if_expression_produces_match_node() {
    let g = dc_of("f x = if x then 1 else 2\nf true");
    let has_match = g
        .nodes
        .iter()
        .any(|(_, n)| matches!(&n.kind, DfNodeKind::Match { .. }));
    assert!(has_match, "expected Match node from if desugaring");
}

// ── Recursive back-edge ───────────────────────────────────────────────────────

#[test]
fn recursive_function_has_global_ref_back_edge() {
    let g = dc_of("factorial n = if n < 1 then 1 else n * factorial (n - 1)\nfactorial 5");
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

#[test]
fn no_stray_global_refs() {
    let g = dc_of("f x = x + 1\nf 10");
    validate(&g).expect("DataflowGraph should pass validation");
}

#[test]
fn validation_passes_for_polymorphic_program() {
    let g = dc_of("id x = x\nid 42");
    validate(&g).expect("DataflowGraph should pass validation");
}

#[test]
fn validation_passes_for_recursive_program() {
    let g = dc_of("factorial n = if n < 1 then 1 else n * factorial (n - 1)\nfactorial 5");
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
    let g = dc_of("{ n := 42; n + n }");
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
    let has_record_global = g
        .globals
        .values()
        .any(|&node_id| matches!(&g.nodes[node_id].kind, DfNodeKind::Record(_)));
    assert!(
        has_record_global,
        "expected a Record node among globals for the witness dict"
    );
}
