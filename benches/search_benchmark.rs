use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::ffi::OsStr;
use std::path::Path;

// Represents the filter inside resolve_note
fn resolve_note_filter_old(candidate: &str, normalized: &str, stem: &str) -> bool {
    let candidate_no_ext = candidate.trim_end_matches(".md");
    let candidate_name = Path::new(candidate_no_ext)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(candidate_no_ext)
        .to_ascii_lowercase();
    candidate.eq_ignore_ascii_case(&normalized)
        || candidate_no_ext.eq_ignore_ascii_case(&normalized)
        || candidate_name == stem
        || candidate
            .to_ascii_lowercase()
            .ends_with(&format!("/{normalized}"))
        || candidate_no_ext
            .to_ascii_lowercase()
            .ends_with(&format!("/{normalized}"))
}

// Represents an optimized filter
fn resolve_note_filter_new(candidate: &str, normalized: &str, stem: &str) -> bool {
    let candidate_no_ext = candidate.trim_end_matches(".md");

    // Quick exact matches
    if candidate.eq_ignore_ascii_case(normalized)
        || candidate_no_ext.eq_ignore_ascii_case(normalized)
    {
        return true;
    }

    // Stem match
    let candidate_name = Path::new(candidate_no_ext)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(candidate_no_ext);

    if candidate_name.eq_ignore_ascii_case(stem) {
        return true;
    }

    // Suffix match check using bytes
    let norm_bytes = normalized.as_bytes();
    let norm_len = norm_bytes.len();
    if norm_len == 0 {
        return false;
    }

    let cand_bytes = candidate.as_bytes();
    if cand_bytes.len() > norm_len + 1
        && cand_bytes[cand_bytes.len() - norm_len - 1] == b'/'
        && cand_bytes[cand_bytes.len() - norm_len..].eq_ignore_ascii_case(norm_bytes)
    {
        return true;
    }

    let cand_no_ext_bytes = candidate_no_ext.as_bytes();
    if cand_no_ext_bytes.len() > norm_len + 1
        && cand_no_ext_bytes[cand_no_ext_bytes.len() - norm_len - 1] == b'/'
        && cand_no_ext_bytes[cand_no_ext_bytes.len() - norm_len..].eq_ignore_ascii_case(norm_bytes)
    {
        return true;
    }

    false
}

fn bench_path(c: &mut Criterion) {
    let candidate1 = "Folder/Subfolder/my Note.md";
    let normalized1 = "my note"; // matches stem
    let stem1 = "my note";

    let candidate2 = "Folder/Subfolder/my Note.md";
    let normalized2 = "subfolder/my note"; // matches format!("/{normalized}") on candidate_no_ext
    let stem2 = "my note";

    let candidate3 = "Folder/Subfolder/my Note.md";
    let normalized3 = "folder/subfolder/my note";
    let stem3 = "my note";

    let candidate4 = "Folder/Subfolder/my Note.md";
    let normalized4 = "some other note"; // no match
    let stem4 = "some other note";

    let mut group = c.benchmark_group("resolve_note_filter");

    group.bench_function("old_stem", |b| {
        b.iter(|| {
            black_box(resolve_note_filter_old(
                black_box(candidate1),
                black_box(normalized1),
                black_box(stem1),
            ))
        })
    });

    group.bench_function("new_stem", |b| {
        b.iter(|| {
            black_box(resolve_note_filter_new(
                black_box(candidate1),
                black_box(normalized1),
                black_box(stem1),
            ))
        })
    });

    group.bench_function("old_suffix", |b| {
        b.iter(|| {
            black_box(resolve_note_filter_old(
                black_box(candidate2),
                black_box(normalized2),
                black_box(stem2),
            ))
        })
    });

    group.bench_function("new_suffix", |b| {
        b.iter(|| {
            black_box(resolve_note_filter_new(
                black_box(candidate2),
                black_box(normalized2),
                black_box(stem2),
            ))
        })
    });

    group.bench_function("old_exact", |b| {
        b.iter(|| {
            black_box(resolve_note_filter_old(
                black_box(candidate3),
                black_box(normalized3),
                black_box(stem3),
            ))
        })
    });

    group.bench_function("new_exact", |b| {
        b.iter(|| {
            black_box(resolve_note_filter_new(
                black_box(candidate3),
                black_box(normalized3),
                black_box(stem3),
            ))
        })
    });

    group.bench_function("old_no_match", |b| {
        b.iter(|| {
            black_box(resolve_note_filter_old(
                black_box(candidate4),
                black_box(normalized4),
                black_box(stem4),
            ))
        })
    });

    group.bench_function("new_no_match", |b| {
        b.iter(|| {
            black_box(resolve_note_filter_new(
                black_box(candidate4),
                black_box(normalized4),
                black_box(stem4),
            ))
        })
    });

    group.finish();
}

criterion_group!(benches, bench_path);
criterion_main!(benches);
