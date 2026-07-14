use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
const INDEX_CACHE_VERSION: u32 = 2;
const KNOWN_VAULTS_CACHE_VERSION: u32 = 3;

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
    pub language: Option<String>,
}

#[derive(Debug)]
pub struct Workspace {
    pub cwd: PathBuf,
    pub runtime: RuntimePaths,
    pub known_vaults: Vec<KnownVault>,
    pub state: SessionState,
    dirty: bool,
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

        let known_vaults = normalize_known_vaults(discover_known_vaults(&runtime));

        Ok(Self {
            cwd,
            runtime,
            known_vaults,
            state,
            dirty: false,
        })
    }

    pub fn save_if_dirty(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let body = serde_json::to_string_pretty(&self.state)?;
        atomic_write_bytes(&self.runtime.state_file, body.as_bytes())?;
        self.dirty = false;
        Ok(())
    }

    pub fn refresh_known_vaults(&mut self) -> Result<()> {
        let cache_file = self.runtime.cache_dir.join("known-vaults.json");
        if cache_file.exists() {
            fs::remove_file(cache_file)?;
        }
        fs::create_dir_all(&self.runtime.cache_dir)?;
        let known_vaults = normalize_known_vaults(discover_known_vaults_uncached());

        let cache = KnownVaultsCache {
            version: KNOWN_VAULTS_CACHE_VERSION,
            scanned_at_ms: to_millis(Some(SystemTime::now())),
            vaults: known_vaults.clone(),
        };
        atomic_write_bytes(
            &self.runtime.cache_dir.join("known-vaults.json"),
            &serde_json::to_vec(&cache)?,
        )?;
        self.known_vaults = known_vaults;
        Ok(())
    }

    pub fn open_vault(&self, selector: Option<&str>) -> Result<VaultContext> {
        let vault = self.resolve_vault(selector)?;
        Ok(VaultContext::new(self.runtime.clone(), vault))
    }

    pub fn open_or_init_vault(&self, selector: Option<&str>) -> Result<VaultContext> {
        match self.open_vault(selector) {
            Ok(vault) => Ok(vault),
            Err(error) => {
                let Some(raw_selector) = selector else {
                    return Err(error);
                };
                if !is_path_like_selector(raw_selector) {
                    return Err(error);
                }
                let path = PathBuf::from(raw_selector);
                fs::create_dir_all(path.join(".obsidian"))?;
                let vault = KnownVault {
                    name: vault_name(&path),
                    path: path.to_string_lossy().to_string(),
                };
                Ok(VaultContext::new(self.runtime.clone(), vault))
            }
        }
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
        self.dirty = true;
    }

    pub fn language(&self) -> &str {
        self.state.language.as_deref().unwrap_or("es")
    }

    pub fn set_language(&mut self, language: &str) {
        self.state.language = Some(language.to_string());
        self.dirty = true;
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
            self.dirty = true;
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
        self.dirty = true;
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
        let rel_path = validate_rel_path(rel_path)?;
        let abs = self.root.join(&rel_path);
        ensure_path_within_root(&self.root, &abs)?;
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

        for entry in WalkDir::new(&root)
            .into_iter()
            .filter_entry(|entry| should_walk_entry(&root, entry.path()))
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
        for entry in WalkDir::new(&root)
            .min_depth(1)
            .into_iter()
            .filter_entry(|entry| should_walk_entry(&root, entry.path()))
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
        atomic_write_bytes(&abs, content.as_bytes())?;
        Ok(())
    }

    pub fn append_text(&self, rel_path: &str, content: &str, inline: bool) -> Result<()> {
        let mut current = self.read_text(rel_path)?;
        if !inline && !current.is_empty() && !current.ends_with('\n') {
            current.push('\n');
        }
        current.push_str(content);
        atomic_write_bytes(&self.rel_to_abs(rel_path)?, current.as_bytes())?;
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
        atomic_write_bytes(&self.rel_to_abs(rel_path)?, next.as_bytes())?;
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
        if to_abs.exists() && to_abs != from_abs {
            bail!("el destino ya existe: {to_rel}");
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
        if to_abs.exists() && to_abs != from_abs {
            bail!("el destino ya existe: {to_rel}");
        }
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
        let stamp = Local::now().format("%Y%m%d%H%M%S%f");
        let target = trash_dir.join(format!("{stamp}-{file_name}"));
        fs::rename(abs, target)?;
        Ok("trashed".to_string())
    }

    pub fn load_index(&self) -> Result<VaultIndex> {
        let cached = read_cache(&self.cache_file).ok();
        let mut cache_dirty = cached.is_none();
        let mut previous = cached.unwrap_or_default();
        let mut next_entries = Vec::new();
        let mut reused = HashMap::<String, CachedFileEntry>::new();
        for entry in previous.files.drain(..) {
            reused.insert(entry.rel_path.clone(), entry);
        }

        for entry in WalkDir::new(&self.root)
            .into_iter()
            .filter_entry(|entry| should_walk_entry(&self.root, entry.path()))
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
                    cache_dirty = true;
                    build_cached_entry(&rel_path, len, modified_ms, &abs)?
                }
            } else {
                cache_dirty = true;
                build_cached_entry(&rel_path, len, modified_ms, &abs)?
            };

            next_entries.push(current);
        }

        if !reused.is_empty() {
            cache_dirty = true;
        }
        next_entries.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
        if cache_dirty {
            let stored = StoredIndex {
                version: INDEX_CACHE_VERSION,
                files: next_entries.clone(),
            };
            write_cache(&self.cache_file, &stored)?;
        }

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
        if ranked.len() > 1 && ranked[0].0 == ranked[1].0 {
            let candidates = ranked
                .iter()
                .take_while(|candidate| candidate.0 == ranked[0].0)
                .map(|candidate| candidate.1.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("selector ambiguo `{selector}`; usa path=<ruta>. candidatos: {candidates}");
        }
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

        let resolver = LinkResolver::new(self.markdown.keys().cloned());
        for (path, meta) in &self.markdown {
            for link in &meta.links {
                if let Some(target) = resolver.resolve(link, Some(path)) {
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
    #[serde(default)]
    pub search_blob: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter_error: Option<String>,
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
        let text = fs::read_to_string(abs)
            .with_context(|| format!("no se pudo leer markdown cacheado: {}", abs.display()))?;
        Some(parse_markdown_with_source(&text, rel_path))
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

#[cfg(test)]
pub fn parse_markdown(text: &str) -> MarkdownMeta {
    parse_markdown_inner(text, None)
}

fn parse_markdown_with_source(text: &str, source: &str) -> MarkdownMeta {
    parse_markdown_inner(text, Some(source))
}

fn parse_markdown_inner(text: &str, source: Option<&str>) -> MarkdownMeta {
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
    let mut frontmatter_error = None;
    if let Some(frontmatter) = frontmatter.as_deref() {
        let yaml_frontmatter = frontmatter_yaml(frontmatter);
        match serde_yaml::from_str::<serde_yaml::Value>(yaml_frontmatter) {
            Ok(value) => {
                if let Some(mapping) = value.as_mapping() {
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
            Err(error) => {
                let target = source.unwrap_or("markdown");
                let message = error.to_string();
                eprintln!("[warn] frontmatter inválido en {target}: {message}");
                frontmatter_error = Some(message);
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
        search_blob: body.to_ascii_lowercase(),
        frontmatter_error,
    }
}

pub fn split_frontmatter(text: &str) -> (Option<String>, &str) {
    let (opening_len, newline) = if text.starts_with("---\r\n") {
        (5, "\r\n")
    } else if text.starts_with("---\n") {
        (4, "\n")
    } else {
        return (None, text);
    };
    let rest = &text[opening_len..];
    for marker in ["---", "..."] {
        let closing = format!("{newline}{marker}{newline}");
        if let Some(end) = rest.find(&closing) {
            let boundary = opening_len + end + closing.len();
            return (Some(text[..boundary].to_string()), &text[boundary..]);
        }
    }
    (None, text)
}

fn frontmatter_yaml(frontmatter: &str) -> &str {
    let yaml = frontmatter
        .strip_prefix("---\r\n")
        .or_else(|| frontmatter.strip_prefix("---\n"))
        .unwrap_or(frontmatter);
    yaml.strip_suffix("\r\n---\r\n")
        .or_else(|| yaml.strip_suffix("\r\n...\r\n"))
        .or_else(|| yaml.strip_suffix("\n---\n"))
        .or_else(|| yaml.strip_suffix("\n...\n"))
        .unwrap_or(yaml)
}

pub fn replace_task_status(text: &str, line: usize, status: &str) -> Result<String> {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
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
    let mut next = lines.join(newline);
    if text.ends_with('\n') {
        next.push_str(newline);
    }
    Ok(next)
}

pub fn read_frontmatter(path: &Path) -> Result<BTreeMap<String, Value>> {
    let text = fs::read_to_string(path)?;
    let (frontmatter, _) = split_frontmatter(&text);
    let Some(frontmatter) = frontmatter else {
        return Ok(BTreeMap::new());
    };
    let yaml = frontmatter_yaml(&frontmatter);
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
    let newline = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let mut next = String::new();
    if !properties.is_empty() {
        next.push_str("---");
        next.push_str(newline);
        next.push_str(&frontmatter.replace('\n', newline));
        next.push_str(newline);
        next.push_str("---");
        next.push_str(newline);
    }
    next.push_str(body);
    atomic_write_bytes(path, next.as_bytes())?;
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

fn validate_rel_path(path: &str) -> Result<String> {
    let normalized = path.replace('\\', "/");
    let mut parts = Vec::<String>::new();

    for component in Path::new(&normalized).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    bail!("path fuera del vault: {path}");
                }
            }
            Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
            Component::RootDir | Component::Prefix(_) => {
                bail!("se requiere un path relativo al vault: {path}");
            }
        }
    }

    Ok(parts.join("/"))
}

fn ensure_path_within_root(root: &Path, target: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("no se pudo resolver el vault {}", root.display()))?;
    let mut existing = target;
    while fs::symlink_metadata(existing).is_err() {
        existing = existing
            .parent()
            .ok_or_else(|| anyhow!("path fuera del vault: {}", target.display()))?;
    }
    let canonical_existing = existing
        .canonicalize()
        .with_context(|| format!("no se pudo resolver {}", existing.display()))?;

    if !canonical_existing.starts_with(&canonical_root) {
        bail!(
            "path fuera del vault mediante enlace simbólico: {}",
            target.display()
        );
    }
    Ok(())
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

struct LinkResolver {
    exact: HashMap<String, String>,
    by_stem: HashMap<String, Vec<String>>,
}

impl LinkResolver {
    fn new(paths: impl IntoIterator<Item = String>) -> Self {
        let mut exact = HashMap::new();
        let mut by_stem = HashMap::<String, Vec<String>>::new();
        for path in paths {
            exact.insert(path.to_ascii_lowercase(), path.clone());
            let stem = Path::new(&path)
                .file_stem()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            by_stem.entry(stem).or_default().push(path);
        }
        Self { exact, by_stem }
    }

    fn resolve(&self, link: &str, active_file: Option<&str>) -> Option<String> {
        let normalized = normalize_rel_path(link).trim_end_matches(".md").to_string();
        let exact = format!("{normalized}.md").to_ascii_lowercase();
        if let Some(path) = self.exact.get(&exact) {
            return Some(path.clone());
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
        let mut candidates = self
            .by_stem
            .get(&stem)?
            .iter()
            .map(|path| (score_candidate(&source_dir, path), path.clone()))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
        candidates.into_iter().next().map(|(_, path)| path)
    }
}

fn value_to_strings(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) => values.iter().flat_map(value_to_strings).collect(),
        _ => Vec::new(),
    }
}

fn runtime_paths() -> Result<RuntimePaths> {
    let base_dir = std::env::var_os("OBSIDIAN_CLI_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| {
            dirs::config_dir()
                .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
                .map(|path| path.join("obsidian-termux-cli"))
                .ok_or_else(|| anyhow!("no se pudo determinar el directorio de configuración"))
        })?;
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct KnownVaultsCache {
    version: u32,
    scanned_at_ms: u64,
    vaults: Vec<KnownVault>,
}

fn discover_known_vaults(runtime: &RuntimePaths) -> Vec<KnownVault> {
    let cache_file = runtime.cache_dir.join("known-vaults.json");
    if let Some(cached) = load_known_vaults_cache(&cache_file) {
        return cached;
    }

    let known = discover_known_vaults_uncached();
    let cache = KnownVaultsCache {
        version: KNOWN_VAULTS_CACHE_VERSION,
        scanned_at_ms: to_millis(Some(SystemTime::now())),
        vaults: known.clone(),
    };
    if let Ok(text) = serde_json::to_string(&cache) {
        let _ = fs::write(cache_file, text);
    }
    known
}

fn load_known_vaults_cache(path: &Path) -> Option<Vec<KnownVault>> {
    if !path.exists() {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let cache = serde_json::from_str::<KnownVaultsCache>(&text).ok()?;
    if cache.version != KNOWN_VAULTS_CACHE_VERSION {
        return None;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let scanned = Duration::from_millis(cache.scanned_at_ms);
    if now.saturating_sub(scanned) > Duration::from_secs(60 * 60 * 6) {
        return None;
    }
    Some(normalize_known_vaults(cache.vaults))
}

fn normalize_known_vaults(vaults: Vec<KnownVault>) -> Vec<KnownVault> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();

    for vault in vaults {
        let path = canonical_or_self(&vault.path_buf());
        let key = path.to_string_lossy().to_string();
        if !seen.insert(key.clone()) {
            continue;
        }

        normalized.push(KnownVault {
            name: vault.name,
            path: key,
        });
    }

    normalized.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.path.cmp(&right.path))
    });
    normalized
}

fn discover_known_vaults_uncached() -> Vec<KnownVault> {
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
            push_known_vault(&mut known, &path_buf);
        }
    }

    for path in discover_documents_vaults() {
        push_known_vault(&mut known, &path);
    }

    known
}

fn push_known_vault(known: &mut Vec<KnownVault>, path: &Path) {
    let path = canonical_or_self(path);
    let path_str = path.to_string_lossy().to_string();
    if known.iter().any(|vault| vault.path == path_str) {
        return;
    }
    known.push(KnownVault {
        name: vault_name(&path),
        path: path_str,
    });
}

fn canonical_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn discover_documents_vaults() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(documents) = dirs::document_dir() {
        roots.push(documents);
    }

    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Documents"));
        roots.push(home.join("storage").join("shared").join("Documents"));
    }

    roots.push(PathBuf::from("/storage/emulated/0/Documents"));
    roots.push(PathBuf::from("/sdcard/Documents"));

    if let Ok(custom) = std::env::var("OBSIDIAN_VAULTS_DIR") {
        roots.push(PathBuf::from(custom));
    }

    discover_vaults_under_roots(&roots)
}

fn discover_vaults_under_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut vaults = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        if !root.exists() {
            continue;
        }

        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(3)
            .into_iter()
            .filter_entry(|entry| should_walk_entry(root, entry.path()))
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_dir() {
                continue;
            }

            let candidate = entry.path();
            if !candidate.join(".obsidian").is_dir() {
                continue;
            }

            let key = canonical_or_self(candidate).to_string_lossy().to_string();
            if seen.insert(key) {
                vaults.push(candidate.to_path_buf());
            }
        }
    }

    vaults
}

fn read_cache(path: &Path) -> Result<StoredIndex> {
    let text = fs::read_to_string(path)?;
    let stored = serde_json::from_str::<StoredIndex>(&text)?;
    if stored.version != INDEX_CACHE_VERSION {
        bail!("cache version incompatible");
    }
    Ok(stored)
}

fn write_cache(path: &Path, cache: &StoredIndex) -> Result<()> {
    atomic_write_bytes(path, serde_json::to_string(cache)?.as_bytes())?;
    Ok(())
}

pub(crate) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("ruta inválida para escritura atómica: {}", path.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|error| anyhow!(error.error))
        .with_context(|| format!("no se pudo reemplazar {}", path.display()))?;
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

fn should_walk_entry(root: &Path, path: &Path) -> bool {
    if path == root {
        return true;
    }
    should_walk(path)
}

fn should_walk(path: &Path) -> bool {
    let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if file_name == "node_modules" || file_name.starts_with('.') {
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

fn is_path_like_selector(value: &str) -> bool {
    value.contains('/')
        || value.contains('\\')
        || value.starts_with('.')
        || value.starts_with('~')
        || Path::new(value).is_absolute()
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
    meta.aliases
        .iter()
        .map(|alias| json!({ "value": alias }))
        .collect()
}

pub fn count_bytes(records: &[FileRecord]) -> u64 {
    records.iter().map(|record| record.len).sum()
}

#[cfg(test)]
mod tests {
    use super::{
        KnownVault, LinkResolver, RuntimePaths, SessionState, VaultContext, Workspace,
        apply_template_tokens, discover_vaults_under_roots, moment_to_chrono, normalize_rel_path,
        parse_markdown,
    };

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
    fn invalid_frontmatter_is_recorded() {
        let meta = parse_markdown("---\ntags: [broken\n---\n# Title\n");

        assert!(meta.frontmatter_error.is_some());
        assert!(meta.properties.is_empty());
    }

    #[test]
    fn parses_and_rewrites_crlf_frontmatter_without_duplication() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Note.md");
        let original = "---\r\ntitle: Original\r\n---\r\nBody\r\n";
        std::fs::write(&path, original).unwrap();

        let mut properties = super::read_frontmatter(&path).unwrap();
        assert_eq!(properties["title"], serde_json::json!("Original"));
        properties.insert("added".to_string(), serde_json::json!(true));
        let (_, body) = super::split_frontmatter(original);
        super::write_frontmatter(&path, &properties, body).unwrap();

        let updated = std::fs::read_to_string(path).unwrap();
        assert!(updated.starts_with("---\r\n"));
        assert!(updated.contains("title: Original\r\n"));
        assert!(updated.contains("added: true\r\n"));
        assert_eq!(updated.matches("---").count(), 2);
        assert!(updated.ends_with("Body\r\n"));
    }

    #[test]
    fn normalizes_relative_paths() {
        assert_eq!(normalize_rel_path("./Inbox\\Note.md"), "Inbox/Note.md");
        assert_eq!(normalize_rel_path("A/../B.md"), "B.md");
    }

    #[test]
    fn vault_paths_reject_absolute_and_parent_escape() {
        let root = tempfile::tempdir().unwrap();
        let vault_root = root.path().join("Vault");
        std::fs::create_dir_all(vault_root.join(".obsidian")).unwrap();
        let runtime = RuntimePaths {
            base_dir: root.path().join("runtime"),
            cache_dir: root.path().join("runtime/cache"),
            state_file: root.path().join("runtime/state.json"),
            history_file: root.path().join("runtime/history.txt"),
        };
        let vault = VaultContext::new(
            runtime,
            KnownVault {
                name: "Vault".to_string(),
                path: vault_root.to_string_lossy().to_string(),
            },
        );

        assert!(vault.rel_to_abs("../outside.md").is_err());
        assert!(
            vault
                .rel_to_abs(&root.path().join("absolute.md").to_string_lossy())
                .is_err()
        );
        assert_eq!(
            vault.rel_to_abs("Notes/../Inbox.md").unwrap(),
            vault_root.join("Inbox.md")
        );
    }

    #[cfg(unix)]
    #[test]
    fn vault_paths_reject_symlinks_that_escape_the_vault() {
        let root = tempfile::tempdir().unwrap();
        let vault_root = root.path().join("Vault");
        let outside = root.path().join("Outside");
        std::fs::create_dir_all(vault_root.join(".obsidian")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.md"), "secret").unwrap();
        std::os::unix::fs::symlink(&outside, vault_root.join("escape")).unwrap();
        let vault = VaultContext::new(
            RuntimePaths {
                base_dir: root.path().join("runtime"),
                cache_dir: root.path().join("runtime/cache"),
                state_file: root.path().join("runtime/state.json"),
                history_file: root.path().join("runtime/history.txt"),
            },
            KnownVault {
                name: "Vault".to_string(),
                path: vault_root.to_string_lossy().to_string(),
            },
        );

        assert!(vault.read_text("escape/secret.md").is_err());
        assert!(vault.write_text("escape/new.md", "blocked", false).is_err());
        assert!(!outside.join("new.md").exists());
    }

    #[test]
    fn converts_moment_tokens() {
        assert_eq!(moment_to_chrono("YYYY-MM-DD"), "%Y-%m-%d");
    }

    #[test]
    fn applies_template_tokens_with_title() {
        let template = "Title: {{title}}";
        let result = apply_template_tokens(template, Some("My Note Title"));
        assert_eq!(result, "Title: My Note Title");
    }

    #[test]
    fn applies_template_tokens_without_title() {
        let template = "Title: {{title}}";
        let result = apply_template_tokens(template, None);
        assert_eq!(result, "Title: ");
    }

    #[test]
    fn applies_template_tokens_date_time() {
        use regex::Regex;

        let template = "Date: {{date}}, Time: {{time}}, DateTime: {{datetime}}";
        let result = apply_template_tokens(template, None);

        // Ensure tests are deterministic and not flaky by using Regex to validate
        // the format of the output, rather than relying on Local::now() which can
        // roll over during the test execution.
        let date_re = Regex::new(r"Date: \d{4}-\d{2}-\d{2}").unwrap();
        let time_re = Regex::new(r"Time: \d{2}:\d{2}").unwrap();
        let datetime_re = Regex::new(r"DateTime: \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.+").unwrap();

        assert!(
            date_re.is_match(&result),
            "Result missing or incorrect Date format: {result}"
        );
        assert!(
            time_re.is_match(&result),
            "Result missing or incorrect Time format: {result}"
        );
        assert!(
            datetime_re.is_match(&result),
            "Result missing or incorrect DateTime format: {result}"
        );
    }

    #[test]
    fn discovers_vaults_in_documents_like_roots() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("obsidian-cli-vault-discovery-{stamp}"));
        let docs = root.join("Documents");
        let vault = docs.join("WorkVault");
        let nested = docs.join("A").join("B").join("DeepVault");

        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        std::fs::create_dir_all(nested.join(".obsidian")).unwrap();

        let found = discover_vaults_under_roots(&[docs]);

        assert!(found.contains(&vault));
        assert!(found.contains(&nested));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn normalizes_known_vaults_before_deduplicating() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("obsidian-cli-vault-dedup-{stamp}"));
        let target = root.join("Documents").join("Main");
        let alias_root = root.join("Alias");

        std::fs::create_dir_all(target.join(".obsidian")).unwrap();
        std::fs::create_dir_all(&alias_root).unwrap();

        #[cfg(unix)]
        {
            let alias = alias_root.join("Main");
            std::os::unix::fs::symlink(&target, &alias).unwrap();

            let vaults = super::normalize_known_vaults(vec![
                KnownVault {
                    name: "Main".to_string(),
                    path: target.to_string_lossy().to_string(),
                },
                KnownVault {
                    name: "Main".to_string(),
                    path: alias.to_string_lossy().to_string(),
                },
            ]);

            assert_eq!(vaults.len(), 1);
            assert_eq!(
                vaults[0].path,
                target.canonicalize().unwrap().to_string_lossy()
            );
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vault_walks_ignore_hidden_directories_by_default() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("obsidian-cli-hidden-walk-{stamp}"));
        let vault_root = root.join("Vault");
        std::fs::create_dir_all(vault_root.join(".obsidian")).unwrap();
        std::fs::create_dir_all(vault_root.join(".trash")).unwrap();
        std::fs::create_dir_all(vault_root.join(".git")).unwrap();
        std::fs::create_dir_all(vault_root.join(".hidden")).unwrap();
        std::fs::write(vault_root.join("Visible.md"), "ok").unwrap();
        std::fs::write(vault_root.join(".trash").join("Deleted.md"), "trash").unwrap();
        std::fs::write(vault_root.join(".git").join("Ghost.md"), "git").unwrap();
        std::fs::write(vault_root.join(".hidden").join("Secret.md"), "hidden").unwrap();

        let runtime = RuntimePaths {
            base_dir: root.join("runtime"),
            cache_dir: root.join("runtime").join("cache"),
            state_file: root.join("runtime").join("state.json"),
            history_file: root.join("runtime").join("history.txt"),
        };
        std::fs::create_dir_all(&runtime.cache_dir).unwrap();
        let vault = VaultContext::new(
            runtime,
            KnownVault {
                name: "Vault".to_string(),
                path: vault_root.to_string_lossy().to_string(),
            },
        );

        let files = vault.list_files(None, Some("md")).unwrap();
        let paths = files
            .iter()
            .map(|file| file.rel_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["Visible.md"]);

        let trash_files = vault.list_files(Some(".trash"), Some("md")).unwrap();
        assert_eq!(trash_files[0].rel_path, ".trash/Deleted.md");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn applies_template_tokens_no_replacements() {
        let template = "Just regular text with {no} {{matching}} tokens";
        let result = apply_template_tokens(template, Some("Title"));
        assert_eq!(result, "Just regular text with {no} {{matching}} tokens");
    }

    #[test]
    fn applies_template_tokens_multiple_same_tokens() {
        let template = "{{title}} - {{title}}";
        let result = apply_template_tokens(template, Some("Double"));
        assert_eq!(result, "Double - Double");
    }

    #[test]
    fn alias_rows_use_named_value_field_for_tables() {
        let meta = parse_markdown("---\naliases:\n  - Inbox Alias\n---\nBody\n");
        let rows = super::alias_rows(&meta);
        assert_eq!(rows[0]["value"], "Inbox Alias");
    }

    #[test]
    fn open_or_init_vault_creates_obsidian_folder_for_path_selector() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("obsidian-cli-init-vault-{stamp}"));
        let runtime = RuntimePaths {
            base_dir: root.join("runtime"),
            cache_dir: root.join("runtime").join("cache"),
            state_file: root.join("runtime").join("state.json"),
            history_file: root.join("runtime").join("history.txt"),
        };
        let workspace = Workspace {
            cwd: root.clone(),
            runtime,
            known_vaults: Vec::new(),
            state: SessionState::default(),
            dirty: false,
        };

        let target = root.join("NewVault");
        let selector = target.to_string_lossy().to_string();
        let vault = workspace.open_or_init_vault(Some(&selector)).unwrap();

        assert!(target.join(".obsidian").is_dir());
        assert_eq!(vault.root, target);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn append_inserts_newline_when_missing() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("obsidian-cli-append-{stamp}"));
        let vault_root = root.join("Vault");
        std::fs::create_dir_all(vault_root.join(".obsidian")).unwrap();
        let file = vault_root.join("Inbox.md");
        std::fs::write(&file, "Hola Mundo").unwrap();

        let runtime = RuntimePaths {
            base_dir: root.join("runtime"),
            cache_dir: root.join("runtime").join("cache"),
            state_file: root.join("runtime").join("state.json"),
            history_file: root.join("runtime").join("history.txt"),
        };
        let workspace = Workspace {
            cwd: root.clone(),
            runtime,
            known_vaults: Vec::new(),
            state: SessionState::default(),
            dirty: false,
        };
        let vault = workspace
            .open_vault(Some(&vault_root.to_string_lossy()))
            .unwrap();
        vault.append_text("Inbox.md", "Nuevo texto", false).unwrap();

        let content = std::fs::read_to_string(file).unwrap();
        assert_eq!(content, "Hola Mundo\nNuevo texto");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn warm_index_does_not_rewrite_unchanged_cache() {
        let root = tempfile::tempdir().unwrap();
        let vault_root = root.path().join("Vault");
        let runtime = RuntimePaths {
            base_dir: root.path().join("runtime"),
            cache_dir: root.path().join("runtime/cache"),
            state_file: root.path().join("runtime/state.json"),
            history_file: root.path().join("runtime/history.txt"),
        };
        std::fs::create_dir_all(vault_root.join(".obsidian")).unwrap();
        std::fs::create_dir_all(&runtime.cache_dir).unwrap();
        std::fs::write(vault_root.join("Inbox.md"), "# Inbox\n[[Other]]\n").unwrap();
        let vault = VaultContext::new(
            runtime,
            KnownVault {
                name: "Vault".to_string(),
                path: vault_root.to_string_lossy().to_string(),
            },
        );

        vault.load_index().unwrap();
        let first_modified = std::fs::metadata(&vault.cache_file)
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        vault.load_index().unwrap();
        let second_modified = std::fs::metadata(&vault.cache_file)
            .unwrap()
            .modified()
            .unwrap();

        assert_eq!(first_modified, second_modified);
    }

    #[test]
    fn link_resolver_uses_exact_paths_and_nearby_notes() {
        let resolver =
            LinkResolver::new(["FolderA/Note.md".to_string(), "FolderB/Note.md".to_string()]);

        assert_eq!(
            resolver.resolve("FolderA/Note", Some("FolderB/Source.md")),
            Some("FolderA/Note.md".to_string())
        );
        assert_eq!(
            resolver.resolve("Note", Some("FolderB/Source.md")),
            Some("FolderB/Note.md".to_string())
        );
    }

    #[test]
    fn note_resolution_rejects_ambiguous_stems_without_context() {
        let mut index = super::VaultIndex::default();
        index.markdown.insert(
            "FolderA/Note.md".to_string(),
            super::MarkdownMeta::default(),
        );
        index.markdown.insert(
            "FolderB/Note.md".to_string(),
            super::MarkdownMeta::default(),
        );

        let error = index.resolve_note("Note", None).unwrap_err().to_string();
        assert!(error.contains("selector ambiguo"));
        assert!(error.contains("FolderA/Note.md"));
        assert_eq!(
            index
                .resolve_note("Note", Some("FolderB/Source.md"))
                .unwrap(),
            "FolderB/Note.md"
        );
    }

    #[test]
    fn move_and_rename_never_replace_existing_notes() {
        let root = tempfile::tempdir().unwrap();
        let vault_root = root.path().join("Vault");
        let runtime = RuntimePaths {
            base_dir: root.path().join("runtime"),
            cache_dir: root.path().join("runtime/cache"),
            state_file: root.path().join("runtime/state.json"),
            history_file: root.path().join("runtime/history.txt"),
        };
        std::fs::create_dir_all(vault_root.join(".obsidian")).unwrap();
        std::fs::create_dir_all(&runtime.cache_dir).unwrap();
        std::fs::write(vault_root.join("A.md"), "source").unwrap();
        std::fs::write(vault_root.join("B.md"), "destination").unwrap();
        let vault = VaultContext::new(
            runtime,
            KnownVault {
                name: "Vault".to_string(),
                path: vault_root.to_string_lossy().to_string(),
            },
        );

        assert!(vault.move_path("A.md", "B.md").is_err());
        assert!(vault.rename_path("A.md", "B.md").is_err());
        assert_eq!(
            std::fs::read_to_string(vault_root.join("A.md")).unwrap(),
            "source"
        );
        assert_eq!(
            std::fs::read_to_string(vault_root.join("B.md")).unwrap(),
            "destination"
        );
    }

    #[test]
    fn task_updates_preserve_crlf_line_endings() {
        let original = "# Tasks\r\n- [ ] Keep CRLF\r\n";
        let updated = super::replace_task_status(original, 2, "x").unwrap();
        assert_eq!(updated, "# Tasks\r\n- [x] Keep CRLF\r\n");
    }
}
