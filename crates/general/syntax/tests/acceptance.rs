/// Fixture-corpus acceptance tests: data-driven, directory-discovered.
///
/// Each `.zt` file is found at runtime via `read_dir`; the expected outcome is
/// determined by the subdirectory it lives in:
///
///   fixtures root   → parse-clean (zero diagnostics + lossless round-trip)
///   valid/          → parse-clean
///   invalid/        → per-file `-- Expected:` marker:
///                       `parse-error` means ≥1 parser/validation diagnostic;
///                       `semantic-error` means parse-clean, semantically invalid
///   semantic_invalid/ → parse-clean today (placeholder for future semantic passes)
///
/// Adding a new fixture file is enough to have it tested — no manual wiring required.
use zutai_syntax::parse;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures")
        .canonicalize()
        .expect("fixtures directory not found")
}

/// Collect every `*.zt` file directly inside `dir` (non-recursive), sorted.
fn read_zt(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {:?}: {e}", dir))
        .filter_map(|e| {
            let e = e.expect("read_dir entry");
            let path = e.path();
            if path.is_file() && path.extension().and_then(|x| x.to_str()) == Some("zt") {
                let name = path
                    .strip_prefix(fixtures_dir())
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                let src = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {:?}: {e}", path));
                Some((name, src))
            } else {
                None
            }
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn check_clean(src: &str) -> Result<(), String> {
    let p = parse(src);
    let text = p.syntax().text().to_string();
    if text != src {
        return Err(format!(
            "round-trip failed: {:?} ≠ {:?}",
            &text[..text.len().min(120)],
            &src[..src.len().min(120)]
        ));
    }
    if !p.diagnostics.is_empty() {
        return Err(format!(
            "expected zero diagnostics, got {}: {:?}",
            p.diagnostics.len(),
            p.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        ));
    }
    Ok(())
}

fn check_error(src: &str) -> Result<(), String> {
    let p = parse(src);
    let text = p.syntax().text().to_string();
    if text != src {
        return Err(format!(
            "round-trip failed: {:?} ≠ {:?}",
            &text[..text.len().min(120)],
            &src[..src.len().min(120)]
        ));
    }
    if p.diagnostics.is_empty() {
        return Err("expected ≥1 diagnostic, got zero".to_string());
    }
    Ok(())
}

fn expected_kind(src: &str) -> Option<&'static str> {
    src.lines().find_map(|line| {
        let line = line.trim();
        if line == "-- Expected: parse-error (>=1 diagnostic + lossless round-trip)"
            || line == "-- Expected: parse-error (≥1 diagnostic + lossless round-trip)"
        {
            Some("parse-error")
        } else if line == "-- Expected: semantic-error (parse-clean + semantic diagnostic)" {
            Some("semantic-error")
        } else {
            None
        }
    })
}

fn run_category<F>(dir: &std::path::Path, check: F)
where
    F: Fn(&str) -> Result<(), String>,
{
    let files = read_zt(dir);
    assert!(
        !files.is_empty(),
        "no *.zt files found in {:?} — wrong path?",
        dir
    );
    let failures: Vec<String> = files
        .iter()
        .filter_map(|(name, src)| check(src).err().map(|e| format!("{name}: {e}")))
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

// ── Fixture corpus tests ──────────────────────────────────────────────────────

#[test]
fn fixtures_root_parse_clean() {
    run_category(&fixtures_dir(), check_clean);
}

#[test]
fn fixtures_valid_parse_clean() {
    run_category(&fixtures_dir().join("valid"), check_clean);
}

#[test]
fn fixtures_invalid_parse_error() {
    let files = read_zt(&fixtures_dir().join("invalid"));
    assert!(!files.is_empty(), "no invalid fixtures found");

    let failures: Vec<String> = files
        .iter()
        .filter_map(|(name, src)| {
            let result = match expected_kind(src) {
                Some("parse-error") => check_error(src),
                Some("semantic-error") => check_clean(src),
                Some(other) => Err(format!("unknown expected fixture kind `{other}`")),
                None => Err("missing `-- Expected:` fixture marker".to_string()),
            };
            result.err().map(|e| format!("{name}: {e}"))
        })
        .collect();

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn fixtures_semantic_invalid_parse_clean() {
    // These fixtures are spec-invalid per v0 but remain parse-clean until their
    // corresponding semantic pass lands.
    run_category(&fixtures_dir().join("semantic_invalid"), check_clean);
}

// ── Never-panic property: random bytes ───────────────────────────────────────

#[test]
fn never_panic_single_bytes() {
    for b in 0u8..=127u8 {
        if let Ok(s) = std::str::from_utf8(&[b]) {
            let p = parse(s);
            assert_eq!(
                p.syntax().text().to_string(),
                s,
                "round-trip failed for byte {b:#04x}"
            );
        }
    }
}

#[test]
fn never_panic_two_byte_pairs() {
    let pairs: &[(&str, &str)] = &[
        ("a", "b"),
        ("\n", "a"),
        ("a", "\n"),
        ("::", "::"),
        (":=", ":="),
        ("->", "->"),
        ("|>", "|>"),
        ("<|", "<|"),
        ("??", "??"),
        (".", "."),
        ("{", "}"),
        ("[", "]"),
        ("(", ")"),
    ];
    for &(a, b) in pairs {
        let src = format!("{a}{b}");
        let p = parse(&src);
        assert_eq!(
            p.syntax().text().to_string(),
            src,
            "round-trip failed for {src:?}"
        );
    }
}

#[test]
fn never_panic_adversarial_sequences() {
    let inputs = [
        "",
        "   ",
        "\n\n\n",
        ":= := :=",
        ":: :: ::",
        "-> -> ->",
        "|> |> |>",
        "<| <| <|",
        "?? ?? ??",
        "{ { { } } }",
        "[ [ [ ] ] ]",
        "( ( ( ) ) )",
        "a b c d e",
        ":: a { } :: b { } :: c { }",
        "x : Int = y : Bool = true",
        "type type type",
        "match match { _ => _ }",
        "if if if then then then else else else",
        "#",
        "##",
        "###",
        "; ; ; ; ;",
        ",,,",
        "...",
        "=====",
        "-----",
        "?????",
    ];
    for src in &inputs {
        let p = parse(src);
        assert_eq!(
            p.syntax().text().to_string(),
            *src,
            "round-trip failed for {src:?}"
        );
    }
}
