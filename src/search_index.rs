use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::vault::{FileRecord, VaultContext};

pub const SEARCH_INDEX_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEngine {
    Scan,
    Index,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineResolution {
    Scan,
    Index,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchIndexStatus {
    pub version: u32,
    pub files: usize,
    pub bytes: u64,
    pub updated_ms: u64,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSearchIndex {
    version: u32,
    vault_hash: String,
    updated_ms: u64,
    files: BTreeMap<String, IndexedFile>,
    postings: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexedFile {
    len: u64,
    modified_ms: u64,
    text: String,
}

pub fn parse_search_engine(value: Option<&str>) -> Result<SearchEngine> {
    match value.unwrap_or("scan") {
        "scan" => Ok(SearchEngine::Scan),
        "index" => Ok(SearchEngine::Index),
        "auto" => Ok(SearchEngine::Auto),
        other => bail!("engine inválido: {other}. Usa `engine=scan|index|auto`"),
    }
}

pub fn resolve_engine(
    requested: SearchEngine,
    index_available: bool,
    is_fresh: bool,
) -> Result<EngineResolution> {
    match requested {
        SearchEngine::Scan => Ok(EngineResolution::Scan),
        SearchEngine::Index => {
            if !index_available {
                bail!("Índice no disponible; ejecuta `obsidian index:build`");
            }
            Ok(EngineResolution::Index)
        }
        SearchEngine::Auto => {
            if index_available && is_fresh {
                Ok(EngineResolution::Index)
            } else {
                Ok(EngineResolution::Scan)
            }
        }
    }
}

pub fn load(vault: &VaultContext) -> Result<StoredSearchIndex> {
    let path = search_index_path(vault);
    let raw = fs::read_to_string(&path).with_context(|| {
        format!(
            "Índice no disponible; ejecuta `obsidian index:build` ({})",
            path.display()
        )
    })?;
    let index: StoredSearchIndex = serde_json::from_str(&raw)
        .with_context(|| "índice corrupto; ejecuta `obsidian index:build` nuevamente")?;
    validate_index(vault, &index)?;
    Ok(index)
}

pub fn status(vault: &VaultContext) -> Result<SearchIndexStatus> {
    let index = load(vault)?;
    Ok(build_status(&index, &search_index_path(vault)))
}

pub fn clean(vault: &VaultContext) -> Result<bool> {
    let path = search_index_path(vault);
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(path)?;
    Ok(true)
}

pub fn build(vault: &VaultContext) -> Result<SearchIndexStatus> {
    let mut next = match load(vault) {
        Ok(existing) => existing,
        Err(_) => StoredSearchIndex {
            version: SEARCH_INDEX_VERSION,
            vault_hash: vault_hash(&vault.root),
            updated_ms: now_ms(),
            files: BTreeMap::new(),
            postings: BTreeMap::new(),
        },
    };

    let mut existing = std::mem::take(&mut next.files);
    let mut refreshed = BTreeMap::new();

    for file in vault.list_files(None, None)? {
        if !is_text_candidate(&file) {
            continue;
        }
        if let Some(prev) = existing.remove(&file.rel_path)
            && prev.len == file.len
            && prev.modified_ms == file.modified_ms
        {
            refreshed.insert(file.rel_path.clone(), prev);
            continue;
        }
        let text = vault.read_text(&file.rel_path).unwrap_or_default();
        refreshed.insert(
            file.rel_path.clone(),
            IndexedFile {
                len: file.len,
                modified_ms: file.modified_ms,
                text,
            },
        );
    }

    next.version = SEARCH_INDEX_VERSION;
    next.vault_hash = vault_hash(&vault.root);
    next.updated_ms = now_ms();
    next.files = refreshed;
    next.postings = rebuild_postings(&next.files);

    write_atomic(&search_index_path(vault), &serde_json::to_vec(&next)?)?;
    Ok(build_status(&next, &search_index_path(vault)))
}

pub fn is_fresh(vault: &VaultContext, index: &StoredSearchIndex) -> Result<bool> {
    validate_index(vault, index)?;
    let mut seen = BTreeSet::new();
    for file in vault.list_files(None, None)? {
        if !is_text_candidate(&file) {
            continue;
        }
        seen.insert(file.rel_path.clone());
        let Some(stored) = index.files.get(&file.rel_path) else {
            return Ok(false);
        };
        if stored.len != file.len || stored.modified_ms != file.modified_ms {
            return Ok(false);
        }
    }
    Ok(index.files.keys().all(|path| seen.contains(path)))
}

pub fn search(
    index: &StoredSearchIndex,
    query: &str,
    case_sensitive: bool,
    scope: Option<&str>,
    with_context: bool,
    limit: usize,
) -> Vec<SearchMatch> {
    let prepared = if case_sensitive {
        query.to_string()
    } else {
        query.to_ascii_lowercase()
    };
    let mut paths = candidate_paths(index, &prepared);
    paths.sort();
    let mut hits = Vec::new();
    let mut seen_files = BTreeSet::new();
    for path in paths {
        if hits.len() >= limit {
            break;
        }
        if let Some(scope) = scope
            && !path.starts_with(scope)
        {
            continue;
        }
        let Some(file) = index.files.get(&path) else {
            continue;
        };
        for (line_idx, line) in file.text.lines().enumerate() {
            if !contains_query(line, &prepared, case_sensitive) {
                continue;
            }
            if with_context {
                hits.push(SearchMatch {
                    path: path.clone(),
                    line: line_idx + 1,
                    text: line.to_string(),
                });
            } else if seen_files.insert(path.clone()) {
                hits.push(SearchMatch {
                    path: path.clone(),
                    line: line_idx + 1,
                    text: String::new(),
                });
            }
            if hits.len() >= limit {
                break;
            }
        }
    }
    hits
}

pub fn contains_query(line: &str, prepared_query: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        line.contains(prepared_query)
    } else {
        line.to_ascii_lowercase().contains(prepared_query)
    }
}

fn validate_index(vault: &VaultContext, index: &StoredSearchIndex) -> Result<()> {
    if index.version != SEARCH_INDEX_VERSION {
        bail!("versión de índice incompatible; ejecuta `obsidian index:build`");
    }
    if index.vault_hash != vault_hash(&vault.root) {
        bail!("índice pertenece a otro vault; ejecuta `obsidian index:build`");
    }
    Ok(())
}

fn candidate_paths(index: &StoredSearchIndex, query: &str) -> Vec<String> {
    if query.chars().count() < 3 {
        return index.files.keys().cloned().collect();
    }
    let grams = trigrams(query);
    let mut iter = grams.into_iter();
    let Some(first) = iter.next() else {
        return index.files.keys().cloned().collect();
    };
    let mut acc: BTreeSet<String> = index
        .postings
        .get(&first)
        .map(|items| items.iter().cloned().collect())
        .unwrap_or_default();
    for gram in iter {
        let current: BTreeSet<String> = index
            .postings
            .get(&gram)
            .map(|items| items.iter().cloned().collect())
            .unwrap_or_default();
        acc = acc.intersection(&current).cloned().collect();
        if acc.is_empty() {
            break;
        }
    }
    acc.into_iter().collect()
}

fn rebuild_postings(files: &BTreeMap<String, IndexedFile>) -> BTreeMap<String, Vec<String>> {
    let mut postings = BTreeMap::<String, BTreeSet<String>>::new();
    for (path, file) in files {
        for gram in trigrams(&file.text.to_ascii_lowercase()) {
            postings.entry(gram).or_default().insert(path.clone());
        }
    }
    postings
        .into_iter()
        .map(|(gram, paths)| (gram, paths.into_iter().collect()))
        .collect()
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
    for i in 0..=(chars.len() - 3) {
        grams.insert(chars[i..i + 3].iter().collect());
    }
    grams
}

fn search_index_path(vault: &VaultContext) -> PathBuf {
    vault
        .root
        .join(".obsidian")
        .join("plugins")
        .join("obsidian-termux-cli")
        .join(format!("search-index-{}.json", vault_hash(&vault.root)))
}

fn build_status(index: &StoredSearchIndex, path: &Path) -> SearchIndexStatus {
    SearchIndexStatus {
        version: index.version,
        files: index.files.len(),
        bytes: index.files.values().map(|file| file.len).sum(),
        updated_ms: index.updated_ms,
        path: path.to_string_lossy().to_string(),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn vault_hash(root: &Path) -> String {
    let mut sha1 = Sha1::new();
    sha1.update(root.to_string_lossy().as_bytes());
    format!("{:x}", sha1.finalize())
}

fn is_text_candidate(file: &FileRecord) -> bool {
    if file.is_markdown {
        return true;
    }
    matches!(
        Path::new(&file.rel_path)
            .extension()
            .and_then(|value| value.to_str()),
        Some("txt" | "md" | "markdown" | "json" | "yaml" | "yml" | "csv" | "tsv")
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::Workspace;
    use std::fs;

    #[test]
    fn parses_engine_values() {
        assert_eq!(parse_search_engine(None).unwrap(), SearchEngine::Scan);
        assert_eq!(
            parse_search_engine(Some("scan")).unwrap(),
            SearchEngine::Scan
        );
        assert_eq!(
            parse_search_engine(Some("index")).unwrap(),
            SearchEngine::Index
        );
        assert_eq!(
            parse_search_engine(Some("auto")).unwrap(),
            SearchEngine::Auto
        );
        assert!(parse_search_engine(Some("foo")).is_err());
    }

    #[test]
    fn auto_fallback_and_index_error() {
        assert_eq!(
            resolve_engine(SearchEngine::Auto, false, false).unwrap(),
            EngineResolution::Scan
        );
        assert_eq!(
            resolve_engine(SearchEngine::Auto, true, true).unwrap(),
            EngineResolution::Index
        );
        assert!(resolve_engine(SearchEngine::Index, false, false).is_err());
    }

    #[test]
    fn scan_and_index_consistency_simple() {
        let root = std::env::temp_dir().join(format!("obsidian-index-test-{}", now_ms()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::write(root.join("a.md"), "hola\notro\ntexto").unwrap();
        fs::write(root.join("b.md"), "nada\nhola mundo").unwrap();

        let workspace = Workspace::load(root.clone()).unwrap();
        let vault = workspace.open_vault(None).unwrap();

        build(&vault).unwrap();
        let index = load(&vault).unwrap();
        let index_hits = search(&index, "hola", false, None, true, usize::MAX);

        let mut scan_hits = Vec::new();
        for file in vault.list_files(None, None).unwrap() {
            let text = vault.read_text(&file.rel_path).unwrap();
            for (line_idx, line) in text.lines().enumerate() {
                if contains_query(line, "hola", false) {
                    scan_hits.push((file.rel_path.clone(), line_idx + 1));
                }
            }
        }

        let index_rows = index_hits
            .into_iter()
            .map(|hit| (hit.path, hit.line))
            .collect::<Vec<_>>();
        assert_eq!(index_rows, scan_hits);

        let _ = fs::remove_dir_all(root);
    }
}
