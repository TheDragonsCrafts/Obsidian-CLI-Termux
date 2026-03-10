use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use walkdir::WalkDir;

static HEADING_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(#{1,6})\s+(?P<text>.+?)\s*$").expect("heading regex"));
static TASK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(?:[-*+]|\d+\.)\s+\[(?P<status>.)\]\s+(?P<text>.*)$").expect("task regex")
});
static WIKILINK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[\[(?P<link>[^\]|#]+)(?:#[^\]|]+)?(?:\|[^\]]+)?\]\]").expect("wikilink regex")
});
static MD_LINK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[[^\]]+\]\((?P<link>[^)]+)\)").expect("markdown link regex"));
static TAG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:^|[^[:alnum:]_/])(?P<tag>#(?:[[:alnum:]_-]+(?:/[[:alnum:]_-]+)*))")
        .expect("tag regex")
});

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub base_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub state_file: PathBuf,
    pub history_file: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownVault {
    pub name: String,
    pub path: String,
}

impl KnownVault {
    pub fn path_buf(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentEntry {
    pub vault_path: String,
    pub path: String,
    pub opened_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionState {
    pub active_vault_path: Option<String>,
    pub active_file: Option<String>,
    pub recents: Vec<RecentEntry>,
}

#[derive(Debug)]
pub struct Workspace {
    pub cwd: PathBuf,
    pub runtime: RuntimePaths,
    pub known_vaults: Vec<KnownVault>,
    pub state: SessionState,
}

impl Workspace {
    pub fn load(cwd: PathBuf) -> Result<Self> {
        let runtime = runtime_paths()?;
        fs::create_dir_all(&runtime.base_dir)?;
        fs::create_dir_all(&runtime.cache_dir)?;

        let state = if runtime.state_file.exists() {
            serde_json::from_str::<SessionState>(&fs::read_to_string(&runtime.state_file)?)
                .unwrap_or_default()
        } else {
            SessionState::default()
        };

        let mut known_vaults = discover_known_vaults();
        known_vaults.sort_by(|left, right| left.name.cmp(&right.name));
        known_vaults.dedup_by(|left, right| left.path == right.path);

        Ok(Self {
            cwd,
            runtime,
            known_vaults,
            state,
        })
    }

    pub fn save(&self) -> Result<()> {
        let body = serde_json::to_string_pretty(&self.state)?;
        fs::write(&self.runtime.state_file, body)?;
        Ok(())
    }

    pub fn open_vault(&self, selector: Option<&str>) -> Result<VaultContext> {
        let vault = self.resolve_vault(selector)?;
        Ok(VaultContext::new(self.runtime.clone(), vault))
    }

    pub fn resolve_vault(&self, selector: Option<&str>) -> Result<KnownVault> {
        if let Some(selector) = selector {
            return self.match_vault(selector);
        }

        if let Some(path) = find_vault_ancestor(&self.cwd) {
            return Ok(KnownVault {
                name: vault_name(&path),
                path: path.to_string_lossy().to_string(),
            });
        }

        if let Some(path) = self.state.active_vault_path.as_deref() {
            let path_buf = PathBuf::from(path);
            if path_buf.join(".obsidian").exists() {
                return Ok(KnownVault {
                    name: vault_name(&path_buf),
                    path: path.to_string(),
                });
            }
        }

        match self.known_vaults.as_slice() {
            [single] => Ok(single.clone()),
            [] => bail!(
                "no se pudo resolver el vault. Usa `vault=<name>` o ejecuta la CLI dentro de un vault"
            ),
            _ => bail!("hay varios vaults conocidos; usa `vault=<name>`"),
        }
    }

    pub fn set_active_vault(&mut self, vault: &VaultContext) {
        self.state.active_vault_path = Some(vault.root.to_string_lossy().to_string());
    }

    pub fn set_active_file(&mut self, vault: &VaultContext, rel_path: &str) {
        self.set_active_vault(vault);
        self.state.active_file = Some(rel_path.to_string());
        self.record_recent(vault, rel_path);
    }

    pub fn clear_active_file(&mut self, vault: &VaultContext, rel_path: &str) {
        if self.state.active_vault_path.as_deref() == Some(vault.root.to_string_lossy().as_ref())
            && self.state.active_file.as_deref() == Some(rel_path)
        {
            self.state.active_file = None;
        }
    }

    pub fn active_file_for(&self, vault: &VaultContext) -> Option<String> {
        if self.state.active_vault_path.as_deref() != Some(vault.root.to_string_lossy().as_ref()) {
            return None;
        }
        self.state.active_file.clone()
    }

    fn record_recent(&mut self, vault: &VaultContext, rel_path: &str) {
        let stamp = Local::now().to_rfc3339();
        self.state.recents.retain(|entry| {
            !(entry.vault_path == vault.root.to_string_lossy() && entry.path == rel_path)
        });
        self.state.recents.insert(
            0,
            RecentEntry {
                vault_path: vault.root.to_string_lossy().to_string(),
                path: rel_path.to_string(),
                opened_at: stamp,
            },
        );
        self.state.recents.truncate(50);
    }

    fn match_vault(&self, selector: &str) -> Result<KnownVault> {
        let selector_lower = selector.to_ascii_lowercase();
        let mut matches = self
            .known_vaults
            .iter()
            .filter(|vault| {
                vault.name.eq_ignore_ascii_case(selector)
                    || vault.path.eq_ignore_ascii_case(selector)
                    || vault
                        .path
                        .to_ascii_lowercase()
                        .ends_with(&selector_lower.replace('\\', "/"))
            })
            .cloned()
            .collect::<Vec<_>>();

        if matches.is_empty() {
            let path = PathBuf::from(selector);
            if path.join(".obsidian").exists() {
                return Ok(KnownVault {
                    name: vault_name(&path),
                    path: path.to_string_lossy().to_string(),
                });
            }
            bail!("vault no encontrado: {selector}");
        }

        if matches.len() > 1 {
            bail!("selector de vault ambiguo: {selector}");
        }

        Ok(matches.remove(0))
    }
}

#[derive(Debug, Clone)]
pub struct VaultContext {
    pub name: String,
    pub root: PathBuf,
    pub obsidian_dir: PathBuf,
    cache_file: PathBuf,
}

impl VaultContext {
    fn new(runtime: RuntimePaths, vault: KnownVault) -> Self {
        let root = vault.path_buf();
        let obsidian_dir = root.join(".obsidian");
        let cache_file = runtime
            .cache_dir
            .join(format!("{}.json", vault_hash(&root)));
        Self {
            name: vault.name,
            root,
            obsidian_dir,
            cache_file,
        }
    }

    pub fn rel_to_abs(&self, rel_path: &str) -> Result<PathBuf> {
        let rel_path = normalize_rel_path(rel_path);
        let abs = self.root.join(rel_path);
        if !abs.starts_with(&self.root) {
            bail!("path fuera del vault");
        }
        Ok(abs)
    }

    pub fn list_files(&self, folder: Option<&str>, ext: Option<&str>) -> Result<Vec<FileRecord>> {
        let root = match folder {
            Some(folder) => self.rel_to_abs(folder)?,
            None => self.root.clone(),
        };
        if !root.exists() {
            bail!("la carpeta no existe");
        }

        let ext = ext.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
        let mut files = Vec::new();

        for entry in WalkDir::new(root)
            .into_iter()
            .filter_entry(|entry| should_walk(entry.path()))
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let abs = entry.path().to_path_buf();
            let rel = rel_from_root(&self.root, &abs)?;
            if let Some(ext) = ext.as_deref() {
                let current = abs
                    .extension()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if current != ext {
                    continue;
                }
            }
            let meta = entry.metadata()?;
            files.push(FileRecord {
                rel_path: rel,
                len: meta.len(),
                modified_ms: to_millis(meta.modified().ok()),
                is_markdown: is_markdown_path(&abs),
            });
        }

        files.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
        Ok(files)
    }

    pub fn list_folders(&self, folder: Option<&str>) -> Result<Vec<String>> {
        let root = match folder {
            Some(folder) => self.rel_to_abs(folder)?,
            None => self.root.clone(),
        };
        if !root.exists() {
            bail!("la carpeta no existe");
        }

        let mut folders = BTreeSet::new();
        for entry in WalkDir::new(root)
            .min_depth(1)
            .into_iter()
            .filter_entry(|entry| should_walk(entry.path()))
        {
            let entry = entry?;
            if entry.file_type().is_dir() {
                folders.insert(rel_from_root(&self.root, entry.path())?);
            }
        }
        Ok(folders.into_iter().collect())
    }

    pub fn read_text(&self, rel_path: &str) -> Result<String> {
        let abs = self.rel_to_abs(rel_path)?;
        fs::read_to_string(&abs).with_context(|| format!("no se pudo leer {rel_path}"))
    }

    pub fn write_text(&self, rel_path: &str, content: &str, overwrite: bool) -> Result<()> {
        let abs = self.rel_to_abs(rel_path)?;
        if abs.exists() && !overwrite {
            bail!("el archivo ya existe; usa `overwrite` para reemplazarlo");
        }
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(abs, content)?;
        Ok(())
    }

    pub fn append_text(&self, rel_path: &str, content: &str, inline: bool) -> Result<()> {
        let mut current = self.read_text(rel_path)?;
        if !inline && !current.is_empty() && !current.ends_with('\n') {
            current.push('\n');
        }
        current.push_str(content);
        fs::write(self.rel_to_abs(rel_path)?, current)?;
        Ok(())
    }

    pub fn prepend_text(&self, rel_path: &str, content: &str, inline: bool) -> Result<()> {
        let current = self.read_text(rel_path)?;
        let (frontmatter, body) = split_frontmatter(&current);
        let mut next = String::new();

        if let Some(frontmatter) = frontmatter {
            next.push_str(&frontmatter);
        }

        if !inline && !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(content);

        if !inline && !content.ends_with('\n') && !body.is_empty() {
            next.push('\n');
        }
        next.push_str(body);
        fs::write(self.rel_to_abs(rel_path)?, next)?;
        Ok(())
    }

    pub fn move_path(&self, from_rel: &str, to_rel: &str) -> Result<String> {
        let from_abs = self.rel_to_abs(from_rel)?;
        let to_rel = normalize_rel_path(to_rel);
        let to_abs = self.rel_to_abs(&to_rel)?;
        if !from_abs.exists() {
            bail!("el archivo no existe: {from_rel}");
        }
        if let Some(parent) = to_abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(from_abs, to_abs)?;
        Ok(to_rel)
    }

    pub fn rename_path(&self, from_rel: &str, name: &str) -> Result<String> {
        let from_abs = self.rel_to_abs(from_rel)?;
        let parent = Path::new(from_rel)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let current_ext = Path::new(from_rel)
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default();

        let mut next_name = name.to_string();
        if Path::new(&next_name).extension().is_none() && !current_ext.is_empty() {
            next_name.push('.');
            next_name.push_str(current_ext);
        }

        let to_rel = normalize_rel_path(&parent.join(next_name).to_string_lossy());
        let to_abs = self.rel_to_abs(&to_rel)?;
        fs::rename(from_abs, to_abs)?;
        Ok(to_rel)
    }

    pub fn delete_path(&self, rel_path: &str, permanent: bool) -> Result<String> {
        let abs = self.rel_to_abs(rel_path)?;
        if !abs.exists() {
            bail!("el archivo no existe");
        }
        if permanent {
            fs::remove_file(abs)?;
            return Ok("deleted".to_string());
        }

        let trash_dir = self.root.join(".trash");
        fs::create_dir_all(&trash_dir)?;
        let file_name = Path::new(rel_path)
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("note.md");
        let stamp = Local::now().format("%Y%m%d%H%M%S");
        let target = trash_dir.join(format!("{stamp}-{file_name}"));
        fs::rename(abs, target)?;
        Ok("trashed".to_string())
    }

    pub fn load_index(&self) -> Result<VaultIndex> {
        let mut previous = read_cache(&self.cache_file).unwrap_or_default();
        let mut next_entries = Vec::new();
        let mut reused = HashMap::<String, CachedFileEntry>::new();
        for entry in previous.files.drain(..) {
            reused.insert(entry.rel_path.clone(), entry);
        }

        for entry in WalkDir::new(&self.root)
            .into_iter()
            .filter_entry(|entry| should_walk(entry.path()))
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }

            let abs = entry.path().to_path_buf();
            let rel_path = rel_from_root(&self.root, &abs)?;
            let meta = entry.metadata()?;
            let modified_ms = to_millis(meta.modified().ok());
            let len = meta.len();

            let cached = reused.remove(&rel_path);
            let current = if let Some(cached) = cached {
                if cached.modified_ms == modified_ms && cached.len == len {
                    cached
                } else {
                    build_cached_entry(&rel_path, len, modified_ms, &abs)?
                }
            } else {
                build_cached_entry(&rel_path, len, modified_ms, &abs)?
            };

            next_entries.push(current);
        }

        next_entries.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
        let stored = StoredIndex {
            version: 1,
            files: next_entries.clone(),
        };
        write_cache(&self.cache_file, &stored)?;

        let mut files = BTreeMap::new();
        for entry in &next_entries {
            files.insert(
                entry.rel_path.clone(),
                FileRecord {
                    rel_path: entry.rel_path.clone(),
                    len: entry.len,
                    modified_ms: entry.modified_ms,
                    is_markdown: entry.markdown.is_some(),
                },
            );
        }

        let mut index = VaultIndex {
            files,
            markdown: BTreeMap::new(),
            resolved_links: BTreeMap::new(),
            unresolved_links: BTreeMap::new(),
            backlinks: BTreeMap::new(),
        };

        for entry in next_entries {
            if let Some(markdown) = entry.markdown {
                index.markdown.insert(entry.rel_path, markdown);
            }
        }

        index.rebuild_graph();
        Ok(index)
    }

    pub fn resolve_target(
        &self,
        index: &VaultIndex,
        file_selector: Option<&str>,
        path_selector: Option<&str>,
        active_file: Option<&str>,
    ) -> Result<String> {
        if let Some(path) = path_selector {
            let normalized = normalize_rel_path(path);
            if !self.rel_to_abs(&normalized)?.exists() {
                bail!("path no encontrado: {normalized}");
            }
            return Ok(normalized);
        }

        if let Some(file) = file_selector {
            return index.resolve_note(file, active_file).with_context(|| {
                format!("no se pudo resolver `file={file}` como wikilink dentro del vault")
            });
        }

        if let Some(active) = active_file {
            return Ok(active.to_string());
        }

        bail!("necesitas `file=<name>`, `path=<path>` o un archivo activo")
    }

    pub fn templates_folder(&self) -> Result<Option<String>> {
        let path = self.obsidian_dir.join("templates.json");
        if !path.exists() {
            return Ok(None);
        }
        let json = fs::read_to_string(path)?;
        let value: Value = serde_json::from_str(&json)?;
        Ok(value
            .get("folder")
            .and_then(Value::as_str)
            .map(normalize_rel_path))
    }

    pub fn daily_settings(&self) -> Result<DailySettings> {
        let path = self.obsidian_dir.join("daily-notes.json");
        if !path.exists() {
            return Ok(DailySettings::default());
        }
        let json = fs::read_to_string(path)?;
        serde_json::from_str(&json).map_err(Into::into)
    }

    pub fn ensure_daily_note_path(&self) -> Result<String> {
        let settings = self.daily_settings()?;
        let format = settings.format.unwrap_or_else(|| "YYYY-MM-DD".to_string());
        let folder = settings.folder.unwrap_or_default();
        let chrono_format = moment_to_chrono(&format);
        let name = Local::now().format(&chrono_format).to_string();
        let rel = if folder.is_empty() {
            format!("{name}.md")
        } else {
            format!("{folder}/{name}.md")
        };
        Ok(normalize_rel_path(&rel))
    }

    pub fn list_bases(&self) -> Result<Vec<String>> {
        let mut bases = self
            .list_files(None, Some("base"))?
            .into_iter()
            .map(|file| file.rel_path)
            .collect::<Vec<_>>();
        bases.sort();
        Ok(bases)
    }
}

impl VaultIndex {
    pub fn resolve_note(&self, selector: &str, active_file: Option<&str>) -> Result<String> {
        let selector = selector.trim();
        let normalized = normalize_rel_path(selector)
            .trim_end_matches(".md")
            .to_string();
        let stem = Path::new(&normalized)
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or(&normalized)
            .to_ascii_lowercase();

        if self.files.contains_key(&normalized) {
            return Ok(normalized);
        }

        let exact_markdown = if normalized.ends_with(".md") {
            normalized.clone()
        } else {
            format!("{normalized}.md")
        };
        if self.files.contains_key(&exact_markdown) {
            return Ok(exact_markdown);
        }

        let source_dir = active_file
            .and_then(|path| Path::new(path).parent())
            .map(|path| normalize_rel_path(&path.to_string_lossy()))
            .unwrap_or_default();

        let mut ranked = self
            .markdown
            .keys()
            .filter_map(|candidate| {
                let candidate_no_ext = candidate.trim_end_matches(".md");
                let candidate_name = Path::new(candidate_no_ext)
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or(candidate_no_ext)
                    .to_ascii_lowercase();
                if candidate.eq_ignore_ascii_case(&normalized)
                    || candidate_no_ext.eq_ignore_ascii_case(&normalized)
                    || candidate_name == stem
                    || candidate
                        .to_ascii_lowercase()
                        .ends_with(&format!("/{normalized}"))
                    || candidate_no_ext
                        .to_ascii_lowercase()
                        .ends_with(&format!("/{normalized}"))
                {
                    Some((score_candidate(&source_dir, candidate), candidate.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        ranked.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
        ranked
            .into_iter()
            .next()
            .map(|(_, path)| path)
            .ok_or_else(|| anyhow!("sin match"))
    }

    pub fn task_by_ref(&self, reference: &str) -> Result<(String, TaskItem)> {
        let (path, line) = reference
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("`ref` debe ser `<path>:<line>`"))?;
        let line = line.parse::<usize>()?;
        let meta = self
            .markdown
            .get(path)
            .ok_or_else(|| anyhow!("archivo no encontrado en el índice"))?;
        let task = meta
            .tasks
            .iter()
            .find(|task| task.line == line)
            .cloned()
            .ok_or_else(|| anyhow!("no hay tarea en {reference}"))?;
        Ok((path.to_string(), task))
    }

    fn rebuild_graph(&mut self) {
        self.resolved_links.clear();
        self.unresolved_links.clear();
        self.backlinks.clear();

        let note_paths = self.markdown.keys().cloned().collect::<Vec<_>>();
        for (path, meta) in &self.markdown {
            for link in &meta.links {
                if let Some(target) = resolve_link_target(&note_paths, link, Some(path)) {
                    *self
                        .resolved_links
                        .entry(path.clone())
                        .or_default()
                        .entry(target.clone())
                        .or_default() += 1;
                    *self
                        .backlinks
                        .entry(target)
                        .or_default()
                        .entry(path.clone())
                        .or_default() += 1;
                } else {
                    *self
                        .unresolved_links
                        .entry(path.clone())
                        .or_default()
                        .entry(link.clone())
                        .or_default() += 1;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailySettings {
    pub folder: Option<String>,
    pub format: Option<String>,
    pub template: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub rel_path: String,
    pub len: u64,
    pub modified_ms: u64,
    pub is_markdown: bool,
}

#[derive(Debug, Clone, Default)]
pub struct VaultIndex {
    pub files: BTreeMap<String, FileRecord>,
    pub markdown: BTreeMap<String, MarkdownMeta>,
    pub resolved_links: BTreeMap<String, BTreeMap<String, usize>>,
    pub unresolved_links: BTreeMap<String, BTreeMap<String, usize>>,
    pub backlinks: BTreeMap<String, BTreeMap<String, usize>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarkdownMeta {
    pub headings: Vec<Heading>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub properties: BTreeMap<String, Value>,
    pub tasks: Vec<TaskItem>,
    pub links: Vec<String>,
    pub word_count: usize,
    pub character_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Heading {
    pub level: usize,
    pub text: String,
    pub line: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskItem {
    pub line: usize,
    pub status: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredIndex {
    version: u32,
    files: Vec<CachedFileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFileEntry {
    rel_path: String,
    len: u64,
    modified_ms: u64,
    markdown: Option<MarkdownMeta>,
}

fn build_cached_entry(
    rel_path: &str,
    len: u64,
    modified_ms: u64,
    abs: &Path,
) -> Result<CachedFileEntry> {
    let markdown = if is_markdown_path(abs) {
        let text = fs::read_to_string(abs).unwrap_or_default();
        Some(parse_markdown(&text))
    } else {
        None
    };

    Ok(CachedFileEntry {
        rel_path: rel_path.to_string(),
        len,
        modified_ms,
        markdown,
    })
}

pub fn parse_markdown(text: &str) -> MarkdownMeta {
    let (frontmatter, body) = split_frontmatter(text);
    let mut headings = Vec::new();
    let mut tasks = Vec::new();
    let mut tags = BTreeSet::new();
    let mut links = Vec::new();

    let mut in_code_fence = false;
    for (index, line) in body.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
        }
        if in_code_fence {
            continue;
        }

        if let Some(captures) = HEADING_RE.captures(line) {
            headings.push(Heading {
                level: captures
                    .get(1)
                    .map(|token| token.as_str().len())
                    .unwrap_or(1),
                text: captures
                    .name("text")
                    .map(|token| token.as_str().trim().to_string())
                    .unwrap_or_default(),
                line: line_number,
            });
        }

        if let Some(captures) = TASK_RE.captures(line) {
            tasks.push(TaskItem {
                line: line_number,
                status: captures
                    .name("status")
                    .map(|token| token.as_str().to_string())
                    .unwrap_or_default(),
                text: captures
                    .name("text")
                    .map(|token| token.as_str().trim().to_string())
                    .unwrap_or_default(),
            });
        }

        for captures in WIKILINK_RE.captures_iter(line) {
            if let Some(target) = captures.name("link") {
                links.push(target.as_str().trim().to_string());
            }
        }

        for captures in MD_LINK_RE.captures_iter(line) {
            if let Some(target) = captures.name("link") {
                let value = target.as_str();
                if !value.contains("://") && !value.starts_with('#') {
                    links.push(value.trim().trim_end_matches(".md").to_string());
                }
            }
        }

        for captures in TAG_RE.captures_iter(line) {
            if let Some(tag) = captures.name("tag") {
                tags.insert(tag.as_str().to_string());
            }
        }
    }

    let mut properties = BTreeMap::new();
    let mut aliases = Vec::new();
    if let Some(frontmatter) = frontmatter.as_deref() {
        let yaml_frontmatter = frontmatter
            .trim_start_matches("---\n")
            .trim_end_matches("\n---\n")
            .trim_end_matches("\n...\n");
        if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(yaml_frontmatter)
            && let Some(mapping) = value.as_mapping()
        {
            for (key, value) in mapping {
                if let Some(key) = key.as_str() {
                    let json_value = serde_json::to_value(value).unwrap_or(Value::Null);
                    if key == "aliases" || key == "alias" {
                        aliases.extend(value_to_strings(&json_value));
                    }
                    if key == "tags" {
                        for tag in value_to_strings(&json_value) {
                            let next = if tag.starts_with('#') {
                                tag
                            } else {
                                format!("#{tag}")
                            };
                            tags.insert(next);
                        }
                    }
                    properties.insert(key.to_string(), json_value);
                }
            }
        }
    }

    MarkdownMeta {
        headings,
        tags: tags.into_iter().collect(),
        aliases,
        properties,
        tasks,
        links,
        word_count: body.split_whitespace().count(),
        character_count: body.chars().count(),
    }
}

pub fn split_frontmatter(text: &str) -> (Option<String>, &str) {
    if !text.starts_with("---\n") {
        return (None, text);
    }
    let rest = &text[4..];
    if let Some(end) = rest.find("\n---\n") {
        let frontmatter = &text[..(4 + end + 5)];
        let body = &text[(4 + end + 5)..];
        return (Some(frontmatter.to_string()), body);
    }
    if let Some(end) = rest.find("\n...\n") {
        let frontmatter = &text[..(4 + end + 5)];
        let body = &text[(4 + end + 5)..];
        return (Some(frontmatter.to_string()), body);
    }
    (None, text)
}

pub fn replace_task_status(text: &str, line: usize, status: &str) -> Result<String> {
    let mut lines = text.lines().map(ToString::to_string).collect::<Vec<_>>();
    let idx = line.saturating_sub(1);
    let target = lines
        .get_mut(idx)
        .ok_or_else(|| anyhow!("línea fuera de rango"))?;
    let replaced = TASK_RE
        .replace(target, |captures: &Captures| {
            let whole = captures
                .get(0)
                .map(|token| token.as_str())
                .unwrap_or_default();
            let slot = captures.name("status").expect("status capture");
            let whole_match = captures.get(0).expect("whole capture");
            let start = slot.start() - whole_match.start();
            let end = slot.end() - whole_match.start();
            let mut next = whole.to_string();
            next.replace_range(start..end, status);
            next
        })
        .to_string();
    *target = replaced;
    let mut next = lines.join("\n");
    if text.ends_with('\n') {
        next.push('\n');
    }
    Ok(next)
}

pub fn read_frontmatter(path: &Path) -> Result<BTreeMap<String, Value>> {
    let text = fs::read_to_string(path)?;
    let (frontmatter, _) = split_frontmatter(&text);
    let Some(frontmatter) = frontmatter else {
        return Ok(BTreeMap::new());
    };
    let yaml = frontmatter
        .trim_start_matches("---\n")
        .trim_end_matches("\n---\n")
        .trim_end_matches("\n...\n");
    let yaml: serde_yaml::Value = serde_yaml::from_str(yaml)?;
    let mut result = BTreeMap::new();
    if let Some(mapping) = yaml.as_mapping() {
        for (key, value) in mapping {
            if let Some(key) = key.as_str() {
                result.insert(
                    key.to_string(),
                    serde_json::to_value(value).unwrap_or(Value::Null),
                );
            }
        }
    }
    Ok(result)
}

pub fn write_frontmatter(
    path: &Path,
    properties: &BTreeMap<String, Value>,
    body: &str,
) -> Result<()> {
    let mut yaml = serde_yaml::Mapping::new();
    for (key, value) in properties {
        yaml.insert(
            serde_yaml::Value::String(key.clone()),
            serde_yaml::to_value(value)?,
        );
    }
    let frontmatter = serde_yaml::to_string(&yaml)?.trim_end().to_string();
    let mut next = String::new();
    if !properties.is_empty() {
        next.push_str("---\n");
        next.push_str(&frontmatter);
        next.push_str("\n---\n");
    }
    next.push_str(body);
    fs::write(path, next)?;
    Ok(())
}

pub fn rel_from_root(root: &Path, abs: &Path) -> Result<String> {
    let rel = abs
        .strip_prefix(root)
        .with_context(|| format!("path fuera del vault: {}", abs.display()))?;
    Ok(normalize_rel_path(&rel.to_string_lossy()))
}

pub fn normalize_rel_path(path: &str) -> String {
    let mut parts = Vec::<String>::new();
    let normalized = path.replace('\\', "/");
    for component in Path::new(&normalized).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = parts.pop();
            }
            Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    parts.join("/")
}

fn score_candidate(source_dir: &str, candidate: &str) -> i32 {
    let mut score = 0;
    if candidate.starts_with(source_dir) {
        score += 20;
    }
    if !candidate.contains('/') {
        score += 10;
    }
    score
}

fn resolve_link_target(
    note_paths: &[String],
    link: &str,
    active_file: Option<&str>,
) -> Option<String> {
    let normalized = normalize_rel_path(link).trim_end_matches(".md").to_string();
    let exact = if normalized.ends_with(".md") {
        normalized.clone()
    } else {
        format!("{normalized}.md")
    };
    if note_paths
        .iter()
        .any(|path| path.eq_ignore_ascii_case(&exact))
    {
        return note_paths
            .iter()
            .find(|path| path.eq_ignore_ascii_case(&exact))
            .cloned();
    }

    let stem = Path::new(&normalized)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(&normalized)
        .to_ascii_lowercase();
    let source_dir = active_file
        .and_then(|path| Path::new(path).parent())
        .map(|path| normalize_rel_path(&path.to_string_lossy()))
        .unwrap_or_default();
    let mut candidates = note_paths
        .iter()
        .filter_map(|path| {
            let base = Path::new(path)
                .file_stem()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if base == stem
                || path
                    .trim_end_matches(".md")
                    .to_ascii_lowercase()
                    .ends_with(&normalized.to_ascii_lowercase())
            {
                Some((score_candidate(&source_dir, path), path.clone()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

fn value_to_strings(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) => values.iter().flat_map(value_to_strings).collect(),
        _ => Vec::new(),
    }
}

fn runtime_paths() -> Result<RuntimePaths> {
    let config_dir = dirs::config_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .ok_or_else(|| anyhow!("no se pudo determinar el directorio de configuración"))?;
    let base_dir = config_dir.join("obsidian-termux-cli");
    let cache_dir = base_dir.join("cache");
    let history_file = base_dir.join("history.txt");
    let state_file = base_dir.join("state.json");
    Ok(RuntimePaths {
        base_dir,
        cache_dir,
        state_file,
        history_file,
    })
}

fn discover_known_vaults() -> Vec<KnownVault> {
    let mut candidates = Vec::new();
    if let Some(config_dir) = dirs::config_dir() {
        candidates.push(config_dir.join("obsidian").join("obsidian.json"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".config").join("obsidian").join("obsidian.json"));
    }
    if let Ok(custom) = std::env::var("OBSIDIAN_CONFIG_DIR") {
        candidates.push(PathBuf::from(custom).join("obsidian.json"));
    }

    let mut known = Vec::new();
    for path in candidates {
        if !path.exists() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(vaults) = value.get("vaults").and_then(Value::as_object) else {
            continue;
        };
        for entry in vaults.values() {
            let Some(vault_path) = entry.get("path").and_then(Value::as_str) else {
                continue;
            };
            let path_buf = PathBuf::from(vault_path);
            if !path_buf.join(".obsidian").exists() {
                continue;
            }
            known.push(KnownVault {
                name: vault_name(&path_buf),
                path: path_buf.to_string_lossy().to_string(),
            });
        }
    }
    known
}

fn read_cache(path: &Path) -> Result<StoredIndex> {
    let text = fs::read_to_string(path)?;
    let stored = serde_json::from_str::<StoredIndex>(&text)?;
    if stored.version != 1 {
        bail!("cache version incompatible");
    }
    Ok(stored)
}

fn write_cache(path: &Path, cache: &StoredIndex) -> Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(serde_json::to_string(cache)?.as_bytes())?;
    Ok(())
}

fn vault_hash(root: &Path) -> String {
    let mut sha1 = Sha1::new();
    sha1.update(root.to_string_lossy().as_bytes());
    format!("{:x}", sha1.finalize())
}

fn vault_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("vault")
        .to_string()
}

fn should_walk(path: &Path) -> bool {
    let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if file_name == ".obsidian" || file_name == ".git" || file_name == "node_modules" {
        return false;
    }
    true
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .map(|ext| ext.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn to_millis(time: Option<SystemTime>) -> u64 {
    time.and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn find_vault_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(next) = current {
        if next.join(".obsidian").exists() {
            return Some(next.to_path_buf());
        }
        current = next.parent();
    }
    None
}

pub fn moment_to_chrono(value: &str) -> String {
    let replacements = [
        ("YYYY", "%Y"),
        ("YY", "%y"),
        ("MMMM", "%B"),
        ("MMM", "%b"),
        ("MM", "%m"),
        ("DD", "%d"),
        ("dddd", "%A"),
        ("ddd", "%a"),
        ("HH", "%H"),
        ("hh", "%I"),
        ("mm", "%M"),
        ("ss", "%S"),
    ];
    let mut current = value.to_string();
    for (from, to) in replacements {
        current = current.replace(from, to);
    }
    current
}

pub fn apply_template_tokens(template: &str, title: Option<&str>) -> String {
    let now = Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H:%M").to_string();
    let datetime = now.to_rfc3339();

    let replacements = [
        ("{{title}}", title.unwrap_or("")),
        ("{{date}}", date.as_str()),
        ("{{time}}", time.as_str()),
        ("{{datetime}}", datetime.as_str()),
    ];

    let mut next = template.to_string();
    for (from, to) in replacements {
        next = next.replace(from, to);
    }
    next
}

pub fn json_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

pub fn property_rows(meta: &MarkdownMeta) -> Vec<Value> {
    meta.properties
        .iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect()
}

pub fn alias_rows(meta: &MarkdownMeta) -> Vec<Value> {
    meta.aliases.iter().map(|alias| json!(alias)).collect()
}

pub fn count_bytes(records: &[FileRecord]) -> u64 {
    records.iter().map(|record| record.len).sum()
}

#[cfg(test)]
mod tests {
    use super::{moment_to_chrono, normalize_rel_path, parse_markdown};

    #[test]
    fn parses_markdown_metadata() {
        let meta = parse_markdown(
            "---\naliases:\n  - Hola\ntags:\n  - termux\n---\n# Title\n- [ ] Task\n[[Other Note]]\n#tag\n",
        );
        assert_eq!(meta.headings.len(), 1);
        assert_eq!(meta.tasks.len(), 1);
        assert!(meta.links.contains(&"Other Note".to_string()));
        assert!(meta.tags.contains(&"#tag".to_string()));
        assert!(meta.aliases.contains(&"Hola".to_string()));
        assert!(meta.tags.contains(&"#termux".to_string()));
    }

    #[test]
    fn normalizes_relative_paths() {
        assert_eq!(normalize_rel_path("./Inbox\\Note.md"), "Inbox/Note.md");
        assert_eq!(normalize_rel_path("A/../B.md"), "B.md");
    }

    #[test]
    fn converts_moment_tokens() {
        assert_eq!(moment_to_chrono("YYYY-MM-DD"), "%Y-%m-%d");
    }
}
