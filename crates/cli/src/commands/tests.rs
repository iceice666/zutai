use super::*;
use std::path::PathBuf;
use std::rc::Rc;
use zutai_eval::thunk::Thunk;
use zutai_eval::value::{BuiltinFn, TupleField};

#[test]
fn count_decls_in_returns_zero_for_unparseable() {
    assert_eq!(count_decls_in(""), 0);
}

#[test]
fn count_decls_in_returns_one_for_single_decl() {
    assert_eq!(count_decls_in("x ::= 1;\nx\n"), 1);
}

#[test]
fn count_decls_in_returns_two_for_two_decls() {
    assert_eq!(count_decls_in("x ::= 1;\ny ::= 2;\nx\n"), 2);
}

#[test]
fn runtime_link_flags_never_request_non_pie() {
    assert!(!runtime_link_flags().contains(&"-no-pie"));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_runtime_link_flags_request_pie() {
    assert!(runtime_link_flags().contains(&"-pie"));
    assert!(!shared_runtime_link_flags().contains(&"-pie"));
}

#[test]
fn value_to_source_covers_scalar_escapes_and_float_suffix() {
    assert_eq!(
        value_to_source(&zutai_eval::Value::Bool(true)),
        Some("true".to_string())
    );
    assert_eq!(
        value_to_source(&zutai_eval::Value::Float(2.0)),
        Some("2.0".to_string())
    );
    assert_eq!(
        value_to_source(&zutai_eval::Value::Float(f64::INFINITY)),
        Some("inf".to_string())
    );
    assert_eq!(
        value_to_source(&zutai_eval::Value::Atom("prod".into())),
        Some("#prod".to_string())
    );
    let text = "quote\" slash\\ line\n cr\r tab\t";
    assert_eq!(
        value_to_source(&zutai_eval::Value::Text(text.into())),
        Some("\"quote\\\" slash\\\\ line\\n cr\\r tab\\t\"".to_string())
    );
}

#[test]
fn value_to_source_covers_tuple_tagged_absent_and_none() {
    let one = Thunk::ready(zutai_eval::Value::Int(1));
    let tuple = zutai_eval::Value::Tuple(Rc::from([
        TupleField {
            name: Some("x".into()),
            value: one.clone(),
        },
        TupleField {
            name: None,
            value: Thunk::ready(zutai_eval::Value::Text("y".into())),
        },
    ]));
    assert_eq!(value_to_source(&tuple), Some("(x = 1, \"y\")".to_string()));

    assert_eq!(
        value_to_source(&zutai_eval::Value::TaggedValue {
            tag: "none".into(),
            payload: Rc::new(vec![]),
        }),
        Some("#none".to_string())
    );
    assert_eq!(
        value_to_source(&zutai_eval::Value::TaggedValue {
            tag: "some".into(),
            payload: Rc::new(vec![
                ("0".into(), Thunk::ready(zutai_eval::Value::Int(1))),
                (
                    "1".into(),
                    Thunk::ready(zutai_eval::Value::Text("x".into())),
                ),
            ]),
        }),
        Some("#some (1, \"x\")".to_string())
    );
    assert_eq!(
        value_to_source(&zutai_eval::Value::TaggedValue {
            tag: "point".into(),
            payload: Rc::new(vec![
                ("x".into(), Thunk::ready(zutai_eval::Value::Int(1))),
                ("y".into(), Thunk::ready(zutai_eval::Value::Int(2))),
            ]),
        }),
        Some("#point {x = 1; y = 2; }".to_string())
    );
    assert_eq!(
        value_to_source(&zutai_eval::Value::Nothing),
        Some("#absent".to_string())
    );
    assert_eq!(
        value_to_source(&zutai_eval::Value::Builtin(BuiltinFn::Print)),
        None
    );
}

#[test]
fn output_path_for_derives_default_paths() {
    assert_eq!(
        output_path_for("main.zt", None, EmitMode::Llvm),
        PathBuf::from("main.ll")
    );
    assert_eq!(
        output_path_for("main.zt", None, EmitMode::Obj),
        PathBuf::from("main.o")
    );
    assert_eq!(
        output_path_for("main.zt", None, EmitMode::Bin),
        PathBuf::from("main")
    );
    assert_eq!(
        output_path_for("main.zt", None, EmitMode::Lib),
        PathBuf::from(format!("libmain{}", shared_library_extension()))
    );
    assert_eq!(
        output_path_for("main.zt", Some("custom.out"), EmitMode::Bin),
        PathBuf::from("custom.out")
    );
}

#[test]
fn missing_native_tool_message_points_to_env_var_and_dev_shell() {
    let mut command = Command::new("__zutai_missing_tool_for_test__");
    let err = run_tool(&mut command, "llc", "assembling LLVM IR")
        .expect_err("missing tool should fail before spawning");
    let message = err.to_string();
    assert!(message.contains("required tool `llc` failed to start"));
    assert!(message.contains("ZUTAI_LLC"));
    assert!(message.contains("nix develop"));
}

// ─── eval isolation unit tests ────────────────────────────────────────────────

#[test]
fn run_isolated_catches_panic() {
    // Swap in a silent hook so the worker panic doesn't pollute test stderr.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = run_isolated(|| panic!("kaboom"));
    std::panic::set_hook(prev);
    match outcome {
        EvalOutcome::Panicked(m) => assert!(m.contains("kaboom"), "unexpected message: {m}"),
        other => panic!("expected Panicked, got {other:?}"),
    }
}

#[test]
fn run_isolated_returns_ok() {
    assert_eq!(
        run_isolated(|| Ok("42".to_string())),
        EvalOutcome::Ok("42".to_string())
    );
}

#[test]
fn run_isolated_propagates_eval_error() {
    assert_eq!(
        run_isolated(|| Err(zutai_eval::EvalError::DivByZero)),
        EvalOutcome::Err(zutai_eval::EvalError::DivByZero)
    );
}

#[test]
fn eval_isolated_evaluates_expression() {
    assert_eq!(
        eval_isolated("if false then 1 else 2\n", None),
        EvalOutcome::Ok("2".to_string())
    );
}

#[test]
fn eval_isolated_reports_eval_error_not_panic() {
    assert_eq!(
        eval_isolated("9223372036854775807 + 1\n", None),
        EvalOutcome::Err(zutai_eval::EvalError::IntOverflow("+"))
    );
}
