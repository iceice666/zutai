use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

const COMPLEX: &str = include_str!("../../fixtures/complex.zti");

fn bench_immediate_parse(c: &mut Criterion) {
    let generated = generated_document(512);
    let string_heavy = generated_string_document(256, 256);
    let inputs = [
        ("complex_fixture", COMPLEX),
        ("generated_512_fields", generated.as_str()),
        ("string_heavy_256x256", string_heavy.as_str()),
    ];

    let mut group = c.benchmark_group("immediate_parse");
    group.sample_size(20);

    for (name, input) in inputs {
        group.throughput(Throughput::Bytes(input.len() as u64));

        #[cfg(target_arch = "x86_64")]
        {
            group.bench_with_input(
                BenchmarkId::new("sse2_parse", name),
                input,
                |bench, input| {
                    bench.iter(|| {
                        let parsed = zutai_im_simd::parse_sse2(black_box(input))
                            .expect("parse should succeed");
                        black_box(parsed);
                    });
                },
            );

            if std::is_x86_feature_detected!("avx2") {
                group.bench_with_input(
                    BenchmarkId::new("avx2_parse", name),
                    input,
                    |bench, input| {
                        bench.iter(|| {
                            // SAFETY: AVX2 support confirmed by the guard above.
                            let parsed = unsafe { zutai_im_simd::parse_avx2(black_box(input)) }
                                .expect("parse should succeed");
                            black_box(parsed);
                        });
                    },
                );
            }
        }

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

fn generated_string_document(fields: usize, string_len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";

    let mut payload = String::with_capacity(string_len);
    for index in 0..string_len {
        payload.push(ALPHABET[index % ALPHABET.len()] as char);
    }

    let mut document = String::from("{\n");
    for index in 0..fields {
        document.push_str("  string-");
        document.push_str(&index.to_string());
        document.push_str(" = \"");
        document.push_str(&payload);
        document.push_str("\";\n");
    }
    document.push_str("}\n");
    document
}

criterion_group!(benches, bench_immediate_parse);
criterion_main!(benches);
