use crate::parser::*;
use zutai_types::*;

#[test]
fn test_parse_empty() {
    let mut input = "{}";
    let output = parse(&mut input).unwrap();

    assert_eq!(output.len(), 0);
}

#[test]
fn test_parse_top_level_requires_block() {
    let mut input = "";
    assert!(parse(&mut input).is_err());
}

#[test]
fn test_parse_atom_body() {
    let mut input = "_hello123-abc";
    assert_eq!(parse_atom_body(&mut input).unwrap(), "_hello123-abc");
    assert_eq!(input, "");

    let mut invalid = "1bad";
    assert!(parse_atom_body(&mut invalid).is_err());
}

#[test]
fn test_parse_atom() {
    let mut input = "#_hello";
    assert_eq!(parse_atom(&mut input).unwrap(), "_hello");
    assert_eq!(input, "");

    let mut invalid = "_hello";
    assert!(parse_atom(&mut invalid).is_err());
}

#[test]
fn test_parse_value_literals() {
    let mut yes = "true";
    assert_eq!(parse_value(&mut yes).unwrap(), Value::True);

    let mut no = "false";
    assert_eq!(parse_value(&mut no).unwrap(), Value::False);

    let mut none_atom = "#none";
    assert_eq!(
        parse_value(&mut none_atom).unwrap(),
        Value::Atom("none".into())
    );

    let mut bare_none = "none";
    assert!(parse_value(&mut bare_none).is_err());
}

#[test]
fn test_parse_value_number_and_string_and_atom() {
    let mut num = "8080";
    assert_eq!(parse_value(&mut num).unwrap(), Value::Integer(8080));

    let mut neg_float = "-2.5e-3";
    assert!(
        matches!(parse_value(&mut neg_float).unwrap(), Value::Float(value) if (value + 0.0025).abs() < f64::EPSILON)
    );

    let mut string = "\"hello\\nworld\"";
    assert_eq!(
        parse_value(&mut string).unwrap(),
        Value::String("hello\nworld".to_string())
    );

    let mut atom = "#logging";
    assert_eq!(
        parse_value(&mut atom).unwrap(),
        Value::Atom("logging".to_string())
    );
}

#[test]
fn test_parse_block_and_array() {
    let mut input = "{\n  host = \"localhost\";\n  port = 8080;\n  tags = [#a;#b;];\n}";
    let parsed = parse(&mut input).unwrap();
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0].field_name, "host");
    assert_eq!(parsed[0].value, Value::String("localhost".to_string()));
    assert_eq!(parsed[1].field_name, "port");
    assert_eq!(parsed[1].value, Value::Integer(8080));
    assert_eq!(parsed[2].field_name, "tags");
    if let Value::Array(values) = &parsed[2].value {
        assert_eq!(values.len(), 2);
    } else {
        panic!("expected array value");
    }
    assert_eq!(input, "");
}

fn field<'a>(block: &'a Block, name: &str) -> &'a Value {
    block
        .iter()
        .find(|pair| pair.field_name == name)
        .map(|pair| &pair.value)
        .expect("missing field")
}

fn as_block(value: &Value) -> &Block {
    match value {
        Value::Block(block) => block,
        _ => panic!("expected block"),
    }
}

fn as_array(value: &Value) -> &Vec<Value> {
    match value {
        Value::Array(values) => values,
        _ => panic!("expected array"),
    }
}

fn as_string(value: &Value) -> &str {
    match value {
        Value::String(text) => text,
        _ => panic!("expected string"),
    }
}

fn as_atom(value: &Value) -> &str {
    match value {
        Value::Atom(text) => text,
        _ => panic!("expected atom"),
    }
}

fn as_integer(value: &Value) -> i64 {
    match value {
        Value::Integer(value) => *value,
        _ => panic!("expected integer"),
    }
}

fn as_float(value: &Value) -> f64 {
    match value {
        Value::Float(value) => *value,
        _ => panic!("expected float"),
    }
}

fn as_bool(value: &Value) -> bool {
    match value {
        Value::True => true,
        Value::False => false,
        _ => panic!("expected bool"),
    }
}

fn assert_float_eq(value: f64, expected: f64) {
    assert!((value - expected).abs() < 1e-12, "{value} != {expected}");
}

#[test]
fn test_parse_complex_fixture_matches_final_ast() {
    let mut input = include_str!("../../fixtures/complex.zti");
    let parsed = parse(&mut input).unwrap();

    assert_eq!(input, "");
    assert_eq!(parsed.len(), 4);

    assert_eq!(as_string(field(&parsed, "schema-version")), "0.4");
    assert_eq!(as_string(field(&parsed, "checksum")), "sha256:deadbeef");
    assert!(as_bool(field(&parsed, "validated")));

    let app = as_block(field(&parsed, "app"));
    assert_eq!(app.len(), 13);
    assert_eq!(as_string(field(&app, "name")), "Zutai Edge Service");
    assert_eq!(as_string(field(&app, "slug")), "zutai-edge");
    assert_eq!(as_integer(field(&app, "version")), 4);
    assert_float_eq(as_float(field(&app, "patch")), 0.11);

    let build = as_block(field(&app, "build"));
    assert_eq!(as_string(field(&build, "revision")), "2026.05.22");

    let compiler = as_block(field(&build, "compiler"));
    assert_eq!(as_string(field(&compiler, "name")), "zti-compiler");
    assert_eq!(
        as_string(field(&compiler, "target")),
        "x86_64-unknown-linux-gnu"
    );
    assert!(as_bool(field(&compiler, "incremental")));
    assert!(!as_bool(field(&compiler, "optimize")));
    assert_eq!(as_integer(field(&compiler, "optimization-level")), 3);

    let features = as_array(field(&compiler, "features"));
    assert_eq!(features.len(), 4);
    assert_eq!(as_atom(&features[0]), "fast-path");
    assert_eq!(as_atom(&features[1]), "arena-alloc");
    assert_eq!(as_atom(&features[2]), "inline-caches");
    assert_eq!(as_atom(&features[3]), "simd");

    let flags = as_array(field(&compiler, "flags"));
    assert_eq!(flags.len(), 5);
    assert_eq!(as_string(&flags[0]), "--deny=warnings");
    assert_eq!(as_string(&flags[1]), "--cap-lto=thin");
    assert_eq!(as_string(&flags[2]), "--codegen=unicode\nenabled");
    assert_eq!(as_string(&flags[3]), "json:\"log\"");
    assert_eq!(as_atom(&flags[4]), "none");

    let artifact = as_block(field(&build, "artifact"));
    let outputs = as_array(field(&artifact, "outputs"));
    assert_eq!(outputs.len(), 2);

    let wasm_output = as_block(&outputs[1]);
    assert_eq!(
        as_string(field(&wasm_output, "path")),
        "dist/zutai-edge.wasm"
    );
    assert_eq!(as_integer(field(&wasm_output, "size-bytes")), 81_920);
    assert_eq!(as_bool(field(&wasm_output, "compressed")), false);

    let runtime = as_block(field(&app, "runtime"));
    let service = as_block(field(&runtime, "service"));
    assert_eq!(as_atom(field(&service, "name")), "api-gateway");
    assert_eq!(as_atom(field(&service, "protocol")), "http");
    let endpoints = as_array(field(&service, "endpoints"));
    assert_eq!(endpoints.len(), 2);
    let status_endpoint = as_block(&endpoints[0]);
    assert_eq!(as_string(field(&status_endpoint, "name")), "status");
    assert_eq!(as_atom(field(&status_endpoint, "method")), "get");

    let submit_endpoint = as_block(&endpoints[1]);
    assert!(as_bool(field(&submit_endpoint, "auth")));
    let rate_limits = as_array(field(&submit_endpoint, "rate-limits"));
    assert_eq!(as_integer(&rate_limits[0]), 120);
    assert_float_eq(as_float(&rate_limits[1]), 10.0);

    let response = as_block(field(&submit_endpoint, "response"));
    assert_eq!(as_atom(field(&response, "schema")), "submission-result");

    let storage = as_block(field(&runtime, "storage"));
    let primary = as_block(field(&storage, "primary"));
    assert_eq!(as_atom(field(&primary, "backend")), "postgres");
    let shards = as_array(field(&primary, "shards"));
    assert_eq!(shards.len(), 3);
    assert_eq!(as_integer(field(as_block(&shards[2]), "id")), 2);

    let cache = as_block(field(&storage, "cache"));
    let nodes = as_array(field(&cache, "nodes"));
    assert_eq!(as_string(field(as_block(&nodes[0]), "host")), "cache-a");
    assert_float_eq(as_float(field(as_block(&nodes[0]), "weight")), 1.0);

    let observability = as_block(field(&runtime, "observability"));
    let metrics = as_block(field(&observability, "metrics"));
    assert_eq!(as_string(field(&metrics, "endpoint")), "/metrics");
    let collect = as_block(field(&metrics, "collect"));
    let gauges = as_array(field(&collect, "gauges"));
    assert_eq!(as_atom(&gauges[1]), "queue-depth");

    let scheduler = as_block(field(&app, "scheduler"));
    let workers = as_array(field(&scheduler, "workers"));
    assert_eq!(workers.len(), 2);
    let ingestion = as_block(&workers[0]);
    assert_eq!(as_atom(field(&ingestion, "name")), "ingestion");
    let ingestion_queues = as_array(field(&ingestion, "queues"));
    assert_eq!(
        as_string(field(as_block(&ingestion_queues[0]), "name")),
        "ingest.incoming"
    );

    let security = as_block(field(&app, "security"));
    let auth = as_block(field(&security, "auth"));
    let providers = as_array(field(&auth, "providers"));
    assert_eq!(providers.len(), 2);
    let jwt_provider = as_block(&providers[0]);
    assert_eq!(as_atom(field(&jwt_provider, "type")), "jwt");
    let audiences = as_array(field(&jwt_provider, "audiences"));
    assert_eq!(as_atom(&audiences[1]), "edge");

    let metadata = as_block(field(&app, "metadata"));
    assert_eq!(as_string(field(&metadata, "notes")), "Line1\\nLine2\nLine3",);
    assert_eq!(
        as_string(field(&metadata, "summary")),
        "Unicode and escapes: Hello 🚀",
    );

    let experiments = as_array(field(&app, "experiments"));
    let experiment = as_block(&experiments[1]);
    assert_eq!(as_string(field(&experiment, "id")), "EX-13");
    assert_float_eq(as_float(field(&experiment, "rollout")), 0.0);

    let diagnostics = as_block(field(&app, "diagnostics"));
    let checks = as_array(field(&diagnostics, "checks"));
    let first_check = as_block(&checks[0]);
    assert_eq!(as_string(field(&first_check, "name")), "schema-valid");
    assert!(as_bool(field(&first_check, "result")));

    let history = as_array(field(&diagnostics, "history"));
    assert_eq!(history.len(), 4);
    assert_eq!(as_atom(&history[3]), "none");
}

#[test]
fn test_parse_duplicate_field_names_error() {
    let mut input = "{a = 1; a = 2;}";
    assert!(parse(&mut input).is_err());
}

#[test]
fn test_block_display_tree() {
    let mut input = "{name = \"localhost\"; nested = {port = 8080;};}";
    let parsed = parse(&mut input).unwrap();

    assert_eq!(
        parsed.to_string(),
        "Block\n├─ name = String(\"localhost\")\n└─ nested = Block\n    └─ port = Integer(8080)\n"
    );
}

#[test]
fn test_block_display_string_escaping() {
    let block = Block(vec![Pair {
        field_name: "notes".to_string(),
        value: Value::String("line1\nline2\"slash\\backslash".to_string()),
    }]);

    assert_eq!(
        block.to_string(),
        "Block\n└─ notes = String(\"line1\\nline2\\\"slash\\\\backslash\")\n"
    );
}

#[test]
fn test_block_display_nested_block_in_array() {
    let block = Block(vec![Pair {
        field_name: "items".to_string(),
        value: Value::Array(vec![Value::Block(Block(vec![Pair {
            field_name: "kind".to_string(),
            value: Value::Atom("leaf".to_string()),
        }]))]),
    }]);

    assert_eq!(
        block.to_string(),
        "Block\n└─ items = Array[1]\n    └─ [0] = Block\n        └─ kind = Atom(leaf)\n"
    );
}
