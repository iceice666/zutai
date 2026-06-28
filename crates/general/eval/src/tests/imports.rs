use super::*;

// ─── `.zti` imports ───────────────────────────────────────────────────────────

#[test]
fn top_level_import_zti_field_access() {
    assert_eq!(
        run_import("cfg ::= import \"config.zti\";\ncfg.port"),
        Value::Int(8080)
    );
}

#[test]
fn top_level_import_zt_value_and_type_members() {
    assert_eq!(
        run_import(
            r#"lib ::= import "value_type_members.zt";

server :: lib.Server = {
  host = "localhost";
  port = lib.defaultPort;
};

server.port"#
        ),
        Value::Int(8080)
    );
}

#[test]
fn import_zti_field_access_int() {
    assert_eq!(
        run_import("cfg ::= import \"config.zti\";\ncfg.port"),
        Value::Int(8080)
    );
}

#[test]
fn import_zti_field_access_text() {
    assert_eq!(
        run_import("cfg ::= import \"config.zti\";\ncfg.host"),
        Value::Text("127.0.0.1".into())
    );
}

#[test]
fn import_zti_field_can_flow_through_print_effect() {
    assert_eq!(
        run_import("cfg ::= import \"config.zti\";\nprint cfg.host"),
        Value::Text("127.0.0.1".into())
    );
}

#[test]
fn import_zt_function_can_run_with_repointed_print() {
    assert_eq!(
        run_import("add ::= import \"func_module.zt\";\n[ print \"using import\"; add 2 3 ]"),
        Value::Int(5)
    );
}

#[test]
fn import_zti_field_access_bool() {
    assert_eq!(
        run_import("cfg ::= import \"config.zti\";\ncfg.debug"),
        Value::Bool(true)
    );
}

#[test]
fn import_zti_field_access_atom() {
    assert_eq!(
        run_import("cfg ::= import \"config.zti\";\ncfg.env"),
        Value::Atom("prod".into())
    );
}

#[test]
fn import_zti_nested_field() {
    assert_eq!(
        run_import("cfg ::= import \"config.zti\";\ncfg.limits.max"),
        Value::Int(100)
    );
}

#[test]
fn import_zti_list_field() {
    match run_import("cfg ::= import \"config.zti\";\ncfg.tags") {
        Value::List(items) => assert_eq!(items.len(), 2),
        other => panic!("expected list, got {other:?}"),
    }
}

#[test]
fn import_zti_whole_record() {
    match run_import("cfg ::= import \"config.zti\";\ncfg") {
        Value::Record(fields) => assert_eq!(fields.len(), 6),
        other => panic!("expected record, got {other:?}"),
    }
}

#[test]
fn import_via_eval_path() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/imports/importer.zt");
    assert_eq!(crate::eval_path(&path).unwrap(), Value::Int(8080));
}

#[test]
fn import_without_base_is_not_runnable() {
    // `eval_file` has no base directory, so the import cannot resolve.
    match eval_file("cfg ::= import \"config.zti\";\ncfg.port") {
        Err(EvalError::NotRunnable(_)) => {}
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

#[test]
fn import_missing_file_is_not_runnable() {
    match run_import_err("cfg ::= import \"nope.zti\";\ncfg") {
        EvalError::NotRunnable(_) => {}
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

// ─── `.zt` module imports ─────────────────────────────────────────────────────

#[test]
fn zt_import_scalar_value() {
    // other.zt evaluates to the bare integer 42.
    assert_eq!(run_import("n ::= import \"other.zt\";\nn"), Value::Int(42));
}

#[test]
fn zt_import_posit_value() {
    let value = run_import("p ::= import \"posit_module.zt\";\np");
    assert!(
        matches!(&value, Value::Posit(_)),
        "expected posit, got {value:?}"
    );
    assert!(
        value.to_string().ends_with("p64e5"),
        "expected p64e5 display, got {value}"
    );
}

#[test]
fn zt_import_record_field() {
    // data_module.zt returns a record whose `doubled` field is 21 * 2.
    assert_eq!(
        run_import("m ::= import \"data_module.zt\";\nm.doubled"),
        Value::Int(42)
    );
}

#[test]
fn zt_importer_fixture_runs_via_eval_path() {
    assert_eq!(
        crate::eval_path(&imports_path("zt_importer.zt")).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn zt_import_whole_record() {
    match run_import("m ::= import \"data_module.zt\";\nm") {
        Value::Record(fields) => assert_eq!(fields.len(), 3),
        other => panic!("expected record, got {other:?}"),
    }
}

#[test]
fn zt_import_transitive_through_zti() {
    // chain_top.zt imports chain_mid.zt which imports config.zti.
    assert_eq!(
        crate::eval_path(&imports_path("chain_top.zt")).unwrap(),
        Value::Int(8080)
    );
}

#[test]
fn zt_import_function_is_callable() {
    // func_module.zt exports `add :: Int -> Int -> Int`.  Calling it across
    // the module boundary must yield the correct result.
    assert_eq!(
        run_import("f ::= import \"func_module.zt\";\nf 2 3"),
        Value::Int(5)
    );
}

#[test]
fn zt_import_function_partial_application() {
    // Partially-applied cross-module function retains the correct arity.
    assert_eq!(
        run_import("f ::= import \"func_module.zt\";\n(f 10) 7"),
        Value::Int(17)
    );
}

#[test]
fn zt_import_sibling_call() {
    // sibling_module.zt: add2 calls `inc` (a sibling top-level binding in the
    // same module).  This exercises the arena switch on BindingRef resolution.
    assert_eq!(
        run_import("lib ::= import \"sibling_module.zt\";\nlib 3"),
        Value::Int(5)
    );
}

#[test]
fn zt_import_mixed_record_data_field() {
    // mixed_module.zt exports a record with both data and function fields.
    // Reading a data field must still work.
    assert_eq!(
        run_import("m ::= import \"mixed_module.zt\";\nm.version"),
        Value::Int(1)
    );
}

#[test]
fn zt_import_mixed_record_function_call() {
    // Calling a function field from an imported mixed record.
    assert_eq!(
        run_import("m ::= import \"mixed_module.zt\";\nm.double 21"),
        Value::Int(42)
    );
}

#[test]
fn zt_import_cycle_is_refused() {
    match crate::eval_path(&imports_path("cycle_a.zt")) {
        Err(EvalError::NotRunnable(_)) => {}
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}
// ─── from_immediate: Im::False and Im::Float ─────────────────────────────────

#[test]
fn import_zti_false_value() {
    // meta.zti has `active = false` — exercises Im::False arm in from_immediate.
    assert_eq!(
        run_import("m ::= import \"meta.zti\";\nm.active"),
        Value::Bool(false)
    );
}

#[test]
fn import_zti_float_value() {
    // meta.zti has `score = 2.5` — exercises Im::Float arm in from_immediate.
    assert_eq!(
        run_import("m ::= import \"meta.zti\";\nm.score"),
        Value::Float(2.5)
    );
}

// ─── .zti import coverage: Text, Atom, List ───────────────────────────────────

#[test]
fn import_zti_text_field() {
    // config.zti has `host = "127.0.0.1"` — exercises ImportedType::Text in import.rs
    assert_eq!(
        run_import("cfg ::= import \"config.zti\";\ncfg.host"),
        Value::Text("127.0.0.1".into())
    );
}

#[test]
fn import_zti_atom_field() {
    // config.zti has `env = #prod` — exercises ImportedType::Atom in import.rs
    assert_eq!(
        run_import("cfg ::= import \"config.zti\";\ncfg.env"),
        Value::Atom("prod".into())
    );
}

#[test]
fn import_zti_empty_list_field() {
    // empty_list.zti has `items = []` — exercises ImportedType::Unknown via empty array
    match run_import("m ::= import \"empty_list.zti\";\nm.items") {
        Value::List(items) => assert!(items.is_empty(), "expected empty list"),
        other => panic!("expected List, got {other:?}"),
    }
}

// ─── .zt import coverage: Optional, Tuple, Union, Type ───────────────────────

#[test]
fn import_zt_optional_module() {
    // optional_module.zt exports Int? — exercises ImportedType::Optional in import.rs
    // the exported value is #none.
    assert_eq!(
        run_import("m ::= import \"optional_module.zt\";\nm"),
        Value::Atom("none".into())
    );
}

#[test]
fn import_zt_tuple_module() {
    // tuple_module.zt exports (Int, Text) — exercises ImportedType::Tuple in import.rs
    match run_import("m ::= import \"tuple_module.zt\";\nm") {
        Value::Tuple(fields) => {
            assert_eq!(fields.len(), 2, "tuple should have 2 fields");
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

#[test]
fn import_zt_union_module() {
    // union_module.zt exports Color (a union) — exercises ImportedType::Union in import.rs
    assert_eq!(
        run_import("m ::= import \"union_module.zt\";\nm"),
        Value::Atom("red".into())
    );
}

#[test]
fn import_zt_type_module() {
    // type_module.zt exports MyInt (a type alias reference) — ImportedType::Type in import.rs.
    // TLC now maps TypeKind::Type to PrimTy::Nothing instead of panicking; the THIR evaluator
    // returns Value::TypeValue for the imported type alias reference.
    let v = run_import("m ::= import \"type_module.zt\";\nm");
    assert!(
        matches!(v, Value::TypeValue(_)),
        "expected TypeValue for imported type alias, got {v:?}"
    );
}

#[test]
fn strict_tlc_rejects_imported_type_value() {
    let src = "m ::= import \"type_module.zt\";\nm";
    match eval_tlc_with_base(src, Some(&imports_dir())).unwrap_err() {
        EvalError::ReflectionUnsupported(message) => {
            assert!(message.contains("runtime Type values"));
        }
        other => panic!("expected ReflectionUnsupported, got {other:?}"),
    }
}

// ─── embedded stdlib + destructuring imports ──────────────────────────────────

#[test]
fn stdlib_stream_qualified_members_evaluate() {
    // `import stdlib.stream` resolves to the embedded module with no base dir.
    let src = "s ::= import stdlib.stream;\n\
               s.fold (\\acc x. acc + x) 0 (s.take 3 (s.cons 10 (s.cons 20 (s.singleton 30))))";
    assert_eq!(run(src), Value::Int(60));
}

#[test]
fn destructured_stdlib_members_evaluate_unqualified() {
    let src = "s ::= import stdlib.stream;\n\
               { map; fold; singleton; cons; } ::= s;\n\
               fold (\\acc x. acc + x) 0 (map (\\x. x * 2) (cons 1 (cons 2 (singleton 3))))";
    assert_eq!(run(src), Value::Int(12));
}

#[test]
fn mixed_destructured_and_ambient_stream_combinators_evaluate() {
    // Imported `map`/`fold` expose the `stdlib.stream` module's `s.Stream`
    // constructor while ambient `take`/`unfold` use the fallback prelude
    // constructor. Both denote the same equirecursive codata type, so unification
    // must converge instead of exhausting type-level expansion fuel.
    let src = "{ map; fold; } ::= import stdlib.stream;\n\
               fold (\\acc x. acc + x) 0 (take 3 (map (\\x. x * 2) (unfold (\\st. #yield { item = st; next = st + 1; }) 1)))";
    assert_eq!(run(src), Value::Int(12));
}

#[test]
fn stdlib_prelude_qualified_members_evaluate() {
    // `import stdlib.prelude` resolves to the embedded module with no base dir.
    let src = "p ::= import stdlib.prelude;\n\
               (p.compose (\\x. x + 1) (\\x. x * 2) 3) + (p.fold (\\acc x. acc + x) 0 (p.map (\\x. x * 2) {1; 2;})) + (match p.head? {9;} { | #none => 0; | #some (h) => h; })";
    assert_eq!(run(src), Value::Int(22));
}

#[test]
fn destructured_stdlib_prelude_members_evaluate() {
    let src = "p ::= import stdlib.prelude;\n\
               { id; compose; map; fold; head?; } ::= p;\n\
               (compose (\\x. x + 1) (\\x. x * 2) (id 3)) + (fold (\\acc x. acc + x) 0 (map (\\x. x * 2) {1; 2;})) + (match head? {9;} { | #none => 0; | #some (h) => h; })";
    assert_eq!(run(src), Value::Int(22));
}

#[test]
fn stdlib_optional_qualified_members_evaluate() {
    let src = "o ::= import stdlib.optional;\n\
               noneInt :: Int? = #none;\n\
               a ::= o.withDefault 0 (o.map (\\x. x + 1) (#some (40)));\n\
               b ::= o.withDefault 0 (o.andThen (\\x. if x > 0 then #some (x + 1) else #none) (#some (1)));\n\
               c ::= o.withDefault 0 (o.filter (\\x. x > 3) (#some (4)));\n\
               d ::= if o.isSome noneInt then 100 else 0;\n\
               e ::= length (o.toList (#some (9)));\n\
               f ::= length (o.toList noneInt);\n\
               a + b + c + d + e + f";
    assert_eq!(run(src), Value::Int(48));
}

#[test]
fn destructured_stdlib_optional_members_evaluate() {
    let src = "{ map; withDefault; isSome; } ::= import stdlib.optional;\n\
               if isSome (map (\\x. x + 1) (#some (4))) then withDefault 0 (map (\\x. x + 1) (#some (4))) else 0";
    assert_eq!(run(src), Value::Int(5));
}

#[test]
fn stdlib_optional_thir_oracle_matches_tlc_path() {
    let srcs = [
        "o ::= import stdlib.optional;\no.withDefault 0 (o.map (\\x. x + 1) (#some (4)))",
        "o ::= import stdlib.optional;\no.withDefault 7 (o.andThen (\\x. #none) (#some (4)))",
        "o ::= import stdlib.optional;\no.toList (#some (3))",
        "o ::= import stdlib.optional;\nnoneInt :: Int? = #none;\no.toList noneInt",
    ];
    for src in srcs {
        let tlc = eval_file(src).expect("TLC eval failed");
        let thir = eval_thir_file(src).expect("THIR oracle eval failed");
        assert_eq!(tlc, thir, "TLC and THIR oracle disagree for:\n{src}");
    }
}

// ─── imported parametric type constructors ────────────────────────────────────

#[test]
fn imported_stdlib_stream_type_constructor_annotation() {
    // `s.Stream Int` in annotation position, with the value built by imported
    // combinators that themselves return the same `Stream A`.
    let src = "s ::= import stdlib.stream;\n\
               xs :: s.Stream Int = s.fromList {1; 2; 3;};\n\
               s.fold (\\acc x. acc + x) 0 (s.take 2 xs)";
    assert_eq!(run(src), Value::Int(3));
}

#[test]
fn imported_user_stream_type_constructor_round_trips() {
    let src = "m ::= import \"stream_module.zt\";\n\
               xs :: m.Stream Int = m.fromList {1; 2; 3;};\n\
               m.takeList 2 xs == {1; 2;}";
    assert_eq!(run_import(src), Value::Bool(true));
}

#[test]
fn imported_user_stream_constructed_via_cons() {
    let src = "m ::= import \"stream_module.zt\";\n\
               xs :: m.Stream Int = m.cons 1 (m.cons 2 m.empty);\n\
               m.takeList 5 xs == {1; 2;}";
    assert_eq!(run_import(src), Value::Bool(true));
}

#[test]
fn imported_stream_multiple_instantiations() {
    // One imported constructor instantiated at two distinct argument types.
    let src = "m ::= import \"stream_module.zt\";\n\
               xs :: m.Stream Int = m.fromList {1; 2;};\n\
               ys :: m.Stream Text = m.fromList {\"a\";};\n\
               (m.takeList 2 xs == {1; 2;}) && (m.takeList 1 ys == {\"a\";})";
    assert_eq!(run_import(src), Value::Bool(true));
}

#[test]
fn imported_stream_matches_ambient_stream() {
    // The same computation via the ambient prelude `Stream` and via an imported
    // user module's `m.Stream Int` must yield the same value (THIR oracle).
    let ambient = "takeList 2 (fromList {1; 2; 3;}) == {1; 2;}";
    let imported = "m ::= import \"stream_module.zt\";\n\
                    xs :: m.Stream Int = m.fromList {1; 2; 3;};\n\
                    m.takeList 2 xs == {1; 2;}";
    assert_eq!(run(ambient), run_import(imported));
    assert_eq!(run_import(imported), Value::Bool(true));
}

#[test]
fn strict_tlc_gates_imported_parametric_constructor_module() {
    // TLC elaboration of `m.Stream Int` succeeds (otherwise this would be a type
    // error, not `ReflectionUnsupported`); only the pre-existing runtime
    // type-value gate refuses, because the module exports a `Stream` type value.
    let src = "m ::= import \"stream_module.zt\";\n\
               xs :: m.Stream Int = m.fromList {1; 2; 3;};\n\
               m.takeList 2 xs == {1; 2;}";
    match eval_tlc_with_base(src, Some(&imports_dir())).unwrap_err() {
        EvalError::ReflectionUnsupported(message) => {
            assert!(message.contains("runtime Type values"));
        }
        other => panic!("expected ReflectionUnsupported, got {other:?}"),
    }
}

#[test]
fn imported_bare_parametric_constructor_is_refused() {
    // A parametric constructor used without arguments (`m.Stream`) is an arity
    // error, like a local generic alias — never silently accepted.
    let src = "m ::= import \"stream_module.zt\";\n\
               x :: m.Stream = m.empty;\n\
               x";
    let _ = run_import_err(src);
}

#[test]
fn imported_higher_kinded_constructor_is_refused() {
    // `Wrap`'s parameter `F` is higher-kinded; export refuses it, so the importer
    // cannot apply `m.Wrap` and the program is refused rather than mistyped.
    let src = "m ::= import \"hkt_module.zt\";\n\
               x :: m.Wrap Int = { value = 1; };\n\
               x";
    let _ = run_import_err(src);
}
