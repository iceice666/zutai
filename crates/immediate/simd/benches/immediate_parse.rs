use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

const COMPLEX: &str = include_str!("../../fixtures/complex.zti");

fn bench_immediate_parse(c: &mut Criterion) {
    let generated = generated_document(512);
    let inputs = [
        ("complex_fixture", COMPLEX),
        ("generated_512_fields", generated.as_str()),
    ];

    let mut group = c.benchmark_group("immediate_parse");
    group.sample_size(20);

    for (name, input) in inputs {
        group.throughput(Throughput::Bytes(input.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("simd_scan", name),
            input,
            |bench, input| {
                bench.iter(|| {
                    let index = zutai_im_simd::scan(black_box(input)).expect("scan should succeed");
                    black_box(index);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("simd_parse", name),
            input,
            |bench, input| {
                bench.iter(|| {
                    let parsed =
                        zutai_im_simd::parse(black_box(input)).expect("parse should succeed");
                    black_box(parsed);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("winnow_parse", name),
            input,
            |bench, input| {
                bench.iter(|| {
                    let mut parser_input: &str = black_box(input);
                    let parsed = zutai_im_syntax::parser::parse(&mut parser_input)
                        .expect("parse should succeed");
                    black_box(parsed);
                });
            },
        );
    }

    group.finish();
}

fn generated_document(fields: usize) -> String {
    let mut document = String::from("{\n");

    for index in 0..fields {
        document.push_str("  field-");
        document.push_str(&index.to_string());
        document.push_str(" = {\n");
        document.push_str("    name = \"service-");
        document.push_str(&index.to_string());
        document.push_str("\";\n");
        document.push_str("    enabled = true;\n");
        document.push_str("    weight = ");
        document.push_str(&(index + 1).to_string());
        document.push_str(".25;\n");
        document.push_str(
            "    tags = [#fast-path; #simd; \"brace } semicolon ; quote \\\"\"; #none;];\n",
        );
        document.push_str("  };\n");
    }

    document.push_str("}\n");
    document
}

criterion_group!(benches, bench_immediate_parse);
criterion_main!(benches);
