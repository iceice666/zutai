use std::fs;
use std::path::{Path, PathBuf};

use zutai_syntax::{SyntaxKind, format_source, parse, tokenize};

const GENERAL_DIRS: &[&str] = &["crates/general/fixtures", "examples", "stdlib", "website"];
const IMMEDIATE_DIRS: &[&str] = &[
    "crates/general/fixtures",
    "crates/immediate/fixtures",
    "examples",
    "stdlib",
    "website",
];

#[test]
fn tracked_source_corpus_formats_idempotently() {
    let root = repo_root();
    let mut sources = Vec::new();
    for directory in GENERAL_DIRS {
        collect_sources(&root.join(directory), "zt", &mut sources);
    }
    sources.sort();
    sources.dedup();

    let mut formatted_count = 0usize;
    for path in sources {
        let source = fs::read_to_string(&path).unwrap();
        let parsed = parse(&source);
        if parsed.ast().is_none() {
            continue;
        }
        let formatted = format_source(&source).unwrap_or_else(|diagnostics| {
            panic!("format {} failed: {diagnostics:?}", path.display())
        });
        assert_preserved(&source, &formatted, &path);
        assert_eq!(
            format_source(&formatted).unwrap(),
            formatted,
            "second formatting pass changed {}",
            path.display()
        );
        formatted_count += 1;
    }
    assert!(
        formatted_count >= 75,
        "formatted only {formatted_count} .zt files"
    );

    let mut immediate = Vec::new();
    for directory in IMMEDIATE_DIRS {
        collect_sources(&root.join(directory), "zti", &mut immediate);
    }
    immediate.sort();
    immediate.dedup();

    let mut immediate_count = 0usize;
    for path in immediate {
        let source = fs::read_to_string(&path).unwrap();
        let Ok(formatted) = zutai_im_syntax::format_source(&source) else {
            continue;
        };
        let mut before = source.as_str();
        let mut after = formatted.as_str();
        assert_eq!(
            zutai_im_syntax::parser::parse(&mut before).unwrap(),
            zutai_im_syntax::parser::parse(&mut after).unwrap(),
            "format changed {}",
            path.display()
        );
        assert_eq!(
            zutai_im_syntax::format_source(&formatted).unwrap(),
            formatted,
            "second formatting pass changed {}",
            path.display()
        );
        immediate_count += 1;
    }
    assert!(
        immediate_count >= 15,
        "formatted only {immediate_count} .zti files"
    );
}

#[test]
fn parseable_spec_fences_format_idempotently() {
    let root = repo_root();
    let mut docs = Vec::new();
    collect_markdown(&root.join("docs/spec"), &mut docs);
    docs.push(root.join("docs/language-manual.md"));
    docs.sort();

    let mut general_count = 0usize;
    let mut immediate_count = 0usize;
    for path in docs {
        for fence in extract_fences(&path) {
            match fence.language.as_str() {
                "zt" => {
                    let Ok(formatted) = format_source(&fence.body) else {
                        continue;
                    };
                    assert_preserved(&fence.body, &formatted, &path);
                    assert_eq!(format_source(&formatted).unwrap(), formatted);
                    general_count += 1;
                }
                "zti" => {
                    let Ok(formatted) = zutai_im_syntax::format_source(&fence.body) else {
                        continue;
                    };
                    let mut before = fence.body.as_str();
                    let mut after = formatted.as_str();
                    assert_eq!(
                        zutai_im_syntax::parser::parse(&mut before).unwrap(),
                        zutai_im_syntax::parser::parse(&mut after).unwrap()
                    );
                    assert_eq!(
                        zutai_im_syntax::format_source(&formatted).unwrap(),
                        formatted
                    );
                    immediate_count += 1;
                }
                _ => {}
            }
        }
    }
    assert!(
        general_count >= 150,
        "formatted only {general_count} .zt fences"
    );
    assert!(
        immediate_count >= 5,
        "formatted only {immediate_count} .zti fences"
    );
}

fn assert_preserved(before: &str, after: &str, path: &Path) {
    let before_tokens: Vec<_> = tokenize(before)
        .into_iter()
        .filter(|token| !matches!(token.kind, SyntaxKind::Whitespace | SyntaxKind::Newline))
        .map(|token| {
            let text = if matches!(token.kind, SyntaxKind::LineComment | SyntaxKind::DocComment) {
                token.text.trim_end_matches('\r')
            } else {
                token.text
            };
            (token.kind, text)
        })
        .collect();
    let after_tokens: Vec<_> = tokenize(after)
        .into_iter()
        .filter(|token| !matches!(token.kind, SyntaxKind::Whitespace | SyntaxKind::Newline))
        .map(|token| (token.kind, token.text))
        .collect();
    assert_eq!(
        before_tokens,
        after_tokens,
        "tokens changed in {}",
        path.display()
    );
    assert_eq!(
        before.lines().count(),
        after.lines().count(),
        "line boundaries changed in {}",
        path.display()
    );
    assert!(
        format_source(after).is_ok(),
        "formatted source did not parse: {}",
        path.display()
    );
}

fn collect_sources(directory: &Path, extension: &str, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_sources(&path, extension, output);
        } else if path.extension().is_some_and(|value| value == extension) {
            output.push(path);
        }
    }
}

fn collect_markdown(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_markdown(&path, output);
        } else if path.extension().is_some_and(|value| value == "md") {
            output.push(path);
        }
    }
}

struct Fence {
    language: String,
    body: String,
}

fn extract_fences(path: &Path) -> Vec<Fence> {
    let source = fs::read_to_string(path).unwrap();
    let mut fences = Vec::new();
    let mut open = None::<Fence>;
    for line in source.lines() {
        if let Some(language) = line.strip_prefix("```") {
            if let Some(fence) = open.take() {
                fences.push(fence);
            } else {
                open = Some(Fence {
                    language: language.trim().to_owned(),
                    body: String::new(),
                });
            }
        } else if let Some(fence) = &mut open {
            if !fence.body.is_empty() {
                fence.body.push('\n');
            }
            fence.body.push_str(line);
        }
    }
    assert!(open.is_none(), "unterminated fence in {}", path.display());
    fences
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}
