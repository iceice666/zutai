use assert_cmd::Command;
use predicates::prelude::*;
use std::process::Command as StdCommand;

fn cli() -> Command {
    Command::cargo_bin("zutai-cli").unwrap()
}

/// Write a temp file and return its path. Uses a fixed name so tests are
/// hermetic across runs; each test uses a distinct name to avoid conflicts.
fn write_tmp(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path.to_str().unwrap().to_string()
}

fn compile_stdout(name: &str, content: &str) -> String {
    let path = write_tmp(name, content);
    let output = cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("compile output should be UTF-8")
}

fn compile_bin_stdout(name: &str, content: &str) -> String {
    let path = write_tmp(&format!("{name}.zt"), content);
    let out = write_tmp(name, "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn llvm_call_uses_slot(llvm: &str, callee: &str, slot: usize) -> bool {
    let suffix = format!(", i64 {slot})");
    llvm.lines()
        .any(|line| line.contains(callee) && line.trim_end().ends_with(&suffix))
}

// ─── No-args / bad-args ───────────────────────────────────────────────────────

#[test]
fn no_args_shows_usage() {
    cli()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn unknown_args_shows_usage() {
    cli()
        .arg("--unknown")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage:"));
}

// ─── `run` subcommand ─────────────────────────────────────────────────────────

#[test]
fn run_valid_zt_file_prints_result() {
    let path = write_tmp("cli_test_valid.zt", "1 + 2\n");
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("3"));
}

#[test]
fn run_posit_file_prints_posit_result() {
    let path = write_tmp("cli_test_posit_run.zt", "1p32 + 2p32\n");
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("3p32"));
}

#[test]
fn run_deep_recursion_does_not_overflow_stack() {
    // Regression: the tree-walking interpreter runs on a large worker stack so
    // deep (but finite) recursion completes instead of aborting the process.
    // `count 5000` overflows the default ~8 MiB main-thread stack.
    let src = "count :: Int -> Int\n  = 0 => 0;\n  = n => 1 + count (n - 1);\ncount 5000\n";
    let path = write_tmp("cli_test_deep_recursion.zt", src);
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("5000"));
}

#[test]
fn run_zt_parse_error_exits_nonzero() {
    // `[1; 2]` is an invalid Zutai expression (list items need semicolons
    // but the outer `[` is parsed as union type syntax).
    let path = write_tmp("cli_test_parse_err.zt", "[1; 2]\n");
    cli().arg("run").arg(&path).assert().failure();
}

#[test]
fn run_zt_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_type_err.zt", "x :: Int = \"bad\"\nx\n");
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

#[test]
fn run_missing_file_exits_nonzero() {
    cli()
        .arg("run")
        .arg("/tmp/zutai_does_not_exist_xyz.zt")
        .assert()
        .failure();
}

#[test]
fn run_handled_effect_program_prints_result() {
    let path = write_tmp("cli_test_run_effect.zt", HANDLED_EFFECT_SRC);
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"ok\""));
}

#[test]
fn run_indirect_bounded_constraint_uses_tlc_default() {
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
same :: <A: Eq> A -> A -> Bool
  = x y => eq x y;
wrapper :: Int -> Bool
  = n => same n n;
wrapper 1

"#;
    let path = write_tmp("cli_test_tlc_default_indirect_constraint.zt", src);
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("true"));
}

// ─── `parse` subcommand ───────────────────────────────────────────────────────

#[test]
fn parse_valid_zt_file_prints_ast() {
    let path = write_tmp("cli_test_parse_zt.zt", "1 + 2\n");
    cli()
        .arg("parse")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Parsed"));
}

#[test]
fn parse_valid_zti_file_prints_ast() {
    let path = write_tmp("cli_test_parse_valid.zti", "{ x = 1; y = 2; }\n");
    cli()
        .arg("parse")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Parsed"));
}

#[test]
fn parse_zt_with_parse_errors_exits_nonzero() {
    // `[1; 2]` produces a parse error in .zt files.
    let path = write_tmp("cli_test_parse_parse_err.zt", "[1; 2]\n");
    cli().arg("parse").arg(&path).assert().failure();
}

#[test]
fn parse_with_unsupported_extension_exits_nonzero() {
    let path = write_tmp("cli_test_bad_ext.xyz", "hello");
    cli()
        .arg("parse")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unsupported"));
}

#[test]
fn parse_missing_file_exits_nonzero() {
    cli()
        .arg("parse")
        .arg("/tmp/zutai_no_such_file.zt")
        .assert()
        .failure();
}

// ─── bare path routing ────────────────────────────────────────────────────────

#[test]
fn bare_zt_path_runs_file() {
    let path = write_tmp("cli_test_bare.zt", "42\n");
    cli()
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("42"));
}

#[test]
fn bare_zti_path_parses_file() {
    let path = write_tmp("cli_test_bare.zti", "{ key = \"value\"; }\n");
    cli()
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Parsed"));
}

#[test]
fn bare_unknown_extension_exits_nonzero() {
    let path = write_tmp("cli_test_bare_bad.txt", "anything");
    cli()
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unsupported"));
}

// ─── REPL ─────────────────────────────────────────────────────────────────────

#[test]
fn repl_quits_on_quit_command() {
    let mut cmd = cli();
    cmd.arg("repl").write_stdin(":quit\n").assert().success();
}

#[test]
fn repl_evaluates_expression() {
    let mut cmd = cli();
    cmd.arg("repl")
        .write_stdin("1 + 1\n:quit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("2"));
}

#[test]
fn repl_evaluates_posit_expression() {
    let mut cmd = cli();
    cmd.arg("repl")
        .write_stdin("1p64 + 2p64\n:quit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("3p64"));
}

#[test]
fn repl_accepts_declaration_then_expression() {
    let mut cmd = cli();
    cmd.arg("repl")
        .write_stdin("x ::= 42\nx\n:quit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("42"));
}

#[test]
fn repl_reset_clears_bindings() {
    let mut cmd = cli();
    cmd.arg("repl")
        .write_stdin("x ::= 10\n:reset\n:quit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("bindings cleared"));
}

// ─── zt file with import error ────────────────────────────────────────────────

#[test]
fn run_zt_with_import_error_exits_nonzero() {
    // Import a file that does not exist → import error.
    let path = write_tmp(
        "cli_test_import_err.zt",
        "lib :: import \"./does_not_exist.zti\"\n1\n",
    );
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("import error"));
}

#[test]
fn run_imported_value_can_flow_through_print_effect() {
    write_tmp("cli_test_print_import.zti", "{ host = \"127.0.0.1\"; }\n");
    let path = write_tmp(
        "cli_test_print_import.zt",
        "cfg :: import \"./cli_test_print_import.zti\"\nprint cfg.host\n",
    );
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("127.0.0.1"));
}

#[test]
fn run_imported_function_can_flow_through_print_effect() {
    write_tmp(
        "cli_test_func_import.zt",
        "add :: Int -> Int -> Int\n  = a b => a + b;\nadd\n",
    );
    let path = write_tmp(
        "cli_test_func_print_import.zt",
        "add :: import \"./cli_test_func_import.zt\"\n{ print \"using import\"; add 2 3 }\n",
    );
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("using import").and(predicate::str::contains("5")));
}

// ─── parse with type/semantic errors ─────────────────────────────────────────

#[test]
fn parse_zt_with_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_parse_type_err.zt", "x :: Int = \"oops\"\nx\n");
    cli()
        .arg("parse")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "type mismatch: expected Int, found Text",
        ));
}

// ─── `check` subcommand ────────────────────────────────────────────────────────
const EFFECT_SRC: &str = r#"
Config :: type { value : Text; }
ParseError :: type Text
parse :: Text -> Config ! { fail ParseError }
  = text => perform fail text;
parse
"#;

const HANDLED_EFFECT_SRC: &str = r#"
result ::= handle { perform warn "diag"; "ok" } with { warn = \d. resume (); }
result
"#;

#[test]
fn check_valid_zt_file_passes() {
    let path = write_tmp("cli_test_check_valid.zt", "1 + 2\n");
    cli()
        .arg("check")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("check passed"));
}

#[test]
fn check_zt_parse_error_exits_nonzero() {
    let path = write_tmp("cli_test_check_parse_err.zt", "[1; 2]\n");
    cli().arg("check").arg(&path).assert().failure();
}

#[test]
fn check_zt_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_check_type_err.zt", "x :: Int = \"bad\"\nx\n");
    cli().arg("check").arg(&path).assert().failure();
}

#[test]
fn check_renders_human_readable_type_diagnostic() {
    // `check` must render THIR type errors with a human message and source
    // context (like parse errors), not dump the raw `SemanticDiagnostic { .. }`
    // Debug form. The exact human string is absent from the old Debug output.
    let path = write_tmp("cli_test_check_render.zt", "x :: Int = \"bad\"\nx\n");
    cli()
        .arg("check")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "type mismatch: expected Int, found Text",
        ));
}

#[test]
fn check_effect_program_passes() {
    let path = write_tmp("cli_test_check_effect.zt", EFFECT_SRC);
    cli()
        .arg("check")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("check passed"));
}
#[test]
fn check_higher_kinded_constraint_passes() {
    let src = "Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nFunctor @List :: { map = \\f xs. xs; }\nmapTwice :: <F: Functor, A> (A -> A) -> F A -> F A\n  = f xs => map f (map f xs);\n1\n";
    let path = write_tmp("cli_test_check_hkt.zt", src);
    cli()
        .arg("check")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("check passed"));
}

#[test]
fn check_witness_kind_mismatch_exits_nonzero() {
    // `Functor @Int` — `Int : Type` but `Functor` constrains a `Type -> Type`.
    let src = "Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nFunctor @Int :: { map = \\f x. x; }\n1\n";
    let path = write_tmp("cli_test_check_hkt_badkind.zt", src);
    cli().arg("check").arg(&path).assert().failure();
}

// ─── `compile` subcommand ──────────────────────────────────────────────────────

#[test]
fn compile_valid_zt_file_emits_llvm_ir() {
    let path = write_tmp("cli_test_compile_valid.zt", "42\n");
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("define i64 @__entry"))
        .stdout(predicate::str::contains("call void @zutai.show"));
}

#[test]
fn compile_zt_parse_error_exits_nonzero() {
    let path = write_tmp("cli_test_compile_parse_err.zt", "[1; 2]\n");
    cli().arg("compile").arg(&path).assert().failure();
}

#[test]
fn compile_zt_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_compile_type_err.zt", "x :: Int = \"bad\"\nx\n");
    cli().arg("compile").arg(&path).assert().failure();
}

#[test]
fn compile_effect_program_is_rejected_by_residual_effect_gate() {
    let path = write_tmp("cli_test_compile_effect.zt", EFFECT_SRC);
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("effect"));
}

#[test]
fn compile_effect_bin_is_rejected_before_toolchain() {
    let path = write_tmp("cli_test_compile_effect_bin.zt", EFFECT_SRC);
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("effect"));
}

#[test]
fn compile_handled_effect_program_emits_folded_value() {
    let path = write_tmp("cli_test_compile_handled_effect.zt", HANDLED_EFFECT_SRC);
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("define i64 @__entry"))
        .stdout(predicate::str::contains("ok"));
}

#[test]
fn compile_handled_effect_record_round_trips_folded_value() {
    let path = write_tmp(
        "cli_test_compile_handled_effect_record.zt",
        r#"
result ::= handle { perform warn "diag"; { a = 1; b = 2; } } with { warn = \d. resume (); }
result
"#,
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("record_new"));
}

#[test]
fn compile_print_list_round_trips_folded_value() {
    let path = write_tmp(
        "cli_test_compile_print_list.zt",
        r#"[print "a"; print "b";]"#,
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("@zutai.effect.print.0"))
        .stdout(predicate::str::contains("@zutai.effect.print.1"))
        .stdout(predicate::str::contains("list_cons"));
}

#[test]
fn compile_reflection_schema_record_lowers_to_llvm() {
    let llvm = compile_stdout(
        "cli_test_compile_reflection_schema_record.zt",
        "Server :: type { host : Text; port? : Int; }\nschema Server\n",
    );
    assert!(llvm.contains("call void @zutai.show"), "{llvm}");
    assert!(llvm.contains("record_new"), "{llvm}");
    assert!(!llvm.contains("reflection builtins"), "{llvm}");
}

#[test]
fn compile_reflection_schema_record_bin_renders_shape() {
    let out = compile_bin_stdout(
        "cli_test_compile_reflection_schema_record_bin",
        "Server :: type { host : Text; port? : Int; }\nschema Server\n",
    );
    assert!(out.contains("kind = #record"), "{out}");
    assert!(out.contains("name = \"host\""), "{out}");
    assert!(out.contains("type = \"Text\""), "{out}");
    assert!(out.contains("optional = true"), "{out}");
}

#[test]
fn compile_reflection_schema_union_bin_renders_shape() {
    let out = compile_bin_stdout(
        "cli_test_compile_reflection_schema_union_bin",
        r#"Result :: type {
  #done;
  #ok: { value : Text; };
}
schema Result
"#,
    );
    assert!(out.contains("kind = #union"), "{out}");
    assert!(out.contains("name = \"ok\""), "{out}");
    assert!(out.contains("type = \"Text\""), "{out}");
    assert!(out.contains("name = \"done\""), "{out}");
}

#[test]
fn compile_reflection_schema_empty_record_bin_renders_empty_fields() {
    let out = compile_bin_stdout(
        "cli_test_compile_reflection_schema_empty_record_bin",
        "Empty :: type {}\nschema Empty\n",
    );
    assert!(out.contains("kind = #record"), "{out}");
    assert!(out.contains("fields = []"), "{out}");
}

#[test]
fn compile_reflection_with_effectful_code_is_rejected() {
    let path = write_tmp(
        "cli_test_compile_reflection_with_effect.zt",
        "Server :: type { host : Text; }\n_unused ::= schema Server\nprint \"hello\"\n",
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("effectful code"));
}

#[test]
fn compile_reflection_schema_plain_enum_bin_renders_empty_variants() {
    let out = compile_bin_stdout(
        "cli_test_compile_reflection_schema_plain_enum_bin",
        "Color :: type { #red; #green; #blue; }\nschema Color\n",
    );
    assert!(out.contains("kind = #union"), "{out}");
    assert!(out.contains("name = \"red\""), "{out}");
    assert!(out.contains("fields = []"), "{out}");
    assert!(out.contains("name = \"blue\""), "{out}");
}

#[test]
fn compile_reflection_fields_raw_type_result_is_rejected() {
    let path = write_tmp(
        "cli_test_compile_reflection_fields_type.zt",
        "Server :: type { host : Text; }\nfields Server\n",
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("returns Type"));
}

#[test]
fn compile_type_entry_is_rejected_before_backend_lowering() {
    let path = write_tmp("cli_test_compile_type_entry.zt", "type Int\n");
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("returns Type"));
}

#[test]
fn compile_type_alias_value_entry_is_rejected_before_backend_lowering() {
    let path = write_tmp(
        "cli_test_compile_type_alias_entry.zt",
        "MyInt :: type Int\nMyInt\n",
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("returns Type"));
}

#[test]
fn compile_writes_to_output_file() {
    let path = write_tmp("cli_test_compile_out.zt", "1 + 1\n");
    let out = write_tmp("cli_test_compile_out.ll", "");
    cli()
        .arg("compile")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(
        content.contains("define i64 @__entry"),
        "output should contain LLVM IR function definitions"
    );
}

#[test]
fn compile_arithmetic_emits_add() {
    let path = write_tmp("cli_test_compile_arith.zt", "3 + 4\n");
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("add i64"));
}

#[test]
fn compile_function_uses_uniform_closure_abi() {
    let path = write_tmp(
        "cli_test_compile_closure_abi.zt",
        "inc :: Int -> Int\n  = x => x + 1;\ninc 41\n",
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        // Top-level function is a static closure object, applied through its code
        // slot — never a direct or raw-pointer call.
        .stdout(predicate::str::contains("@zutai.closure.inc"))
        .stdout(predicate::str::contains("getelementptr i64"))
        .stdout(predicate::str::contains("call i64 @inc(i64 41)").not())
        .stdout(predicate::str::contains("to i64 (i64)*").not());
}

#[test]
fn compile_capturing_lambda_uses_heap_closure() {
    // `adder n x = x + n` curries to `\n. \x. x + n`; the inner lambda captures
    // `n`, so applying `adder` allocates a one-capture heap closure.
    let path = write_tmp(
        "cli_test_compile_closure_capture.zt",
        "adder n x = x + n\nadder 10 5\n",
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        // (2 + 1 capture) * 8 = 24 bytes; header (1 << 8) | TAG_CLOSURE = 263.
        .stdout(predicate::str::contains("call i64 @zutai.alloc(i64 24)"))
        .stdout(predicate::str::contains("store i64 263"))
        .stdout(predicate::str::contains("__fn").not());
}

#[test]
fn compile_posit32_emits_helper_and_show_runtime() {
    let path = write_tmp("cli_test_compile_posit32.zt", "1p32e3 + 2p32e3\n");
    cli().arg("compile").arg(&path).assert().success().stdout(
        predicate::str::contains("call i32 @zutai.posit32e3.add")
            .and(predicate::str::contains("trunc i64"))
            .and(predicate::str::contains("call void @zutai.show")),
    );
}

#[test]
fn compile_posit64_emits_helper_and_show_runtime() {
    let path = write_tmp("cli_test_compile_posit64.zt", "1p64e5 + 2p64e5\n");
    cli().arg("compile").arg(&path).assert().success().stdout(
        predicate::str::contains("call i64 @zutai.posit64e5.add")
            .and(predicate::str::contains("call void @zutai.show")),
    );
}

#[test]
fn compile_record_result_emits_type_descriptor_and_show() {
    let llvm = compile_stdout(
        "cli_test_compile_descriptor_record.zt",
        "r ::= { host = \"localhost\"; port = 8080; }\nr\n",
    );
    assert!(llvm.contains("@zutai.desc."), "{llvm}");
    assert!(llvm.contains("@zutai.desc.str."), "{llvm}");
    assert!(llvm.contains("call void @zutai.show"), "{llvm}");
    assert!(llvm.contains(" = ptrtoint ptr @zutai.desc."), "{llvm}");
    assert!(!llvm.contains("ptrtoint (ptr @"), "{llvm}");
}

#[test]
fn compile_static_address_ir_uses_pie_safe_forms() {
    let llvm = compile_stdout(
        "cli_test_compile_static_address_pie_safe.zt",
        "{ text = \"hi\"; atom = #prod; }\n",
    );
    assert!(llvm.contains(" = ptrtoint ptr @zutai.text."), "{llvm}");
    assert!(llvm.contains(" = ptrtoint ptr @zutai.atom."), "{llvm}");
    assert!(llvm.contains(" = ptrtoint ptr @zutai.desc."), "{llvm}");
    assert!(!llvm.contains("ptrtoint (ptr @"), "{llvm}");
}

#[test]
fn compile_union_construction_uses_dense_tags() {
    let src = r#"
Shape :: type {
  #circle: { radius: Int; };
  #square: { side: Int; };
}
c :: Shape = #circle { radius = 3; }
s :: Shape = #square { side = 4; }
c
"#;
    let llvm = compile_stdout("cli_test_compile_dense_union_tags.zt", src);
    assert!(
        llvm.contains("call i64 @zutai.variant_new(i64 0,"),
        "{llvm}"
    );
    assert!(
        llvm.contains("call i64 @zutai.variant_new(i64 1,"),
        "{llvm}"
    );
}

#[test]
fn compile_emit_obj_writes_object() {
    let path = write_tmp("cli_test_compile_emit_obj.zt", "42\n");
    let out = write_tmp("cli_test_compile_emit_obj.o", "");
    cli()
        .arg("compile")
        .arg("--emit=obj")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
}

#[test]
fn compile_emit_bin_runs() {
    let path = write_tmp("cli_test_compile_emit_bin.zt", "42\n");
    let out = write_tmp("cli_test_compile_emit_bin", "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "42\n");
}

#[test]
fn compile_emit_bin_record_descriptor_matches_slots() {
    let path = write_tmp(
        "cli_test_compile_emit_bin_record_slots.zt",
        "{ prime_count = 10; compact_primes = [2; 3; 5;]; }\n",
    );
    let out = write_tmp("cli_test_compile_emit_bin_record_slots", "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "{ compact_primes = [2; 3; 5];  prime_count = 10 }\n",
    );
}

#[test]
fn compile_emit_bin_tuple_runs() {
    assert_eq!(
        compile_bin_stdout("cli_test_compile_emit_bin_tuple", "(1, \"two\")\n"),
        "(1, \"two\")\n"
    );
}

#[test]
fn compile_emit_bin_union_runs() {
    let src = r#"Shape :: type {
  #circle: { radius: Int; };
  #square: { side: Int; };
}
shape :: Shape = #circle { radius = 3; }
shape
"#;
    assert_eq!(
        compile_bin_stdout("cli_test_compile_emit_bin_union", src),
        "#circle { radius = 3 }\n"
    );
}

#[test]
fn compile_emit_bin_text_runs() {
    assert_eq!(
        compile_bin_stdout("cli_test_compile_emit_bin_text", "\"hello\"\n"),
        "\"hello\"\n"
    );
}

#[test]
fn compile_emit_bin_atom_runs() {
    assert_eq!(
        compile_bin_stdout("cli_test_compile_emit_bin_atom", "#prod\n"),
        "#prod\n"
    );
}

#[test]
fn compile_emit_bin_posit_runs() {
    assert_eq!(
        compile_bin_stdout("cli_test_compile_emit_bin_posit", "1p32 + 2p32\n"),
        "3p32\n"
    );
}

#[test]
fn compile_emit_bin_multi_arg_division_runs() {
    let path = write_tmp(
        "cli_test_compile_emit_bin_divides.zt",
        "divides :: Int -> Int -> Bool\n  = p n => (n / p) * p == n;\n\ndivides 2 4\n",
    );
    let out = write_tmp("cli_test_compile_emit_bin_divides", "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "true\n");
}

#[test]
fn compile_emit_bin_recursive_function_runs() {
    let path = write_tmp(
        "cli_test_compile_emit_bin_fib.zt",
        "fib :: Int -> Int\n  = n => if n < 2 then n else fib (n - 1) + fib (n - 2);\n\nfib 10\n",
    );
    let out = write_tmp("cli_test_compile_emit_bin_fib", "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "55\n");
}

#[test]
fn compile_derive_witness_program_passes() {
    let src = r#"
Point :: type { x : Int; y : Int; }
p1 :: Point = { x = 1; y = 2; }
p2 :: Point = { x = 1; y = 2; }
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Point :: derive
eq p1 p2
"#;
    let path = write_tmp("cli_test_compile_derive.zt", src);
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("define i64 @__entry"));
}
#[test]
fn compile_conditional_witness_program_passes() {
    // A conditional witness `Eq @(Pair A) :: <A: Eq>` resolves through the compile
    // (TLC -> dataflow) pipeline: the parametric witness is applied to the
    // recursively resolved `Eq @Int` component dict.
    let src = r#"
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Pair :: <A> type { fst : A; snd : A; }
Eq @(Pair A) :: <A: Eq> { eq = \p q. eq p.fst q.fst; }
p1 :: Pair Int = { fst = 1; snd = 2; }
p2 :: Pair Int = { fst = 1; snd = 2; }
eq p1 p2
"#;
    let path = write_tmp("cli_test_compile_conditional.zt", src);
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("define i64 @__entry"));
}

// ─── `select` projection (check / run / compile) ───────────────────────────────

const SELECT_SRC: &str =
    "s ::= { host = \"h\"; port = 8080; name = \"n\"; }\nselect s { port; host; }\n";
const SELECT_BAD_SRC: &str = "s ::= { host = \"h\"; }\nselect s { missing; }\n";

#[test]
fn check_select_passes() {
    let path = write_tmp("cli_test_check_select.zt", SELECT_SRC);
    cli()
        .arg("check")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("check passed"));
}

#[test]
fn run_select_projects_record() {
    let path = write_tmp("cli_test_run_select.zt", SELECT_SRC);
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("8080"))
        .stdout(predicate::str::contains("name").not());
}

#[test]
fn compile_select_emits_record_projection() {
    let llvm = compile_stdout("cli_test_compile_select.zt", SELECT_SRC);
    assert!(llvm.contains("call i64 @zutai.record_get"), "{llvm}");
    assert!(llvm.contains("call i64 @zutai.record_new"), "{llvm}");
    assert!(llvm_call_uses_slot(&llvm, "@zutai.record_get", 2), "{llvm}");
    assert!(llvm_call_uses_slot(&llvm, "@zutai.record_get", 0), "{llvm}");
    assert!(!llvm.contains("10100688915994460070"), "{llvm}");
}

#[test]
fn compile_record_update_emits_record_update_call() {
    let src = r#"
Server :: type { host : Text; port : Int; }
server :: Server = { host = "localhost"; port = 80; }
server with { port = 8080; }
"#;
    let llvm = compile_stdout("cli_test_compile_record_update.zt", src);
    assert!(llvm.contains("call i64 @zutai.record_update"), "{llvm}");
    assert!(
        llvm.lines().any(|line| {
            line.contains("call i64 @zutai.record_update") && line.contains(", i64 1, i64 8080)")
        }),
        "{llvm}"
    );
    assert!(!llvm.contains("10100688915994460070"), "{llvm}");
}

#[test]
fn compile_record_access_uses_sorted_slot_zero() {
    let llvm = compile_stdout(
        "cli_test_compile_record_slot_zero.zt",
        "r ::= { b = 10; a = 20; }\nr.a\n",
    );
    assert!(llvm_call_uses_slot(&llvm, "@zutai.record_get", 0), "{llvm}");
    assert!(
        llvm.lines().any(|line| {
            line.contains("call void @zutai.record_set") && line.contains(", i64 0, i64 20)")
        }),
        "{llvm}"
    );
    assert!(!llvm.contains("12638187200555641996"), "{llvm}");
}

#[test]
fn compile_record_access_uses_sorted_slot_one() {
    let llvm = compile_stdout(
        "cli_test_compile_record_slot_one.zt",
        "r ::= { b = 10; a = 20; }\nr.b\n",
    );
    assert!(llvm_call_uses_slot(&llvm, "@zutai.record_get", 1), "{llvm}");
    assert!(
        llvm.lines().any(|line| {
            line.contains("call void @zutai.record_set") && line.contains(", i64 1, i64 10)")
        }),
        "{llvm}"
    );
    assert!(!llvm.contains("12638190499090526629"), "{llvm}");
}

#[test]
fn compile_tuple_pattern_uses_positional_slot() {
    let src = r#"
first :: (Int, Int) -> Int
  = (x, _) => x;
first (1, 2)
"#;
    let llvm = compile_stdout("cli_test_compile_tuple_pattern_slot.zt", src);
    assert!(llvm_call_uses_slot(&llvm, "@zutai.record_get", 0), "{llvm}");
}

#[test]
fn compile_variant_pattern_uses_variant_value() {
    let src = r#"
Shape :: type {
  #circle: { radius: Int; };
  #square: { side: Int; };
}
area :: Shape -> Int
  = #circle { radius = r; } => r;
  = #square { side = s; } => s;
area (#circle { radius = 3; })
"#;
    let llvm = compile_stdout("cli_test_compile_variant_pattern_value.zt", src);
    assert!(llvm.contains("call i64 @zutai.variant_value"), "{llvm}");
}

const OVERLAY_SRC: &str = r#"
Config :: type { host : Text; port : Int; }
defaults :: Config = { host = "localhost"; port = 80; }
patch :: Patch Config = { port = 8080; }
defaults |> overlay patch
"#;

const OVERLAY_DEEP_SRC: &str = r#"
Server :: type { host : Text; port : Int; }
Config :: type { server : Server; name : Text; }
defaults :: Config = {
  server = { host = "localhost"; port = 80; };
  name = "dev";
}
patch :: DeepPatch Config = { server = { port = 8080; }; }
defaults |> overlayDeep patch
"#;

#[test]
fn check_overlay_passes() {
    let path = write_tmp("cli_test_check_overlay.zt", OVERLAY_SRC);
    cli()
        .arg("check")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("check passed"));
}

#[test]
fn run_overlay_merges_record() {
    let path = write_tmp("cli_test_run_overlay.zt", OVERLAY_SRC);
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("host = \"localhost\""))
        .stdout(predicate::str::contains("port = 8080"));
}

#[test]
fn compile_overlay_program_lowers_to_record_update() {
    let llvm = compile_stdout("cli_test_compile_overlay.zt", OVERLAY_SRC);
    assert!(llvm.contains("call i64 @zutai.record_update"), "{llvm}");
}

#[test]
fn dataflow_overlay_program_lowers_to_record_update() {
    let path = write_tmp("cli_test_dataflow_overlay.zt", OVERLAY_SRC);
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("RecordUpdate"));
}

#[test]
fn compile_overlay_emit_bin_runs() {
    let path = write_tmp("cli_test_compile_overlay_bin.zt", OVERLAY_SRC);
    let out = write_tmp("cli_test_compile_overlay_bin", "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("host = \"localhost\""), "{stdout}");
    assert!(stdout.contains("port = 8080"), "{stdout}");
}

#[test]
fn compile_overlay_deep_emit_bin_runs() {
    let path = write_tmp("cli_test_compile_overlay_deep_bin.zt", OVERLAY_DEEP_SRC);
    let out = write_tmp("cli_test_compile_overlay_deep_bin", "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("host = \"localhost\""), "{stdout}");
    assert!(stdout.contains("port = 8080"), "{stdout}");
    assert!(stdout.contains("name = \"dev\""), "{stdout}");
}

#[test]
fn check_select_unknown_field_exits_nonzero() {
    let path = write_tmp("cli_test_check_select_bad.zt", SELECT_BAD_SRC);
    cli().arg("check").arg(&path).assert().failure();
}

#[test]
fn run_select_unknown_field_exits_nonzero() {
    let path = write_tmp("cli_test_run_select_bad.zt", SELECT_BAD_SRC);
    cli().arg("run").arg(&path).assert().failure();
}

#[test]
fn compile_select_unknown_field_exits_nonzero() {
    let path = write_tmp("cli_test_compile_select_bad.zt", SELECT_BAD_SRC);
    cli().arg("compile").arg(&path).assert().failure();
}

// ─── `dataflow` subcommand ─────────────────────────────────────────────────────

#[test]
fn dataflow_valid_zt_file_prints_graph() {
    let path = write_tmp("cli_test_dataflow_valid.zt", "42\n");
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("DataflowGraph"));
}

#[test]
fn dataflow_zt_parse_error_exits_nonzero() {
    let path = write_tmp("cli_test_dataflow_parse_err.zt", "[1; 2]\n");
    cli().arg("dataflow").arg(&path).assert().failure();
}

#[test]
fn dataflow_effect_program_is_rejected_by_residual_effect_gate() {
    let path = write_tmp("cli_test_dataflow_effect.zt", EFFECT_SRC);
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("effect"));
}

#[test]
fn dataflow_reflection_schema_lowers_to_graph() {
    let path = write_tmp(
        "cli_test_dataflow_reflection_schema.zt",
        "Server :: type { host : Text; }\nschema Server\n",
    );
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Record"));
}

#[test]
fn dataflow_type_entry_is_rejected_before_backend_lowering() {
    let path = write_tmp("cli_test_dataflow_type_entry.zt", "type Int\n");
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("returns Type"));
}

#[test]
fn dataflow_type_alias_value_entry_is_rejected_before_backend_lowering() {
    let path = write_tmp(
        "cli_test_dataflow_type_alias_entry.zt",
        "MyInt :: type Int\nMyInt\n",
    );
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("returns Type"));
}

// ─── prelude `print` effect binding ───────────────────────────────────────────

#[test]
fn run_print_writes_to_stdout() {
    // The side effect emits `hello`; the returned value displays as `"hello"`.
    let path = write_tmp("cli_test_print.zt", "print \"hello\"\n");
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("hello").and(predicate::str::contains("\"hello\"")));
}

#[test]
fn run_print_list_emits_each_line() {
    let path = write_tmp(
        "cli_test_print_list.zt",
        "[print \"a\"; print \"b\"; print \"c\";]\n",
    );
    cli().arg("run").arg(&path).assert().success().stdout(
        predicate::str::contains("a")
            .and(predicate::str::contains("b"))
            .and(predicate::str::contains("c")),
    );
}

#[test]
fn run_effect_sequence_prints_in_order() {
    let path = write_tmp(
        "cli_test_print_sequence.zt",
        "{ perform io.print \"a\"; perform io.print \"b\"; 7 }\n",
    );
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout("a\nb\n7\n");
}

#[test]
fn compile_print_program_replays_host_print() {
    let path = write_tmp("cli_test_print_compile.zt", "print \"x\"\n");
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("@zutai.effect.print.0"))
        .stdout(predicate::str::contains("call void @zutai.print_text"));
}

#[test]
fn dataflow_print_program_lowers_after_host_effect_fold() {
    let path = write_tmp("cli_test_print_dataflow.zt", "print \"x\"\n");
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Text"));
}

#[test]
fn parse_invalid_zti_exits_nonzero() {
    let path = write_tmp("cli_test_parse_invalid.zti", "{ a = 1 }\n");
    cli()
        .arg("parse")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to parse .zti"));
}

#[test]
fn bare_invalid_zti_exits_nonzero() {
    let path = write_tmp("cli_test_bare_invalid.zti", "{ a = [1] ; }\n");
    cli()
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Failed to parse .zti"));
}

#[test]
fn run_integer_overflow_exits_runtime_error() {
    let path = write_tmp("cli_test_run_overflow.zt", "9223372036854775807 + 1\n");
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "runtime error: integer overflow in `+`",
        ));
}

#[test]
fn dataflow_zt_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_dataflow_type_err.zt", "x :: Int = \"bad\"\nx\n");
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "type mismatch: expected Int, found Text",
        ));
}

#[test]
fn compile_entry_function_is_rejected() {
    let path = write_tmp(
        "cli_test_compile_entry_function.zt",
        "id :: Int -> Int\n  = x => x;\nid\n",
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "compiled entry point returns a function",
        ));
}
