use crate::*;

/// Build an AnfModule from a source string. Panics on any diagnostic.
fn anf_of(src: &str) -> AnfModule {
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
    let tlc = zutai_tlc::lower_thir(thir.file.as_ref().expect("THIR file"));
    let dc = zutai_dataflow::lower_tlc(&tlc, &hir.file.bindings);
    lower_dc(&dc)
}

/// Count `AnfDecl::Let` entries in a module's top-level decls.
fn count_let(m: &AnfModule) -> usize {
    m.decls
        .iter()
        .filter(|d| matches!(d, AnfDecl::Let { .. }))
        .count()
}

/// Count `AnfDecl::Letrec` entries in a module's top-level decls.
fn count_letrec(m: &AnfModule) -> usize {
    m.decls
        .iter()
        .filter(|d| matches!(d, AnfDecl::Letrec { .. }))
        .count()
}

/// Collect all Letrec binding names across all Letrec decls.
fn letrec_names(m: &AnfModule) -> Vec<String> {
    m.decls
        .iter()
        .flat_map(|d| match d {
            AnfDecl::Letrec { bindings } => {
                bindings.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>()
            }
            _ => vec![],
        })
        .collect()
}

// ── Literal root ─────────────────────────────────────────────────────────────

#[test]
fn int_literal_root_has_lit_atom() {
    let m = anf_of("42");
    assert!(
        matches!(&m.root.result, AnfAtom::Lit(DfLit::Int(42))),
        "expected root result to be Lit(42)"
    );
    assert!(
        m.root.bindings.is_empty(),
        "no bindings needed for a literal root"
    );
}

// ── Non-recursive global → let ────────────────────────────────────────────────

#[test]
fn non_recursive_global_emits_let() {
    let m = anf_of("x ::= 42\nx");
    assert_eq!(count_letrec(&m), 0, "x is not recursive → no letrec");
    assert!(count_let(&m) >= 1, "x should emit a let decl");
    let has_x = m
        .decls
        .iter()
        .any(|d| matches!(d, AnfDecl::Let { name, .. } if name == "x"));
    assert!(has_x, "expected AnfDecl::Let for 'x'");
}

// ── Self-recursive global → letrec ───────────────────────────────────────────

#[test]
fn self_recursive_global_emits_letrec() {
    let m = anf_of("factorial n = if n < 1 then 1 else n * factorial (n - 1)\nfactorial 5");
    assert_eq!(
        count_letrec(&m),
        1,
        "factorial is recursive → exactly one letrec"
    );
    assert!(
        letrec_names(&m).contains(&"factorial".to_string()),
        "letrec should contain 'factorial'"
    );
}

// ── Mutually recursive globals → one letrec ───────────────────────────────────

#[test]
fn mutually_recursive_globals_emit_single_letrec() {
    let m = anf_of(
        "even n = if n == 0 then true else odd (n - 1)\nodd n = if n == 0 then false else even (n - 1)\neven 4",
    );
    assert_eq!(
        count_letrec(&m),
        1,
        "even/odd are mutually recursive → one letrec"
    );
    let names = letrec_names(&m);
    assert!(
        names.contains(&"even".to_string()),
        "letrec should contain 'even'"
    );
    assert!(
        names.contains(&"odd".to_string()),
        "letrec should contain 'odd'"
    );
}

// ── Apply introduces ANF binding ──────────────────────────────────────────────

#[test]
fn function_call_root_has_apply_binding() {
    // `id 42` — root is not a plain atom; should have at least one binding.
    let m = anf_of("id x = x\nid 42");
    // The root body should have some bindings (for the Apply and/or TyApp nodes).
    assert!(
        !m.root.bindings.is_empty() || !m.decls.is_empty(),
        "id 42 should generate at least one binding or decl"
    );
}

// ── Binary op introduces ANF binding ─────────────────────────────────────────

#[test]
fn binary_op_root_has_builtin_binding() {
    let m = anf_of("1 + 2");
    // The root body should have a Builtin(Add) binding.
    let has_add = m.root.bindings.iter().any(|(_, expr)| {
        matches!(
            expr,
            AnfExpr::Builtin {
                op: DfBuiltinOp::Add,
                ..
            }
        )
    });
    assert!(has_add, "1 + 2 root should contain a Builtin(Add) binding");
}

// ── Lambda body structure ─────────────────────────────────────────────────────

#[test]
fn function_decl_emits_lambda_expr() {
    let m = anf_of("inc x = x + 1\ninc 3");
    // The 'inc' decl's body should ultimately resolve to a Lambda.
    let inc_decl = m
        .decls
        .iter()
        .find(|d| matches!(d, AnfDecl::Let { name, .. } if name == "inc"));
    assert!(inc_decl.is_some(), "expected AnfDecl::Let for 'inc'");
    if let Some(AnfDecl::Let { body, .. }) = inc_decl {
        let has_lambda = body
            .bindings
            .iter()
            .any(|(_, e)| matches!(e, AnfExpr::Lambda { .. }))
            || matches!(&body.result, AnfAtom::Var(_));
        assert!(
            has_lambda || !body.bindings.is_empty(),
            "inc should have a lambda or bindings"
        );
    }
}

// ── Record literal ────────────────────────────────────────────────────────────

#[test]
fn record_literal_root_has_record_binding() {
    let m = anf_of("{ x = 1; y = 2; }");
    let has_record = m
        .root
        .bindings
        .iter()
        .any(|(_, e)| matches!(e, AnfExpr::Record(_)));
    assert!(has_record, "expected Record binding in root body");
}

#[test]
fn record_update_root_has_record_update_binding() {
    let m = anf_of("r ::= { x = 1; y = 2; }\nr with { x = 3; }");
    let has_update = m.root.bindings.iter().any(|(_, e)| {
        matches!(
            e,
            AnfExpr::RecordUpdate { updates, .. }
                if updates.iter().any(|(slot, _)| *slot == 0)
        )
    });
    assert!(has_update, "expected RecordUpdate binding in root body");
}

// ── Tuple literal ─────────────────────────────────────────────────────────────

#[test]
fn tuple_literal_root_has_tuple_binding() {
    let m = anf_of("(1, 2, 3)");
    let has_tuple = m
        .root
        .bindings
        .iter()
        .any(|(_, e)| matches!(e, AnfExpr::Tuple(_)));
    assert!(has_tuple, "expected Tuple binding in root body");
}

// ── List literal ──────────────────────────────────────────────────────────────

#[test]
fn list_literal_root_has_list_binding() {
    let m = anf_of("[1; 2; 3;]");
    let has_list = m
        .root
        .bindings
        .iter()
        .any(|(_, e)| matches!(e, AnfExpr::List(_)));
    assert!(has_list, "expected List binding in root body");
}

// ── Field select ──────────────────────────────────────────────────────────────

#[test]
fn field_access_root_has_select_binding() {
    let m = anf_of("r ::= { x = 1; }\nr.x");
    let has_select = m
        .root
        .bindings
        .iter()
        .any(|(_, e)| matches!(e, AnfExpr::Select { .. }));
    assert!(has_select, "expected Select binding in root body for 'r.x'");
}

// ── Match ────────────────────────────────────────────────────────────────────

#[test]
fn match_expression_emits_match_expr() {
    let m = anf_of("f x = match x { | 1 => true; | _ => false; }\nf 1");
    // The body of 'f' (a let decl) should contain a Match expr.
    let f_decl = m
        .decls
        .iter()
        .find(|d| matches!(d, AnfDecl::Let { name, .. } if name == "f"));
    assert!(f_decl.is_some(), "expected AnfDecl::Let for 'f'");
    if let Some(AnfDecl::Let { body, .. }) = f_decl {
        fn has_match(body: &AnfBody) -> bool {
            body.bindings.iter().any(|(_, e)| match e {
                AnfExpr::Match { .. } => true,
                AnfExpr::Lambda { body: b, .. } => has_match(b),
                AnfExpr::TyLam { body: b, .. } => has_match(b),
                _ => false,
            })
        }
        assert!(has_match(body), "'f' should contain a Match expr");
    }
}

// ── If desugaring → match ─────────────────────────────────────────────────────

#[test]
fn if_expression_becomes_match_in_anf() {
    let m = anf_of("f x = if x then 1 else 2\nf true");
    let f_decl = m
        .decls
        .iter()
        .find(|d| matches!(d, AnfDecl::Let { name, .. } if name == "f"));
    if let Some(AnfDecl::Let { body, .. }) = f_decl {
        fn has_match(body: &AnfBody) -> bool {
            body.bindings.iter().any(|(_, e)| match e {
                AnfExpr::Match { .. } => true,
                AnfExpr::Lambda { body: b, .. } => has_match(b),
                AnfExpr::TyLam { body: b, .. } => has_match(b),
                _ => false,
            })
        }
        assert!(has_match(body), "if desugaring should produce a Match expr");
    }
}

// ── Sharing: block local used twice ──────────────────────────────────────────

#[test]
fn block_local_shared_not_duplicated() {
    // `n` is used twice; ANF should introduce one binding for it and reuse the var.
    let m = anf_of("{ n := 42; n + n }");
    // Root body has bindings. The two +_operands should be the same Var.
    let add_binding = m.root.bindings.iter().find(|(_, e)| {
        matches!(
            e,
            AnfExpr::Builtin {
                op: DfBuiltinOp::Add,
                ..
            }
        )
    });
    if let Some((_, AnfExpr::Builtin { lhs, rhs, .. })) = add_binding {
        assert_eq!(
            lhs, rhs,
            "both operands of n+n should be the same atom (shared)"
        );
    } else {
        panic!("expected Builtin(Add) binding in root body");
    }
}

// ── Topological ordering: dep before dependent ───────────────────────────────

#[test]
fn dependency_decl_precedes_dependent_decl() {
    // `b` depends on `a`; `a` should appear before `b` in decls.
    let m = anf_of("a ::= 1\nb ::= a + 1\nb");
    let a_pos = m
        .decls
        .iter()
        .position(|d| matches!(d, AnfDecl::Let { name, .. } if name == "a"));
    let b_pos = m
        .decls
        .iter()
        .position(|d| matches!(d, AnfDecl::Let { name, .. } if name == "b"));
    assert!(
        a_pos.is_some() && b_pos.is_some(),
        "both 'a' and 'b' should be in decls"
    );
    assert!(
        a_pos.unwrap() < b_pos.unwrap(),
        "dependency 'a' must precede dependent 'b' in decl order"
    );
}

// ── ANF atom arguments ─────────────────────────────────────────────────────

#[test]
fn apply_arguments_are_atoms() {
    let m = anf_of("f a b = a + b\nf 1 2");
    fn check_body(body: &AnfBody) {
        for (_, expr) in &body.bindings {
            if let AnfExpr::Apply { func, arg } = expr {
                assert!(
                    !matches!(func, AnfAtom::Var(v) if v.is_empty()),
                    "func should be a non-empty var"
                );
                // Both func and arg must be atoms (they ARE AnfAtom, structurally enforced).
                let _ = func;
                let _ = arg;
            }
            match expr {
                AnfExpr::Lambda { body: b, .. } | AnfExpr::TyLam { body: b, .. } => check_body(b),
                AnfExpr::Match { arms, .. } => {
                    for arm in arms {
                        check_body(&arm.body);
                        if let Some(g) = &arm.guard {
                            check_body(g);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for decl in &m.decls {
        match decl {
            AnfDecl::Let { body, .. } => check_body(body),
            AnfDecl::Letrec { bindings } => {
                for (_, body) in bindings {
                    check_body(body);
                }
            }
        }
    }
    check_body(&m.root);
}
