//! A small, dependency-free Language Server Protocol implementation.
//!
//! The server intentionally owns only protocol/session state. Semantic work is
//! delegated to `zutai-semantic`, keeping the editor and CLI on the same
//! parse → HIR → THIR path.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;

pub(super) fn document_text(params: &Value) -> Option<(String, Document)> {
    let document = params.get("textDocument")?;
    Some((
        document.get("uri")?.as_str()?.to_owned(),
        Document {
            text: document.get("text")?.as_str()?.to_owned(),
            version: document.get("version").and_then(Value::as_i64),
        },
    ))
}

pub(super) fn path_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn portable_package_path(path: &std::path::Path) -> Option<(&str, &str)> {
    let mut components = path.components();
    (components.next()?.as_os_str() == "<package>").then_some(())?;
    let package = components.next()?.as_os_str().to_str()?;
    let source = path
        .strip_prefix(Path::new("<package>").join(package))
        .ok()?;
    source.to_str().map(|source| (package, source))
}

pub(super) fn severity(severity: zutai_syntax::Severity) -> u8 {
    match severity {
        zutai_syntax::Severity::Error => 1,
        zutai_syntax::Severity::Warning => 2,
        zutai_syntax::Severity::Info => 3,
        zutai_syntax::Severity::Hint => 4,
    }
}

pub(super) const KEYWORDS: &[&str] = &[
    "cond", "false", "handle", "if", "import", "match", "perform", "resume", "select", "then",
    "true", "type", "with",
];

pub(super) fn completion_prefix(source: &str, offset: usize) -> (usize, String) {
    let mut start = floor_boundary(source, offset.min(source.len()));
    while let Some(character) = source[..start].chars().next_back() {
        if !zutai_syntax::ident::is_ident_continue(character) {
            break;
        }
        start -= character.len_utf8();
    }
    let prefix = &source[start..offset];
    if prefix
        .chars()
        .next()
        .is_some_and(zutai_syntax::ident::is_ident_start)
    {
        (start, prefix.to_owned())
    } else {
        (offset, String::new())
    }
}

pub(super) fn valid_identifier(name: &str) -> bool {
    let tokens = zutai_syntax::tokenize(name);
    matches!(tokens.as_slice(), [token] if token.kind == zutai_syntax::SyntaxKind::Ident && token.text == name)
}

pub(super) fn contains(span: zutai_syntax::Span, offset: usize) -> bool {
    (span.start as usize) <= offset && offset <= span.end as usize
}

pub(super) fn range(source: &str, start: usize, end: usize) -> Value {
    json!({ "start": position_at(source, start), "end": position_at(source, end) })
}

pub(super) fn position_at(source: &str, offset: usize) -> Value {
    let offset = floor_boundary(source, offset.min(source.len()));
    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let character = source[line_start..offset].encode_utf16().count();
    json!({ "line": line, "character": character })
}

pub(super) fn offset_at(source: &str, position: &Value) -> Option<usize> {
    let line = position.get("line")?.as_u64()? as usize;
    let character = position.get("character")?.as_u64()? as usize;
    let line_start = if line == 0 {
        0
    } else {
        source
            .match_indices('\n')
            .nth(line - 1)
            .map(|(index, _)| index + 1)?
    };
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |index| line_start + index);
    let mut utf16 = 0;
    for (index, ch) in source[line_start..line_end].char_indices() {
        if utf16 >= character {
            return Some(line_start + index);
        }
        utf16 += ch.len_utf16();
        if utf16 >= character {
            return Some(line_start + index + ch.len_utf8());
        }
    }
    Some(line_end)
}

pub(super) fn floor_boundary(source: &str, mut offset: usize) -> usize {
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

pub(super) fn file_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    let path = path.strip_prefix("localhost").unwrap_or(path);
    Some(PathBuf::from(percent_decode(path)))
}

pub(super) fn file_uri(path: &std::path::Path) -> String {
    let mut encoded = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write as _;
            write!(encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    encoded
}

pub(super) fn percent_decode(input: &str) -> String {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            output.push(high * 16 + low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

pub(super) fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn publish(
    output: &mut impl Write,
    uri: &str,
    version: Option<i64>,
    diagnostics: Vec<Value>,
) -> io::Result<()> {
    send(
        output,
        json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": { "uri": uri, "version": version, "diagnostics": diagnostics } }),
    )
}

pub(super) fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let Some(content_length) = content_length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing Content-Length header",
        ));
    };
    let mut body = vec![0; content_length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(super) fn send(output: &mut impl Write, message: Value) -> io::Result<()> {
    let body = serde_json::to_vec(&message).expect("JSON-RPC messages are serializable");
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}
