use criterion::{black_box, criterion_group, criterion_main, Criterion};
use obsidian_termux_cli::vault::{VaultIndex, FileRecord, MarkdownMeta};

fn bench_resolve_note(c: &mut Criterion) {
    let mut index = VaultIndex::default();

    // Add 10,000 files to the index
    for i in 0..10000 {
        let path = format!("folder_{}/subfolder_{}/note_{}.md", i % 10, i % 100, i);
        index.files.insert(path.clone(), FileRecord {
            rel_path: path.clone(),
            len: 100,
            modified_ms: 0,
            is_markdown: true,
        });
        index.markdown.insert(path.clone(), MarkdownMeta::default());
        index.note_paths.push(obsidian_termux_cli::vault::NotePath::new(&path));
    }

    c.bench_function("resolve_note_not_found", |b| {
        b.iter(|| {
            let _ = index.resolve_note(black_box("NonExistentNote"), None);
        })
    });
}

criterion_group!(benches, bench_resolve_note);
criterion_main!(benches);
