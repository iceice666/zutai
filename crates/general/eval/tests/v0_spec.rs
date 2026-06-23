//! Acceptance sweep for runnable/checkable code fences in `docs/spec/v0`.
//!
//! The v0 spec intentionally contains a mix of complete programs, snippets,
//! pseudo grammar, and known-invalid examples. This test still extracts every
//! fenced block and runs the relevant compiler front door over it. Any `.zt`
//! fence that currently checks or runs, and any `.zti` fence that currently
//! parses, is snapshotted below so surviving examples stay acceptance-tested.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use zutai_eval::eval_file;
use zutai_semantic::analyze;

#[derive(Debug)]
struct Fence {
    id: String,
    lang: String,
    body: String,
}

const EXPECTED_ZTI_PARSE: &[&str] = &[
    "docs/spec/v0/01-overview/file-modes.md#01",
    "docs/spec/v0/02-lexical/conventions.md#10",
    "docs/spec/v0/02-lexical/conventions.md#11",
    "docs/spec/v0/02-lexical/conventions.md#15",
    "docs/spec/v0/03-immediate-mode/immediate-mode.md#02",
    "docs/spec/v0/03-immediate-mode/immediate-mode.md#06",
    "docs/spec/v0/07-modules/serialization-boundary.md#05",
    "docs/spec/v0/08-reference/complete-example.md#01",
    "docs/spec/v0/08-reference/complete-example.md#03",
];
const EXPECTED_ZT_CHECK: &[&str] = &[
    "docs/spec/v0/02-lexical/conventions.md#16",
    "docs/spec/v0/02-lexical/conventions.md#24",
    "docs/spec/v0/04-general-mode/file-structure.md#15",
    "docs/spec/v0/04-general-mode/file-structure.md#17",
    "docs/spec/v0/04-general-mode/functions.md#14",
    "docs/spec/v0/04-general-mode/imports.md#05",
    "docs/spec/v0/04-general-mode/values.md#03",
    "docs/spec/v0/04-general-mode/values.md#04",
    "docs/spec/v0/04-general-mode/values.md#05",
    "docs/spec/v0/04-general-mode/values.md#07",
    "docs/spec/v0/04-general-mode/values.md#13",
    "docs/spec/v0/04-general-mode/values.md#14",
    "docs/spec/v0/05-type-system/field-access.md#03",
    "docs/spec/v0/05-type-system/field-access.md#04",
    "docs/spec/v0/05-type-system/records.md#04",
    "docs/spec/v0/05-type-system/records.md#05",
    "docs/spec/v0/05-type-system/records.md#06",
    "docs/spec/v0/07-modules/modules.md#01",
    "docs/spec/v0/07-modules/serialization-boundary.md#04",
    "docs/spec/v0/07-modules/serialization-boundary.md#06",
];
const EXPECTED_ZT_RUN: &[(&str, &str)] = &[
    (
        "docs/spec/v0/02-lexical/conventions.md#16",
        "{ profile = #prod }",
    ),
    (
        "docs/spec/v0/02-lexical/conventions.md#24",
        "{ host = \"localhost\";  target_triple = \"x86_64-linux\" }",
    ),
    (
        "docs/spec/v0/04-general-mode/file-structure.md#15",
        "<function/1>",
    ),
    ("docs/spec/v0/04-general-mode/file-structure.md#17", "120"),
    (
        "docs/spec/v0/04-general-mode/functions.md#14",
        "<function/1>",
    ),
    ("docs/spec/v0/04-general-mode/imports.md#05", "#prod"),
    ("docs/spec/v0/04-general-mode/values.md#03", "<type>"),
    ("docs/spec/v0/04-general-mode/values.md#04", "<type>"),
    ("docs/spec/v0/04-general-mode/values.md#05", "\"hello\""),
    (
        "docs/spec/v0/04-general-mode/values.md#07",
        "{ host = \"localhost\";  port = 8080 }",
    ),
    (
        "docs/spec/v0/04-general-mode/values.md#13",
        "[\"alpha\"; \"beta\"; \"gamma\"]",
    ),
    (
        "docs/spec/v0/04-general-mode/values.md#14",
        "[\"alpha\"; \"beta\"; \"gamma\"]",
    ),
    ("docs/spec/v0/05-type-system/field-access.md#03", "#absent"),
    (
        "docs/spec/v0/05-type-system/field-access.md#04",
        "#present (8080)",
    ),
    (
        "docs/spec/v0/05-type-system/records.md#04",
        "{ host = \"localhost\";  port = 8080 }",
    ),
    ("docs/spec/v0/05-type-system/records.md#05", "<type>"),
    ("docs/spec/v0/05-type-system/records.md#06", "<type>"),
    (
        "docs/spec/v0/07-modules/modules.md#01",
        "{ RawServer = <type>;  Server = <type>;  normalize = <function/1> }",
    ),
    (
        "docs/spec/v0/07-modules/serialization-boundary.md#04",
        "{ profile = #prod }",
    ),
    (
        "docs/spec/v0/07-modules/serialization-boundary.md#06",
        "{ a = #quit;  b = #spawn { command = \"ghostty\" } }",
    ),
];

#[test]
fn v0_spec_code_fences_have_stable_acceptance_coverage() {
    let fences = extract_v0_spec_fences();
    let mut zti_parse = Vec::new();
    let mut zt_check = Vec::new();
    let mut zt_run = BTreeMap::new();
    let mut zti_seen = 0;
    let mut zt_seen = 0;

    for fence in fences {
        match fence.lang.as_str() {
            "zti" => {
                zti_seen += 1;
                if zutai_im::parse(&fence.body).is_ok() {
                    zti_parse.push(fence.id);
                }
            }
            "zt" => {
                zt_seen += 1;
                let analysis = analyze(&fence.body);
                if analysis.diagnostics.is_empty() && analysis.is_thir_complete() {
                    zt_check.push(fence.id.clone());
                    if let Ok(value) = eval_file(&fence.body) {
                        zt_run.insert(fence.id, value.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    assert!(zti_seen > 0, "expected at least one .zti fence");
    assert!(zt_seen > 0, "expected at least one .zt fence");
    assert_eq!(
        expected_vec(EXPECTED_ZTI_PARSE),
        zti_parse,
        "parsed .zti fences changed"
    );
    assert_eq!(
        expected_vec(EXPECTED_ZT_CHECK),
        zt_check,
        "checking .zt fences changed"
    );
    assert_eq!(
        expected_map(EXPECTED_ZT_RUN),
        zt_run,
        "runnable .zt fences or their displayed values changed"
    );
}

fn extract_v0_spec_fences() -> Vec<Fence> {
    let root = repo_root();
    let docs = root.join("docs/spec/v0");
    let mut files = Vec::new();
    collect_markdown_files(&docs, &mut files);
    files.sort();

    let mut fences = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or_else(|_| panic!("{} not under {}", path.display(), root.display()))
            .to_string_lossy()
            .replace('\\', "/");
        let text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let mut open = None::<OpenFence>;
        let mut ordinal = 0usize;

        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("```") {
                if let Some(open_fence) = open.take() {
                    ordinal += 1;
                    fences.push(Fence {
                        id: format!("{rel}#{ordinal:02}"),
                        lang: open_fence.lang,
                        body: open_fence.body.join("\n"),
                    });
                } else {
                    open = Some(OpenFence {
                        lang: rest.trim().to_string(),
                        body: Vec::new(),
                    });
                }
            } else if let Some(open_fence) = &mut open {
                open_fence.body.push(line.to_string());
            }
        }

        assert!(open.is_none(), "unterminated code fence in {rel}");
    }

    fences
}

#[derive(Debug)]
struct OpenFence {
    lang: String,
    body: Vec<String>,
}

fn collect_markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display())) {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("canonical repo root")
}

fn expected_vec(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn expected_map(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}
