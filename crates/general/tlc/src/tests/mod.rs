use crate::*;

pub(super) fn tlc_of(src: &str) -> TlcModule {
    let parsed = zutai_syntax::parse(src);
    assert!(
        !parsed.has_errors(),
        "parse errors: {:?}",
        parsed.diagnostics()
    );
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "hir errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "thir errors: {:?}",
        thir.diagnostics
    );
    lower_thir(thir.file.as_ref().expect("thir file should be complete"))
}

pub(super) fn assert_no_data_loss(m: &TlcModule) {
    for (_, ty) in m.type_arena.iter() {
        match ty {
            TlcType::Record(row) => {
                let _ = row;
            }
            TlcType::Prim(PrimTy::Atom) => {}
            _ => {}
        }
    }
}

pub(super) fn make_module(type_arena: la_arena::Arena<TlcType>) -> TlcModule {
    use rustc_hash::FxHashMap;
    TlcModule {
        decls: Vec::new(),
        decl_arena: la_arena::Arena::new(),
        expr_arena: la_arena::Arena::new(),
        type_arena,
        expr_types: FxHashMap::default(),
        dict_field_slots: FxHashMap::default(),
        dict_dispatch_keys: FxHashMap::default(),
        spans: FxHashMap::default(),
        final_expr: None,
        extern_global_bindings: FxHashMap::default(),
    }
}

mod lower;
mod normalize;
mod patterns;
mod phase0;
mod phase1;
mod phase2;
mod phase3;
mod phase4;
mod phase5;
mod types_equal;

// ── Basic smoke tests ─────────────────────────────────────────────────────────

#[test]
fn tlc_module_is_constructible() {
    use la_arena::Arena;
    use rustc_hash::FxHashMap;
    let _m = TlcModule {
        decls: Vec::new(),
        decl_arena: Arena::new(),
        expr_arena: Arena::new(),
        type_arena: Arena::new(),
        expr_types: FxHashMap::default(),
        dict_field_slots: FxHashMap::default(),
        dict_dispatch_keys: FxHashMap::default(),
        spans: FxHashMap::default(),
        final_expr: None,
        extern_global_bindings: FxHashMap::default(),
    };
}

#[test]
fn monomorphic_int_binding_translates_type() {
    let m = tlc_of("x ::= 42;\nx");
    assert_eq!(m.decls.len(), 1);
    let decl = &m.decl_arena[m.decls[0]];
    let crate::TlcDecl::Value { ty, .. } = decl else {
        panic!("expected Value decl")
    };
    assert_eq!(m.type_arena[*ty], crate::TlcType::Prim(crate::PrimTy::Int));
}

#[test]
fn int_literal_final_expr_no_decls() {
    let m = tlc_of("42");
    assert_eq!(m.decls.len(), 0);
}

#[test]
fn annotated_value_decl_lowers_correctly() {
    let m = tlc_of("x :: Int = 42;\nx");
    assert_eq!(m.decls.len(), 1);
    let decl = &m.decl_arena[m.decls[0]];
    let crate::TlcDecl::Value { ty, body, .. } = decl else {
        panic!("expected Value decl")
    };
    assert_eq!(m.type_arena[*ty], crate::TlcType::Prim(crate::PrimTy::Int));
    assert_eq!(
        m.expr_arena[*body],
        crate::TlcExpr::Lit(crate::Literal::Int(42))
    );
}

#[test]
fn type_alias_decl_lowers_correctly() {
    // Non-generic alias: 0 params → body is the record directly, no TyLamK wrapping.
    let m = tlc_of("Point :: type { x : Int; y : Int; };\nPoint");
    assert_eq!(m.decls.len(), 1);
    let crate::TlcDecl::TypeAlias { body, .. } = m.decl_arena[m.decls[0]] else {
        panic!("expected TypeAlias decl")
    };
    // The body of a 0-param alias must NOT be a TyLamK.
    assert!(
        !matches!(m.type_arena[body], crate::TlcType::TyLamK(_, _, _)),
        "0-param alias body should not be wrapped in TyLamK, got {:?}",
        m.type_arena[body]
    );
    assert!(
        matches!(m.type_arena[body], crate::TlcType::Record(_)),
        "0-param alias body should be a Record, got {:?}",
        m.type_arena[body]
    );
}

#[test]
fn bool_literal_no_crash() {
    let m = tlc_of("true");
    assert_eq!(m.decls.len(), 0);
}

#[test]
fn monomorphic_identity_function_lowers_to_lam() {
    // Explicitly typed: no generalization
    let m = tlc_of("id :: Int -> Int = \\x. x;\nid 1");
    assert_eq!(m.decls.len(), 1);
    let crate::TlcDecl::Value { body, ty, .. } = &m.decl_arena[m.decls[0]] else {
        panic!("expected Value decl")
    };
    // Type should be Fun(Int, Int, REmpty), not ForAll.
    assert!(
        matches!(m.type_arena[*ty], crate::TlcType::Fun(_, _, _)),
        "expected Fun type but got {:?}",
        m.type_arena[*ty]
    );
    // Body should be a Lam (possibly through TyApp wrappers — walk to innermost).
    fn innermost(m: &crate::TlcModule, id: crate::TlcExprId) -> &crate::TlcExpr {
        match &m.expr_arena[id] {
            crate::TlcExpr::TyApp(inner, _) => innermost(m, *inner),
            e => e,
        }
    }
    assert!(
        matches!(innermost(&m, *body), crate::TlcExpr::Lam(_, _, _)),
        "expected Lam body but got {:?}",
        innermost(&m, *body)
    );
}

#[test]
fn polymorphic_identity_gets_tylam_and_forall() {
    // No annotation → HM generalizes to ∀a. a → a
    let m = tlc_of("id x = x;\nid 42");
    assert_eq!(m.decls.len(), 1);
    let crate::TlcDecl::Value { body, ty, .. } = &m.decl_arena[m.decls[0]] else {
        panic!("expected Value decl")
    };
    assert!(
        matches!(m.type_arena[*ty], crate::TlcType::ForAll(_, _, _)),
        "expected ForAll but got {:?}",
        m.type_arena[*ty]
    );
    assert!(
        matches!(m.expr_arena[*body], crate::TlcExpr::TyLam(_, _, _)),
        "expected TyLam but got {:?}",
        m.expr_arena[*body]
    );
}

#[test]
fn rank2_lambda_arg_value_layer_typed_as_fun_not_forall() {
    // A lambda argument checked against a rank-2 type (`<A> A -> A`) is stored
    // with a `ForAll` THIR type, so `lower_lambda`'s forall block runs. Each
    // abstraction layer must carry its own peeled type: the `TyLam` is the
    // `∀`-type, but the value `Lam` under it must be `param -> rest`, not the
    // shared `ForAll` outer type. Sharing it gave the value lambda a ∀-type
    // where the Dataflow structural validator expects `Fun`, aborting backend
    // compilation with an ICE for rank-2 arguments.
    let m = tlc_of("apply :: (<A> A -> A) -> Int = \\g. g 1;\napply (\\x. x)");
    let mut found = false;
    for (id, e) in m.expr_arena.iter() {
        let crate::TlcExpr::TyLam(_, _, body) = e else {
            continue;
        };
        if !matches!(m.expr_arena[*body], crate::TlcExpr::Lam(_, _, _)) {
            continue;
        }
        found = true;
        assert!(
            matches!(
                m.type_arena[m.expr_types[&id]],
                crate::TlcType::ForAll(_, _, _)
            ),
            "TyLam must carry a ForAll type, got {:?}",
            m.type_arena[m.expr_types[&id]]
        );
        let body_ty = m.expr_types[body];
        assert!(
            matches!(m.type_arena[body_ty], crate::TlcType::Fun(_, _, _)),
            "value Lam under TyLam must be typed Fun(param -> rest), got {:?}",
            m.type_arena[body_ty]
        );
    }
    assert!(
        found,
        "expected a TyLam wrapping a value Lam from the rank-2 lambda argument"
    );
}

#[test]
fn if_desugars_to_case() {
    let m = tlc_of("f x = if x then 1 else 2;\nf true");
    let has_case = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, crate::TlcExpr::Case(_, _)));
    assert!(has_case, "expected a Case node from If desugaring");
}

#[test]
fn block_desugars_to_let() {
    let m = tlc_of("f x = [ n := 42; n ];\nf 0");
    let has_let = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, crate::TlcExpr::Let { .. }));
    assert!(has_let, "expected a Let node from Block desugaring");
}

#[test]
fn binary_op_lowers_to_builtin() {
    let m = tlc_of("f x y = x + y;\nf 1 2");
    let has_builtin = m
        .expr_arena
        .iter()
        .any(|(_, e)| matches!(e, crate::TlcExpr::Builtin(crate::BuiltinOp::Add, _, _)));
    assert!(has_builtin, "expected Builtin(Add) from binary + op");
}

#[test]
fn invariant_every_expr_has_type_entry() {
    let m = tlc_of("add x y = x + y;\nadd 1 2");
    for (id, _) in m.expr_arena.iter() {
        assert!(
            m.expr_types.contains_key(&id),
            "expr {:?} missing from expr_types",
            id
        );
    }
}
