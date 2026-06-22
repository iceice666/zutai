// ── Phase 1 tests: kind annotations on polymorphism nodes ────────────────────

use super::tlc_of;
use crate::{Kind, TlcExpr, TlcType};

#[test]
fn ordinary_polymorphism_defaults_to_lowest_universe() {
    let programs = [
        "id x = x\nid 42",
        "const a b = a\nconst 1 2",
        "add x y = x + y\nadd 1 2",
    ];

    for src in programs {
        let m = tlc_of(src);

        for (_, ty) in m.type_arena.iter() {
            if let TlcType::TyVar(_, kind) = ty {
                assert_eq!(
                    *kind,
                    Kind::Type(0),
                    "ordinary TyVar carries non-ground kind in program: {src}"
                );
            }
            if let TlcType::ForAll(_, kind, _) = ty {
                assert_eq!(
                    *kind,
                    Kind::Type(0),
                    "ordinary ForAll carries non-ground kind in program: {src}"
                );
            }
        }

        for (_, expr) in m.expr_arena.iter() {
            if let TlcExpr::TyLam(_, kind, _) = expr {
                assert_eq!(
                    *kind,
                    Kind::Type(0),
                    "ordinary TyLam carries non-ground kind in program: {src}"
                );
            }
        }
    }
}

#[test]
fn higher_universe_alias_application_reaches_tlc_kind() {
    let m = tlc_of(
        r#"
Pair :: <A, B> type { first : A; second : B; }
MetaPair :: type Pair Int Type
MetaPair
"#,
    );

    let has_type_one = m.type_arena.iter().any(|(_, ty)| match ty {
        TlcType::TyVar(_, kind) => kind_mentions_type_one(kind),
        TlcType::TyLamK(_, kind, _) | TlcType::ForAll(_, kind, _) => kind_mentions_type_one(kind),
        _ => false,
    });

    assert!(
        has_type_one,
        "expected Pair Int Type lowering to carry Kind::Type(1)"
    );
}

#[test]
fn unused_higher_universe_alias_arg_keeps_tlc_result_ground() {
    let m = tlc_of(
        r#"
Const :: <A, B> type A
ConstIntType :: type Const Int Type
ConstIntType
"#,
    );

    let has_type_one = m.type_arena.iter().any(|(_, ty)| match ty {
        TlcType::TyVar(_, kind) => kind_mentions_type_one(kind),
        TlcType::TyLamK(_, kind, _) | TlcType::ForAll(_, kind, _) => kind_mentions_type_one(kind),
        _ => false,
    });

    assert!(
        !has_type_one,
        "unused Type argument must not raise Const Int Type result kind"
    );
}

#[test]
fn nested_alias_application_preserves_outer_substitution_level() {
    let m = tlc_of(
        r#"
Pair :: <A, B> type { first : A; second : B; }
Wrap :: <X> type Pair X Text
Use :: type Wrap Type
Use
"#,
    );

    let has_type_one = m.type_arena.iter().any(|(_, ty)| match ty {
        TlcType::TyVar(_, kind) => kind_mentions_type_one(kind),
        TlcType::TyLamK(_, kind, _) | TlcType::ForAll(_, kind, _) => kind_mentions_type_one(kind),
        _ => false,
    });

    assert!(
        has_type_one,
        "Wrap Type must lower through nested Pair X Text at Kind::Type(1)"
    );
}

fn kind_mentions_type_one(kind: &Kind) -> bool {
    match kind {
        Kind::Type(1) => true,
        Kind::Type(_) => false,
        Kind::Row(inner) => kind_mentions_type_one(inner),
        Kind::Arrow(from, to) => kind_mentions_type_one(from) || kind_mentions_type_one(to),
    }
}
