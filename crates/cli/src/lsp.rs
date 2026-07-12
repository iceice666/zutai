//! A small, dependency-free Language Server Protocol implementation.
//!
//! The server intentionally owns only protocol/session state. Semantic work is
//! delegated to `zutai-semantic`, keeping the editor and CLI on the same
//! parse → HIR → THIR path.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::diagnostics::format_import_diagnostic;

pub(crate) fn run() -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut server = Server::default();

    while let Some(message) = read_message(&mut input)? {
        let should_exit = server.handle(message, &mut output)?;
        if should_exit {
            break;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, String>,
}

impl Server {
    fn handle(&mut self, message: Value, output: &mut impl Write) -> io::Result<bool> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(false);
        };
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => {
                if let Some(id) = id {
                    send(
                        output,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "capabilities": {
                                    "textDocumentSync": 1,
                                    "hoverProvider": true
                                },
                                "serverInfo": { "name": "zutai", "version": env!("CARGO_PKG_VERSION") }
                            }
                        }),
                    )?;
                }
            }
            "shutdown" => {
                if let Some(id) = id {
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": null }),
                    )?;
                }
            }
            "exit" => return Ok(true),
            "textDocument/didOpen" => {
                if let Some((uri, text)) = document_text(&params) {
                    self.documents.insert(uri.clone(), text);
                    self.publish_diagnostics(&uri, output)?;
                }
            }
            "textDocument/didChange" => {
                let uri = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let text = params
                    .get("contentChanges")
                    .and_then(Value::as_array)
                    .and_then(|changes| changes.last())
                    .and_then(|change| change.get("text"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let (Some(uri), Some(text)) = (uri, text) {
                    self.documents.insert(uri.clone(), text);
                    self.publish_diagnostics(&uri, output)?;
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) {
                    self.documents.remove(uri);
                    publish(output, uri, Vec::new())?;
                }
            }
            "textDocument/hover" => {
                if let Some(id) = id {
                    let result = self.hover(&params);
                    send(
                        output,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )?;
                }
            }
            _ => {
                if let Some(id) = id {
                    send(
                        output,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": format!("method not found: {method}") }
                        }),
                    )?;
                }
            }
        }
        Ok(false)
    }

    fn publish_diagnostics(&self, uri: &str, output: &mut impl Write) -> io::Result<()> {
        let Some(source) = self.source_for(uri) else {
            return publish(output, uri, Vec::new());
        };
        let diagnostics = analyze(&source, uri)
            .map(|analysis| diagnostics(&source, &analysis))
            .unwrap_or_default();
        publish(output, uri, diagnostics)
    }

    fn hover(&self, params: &Value) -> Value {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return Value::Null;
        };
        let Some(source) = self.source_for(uri) else {
            return Value::Null;
        };
        let Some(offset) = params
            .get("position")
            .and_then(|position| offset_at(&source, position))
        else {
            return Value::Null;
        };
        let Some(analysis) = analyze(&source, uri) else {
            return Value::Null;
        };
        let Some(file) = analysis.thir.and_then(|lowered| lowered.file) else {
            return Value::Null;
        };
        let expr = file
            .expr_arena
            .iter()
            .filter(|(_, expr)| contains(expr.span, offset))
            .min_by_key(|(_, expr)| expr.span.end.saturating_sub(expr.span.start))
            .map(|(_, expr)| expr);
        let Some(expr) = expr else {
            return Value::Null;
        };
        let contents = format!("```zutai\n{}\n```", render_type(&file, expr.ty));
        json!({
            "contents": { "kind": "markdown", "value": contents },
            "range": range(&source, expr.span.start as usize, expr.span.end as usize),
        })
    }

    fn source_for(&self, uri: &str) -> Option<String> {
        self.documents
            .get(uri)
            .cloned()
            .or_else(|| file_path(uri).and_then(|path| std::fs::read_to_string(path).ok()))
    }
}

fn document_text(params: &Value) -> Option<(String, String)> {
    let document = params.get("textDocument")?;
    Some((
        document.get("uri")?.as_str()?.to_owned(),
        document.get("text")?.as_str()?.to_owned(),
    ))
}

fn analyze(source: &str, uri: &str) -> Option<zutai_semantic::Analysis> {
    let path = file_path(uri)?;
    if path.extension().and_then(|ext| ext.to_str()) != Some("zt") {
        return None;
    }
    Some(zutai_semantic::analyze_with_base(
        source,
        path.parent(),
        zutai_semantic::AnalysisOptions::default(),
    ))
}

fn diagnostics(source: &str, analysis: &zutai_semantic::Analysis) -> Vec<Value> {
    analysis
        .diagnostics
        .iter()
        .map(|diagnostic| match &diagnostic.kind {
            zutai_semantic::SemanticDiagnosticKind::Parse(parse) => json!({
                "range": range(source, parse.primary_span().start as usize, parse.primary_span().end as usize),
                "severity": severity(parse.severity),
                "code": parse.code,
                "source": "zutai",
                "message": parse.message,
            }),
            zutai_semantic::SemanticDiagnosticKind::Import(import) => json!({
                "range": range(source, 0, 0),
                "severity": 1,
                "source": "zutai",
                "message": format_import_diagnostic(import),
            }),
            _ => {
                let (message, start, end) = zutai_eval::describe_semantic_diagnostic(diagnostic)
                    .expect("HIR and THIR diagnostics always have a source span");
                json!({
                    "range": range(source, start as usize, end as usize),
                    "severity": 1,
                    "source": "zutai",
                    "message": message,
                })
            }
        })
        .collect()
}

fn severity(severity: zutai_syntax::Severity) -> u8 {
    match severity {
        zutai_syntax::Severity::Error => 1,
        zutai_syntax::Severity::Warning => 2,
        zutai_syntax::Severity::Info => 3,
        zutai_syntax::Severity::Hint => 4,
    }
}

fn render_type(file: &zutai_thir::ThirFile, id: zutai_thir::TypeId) -> String {
    fn go(
        file: &zutai_thir::ThirFile,
        id: zutai_thir::TypeId,
        seen: &mut Vec<zutai_thir::TypeId>,
    ) -> String {
        if seen.contains(&id) {
            return "…".to_string();
        }
        let Some(ty) = file.type_arena.get(id.0 as usize) else {
            return "<invalid type>".to_string();
        };
        seen.push(id);
        let result = match &ty.kind {
            zutai_thir::TypeKind::Type(_) => "Type".to_string(),
            zutai_thir::TypeKind::Bool => "Bool".to_string(),
            zutai_thir::TypeKind::Text => "Text".to_string(),
            zutai_thir::TypeKind::Int => "Int".to_string(),
            zutai_thir::TypeKind::Float => "Float".to_string(),
            zutai_thir::TypeKind::FixedNum(width) => width.name().to_string(),
            zutai_thir::TypeKind::Posit(spec) => format!("{spec:?}"),
            zutai_thir::TypeKind::Opaque(name) => name.clone(),
            zutai_thir::TypeKind::Atom(name) => format!("#{name}"),
            zutai_thir::TypeKind::True => "true".to_string(),
            zutai_thir::TypeKind::False => "false".to_string(),
            zutai_thir::TypeKind::List(inner) => format!("List {}", go(file, *inner, seen)),
            zutai_thir::TypeKind::Optional(inner) => format!("{}?", go(file, *inner, seen)),
            zutai_thir::TypeKind::Maybe(inner) => format!("Maybe {}", go(file, *inner, seen)),
            zutai_thir::TypeKind::Patch { target, deep } => {
                format!(
                    "{}Patch {}",
                    if *deep { "Deep" } else { "" },
                    go(file, *target, seen)
                )
            }
            zutai_thir::TypeKind::Record(fields, tail) => {
                let mut fields: Vec<_> = fields
                    .iter()
                    .map(|field| {
                        format!(
                            "{}{}: {}",
                            field.name,
                            if field.optional { "?" } else { "" },
                            go(file, field.ty, seen)
                        )
                    })
                    .collect();
                if !matches!(tail, zutai_thir::RowTail::Closed) {
                    fields.push("...".to_string());
                }
                format!("{{ {} }}", fields.join("; "))
            }
            zutai_thir::TypeKind::Union(variants, tail) => {
                let mut variants: Vec<_> = variants
                    .iter()
                    .map(|variant| match variant.payload {
                        Some(payload) => format!("#{} ({})", variant.name, go(file, payload, seen)),
                        None => format!("#{}", variant.name),
                    })
                    .collect();
                if !matches!(tail, zutai_thir::RowTail::Closed) {
                    variants.push("...".to_string());
                }
                format!("<{}>", variants.join(" | "))
            }
            zutai_thir::TypeKind::Tuple(items) => format!(
                "({})",
                items
                    .iter()
                    .map(|item| match item {
                        zutai_thir::TypeTupleItem::Named { name, ty, .. } =>
                            format!("{name}: {}", go(file, *ty, seen)),
                        zutai_thir::TypeTupleItem::Positional(ty) => go(file, *ty, seen),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            zutai_thir::TypeKind::Function { from, to } => {
                format!("{} -> {}", go(file, *from, seen), go(file, *to, seen))
            }
            zutai_thir::TypeKind::Effect { base, row } => {
                let ops = row
                    .ops
                    .iter()
                    .map(|op| {
                        format!(
                            "{}: {} -> {}",
                            op.name,
                            go(file, op.param, seen),
                            go(file, op.result, seen)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("{} ! {{ {ops} }}", go(file, *base, seen))
            }
            zutai_thir::TypeKind::Never => "Never".to_string(),
            zutai_thir::TypeKind::TypeVar(binding)
            | zutai_thir::TypeKind::Alias(binding)
            | zutai_thir::TypeKind::Con(binding) => file
                .binding_names
                .get(binding.0 as usize)
                .cloned()
                .unwrap_or_else(|| format!("T{}", binding.0)),
            zutai_thir::TypeKind::InferVar(id) => format!("?{id}"),
            zutai_thir::TypeKind::AliasApply { binding, args } => format!(
                "{} {}",
                file.binding_names
                    .get(binding.0 as usize)
                    .cloned()
                    .unwrap_or_else(|| format!("T{}", binding.0)),
                args.iter()
                    .map(|arg| go(file, *arg, seen))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            zutai_thir::TypeKind::Apply { func, arg } => {
                format!("{} {}", go(file, *func, seen), go(file, *arg, seen))
            }
            zutai_thir::TypeKind::ForAll { params, body, .. } => format!(
                "<{}> {}",
                params
                    .iter()
                    .map(|binding| file
                        .binding_names
                        .get(binding.0 as usize)
                        .cloned()
                        .unwrap_or_else(|| format!("T{}", binding.0)))
                    .collect::<Vec<_>>()
                    .join(", "),
                go(file, *body, seen)
            ),
            zutai_thir::TypeKind::Error => "<type error>".to_string(),
        };
        seen.pop();
        result
    }
    go(file, id, &mut Vec::new())
}

fn contains(span: zutai_syntax::Span, offset: usize) -> bool {
    (span.start as usize) <= offset && offset <= span.end as usize
}

fn range(source: &str, start: usize, end: usize) -> Value {
    json!({ "start": position_at(source, start), "end": position_at(source, end) })
}

fn position_at(source: &str, offset: usize) -> Value {
    let offset = floor_boundary(source, offset.min(source.len()));
    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let character = source[line_start..offset].encode_utf16().count();
    json!({ "line": line, "character": character })
}

fn offset_at(source: &str, position: &Value) -> Option<usize> {
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

fn floor_boundary(source: &str, mut offset: usize) -> usize {
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn file_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    let path = path.strip_prefix("localhost").unwrap_or(path);
    Some(PathBuf::from(percent_decode(path)))
}

fn percent_decode(input: &str) -> String {
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

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn publish(output: &mut impl Write, uri: &str, diagnostics: Vec<Value>) -> io::Result<()> {
    send(
        output,
        json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": { "uri": uri, "diagnostics": diagnostics } }),
    )
}

fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
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

fn send(output: &mut impl Write, message: Value) -> io::Result<()> {
    let body = serde_json::to_vec(&message).expect("JSON-RPC messages are serializable");
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_use_utf16_code_units() {
        let source = "x😀\n終";
        assert_eq!(position_at(source, 5), json!({ "line": 0, "character": 3 }));
        assert_eq!(
            offset_at(source, &json!({ "line": 0, "character": 3 })),
            Some(5)
        );
    }

    #[test]
    fn parse_hover_and_publish_diagnostics() {
        let uri = "file:///tmp/example.zt";
        let mut server = Server::default();
        let mut output = Vec::new();
        let open = json!({ "method": "textDocument/didOpen", "params": { "textDocument": { "uri": uri, "text": "x ::= 1;\nx" } } });
        server.handle(open, &mut output).unwrap();
        assert!(String::from_utf8_lossy(&output).contains("publishDiagnostics"));

        let hover = server.hover(
            &json!({ "textDocument": { "uri": uri }, "position": { "line": 1, "character": 0 } }),
        );
        assert_eq!(
            hover.pointer("/contents/value").and_then(Value::as_str),
            Some("```zutai\nInt\n```")
        );
    }

    #[test]
    fn parser_diagnostic_includes_protocol_range() {
        let analysis = analyze("x ::= ;\nx", "file:///tmp/bad.zt").unwrap();
        let diagnostics = diagnostics("x ::= ;\nx", &analysis);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].get("range").is_some());
    }

    #[test]
    fn framing_round_trip() {
        let input = b"Content-Length: 17\r\n\r\n{\"method\":\"ping\"}";
        assert_eq!(
            read_message(&mut &input[..]).unwrap(),
            Some(json!({ "method": "ping" }))
        );
    }

    #[test]
    fn file_uris_preserve_absolute_paths() {
        assert_eq!(
            file_path("file:///tmp/example.zt"),
            Some(PathBuf::from("/tmp/example.zt"))
        );
        assert_eq!(
            file_path("file://localhost/tmp/example.zt"),
            Some(PathBuf::from("/tmp/example.zt"))
        );
    }
}
