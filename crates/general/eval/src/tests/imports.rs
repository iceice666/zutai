use super::*;

// ─── `.zti` imports ───────────────────────────────────────────────────────────

#[test]
fn import_zti_field_access_int() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.port"),
        Value::Int(8080)
    );
}

#[test]
fn import_zti_field_access_text() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.host"),
        Value::Text("127.0.0.1".into())
    );
}

#[test]
fn import_zti_field_can_flow_through_print_effect() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\nprint cfg.host"),
        Value::Text("127.0.0.1".into())
    );
}

#[test]
fn import_zt_function_can_run_with_repointed_print() {
    assert_eq!(
        run_import("add := import \"func_module.zt\"\n{ print \"using import\"; add 2 3 }"),
        Value::Int(5)
    );
}

#[test]
fn import_zti_field_access_bool() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.debug"),
        Value::Bool(true)
    );
}

#[test]
fn import_zti_field_access_atom() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.env"),
        Value::Atom("prod".into())
    );
}

#[test]
fn import_zti_nested_field() {
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.limits.max"),
        Value::Int(100)
    );
}

#[test]
fn import_zti_list_field() {
    match run_import("cfg := import \"config.zti\"\ncfg.tags") {
        Value::List(items) => assert_eq!(items.len(), 2),
        other => panic!("expected list, got {other:?}"),
    }
}

#[test]
fn import_zti_whole_record() {
    match run_import("cfg := import \"config.zti\"\ncfg") {
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
    match eval_file("cfg := import \"config.zti\"\ncfg.port") {
        Err(EvalError::NotRunnable(_)) => {}
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

#[test]
fn import_missing_file_is_not_runnable() {
    match run_import_err("cfg := import \"nope.zti\"\ncfg") {
        EvalError::NotRunnable(_) => {}
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

// ─── `.zt` module imports ─────────────────────────────────────────────────────

#[test]
fn zt_import_scalar_value() {
    // other.zt evaluates to the bare integer 42.
    assert_eq!(run_import("n := import \"other.zt\"\nn"), Value::Int(42));
}

#[test]
fn zt_import_record_field() {
    // data_module.zt returns a record whose `doubled` field is 21 * 2.
    assert_eq!(
        run_import("m := import \"data_module.zt\"\nm.doubled"),
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
    match run_import("m := import \"data_module.zt\"\nm") {
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
        run_import("f := import \"func_module.zt\"\nf 2 3"),
        Value::Int(5)
    );
}

#[test]
fn zt_import_function_partial_application() {
    // Partially-applied cross-module function retains the correct arity.
    assert_eq!(
        run_import("f := import \"func_module.zt\"\n(f 10) 7"),
        Value::Int(17)
    );
}

#[test]
fn zt_import_sibling_call() {
    // sibling_module.zt: add2 calls `inc` (a sibling top-level binding in the
    // same module).  This exercises the arena switch on BindingRef resolution.
    assert_eq!(
        run_import("lib := import \"sibling_module.zt\"\nlib 3"),
        Value::Int(5)
    );
}

#[test]
fn zt_import_mixed_record_data_field() {
    // mixed_module.zt exports a record with both data and function fields.
    // Reading a data field must still work.
    assert_eq!(
        run_import("m := import \"mixed_module.zt\"\nm.version"),
        Value::Int(1)
    );
}

#[test]
fn zt_import_mixed_record_function_call() {
    // Calling a function field from an imported mixed record.
    assert_eq!(
        run_import("m := import \"mixed_module.zt\"\nm.double 21"),
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
        run_import("m := import \"meta.zti\"\nm.active"),
        Value::Bool(false)
    );
}

#[test]
fn import_zti_float_value() {
    // meta.zti has `score = 2.5` — exercises Im::Float arm in from_immediate.
    assert_eq!(
        run_import("m := import \"meta.zti\"\nm.score"),
        Value::Float(2.5)
    );
}

// ─── .zti import coverage: Text, Atom, List ───────────────────────────────────

#[test]
fn import_zti_text_field() {
    // config.zti has `host = "127.0.0.1"` — exercises ImportedType::Text in import.rs
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.host"),
        Value::Text("127.0.0.1".into())
    );
}

#[test]
fn import_zti_atom_field() {
    // config.zti has `env = #prod` — exercises ImportedType::Atom in import.rs
    assert_eq!(
        run_import("cfg := import \"config.zti\"\ncfg.env"),
        Value::Atom("prod".into())
    );
}

#[test]
fn import_zti_empty_list_field() {
    // empty_list.zti has `items = []` — exercises ImportedType::Unknown via empty array
    match run_import("m := import \"empty_list.zti\"\nm.items") {
        Value::List(items) => assert!(items.is_empty(), "expected empty list"),
        other => panic!("expected List, got {other:?}"),
    }
}

// ─── .zt import coverage: Optional, Tuple, Union, Type ───────────────────────

#[test]
fn import_zt_optional_module() {
    // optional_module.zt exports Int? — exercises ImportedType::Optional in import.rs
    // cfg.port is absent so the result is #none.
    assert_eq!(
        run_import("m := import \"optional_module.zt\"\nm"),
        Value::Atom("none".into())
    );
}

#[test]
fn import_zt_tuple_module() {
    // tuple_module.zt exports (Int, Text) — exercises ImportedType::Tuple in import.rs
    match run_import("m := import \"tuple_module.zt\"\nm") {
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
        run_import("m := import \"union_module.zt\"\nm"),
        Value::Atom("red".into())
    );
}

#[test]
fn import_zt_type_module() {
    // type_module.zt exports MyInt (a type alias reference) — ImportedType::Type in import.rs.
    // TLC now maps TypeKind::Type to PrimTy::Nothing instead of panicking; the THIR evaluator
    // returns Value::TypeValue for the imported type alias reference.
    let v = run_import("m := import \"type_module.zt\"\nm");
    assert!(
        matches!(v, Value::TypeValue(_)),
        "expected TypeValue for imported type alias, got {v:?}"
    );
}

#[test]
fn strict_tlc_rejects_imported_type_value() {
    let src = "m := import \"type_module.zt\"\nm";
    match eval_tlc_with_base(src, Some(&imports_dir())).unwrap_err() {
        EvalError::ReflectionUnsupported(message) => {
            assert!(message.contains("runtime Type values"));
        }
        other => panic!("expected ReflectionUnsupported, got {other:?}"),
    }
}
