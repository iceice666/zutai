use super::*;
use crate::{EffectHandler, TlcSession};
use std::cell::RefCell;
use std::collections::BTreeMap;

#[test]
fn persistent_session_applies_entry_closure_repeatedly() {
    let analysis = zutai_semantic::analyze("add :: Int -> Int = x => x + 1;\nadd");
    let session = TlcSession::from_analysis(&analysis).unwrap();
    let add = session.entry().unwrap();

    assert_eq!(
        session.apply(add.clone(), Value::Int(1)).unwrap(),
        Value::Int(2)
    );
    assert_eq!(session.apply(add, Value::Int(41)).unwrap(), Value::Int(42));
}

#[test]
fn session_owns_imported_modules_after_analysis_is_dropped() {
    let session = {
        let sources = BTreeMap::from([
            (
                "main.zt".to_string(),
                "lib ::= import \"lib.zt\";\nlib.sum2".to_string(),
            ),
            (
                "lib.zt".to_string(),
                "sum2 :: Int -> Int -> Int = a b => a + b;\n{ sum2 = sum2; }".to_string(),
            ),
        ]);
        let analysis = zutai_semantic::analyze_sources(
            "main.zt",
            &sources,
            zutai_semantic::AnalysisOptions::default(),
        )
        .unwrap();
        TlcSession::from_analysis(&analysis).unwrap()
    };

    let add = session.entry().unwrap();
    assert_eq!(
        session.apply2(add, Value::Int(20), Value::Int(22)).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn session_retains_callable_next_to_erased_type_export() {
    let sources = BTreeMap::from([
        (
            "main.zt".to_string(),
            "module ::= import \"typed.zt\";\nmodule.f".to_string(),
        ),
        (
            "typed.zt".to_string(),
            "T :: type Int;\nf :: Int -> Int = value => value + 1;\n{ T = T; f = f; }".to_string(),
        ),
    ]);
    let analysis = zutai_semantic::analyze_sources(
        "main.zt",
        &sources,
        zutai_semantic::AnalysisOptions::default(),
    )
    .unwrap();
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);

    let session = TlcSession::from_analysis(&analysis).unwrap();
    let function = session.entry().unwrap();
    assert_eq!(
        session.apply(function, Value::Int(41)).unwrap(),
        Value::Int(42)
    );

    // Strict TLC entry points permit a type-free root expression to use a value
    // exported beside an erased Type member in a dependency.
    assert_eq!(
        eval_tlc_with_base(
            "module ::= import \"value_type_members.zt\";\nmodule.defaultPort",
            Some(&imports_dir()),
        )
        .unwrap(),
        Value::Int(8080)
    );
    assert!(matches!(
        crate::eval_tlc_file("T :: type Int;\nT").unwrap_err(),
        EvalError::ReflectionUnsupported(_)
    ));
}

#[test]
fn session_retains_imported_operator_witnesses_across_calls() {
    let analysis = zutai_semantic::analyze_with_base(
        "w ::= import \"witness_eq_int_operator.zt\";\ncompare :: Int -> Int -> Bool = a b => a == b;\ncompare",
        Some(&imports_dir()),
        zutai_semantic::AnalysisOptions::default(),
    );
    let session = TlcSession::from_analysis(&analysis).unwrap();
    let compare = session.entry().unwrap();

    // The imported witness deliberately returns false even for equal Ints.
    assert_eq!(
        session
            .apply2(compare.clone(), Value::Int(1), Value::Int(1))
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        session
            .apply2(compare, Value::Int(2), Value::Int(2))
            .unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn session_materializes_imported_partial_alias_witness() {
    let analysis = zutai_semantic::analyze_with_base(
        "_ ::= import \"hkt_witness_result.zt\";\nFunctor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nResult :: <E, A> type { #ok : { value : A; }; #err : { error : E; }; };\ngood :: Result Text Int = #ok { value = 5; };\nmap (\\x. x + 1) good",
        Some(&imports_dir()),
        zutai_semantic::AnalysisOptions::default(),
    );
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    let session =
        TlcSession::from_analysis(&analysis).expect("partial alias witness should materialize");
    assert_eq!(
        session.entry().expect("imported map should evaluate"),
        run("#ok { value = 5; }")
    );
}

#[test]
fn session_dispatches_imported_collection_list_witness() {
    let analysis = zutai_semantic::analyze(
        "_ ::= import stdlib.collection;\nFunctor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nxs :: List Int = { 1; 2; };\nmap (\\x. x + 1) xs",
    );
    assert!(analysis.is_thir_complete(), "{:?}", analysis.diagnostics);
    assert_eq!(
        TlcSession::from_analysis(&analysis)
            .unwrap()
            .entry()
            .unwrap(),
        run("{ 2; 3; }")
    );
    assert_eq!(
        crate::eval_file(
            "_ ::= import stdlib.collection;\nFunctor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nxs :: List Int = { 1; 2; };\nmap (\\x. x + 1) xs",
        )
        .unwrap(),
        run("{ 2; 3; }")
    );
}

#[test]
fn default_eval_dispatches_all_collection_witnesses() {
    let source = r#"
_ ::= import stdlib.collection;
Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }
Foldable :: <F :: Type -> Type> @F { fold :: <A, B> (B -> A -> B) -> B -> F A -> B; }
Result :: <E, A> type { #ok : { value : A; }; #err : { error : E; }; };
listValue :: List Int = { 1; 2; 3; };
optionalValue :: Int? = #some (4);
resultValue :: Result Text Int = #ok { value = 5; };
list ::= fold (\sum value. sum + value) 0 (map (\value. value * 2) listValue);
optional ::= fold (\sum value. sum + value) 0 (map (\value. value + 1) optionalValue);
result ::= fold (\sum value. sum + value) 0 (map (\value. value + 1) resultValue);
{ list; optional; result; }
"#;
    assert_eq!(crate::eval_file(source).unwrap(), run("{ 12; 5; 6; }"));
}

#[test]
fn session_force_thunk_and_deep_force_are_available_to_hosts() {
    let analysis = zutai_semantic::analyze(
        "make :: Int -> { answer : Int; } = x => { answer = x + 1; };\nmake",
    );
    let session = TlcSession::from_analysis(&analysis).unwrap();
    let make = session.entry().unwrap();
    let record = session.apply(make, Value::Int(41)).unwrap();
    let Value::Record(fields) = record.clone() else {
        panic!("expected record, got {record:?}");
    };
    let answer = &fields[0].1;
    assert_eq!(session.force_thunk(answer).unwrap(), Value::Int(42));
    let Value::Record(forced) = session.force(record).unwrap() else {
        panic!("expected forced record");
    };
    assert_eq!(forced[0].1.peek(), Some(Value::Int(42)));
}

#[derive(Default)]
struct RecordingHandler {
    operations: RefCell<Vec<(String, Value)>>,
}

impl EffectHandler for RecordingHandler {
    fn handle(&self, operation: &str, argument: Value) -> Result<Value, EvalError> {
        self.operations
            .borrow_mut()
            .push((operation.to_string(), argument.clone()));
        match operation {
            "browser.focus" => Ok(Value::Tuple(std::rc::Rc::from([]))),
            "io.print" => Ok(argument),
            _ => Err(EvalError::UnhandledEffect(operation.to_string())),
        }
    }
}

#[test]
fn custom_handler_intercepts_and_resumes_residual_effect() {
    let analysis = zutai_semantic::analyze(r#"perform io.print "search""#);
    let session = TlcSession::from_analysis(&analysis).unwrap();
    let handler = RecordingHandler::default();

    assert_eq!(
        session.entry_with_handler(&handler).unwrap(),
        Value::Text("search".into())
    );
    let effects = handler.operations.borrow();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].0, "io.print");
    assert_eq!(effects[0].1, Value::Text("search".into()));
}

#[test]
fn custom_handler_is_used_across_curried_application() {
    let analysis = zutai_semantic::analyze(
        r#"
focusAfter :: Text -> Int -> Int ! { browser.focus : Text -> Unit; }
  = target value => [ perform browser.focus target; value + 1 ];
focusAfter
"#,
    );
    let session = TlcSession::from_analysis(&analysis).unwrap();
    let function = session.entry().unwrap();
    let handler = RecordingHandler::default();
    let value = session
        .apply2_with_handler(
            function,
            Value::Text("name".into()),
            Value::Int(41),
            &handler,
        )
        .unwrap();

    assert_eq!(value, Value::Int(42));
    assert_eq!(handler.operations.borrow().len(), 1);
}
