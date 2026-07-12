use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn dedicated_cli_exposes_build_and_serve() {
    Command::cargo_bin("zutai-web")
        .expect("zutai-web binary")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Build and serve Zutai browser applications",
        ))
        .stdout(predicate::str::contains("build"))
        .stdout(predicate::str::contains("serve"));
}

#[test]
fn build_help_documents_the_web_entry_contract() {
    Command::cargo_bin("zutai-web")
        .expect("zutai-web binary")
        .args(["build", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Browser program entry `.zt` file"))
        .stdout(predicate::str::contains("--out-dir"))
        .stdout(predicate::str::contains("--source-root"))
        .stdout(predicate::str::contains("--public-dir"));
}

#[test]
fn serve_help_documents_development_server_options() {
    Command::cargo_bin("zutai-web")
        .expect("zutai-web binary")
        .args(["serve", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--addr"))
        .stdout(predicate::str::contains("--no-build"))
        .stdout(predicate::str::contains("127.0.0.1:8787"));
}

#[test]
fn build_rejects_a_missing_entry_without_invoking_the_toolchain() {
    Command::cargo_bin("zutai-web")
        .expect("zutai-web binary")
        .args(["build", "definitely-missing-main.zt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));
}
