use super::*;
use crate::UniverseLevel;

fn diagnostics(src: &str) -> Vec<ThirDiagnosticKind> {
    lower(src).diagnostics.into_iter().map(|d| d.kind).collect()
}

fn count_too_low(src: &str) -> usize {
    diagnostics(src)
        .iter()
        .filter(|k| matches!(k, ThirDiagnosticKind::ExplicitLevelTooLow { .. }))
        .count()
}

// ── Spec examples (docs/v2_spec/04-universe-levels.md "Explicit Level Syntax") ──

#[test]
fn small_accepts_known_level() {
    // `Small :: $0 = Int` — Int : Type-0, fits in $0.
    completed_file("Small :: $0 = Int;\n1");
}

#[test]
fn type_of_types_accepts() {
    // `TypeOfTypes :: $1 = $0` — $0 : $1.
    completed_file("TypeOfTypes :: $1 = $0;\n1");
}

#[test]
fn explicit_level_too_low_rejected() {
    // `Bad :: $0 = $0` — $0 : $1, not $0.
    assert_eq!(count_too_low("Bad :: $0 = $0;\n1"), 1);
}

#[test]
fn cumulativity_accepts_higher_annotation() {
    // `Ok :: $5 = Int` — Int : Type-0 within $5.
    completed_file("Ok :: $5 = Int;\n1");
}

#[test]
fn level_binder_identity_accepts() {
    // `<$l>` binder on a type-level identity. Exercises level-binder resolution
    // and per-use linking across the signature.
    completed_file("Id :: <$l> $l -> $l\n  = x => x;\n1");
}

#[test]
fn successor_and_max_levels_accept() {
    // `$(l + 1)` and `$(max a b)` parse, resolve, and check.
    completed_file("Bump :: <$l> $l -> $(l + 1)\n  = x => x;\n1");
    completed_file("Join :: <$a, $b> $a -> $b -> $(max a b)\n  = x y => x;\n1");
}

// ── Per-use linking, NOT prenex polymorphism ──

/// Collect every `TypeKind::Type(level)` reachable from a type, following the
/// function-arrow spine.
fn collect_universe_levels(file: &ThirFile, ty: TypeId, out: &mut Vec<UniverseLevel>) {
    match &file.type_arena[ty.0 as usize].kind {
        TypeKind::Type(level) => out.push(level.clone()),
        TypeKind::Function { from, to } => {
            collect_universe_levels(file, *from, out);
            collect_universe_levels(file, *to, out);
        }
        _ => {}
    }
}

#[test]
fn level_binder_links_all_occurrences() {
    // `<$l> $l -> $l -> $l`: every `$l` shares ONE meta (linking), not three
    // independent inferred levels.
    let file = completed_file("Pair :: <$l> $l -> $l -> $l\n  = A B => A;\n1");
    let sig = file
        .decls
        .iter()
        .find_map(|&id| match &file.decl_arena[id].kind {
            ThirDeclKind::Function { sig, .. } => Some(*sig),
            _ => None,
        })
        .expect("Pair should lower to a function decl");
    let mut levels = Vec::new();
    collect_universe_levels(&file, sig, &mut levels);
    assert_eq!(
        levels.len(),
        3,
        "expected three `$l` universes, got {levels:?}"
    );
    assert!(
        levels.iter().all(|l| *l == levels[0]),
        "all `$l` occurrences must share one level (linking), got {levels:?}"
    );
    assert!(
        matches!(levels[0], UniverseLevel::Meta(_)),
        "linked level should be a single shared meta, got {:?}",
        levels[0]
    );
}

// ── Verification gate: explicit levels reject nothing bare `Type` accepts ──

#[test]
fn bare_type_higher_kinded_still_accepts() {
    completed_file("Pair :: <A, B> type { first : A; second : B; };\ntype (Pair Int Text)");
}

#[test]
fn pair_int_type_still_accepts() {
    // `Pair Int Type` was accepted before explicit levels; it still is.
    completed_file("Pair :: <A, B> type { first : A; second : B; };\ntype (Pair Int Type)");
}

#[test]
fn type_level_identity_with_bare_type_unaffected() {
    // The bare-`Type` form of the identity must keep working unchanged.
    completed_file("Id :: Type -> Type\n  = x => x;\n1");
}

#[test]
fn recursive_type_valued_alias_universe_terminates() {
    // Step 0 risk: the `type_universe` cache seeds `Known(0)` as a cycle breaker.
    // A self-referential generic alias must still compute a universe (no infinite
    // expansion, no spurious universe-level cycle) now that `Type` carries a level.
    completed_file(
        "Tree :: <A> type { #leaf : A; #node : { left : Tree A; right : Tree A; }; };\n\
         type (Tree Int)",
    );
}
