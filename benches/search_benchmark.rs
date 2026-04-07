use std::collections::{BTreeMap, BTreeSet};

use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn build_dataset(files: usize, lines_per_file: usize) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for file_idx in 0..files {
        let mut text = String::new();
        for line_idx in 0..lines_per_file {
            if file_idx % 13 == 0 && line_idx % 17 == 0 {
                text.push_str(&format!(
                    "line {line_idx} contains obsidian termux search target\\n"
                ));
            } else {
                text.push_str(&format!(
                    "line {line_idx} lorem ipsum dolor sit amet {file_idx}\\n"
                ));
            }
        }
        rows.push((format!("note-{file_idx}.md"), text));
    }
    rows
}

fn scan_search(dataset: &[(String, String)], query: &str) -> usize {
    let mut hits = 0;
    let query = query.to_ascii_lowercase();
    for (_, text) in dataset {
        for line in text.lines() {
            if line.to_ascii_lowercase().contains(&query) {
                hits += 1;
            }
        }
    }
    hits
}

fn trigram_index(dataset: &[(String, String)]) -> BTreeMap<String, BTreeSet<String>> {
    let mut postings = BTreeMap::<String, BTreeSet<String>>::new();
    for (path, text) in dataset {
        for gram in trigrams(&text.to_ascii_lowercase()) {
            postings.entry(gram).or_default().insert(path.clone());
        }
    }
    postings
}

fn index_search(
    dataset: &[(String, String)],
    postings: &BTreeMap<String, BTreeSet<String>>,
    query: &str,
) -> usize {
    let query = query.to_ascii_lowercase();
    let mut grams = trigrams(&query).into_iter();
    let Some(first) = grams.next() else {
        return 0;
    };
    let mut candidates = postings.get(&first).cloned().unwrap_or_default();
    for gram in grams {
        let current = postings.get(&gram).cloned().unwrap_or_default();
        candidates = candidates.intersection(&current).cloned().collect();
        if candidates.is_empty() {
            return 0;
        }
    }

    let mut hits = 0;
    for (path, text) in dataset {
        if !candidates.contains(path) {
            continue;
        }
        for line in text.lines() {
            if line.to_ascii_lowercase().contains(&query) {
                hits += 1;
            }
        }
    }
    hits
}

fn trigrams(text: &str) -> BTreeSet<String> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut grams = BTreeSet::new();
    if chars.len() < 3 {
        if !text.is_empty() {
            grams.insert(text.to_string());
        }
        return grams;
    }
    for idx in 0..=(chars.len() - 3) {
        grams.insert(chars[idx..idx + 3].iter().collect());
    }
    grams
}

fn bench_scan_vs_index(c: &mut Criterion) {
    let dataset = build_dataset(250, 120);
    let postings = trigram_index(&dataset);
    let query = "search target";

    c.bench_function("scan_search_synthetic", |b| {
        b.iter(|| black_box(scan_search(black_box(&dataset), black_box(query))))
    });

    c.bench_function("index_search_synthetic", |b| {
        b.iter(|| {
            black_box(index_search(
                black_box(&dataset),
                black_box(&postings),
                black_box(query),
            ))
        })
    });
}

criterion_group!(benches, bench_scan_vs_index);
criterion_main!(benches);
