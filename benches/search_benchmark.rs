use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn contains_query_old(line: &str, query: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        line.contains(query)
    } else {
        line.to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
    }
}

fn contains_query_new(line: &str, query: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        line.contains(query)
    } else {
        line.to_ascii_lowercase().contains(query)
    }
}

fn bench_search(c: &mut Criterion) {
    let line = "This is a very long line with some TEXT inside it that we want to search for, testing performance of different approaches.";
    let query_old = "text";
    let query_new = "text";

    c.bench_function("contains_query_old", |b| {
        b.iter(|| {
            black_box(contains_query_old(
                black_box(line),
                black_box(query_old),
                black_box(false),
            ))
        })
    });

    c.bench_function("contains_query_new", |b| {
        b.iter(|| {
            black_box(contains_query_new(
                black_box(line),
                black_box(query_new),
                black_box(false),
            ))
        })
    });
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
