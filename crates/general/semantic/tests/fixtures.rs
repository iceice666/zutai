//! Semantic/TLC gating for the shared `.zt` fixture corpus.
//!
//! `expr_core.zt` and `valid/*.zt` MUST analyze cleanly through THIR and
//! lower to TLC with a typed final expression. `invalid/*.zt` MUST be rejected
//! (a parse/semantic diagnostic or an incomplete THIR). The syntax crate still
//! parse-gates these fixtures; this test keeps their downstream semantics from
//! silently rotting away. Directories are scanned at runtime, so a newly added
//! valid/invalid fixture is covered without editing this file.

use std::path::{Path, PathBuf};

use zutai_semantic::analyze_path;

fn zt_fixtures(kind: &str) -> Vec<PathBuf> {
    let dir = fixtures_dir().join(kind);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "zt"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .zt fixtures in {}", dir.display());
    paths
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn valid_semantic_fixtures() -> Vec<PathBuf> {
    let mut paths = vec![fixtures_dir().join("expr_core.zt")];
    paths.extend(zt_fixtures("valid"));
    paths
}

fn name(path: &Path) -> std::borrow::Cow<'_, str> {
    path.file_name().unwrap_or_default().to_string_lossy()
}

#[test]
fn valid_fixtures_lower_to_tlc() {
    for path in valid_semantic_fixtures() {
        let analysis =
            analyze_path(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            analysis.diagnostics.is_empty() && analysis.is_thir_complete(),
            "{} must analyze cleanly through THIR, but reported:\n{:#?}",
            name(&path),
            analysis.diagnostics,
        );
        let tlc = analysis
            .tlc
            .as_ref()
            .unwrap_or_else(|| panic!("{} must lower to TLC", name(&path)));
        let final_expr = tlc
            .final_expr
            .unwrap_or_else(|| panic!("{} TLC must retain a final expression", name(&path)));
        assert!(
            tlc.expr_types.contains_key(&final_expr),
            "{} TLC final expression must have a type",
            name(&path),
        );
    }
}

#[test]
fn invalid_fixtures_are_rejected() {
    for path in zt_fixtures("invalid") {
        let analysis =
            analyze_path(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            !analysis.diagnostics.is_empty() || !analysis.is_thir_complete(),
            "invalid/{} must be rejected, but it analyzed cleanly",
            name(&path),
        );
    }
}
