use super::*;

// ─── format_thir_diagnostic arms ─────────────────────────────────────────────
//
// Each test exercises one arm of `format_thir_diagnostic` in lib.rs by
// feeding a program that passes parse + HIR but fails THIR.

fn type_check_messages(src: &str) -> Vec<String> {
    let err = run_err(src);
    let EvalError::TypeCheckFailed(msgs) = err else {
        panic!("expected TypeCheckFailed for:\n{src}\ngot: {err:?}");
    };
    msgs
}

fn assert_type_check_failed(src: &str) {
    let _ = type_check_messages(src);
}

#[test]
fn diagnostic_polish_eval_record_mismatch_message() {
    let src = r#"
S :: type { x : Int; y : Text; };
T :: type { x : Int; };
f :: S -> Int
  = _ => 0;
t :: T = { x = 1; };
f t
"#;
    let msgs = type_check_messages(src);
    assert!(
        msgs.iter()
            .any(|m| { m == "type mismatch: expected { x : Int; y : Text; }, found { x : Int; }" })
    );
}

#[test]
fn diagnostic_polish_eval_row_tail_overlap_message() {
    let src = r#"
Base :: type { host : Text; port : Int; };
Bad :: type { host : Int; ...Base; };
Bad
"#;
    let msgs = type_check_messages(src);
    assert!(msgs.iter().any(|m| {
        m == "record row tail `...Base` overlaps explicit field `host`: existing `host : Int`, incoming `host : Text`"
    }));
}

#[test]
fn thir_diag_invalid_binary_operands() {
    // `true + 1` — booleans don't support `+`
    assert_type_check_failed("true + 1");
}

#[test]
fn thir_diag_empty_list_needs_type() {
    // `{;}` with no type context
    assert_type_check_failed("{;}");
}

#[test]
fn thir_diag_expected_function() {
    // Applying 1 (an Int) as a function
    assert_type_check_failed("1 2");
}

#[test]
fn thir_diag_unsupported_feature_empty_match() {
    // Empty match arms — must be on a separate line so `{}` isn't parsed as a
    // record argument applied to the scrutinee.
    assert_type_check_failed("match 1\n{}");
}

#[test]
fn thir_diag_non_exhaustive_match() {
    // Integer match with only one literal arm — not exhaustive
    assert_type_check_failed("match 1 {\n  | 1 => 2;\n}");
}

#[test]
fn thir_diag_unreachable_match_arm() {
    // Wildcard makes the next arm unreachable
    assert_type_check_failed("match 1 {\n  | _ => 1;\n  | 1 => 2;\n}");
}

#[test]
fn thir_diag_match_arm_pattern_count_mismatch() {
    // Match arm with 2 patterns instead of 1
    assert_type_check_failed("match 1 {\n  | a b => 1;\n}");
}

#[test]
fn thir_diag_function_clause_arity_mismatch() {
    // Function typed Int -> Int but clause has 2 patterns
    assert_type_check_failed("f :: Int -> Int\n  = x y => x;\nf 1");
}

#[test]
fn thir_diag_tuple_arity_mismatch() {
    // Type says (Int, Int) but value has 3 elements
    assert_type_check_failed("x :: (Int, Int) = (1, 2, 3);\nx");
}

#[test]
fn thir_diag_alias_cycle() {
    // Mutually cyclic type aliases
    let src = "
A :: type B;
B :: type A;
x :: A = 1;
x
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_type_constructor_arity_mismatch() {
    // Pair needs 2 type args but only 1 given
    let src = "
Pair :: <A, B> type { first : A; second : B; };
x :: Pair Text = x;
x
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_invalid_type_expression() {
    // Number literal `1` in type annotation position → ExprEscape → InvalidTypeExpression
    assert_type_check_failed("x :: 1 = 1;\nx");
}

#[test]
fn thir_diag_unknown_field() {
    // Accessing field `y` that doesn't exist on `{ x : Int }`
    assert_type_check_failed("{ x = 1; }.y");
}

#[test]
fn thir_diag_missing_record_field() {
    let src = "
Server :: type { host : Text; port : Int; };
s :: Server = { host = \"localhost\"; };
s
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_unexpected_record_field() {
    let src = "
Server :: type { host : Text; };
s :: Server = { host = \"localhost\"; port = 8080; };
s
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_witness_field_type_mismatch() {
    // Witness field `(==)` should be `Int -> Int -> Bool` but is given `42` (Int)
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = 42; }
1 == 1
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_missing_witness_field() {
    // Witness for Eq @Int is missing the required `(==)` field
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: {}
1 == 1
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_unknown_witness_field() {
    // Witness has field `extra` not declared in constraint
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \\a b. true; extra = 42; }
1 == 1
";
    assert_type_check_failed(src);
}

#[test]
fn thir_diag_conflicting_witness() {
    // Two witnesses for Eq @Int
    let src = "
Eq :: <A> @A { (==) :: A -> A -> Bool; }
Eq @Int :: { (==) = \\a b. true; }
Eq @Int :: { (==) = \\a b. false; }
1 == 1
";
    assert_type_check_failed(src);
}
// ─── algebraic effects ────────────────────────────────────────────────────────

#[test]
fn handled_warn_resume_runs_rest_of_computation() {
    assert_eq!(
        run(r#"
result ::= handle [ perform warn "diag"; "ok" ] with { warn = \d. resume (); };
result
"#,),
        Value::Text("ok".into())
    );
}

#[test]
fn handled_effect_in_block_local_initializer_resumes() {
    assert_eq!(
        run(r#"
compute :: Text -> Int ! { query : Text -> Int; }
  = _ => [ x := perform query "q"; x + 1 ];
handle compute "go" with { query = \u. resume 41; }
"#,),
        Value::Int(42)
    );
}

#[test]
fn repeated_handled_effects_resume_with_distinct_handler_bindings() {
    assert_eq!(
        run(r#"
result ::= handle (perform query 1) + (perform query 2) with { query = \n. resume n; };
result
"#,),
        Value::Int(3)
    );
}

#[test]
fn handled_effect_resume_value_can_reference_param_inside_tuple() {
    assert_eq!(
        run(r#"
result ::= handle perform query 1 with { query = \n. resume (n, n); };
result
"#,)
        .to_string(),
        "(1, 1)"
    );
}
#[test]
fn handled_fail_can_return_without_resuming() {
    assert_eq!(
        run(r#"
result ::= handle [ perform fail "bad"; "unreachable" ] with { fail = \e. "fallback"; };
result
"#,),
        Value::Text("fallback".into())
    );
}

#[test]
fn top_level_io_print_is_handled_by_host_boundary() {
    assert_eq!(
        run(r#"perform io.print "hello""#),
        Value::Text("hello".into())
    );
}

#[test]
fn top_level_standard_host_effect_is_handled_by_host_boundary() {
    let path = std::env::temp_dir().join("zutai_eval_host_fs_read.txt");
    std::fs::write(&path, "host-read").unwrap();
    let path = path
        .to_str()
        .unwrap()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let src = format!(
        r#"
readFile :: Path -> Text ! {{ fs.read : Path -> Text; }}
  = path => perform fs.read path;
readFile "{path}"
"#
    );
    assert_eq!(run(&src), Value::Text("host-read".into()));
}

fn zt_string_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn expect_some_text(value: Value, expected: &str) {
    let Value::TaggedValue { tag, payload } = value else {
        panic!("expected #some tagged value, got {value:?}");
    };
    assert_eq!(tag.as_ref(), "some");
    let Some((_, thunk)) = payload.first() else {
        panic!("expected #some payload");
    };
    assert_eq!(thunk.peek(), Some(Value::Text(expected.into())));
}

fn expect_none(value: Value) {
    assert_eq!(value, Value::Atom("none".into()));
}

#[test]
fn stdlib_fs_imports_effectful_handle_helpers() {
    assert_eq!(run("fs ::= import stdlib.fs;\n1"), Value::Int(1));
}

#[test]
fn scoped_fs_handles_write_read_lines_and_eof() {
    let path = std::env::temp_dir().join("zutai_eval_scoped_fs_roundtrip.txt");
    let path = zt_string_literal(path.to_str().unwrap());
    let src = format!(
        r#"
RoundTrip :: type {{ first : Text?; second : Text?; eof : Text?; }};
WriteTextRequest :: type {{ contents : Text; writer : Writer; }};
writeTextRequest :: Writer -> Text -> WriteTextRequest
  = writer contents => {{ contents = contents; writer = writer; }};
roundTrip :: Path -> RoundTrip ! {{ fs.openWrite : Path -> Writer; fs.writeText : WriteTextRequest -> Unit; fs.closeWrite : Writer -> Unit; fs.openRead : Path -> Reader; fs.readLine : Reader -> Text?; fs.closeRead : Reader -> Unit; }}
  = path => [
    writer := perform fs.openWrite path;
    wrote := perform fs.writeText (writeTextRequest writer "alpha\nbeta\n");
    closedWriter := perform fs.closeWrite writer;
    reader := perform fs.openRead path;
    first := perform fs.readLine reader;
    second := perform fs.readLine reader;
    eof := perform fs.readLine reader;
    closedReader := perform fs.closeRead reader;
    {{ first = first; second = second; eof = eof; }}
  ];
roundTrip "{path}"
"#
    );
    let value = run(&src);
    expect_some_text(record_field_value(&value, "first"), "alpha");
    expect_some_text(record_field_value(&value, "second"), "beta");
    expect_none(record_field_value(&value, "eof"));
}

#[test]
fn scoped_fs_handles_double_close_is_unit() {
    let path = std::env::temp_dir().join("zutai_eval_scoped_fs_double_close.txt");
    let path = zt_string_literal(path.to_str().unwrap());
    let src = format!(
        r#"
closeTwice :: Path -> Text ! {{ fs.openWrite : Path -> Writer; fs.closeWrite : Writer -> Unit; }}
  = path => [
    writer := perform fs.openWrite path;
    first := perform fs.closeWrite writer;
    second := perform fs.closeWrite writer;
    "ok"
  ];
closeTwice "{path}"
"#
    );
    assert_eq!(run(&src), Value::Text("ok".into()));
}

#[test]
fn scoped_fs_handles_fail_after_close() {
    let path = std::env::temp_dir().join("zutai_eval_scoped_fs_after_close.txt");
    let path = zt_string_literal(path.to_str().unwrap());
    let src = format!(
        r#"
WriteTextRequest :: type {{ contents : Text; writer : Writer; }};
writeTextRequest :: Writer -> Text -> WriteTextRequest
  = writer contents => {{ contents = contents; writer = writer; }};
bad :: Path -> Unit ! {{ fs.openWrite : Path -> Writer; fs.writeText : WriteTextRequest -> Unit; fs.closeWrite : Writer -> Unit; }}
  = path => [
    writer := perform fs.openWrite path;
    closed := perform fs.closeWrite writer;
    perform fs.writeText (writeTextRequest writer "late")
  ];
bad "{path}"
"#
    );
    match run_err(&src) {
        EvalError::EffectfulNotExecutable(msg) => assert!(msg.contains("closed"), "{msg}"),
        other => panic!("expected after-close runtime error, got {other:?}"),
    }
}

#[test]
fn whole_file_fs_read_write_compatibility_still_works() {
    let path = std::env::temp_dir().join("zutai_eval_fs_compat.txt");
    let path = zt_string_literal(path.to_str().unwrap());
    let src = format!(
        r#"
WriteRequest :: type {{ contents : Text; path : Path; }};
writeRequest :: Path -> Text -> WriteRequest
  = path contents => {{ contents = contents; path = path; }};
compat :: Path -> Text ! {{ fs.write : WriteRequest -> Unit; fs.read : Path -> Text; }}
  = path => [
    wrote := perform fs.write (writeRequest path "compat");
    perform fs.read path
  ];
compat "{path}"
"#
    );
    assert_eq!(run(&src), Value::Text("compat".into()));
}

#[test]
fn top_level_net_http_echo_effects_are_handled_by_host_boundary() {
    let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let src = format!(
        r#"
t ::= import stdlib.text;
serveOnce :: Int -> Text ! {{ net.listen : Int -> Int; net.accept : Int -> Int; net.read : Int -> Text; net.write : Text -> Unit; net.close : Int -> Unit; }}
  = port => [
    listener := perform net.listen port;
    conn := perform net.accept listener;
    requestLine := perform net.read conn;
    response := t.join "" {{
      "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n";
      requestLine;
    }};
    written := perform net.write response;
    closed := perform net.close conn;
    requestLine
  ];
serveOnce {port}
"#
    );
    let server = std::thread::spawn(move || {
        assert_eq!(run(&src), Value::Text("GET /echo HTTP/1.1".into()));
    });

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut stream = loop {
        match std::net::TcpStream::connect(addr) {
            Ok(stream) => break stream,
            Err(err) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
                let _ = err;
            }
            Err(err) => panic!("failed to connect to Zutai net.listen on {addr}: {err}"),
        }
    };

    use std::io::{Read, Write};
    stream
        .write_all(b"GET /echo HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut echoed = String::new();
    stream.read_to_string(&mut echoed).unwrap();

    assert_eq!(
        echoed,
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nGET /echo HTTP/1.1"
    );
    server.join().unwrap();
}

#[test]
fn top_level_dynamic_zti_load_returns_data_envelope() {
    let path = std::env::temp_dir().join("zutai_eval_dynamic_load.zti");
    std::fs::write(
        &path,
        r#"{ host = "localhost"; port = 8080; flags = [true; #fast;]; }"#,
    )
    .unwrap();
    let path = path
        .to_str()
        .unwrap()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let src = format!(
        r#"
match loadZti "{path}" {{
  | #record {{ fields = fields; }} => match listHead fields {{
      | {{ name = name; value = #text {{ value = value; }}; }} => name == "host" && value == "localhost";
      | _ => false;
    }};
  | _ => false;
}}
"#
    );
    assert_eq!(run(&src), Value::Bool(true));
}

#[test]
fn top_level_dynamic_zt_load_returns_data_envelope() {
    let path = std::env::temp_dir().join("zutai_eval_dynamic_load.zt");
    std::fs::write(&path, r#"{ mode = #prod; port = 8000 + 80; }"#).unwrap();
    let path = path
        .to_str()
        .unwrap()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let src = format!(
        r#"
match loadZt "{path}" {{
  | #record {{ fields = fields; }} => match listHead fields {{
      | {{ name = name; value = #atom {{ value = value; }}; }} => name == "mode" && value == "prod";
      | _ => false;
    }};
  | _ => false;
}}
"#
    );
    assert_eq!(run(&src), Value::Bool(true));
}

#[test]
fn source_handler_can_make_standard_host_effect_pure() {
    assert_eq!(
        run(r#"
result ::= handle [ perform fs.read "ignored" ] with { fs.read = \path. "mock"; };
result
"#),
        Value::Text("mock".into())
    );
}

#[test]
fn source_handler_intercepts_repointed_print_builtin() {
    assert_eq!(
        run(r#"
result ::= handle print "x" with { io.print = \text. "handled"; };
result
"#,),
        Value::Text("handled".into())
    );
}

#[test]
fn non_tail_resume_reenters_suspended_expression() {
    assert_eq!(
        run(r#"
compute :: Text -> Int ! { query : Text -> Int; }
  = _ => (perform query "question") + 1;
result ::= handle compute "go" with { query = \u. resume 41; };
result
"#,),
        Value::Int(42)
    );
}

#[test]
fn forwarded_effect_reaches_outer_handler() {
    assert_eq!(
        run(r#"
result ::= handle (handle [ perform fail "bad"; "unreachable" ] with { fail = \e. [ perform log e; "fallback" ]; }) with { log = \msg. resume (); };
result
"#,),
        Value::Text("fallback".into())
    );
}

#[test]
fn value_clause_runs_only_on_normal_completion() {
    assert_eq!(
        run(r#"
normal ::= handle "ok" with { value = \v. "done"; };
normal
"#,),
        Value::Text("done".into())
    );
    assert_eq!(
        run(r#"
abort ::= handle perform fail "bad" with { value = \v. "done"; fail = \e. "fallback"; };
abort
"#,),
        Value::Text("fallback".into())
    );
}

#[test]
fn row_polymorphic_effect_signature_lowers_through_tlc() {
    // The open-effect-row foundation: a signature with an effect-row variable
    // `...e` lowers cleanly through TLC and evaluates. This pins the TLC
    // `collect_sig_row_params` Effect arm — the row-variable param must be
    // quantified with row kind (not ground), matching the THIR collector.
    let src = "forward :: <e> (Unit -> Int ! { ...e; }) -> Unit -> Int ! { ...e; }\n  = f => f;\nforward\n";
    assert_eq!(run(src).to_string(), "<function/1>");
}

#[test]
fn call_site_effect_row_inference_pure_and_effectful_args() {
    // Call-site effect-row inference: applying a row-polymorphic function to an
    // argument with a concrete effect row solves the instantiated open tail `...e`.
    // A pure (explicitly-closed) thunk solves it to the empty row; an effectful
    // thunk threads its op through to the result where the handler discharges it.
    let forward =
        "forward :: <e> (Unit -> Int ! { ...e; }) -> Unit -> Int ! { ...e; }\n  = f => f;\n";
    assert_eq!(
        run(&format!(
            "{forward}g :: Unit -> Int ! {{}} = \\_. 9;\n(forward g) ()\n"
        ))
        .to_string(),
        "9"
    );
    assert_eq!(
        run(&format!(
            "{forward}handle (forward (\\_. perform tick ()) ()) with {{ tick = \\_. resume 5; }}\n"
        ))
        .to_string(),
        "5"
    );
}

#[test]
fn ambient_streameff_effectful_generator_runs_under_handler() {
    // The ergonomic effectful-stream type: the ambient prelude `StreamEff A e`
    // alias names the supported V3-G4 idiom. A `stream { yield perform … }`
    // checks against `StreamEff Int { tick }` and, consumed strictly under a
    // granting handler, threads `tick` to the handler — matching the raw-cell-type
    // form (`effectful_generator_runs_under_granted_handler`) but with a named type.
    let src = "sumEff :: StreamEff Int { tick : Unit -> Int; } -> Int ! { tick : Unit -> Int; }\n  = s => match s () {\n    | #nil => 0;\n    | #cons { head = h; tail = t; } => h + sumEff t;\n  };\nhandle (sumEff (stream { yield perform tick (); yield perform tick (); })) with {\n  tick = \\_. resume 5;\n}\n";
    assert_eq!(run(src), Value::Int(10));
}

#[test]
fn ambient_streameff_pure_arg_to_row_polymorphic_consumer() {
    // `StreamEff A {}` is exactly `Stream A`: a pure stream value flows into a
    // consumer polymorphic over the effect row `...e`. This is the case the
    // flexible effect-row tail enables — a closed-row stream meeting the
    // instantiated open tail of the consumer's expanded thunk parameter.
    let src = "headOr :: <A, e> A -> StreamEff A e -> A ! { ...e; }\n  = d s => match s () { | #nil => d; | #cons { head = h; tail = t; } => h; };\npureS :: StreamEff Int {} = \\_. #cons { head = 7; tail = \\_. #nil; };\nheadOr 0 pureS\n";
    assert_eq!(run(src), Value::Int(7));
}

#[test]
fn finally_runs_on_normal_completion() {
    // V3-G4 finalization: a `finally` teardown runs when the handled computation
    // completes normally. The teardown performs `mark`, handled by the enclosing
    // handler which aborts to a sentinel — so observing the sentinel proves the
    // teardown ran (the un-finalized result would be the inner value "inner").
    assert_eq!(
        run(r#"
result ::= handle (handle "inner" with { finally = perform mark (); }) with { mark = \_. "finalized"; };
result
"#,),
        Value::Text("finalized".into())
    );
    // The teardown's own result is discarded — the handle still yields its value.
    assert_eq!(
        run(r#"
passthrough ::= handle "body" with { finally = "cleanup"; };
passthrough
"#,),
        Value::Text("body".into())
    );
}

#[test]
fn finally_runs_when_handler_aborts() {
    // The teardown fires on the *abort* path too: the inner `fail` handler returns
    // without resuming (discarding the continuation), yet `finally` still runs.
    assert_eq!(
        run(r#"
result ::= handle (handle perform fail "x" with { fail = \e. "fallback"; finally = perform mark (); }) with { mark = \_. "finalized"; };
result
"#,),
        Value::Text("finalized".into())
    );
}

#[test]
fn finally_runs_after_early_stream_consumption() {
    // The resource-finalization scenario: an effectful generator is consumed
    // *partially* (`take2` forces only the first two cells of a three-element
    // stream), yet the granting handler's `finally` teardown still fires. `close`
    // escapes to the enclosing handler, which aborts to `0 - 1`; observing it
    // proves the resource was finalized despite the early stop. `tick` resumes 5
    // twice, so the un-finalized result would be 10.
    let src = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
take2 :: (Unit -> Cell) -> Int ! { tick : Unit -> Int; }
  = s => match s () {
    | #nil => 0;
    | #cons { head = h; tail = t; } => h + (match t () {
        | #nil => 0;
        | #cons { head = h2; tail = u; } => h2;
      });
  };
result ::= handle (
  handle (take2 (stream { yield perform tick (); yield perform tick (); yield perform tick (); })) with {
    tick = \_. resume 5;
    finally = perform close ();
  }
) with {
  close = \_. 0 - 1;
};
result
"#;
    assert_eq!(run(src), Value::Int(-1));
}

#[test]
fn cancellation_stops_generator_via_aborting_granting_handler() {
    // V3-G4 cancellation: a consumer signals mid-stream cancellation by
    // performing a `stop` operation whose granting-handler clause *aborts*
    // (returns without `resume`). The generator stops mid-stream — the third
    // `tick` never fires — and the consumer's accumulated result rides out on
    // `stop`'s argument. (Were the generator run to exhaustion, the result
    // would be 15; cancellation after the second element yields 10.)
    let src = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
foldUntil :: Int -> (Unit -> Cell) -> Int ! { tick : Unit -> Int; stop : Int -> Int; }
  = acc s => match s () {
    | #nil => acc;
    | #cons { head = h; tail = t; } => (if acc + h > 7 then perform stop (acc + h) else foldUntil (acc + h) t);
  };
handle (foldUntil 0 (stream { yield perform tick (); yield perform tick (); yield perform tick (); })) with {
  tick = \_. resume 5;
  stop = \r. r;
}
"#;
    assert_eq!(run(src), Value::Int(10));
}

#[test]
fn cancellation_runs_finally_on_the_granting_handler() {
    // The supported cancellation idiom co-locates the aborting `stop` clause
    // with the `finally` teardown on the *granting* handler. Cancelling the
    // generator mid-stream still fires `finally`: `close` escapes to the
    // enclosing handler, which aborts to `0 - 1`. Observing it proves the
    // resource was finalized on cancellation (the un-finalized result is 10).
    let src = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
foldUntil :: Int -> (Unit -> Cell) -> Int ! { tick : Unit -> Int; stop : Int -> Int; }
  = acc s => match s () {
    | #nil => acc;
    | #cons { head = h; tail = t; } => (if acc + h > 7 then perform stop (acc + h) else foldUntil (acc + h) t);
  };
handle (
  handle (foldUntil 0 (stream { yield perform tick (); yield perform tick (); yield perform tick (); })) with {
    tick = \_. resume 5;
    stop = \r. r;
    finally = perform close ();
  }
) with { close = \_. 0 - 1; }
"#;
    assert_eq!(run(src), Value::Int(-1));
}

#[test]
fn cancellation_across_a_finalizer_boundary_unwinds_inner_finally() {
    // Cross-boundary cancellation now unwinds the inner finalizer instead of
    // refusing. `stop` is handled by an outer aborting clause, so the suspended
    // continuation carrying the inner `finally` is discarded; the interpreter
    // runs that finalizer explicitly before completing the abort. `close` is
    // handled by the same outer handler and aborts to 999, reusing the existing
    // finalizer semantics where a finalizer's own handled abort determines the
    // result.
    let src = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
foldUntil :: Int -> (Unit -> Cell) -> Int ! { tick : Unit -> Int; stop : Int -> Int; }
  = acc s => match s () {
    | #nil => acc;
    | #cons { head = h; tail = t; } => (if acc + h > 7 then perform stop (acc + h) else foldUntil (acc + h) t);
  };
handle (
  handle (foldUntil 0 (stream { yield perform tick (); yield perform tick (); yield perform tick (); })) with {
    tick = \_. resume 5;
    finally = perform close ();
  }
) with {
  stop = \r. r;
  close = \_. 999;
}
"#;
    assert_eq!(run(src), Value::Int(999));
}

#[test]
fn cancellation_across_stacked_finalizers_unwinds_inner_to_outer() {
    // The inner finalizer runs first. Its `close` handler resumes, so the abort
    // value (10) continues to the outer finalizer; the outer `mark` handler
    // aborts to 999 and determines the final result.
    let src = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
foldUntil :: Int -> (Unit -> Cell) -> Int ! { tick : Unit -> Int; stop : Int -> Int; }
  = acc s => match s () {
    | #nil => acc;
    | #cons { head = h; tail = t; } => (if acc + h > 7 then perform stop (acc + h) else foldUntil (acc + h) t);
  };
handle (
  handle (
    handle (foldUntil 0 (stream { yield perform tick (); yield perform tick (); })) with {
      tick = \_. resume 5;
      finally = perform close ();
    }
  ) with {
    finally = perform mark ();
  }
) with {
  stop = \r. r;
  close = \_. resume ();
  mark = \_. 999;
}
"#;
    assert_eq!(run(src), Value::Int(999));
}

#[test]
fn effect_resumed_across_a_finalizer_boundary_still_runs() {
    // The refusal is abort-only: an effect that escapes an inner
    // `finally`-bearing handle and is *resumed* (not aborted) by an outer
    // handler runs unchanged. `ask` escapes the inner handle (which grants only
    // `finally`), the outer handler resumes it twice with 5, the inner handle
    // settles, and its `finally` `close` runs (resumed by the outer handler) —
    // the marked-but-resumed finalizer count is harmless.
    let src = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
sumEff :: (Unit -> Cell) -> Int ! { ask : Unit -> Int; }
  = s => match s () { | #nil => 0; | #cons { head = h; tail = t; } => h + sumEff t; };
handle (
  handle (sumEff (stream { yield perform ask (); yield perform ask (); })) with {
    finally = perform close ();
  }
) with {
  ask = \_. resume 5;
  close = \_. resume ();
}
"#;
    assert_eq!(run(src), Value::Int(10));
}

#[test]
fn effectful_stream_generator_against_pure_stream_alias_is_rejected() {
    // Annotating an effectful generator as the *pure* `Stream A` alias
    // (`Stream A = Unit -> StreamCell A`) is rejected: the `yield perform …`
    // defers a `fs.read` effect into the cell thunk, which cannot satisfy the
    // pure thunk the alias demands. The effect is refused, never silently
    // dropped. (The *supported* V3-G4 form threads the effect through the
    // consumer's row and consumes the generator under a handler — see
    // `effectful_generator_runs_under_granted_handler`.)
    let err = run_err(
        "load :: FsRead -> Stream Text ! { fs.read : Path -> Text; }\n  = fs => stream { yield perform fs.read \"Cargo.toml\"; };\n1\n",
    );
    let EvalError::TypeCheckFailed(messages) = err else {
        panic!("expected TypeCheckFailed, got {err:?}");
    };
    assert!(
        messages.iter().any(|msg| msg.contains("fs.read")),
        "expected fs.read effect diagnostic, got {messages:?}"
    );
}

#[test]
fn effectful_generator_runs_under_granted_handler() {
    // V3-G4 (reference-interpreter level): a generator that performs an effect in
    // its cells runs when consumed *strictly* under a granting handler. The effect
    // rides on the consumer's row (a `perform` in a lazy cell field is deferred to
    // whoever forces it), so `sumEff` — which forces each head with `h + …` inside
    // the `handle` — fires `tick` in the handler's dynamic extent. Two `perform
    // tick`s, each resumed with 5, sum to 10.
    let src = "Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };\nsumEff :: (Unit -> Cell) -> Int ! { tick : Unit -> Int; }\n  = s => match s () {\n    | #nil => 0;\n    | #cons { head = h; tail = t; } => h + sumEff t;\n  };\nhandle (sumEff (stream { yield perform tick (); yield perform tick (); })) with {\n  tick = \\_. resume 5;\n}\n";
    assert_eq!(run(src), Value::Int(10));
}

#[test]
fn effectful_generator_without_a_handler_is_rejected() {
    // The dual of the above: the same effectful generator with no handler (and a
    // pure consumer) is refused — `tick` escapes the (empty) ambient effect row.
    // A refused program is the safe direction; the effect is never silently lost.
    let err = run_err(
        "Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };\nsumEff :: (Unit -> Cell) -> Int\n  = s => match s () { | #nil => 0; | #cons { head = h; tail = t; } => h + sumEff t; };\nsumEff (stream { yield perform tick (); })\n",
    );
    let EvalError::TypeCheckFailed(messages) = err else {
        panic!("expected TypeCheckFailed, got {err:?}");
    };
    assert!(
        messages.iter().any(|msg| msg.contains("tick")),
        "expected an unhandled `tick` effect diagnostic, got {messages:?}"
    );
}

#[test]
fn stream_generator_rejects_unsupported_residual_host_effects() {
    let err = run_err(r#"stream { yield perform net.next (); }"#);
    let EvalError::TypeCheckFailed(messages) = err else {
        panic!("expected TypeCheckFailed, got {err:?}");
    };
    assert!(
        messages.iter().any(|msg| msg.contains("net.next")),
        "expected net.next effect diagnostic, got {messages:?}"
    );
}

// ─── prelude `print` effect binding ───────────────────────────────────────────

#[test]
fn print_returns_its_argument() {
    // `print :: Text -> Text ! { io.print : Text -> Text; }`; the host run
    // boundary handles `io.print` and resumes with the printed text.
    assert_eq!(run(r#"print "hello""#), Value::Text("hello".into()));
}

#[test]
fn print_via_forward_pipeline() {
    // `"x" |> print` desugars to `print "x"`.
    assert_eq!(run(r#""piped" |> print"#), Value::Text("piped".into()));
}

#[test]
fn print_in_list_returns_all_elements() {
    // force_deep forces every element, so each `print` fires and the list value
    // is the list of returned texts.
    match run(r#"{print "a"; print "b"; print "c";}"#) {
        Value::List(items) => {
            let texts: Vec<_> = items.iter().filter_map(|t| t.peek()).collect();
            assert_eq!(
                texts,
                vec![
                    Value::Text("a".into()),
                    Value::Text("b".into()),
                    Value::Text("c".into()),
                ]
            );
        }
        other => panic!("expected List, got {other:?}"),
    }
}

#[test]
fn print_result_is_usable_text() {
    // The returned text composes with ordinary text operations.
    assert_eq!(run(r#"print "z" < "a""#), Value::Bool(false));
}

#[test]
fn print_lambda_param_shadows_builtin() {
    // A lambda parameter named `print` shadows the prelude builtin; applying it
    // is ordinary variable application, not the builtin effect.
    let src = "apply :: (Text -> Text) -> Text -> Text\n  = f x => f x;\napply (\\print. print) \"shadowed\"";
    assert_eq!(run(src), Value::Text("shadowed".into()));
}

#[test]
fn print_unapplied_is_a_function_value() {
    // Referencing `print` without applying it yields the runtime-dispatching
    // function value used by both direct and higher-order calls.
    assert!(matches!(run("print"), Value::TlcClosure(_)));
}

#[test]
fn redefining_print_at_top_level_is_rejected() {
    // `print` is reserved in the root scope, so a top-level redefinition is a
    // DuplicateBinding error and the program refuses to run.
    let err = run_err("print ::= 5;\nprint");
    assert!(matches!(err, EvalError::NotRunnable(_)), "got {err:?}");
}

#[test]
fn occurs_check_rejects_infinite_self_application() {
    // `(\x. x x) (\x. x x)` requires an infinite type. The occurs check must
    // reject it at THIR (clean diagnostic) rather than silently accepting and
    // overflowing the stack at runtime.
    let msgs = type_check_messages("(\\x. x x) (\\x. x x)");
    assert!(
        msgs.iter().any(|m| m.contains("infinite type")),
        "expected an infinite-type diagnostic, got: {msgs:?}"
    );
}
