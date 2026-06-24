use super::*;

fn universe_level(ty: &TypeExpr) -> &Level {
    match ty {
        TypeExpr::UniverseType { level, .. } => level,
        other => panic!("expected UniverseType, got {other:?}"),
    }
}

#[test]
fn known_level_annotation() {
    let f = parse_str("Small :: $0 = Int\n1");
    let (_, ty, _) = as_typed(decl_by(&f, "Small"));
    assert!(matches!(universe_level(ty), Level::Known { value: 0, .. }));
}

#[test]
fn level_variable_annotation() {
    let f = parse_str("F :: $l = Int\n1");
    let (_, ty, _) = as_typed(decl_by(&f, "F"));
    assert!(matches!(universe_level(ty), Level::Var { name, .. } if name == "l"));
}

#[test]
fn successor_level() {
    let f = parse_str("F :: $(l + 2) = Int\n1");
    let (_, ty, _) = as_typed(decl_by(&f, "F"));
    match universe_level(ty) {
        Level::Succ { base, by, .. } => {
            assert_eq!(*by, 2);
            assert!(matches!(base.as_ref(), Level::Var { name, .. } if name == "l"));
        }
        other => panic!("expected Succ, got {other:?}"),
    }
}

#[test]
fn max_level() {
    let f = parse_str("F :: $(max a b) = Int\n1");
    let (_, ty, _) = as_typed(decl_by(&f, "F"));
    match universe_level(ty) {
        Level::Max { left, right, .. } => {
            assert!(matches!(left.as_ref(), Level::Var { name, .. } if name == "a"));
            assert!(matches!(right.as_ref(), Level::Var { name, .. } if name == "b"));
        }
        other => panic!("expected Max, got {other:?}"),
    }
}

#[test]
fn nested_max_level() {
    let f = parse_str("F :: $(max a (max b c)) = Int\n1");
    let (_, ty, _) = as_typed(decl_by(&f, "F"));
    match universe_level(ty) {
        Level::Max { right, .. } => {
            assert!(matches!(right.as_ref(), Level::Max { .. }));
        }
        other => panic!("expected Max, got {other:?}"),
    }
}

#[test]
fn value_position_universe() {
    // `$0` on the RHS (value position) parses as a TypeForm of the universe.
    let f = parse_str("TypeOfTypes :: $1 = $0\n1");
    let (_, _, value) = as_typed(decl_by(&f, "TypeOfTypes"));
    match value {
        Expr::TypeForm { ty, .. } => {
            assert!(matches!(universe_level(ty), Level::Known { value: 0, .. }));
        }
        other => panic!("expected TypeForm, got {other:?}"),
    }
}

#[test]
fn single_level_binder() {
    let f =
        parse_str("Pair :: <$l> $l -> $l -> $l\n  = A B => type { first : A; second : B; };\n1");
    let (_, params, _, _) = as_function(decl_by(&f, "Pair"));
    assert_eq!(params.len(), 1);
    assert!(params[0].is_level);
    assert_eq!(params[0].name, "l");
}

#[test]
fn multiple_level_binders() {
    let f = parse_str(
        "Sum :: <$a, $b> $a -> $b -> $(max a b)\n  = X Y => type { #left : X; #right : Y; };\n1",
    );
    let (_, params, _, _) = as_function(decl_by(&f, "Sum"));
    assert_eq!(params.len(), 2);
    assert!(params.iter().all(|p| p.is_level));
}

#[test]
fn mixed_type_and_level_binders() {
    let f = parse_str("F :: <A, $l> A -> $l\n  = x => x;\n1");
    let (_, params, _, _) = as_function(decl_by(&f, "F"));
    assert_eq!(params.len(), 2);
    assert!(!params[0].is_level);
    assert!(params[1].is_level);
}

#[test]
fn non_literal_successor_is_parse_error() {
    // `$(l + m)` — a non-literal addend is rejected.
    assert!(!parse_kinds("F :: $(l + m) = Int\n1").is_empty());
}

#[test]
fn level_binder_rejects_bound() {
    // `$l : Foo` — a level binder may not carry a bound.
    assert!(!parse_kinds("F :: <$l : Foo> Int -> Int\n  = x => x;\n1").is_empty());
}

#[test]
fn universe_round_trips_through_display() {
    let s = parse_str("Small :: $0 = Int\n1").to_string();
    assert!(s.contains("TyUniverse $0"), "got:\n{s}");
    let s = parse_str("F :: $(max a b) = Int\n1").to_string();
    assert!(s.contains("TyUniverse $(max a b)"), "got:\n{s}");
}
