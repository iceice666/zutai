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
