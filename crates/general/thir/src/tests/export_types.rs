use super::*;
use zutai_syntax::posit::PositSpec;

// ── export_type coverage ──────────────────────────────────────────────────────

/// Helper: export the type of the final expression in a completed program.
fn export_final(src: &str) -> ImportedType {
    let file = completed_file(src);
    let final_ty = file.expr_arena[file.final_expr].ty;
    export_type(&file, final_ty).expect("export should succeed")
}

#[test]
fn export_type_int() {
    assert!(matches!(export_final("42"), ImportedType::Int));
}

#[test]
fn export_type_float() {
    assert!(matches!(export_final("1.5"), ImportedType::Float));
}

#[test]
fn export_type_posit() {
    assert!(matches!(
        export_final("1p64e5"),
        ImportedType::Posit(spec) if spec == (PositSpec { nbits: 64, es: 5 })
    ));
}

#[test]
fn export_type_text() {
    assert!(matches!(export_final(r#""hello""#), ImportedType::Text));
}

#[test]
fn export_type_bool_literal() {
    // TypeKind::True from a `true` literal → ImportedType::Bool.
    assert!(matches!(export_final("true"), ImportedType::Bool));
}

#[test]
fn export_type_atom() {
    assert!(matches!(export_final("#foo"), ImportedType::Atom(_)));
}

#[test]
fn export_type_list() {
    assert!(matches!(
        export_final("xs :: List Int = {1; 2;};\nxs"),
        ImportedType::List(_)
    ));
}

#[test]
fn export_type_optional() {
    let file = completed_file("x :: Int? = #none;\nx");
    let ty = file.expr_arena[file.final_expr].ty;
    assert!(matches!(
        export_type(&file, ty),
        Ok(ImportedType::Optional(_))
    ));
}

#[test]
fn export_type_maybe() {
    let file = completed_file("S :: type { v? : Int; };\ns :: S = {};\ns.v");
    let ty = file.expr_arena[file.final_expr].ty;
    assert!(matches!(export_type(&file, ty), Ok(ImportedType::Maybe(_))));
}

#[test]
fn export_type_record() {
    assert!(matches!(
        export_final("{ x = 1; }"),
        ImportedType::Record(_)
    ));
}

#[test]
fn export_type_tuple_positional() {
    // Positional tuple → ImportedType::Tuple with ImportedTupleItem::Positional.
    assert!(matches!(export_final("(1, true)"), ImportedType::Tuple(_)));
}

#[test]
fn export_type_tuple_named() {
    // Named tuple items exercise the TypeTupleItem::Named arm in export.
    let file = completed_file("x :: (a : Int, b : Text) = (a = 1, b = \"hi\");\nx");
    let ty = file.expr_arena[file.final_expr].ty;
    assert!(matches!(export_type(&file, ty), Ok(ImportedType::Tuple(_))));
}

#[test]
fn export_type_union_no_payload() {
    assert!(matches!(
        export_final("R :: type { #ok; #err; };\nx :: R = #ok;\nx"),
        ImportedType::Union(_)
    ));
}

#[test]
fn export_type_union_with_payload() {
    // Union variant with record payload exercises the Some(ty) branch in export.
    assert!(matches!(
        export_final("R :: type { #ok: { v : Int; }; #err; };\nx :: R = #ok { v = 42; };\nx"),
        ImportedType::Union(_)
    ));
}

#[test]
fn export_type_function() {
    assert!(matches!(
        export_final("f :: Int -> Int = \\x. x;\nf"),
        ImportedType::Function { .. }
    ));
}

#[test]
fn export_type_alias_resolves_to_inner_type() {
    // TypeKind::Alias → follows alias map → resolves to Int.
    assert!(matches!(
        export_final("MyInt :: type Int;\nx :: MyInt = 42;\nx"),
        ImportedType::Int
    ));
}

#[test]
fn export_type_type_value() {
    // TypeKind::Type (a type-value binding) → ImportedType::Type.
    assert!(matches!(
        export_final("MyInt :: type Int;\nMyInt"),
        ImportedType::Type(_)
    ));
}

// ── export_type_value: parametric type constructors ──────────────────────────

/// Helper: export the denotation of a final type-value expression as a
/// constructor (mirrors `enrich_with_type_denotations` in the semantic layer).
fn export_final_type_value(src: &str) -> ImportedType {
    let file = completed_file(src);
    let final_expr = &file.expr_arena[file.final_expr];
    let ThirExprKind::TypeValue(tid) = final_expr.kind else {
        panic!("final expression is not a type value");
    };
    export_type_value(&file, tid).expect("export should succeed")
}

#[test]
fn export_type_value_non_parametric_falls_back() {
    // A non-parametric alias must export exactly as before (structural denotation),
    // not as a `TypeCon` — the `serverLib.Server` path stays untouched.
    let exported = export_final_type_value("MyInt :: type Int;\nMyInt");
    assert!(matches!(exported, ImportedType::Int));
}

#[test]
fn export_type_value_parametric_constructor_preserves_params() {
    // `Pair :: <A, B> type { ... }` exports as a two-parameter constructor whose
    // body references the params via `TyVar`.
    let exported = export_final_type_value("Pair :: <A, B> type { first : A; second : B; };\nPair");
    let ImportedType::TypeCon { params, body } = exported else {
        panic!("expected a TypeCon, got {exported:?}");
    };
    assert_eq!(params.len(), 2, "two type parameters");
    let ImportedType::Record(fields) = *body else {
        panic!("expected a record body");
    };
    assert_eq!(fields.len(), 2);
    for field in &fields {
        assert!(
            matches!(field.ty, ImportedType::TyVar(id) if params.contains(&id)),
            "field {} should be a parameter TyVar, got {:?}",
            field.name,
            field.ty
        );
    }
}

#[test]
fn export_type_value_recursive_constructor_is_bounded() {
    // A recursive constructor must export with the self-reference kept as a
    // bounded `ConApply` — never unfolded (the test terminating proves it).
    let exported = export_final_type_value(
        "Lst :: <A> type { #nil; #cons : { head : A; tail : Lst A; }; };\nLst",
    );
    let ImportedType::TypeCon { params, body } = exported else {
        panic!("expected a TypeCon, got {exported:?}");
    };
    assert_eq!(params.len(), 1);
    let ImportedType::Union(variants) = *body else {
        panic!("expected a union body");
    };
    let cons = variants
        .iter()
        .find(|v| v.name == "cons")
        .expect("#cons variant");
    let ImportedType::Record(rec) = cons.payload.as_deref().expect("payload") else {
        panic!("expected a record payload");
    };
    let tail = rec.iter().find(|f| f.name == "tail").expect("tail field");
    assert!(
        matches!(&tail.ty, ImportedType::ConApply { ctor, args }
            if ctor == "Lst" && args.len() == 1),
        "tail should be ConApply Lst[_], got {:?}",
        tail.ty
    );
}

#[test]
fn export_value_type_references_sibling_constructor() {
    // A value whose type mentions `Box A` exports that as a `ConApply` referring
    // to the same constructor, so sibling exports unify with `s.Box Int`.
    let file = completed_file(
        "Box :: <A> type { value : A; };\nwrap :: <A> A -> Box A = x => { value = x; };\nwrap",
    );
    let final_ty = file.expr_arena[file.final_expr].ty;
    let exported = export_type(&file, final_ty).expect("export should succeed");
    let ImportedType::Function { to, .. } = exported else {
        panic!("expected a function, got {exported:?}");
    };
    assert!(
        matches!(&*to, ImportedType::ConApply { ctor, .. } if ctor == "Box"),
        "result should be ConApply Box[_], got {to:?}"
    );
}

#[test]
fn export_higher_kinded_constructor_param_is_refused() {
    // A higher-kinded constructor parameter cannot cross the boundary in this
    // phase — export must refuse rather than emit a malformed descriptor.
    let file = completed_file("Wrap :: <F :: Type -> Type, A> type { value : F A; };\nWrap");
    let final_expr = &file.expr_arena[file.final_expr];
    let ThirExprKind::TypeValue(tid) = final_expr.kind else {
        panic!("final expression is not a type value");
    };
    assert!(
        export_type_value(&file, tid).is_err(),
        "higher-kinded constructor param should be refused"
    );
}

// ── type_matches: Record / Tuple / Union / List deep match ───────────────────

#[test]
fn type_matches_record_to_record_exercises_record_types_match() {
    // `f :: { x : Int; } -> { x : Int; } = \\r. r` forces type_matches on two
    // distinct Record TypeIds with the same structure.
    let file = completed_file("f :: { x : Int; } -> { x : Int; } = \\r. r;\nf { x = 1; }");
    assert!(matches!(final_type_kind(&file), TypeKind::Record(_, _)));
}

#[test]
fn type_matches_tuple_to_tuple_exercises_tuple_types_match() {
    // Function returning its argument of tuple type — distinct tuple TypeIds, same structure.
    let file = completed_file("f :: (Int, Text) -> (Int, Text) = \\p. p;\nf (1, \"a\")");
    assert!(matches!(final_type_kind(&file), TypeKind::Tuple(_)));
}

#[test]
fn type_matches_union_to_union() {
    // Union-to-union: `f :: R -> R = \\x. x`.
    // type_matches is called with two Union TypeIds during function body check.
    // The result type is `R` which is Alias(R_binding).
    let file = completed_file("R :: type { #ok; #err; };\nf :: R -> R = \\x. x;\nf #ok");
    // The file must complete without errors — the union-to-union type check passes.
    let _ = file;
}
