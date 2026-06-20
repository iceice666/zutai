//! Semantic gating for the shared `.zt` fixture corpus.
//!
//! `valid/*.zt` MUST analyze cleanly through THIR with zero diagnostics, and
//! `invalid/*.zt` MUST be rejected (a parse/semantic diagnostic or an
//! incomplete THIR). These fixtures were previously only parse-gated in
//! `zutai-syntax`, so their *semantics* silently rotted away from the
//! implementation. This test keeps them honest. Directories are scanned at
//! runtime, so a newly added fixture is covered without editing this file.

use std::path::{Path, PathBuf};

use zutai_semantic::analyze_path;

fn zt_fixtures(kind: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures")
        .join(kind);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "zt"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .zt fixtures in {}", dir.display());
    paths
}

fn name(path: &Path) -> std::borrow::Cow<'_, str> {
    path.file_name().unwrap_or_default().to_string_lossy()
}

#[test]
fn valid_fixtures_analyze_cleanly() {
    for path in zt_fixtures("valid") {
        let analysis =
            analyze_path(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            analysis.diagnostics.is_empty() && analysis.is_thir_complete(),
            "valid/{} must analyze cleanly through THIR, but reported:\n{:#?}",
            name(&path),
            analysis.diagnostics,
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
