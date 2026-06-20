use assert_cmd::Command;
use predicates::prelude::*;

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
fn run_deep_recursion_does_not_overflow_stack() {
    // Regression: the tree-walking interpreter runs on a large worker stack so
    // deep (but finite) recursion completes instead of aborting the process.
    // `count 5000` overflows the default ~8 MiB main-thread stack.
    let src = "count :: Int -> Int {\n  | 0 => 0;\n  | n => 1 + count (n - 1);\n}\ncount 5000\n";
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
fn repl_accepts_declaration_then_expression() {
    let mut cmd = cli();
    cmd.arg("repl")
        .write_stdin("x := 42\nx\n:quit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("42"));
}

#[test]
fn repl_reset_clears_bindings() {
    let mut cmd = cli();
    cmd.arg("repl")
        .write_stdin("x := 10\n:reset\n:quit\n")
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
        "lib := import \"./does_not_exist.zti\"\n1\n",
    );
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("import error"));
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
        .stderr(predicate::str::contains("error"));
}

// ─── `check` subcommand ────────────────────────────────────────────────────────

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

// ─── `compile` subcommand ──────────────────────────────────────────────────────

#[test]
fn compile_valid_zt_file_emits_llvm_ir() {
    let path = write_tmp("cli_test_compile_valid.zt", "42\n");
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("define i64 @__entry"));
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
    "s := { host = \"h\"; port = 8080; name = \"n\"; }\nselect s { port; host; }\n";
const SELECT_BAD_SRC: &str = "s := { host = \"h\"; }\nselect s { missing; }\n";

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
    let path = write_tmp("cli_test_compile_select.zt", SELECT_SRC);
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .success()
        // Match the call sites (not the always-present runtime `declare`s): a
        // `record_get` projection per selected field and a `record_new` to build
        // the projected record — neither appears unless `select` actually lowered.
        .stdout(predicate::str::contains("call i64 @zutai.record_get"))
        .stdout(predicate::str::contains("call i64 @zutai.record_new"));
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

// ─── prelude `print` builtin ───────────────────────────────────────────────────

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
fn compile_print_program_is_rejected() {
    // The v0 compiled core has no ambient effects; `print` is interpreter-only.
    let path = write_tmp("cli_test_print_compile.zt", "print \"x\"\n");
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("interpreter-only"));
}

#[test]
fn dataflow_print_program_is_rejected() {
    let path = write_tmp("cli_test_print_dataflow.zt", "print \"x\"\n");
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("interpreter-only"));
}
