use assert_cmd::Command;
use predicates::prelude::*;
use std::{
    path::{Path, PathBuf},
    process::Command as StdCommand,
};

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
fn general_fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../general/fixtures/valid")
        .join(name)
        .to_str()
        .expect("fixture path must be UTF-8")
        .to_owned()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn example_fixture(name: &str) -> String {
    workspace_root()
        .join("examples")
        .join(name)
        .to_str()
        .expect("example path must be UTF-8")
        .to_owned()
}

fn zt_string_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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

fn run_stdout(name: &str, content: &str) -> String {
    let path = write_tmp(name, content);
    let output = cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("run output should be UTF-8")
}

fn run_path_stdout(path: &str) -> String {
    let output = cli()
        .arg("run")
        .arg(path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("run output should be UTF-8")
}

fn compile_path_bin_stdout(name: &str, path: &str) -> String {
    let out = write_tmp(name, "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn check_passes(name: &str, content: &str) {
    let path = write_tmp(name, content);
    cli()
        .arg("check")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("check passed"));
}

fn check_path_passes(path: &str) {
    cli()
        .arg("check")
        .arg(path)
        .assert()
        .success()
        .stdout(predicate::str::contains("check passed"));
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
fn run_unicode_identifier_prints_result() {
    assert_eq!(
        run_stdout("cli_test_unicode_ident.zt", "café ::= 42;\ncafé\n"),
        "42\n"
    );
}

#[test]
fn run_stream_generator_folds_codata_stream() {
    // `Stream A` is demand-driven codata, so a generator is observed by forcing
    // it (`s ()` yields a `#nil`/`#cons` cell), not printed as a list.
    let path = write_tmp(
        "cli_test_stream_generator.zt",
        "sumS :: Stream Int -> Int\n  = s => match s () {\n    | #nil => 0;\n    | #cons { head = h; tail = t; } => h + sumS t;\n  };\nsumS (stream { yield 1; yield 2; yield 3; })\n",
    );
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("6"));
}

#[test]
fn real_examples_check_run_and_compile_match() {
    let cases = [
        (
            "service_health.zt",
            [
                "activeCount = 2",
                "budget = #overBudget",
                "validation = #clean",
            ],
        ),
        (
            "canary_forecast.zt",
            ["band = #hot", "peakScore = 93", "hotterThanNinety = true"],
        ),
    ];

    for (name, snippets) in cases {
        let path = example_fixture(name);
        check_path_passes(&path);

        let rendered = run_path_stdout(&path);
        for snippet in snippets {
            assert!(
                rendered.contains(snippet),
                "{name} output should contain `{snippet}`; got {rendered}"
            );
        }

        let native = compile_path_bin_stdout(&format!("cli_test_example_{name}"), &path);
        assert_eq!(native, rendered, "{name} native output must match run");
    }
}

// V3-G1: `Stream A` is demand-driven codata (`Unit -> StreamCell A`). A finite
// generator folds to the same value the interpreter computes.
const CODATA_STREAM_FINITE_SRC: &str = "sumS :: Stream Int -> Int\n  = s => match s () {\n    | #nil => 0;\n    | #cons { head = h; tail = t; } => h + sumS t;\n  };\nsumS (stream { yield 1; yield 2; yield 3; })\n";

// An *infinite* stream (`nats`) bounded by a demand-driven `takeSum`: forcing only
// the first 5 cells terminates. Sum 0..4 = 10. Proves laziness on both paths.
const CODATA_STREAM_INFINITE_SRC: &str = "nats :: Int -> Stream Int\n  = n _ => #cons { head = n; tail = nats (n + 1); };\ntakeSum :: Int -> Stream Int -> Int\n  = k s => if k < 1 then 0 else match s () {\n    | #nil => 0;\n    | #cons { head = h; tail = t; } => h + takeSum (k - 1) t;\n  };\ntakeSum 5 (nats 0)\n";

#[test]
fn compile_codata_stream_finite_generator_matches_oracle() {
    let native = compile_bin_stdout("cli_test_codata_finite", CODATA_STREAM_FINITE_SRC);
    let interp = run_stdout("cli_test_codata_finite_oracle.zt", CODATA_STREAM_FINITE_SRC);
    assert_eq!(native.trim(), "6");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_codata_stream_infinite_take_matches_oracle() {
    // Demand-driven: an infinite stream must terminate under `take` on the native
    // backend (a strict mislowering would loop or diverge).
    let native = compile_bin_stdout("cli_test_codata_infinite", CODATA_STREAM_INFINITE_SRC);
    let interp = run_stdout(
        "cli_test_codata_infinite_oracle.zt",
        CODATA_STREAM_INFINITE_SRC,
    );
    assert_eq!(native.trim(), "10");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// V3-G2: the imported `Stream` API (map/filter/take/drop/fold/cons/
// singleton/uncons), native-compiled, matching the oracle.
const PRELUDE_STREAM_PIPELINE_SRC: &str = "s ::= import stdlib.stream;\ncountFrom :: Int -> s.Stream Int\n  = n _ => #cons { head = n; tail = countFrom (n + 1); };\ns.fold (\\a b. a + b) 0 (s.drop 1 (s.take 4 (s.filter (\\x. x > 15) (s.map (\\x. x * 10) (countFrom 1)))))\n";

#[test]
fn compile_prelude_stream_pipeline_matches_oracle() {
    // map *10 -> 10,20,30,40,50; filter >15 -> 20,30,40,50; take 4 -> 20,30,40,50;
    // drop 1 -> 30,40,50; fold + -> 120.
    let native = compile_bin_stdout("cli_test_prelude_stream", PRELUDE_STREAM_PIPELINE_SRC);
    let interp = run_stdout(
        "cli_test_prelude_stream_oracle.zt",
        PRELUDE_STREAM_PIPELINE_SRC,
    );
    assert_eq!(native.trim(), "120");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

const PRELUDE_STREAM_CONS_SRC: &str = "s ::= import stdlib.stream;\nfirstOr :: Int -> s.Stream Int -> Int\n  = d xs => match s.uncons xs { | #none => d; | #some { head = h; tail = _; } => h; };\nfirstOr 0 (s.cons 99 (s.singleton 7))\n";

#[test]
fn compile_prelude_stream_cons_uncons_matches_oracle() {
    let native = compile_bin_stdout("cli_test_prelude_cons", PRELUDE_STREAM_CONS_SRC);
    let interp = run_stdout("cli_test_prelude_cons_oracle.zt", PRELUDE_STREAM_CONS_SRC);
    assert_eq!(native.trim(), "99");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

const PRELUDE_LIST_PIPELINE_SRC: &str =
    "{1; 2; 3; 4;} |> filter (\\x. x > 1) |> map (\\x. x * 2) |> fold (\\acc x. acc + x) 0\n";

#[test]
fn compile_prelude_list_pipeline_matches_oracle() {
    let native = compile_bin_stdout("cli_test_prelude_list", PRELUDE_LIST_PIPELINE_SRC);
    let interp = run_stdout("cli_test_prelude_list_oracle.zt", PRELUDE_LIST_PIPELINE_SRC);
    assert_eq!(native.trim(), "18");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

const STDLIB_OPTIONAL_PIPELINE_SRC: &str = "o ::= import stdlib.optional;\n\
noneInt :: Int? = #none;\n\
score :: Int? -> Int\n  = maybe => maybe |> o.filter (\\x. x > 1) |> o.map (\\x. x * 10) |> o.andThen (\\x. if x > 20 then #some (x + 1) else #none) |> o.withDefault 5;\n\
score (#some (3)) + score (#some (1)) + score noneInt + length (o.toList (#some (9))) + length (o.toList noneInt)\n";

#[test]
fn compile_stdlib_optional_pipeline_matches_oracle() {
    let native = compile_bin_stdout("cli_test_stdlib_optional", STDLIB_OPTIONAL_PIPELINE_SRC);
    let interp = run_stdout(
        "cli_test_stdlib_optional_oracle.zt",
        STDLIB_OPTIONAL_PIPELINE_SRC,
    );
    assert_eq!(native.trim(), "42");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

const STDLIB_RESULT_PIPELINE_SRC: &str = "r ::= import stdlib.result;\n\
good :: r.Result Text Int = r.ok 3;\n\
bad :: r.Result Int Int = r.err 4;\n\
score :: r.Result Text Int -> Int\n  = res => res |> r.map (\\x. x * 10) |> r.andThen (\\x. if x > 20 then r.ok (x + 1) else r.err \"small\") |> r.withDefault 5;\n\
v1 :: r.Validation Text Int = r.invalid {\"a\";};\n\
v2 :: r.Validation Text Int = r.invalid {\"b\"; \"c\";};\n\
score good + score (r.ok 1) + score (r.err \"no\") + length (r.errors (r.map2 (\\x y. x + y) v1 v2)) + (match r.mapErr (\\e. e + 1) bad { | #err { error = e; } => e - 2; | #ok { value = _; } => 0; })\n";

#[test]
fn compile_stdlib_result_pipeline_matches_oracle() {
    let native = compile_bin_stdout("cli_test_stdlib_result", STDLIB_RESULT_PIPELINE_SRC);
    let interp = run_stdout(
        "cli_test_stdlib_result_oracle.zt",
        STDLIB_RESULT_PIPELINE_SRC,
    );
    assert_eq!(native.trim(), "47");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

const STDLIB_NUM_PIPELINE_SRC: &str = "n ::= import stdlib.num;\n\
score :: Int -> Int\n  = x => x |> n.clamp 0 10 |> n.pow 2 |> n.rem 17;\n\
score 12 + score (0 - 3) + n.gcd (0 - 84) 30 + n.abs (0 - 9) + n.round 2.6 + n.truncate 2.9 + (if n.toFloat 4 == 4.0 then 5 else 0)\n";

#[test]
fn compile_stdlib_num_pipeline_matches_oracle() {
    let native = compile_bin_stdout("cli_test_stdlib_num", STDLIB_NUM_PIPELINE_SRC);
    let interp = run_stdout("cli_test_stdlib_num_oracle.zt", STDLIB_NUM_PIPELINE_SRC);
    assert_eq!(native.trim(), "42");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

const STDLIB_TEXT_CMP_PIPELINE_SRC: &str = "t ::= import stdlib.text;\n\
c ::= import stdlib.cmp;\n\
o ::= import stdlib.optional;\n\
parts ::= t.split \",\" \"a,b,c\";\n\
textScore :: Int = t.length (t.join \":\" parts) + t.length (t.trim \"  z  \") + t.length (t.replace \"a\" \"o\" \"cat\") + o.withDefault 0 (t.parseInt \"42\");\n\
cmpScore :: Int = if c.then (c.compareText \"a\" \"b\") (c.reverse c.gt) == c.lt then 7 else 0;\n\
textScore + cmpScore\n";

#[test]
fn compile_stdlib_text_cmp_pipeline_matches_oracle() {
    let native = compile_bin_stdout("cli_test_stdlib_text_cmp", STDLIB_TEXT_CMP_PIPELINE_SRC);
    let interp = run_stdout(
        "cli_test_stdlib_text_cmp_oracle.zt",
        STDLIB_TEXT_CMP_PIPELINE_SRC,
    );
    assert_eq!(native.trim(), "58");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// Release slice R0: one CLI acceptance pack for the shipped V3 + stdlib-H
// envelope. It intentionally crosses `check`, interpreter `run`, and native
// `compile --emit=bin`: ambient source preludes, explicit stdlib modules, codata
// generators/streams, and runtime `io.print` must all agree. The aggregate stdlib
// program stays pure because imported stdlib module records expose type members;
// `io.print` is covered as a separate case in the same pack.
const RELEASE_ACCEPTANCE_SRC: &str = r#"s ::= import stdlib.stream;
o ::= import stdlib.optional;
r ::= import stdlib.result;
n ::= import stdlib.num;
t ::= import stdlib.text;
c ::= import stdlib.cmp;

range :: Int -> Int -> s.Stream Int
  = lo hi => stream {
    if lo < hi then {
      yield lo;
      yield from range (lo + 1) hi;
    }
  };

functionScore :: Int = compose (\x. x + 1) (\x. x * 2) (flip (\x y. x - y) 3 10);
listScore :: Int = fold (\acc x. acc + x) 0 (map (\x. x * 2) (filter (\x. x > 1) {1; 2; 3;}));
optionalScore :: Int = o.withDefault 0 (o.map (\x. x + 1) (#some (4))) + length (o.toList (#some (9)));
resultScore :: Int = r.withDefault 0 (r.map (\x. x * 3) (r.ok 7)) + length (r.errors (r.map2 (\x y. x + y) (r.invalid {"a";}) (r.invalid {"b"; "c";})));
numScore :: Int = n.gcd (0 - 84) 30 + n.abs (0 - 5) + n.round 2.6 + n.truncate 2.9;
textScore :: Int = t.length (t.join ":" (t.split "," "a,b")) + o.withDefault 0 (t.parseInt "11");
cmpScore :: Int = if c.then (c.compareInt 1 2) (c.reverse c.gt) == c.lt then 7 else 0;
streamScore :: Int = s.fold (\acc x. acc + x) 0 (s.map (\x. x * 2) (s.take 3 (range 1 10)));
total :: Int = functionScore + listScore + optionalScore + resultScore + numScore + textScore + cmpScore + streamScore;
total
"#;
const RELEASE_ACCEPTANCE_PRINT_SRC: &str = "print \"release-acceptance\"\n";

#[test]
fn release_acceptance_pack_check_run_compile_match() {
    check_passes(
        "cli_test_release_acceptance_check.zt",
        RELEASE_ACCEPTANCE_SRC,
    );
    let interp = run_stdout(
        "cli_test_release_acceptance_oracle.zt",
        RELEASE_ACCEPTANCE_SRC,
    );
    let native = compile_bin_stdout("cli_test_release_acceptance", RELEASE_ACCEPTANCE_SRC);
    assert_eq!(interp, "104\n");
    assert_eq!(native, interp, "native must match the interpreter oracle");

    check_passes(
        "cli_test_release_acceptance_print_check.zt",
        RELEASE_ACCEPTANCE_PRINT_SRC,
    );
    let print_interp = run_stdout(
        "cli_test_release_acceptance_print_oracle.zt",
        RELEASE_ACCEPTANCE_PRINT_SRC,
    );
    let print_native = compile_bin_stdout(
        "cli_test_release_acceptance_print",
        RELEASE_ACCEPTANCE_PRINT_SRC,
    );
    assert_eq!(print_interp, "release-acceptance\n\"release-acceptance\"\n");
    assert_eq!(
        print_native, print_interp,
        "native print must match the interpreter oracle"
    );
}

// V3-G2 residual: `unfold` — the canonical codata producer (step + seed). A
// `Step S A` (`#done`/`#yield { item; next }`) step function drives an infinite
// stream that `take`/`fold` bound. Native-compiled, matching the oracle.
const PRELUDE_STREAM_UNFOLD_SRC: &str = "s ::= import stdlib.stream;\nstep :: Int -> s.Step Int Int\n  = n => if n > 5 then #done else #yield { item = n; next = n + 1; };\ns.fold (\\a b. a + b) 0 (s.take 4 (s.unfold step 1))\n";

#[test]
fn compile_prelude_stream_unfold_matches_oracle() {
    // unfold step 1 → 1,2,3,4,5; take 4 → 1,2,3,4; fold (+) 0 → 10.
    let native = compile_bin_stdout("cli_test_prelude_unfold", PRELUDE_STREAM_UNFOLD_SRC);
    let interp = run_stdout(
        "cli_test_prelude_unfold_oracle.zt",
        PRELUDE_STREAM_UNFOLD_SRC,
    );
    assert_eq!(native.trim(), "10");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// V3-G2 residual: `empty` — the empty stream. A polymorphic *nullary value*; the
// case that drove the `BindingRef`-instantiation fix (it leaks a rigid type var
// when referenced outside callee position). Consumed by `cons`/`fold` here.
const PRELUDE_STREAM_EMPTY_SRC: &str = "sumI :: Stream Int -> Int\n  = s => match s () { | #nil => 0; | #cons { head = h; tail = t; } => h + sumI t; };\nsumI (cons 5 (cons 7 empty))\n";

#[test]
fn compile_prelude_stream_empty_matches_oracle() {
    // empty = []; cons 7 → [7]; cons 5 → [5,7]; sum = 12.
    let native = compile_bin_stdout("cli_test_prelude_empty", PRELUDE_STREAM_EMPTY_SRC);
    let interp = run_stdout("cli_test_prelude_empty_oracle.zt", PRELUDE_STREAM_EMPTY_SRC);
    assert_eq!(native.trim(), "12");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// `empty` instantiates independently per use — consumed at both `Stream Bool`
// and `Stream Int` in one program — proving true polymorphism on the backend.
const PRELUDE_STREAM_EMPTY_POLY_SRC: &str = "s ::= import stdlib.stream;\nsumI :: s.Stream Int -> Int\n  = xs => match xs () { | #nil => 0; | #cons { head = h; tail = t; } => h + sumI t; };\nfirstB :: Bool -> s.Stream Bool -> Bool\n  = d xs => match s.uncons xs { | #none => d; | #some { head = h; tail = _; } => h; };\nif firstB true s.empty then sumI (s.cons 5 (s.cons 7 s.empty)) else sumI s.empty\n";

#[test]
fn compile_prelude_stream_empty_polymorphic_matches_oracle() {
    // firstB true empty → true (empty has no head); then sumI [5,7] = 12.
    let native = compile_bin_stdout("cli_test_prelude_empty_poly", PRELUDE_STREAM_EMPTY_POLY_SRC);
    let interp = run_stdout(
        "cli_test_prelude_empty_poly_oracle.zt",
        PRELUDE_STREAM_EMPTY_POLY_SRC,
    );
    assert_eq!(native.trim(), "12");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// V3-G2 residual: List interop. `fromList` adapts a builtin `List` into a codata
// `Stream`, `map` transforms it, `toList` materializes the finite result back to a
// `List`. Exercises the scalar list-bridge primitives (`listIsNil`/`listHead`/
// `listTail` under `fromList`, `listCons`/`listEmpty` under `toList`) on the native
// backend against the interpreter oracle.
const PRELUDE_STREAM_TOLIST_SRC: &str = "s ::= import stdlib.stream;\ndouble :: Int -> Int = x => x * 2;\ns.toList (s.map double (s.fromList {1; 2; 3;}))\n";

#[test]
fn compile_prelude_stream_tolist_matches_oracle() {
    // fromList [1,2,3] → stream; map (*2) → 2,4,6; toList → [2; 4; 6].
    let native = compile_bin_stdout("cli_test_prelude_tolist", PRELUDE_STREAM_TOLIST_SRC);
    let interp = run_stdout(
        "cli_test_prelude_tolist_oracle.zt",
        PRELUDE_STREAM_TOLIST_SRC,
    );
    assert_eq!(native.trim(), "[2; 4; 6]");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// `fromList` feeds the ordinary stream pipeline; folding to a scalar avoids
// depending on list rendering and proves the List→Stream adaptation steps lazily.
const PRELUDE_STREAM_FROMLIST_SRC: &str = "s ::= import stdlib.stream;\ns.fold (\\a b. a + b) 0 (s.filter (\\x. x > 15) (s.fromList {10; 20; 30;}))\n";

#[test]
fn compile_prelude_stream_fromlist_matches_oracle() {
    // fromList [10,20,30] → filter >15 → 20,30 → fold (+) 0 = 50.
    let native = compile_bin_stdout("cli_test_prelude_fromlist", PRELUDE_STREAM_FROMLIST_SRC);
    let interp = run_stdout(
        "cli_test_prelude_fromlist_oracle.zt",
        PRELUDE_STREAM_FROMLIST_SRC,
    );
    assert_eq!(native.trim(), "50");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// `takeList` materializes a bounded prefix of an *infinite* generator — proving the
// `toList ∘ take` composition terminates natively under demand.
const PRELUDE_STREAM_TAKELIST_SRC: &str = "countFrom :: Int -> Stream Int\n  = n _ => #cons { head = n; tail = countFrom (n + 1); };\ntakeList 5 (countFrom 1)\n";

#[test]
fn compile_prelude_stream_takelist_infinite_matches_oracle() {
    // takeList 5 (countFrom 1) → [1; 2; 3; 4; 5].
    let native = compile_bin_stdout("cli_test_prelude_takelist", PRELUDE_STREAM_TAKELIST_SRC);
    let interp = run_stdout(
        "cli_test_prelude_takelist_oracle.zt",
        PRELUDE_STREAM_TAKELIST_SRC,
    );
    assert_eq!(native.trim(), "[1; 2; 3; 4; 5]");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// Empty edges: `takeList 0` and `toList empty` both materialize the empty list.
const PRELUDE_STREAM_TOLIST_EMPTY_SRC: &str = "takeList 0 (cons 1 (singleton 2))\n";

#[test]
fn compile_prelude_stream_tolist_empty_matches_oracle() {
    let native = compile_bin_stdout(
        "cli_test_prelude_tolist_empty",
        PRELUDE_STREAM_TOLIST_EMPTY_SRC,
    );
    let interp = run_stdout(
        "cli_test_prelude_tolist_empty_oracle.zt",
        PRELUDE_STREAM_TOLIST_EMPTY_SRC,
    );
    assert_eq!(native.trim(), "[]");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// V3-G3: richer `yield` — a recursive generator (guard `if` + `yield` + tail
// `yield from`) folds to the same value as the equivalent `unfold`. `range 1 6`
// yields 1..5; sum = 15.
const G3_RECURSIVE_GEN_SRC: &str = "range :: Int -> Int -> Stream Int\n  = lo hi => stream {\n    if lo < hi then {\n      yield lo;\n      yield from range (lo + 1) hi;\n    }\n  };\nsumS :: Stream Int -> Int\n  = s => match s () {\n    | #nil => 0;\n    | #cons { head = h; tail = t; } => h + sumS t;\n  };\nsumS (range 1 6)\n";

#[test]
fn compile_g3_recursive_generator_matches_oracle() {
    let native = compile_bin_stdout("cli_test_g3_recursive", G3_RECURSIVE_GEN_SRC);
    let interp = run_stdout("cli_test_g3_recursive_oracle.zt", G3_RECURSIVE_GEN_SRC);
    assert_eq!(native.trim(), "15");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

// V3-G3: conditional yield (emit-or-skip) composed with imported `take`/`fold`
// over an *infinite* recursive generator — proves demand drives the conditional
// on the native backend. `evensFrom 0` yields 0,2,4,…; take 4 → 0,2,4,6; sum 12.
const G3_CONDITIONAL_GEN_SRC: &str = "s ::= import stdlib.stream;\nevensFrom :: Int -> s.Stream Int\n  = n => stream {\n    if n - (n / 2) * 2 == 0 then { yield n; }\n    yield from evensFrom (n + 1);\n  };\ns.fold (\\a b. a + b) 0 (s.take 4 (evensFrom 0))\n";

#[test]
fn compile_g3_conditional_infinite_generator_matches_oracle() {
    let native = compile_bin_stdout("cli_test_g3_conditional", G3_CONDITIONAL_GEN_SRC);
    let interp = run_stdout("cli_test_g3_conditional_oracle.zt", G3_CONDITIONAL_GEN_SRC);
    assert_eq!(native.trim(), "12");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn prelude_stream_name_yields_to_user_definition() {
    // The prelude is a fallback: a user/constraint binding named like a prelude
    // function (here a `Functor` method `map`) wins, with no collision.
    let src = "Functor :: <F :: Type -> Type> @F { map :: <A, B> (A -> B) -> F A -> F B; }\nFunctor @List :: { map = \\f xs. xs; }\nmapTwice :: <F: Functor, A> (A -> A) -> F A -> F A\n  = f xs => map f (map f xs);\n1\n";
    let path = write_tmp("cli_test_prelude_fallback.zt", src);
    cli()
        .arg("check")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("check passed"));
}

// Stdlib slice B: the ambient function prelude (`id`/`const`/`compose`/`flip`) —
// no import needed, ordinary polymorphic source decls, native-compiled, matching
// the oracle on representative higher-order uses.
const PRELUDE_FUNCTIONS_SRC: &str =
    "compose (\\x. x + 1) (\\x. x * 2) (flip (\\x y. x - y) 3 10)\n";

#[test]
fn compile_prelude_functions_matches_oracle() {
    // flip (-) 3 10 = 7; (*2) 7 = 14; (+1) 14 = 15.
    let native = compile_bin_stdout("cli_test_prelude_functions", PRELUDE_FUNCTIONS_SRC);
    let interp = run_stdout(
        "cli_test_prelude_functions_oracle.zt",
        PRELUDE_FUNCTIONS_SRC,
    );
    assert_eq!(native.trim(), "15");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_prelude_functions_user_shadow_matches_oracle() {
    // A user `compose` shadows the ambient one; the program still native-compiles
    // and matches the oracle (the prelude is a fallback, never a duplicate).
    let src = "compose :: Int -> Int -> Int = a b => a - b;\ncompose 10 3\n";
    let native = compile_bin_stdout("cli_test_prelude_functions_shadow", src);
    let interp = run_stdout("cli_test_prelude_functions_shadow_oracle.zt", src);
    assert_eq!(native.trim(), "7");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_stdlib_prelude_import_matches_oracle() {
    // `import stdlib.prelude` exports the same helpers across the import
    // boundary; native must match the interpreter oracle.
    let src = "p ::= import stdlib.prelude;\n{ id; compose; flip; } ::= p;\ncompose (\\x. x + 1) id (flip (\\x y. x - y) 3 10)\n";
    let native = compile_bin_stdout("cli_test_stdlib_prelude_import", src);
    let interp = run_stdout("cli_test_stdlib_prelude_import_oracle.zt", src);
    // flip (-) 3 10 = 7; id 7 = 7; (+1) 7 = 8.
    assert_eq!(native.trim(), "8");
    assert_eq!(native, interp, "native must match the interpreter oracle");
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
    // `{1; 2}` is an invalid Zutai expression (list items must each end with
    // `;`, so the missing trailing `;` after `2` is a parse error).
    let path = write_tmp("cli_test_parse_err.zt", "{1; 2}\n");
    cli().arg("run").arg(&path).assert().failure();
}

#[test]
fn run_zt_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_type_err.zt", "x :: Int = \"bad\";\nx\n");
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

// ─── `json` subcommand ────────────────────────────────────────────────────────

#[test]
fn json_zti_file_prints_natural_json() {
    let path = write_tmp(
        "cli_test_json.zti",
        "{ host = \"localhost\"; port = 8080; mode = #prod; flags = [true; #fast;]; }\n",
    );
    cli()
        .arg("json")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"host\": \"localhost\""))
        .stdout(predicate::str::contains("\"port\": 8080"))
        .stdout(predicate::str::contains("\"mode\": \"#prod\""))
        .stdout(predicate::str::contains("\"flags\""));
}

#[test]
fn json_zt_file_evaluates_final_result() {
    let path = write_tmp(
        "cli_test_json_eval.zt",
        "cfg ::= { host = \"localhost\"; port = 8000 + 80; };\ncfg\n",
    );
    cli()
        .arg("json")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"port\": 8080"))
        .stdout(predicate::str::contains("8000 + 80").not());
}

#[test]
fn json_zt_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_json_type_err.zt", "x :: Int = \"bad\";\nx\n");
    cli()
        .arg("json")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("type error"));
}

#[test]
fn json_unsupported_extension_exits_nonzero() {
    let path = write_tmp("cli_test_json_unsupported.txt", "{ x = 1; }\n");
    cli()
        .arg("json")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unsupported"));
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
    // `{1; 2}` produces a parse error in .zt files.
    let path = write_tmp("cli_test_parse_parse_err.zt", "{1; 2}\n");
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
        .write_stdin("x ::= 42;\nx\n:quit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("42"));
}

#[test]
fn repl_reset_clears_bindings() {
    let mut cmd = cli();
    cmd.arg("repl")
        .write_stdin("x ::= 10;\n:reset\n:quit\n")
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
        "lib ::= import \"./does_not_exist.zti\";\n1\n",
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
        "cfg ::= import \"./cli_test_print_import.zti\";\nprint cfg.host\n",
    );
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("127.0.0.1"));
}

#[test]
fn compile_zt_imported_generic_fn_matches_oracle() {
    // Cross-module polymorphism: a dependency exporting a polymorphic function,
    // used at a concrete type, must lower natively (the dependency global keeps
    // its free-`TyVar` type; under untagged-i64 it is the same machine code as any
    // instantiation) and match the interpreter oracle.
    let (interp, native) = import_run_vs_compile(
        "xm_generic_fn",
        "main.zt",
        &[
            ("dep.zt", "idS :: <A> A -> A = x => x;\nidS\n"),
            ("main.zt", "dep ::= import \"dep.zt\";\ndep 42\n"),
        ],
    );
    assert_eq!(native.trim(), "42");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_imported_generic_record_matches_oracle() {
    // A dependency exporting a record of polymorphic functions (the stdlib shape)
    // used at a concrete type lowers natively.
    let (interp, native) = import_run_vs_compile(
        "xm_generic_record",
        "main.zt",
        &[
            (
                "dep.zt",
                "apply :: <A, B> (A -> B) -> A -> B = f x => f x;\n{ apply = apply; }\n",
            ),
            (
                "main.zt",
                "dep ::= import \"dep.zt\";\ndep.apply (\\x. x + 1) 41\n",
            ),
        ],
    );
    assert_eq!(native.trim(), "42");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_imported_generic_multitype_matches_oracle() {
    // The import boundary now carries polymorphism (XM-1..3): an imported generic
    // is quantified and instantiated fresh per use, so using it at two different
    // concrete types type-checks and lowers natively. `id` at Bool and at Int:
    // `if dep true then dep 1 else 0` = 1.
    let (interp, native) = import_run_vs_compile(
        "xm_generic_multitype",
        "main.zt",
        &[
            ("dep.zt", "idS :: <A> A -> A = x => x;\nidS\n"),
            (
                "main.zt",
                "dep ::= import \"dep.zt\";\nif dep true then dep 1 else 0\n",
            ),
        ],
    );
    assert_eq!(native.trim(), "1");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_imported_generic_record_multitype_matches_oracle() {
    // A record of generic functions (the importable-stdlib shape) used at two
    // types: `apply` at Int->Int and at Bool->Bool.
    let (interp, native) = import_run_vs_compile(
        "xm_generic_record_multitype",
        "main.zt",
        &[
            (
                "dep.zt",
                "apply :: <A, B> (A -> B) -> A -> B = f x => f x;\n{ apply = apply; }\n",
            ),
            (
                "main.zt",
                "dep ::= import \"dep.zt\";\nfirst :: Int -> Bool -> Int = i _ => i;\nfirst (dep.apply (\\x. x + 1) 41) (dep.apply (\\b. b) true)\n",
            ),
        ],
    );
    assert_eq!(native.trim(), "42");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_imported_stream_module_matches_oracle() {
    // V3-G6: the codata `Stream` combinators packaged as an importable `.zt`
    // module. The module source is the single source of truth that also backs the
    // ambient prelude (`zutai_hir::STREAM_MODULE_SRC`), so importing it must yield
    // the same combinators the ambient surface already exposes — exercised here
    // through the import boundary (qualified `s.map`/`s.filter`/…). A program builds
    // a finite stream, maps/filters/takes/folds it, and the native compile must
    // match the interpreter oracle. The recursive `Stream` codata type crossing the
    // boundary is the case that drove the cross-module global-ref compat fix.
    let (interp, native) = import_run_vs_compile(
        "g6_stream_module",
        "main.zt",
        &[
            ("stream.zt", zutai_hir::STREAM_MODULE_SRC),
            (
                "main.zt",
                "s ::= import \"stream.zt\";\n\
                 isEven :: Int -> Bool = n => (n / 2) * 2 == n;\n\
                 add :: Int -> Int -> Int = a b => a + b;\n\
                 double :: Int -> Int = x => x * 2;\n\
                 s.fold add 0 (s.take 2 (s.map double (s.filter isEven (s.cons 1 (s.cons 2 (s.cons 3 (s.cons 4 (s.singleton 6))))))))\n",
            ),
        ],
    );
    // src = [1,2,3,4,6]; filter even = [2,4,6]; map (*2) = [4,8,12]; take 2 = [4,8];
    // fold (+) 0 = 12.
    assert_eq!(native.trim(), "12");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_imported_stream_unfold_matches_oracle() {
    // V3-G2 residual through the import boundary: `s.unfold` builds a stream from a
    // step function. The `Step S A` producer union crosses the import boundary
    // structurally inside `unfold`'s signature (the `Step` type is not an exported
    // field, so the importer names a local structural alias for the step
    // annotation). Native compile must match the interpreter oracle.
    let (interp, native) = import_run_vs_compile(
        "g6_stream_unfold",
        "main.zt",
        &[
            ("stream.zt", zutai_hir::STREAM_MODULE_SRC),
            (
                "main.zt",
                "s ::= import \"stream.zt\";\n\
                 Step :: <S, A> type { #done; #yield : { item : A; next : S; }; };\n\
                 step :: Int -> Step Int Int = n => if n > 5 then #done else #yield { item = n; next = n + 1; };\n\
                 add :: Int -> Int -> Int = a b => a + b;\n\
                 s.fold add 0 (s.take 4 (s.unfold step 1))\n",
            ),
        ],
    );
    // unfold step 1 → 1,2,3,4,5; take 4 → 1,2,3,4; fold (+) 0 → 10.
    assert_eq!(native.trim(), "10");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_imported_stream_empty_matches_oracle() {
    // `s.empty` through the import boundary: a polymorphic nullary value crossing
    // the module boundary, then consumed by `s.cons`/`s.fold`. Native == oracle.
    let (interp, native) = import_run_vs_compile(
        "g6_stream_empty",
        "main.zt",
        &[
            ("stream.zt", zutai_hir::STREAM_MODULE_SRC),
            (
                "main.zt",
                "s ::= import \"stream.zt\";\n\
                 add :: Int -> Int -> Int = a b => a + b;\n\
                 s.fold add 0 (s.cons 5 (s.cons 7 s.empty))\n",
            ),
        ],
    );
    assert_eq!(native.trim(), "12");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_imported_stream_list_interop_matches_oracle() {
    // V3-G2 residual through the import boundary: `s.fromList`/`s.toList`/`s.takeList`
    // qualified on the imported module. A builtin `List` round-trips List→Stream→List
    // through the qualified combinators; native compile must match the oracle.
    let (interp, native) = import_run_vs_compile(
        "g6_stream_list_interop",
        "main.zt",
        &[
            ("stream.zt", zutai_hir::STREAM_MODULE_SRC),
            (
                "main.zt",
                "s ::= import \"stream.zt\";\n\
                 double :: Int -> Int = x => x * 2;\n\
                 s.toList (s.take 2 (s.map double (s.fromList {1; 2; 3;})))\n",
            ),
        ],
    );
    // fromList [1,2,3] → map (*2) → 2,4,6; take 2 → 2,4; toList → [2; 4].
    assert_eq!(native.trim(), "[2; 4]");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_stream_combinators_destructure_import_matches_oracle() {
    // Stream `map`/`fold` are no longer ambient because unqualified names belong
    // to the List prelude; destructuring `stdlib.stream` still makes the stream
    // surface available unqualified when explicitly requested.
    let (interp, native) = import_run_vs_compile(
        "g6_stream_destructure_import",
        "main.zt",
        &[(
            "main.zt",
            "{ map; fold; take; unfold; } ::= import stdlib.stream;\n\
             fold (\\acc x. acc + x) 0 (take 3 (map (\\x. x * 2) (unfold (\\st. #yield { item = st; next = st + 1; }) 1)))\n",
        )],
    );
    assert_eq!(native.trim(), "12");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn nonconflicting_ambient_stream_prelude_matches_imported_module() {
    // Stream `cons`/`singleton`/`take` stay ambient; `map`/`fold` are qualified
    // through `stdlib.stream` so unqualified `List` pipeline names remain
    // available in the same scope.
    let ambient = write_tmp(
        "g6_ambient_stream.zt",
        "s ::= import stdlib.stream;\n\
         double :: Int -> Int = x => x * 2;\n\
         add :: Int -> Int -> Int = a b => a + b;\n\
         s.fold add 0 (take 2 (s.map double (cons 1 (cons 2 (singleton 3)))))\n",
    );
    let out = cli()
        .arg("run")
        .arg(&ambient)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    // map (*2) [1,2,3] = [2,4,6]; take 2 = [2,4]; fold (+) 0 = 6.
    assert_eq!(String::from_utf8(out).unwrap().trim(), "6");
}

#[test]
fn compile_zt_imported_unexportable_value_stays_monomorphic() {
    // Only genuine type parameters are generalized across the boundary. A value
    // of an un-exportable type (interned as an unconstrained `Unknown`) must NOT
    // become polymorphic — using it at two incompatible types is a clean type
    // error (monomorphic-by-use), never accepted-and-miscompiled / an ICE.
    let dir = std::env::temp_dir().join("zutai_imp_xm_unexportable");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("dep.zt"),
        "Box :: <A> type { #box : { val : A; }; };\nb :: Box Int = #box { val = 7; };\nb\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.zt"),
        "dep ::= import \"dep.zt\";\ng :: Int -> Bool -> Int = i _ => i;\ng dep dep\n",
    )
    .unwrap();
    let stderr = cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(dir.join("main.zt"))
        .arg("-o")
        .arg(dir.join("out.bin"))
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(
        !stderr.contains("internal compiler error") && !stderr.contains("panicked"),
        "un-exportable import used at two types must be a clean rejection, not an ICE: {stderr}"
    );
}

#[test]
fn compile_zt_imported_unexportable_value_through_generic_matches_oracle() {
    // A value of an un-exportable type (interned as an unconstrained `Unknown`)
    // passed only to a generic that never pins it leaves an opaque use-site type
    // on the dependency global. Under untagged-i64 it is a machine-safe
    // pass-through, so it must compile (matching the interpreter), not ICE.
    let (interp, native) = import_run_vs_compile(
        "xm_unexportable_generic",
        "main.zt",
        &[
            (
                "dep.zt",
                "Box :: <A> type { #box : { val : A; }; };\nb :: Box Int = #box { val = 7; };\nb\n",
            ),
            (
                "main.zt",
                "dep ::= import \"dep.zt\";\nign :: <A> A -> Int = _ => 0;\nign dep\n",
            ),
        ],
    );
    assert_eq!(native.trim(), "0");
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn run_bare_filename_import_parent_escape_is_rejected() {
    let root = std::env::temp_dir();
    let dir = root.join("zutai_cli_bare_base");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(root.join("zutai_cli_bare_escape.zti"), "{ secret = 1; }\n").unwrap();
    std::fs::write(
        dir.join("main.zt"),
        "cfg ::= import \"../zutai_cli_bare_escape.zti\";\ncfg.secret\n",
    )
    .unwrap();

    cli()
        .current_dir(&dir)
        .arg("run")
        .arg("main.zt")
        .assert()
        .failure()
        .stderr(predicate::str::contains("escapes"));
}

#[test]
fn run_imported_function_can_flow_through_print_effect() {
    write_tmp(
        "cli_test_func_import.zt",
        "add :: Int -> Int -> Int\n  = a b => a + b;\nadd\n",
    );
    let path = write_tmp(
        "cli_test_func_print_import.zt",
        "add ::= import \"./cli_test_func_import.zt\";\n[ print \"using import\"; add 2 3 ]\n",
    );
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("using import").and(predicate::str::contains("5")));
}

#[test]
fn compile_zt_value_import_matches_oracle() {
    // A pure `.zt` module exporting a record: native must merge it into one
    // Dataflow Core graph and produce the same output as the interpreter.
    let (interp, native) = import_run_vs_compile(
        "zt_value",
        "main.zt",
        &[
            (
                "dep.zt",
                "base ::= 21;\n{ doubled = base; name = \"svc\"; }\n",
            ),
            ("main.zt", "dep ::= import \"dep.zt\";\ndep\n"),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_int_import_matches_oracle() {
    // A `.zt` module exporting a computed integer: numeric globals cross the
    // module boundary correctly.
    let (interp, native) = import_run_vs_compile(
        "zt_int",
        "main.zt",
        &[
            ("lib.zt", "x ::= 7;\ny ::= 6;\nx * y\n"),
            ("main.zt", "n ::= import \"lib.zt\";\nn + 1\n"),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    assert!(native.trim().contains("43"), "expected 43, got {native:?}");
}

// ─── parse with type/semantic errors ─────────────────────────────────────────

#[test]
fn parse_zt_with_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_parse_type_err.zt", "x :: Int = \"oops\";\nx\n");
    cli()
        .arg("parse")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "type mismatch: expected Int, found Text",
        ));
}

#[test]
fn parse_zt_with_valid_import_prints_ast() {
    // Before fix: analyze(base=None) short-circuits every import to
    // NoBaseDirectory and THIR emits "unsupported feature: imports".
    // After fix: analyze_with_base resolves the import and the AST prints.
    write_tmp(
        "cli_test_parse_import_cfg.zti",
        "{ host = \"127.0.0.1\"; }\n",
    );
    let path = write_tmp(
        "cli_test_parse_import_cfg.zt",
        "cfg ::= import \"./cli_test_parse_import_cfg.zti\";\ncfg.host\n",
    );
    cli()
        .arg("parse")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Parsed"));
}

#[test]
fn parse_zt_with_import_error_surfaces_root_cause() {
    // Before fix: Import diagnostic was filtered out; only the THIR cascade
    // ("unsupported feature: imports") appeared in stderr.
    // After fix: the FileNotFound import diagnostic is included in the filter.
    let path = write_tmp(
        "cli_test_parse_import_err.zt",
        "cfg ::= import \"./does_not_exist_parse.zti\";\ncfg\n",
    );
    cli()
        .arg("parse")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("import error"))
        .stderr(predicate::str::contains("file not found"));
}

// ─── `check` subcommand ────────────────────────────────────────────────────────
const EFFECT_SRC: &str = r#"
Config :: type { value : Text; };
ParseError :: type Text;
parse :: Text -> Config ! { fail ParseError; }
  = text => perform fail text;
parse
"#;

const OPEN_ROW_SELECT_SRC: &str = r#"
getN :: { n : Int; ...; } -> Int = x => x.n;
getN { extra = 7; n = 5; }
"#;

const HANDLED_EFFECT_SRC: &str = r#"
result ::= handle [ perform warn "diag"; "ok" ] with { warn = \d. resume (); };
result
"#;

const COMPILED_EFFECT_FIXTURES: &[(&str, &str)] = &[
    ("handled_warn_resume", HANDLED_EFFECT_SRC),
    (
        "handler_direct_return",
        r#"
result ::= handle [ perform fail "bad"; "unreachable" ] with { fail = \e. "fallback"; };
result
"#,
    ),
    (
        "forwarded_handler",
        r#"
result ::= handle [
  handle [ perform fail "bad"; "unreachable" ] with {
    fail = \e. [ perform log e; "fallback" ];
  }
] with {
  log = \d. resume ();
};
result
"#,
    ),
    (
        "multi_op_nested_handlers",
        r#"
result ::= handle [
  handle [ perform inner "x"; perform outer "y"; perform note "z"; "ok" ] with {
    inner = \d. resume ();
    note = \d. resume ();
  }
] with {
  outer = \d. resume ();
};
result
"#,
    ),
    (
        "source_handler_intercepts_print",
        r#"
result ::= handle print "x" with { io.print = \text. "handled"; };
result
"#,
    ),
    ("ambient_print", "print \"hello\"\n"),
    (
        "print_sequence",
        r#"[ perform io.print "a"; perform io.print "b"; 7 ]
"#,
    ),
    (
        "branch_print",
        r#"if 1 < 2 then print "then" else print "else"
"#,
    ),
    ("print_list", r#"{ print "a"; print "b"; }"#),
    (
        "print_function",
        r#"
printer :: Text -> Text ! { io.print : Text -> Text; }
  = t => print t;
printer "fn"
"#,
    ),
    (
        "higher_order_print",
        r#"
apply :: (Text -> Text ! { io.print : Text -> Text; }) -> Text ! { io.print : Text -> Text; }
  = f => f "ho";
apply print
"#,
    ),
    (
        "cross_fn_fail_handled",
        r#"
boom :: Int -> Int ! { fail Text; } = n => perform fail "no";
handle boom 1 with { value = \v. v; fail = \m. 0; }
"#,
    ),
    (
        "cross_fn_curried_handled",
        r#"
addperf :: Int -> Int -> Int ! { fail Text; }
  = a b => [ perform fail "x"; a + b ];
handle addperf 3 4 with { value = \v. v; fail = \m. 99; }
"#,
    ),
    (
        "cross_fn_resume",
        r#"
g :: Int -> Int ! { op : Int -> Int; } = n => perform op n;
handle g 1 with { value = \v. v; op = \v. resume (v + 1); }
"#,
    ),
    (
        "cross_fn_arg_effect_order",
        r#"
g :: Int -> Int ! { fail Text; } = n => perform fail "x";
handle g (perform ask ()) with {
  value = \v. v;
  ask = \m. resume 0;
  fail = \m. 7;
}
"#,
    ),
    (
        "cross_fn_chain",
        r#"
inner :: Int -> Int ! { fail Text; } = n => perform fail "z";
outer :: Int -> Int ! { fail Text; } = n => inner (n + 1);
handle outer 5 with { value = \v. v; fail = \m. 42; }
"#,
    ),
    (
        "cross_fn_two_call_sites",
        r#"
g :: Int -> Int ! { op : Int -> Int; } = n => perform op n;
handle (g 1) + (g 2) with { value = \v. v; op = \v. resume v; }
"#,
    ),
    (
        "unused_effectful_decl",
        r#"
boom :: Int -> Int ! { fail Text; } = n => perform fail "no";
42
"#,
    ),
];
const COMPILED_WITNESS_FIXTURES: &[(&str, &str)] = &[
    (
        "two_method_sorted_slot",
        r#"
Ord :: <A> @A { lt :: A -> A -> Bool; gt :: A -> A -> Bool; }
Ord @Int :: { lt = \a b. a < b; gt = \a b. a > b; }
lt 1 2
"#,
    ),
    (
        "derive_eq_record",
        r#"
Point :: type { x : Int; y : Int; };
p1 :: Point = { x = 1; y = 2; };
p2 :: Point = { x = 9; y = 2; };
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Point :: derive
eq p1 p2
"#,
    ),
    (
        "conditional_list_witness",
        r#"
Eq :: <A> @A { eq :: A -> A -> Bool; } derive
Eq @Int :: { eq = \a b. a == b; }
Eq @(List A) :: <A: Eq> { eq = \xs ys. true; }
eq { 1; 2; } { 3; 4; }
"#,
    ),
    (
        "method_level_type_param",
        r#"
Conv :: <A> @A { conv :: <B> A -> B -> A; }
Conv @Int :: { conv = \a b. a; }
useConv :: <A: Conv> A -> A = x => conv x 0;
useConv 5
"#,
    ),
    (
        "conditional_pair_witness",
        r#"
Pair :: <A> type { fst : A; snd : A; };
Eq :: <A> @A { eq :: A -> A -> Bool; }
Eq @Int :: { eq = \a b. a == b; }
Eq @(Pair A) :: <A: Eq> { eq = \p q. eq p.fst q.fst; }
p1 :: Pair Int = { fst = 1; snd = 2; };
p2 :: Pair Int = { fst = 1; snd = 2; };
eq p1 p2
"#,
    ),
];
const COMPILED_SHOW_FIXTURES: &[(&str, &str)] = &[
    (
        "nullary_union",
        "Tree :: type { #leaf; #node : { val : Int; left : Tree; right : Tree; }; };\n#leaf\n",
    ),
    (
        "enum_member",
        "Color :: type { #red; #green; #blue; };\n#green\n",
    ),
    ("maybe_present", "x :: Maybe Int = #present (42);\nx\n"),
    ("maybe_absent", "x :: Maybe Int = #absent;\nx\n"),
    ("optional_some", "x :: Int? = #some (7);\nx\n"),
    ("optional_none", "x :: Int? = #none;\nx\n"),
    (
        "record_optional_field",
        "r :: { x : Int?; y : Int; } = { x = #some (42); y = 10; };\nr\n",
    ),
    (
        "recursive_maybe",
        "Nested :: type { next : Maybe Nested; val : Int; };\nmkNested :: Int -> Nested\n  = n => if n == 0 then { next = #absent; val = 0; } else { next = #present (mkNested (n - 1)); val = n; };\nmkNested 2\n",
    ),
    ("float_value", "x :: Float = 5.0;\nx\n"),
    ("coalesce_some", "x :: Int? = #some (7);\nx ?? 0\n"),
    ("coalesce_none", "y :: Int? = #none;\ny ?? 99\n"),
    // ── value-rendering divergence guards (docs/TBD.md) ──────────────────────
    // The backend sorts record fields by name for slot layout; the interpreter
    // must render the same order. These exercise the shapes a backend/interp
    // rendering divergence would hide: non-alphabetical records (flat + nested),
    // user-union variants, nested tuples, text escaping, and negative integers.
    ("nonalpha_record", "{ zebra = 1; apple = 2; mango = 3; }\n"),
    (
        "nested_nonalpha_record",
        "{ outer_z = { inner_z = 1; inner_a = 2; }; outer_a = 3; }\n",
    ),
    (
        "variant_record_payload",
        "Shape :: type { #square : { side : Int; }; #circle : { radius : Int; }; };\ns :: Shape = #circle { radius = 5; };\ns\n",
    ),
    (
        "variant_in_record",
        "Status :: type { #ok; #err; };\nr :: { tag : Status; n : Int; } = { tag = #err; n = 3; };\nr\n",
    ),
    ("nested_tuple", "(1, (2, (3, 4)))\n"),
    ("tuple_in_record", "{ pos = (1, 2); name = \"x\"; }\n"),
    ("text_escapes", "\"x\\ty\\nz\\\"w\\\\v\"\n"),
    ("negative_in_record", "{ b = 0 - 3; a = 7; }\n"),
    ("negatives_in_list", "{ 0 - 1; 0 - 2; 3; }\n"),
];

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
    let path = write_tmp("cli_test_check_parse_err.zt", "{1; 2}\n");
    cli().arg("check").arg(&path).assert().failure();
}

#[test]
fn check_zt_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_check_type_err.zt", "x :: Int = \"bad\";\nx\n");
    cli().arg("check").arg(&path).assert().failure();
}

#[test]
fn check_renders_human_readable_type_diagnostic() {
    // `check` must render THIR type errors with a human message and source
    // context (like parse errors), not dump the raw `SemanticDiagnostic { .. }`
    // Debug form. The exact human string is absent from the old Debug output.
    let path = write_tmp("cli_test_check_render.zt", "x :: Int = \"bad\";\nx\n");
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
    let path = write_tmp("cli_test_compile_parse_err.zt", "{1; 2}\n");
    cli().arg("compile").arg(&path).assert().failure();
}

#[test]
fn compile_zt_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_compile_type_err.zt", "x :: Int = \"bad\";\nx\n");
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
fn compiled_recursive_effectful_fn_matches_oracle() {
    // E1: a self-recursive effectful function cannot be inlined, but the reify
    // pass lowers the handle to a generated free-monad driver that compiles
    // natively and matches the interpreter. Here the `fail` handler aborts
    // without resuming (no finite unfolding needed), yielding 0.
    let src = r#"
loop :: Int -> Int ! { fail Text; }
  = n => if n < 1 then perform fail "z" else loop (n - 1);
handle loop 3 with { value = \v. v; fail = \m. 0; }
"#;
    let run_output = run_stdout("cli_test_compile_rec_effect.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_compile_rec_effect", src);
    assert_eq!(run_output, "0\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled recursive effectful fn must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_higher_order_effectful_value_matches_oracle() {
    // E2: an effectful function passed as a value. The reify pass rewrites the
    // effectful parameter's type to its `Computation` form and treats `f 1` as a
    // reified call, so this compiles natively and matches the interpreter. The
    // `fail` handler aborts without resuming, yielding 0.
    let src = r#"
g :: Int -> Int ! { fail Text; } = n => perform fail "x";
apply :: (Int -> Int ! { fail Text; }) -> Int ! { fail Text; } = f => f 1;
handle apply g with { value = \v. v; fail = \m. 0; }
"#;
    let run_output = run_stdout("cli_test_compile_ho_effect.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_compile_ho_effect", src);
    assert_eq!(run_output, "0\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled higher-order effectful value must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_partial_application_effect_matches_oracle() {
    // E4: a partially-applied effectful function. `addP = add 10` has no leading
    // lambdas, so the reify pass eta-expands it to full value arity before
    // lowering. `addP 5` completes the call: x = op 10 (resume 10), 10 + 5 + 10 = 25.
    let src = concat!(
        "add :: Int -> Int -> Int ! { op : Int -> Int; }\n",
        "  = a b => [ x := perform op a; a + b + x ];\n",
        "addP ::= add 10;\n",
        "handle [ addP 5 ] with { op = \\v. resume v; }\n",
    );
    let run_output = run_stdout("cli_test_partial_effect_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_partial_effect_compiled", src);
    assert_eq!(run_output, "25\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled partial-application effect must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_inline_partial_application_effect_matches_oracle() {
    // E4 follow-up: an *inline* partially-applied effectful function passed as a
    // higher-order argument (`applyTo (addP 5)`). The named-binding form
    // (`applyTo addP`) already compiled; the reify pass now eta-expands the inline
    // partial application to a lambda value and reifies its body at the call site,
    // so this matches the oracle. `x = op 5 = 5`, `a + b + x = 5 + 11 + 5 = 21`.
    let src = concat!(
        "addP :: Int -> Int -> Int ! { op : Int -> Int; }\n",
        "  = a b => [ x := perform op a; a + b + x ];\n",
        "applyTo :: (Int -> Int ! { op : Int -> Int; }) -> Int ! { op : Int -> Int; }\n",
        "  = f => f 11;\n",
        "handle applyTo (addP 5) with { op = \\v. resume v; }\n",
    );
    let run_output = run_stdout("cli_test_inline_partial_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_inline_partial_compiled", src);
    assert_eq!(run_output, "21\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled inline partial-application effect must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_record_field_effectful_call_matches_oracle() {
    // An effectful function stored in a record field, then projected and called
    // (`box.f 7`). The reify pass discovers the callee through the record wrapper,
    // rewrites the wrapper's field type to `… -> Computation`, and reifies the
    // `GetField`-headed call. `x = op 7 = 14`, `n + x = 7 + 14 = 21`.
    let src = concat!(
        "g :: Int -> Int ! { op : Int -> Int; } = n => [ x := perform op n; n + x ];\n",
        "box :: { f : Int -> Int ! { op : Int -> Int; }; } = { f = g; };\n",
        "handle box.f 7 with { op = \\v. resume (v * 2); }\n",
    );
    let run_output = run_stdout("cli_test_record_field_effect_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_record_field_effect_compiled", src);
    assert_eq!(run_output, "21\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled record-field effectful call must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_record_field_partial_application_effect_matches_oracle() {
    // The two new paths composed: a record-field projection that is *also* an inline
    // partial application passed as a higher-order argument (`applyTo (box.f 5)`).
    // `x = op 5 = 5`, `a + b + x = 5 + 11 + 5 = 21`.
    let src = concat!(
        "addP :: Int -> Int -> Int ! { op : Int -> Int; }\n",
        "  = a b => [ x := perform op a; a + b + x ];\n",
        "box :: { f : Int -> Int -> Int ! { op : Int -> Int; }; } = { f = addP; };\n",
        "applyTo :: (Int -> Int ! { op : Int -> Int; }) -> Int ! { op : Int -> Int; }\n",
        "  = k => k 11;\n",
        "handle applyTo (box.f 5) with { op = \\v. resume v; }\n",
    );
    let run_output = run_stdout("cli_test_record_partial_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_record_partial_compiled", src);
    assert_eq!(run_output, "21\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled record-field partial application must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_effectful_operand_recursion_matches_oracle() {
    // E5 (reify generalization): a recursive effectful call used as a *builtin
    // operand* (`x + total (n - 1)`), not just in tail position. The reify pass
    // composes it through `bind`, left-to-right. total 3 = (3+1)+(2+1)+(1+1) = 9.
    let src = concat!(
        "total :: Int -> Int ! { op : Int -> Int; }\n",
        "  = 0 => 0;\n",
        "  = n => [ x := perform op n; x + total (n - 1) ];\n",
        "handle total 3 with { op = \\v. resume (v + 1); }\n",
    );
    let run_output = run_stdout("cli_test_eff_operand_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_eff_operand_compiled", src);
    assert_eq!(run_output, "9\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled effectful-operand recursion must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_recursive_higher_order_effect_matches_oracle() {
    // E2: a recursive higher-order function — `applyN` takes an effectful callback
    // `f` and recurses, so it is reified (not inlined) with `f`'s type rewritten to
    // `Int -> Computation`. Each `f n` resumes with `v + 1`, accumulated across the
    // recursion: applyN g 3 0 = (3+1)+(2+1)+(1+1)+0 = 9.
    let src = concat!(
        "g :: Int -> Int ! { op : Int -> Int; } = n => perform op n;\n",
        "applyN :: (Int -> Int ! { op : Int -> Int; }) -> Int -> Int -> Int ! { op : Int -> Int; }\n",
        "  = f 0 acc => acc;\n",
        "  = f n acc => [ x := f n; applyN f (n - 1) (acc + x) ];\n",
        "handle applyN g 3 0 with { op = \\v. resume (v + 1); }\n",
    );
    let run_output = run_stdout("cli_test_ho_rec_effect_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_ho_rec_effect_compiled", src);
    assert_eq!(run_output, "9\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled recursive higher-order effect must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_effectful_generator_matches_oracle() {
    // V3-G4: an effectful generator (`stream { yield perform … }`) consumed
    // strictly under a handler. The deferred `perform` in the cell's `head` field
    // is reified into `Computation`-data (carrier on the field, not the demand
    // thunk, so `Computation` stays monomorphic); the consumer `bind`s it. One
    // yield resumes 5, summed to 5.
    let one = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
sumEff :: (Unit -> Cell) -> Int ! { tick : Unit -> Int; }
  = s => match s () { | #nil => 0; | #cons { head = h; tail = t; } => h + sumEff t; };
handle (sumEff (stream { yield perform tick (); })) with { tick = \_. resume 5; }
"#;
    assert_eq!(run_stdout("cli_test_eff_gen_one.zt", one), "5\n");
    assert_eq!(compile_bin_stdout("cli_test_eff_gen_one", one), "5\n");

    // Two yields → 10; the second cell's effect fires only when the consumer
    // forces its tail thunk, preserving demand order.
    let two = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
sumEff :: (Unit -> Cell) -> Int ! { tick : Unit -> Int; }
  = s => match s () { | #nil => 0; | #cons { head = h; tail = t; } => h + sumEff t; };
handle (sumEff (stream { yield perform tick (); yield perform tick (); })) with { tick = \_. resume 5; }
"#;
    assert_eq!(
        compile_bin_stdout("cli_test_eff_gen_two", two),
        run_stdout("cli_test_eff_gen_two.zt", two),
    );
}

#[test]
fn compiled_effectful_generator_ordering_matches_oracle() {
    // V3-G4: three yields with distinct resume values lock left-to-right demand
    // order and resume-value threading: tick v resumes v*10, so 1+2+3 yields
    // 10+20+30 = 60.
    let src = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
sumEff :: (Unit -> Cell) -> Int ! { tick : Int -> Int; }
  = s => match s () { | #nil => 0; | #cons { head = h; tail = t; } => h + sumEff t; };
handle (sumEff (stream { yield perform tick 1; yield perform tick 2; yield perform tick 3; })) with { tick = \v. resume (v * 10); }
"#;
    assert_eq!(run_stdout("cli_test_eff_gen_ord.zt", src), "60\n");
    assert_eq!(compile_bin_stdout("cli_test_eff_gen_ord", src), "60\n");
}

#[test]
fn compile_resource_effectful_generator_stays_gated() {
    // Resource-backed generator cells are interpreter-only: even with an explicit
    // source handler granting `fs.read`, native lowering must refuse the deferred
    // non-`io.print` host operation instead of reifying it into a backend stream.
    let data_path = write_tmp("cli_test_resource_gen_data.txt", "mock");
    let src = format!(
        r#"
Cell :: type {{ #nil; #cons : {{ head : Text; tail : Unit -> Cell; }}; }};
first :: (Unit -> Cell) -> Text ! {{ fs.read : Path -> Text; }}
  = s => match s () {{ | #nil => "empty"; | #cons {{ head = h; tail = t; }} => h; }};
handle (first (stream {{ yield perform fs.read "{}"; }})) with {{ fs.read = \path. resume "mock"; }}
"#,
        zt_string_literal(&data_path)
    );
    assert_eq!(
        run_stdout("cli_test_resource_gen_oracle.zt", &src),
        "\"mock\"\n"
    );
    let path = write_tmp("cli_test_resource_gen_compile_gated.zt", &src);
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("effect"));
}

#[test]
fn resource_generator_lazy_escape_is_rejected_at_force_boundary() {
    // Returning the unforced effectful head from inside the granting handler does
    // not transfer the resource lifetime. Displaying the escaped value forces it
    // after the handler is gone, so the effect is refused as unhandled.
    let src = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
firstLazy :: (Unit -> Cell) -> Int ! { tick : Unit -> Int; }
  = s => match s () { | #nil => 0; | #cons { head = h; tail = t; } => h; };
handle (firstLazy (stream { yield perform tick (); })) with { tick = \_. resume 5; }
"#;
    let path = write_tmp("cli_test_resource_lazy_escape.zt", src);
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("unhandled effect `tick`"));
}

#[test]
fn resource_generator_finalizes_once_on_legal_shapes() {
    let full = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
sumEff :: (Unit -> Cell) -> Int ! { tick : Unit -> Int; }
  = s => match s () { | #nil => 0; | #cons { head = h; tail = t; } => h + sumEff t; };
handle (sumEff (stream { yield perform tick (); yield perform tick (); })) with {
  tick = \_. resume 5;
  finally = print "close";
}
"#;
    assert_eq!(
        run_stdout("cli_test_resource_full_once.zt", full),
        "close\n10\n"
    );

    let partial = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
take2 :: (Unit -> Cell) -> Int ! { tick : Unit -> Int; }
  = s => match s () {
    | #nil => 0;
    | #cons { head = h; tail = t; } => h + (match t () { | #nil => 0; | #cons { head = h2; tail = u; } => h2; });
  };
handle (take2 (stream { yield perform tick (); yield perform tick (); yield perform tick (); })) with {
  tick = \_. resume 5;
  finally = print "close";
}
"#;
    assert_eq!(
        run_stdout("cli_test_resource_partial_once.zt", partial),
        "close\n10\n"
    );

    let cancel = r#"
Cell :: type { #nil; #cons : { head : Int; tail : Unit -> Cell; }; };
foldUntil :: Int -> (Unit -> Cell) -> Int ! { tick : Unit -> Int; stop : Int -> Int; }
  = acc s => match s () {
    | #nil => acc;
    | #cons { head = h; tail = t; } => (if acc + h > 7 then perform stop (acc + h) else foldUntil (acc + h) t);
  };
handle (foldUntil 0 (stream { yield perform tick (); yield perform tick (); yield perform tick (); })) with {
  tick = \_. resume 5;
  stop = \r. r;
  finally = print "close";
}
"#;
    assert_eq!(
        run_stdout("cli_test_resource_cancel_once.zt", cancel),
        "close\n10\n"
    );

    let nested = r#"
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
      finally = print "inner";
    }
  ) with {
    finally = print "outer";
  }
) with {
  stop = \r. r;
}
"#;
    assert_eq!(
        run_stdout("cli_test_resource_nested_once.zt", nested),
        "inner\nouter\n10\n"
    );
}
#[test]
fn compiled_finally_clause_matches_oracle() {
    // E5: a `finally` teardown clause. The reify pipeline desugars it to
    // `let r = (handle … without finally) in [ teardown; r ]`, so the teardown
    // runs in the outer row after the handle reduces and the result is preserved.
    // A pure teardown is discarded; the handle still yields "body".
    let src = r#"
main ::= handle "body" with { finally = "cleanup"; };
main
"#;
    let run_output = run_stdout("cli_test_compile_finally.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_compile_finally", src);
    assert_eq!(run_output, "\"body\"\n");
    assert_eq!(
        compiled_output, run_output,
        "pure finally must match oracle"
    );

    // The teardown runs on normal completion, performing into the outer handler
    // (which aborts to a sentinel — observing it proves finalization ran).
    let normal = r#"
result ::= handle (handle "inner" with { finally = perform mark (); }) with { mark = \_. "finalized"; };
result
"#;
    assert_eq!(
        compile_bin_stdout("cli_test_finally_normal", normal),
        run_stdout("cli_test_finally_normal.zt", normal),
    );

    // The teardown also runs on the handler-abort path (inner `fail` returns
    // without resuming) and still performs its outer effect.
    let abort = r#"
result ::= handle (handle perform fail "x" with { fail = \e. "fallback"; finally = perform mark (); }) with { mark = \_. "finalized"; };
result
"#;
    assert_eq!(
        compile_bin_stdout("cli_test_finally_abort", abort),
        run_stdout("cli_test_finally_abort.zt", abort),
    );
}

#[test]
fn compile_open_row_select_lowers_to_llvm() {
    // Phase C: an open-row field select is monomorphized at the concrete call site
    // (the field's slot is recomputed for the concrete record layout), so it now
    // lowers to LLVM instead of being gated.
    let llvm = compile_stdout("cli_test_compile_open_row.zt", OPEN_ROW_SELECT_SRC);
    assert!(llvm.contains("define i64 @__entry"), "{llvm}");
}

#[test]
fn compile_bin_open_row_select_matches_oracle() {
    // Phase C parity: the native binary reads the correct field (slot recomputed
    // for the concrete `{ extra; n }` layout) and matches the interpreter oracle.
    let native = compile_bin_stdout("cli_test_compile_bin_open_row", OPEN_ROW_SELECT_SRC);
    let interp = run_stdout("cli_test_run_open_row_oracle.zt", OPEN_ROW_SELECT_SRC);
    assert_eq!(native.trim(), "5");
    assert_eq!(
        native.trim(),
        interp.trim(),
        "native must match the interpreter oracle"
    );
}

#[test]
fn run_open_row_select_evaluates_correctly() {
    // The interpreter resolves fields by name and handles open records soundly.
    let output = run_stdout("cli_test_run_open_row.zt", OPEN_ROW_SELECT_SRC);
    assert_eq!(output.trim(), "5");
}

#[test]
fn compile_open_row_select_discriminates_slot_per_concrete_record() {
    // Phase C: each concrete call site recomputes the field's slot for its own
    // record. `getN` reads `n` from two records with different sibling fields
    // (`{a;n}` → n at slot 1, `{m;n;z}` → n at slot 1, never the slot-0 sibling).
    // A wrong (view-derived slot 0) read would return `a`/`m` instead of `n`.
    let src = "getN :: { n : Int; ...; } -> Int = x => x.n;\n\
               (getN { a = 1; n = 2; }, getN { m = 3; z = 4; n = 9; })\n";
    let native = compile_bin_stdout("cli_test_open_row_disc", src);
    let interp = run_stdout("cli_test_open_row_disc_oracle.zt", src);
    assert_eq!(native.trim(), interp.trim(), "native must match the oracle");
    assert!(
        native.contains('2') && native.contains('9'),
        "expected (2, 9), got {native:?}"
    );
}

#[test]
fn compile_unspecializable_open_row_select_stays_gated() {
    // Soundness: an open-row select that cannot be monomorphized to a concrete
    // record (here `mid` is applied to `top`'s still-open parameter) is left
    // gated rather than miscompiled. The interpreter still runs it.
    let src = "mid :: { n : Int; ...; } -> Int = x => x.n;\n\
               top :: { n : Int; ...; } -> Int = y => mid y;\n\
               top { extra = 7; n = 5; }\n";
    let path = write_tmp("cli_test_open_row_gated.zt", src);
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("open record row"));
    assert_eq!(
        run_stdout("cli_test_open_row_gated_run.zt", src).trim(),
        "5"
    );
}

#[test]
fn compile_recursive_open_row_param_function_matches_oracle() {
    // Regression: a recursive function with an open-row PARAMETER that does not
    // select a field from it must not be monomorphized (clone_expr reuses binder
    // ids; inlining a concrete self-call would nest a clone that removes a
    // still-live binding). It compiles unchanged and matches the oracle.
    let src = "f :: { n : Int; ...; } -> Int -> Int \
               = r k => if k < 1 then 0 else (f { n = 0; } (k - 1)) + k;\n\
               f { n = 7; } 3\n";
    let native = compile_bin_stdout("cli_test_rec_open_row_param", src);
    let interp = run_stdout("cli_test_rec_open_row_param_oracle.zt", src);
    assert_eq!(native.trim(), "6");
    assert_eq!(native.trim(), interp.trim(), "native must match the oracle");
}

#[test]
fn compiled_effect_fixtures_match_eval_tlc_oracle() {
    for (name, source) in COMPILED_EFFECT_FIXTURES {
        let run_output = run_stdout(&format!("cli_test_effect_oracle_{name}.zt"), source);
        let compiled_output =
            compile_bin_stdout(&format!("cli_test_effect_compiled_{name}"), source);
        assert_eq!(
            compiled_output, run_output,
            "compiled output must match eval_tlc oracle for {name}"
        );
    }
}

#[test]
fn compiled_witness_fixtures_match_eval_tlc_oracle() {
    for (name, source) in COMPILED_WITNESS_FIXTURES {
        let run_output = run_stdout(&format!("cli_test_witness_oracle_{name}.zt"), source);
        let compiled_output =
            compile_bin_stdout(&format!("cli_test_witness_compiled_{name}"), source);
        assert_eq!(
            compiled_output, run_output,
            "compiled output must match eval_tlc oracle for {name}"
        );
    }
}

#[test]
fn compiled_show_fixtures_match_eval_tlc_oracle() {
    for (name, source) in COMPILED_SHOW_FIXTURES {
        let run_output = run_stdout(&format!("cli_test_show_oracle_{name}.zt"), source);
        let compiled_output = compile_bin_stdout(&format!("cli_test_show_compiled_{name}"), source);
        assert_eq!(
            compiled_output, run_output,
            "compiled output must match eval_tlc oracle for {name}"
        );
    }
}

#[test]
fn compiled_polymorphic_and_nested_match_fixtures_match_oracle() {
    // Guards two backend regressions. `nested_match` destructures record fields at
    // the same slot across several match arms in one function; the SSA lowerer used
    // to name those temporaries by slot, colliding on `%__rec_0` and failing `llc`.
    // `large_program`'s polymorphic `constFn`/`compose`/`flip` curry over distinct
    // type variables; TLC used to type every curried lambda layer with the full
    // signature, handing an inner lambda a param type from the wrong position and
    // tripping the Dataflow structural validator. Both produced invalid output
    // before the fixes; here the compiled output must match the interpreter oracle.
    for name in ["nested_match.zt", "large_program.zt"] {
        let source = std::fs::read_to_string(general_fixture(name))
            .unwrap_or_else(|e| panic!("read {name}: {e}"));
        let run_output = run_stdout(&format!("cli_test_fixture_oracle_{name}"), &source);
        let compiled_output =
            compile_bin_stdout(&format!("cli_test_fixture_compiled_{name}"), &source);
        assert_eq!(
            compiled_output, run_output,
            "compiled output must match eval_tlc oracle for {name}"
        );
    }
}

#[test]
fn compiled_rank2_lambda_arg_matches_oracle() {
    // A lambda argument checked against a rank-2 type (`<A> A -> A`) used to
    // abort backend compilation: TLC typed the value lambda layer with the full
    // `∀`-type, so the Dataflow structural validator found a non-`Fun` where a
    // `Lam` node requires one and panicked with an ICE. The compiled output must
    // now lower cleanly and match the interpreter oracle.
    let src = "apply :: (<A> A -> A) -> Int = \\g. g 1;\napply (\\x. x)\n";
    let run_output = run_stdout("cli_test_rank2_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_rank2_compiled", src);
    assert_eq!(
        compiled_output, run_output,
        "compiled output must match eval_tlc oracle for the rank-2 lambda argument"
    );
}
#[test]
fn rsa_fixture_runs_and_emits_llvm_pipeline() {
    let source =
        std::fs::read_to_string(general_fixture("rsa_roundtrip.zt")).expect("read RSA fixture");
    let run_output = run_stdout("cli_test_rsa_oracle.zt", &source);
    assert!(run_output.contains("modulus = 3233"), "{run_output}");
    assert!(
        run_output.contains("private_exponent = 2753"),
        "{run_output}"
    );
    assert!(run_output.contains("cipher = 2790"), "{run_output}");
    assert!(run_output.contains("decrypted = 65"), "{run_output}");
    assert!(run_output.contains("score = 5608"), "{run_output}");
    assert!(run_output.contains("verdict = #ok"), "{run_output}");

    let llvm = compile_stdout("cli_test_rsa_compiled.zt", &source);
    assert!(llvm.contains("define i64 @__entry"), "{llvm}");
    assert!(llvm.contains("call void @zutai.show"), "{llvm}");
    assert!(llvm.contains("define i64 @verdict()"), "{llvm}");
}

#[test]
fn compile_nullary_variant_bin_renders_tag() {
    assert_eq!(
        compile_bin_stdout(
            "cli_test_show_leaf",
            "Tree :: type { #leaf; #node : { val : Int; left : Tree; right : Tree; }; };\n#leaf\n",
        ),
        "#leaf\n"
    );
}

#[test]
fn compile_maybe_present_bin_renders_payload() {
    assert_eq!(
        compile_bin_stdout(
            "cli_test_show_present",
            "x :: Maybe Int = #present (42);\nx\n",
        ),
        "#present (42)\n"
    );
}

// Phase 35 (escaping-effect residual-ABI spike): a reified free-monad value —
// a recursive union whose operation arm carries `resume : Int -> Free` (a
// function field pointing back at the recursive type) — lowers over the cyclic
// `DfTyId` machinery (Phase 25) all the way to native and matches the oracle.
// This is the hand-defunctionalized equivalent of a recursive/self-tail
// effectful callee, which `compile` still rejects directly (see
// `compile_rejects_recursive_effectful_callee`). It confirms the encoding the
// spike costed; it does NOT add automatic `perform`/`handle` elaboration.
const FREE_MONAD_SPINE_SRC: &str = r#"
Free :: type {
  #pure : { value : Int; };
  #ask  : { payload : Int; resume : Int -> Free; };
};

go :: Int -> Int -> Free
  = 0 acc => #pure { value = acc; };
  = n acc => #ask { payload = n; resume = \k. go (n - 1) (acc + k); };

run :: Free -> Int
  = #pure { value = v; }               => v;
  = #ask { payload = p; resume = r; }  => run (r (p * 10));

run (go 10 0)
"#;

#[test]
fn compiled_free_monad_spine_matches_oracle() {
    // The resumed value (`p * 10`) must thread back through the stored
    // `resume : Int -> Free` closure into the accumulator, so this exercises a
    // genuine boxed-closure call across a fold of an unbounded perform spine —
    // not a constant-folded tree. Both paths must yield 550 = (10+9+...+1)*10.
    let run_output = run_stdout("cli_test_free_monad_oracle.zt", FREE_MONAD_SPINE_SRC);
    let compiled_output = compile_bin_stdout("cli_test_free_monad_compiled", FREE_MONAD_SPINE_SRC);
    assert_eq!(run_output, "550\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled free-monad perform spine must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_recursive_effectful_callee_matches_oracle() {
    // E1: a recursive, self-tail effectful callee — the category the free-monad
    // encoding above reifies by hand — now lowers automatically. The reify pass
    // rewrites `go` to a `Computation`-returning function and drives it with a
    // generated `run`/`bind`. `go` accumulates the payload and resumes with unit;
    // `go 10 0` performs `warn` ten times and returns 10+9+...+1 = 55, threading
    // each resumed unit back through the stored continuation.
    let src = concat!(
        "go :: Int -> Int -> Int ! { warn Int; }\n",
        "  = 0 acc => acc;\n",
        "  = n acc => [ perform warn n; go (n - 1) (acc + n) ];\n",
        "result ::= handle [ go 10 0 ] with { warn = \\w. resume (); };\n",
        "result\n",
    );
    let run_output = run_stdout("cli_test_rec_effect_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_rec_effect_compiled", src);
    assert_eq!(run_output, "55\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled recursive effectful callee must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_mutually_recursive_effects_match_oracle() {
    // E1: mutual recursion across the effectful-call graph. Both `ping` and `pong`
    // perform `warn` and tail-call each other; the reify pass rewrites both to
    // `Computation` form under one handler. `ping 6` performs six times and
    // returns 0.
    let src = concat!(
        "ping :: Int -> Int ! { warn Int; }\n",
        "  = 0 => 0;\n",
        "  = n => [ perform warn n; pong (n - 1) ];\n",
        "pong :: Int -> Int ! { warn Int; }\n",
        "  = 0 => 0;\n",
        "  = n => [ perform warn n; ping (n - 1) ];\n",
        "result ::= handle [ ping 6 ] with { warn = \\w. resume (); };\n",
        "result\n",
    );
    let run_output = run_stdout("cli_test_mutual_effect_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_mutual_effect_compiled", src);
    assert_eq!(run_output, "0\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled mutually-recursive effects must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_mutually_recursive_resume_value_matches_oracle() {
    // E1 + free-monad value threading: a mutually-recursive effectful pair that
    // *consumes* the resumed value and uses the recursive effectful call as a
    // builtin operand (`x + odd (n - 1)`). This stresses the CPS rewrite of a
    // mutual group (commit 4d65e1f) together with the reify `bind` carrying an
    // Int-valued continuation across the mutual edge (commit a2eb04f) — the
    // existing mutual test only resumes with unit. `op n` resumes `n * 2`:
    //   even 4 = 8 + odd 3, odd 3 = 6 + even 2, even 2 = 4 + odd 1,
    //   odd 1 = 2 + even 0, even 0 = 0  ⇒  2,6,12,20  ⇒  20.
    let src = concat!(
        "even :: Int -> Int ! { op : Int -> Int; }\n",
        "  = 0 => 0;\n",
        "  = n => [ x := perform op n; x + odd (n - 1) ];\n",
        "odd :: Int -> Int ! { op : Int -> Int; }\n",
        "  = 0 => 0;\n",
        "  = n => [ x := perform op n; x + even (n - 1) ];\n",
        "handle even 4 with { op = \\v. resume (v * 2); }\n",
    );
    let run_output = run_stdout("cli_test_mutual_resume_val_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_mutual_resume_val_compiled", src);
    assert_eq!(run_output, "20\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled mutually-recursive resume-value must match the eval_tlc oracle"
    );
}

#[test]
fn compiled_recursive_resume_value_matches_oracle() {
    // E1: a recursive effectful callee that *consumes* the resumed value (not just
    // unit). `op : Int -> Int` so each `perform op n` yields the handler's
    // `resume (v + 1)`, threaded into the accumulator across the recursion. The
    // generated `bind` must carry each resumed Int back through the continuation:
    // sum 3 0 = (3+1) + (2+1) + (1+1) + 0 = 9.
    let src = concat!(
        "sum :: Int -> Int -> Int ! { op : Int -> Int; }\n",
        "  = 0 acc => acc;\n",
        "  = n acc => [ x := perform op n; sum (n - 1) (acc + x) ];\n",
        "handle sum 3 0 with { value = \\v. v; op = \\v. resume (v + 1); }\n",
    );
    let run_output = run_stdout("cli_test_resume_val_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_resume_val_compiled", src);
    assert_eq!(run_output, "9\n");
    assert_eq!(
        compiled_output, run_output,
        "compiled recursive resume-value must match the eval_tlc oracle"
    );
}

#[test]
fn compile_handled_effect_program_uses_runtime_pipeline() {
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
fn compile_handled_effect_record_round_trips_runtime_pipeline() {
    let path = write_tmp(
        "cli_test_compile_handled_effect_record.zt",
        r#"
result ::= handle [ perform warn "diag"; { a = 1; b = 2; } ] with { warn = \d. resume (); };
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
fn compile_multi_op_and_nested_handlers_emit_runtime_value() {
    let llvm = compile_stdout(
        "cli_test_compile_nested_handlers.zt",
        r#"
result ::= handle [
  handle [ perform inner "x"; perform outer "y"; perform note "z"; "ok" ] with {
    inner = \d. resume ();
    note = \d. resume ();
  }
] with {
  outer = \d. resume ();
};
result
"#,
    );
    assert!(llvm.contains("ok"), "{llvm}");
}

#[test]
fn compile_handler_clause_forwarding_emits_direct_return() {
    let llvm = compile_stdout(
        "cli_test_compile_handler_forwarding.zt",
        r#"
result ::= handle [
  handle [ perform fail "bad"; "unreachable" ] with {
    fail = \e. [ perform log e; "fallback" ];
  }
] with {
  log = \d. resume ();
};
result
"#,
    );
    assert!(llvm.contains("fallback"), "{llvm}");
}

#[test]
fn compile_print_list_uses_runtime_print_dispatch() {
    let llvm = compile_stdout(
        "cli_test_compile_print_list.zt",
        r#"{ print "a"; print "b"; }"#,
    );
    assert!(!llvm.contains("@zutai.aot.print"), "{llvm}");
    assert!(!llvm.contains("@zutai.effect.print"), "{llvm}");
    assert!(llvm.contains("call void @zutai.print_text"), "{llvm}");
    assert!(llvm.contains("list_cons"), "{llvm}");
}

#[test]
fn compile_reflection_schema_record_lowers_to_llvm() {
    let llvm = compile_stdout(
        "cli_test_compile_reflection_schema_record.zt",
        "Server :: type { host : Text; port? : Int; };\nschema Server\n",
    );
    assert!(llvm.contains("call void @zutai.show"), "{llvm}");
    assert!(llvm.contains("record_new"), "{llvm}");
    assert!(!llvm.contains("reflection builtins"), "{llvm}");
}

#[test]
fn compile_reflection_schema_record_bin_renders_shape() {
    let out = compile_bin_stdout(
        "cli_test_compile_reflection_schema_record_bin",
        "Server :: type { host : Text; port? : Int; };\nschema Server\n",
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
};
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
        "Empty :: type {};\nschema Empty\n",
    );
    assert!(out.contains("kind = #record"), "{out}");
    assert!(out.contains("fields = []"), "{out}");
}

#[test]
fn compile_reflection_with_effectful_code_is_rejected() {
    let path = write_tmp(
        "cli_test_compile_reflection_with_effect.zt",
        "Server :: type { host : Text; };\n_unused ::= schema Server;\nprint \"hello\"\n",
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
        "Color :: type { #red; #green; #blue; };\nschema Color\n",
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
        "Server :: type { host : Text; };\nfields Server\n",
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("returns Type"));
}

#[test]
fn compiled_variants_reflection_matches_oracle() {
    // `variants` reflection used to bypass AOT folding (only `fields`/`schema`
    // were detected) and reach Dataflow Core, where it silently miscompiled to an
    // empty result. It must now fold to the same serialized list the interpreter
    // produces.
    let src = "Color :: type { #red: {}; #green: {}; };\nvariants (Color)\n";
    let run_output = run_stdout("cli_test_variants_reflect_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_variants_reflect_compiled", src);
    assert_eq!(compiled_output, run_output);
}

#[test]
fn compiled_union_extension_matches_oracle() {
    // Union extension (`...Shape` spreading an existing union into a new one)
    // was listed as check-plus-interpreter only, but the spread members keep
    // their tags through TLC->DC, so both construction and tag dispatch across
    // the extended union compile with full parity. Cover a spread member
    // (`#square` from `Shape`) and a freshly added member (`#sphere`).
    let src = r#"
Shape :: type { #circle: { radius : Int; }; #square: { side : Int; }; };
Shape3D :: type { ...Shape; #sphere: { radius : Int; }; };
f :: Shape3D -> Int
  = #circle { radius = r; } => r;
  = #square { side = s; } => s + 100;
  = #sphere { radius = r; } => r * 10;
f (#square { side = 4; })
"#;
    let run_output = run_stdout("cli_test_union_extension_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_union_extension_compiled", src);
    assert_eq!(compiled_output, run_output);
    assert_eq!(compiled_output.trim(), "104");
}

#[test]
fn compiled_open_union_rest_match_matches_oracle() {
    // Phase D: a polymorphic match over a `<Rest>`-tailed open union. The
    // type-checker now accepts a member pattern against the rigid open union
    // (`union_rows_match` no longer demands the found tail equal the row var);
    // the match is tag-dispatched, so it compiles with no special row lowering.
    // Cover explicit members (`#dev`, `#test`) and a tail member (`#prod` → `_`).
    let src = "classify :: <Rest> { #dev; #test; ...Rest; } -> Int\n\
               = #dev => 1;\n  = #test => 2;\n  = _ => 9;\n\
               (classify #dev, classify #test, classify #prod)\n";
    let run_output = run_stdout("cli_test_open_union_rest_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_open_union_rest_compiled", src);
    assert_eq!(compiled_output, run_output, "native must match the oracle");
    assert!(
        compiled_output.contains('1')
            && compiled_output.contains('2')
            && compiled_output.contains('9'),
        "expected (1, 2, 9), got {compiled_output:?}"
    );
}

#[test]
fn open_union_rest_match_without_wildcard_is_non_exhaustive() {
    // The rigid `<Rest>` tail has unknown members, so a match over it still
    // requires a wildcard to be exhaustive.
    let path = write_tmp(
        "cli_test_open_union_rest_noexh.zt",
        "classify :: <Rest> { #dev; ...Rest; } -> Int = #dev => 1;\nclassify #dev\n",
    );
    cli()
        .arg("check")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("non-exhaustive"));
}

#[test]
fn compiled_witness_reflection_dispatch_matches_oracle() {
    // `(witness C @T).method arg` is the `WitnessReflect` expression form, not a
    // builtin binding, so it escaped reflection detection and ICE'd the backend
    // (Dataflow structural validator: a witness dict typed where a Fun/Record was
    // expected). It must now AOT-fold through the TLC evaluator to the same value
    // the interpreter computes.
    let src = r#"
Point :: type { x : Int; y : Int; };
p :: Point = { x = 1; y = 2; };
Show :: <A> @A { show :: A -> Text; } derive = <T> => \x. x
Show @Point :: derive
(witness Show @Point).show p
"#;
    let run_output = run_stdout("cli_test_witness_reflect_oracle.zt", src);
    let compiled_output = compile_bin_stdout("cli_test_witness_reflect_compiled", src);
    assert_eq!(compiled_output, run_output);
}

#[test]
fn compile_bare_witness_dict_is_rejected_not_iced() {
    // A bare `witness C @T` entry evaluates to a witness dictionary (holds
    // functions), which cannot be serialized to a backend value. The AOT-fold
    // gate must reject it cleanly rather than crash the compiler.
    let path = write_tmp(
        "cli_test_compile_bare_witness.zt",
        "Eq :: <A> @A { eq :: A -> A -> Bool; } derive\nEq @Int :: derive\nwitness Eq @Int\n",
    );
    cli()
        .arg("compile")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("did not fold to a backend value"));
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
        "MyInt :: type Int;\nMyInt\n",
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
        "adder n x = x + n;\nadder 10 5\n",
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
        "r ::= { host = \"localhost\"; port = 8080; };\nr\n",
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
};
c :: Shape = #circle { radius = 3; };
s :: Shape = #square { side = 4; };
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
        "{ prime_count = 10; compact_primes = { 2; 3; 5; }; }\n",
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
};
shape :: Shape = #circle { radius = 3; };
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

// A bounded-depth recursion that allocates one short-lived record per step:
// O(n) heap garbage for an O(1) live set — the canonical arena-leak shape.
const HEAP_STRESS_SRC: &str = r#"box :: Int -> { v: Int; }
  = n => { v = n; };
unbox :: { v: Int; } -> Int
  = { v = x; } => x;
sum :: Int -> Int -> Int
  = n acc => if n < 1 then acc else sum (n - 1) (acc + unbox (box n));
sum 4000 0
"#;

#[test]
fn compile_emit_bin_heap_stress_runs_under_default_cap() {
    // Under the default ceiling the heavy-allocating program still runs and
    // produces the right value (the cap must not regress legitimate programs).
    assert_eq!(
        compile_bin_stdout("cli_test_heap_stress", HEAP_STRESS_SRC),
        "8002000\n"
    );
}

#[test]
fn compile_emit_bin_heap_stress_aborts_over_cap() {
    let path = write_tmp("cli_test_heap_stress_cap.zt", HEAP_STRESS_SRC);
    let out = write_tmp("cli_test_heap_stress_cap", "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    // ~4000 records (~64 KiB) overrun a 32 KiB ceiling: abort cleanly with a
    // diagnostic instead of leaking until the OS OOM-kills the process. Pins
    // `ZUTAI_GC=0` — the cap is the leak-by-default runaway guard; with the
    // default-on collector the dead records would be reclaimed and never hit it.
    let output = StdCommand::new(&out)
        .env("ZUTAI_HEAP_MAX", "32k")
        .env("ZUTAI_GC", "0")
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a tiny heap cap should abort the program: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("heap limit exceeded"),
        "abort should explain the heap-cap; stderr: {stderr}"
    );
}

#[test]
fn compile_emit_bin_heap_stats_dump_reports_allocations() {
    let path = write_tmp("cli_test_heap_stats.zt", HEAP_STRESS_SRC);
    let out = write_tmp("cli_test_heap_stats", "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    // ZUTAI_HEAP_STATS prints an exit-time allocation report on stderr without
    // changing program output (4000 `box` records => "record 4000").
    let output = StdCommand::new(&out)
        .env("ZUTAI_HEAP_STATS", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "program should run: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "8002000\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("zutai heap stats:") && stderr.contains("record 4000"),
        "stats dump should report record allocations; stderr: {stderr}"
    );
}

#[test]
fn compile_emit_bin_uncurried_accumulator_drops_call_churn() {
    // Phase 33: the saturated recursive call `sum (n - 1) (…)` collapses to a
    // direct call to an uncurried 2-arg worker, and the multi-parameter clause's
    // arg-tuple is scalar-replaced. The per-iteration closure and arg-tuple
    // allocations vanish (`tuple 0`, `closure/raw 0`); only the explicit `box`
    // records remain, and the result is unchanged.
    let path = write_tmp("cli_test_uncurry_churn.zt", HEAP_STRESS_SRC);
    let out = write_tmp("cli_test_uncurry_churn", "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out)
        .env("ZUTAI_HEAP_STATS", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "program should run: {output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "8002000\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Calling-convention churn is gone; the explicit `box` records remain.
    assert!(
        stderr.contains("tuple 0") && stderr.contains("closure/raw 0"),
        "uncurrying should remove per-call arg-tuple + closure allocations; stderr: {stderr}"
    );
    assert!(
        stderr.contains("record 4000"),
        "explicit box records are user data and must remain; stderr: {stderr}"
    );
}

// ── Phase 34 GC-gate measurement ────────────────────────────────────────────────

/// The canonical accumulator shape, parametric on iteration count `n`: a
/// tail-recursive worker that allocates one short-lived `box` record per step
/// while keeping only a scalar `acc` live. After Phase 33 this is the *only*
/// per-step allocation (no call churn), so it is the cleanest probe for the
/// Phase 34 gate condition "accumulator garbage dominates".
fn accumulator_src(n: u64) -> String {
    format!(
        "box :: Int -> {{ v: Int; }}\n  = n => {{ v = n; }};\n\
unbox :: {{ v: Int; }} -> Int\n  = {{ v = x; }} => x;\n\
sum :: Int -> Int -> Int\n  = n acc => if n < 1 then acc else sum (n - 1) (acc + unbox (box n));\n\
sum {n} 0\n"
    )
}

/// Read the first integer that follows `needle` in `haystack`. Skips any
/// non-digit run between the needle and the number, then takes the digit run.
fn int_after(haystack: &str, needle: &str) -> u64 {
    let start = haystack
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?} in: {haystack}"))
        + needle.len();
    let digits: String = haystack[start..]
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("no integer after {needle:?} in: {haystack}"))
}

/// Compile `src` to a native binary, run it with `ZUTAI_HEAP_STATS=1`, and
/// return `(stdout, stderr)` (the stats line lands on stderr). Pins `ZUTAI_GC=0`:
/// this helper measures the *leak-by-default* allocation footprint, so the
/// now-default-on collector is explicitly opted out.
fn run_with_heap_stats(name: &str, src: &str) -> (String, String) {
    let path = write_tmp(&format!("{name}.zt"), src);
    let out = write_tmp(name, "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let output = StdCommand::new(&out)
        .env("ZUTAI_HEAP_STATS", "1")
        .env("ZUTAI_GC", "0")
        .output()
        .unwrap();
    assert!(output.status.success(), "program should run: {output:?}");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Phase 34 gate measurement (TBD.md gate condition (b): "accumulator garbage
/// dominates after Phase 33"). The accumulator has an O(1) live set — it returns
/// a single `Int` and retains no heap structure — yet allocates one `box` record
/// per step. We compile it at `n` and `2n`, read `ZUTAI_HEAP_STATS`, and assert
/// that there is exactly one user-data record per step (so the footprint is
/// genuine user data, not the call churn Phase 33 removed) and that the bytes
/// allocated grow linearly (2× when `n` doubles) — O(n) garbage.
///
/// A linear-growing footprint against an O(1) live set is precisely the
/// dominating-garbage signal a collector would reclaim. Sizes sit well above the
/// 1 MiB arena chunk granularity so the ratio is meaningful; run with
/// `--nocapture` to see the measured numbers.
#[test]
fn compile_emit_bin_accumulator_garbage_dominates_gc_gate() {
    const N: u64 = 200_000;
    let (out_n, stats_n) = run_with_heap_stats("cli_test_gc_gate_n", &accumulator_src(N));
    let (out_2n, stats_2n) = run_with_heap_stats("cli_test_gc_gate_2n", &accumulator_src(2 * N));

    // Semantics preserved: sum of 1..k = k(k+1)/2.
    assert_eq!(out_n.trim(), (N * (N + 1) / 2).to_string());
    assert_eq!(out_2n.trim(), (2 * N * (2 * N + 1) / 2).to_string());

    // One user-data `box` record per step — the genuine garbage, not call churn.
    let rec_n = int_after(&stats_n, "record ");
    let rec_2n = int_after(&stats_2n, "record ");
    assert_eq!(
        rec_n, N,
        "expected one box record per step; stats: {stats_n}"
    );
    assert_eq!(
        rec_2n,
        2 * N,
        "expected one box record per step; stats: {stats_2n}"
    );

    // O(n) garbage: doubling the work doubles the bytes allocated. The exact
    // linear signal lives in the cumulative `allocated` counter (the `peak
    // committed` figure rounds up to the 1 MiB chunk granularity, so it only
    // tracks this approximately and is reported below for context).
    let alloc_n = int_after(&stats_n, "allocated ");
    let alloc_2n = int_after(&stats_2n, "allocated ");
    let ratio = alloc_2n as f64 / alloc_n as f64;
    assert!(
        (1.95..=2.05).contains(&ratio),
        "allocated bytes should grow linearly with work (O(n) garbage); \
         alloc({N})={alloc_n} alloc({})={alloc_2n} ratio={ratio:.3}",
        2 * N
    );

    // The live set is O(1), so the marginal footprint per extra step is a small
    // constant: one box record (1-word header + 1 field = 16 B).
    let per_step = (alloc_2n - alloc_n) / N;
    assert!(
        (16..=32).contains(&per_step),
        "marginal footprint should be one small record per step; got {per_step} B/step"
    );

    // Peak committed is the realized footprint a collector would shrink; report
    // it for the gate decision but don't pin its chunk-quantized exact value.
    let peak_n = int_after(&stats_n, "peak committed ");
    let peak_2n = int_after(&stats_2n, "peak committed ");
    assert!(
        peak_2n > peak_n,
        "peak committed must grow with work; peak({N})={peak_n} peak({})={peak_2n}",
        2 * N
    );

    eprintln!(
        "phase-34 gate: records {rec_n}->{rec_2n}; allocated {alloc_n}B->{alloc_2n}B \
         (ratio {ratio:.3}, ~{per_step} B/step); peak committed {peak_n}B->{peak_2n}B; \
         live set O(1) => garbage dominates (gate met)"
    );
}

// ── Phase 34 conservative mark-sweep collector (opt-in) ──────────────────────────

/// Compile `src` to a native binary and run it with `env` set; return
/// `(stdout, stderr)`.
fn run_bin_env(name: &str, src: &str, env: &[(&str, &str)]) -> (String, String) {
    let path = write_tmp(&format!("{name}.zt"), src);
    let out = write_tmp(name, "");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let mut cmd = StdCommand::new(&out);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd.output().unwrap();
    assert!(output.status.success(), "program should run: {output:?}");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Phase 34 acceptance: with the collector enabled (`ZUTAI_GC`), the accumulator's
/// realized footprint (peak committed) stays *flat* as the work grows 8×, where
/// the leak-by-default arena grows ~linearly (the gate test above). Bounded
/// memory for a bounded-live / unbounded-allocation program is exactly the
/// property that justified building the collector.
#[test]
fn compile_emit_bin_gc_keeps_footprint_flat() {
    const N: u64 = 100_000;
    let gc = [("ZUTAI_GC", "1"), ("ZUTAI_HEAP_STATS", "1")];
    let (out_n, stats_n) = run_bin_env("cli_test_gc_flat_n", &accumulator_src(N), &gc);
    let (out_8n, stats_8n) = run_bin_env("cli_test_gc_flat_8n", &accumulator_src(8 * N), &gc);

    // Semantics preserved under collection.
    assert_eq!(out_n.trim(), (N * (N + 1) / 2).to_string());
    assert_eq!(out_8n.trim(), (8 * N * (8 * N + 1) / 2).to_string());

    let peak_n = int_after(&stats_n, "peak committed ");
    let peak_8n = int_after(&stats_8n, "peak committed ");
    let ratio = peak_8n as f64 / peak_n as f64;
    assert!(
        ratio < 1.5,
        "8× the work must not ~8× the footprint under GC; \
         peak({N})={peak_n} peak({})={peak_8n} ratio={ratio:.2}",
        8 * N
    );

    // The collector ran and reclaimed far more than a single footprint of garbage.
    let reclaimed = int_after(&stats_8n, "reclaimed ");
    assert!(
        reclaimed > peak_8n,
        "GC should reclaim much more than one footprint; reclaimed={reclaimed} stats: {stats_8n}"
    );

    eprintln!(
        "phase-34 gc: peak committed {peak_n}B (n={N}) vs {peak_8n}B (n={}); ratio {ratio:.2} \
         (flat); reclaimed {reclaimed}B",
        8 * N
    );
}

// A program whose result depends on an O(n) *live* heap structure: a 2000-node
// linked list, fully built before it is summed, so every node must survive until
// the fold reads it.
const GC_LIVE_CHAIN_SRC: &str = "Chain :: type { #nil; #cons : { head : Int; tail : Chain; }; };\n\
build :: Int -> Chain\n  = 0 => #nil;\n  = n => #cons { head = n; tail = build (n - 1); };\n\
sumL :: Chain -> Int\n  = #nil => 0;\n  = #cons { head = h; tail = t; } => h + sumL t;\n\
sumL (build 2000)\n";

/// Phase 34 soundness: with the collector running before *every* allocation
/// (`ZUTAI_GC_STRESS`), a program that sums a fully-built 2000-node list must
/// still produce the correct value. If the conservative root/heap scan missed a
/// live reference the list would lose nodes and the sum would be wrong — a wrong
/// value here would mean the collector is unsound.
#[test]
fn compile_emit_bin_gc_stress_preserves_live_structure() {
    let (out, _) = run_bin_env(
        "cli_test_gc_stress_chain",
        GC_LIVE_CHAIN_SRC,
        &[("ZUTAI_GC_STRESS", "1")],
    );
    // 1 + 2 + ... + 2000 = 2001000.
    assert_eq!(out.trim(), "2001000");
}

/// Phase 34: the exit-time stats dump reports collector activity when GC and heap
/// stats are both enabled.
#[test]
fn compile_emit_bin_gc_reports_collections() {
    const N: u64 = 200_000;
    let (out, stats) = run_bin_env(
        "cli_test_gc_report",
        &accumulator_src(N),
        &[("ZUTAI_GC", "1"), ("ZUTAI_HEAP_STATS", "1")],
    );
    assert_eq!(out.trim(), (N * (N + 1) / 2).to_string());
    assert!(stats.contains("zutai gc stats:"), "stats: {stats}");
    let collections = int_after(&stats, "gc stats: ");
    assert!(
        collections > 0,
        "expected at least one collection; stats: {stats}"
    );
}

// ── V3-G5: GC keeps unbounded stream pipelines flat ──────────────────────────

/// A long-running pipeline over a genuinely *unbounded* codata stream: an
/// infinite recursive generator (`countFrom`, V3-G3) bounded by `take n` and
/// summed with `fold`. Demand-driven codata produces each cell, sums it, and
/// drops it, so the live set is O(1) while allocation is O(n) — the unbounded
/// sequence reaching the backend that V3-G1 made possible (GC gate condition (a)).
fn stream_pipeline_src(n: u64) -> String {
    format!(
        "s ::= import stdlib.stream;\ncountFrom :: Int -> Stream Int\n  = m => stream {{ yield m; yield from countFrom (m + 1); }};\n\
s.fold (\\a b. a + b) 0 (take {n} (countFrom 1))\n"
    )
}

/// V3-G5 acceptance: a long-running `unfold`/stream pipeline holds steady-state
/// RSS flat under collection while producing correct output. With `ZUTAI_GC`, the
/// realized footprint (peak committed) stays flat as the bounded `take` grows 8×,
/// where the leak-by-default (`ZUTAI_GC=0`) arena grows ~linearly. This is the
/// stream analogue of `compile_emit_bin_gc_keeps_footprint_flat`: the live set is
/// O(1) (one cell plus the fold accumulator) but allocation is O(n), so the
/// collector is what keeps it bounded. (The collector is now on by default; this
/// test sets `ZUTAI_GC=1` explicitly, equivalent to the default.)
#[test]
fn compile_emit_bin_gc_keeps_stream_footprint_flat() {
    const N: u64 = 100_000;
    let gc = [("ZUTAI_GC", "1"), ("ZUTAI_HEAP_STATS", "1")];
    let (out_n, stats_n) = run_bin_env("cli_test_gc_stream_n", &stream_pipeline_src(N), &gc);
    let (out_8n, stats_8n) = run_bin_env("cli_test_gc_stream_8n", &stream_pipeline_src(8 * N), &gc);

    // Semantics preserved under collection: sum of 1..k = k(k+1)/2.
    assert_eq!(out_n.trim(), (N * (N + 1) / 2).to_string());
    assert_eq!(out_8n.trim(), (8 * N * (8 * N + 1) / 2).to_string());

    let peak_n = int_after(&stats_n, "peak committed ");
    let peak_8n = int_after(&stats_8n, "peak committed ");
    let ratio = peak_8n as f64 / peak_n as f64;
    assert!(
        ratio < 1.5,
        "8× the stream length must not ~8× the footprint under GC; \
         peak({N})={peak_n} peak({})={peak_8n} ratio={ratio:.2}",
        8 * N
    );

    // The collector ran and reclaimed far more than a single footprint of the
    // discarded stream cells (records, `#cons` variants, tail closures).
    let reclaimed = int_after(&stats_8n, "reclaimed ");
    assert!(
        reclaimed > peak_8n,
        "GC should reclaim much more than one footprint; reclaimed={reclaimed} stats: {stats_8n}"
    );

    eprintln!(
        "v3-g5 gc: stream peak committed {peak_n}B (n={N}) vs {peak_8n}B (n={}); ratio {ratio:.2} \
         (flat); reclaimed {reclaimed}B",
        8 * N
    );
}

/// V3-G5 soundness: with the collector running before *every* allocation
/// (`ZUTAI_GC_STRESS`), the stream pipeline must still produce the correct value.
/// The fold's accumulator and the currently-forced cell are stack-only roots; if
/// the conservative scan missed one, an in-flight cell would be freed and the sum
/// would be wrong. Sum of 1..500 = 125250.
#[test]
fn compile_emit_bin_gc_stress_preserves_stream_output() {
    let (out, _) = run_bin_env(
        "cli_test_gc_stress_stream",
        &stream_pipeline_src(500),
        &[("ZUTAI_GC_STRESS", "1")],
    );
    assert_eq!(out.trim(), "125250");
}

/// The collector is **on by default** (no env var needed) wherever the
/// conservative stack scan is wired up, and `ZUTAI_GC=0` opts back out to
/// leak-by-default. A bounded-live / unbounded-allocation accumulator run with no
/// GC env keeps its footprint small and reports collector activity; the same
/// binary with `ZUTAI_GC=0` leaks (a much larger peak, no collector). Both produce
/// the correct value.
#[test]
fn compile_emit_bin_gc_is_default_on_with_opt_out() {
    const N: u64 = 800_000;
    let src = accumulator_src(N);
    let expected = (N * (N + 1) / 2).to_string();

    let (out_default, stats_default) =
        run_bin_env("cli_test_gc_default_on", &src, &[("ZUTAI_HEAP_STATS", "1")]);
    let (out_off, stats_off) = run_bin_env(
        "cli_test_gc_opt_out",
        &src,
        &[("ZUTAI_HEAP_STATS", "1"), ("ZUTAI_GC", "0")],
    );

    // Semantics identical on both paths.
    assert_eq!(out_default.trim(), expected);
    assert_eq!(out_off.trim(), expected);

    // Default-on: the collector ran (stats line present) and held the footprint
    // far below the opt-out leak.
    assert!(
        stats_default.contains("zutai gc stats:"),
        "GC should run by default; stats: {stats_default}"
    );
    assert!(
        !stats_off.contains("zutai gc stats:"),
        "ZUTAI_GC=0 must opt out of the collector; stats: {stats_off}"
    );
    let peak_default = int_after(&stats_default, "peak committed ");
    let peak_off = int_after(&stats_off, "peak committed ");
    assert!(
        peak_default * 2 < peak_off,
        "default-on footprint must be far below the leak-by-default opt-out; \
         default={peak_default} opt_out={peak_off}"
    );
}

#[test]
fn compile_derive_witness_program_passes() {
    let src = r#"
Point :: type { x : Int; y : Int; };
p1 :: Point = { x = 1; y = 2; };
p2 :: Point = { x = 1; y = 2; };
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
Pair :: <A> type { fst : A; snd : A; };
Eq @(Pair A) :: <A: Eq> { eq = \p q. eq p.fst q.fst; }
p1 :: Pair Int = { fst = 1; snd = 2; };
p2 :: Pair Int = { fst = 1; snd = 2; };
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
    "s ::= { host = \"h\"; port = 8080; name = \"n\"; };\nselect s { port; host; }\n";
const SELECT_BAD_SRC: &str = "s ::= { host = \"h\"; };\nselect s { missing; }\n";

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
Server :: type { host : Text; port : Int; };
server :: Server = { host = "localhost"; port = 80; };
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
        "r ::= { b = 10; a = 20; };\nr.a\n",
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
        "r ::= { b = 10; a = 20; };\nr.b\n",
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
};
area :: Shape -> Int
  = #circle { radius = r; } => r;
  = #square { side = s; } => s;
area (#circle { radius = 3; })
"#;
    let llvm = compile_stdout("cli_test_compile_variant_pattern_value.zt", src);
    assert!(llvm.contains("call i64 @zutai.variant_value"), "{llvm}");
}

const OVERLAY_SRC: &str = r#"
Config :: type { host : Text; port : Int; };
defaults :: Config = { host = "localhost"; port = 80; };
patch :: Patch Config = { port = 8080; };
defaults |> overlay patch
"#;

const OVERLAY_DEEP_SRC: &str = r#"
Server :: type { host : Text; port : Int; };
Config :: type { server : Server; name : Text; };
defaults :: Config = {
  server = { host = "localhost"; port = 80; };
  name = "dev";
};
patch :: DeepPatch Config = { server = { port = 8080; }; };
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
    let path = write_tmp("cli_test_dataflow_parse_err.zt", "{1; 2}\n");
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
        "Server :: type { host : Text; };\nschema Server\n",
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
        "MyInt :: type Int;\nMyInt\n",
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
        "{ print \"a\"; print \"b\"; print \"c\"; }\n",
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
        "[ perform io.print \"a\"; perform io.print \"b\"; 7 ]\n",
    );
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .success()
        .stdout("a\nb\n7\n");
}

#[test]
fn compile_print_program_uses_runtime_print_dispatch() {
    let llvm = compile_stdout("cli_test_print_compile.zt", "print \"x\"\n");
    assert!(!llvm.contains("@zutai.aot.print"), "{llvm}");
    assert!(llvm.contains("call void @zutai.print_text"), "{llvm}");
    assert!(llvm.contains("define i64 @__entry"), "{llvm}");
}

#[test]
fn compile_print_program_prints_at_runtime() {
    let out = compile_bin_stdout("cli_test_print_runtime", "print \"hello\"\n");
    assert_eq!(out, "hello\n\"hello\"\n");
}

#[test]
fn compile_print_branch_prints_taken_branch_at_runtime() {
    let out = compile_bin_stdout(
        "cli_test_print_branch_runtime",
        r#"if 1 < 2 then print "then" else print "else"
"#,
    );
    assert_eq!(out, "then\n\"then\"\n");
}

#[test]
fn compile_print_function_prints_at_runtime() {
    let out = compile_bin_stdout(
        "cli_test_print_function_runtime",
        r#"
printer :: Text -> Text ! { io.print : Text -> Text; }
  = t => print t;
printer "fn"
"#,
    );
    assert_eq!(out, "fn\n\"fn\"\n");
}

#[test]
fn compile_higher_order_print_prints_at_runtime() {
    let out = compile_bin_stdout(
        "cli_test_higher_order_print_runtime",
        r#"
apply :: (Text -> Text ! { io.print : Text -> Text; }) -> Text ! { io.print : Text -> Text; }
  = f => f "ho";
apply print
"#,
    );
    assert_eq!(out, "ho\n\"ho\"\n");
}

#[test]
fn compile_local_print_binding_does_not_dispatch_host_effect() {
    let out = compile_bin_stdout(
        "cli_test_local_print_binding",
        r#"(\print. print "x") (\t. "local")
"#,
    );
    assert_eq!(out, "\"local\"\n");
}

#[test]
fn run_fs_read_dispatches_granted_host_effect() {
    let data_path = write_tmp("cli_test_host_fs_read_data.txt", "phase27");
    let source = format!(
        r#"
readFile :: Path -> Text ! {{ fs.read : Path -> Text; }}
  = path => perform fs.read path;
readFile "{}"
"#,
        zt_string_literal(&data_path)
    );
    let out = run_stdout("cli_test_host_fs_read.zt", &source);
    assert_eq!(out, "\"phase27\"\n");
}

#[test]
fn compile_fs_read_dispatches_granted_host_effect_at_runtime() {
    let data_path = write_tmp("cli_test_compile_host_fs_read_data.txt", "compiled");
    let source = format!(
        r#"
readFile :: Path -> Text ! {{ fs.read : Path -> Text; }}
  = path => perform fs.read path;
readFile "{}"
"#,
        zt_string_literal(&data_path)
    );
    let out = compile_bin_stdout("cli_test_compile_host_fs_read", &source);
    assert_eq!(out, "\"compiled\"\n");
}

#[test]
fn run_load_zti_returns_data_envelope() {
    let data_path = write_tmp(
        "cli_test_dynamic_load_data.zti",
        r#"{ host = "localhost"; port = 8080; }"#,
    );
    let source = format!(
        r#"
match loadZti "{}" {{
  | #record {{ fields = fields; }} => match listHead fields {{
      | {{ name = name; value = #text {{ value = value; }}; }} => name == "host" && value == "localhost";
      | _ => false;
    }};
  | _ => false;
}}
"#,
        zt_string_literal(&data_path)
    );
    let out = run_stdout("cli_test_dynamic_load_zti.zt", &source);
    assert_eq!(out, "true\n");
}

#[test]
fn compile_load_zt_dispatches_at_runtime() {
    let data_path = write_tmp("cli_test_dynamic_load_data.zt", r#"{ mode = #prod; }"#);
    let source = format!(
        r#"loadZt "{}"
"#,
        zt_string_literal(&data_path)
    );
    let out = compile_bin_stdout("cli_test_dynamic_load_zt", &source);
    assert!(out.contains("#record"), "{out}");
    assert!(out.contains("mode"), "{out}");
    assert!(out.contains("prod"), "{out}");
}

#[test]
fn compile_env_get_dispatches_optional_host_effect() {
    let out = compile_bin_stdout(
        "cli_test_compile_host_env_get",
        r#"
lookup :: Text -> Text? ! { env.get : Text -> Text?; }
  = name => perform env.get name;
lookup "__ZUTAI_PHASE27_UNSET__" ?? "missing"
"#,
    );
    assert_eq!(out, "\"missing\"\n");
}

#[test]
fn compile_env_get_some_branch_dispatches_optional_host_effect() {
    let out = compile_bin_stdout(
        "cli_test_compile_host_env_get_some",
        r#"
lookup :: Text -> Text? ! { env.get : Text -> Text?; }
  = name => perform env.get name;
lookup "HOME" ?? "__missing_home__"
"#,
    );
    assert_ne!(out, "\"__missing_home__\"\n");
}

#[test]
fn compile_fs_write_dispatches_and_can_read_back() {
    let path = write_tmp("cli_test_compile_host_fs_write_data.txt", "");
    let source = format!(
        r#"
[ perform fs.write {{ contents = "written"; path = "{}"; }}; perform fs.read "{}" ]
"#,
        zt_string_literal(&path),
        zt_string_literal(&path)
    );
    let out = compile_bin_stdout("cli_test_compile_host_fs_write", &source);
    assert_eq!(out, "\"written\"\n");
}

#[test]
fn compile_clock_now_dispatches_host_effect() {
    let out = compile_bin_stdout(
        "cli_test_compile_host_clock_now",
        r#"
now :: Unit -> Instant ! { clock.now : Unit -> Instant; }
  = tick => perform clock.now tick;
now ()
"#,
    );
    assert!(out.starts_with('"') && out.ends_with("\"\n"), "{out:?}");
}

#[test]
fn compile_rng_next_dispatches_deterministic_host_effect() {
    let out = compile_bin_stdout(
        "cli_test_compile_host_rng_next",
        r#"
next :: Unit -> Int ! { rng.next : Unit -> Int; }
  = tick => perform rng.next tick;
next ()
"#,
    );
    assert_eq!(out, "1618330555464769024\n");
}

#[test]
fn capability_record_entry_supplies_advisory_tokens() {
    // Spec §"Entry Boundary": a `{ caps } -> Result` entry has its capability
    // record supplied by the host. Run and compile must agree.
    let data_path = write_tmp("cli_test_cap_entry_data.txt", "capdata");
    let source = format!(
        r#"
readConfig :: FsRead -> Text ! {{ fs.read : Path -> Text; }}
  = fs => perform fs.read "{}";
main :: {{ fs : FsRead; }} -> Text ! {{ fs.read : Path -> Text; }}
  = caps => readConfig caps.fs;
main
"#,
        zt_string_literal(&data_path)
    );
    let run = run_stdout("cli_test_cap_entry_run.zt", &source);
    let compiled = compile_bin_stdout("cli_test_cap_entry_compile", &source);
    assert_eq!(run, "\"capdata\"\n");
    assert_eq!(compiled, run, "capability-entry run vs compile mismatch");
}

#[test]
fn capability_single_entry_supplies_token() {
    let data_path = write_tmp("cli_test_cap_single_data.txt", "single");
    let source = format!(
        r#"
main :: FsRead -> Text ! {{ fs.read : Path -> Text; }}
  = fs => perform fs.read "{}";
main
"#,
        zt_string_literal(&data_path)
    );
    assert_eq!(
        compile_bin_stdout("cli_test_cap_single", &source),
        "\"single\"\n"
    );
}

#[test]
fn capability_curried_entry_supplies_all_tokens() {
    // Curried capability parameters are each supplied a token.
    let data_path = write_tmp("cli_test_cap_curried_data.txt", "curried");
    let source = format!(
        r#"
main :: FsRead -> Env -> Text ! {{ fs.read : Path -> Text; env.get : Text -> Text?; }}
  = fs e => perform fs.read "{}";
main
"#,
        zt_string_literal(&data_path)
    );
    assert_eq!(
        compile_bin_stdout("cli_test_cap_curried", &source),
        "\"curried\"\n"
    );
}

#[test]
fn compile_user_function_named_main_does_not_collide_with_c_entry() {
    // A user binding named `main` must not redefine the C entry symbol.
    let out = compile_bin_stdout(
        "cli_test_user_main",
        "main :: Int -> Int = x => x;\nmain 5\n",
    );
    assert_eq!(out, "5\n");
}

#[test]
fn compile_non_capability_function_entry_is_rejected() {
    // Only capability-shaped entry functions are supplied tokens; a plain
    // function entry still cannot be rendered by the runtime ABI.
    let path = write_tmp(
        "cli_test_noncap_fn_entry.zt",
        "f :: Int -> Int = x => x;\nf\n",
    );
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("function"));
}

#[test]
fn dataflow_print_program_lowers_with_runtime_host_print() {
    let path = write_tmp("cli_test_print_dataflow.zt", "print \"x\"\n");
    cli()
        .arg("dataflow")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("HostPrint"));
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
fn run_worker_panic_exits_cleanly_without_repanic() {
    // Trigger a parser panic via a non-ASCII whitespace character in a decl
    // position (backlog #01: byte-slice at mid-char panics in the lookahead).
    // Before fix: .join().expect() re-panicked the main thread → exit code 101
    // and "thread 'main' panicked" in stderr.
    // After fix: the panic is absorbed by catch_unwind in run_isolated → clean
    // "internal evaluator error:" line on stderr and exit code 1.
    // Note: once backlog #01 is also fixed this input yields a clean parse error
    // that still satisfies .code(1) and no-re-panic; the unit tests in
    // commands/tests.rs remain the definitive guard for the conversion itself.
    let path = write_tmp("cli_test_run_panic.zt", "foo\u{00A0}:: Int\nfoo = 42\n42\n");
    cli()
        .arg("run")
        .arg(&path)
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("thread 'main' panicked").not());
}

#[test]
fn dataflow_zt_type_error_exits_nonzero() {
    let path = write_tmp("cli_test_dataflow_type_err.zt", "x :: Int = \"bad\";\nx\n");
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

/// Write an importer plus its imported files into a fresh, unique temp directory
/// so relative imports resolve, then return `(interpreter_stdout, native_stdout)`
/// for the importer. Both paths must succeed; callers compare the two outputs to
/// each other (the differential property) and to an expected literal.
fn import_run_vs_compile(test: &str, importer: &str, files: &[(&str, &str)]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("zutai_imp_{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, content) in files {
        std::fs::write(dir.join(name), content).unwrap();
    }
    let importer_path = dir.join(importer);
    let interp = cli()
        .arg("run")
        .arg(&importer_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let interp = String::from_utf8(interp).expect("run output should be UTF-8");
    let bin = dir.join("out.bin");
    cli()
        .arg("compile")
        .arg("--emit=bin")
        .arg(&importer_path)
        .arg("-o")
        .arg(&bin)
        .assert()
        .success();
    let native = StdCommand::new(&bin).output().unwrap();
    assert!(native.status.success(), "{native:?}");
    let native = String::from_utf8(native.stdout).expect("native output should be UTF-8");
    (interp, native)
}

#[test]
fn compile_zti_import_field_matches_oracle() {
    let (interp, native) = import_run_vs_compile(
        "zti_field",
        "main.zt",
        &[
            (
                "config.zti",
                "{\n  host = \"127.0.0.1\";\n  port = 8080;\n}\n",
            ),
            ("main.zt", "cfg ::= import \"config.zti\";\ncfg.port\n"),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    assert_eq!(native, "8080\n");
}

#[test]
fn compile_zti_import_whole_record_matches_oracle() {
    // Nested record, list, atom, bool, text, and int data all lower inline and
    // must render identically to the interpreter (name-sorted record display).
    let (interp, native) = import_run_vs_compile(
        "zti_record",
        "main.zt",
        &[
            (
                "data.zti",
                "{\n  a = 1;\n  nested = { b = 2; };\n  items = [10; 20;];\n  flag = true;\n  tag = #ok;\n  name = \"hi\";\n}\n",
            ),
            ("main.zt", "d ::= import \"data.zti\";\nd\n"),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
}

#[test]
fn compile_zt_function_import_matches_oracle() {
    // A `.zt` module exporting a function: native compile lowers the function
    // and applies it correctly.
    let (interp, native) = import_run_vs_compile(
        "zt_func",
        "main.zt",
        &[
            (
                "lib.zt",
                "add :: Int -> Int -> Int\n  = a b => a + b;\nadd\n",
            ),
            ("main.zt", "f ::= import \"lib.zt\";\nf 2 3\n"),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    assert!(native.trim().contains("5"), "expected 5, got {native:?}");
}

#[test]
fn compile_zt_transitive_import_matches_oracle() {
    // Chain: top.zt → mid.zt (imports config.zti) — tests A.c: a one-arena
    // merge across a .zt→.zt→.zti chain.
    let (interp, native) = import_run_vs_compile(
        "zt_chain",
        "top.zt",
        &[
            ("config.zti", "{ host = \"127.0.0.1\"; port = 8080; }\n"),
            (
                "mid.zt",
                "cfg ::= import \"config.zti\";\n{ port = cfg.port; }\n",
            ),
            ("top.zt", "mid ::= import \"mid.zt\";\nmid.port\n"),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    assert!(
        native.trim().contains("8080"),
        "expected 8080, got {native:?}"
    );
}

#[test]
fn compile_zt_diamond_import_matches_oracle() {
    // Diamond: main imports a and b, both import base independently (two
    // separate Analysis objects for the same file). Each gets its own namespace
    // prefix; results are numerically correct.
    let (interp, native) = import_run_vs_compile(
        "zt_diamond",
        "main.zt",
        &[
            ("base.zt", "n ::= 10;\nn\n"),
            ("a.zt", "base ::= import \"base.zt\";\nbase + 1\n"),
            ("b.zt", "base ::= import \"base.zt\";\nbase + 2\n"),
            (
                "main.zt",
                "a ::= import \"a.zt\";\nb ::= import \"b.zt\";\na + b\n",
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    assert!(native.trim().contains("23"), "expected 23, got {native:?}");
}
#[test]
fn compile_zt_imported_concrete_witness_matches_oracle() {
    // Cross-module concrete witness dispatch: dep declares the constraint + witness,
    // root re-declares the same constraint (making `eq` in scope), but provides
    // NO local witness for Int. Previously gated by IMPORT_WITNESS_REASON; now
    // dispatches natively via the extern witness table (GlobalRef to dep's global).
    let (interp, native) = import_run_vs_compile(
        "zt_witness_concrete",
        "main.zt",
        &[
            (
                "eq_lib.zt",
                "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. a == b; }\n1\n",
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"eq_lib.zt\";\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "eq 3 3\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    assert!(
        native.trim().contains("true"),
        "expected true, got {native:?}"
    );
}

#[test]
fn compile_zt_imported_bool_witness_matches_oracle() {
    // Concrete `Eq @Bool` witness imported from a dep; root re-declares the constraint.
    let (interp, native) = import_run_vs_compile(
        "zt_witness_bool",
        "main.zt",
        &[
            (
                "bool_eq.zt",
                "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Bool :: { eq = \\a b. a == b; }\ntrue\n",
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"bool_eq.zt\";\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "eq false false\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    assert!(
        native.trim().contains("true"),
        "expected true, got {native:?}"
    );
}

#[test]
fn compile_zt_imported_ord_witness_matches_oracle() {
    // Concrete `Ord @Int` witness imported from dep; root re-declares Ord.
    let (interp, native) = import_run_vs_compile(
        "zt_witness_ord",
        "main.zt",
        &[
            (
                "cmp_lib.zt",
                "Ord :: <A> @A { lt :: A -> A -> Bool; }\nOrd @Int :: { lt = \\a b. a < b; }\n0\n",
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"cmp_lib.zt\";\n",
                    "Ord :: <A> @A { lt :: A -> A -> Bool; }\n",
                    "lt 1 2\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    assert!(
        native.trim().contains("true"),
        "expected true, got {native:?}"
    );
}

#[test]
fn compile_zt_imported_multi_instance_witness_matches_oracle() {
    // A dep exports TWO concrete witnesses for the same constraint method
    // (`Eq @Int` and `Eq @Bool`). Each call site must dispatch to the instance
    // whose target matches the operand. The interpreter previously resolved
    // imported methods by NAME only, so two same-named instances were ambiguous
    // and the call refused; native dispatches via the type-keyed extern table.
    // The `Eq @Bool` witness returns a constant `false` (≠ structural equality of
    // `true == true`), so the result discriminates correct dispatch from wrong.
    let (interp, native) = import_run_vs_compile(
        "zt_witness_multi",
        "main.zt",
        &[
            (
                "eq_lib.zt",
                "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. a == b; }\nEq @Bool :: { eq = \\a b. false; }\n1\n",
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"eq_lib.zt\";\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "(eq 3 3, eq true true)\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    // eq 3 3 -> true (Int witness ==), eq true true -> false (Bool witness sentinel).
    assert!(
        native.contains("true") && native.contains("false"),
        "expected both true (Int) and false (Bool), got {native:?}"
    );
}

#[test]
fn compile_zt_imported_conditional_pair_witness_matches_oracle() {
    // Phase B: cross-module CONDITIONAL witness dispatch. The dep exports a
    // parametric `Eq @(Pair A) :: <A: Eq>`; the root applies `eq` to `Pair Int`
    // values. Both paths must structurally match `Pair Int` against the imported
    // witness's `{fst:?,snd:?}` shape and dispatch through the recursively
    // resolved `Eq @Int` component dict. Mirrors the in-module conditional test.
    let (interp, native) = import_run_vs_compile(
        "zt_witness_cond_pair",
        "main.zt",
        &[
            (
                "eq_lib.zt",
                concat!(
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "Eq @Int :: { eq = \\a b. a == b; }\n",
                    "Pair :: <A> type { fst : A; snd : A; };\n",
                    "Eq @(Pair A) :: <A: Eq> { eq = \\p q. eq p.fst q.fst; }\n",
                    "1\n",
                ),
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"eq_lib.zt\";\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "Pair :: <A> type { fst : A; snd : A; };\n",
                    "p1 :: Pair Int = { fst = 1; snd = 2; };\n",
                    "p2 :: Pair Int = { fst = 1; snd = 9; };\n",
                    "p3 :: Pair Int = { fst = 7; snd = 2; };\n",
                    "(eq p1 p2, eq p1 p3)\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    // eq p1 p2 -> true (same fst), eq p1 p3 -> false (different fst).
    assert!(
        native.contains("true") && native.contains("false"),
        "expected (true, false), got {native:?}"
    );
}

#[test]
fn compile_zt_imported_conditional_list_witness_matches_oracle() {
    // Phase B: cross-module conditional witness over a builtin constructor. The
    // dep exports `Eq @(List A) :: <A: Eq>` returning a `false` sentinel; the
    // root dispatches `eq` on both `Int` (imported concrete, structural ==) and
    // `List Int` (imported conditional, sentinel). The discriminating result
    // proves each call resolves to the instance whose target matches the operand.
    let (interp, native) = import_run_vs_compile(
        "zt_witness_cond_list",
        "main.zt",
        &[
            (
                "eq_lib.zt",
                concat!(
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "Eq @Int :: { eq = \\a b. a == b; }\n",
                    "Eq @(List A) :: <A: Eq> { eq = \\xs ys. false; }\n",
                    "1\n",
                ),
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"eq_lib.zt\";\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "(eq 1 1, eq { 1; } { 1; })\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    // eq 1 1 -> true (Int ==), eq { 1; } { 1; } -> false (List sentinel).
    assert!(
        native.contains("true") && native.contains("false"),
        "expected both true (Int) and false (List), got {native:?}"
    );
}

#[test]
fn compile_zt_imported_nested_conditional_witness_matches_oracle() {
    // Phase B: a conditional witness whose component is itself conditional
    // (`Eq @(List (Pair Int))` resolves `Eq @(List A)` over `Eq @(Pair A)` over
    // `Eq @Int`). Exercises (a) recursive component-dict resolution on both
    // paths, (b) the nested-alias key fix (`Pair Int` keys as `{fst:Int,snd:Int}`,
    // not `{fst:@N,...}`), and (c) distinct virtual globals for several imported
    // witnesses used at one site (the upward-counting virtual-binding allocator).
    let (interp, native) = import_run_vs_compile(
        "zt_witness_cond_nested",
        "main.zt",
        &[
            (
                "eq_lib.zt",
                concat!(
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "Eq @Int :: { eq = \\a b. a == b; }\n",
                    "Pair :: <A> type { fst : A; snd : A; };\n",
                    "Eq @(Pair A) :: <A: Eq> { eq = \\p q. eq p.fst q.fst; }\n",
                    "Eq @(List A) :: <A: Eq> { eq = \\xs ys. false; }\n",
                    "1\n",
                ),
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"eq_lib.zt\";\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "Pair :: <A> type { fst : A; snd : A; };\n",
                    "a :: Pair Int = { fst = 1; snd = 2; };\n",
                    "b :: Pair Int = { fst = 1; snd = 2; };\n",
                    "xs :: List (Pair Int) = { a; };\n",
                    "ys :: List (Pair Int) = { b; };\n",
                    "(eq a b, eq xs ys)\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    // eq a b -> true (Pair via Int ==), eq xs ys -> false (List sentinel).
    assert!(
        native.contains("true") && native.contains("false"),
        "expected (true, false), got {native:?}"
    );
}

#[test]
fn compile_zt_imported_conditional_optional_witness_matches_oracle() {
    // Phase B parity guard (reviewer finding 1): the `Optional A` target keys as
    // the postfix `Int?`; the interpreter's balanced-token matcher must reserve
    // the trailing `?` for the Optional marker rather than letting the hole eat it.
    let (interp, native) = import_run_vs_compile(
        "zt_witness_cond_opt",
        "main.zt",
        &[
            (
                "eq_lib.zt",
                concat!(
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "Eq @Int :: { eq = \\a b. a == b; }\n",
                    "Eq @(Optional A) :: <A: Eq> { eq = \\a b. false; }\n",
                    "1\n",
                ),
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"eq_lib.zt\";\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "x :: Int? = #some (1);\n",
                    "y :: Int? = #some (1);\n",
                    "(eq 1 1, eq x y)\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    // eq 1 1 -> true (Int ==), eq x y -> false (Optional sentinel).
    assert!(
        native.contains("true") && native.contains("false"),
        "expected (true, false), got {native:?}"
    );
}

#[test]
fn compile_zt_imported_conditional_cross_constraint_component_matches_oracle() {
    // Phase B parity guard (reviewer finding 3): a conditional witness whose
    // parameter bound names a DIFFERENT constraint (`Show`) than its head (`Eq`),
    // which the importer never declares. The component dict must resolve from the
    // imported extern tables by name, with no local constraint declaration.
    let (interp, native) = import_run_vs_compile(
        "zt_witness_cond_xc",
        "main.zt",
        &[
            (
                "eq_lib.zt",
                concat!(
                    "Show :: <A> @A { show :: A -> Text; }\n",
                    "Show @Int :: { show = \\a. \"n\"; }\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "Eq @(List A) :: <A: Show> { eq = \\xs ys. false; }\n",
                    "1\n",
                ),
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"eq_lib.zt\";\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "eq { 1; } { 1; }\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    assert!(native.contains("false"), "expected false, got {native:?}");
}

#[test]
fn compile_zt_imported_conditional_digit_suffix_record_matches_oracle() {
    // Phase B parity guard (reviewer finding 2): record field names where one is a
    // prefix of another with a digit suffix (`x`, `x2`) sort differently by name
    // vs by the rendered `name:type` part. The pattern's field order must match
    // `structural_witness_key`'s part-sorted dispatch key order.
    let (interp, native) = import_run_vs_compile(
        "zt_witness_cond_digit",
        "main.zt",
        &[
            (
                "eq_lib.zt",
                concat!(
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "Eq @Int :: { eq = \\a b. a == b; }\n",
                    "Rec :: <A> type { x : A; x2 : A; };\n",
                    "Eq @(Rec A) :: <A: Eq> { eq = \\p q. eq p.x q.x; }\n",
                    "1\n",
                ),
            ),
            (
                "main.zt",
                concat!(
                    "_ ::= import \"eq_lib.zt\";\n",
                    "Eq :: <A> @A { eq :: A -> A -> Bool; }\n",
                    "Rec :: <A> type { x : A; x2 : A; };\n",
                    "r1 :: Rec Int = { x = 1; x2 = 2; };\n",
                    "r2 :: Rec Int = { x = 9; x2 = 2; };\n",
                    "(eq r1 r1, eq r1 r2)\n",
                ),
            ),
        ],
    );
    assert_eq!(native, interp, "native must match the interpreter oracle");
    // eq r1 r1 -> true (same x), eq r1 r2 -> false (different x).
    assert!(
        native.contains("true") && native.contains("false"),
        "expected (true, false), got {native:?}"
    );
}
