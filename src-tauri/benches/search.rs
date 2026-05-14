use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("fuzzy_match_name", |b| {
        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let query = "quikfind";
        let name = "QuikFind.App.exe";

        b.iter(|| {
            black_box(
                matcher
                    .fuzzy_match(
                        nucleo::Utf32Str::Ascii(black_box(query).as_bytes()),
                        nucleo::Utf32Str::Ascii(black_box(name).as_bytes()),
                    )
                    .is_some(),
            );
        });
    });

    c.bench_function("compute_file_id", |b| {
        b.iter(|| {
            let input = format!(
                "{path}:{size}:{modified}",
                path = black_box("/path/to/file.txt"),
                size = black_box(1024u64),
                modified = black_box(1_700_000_000i64),
            );
            black_box(blake3::hash(input.as_bytes()));
        });
    });

    c.bench_function("simple_fuzzy_scoring", |b| {
        b.iter(|| {
            let query = "brwsr";
            let names = [
                "Browser",
                "BrowserDev",
                "Firefox Browser",
                "Chrome Browser",
                "Brave Browser",
            ];
            for name in &names {
                let q: Vec<char> = query.chars().collect();
                let n: Vec<char> = name.to_lowercase().chars().collect();
                let mut qi = 0;
                let mut score = 0.0f32;
                let mut consecutive = 0.0f32;
                for &nc in &n {
                    if qi < q.len() && nc == q[qi] {
                        qi += 1;
                        consecutive += 1.0;
                        score += consecutive;
                    } else {
                        consecutive = 0.0;
                    }
                }
                #[allow(clippy::cast_precision_loss)]
                black_box(if qi == q.len() { score / n.len() as f32 } else { 0.0 });
            }
        });
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
