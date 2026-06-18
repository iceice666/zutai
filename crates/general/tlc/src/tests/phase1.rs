// ── Phase 1 tests: kind annotations on polymorphism nodes ────────────────────

use crate::{Kind, TlcExpr, TlcType};
use super::tlc_of;

/// Every `TyVar`, `ForAll`, and `TyLam` node produced in Phase 1 must carry
/// `Kind::Type(0)`. This locks the invariant so later phases that introduce
/// non-ground kinds (Phase 3 HKT, Phase 5 constraints) must do so deliberately.
#[test]
fn phase1_all_polymorphism_nodes_carry_ground_kind() {
    // Three representative programs: polymorphic identity, multi-param function,
    // and a type alias reference.
    let programs = [
        "id x = x\nid 42",
        "add x y = x + y\nadd 1 2",
        "Point :: type { x : Int; y : Int; }\nid x = x\nid 0",
    ];

    for src in programs {
        let m = tlc_of(src);

        // Every TyVar in the type arena must carry Kind::Type(0).
        for (_, ty) in m.type_arena.iter() {
            if let TlcType::TyVar(_, kind) = ty {
                assert_eq!(
                    *kind,
                    Kind::Type(0),
                    "TyVar carries non-ground kind in program: {src}"
                );
            }
            if let TlcType::ForAll(_, kind, _) = ty {
                assert_eq!(
                    *kind,
                    Kind::Type(0),
                    "ForAll carries non-ground kind in program: {src}"
                );
            }
        }

        // Every TyLam in the expr arena must carry Kind::Type(0).
        for (_, expr) in m.expr_arena.iter() {
            if let TlcExpr::TyLam(_, kind, _) = expr {
                assert_eq!(
                    *kind,
                    Kind::Type(0),
                    "TyLam carries non-ground kind in program: {src}"
                );
            }
        }
    }
}
