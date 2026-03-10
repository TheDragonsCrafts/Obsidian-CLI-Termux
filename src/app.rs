use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Result, anyhow, bail};
use csv::WriterBuilder;
use rand::prelude::IndexedRandom;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::parser::Invocation;
use crate::registry::{SupportLevel, command_help, find, overview};
use crate::vault::{
    DailySettings, FileRecord, VaultContext, VaultIndex, Workspace, alias_rows,
    apply_template_tokens, count_bytes, json_string, normalize_rel_path, property_rows,
    read_frontmatter, replace_task_status, split_frontmatter, write_frontmatter,
};

pub struct App {
    pub workspace: Workspace,
}

impl App {
    pub fn load() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        Ok(Self {
            workspace: Workspace::load(cwd)?,
        })
    }

    pub fn execute(&mut self, invocation: Invocation) -> Result<String> {
        let output = match invocation.command.as_str() {
            "help" => self.cmd_help(&invocation)?,
            "version" => self.cmd_version(),
            "vault" => self.cmd_vault(&invocation)?,
            "vaults" => self.cmd_vaults(&invocation)?,
            "vault:open" => self.cmd_vault_open(&invocation)?,
            "file" => self.cmd_file(&invocation)?,
            "files" => self.cmd_files(&invocation)?,
            "folder" => self.cmd_folder(&invocation)?,
            "folders" => self.cmd_folders(&invocation)?,
            "open" => self.cmd_open(&invocation)?,
            "create" => self.cmd_create(&invocation)?,
            "read" => self.cmd_read(&invocation)?,
            "append" => self.cmd_append(&invocation)?,
            "prepend" => self.cmd_prepend(&invocation)?,
            "move" => self.cmd_move(&invocation)?,
            "rename" => self.cmd_rename(&invocation)?,
            "delete" => self.cmd_delete(&invocation)?,
            "links" => self.cmd_links(&invocation)?,
            "backlinks" => self.cmd_backlinks(&invocation)?,
            "unresolved" => self.cmd_unresolved(&invocation)?,
            "orphans" => self.cmd_orphans(&invocation)?,
            "deadends" => self.cmd_deadends(&invocation)?,
            "outline" => self.cmd_outline(&invocation)?,
            "daily" => self.cmd_daily(&invocation)?,
            "daily:path" => self.cmd_daily_path(&invocation)?,
            "daily:read" => self.cmd_daily_read(&invocation)?,
            "daily:append" => self.cmd_daily_append(&invocation)?,
            "daily:prepend" => self.cmd_daily_prepend(&invocation)?,
            "search" => self.cmd_search(&invocation, false)?,
            "search:context" => self.cmd_search(&invocation, true)?,
            "tags" => self.cmd_tags(&invocation)?,
            "tag" => self.cmd_tag(&invocation)?,
            "tasks" => self.cmd_tasks(&invocation)?,
            "task" => self.cmd_task(&invocation)?,
            "aliases" => self.cmd_aliases(&invocation)?,
            "properties" => self.cmd_properties(&invocation)?,
            "property:set" => self.cmd_property_set(&invocation)?,
            "property:remove" => self.cmd_property_remove(&invocation)?,
            "property:read" => self.cmd_property_read(&invocation)?,
            "templates" => self.cmd_templates(&invocation)?,
            "template:read" => self.cmd_template_read(&invocation)?,
            "template:insert" => self.cmd_template_insert(&invocation)?,
            "bases" => self.cmd_bases(&invocation)?,
            "bookmarks" => self.cmd_bookmarks(&invocation)?,
            "bookmark" => self.cmd_bookmark(&invocation)?,
            "plugins" => self.cmd_plugins(&invocation, false)?,
            "plugins:enabled" => self.cmd_plugins(&invocation, true)?,
            "plugin" => self.cmd_plugin(&invocation)?,
            "plugin:enable" => self.cmd_plugin_toggle(&invocation, true)?,
            "plugin:disable" => self.cmd_plugin_toggle(&invocation, false)?,
            "plugin:uninstall" => self.cmd_plugin_uninstall(&invocation)?,
            "themes" => self.cmd_themes(&invocation)?,
            "theme" => self.cmd_theme(&invocation)?,
            "theme:set" => self.cmd_theme_set(&invocation)?,
            "theme:uninstall" => self.cmd_theme_uninstall(&invocation)?,
            "snippets" => self.cmd_snippets(&invocation, false)?,
            "snippets:enabled" => self.cmd_snippets(&invocation, true)?,
            "snippet:enable" => self.cmd_snippet_toggle(&invocation, true)?,
            "snippet:disable" => self.cmd_snippet_toggle(&invocation, false)?,
            "random" => self.cmd_random(&invocation, false)?,
            "random:read" => self.cmd_random(&invocation, true)?,
            "recents" => self.cmd_recents(&invocation)?,
            "wordcount" => self.cmd_wordcount(&invocation)?,
            "web" => self.cmd_web(&invocation)?,
            other => {
                if let Some(spec) = find(other) {
                    match spec.support {
                        SupportLevel::BridgeOnly => bail!(
                            "`{other}` requiere un bridge/plugin de Obsidian; el backend local de Termux no puede ejecutarlo todavía"
                        ),
                        _ => bail!("`{other}` está registrado pero aún no tiene implementación"),
                    }
                } else {
                    bail!("comando desconocido: {other}");
                }
            }
        };

        if invocation.global.copy && !output.is_empty() {
            copy_to_clipboard(&output)?;
        }

        self.workspace.save()?;
        Ok(output)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct PluginInfo {
    id: String,
    kind: String,
    enabled: bool,
    version: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ThemeInfo {
    name: String,
    version: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct SnippetInfo {
    name: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct PluginManifest {
    version: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThemeManifest {
    version: Option<String>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct AppearanceConfig {
    #[serde(rename = "cssTheme")]
    css_theme: Option<String>,
    #[serde(rename = "enabledCssSnippets", default)]
    enabled_css_snippets: Vec<String>,
}

fn required_param<'a>(invocation: &'a Invocation, key: &str) -> Result<&'a str> {
    invocation
        .param(key)
        .ok_or_else(|| anyhow!("faltó `{key}=...`"))
}

fn key_value_block(entries: &[(&str, String)]) -> String {
    entries
        .iter()
        .map(|(key, value)| format!("{key}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_strings(values: &[String], format: Option<&str>) -> Result<String> {
    match format.unwrap_or("text") {
        "json" => Ok(serde_json::to_string_pretty(values)?),
        "csv" => render_table(&["value"], values.iter().map(|value| [value]), b','),
        "tsv" => render_table(&["value"], values.iter().map(|value| [value]), b'\t'),
        _ => Ok(values.join("\n")),
    }
}

fn render_files(values: &[FileRecord], format: Option<&str>) -> Result<String> {
    let rows = values
        .iter()
        .map(|file| {
            json!({
                "path": file.rel_path,
                "size": file.len,
                "modified_ms": file.modified_ms,
                "markdown": file.is_markdown,
            })
        })
        .collect::<Vec<_>>();
    render_json_rows(rows, format, Some(&["path", "size", "modified_ms", "markdown"]))
}

fn render_counts(label: &str, values: &[(String, usize)], invocation: &Invocation) -> Result<String> {
    if invocation.param("format") == Some("json") {
        return Ok(serde_json::to_string_pretty(
            &values
                .iter()
                .map(|(name, count)| json!({ label: name, "count": count }))
                .collect::<Vec<_>>(),
        )?);
    }
    if invocation.has_flag("counts") || invocation.has_flag("verbose") {
        let rows = values
            .iter()
            .map(|(name, count)| vec![name.clone(), count.to_string()])
            .collect::<Vec<_>>();
        return render_table(&[label, "count"], &rows, if invocation.param("format") == Some("csv") { b',' } else { b'\t' });
    }
    render_strings(
        &values.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>(),
        invocation.param("format"),
    )
}

fn render_path_counts(values: &[(String, usize)], invocation: &Invocation) -> Result<String> {
    render_counts("path", values, invocation)
}

fn render_json_rows(rows: Vec<Value>, format: Option<&str>, headers: Option<&[&str]>) -> Result<String> {
    match format.unwrap_or("text") {
        "json" => Ok(serde_json::to_string_pretty(&rows)?),
        "csv" => render_value_rows(headers.unwrap_or(&[]), &rows, b','),
        "tsv" => render_value_rows(headers.unwrap_or(&[]), &rows, b'\t'),
        _ => {
            if let Some(headers) = headers {
                let mut simple_rows = Vec::new();
                for row in &rows {
                    if let Some(object) = row.as_object() {
                        simple_rows.push(
                            headers
                                .iter()
                                .map(|header| {
                                    object
                                        .get(*header)
                                        .map(display_value)
                                        .unwrap_or_default()
                                })
                                .collect::<Vec<_>>(),
                        );
                    }
                }
                render_table(headers, &simple_rows, b'\t')
            } else {
                Ok(rows
                    .iter()
                    .map(display_value)
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
    }
}

fn render_value_rows(headers: &[&str], rows: &[Value], delimiter: u8) -> Result<String> {
    let mapped = rows
        .iter()
        .map(|value| {
            headers
                .iter()
                .map(|header| {
                    value.get(*header)
                        .map(display_value)
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    render_table(headers, &mapped, delimiter)
}

fn render_table<I, T>(headers: &[&str], rows: I, delimiter: u8) -> Result<String>
where
    I: IntoIterator<Item = T>,
    T: IntoIterator,
    T::Item: AsRef<[u8]>,
{
    let mut writer = WriterBuilder::new()
        .delimiter(delimiter)
        .from_writer(Vec::new());
    if !headers.is_empty() {
        writer.write_record(headers)?;
    }
    for row in rows {
        writer.write_record(row)?;
    }
    let bytes = writer.into_inner()?;
    Ok(String::from_utf8(bytes)?)
}

fn display_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => values.iter().map(display_value).collect::<Vec<_>>().join(", "),
        Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn create_target_path(invocation: &Invocation) -> Result<String> {
    if let Some(path) = invocation.param("path") {
        return Ok(normalize_rel_path(path));
    }
    let name = required_param(invocation, "name")?;
    let mut path = normalize_rel_path(name);
    if Path::new(&path).extension().is_none() {
        path.push_str(".md");
    }
    Ok(path)
}

/// Checks if a line contains a search query.
/// If `case_sensitive` is false, `query` MUST already be lowercased.
fn contains_query(line: &str, query: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        line.contains(query)
    } else {
        line.to_ascii_lowercase().contains(query)
    }
}

fn task_matches(task: &crate::vault::TaskItem, invocation: &Invocation) -> bool {
    if let Some(status) = invocation.param("status") {
        return task.status == status;
    }
    if invocation.has_flag("done") {
        return task.status.trim().eq_ignore_ascii_case("x");
    }
    if invocation.has_flag("todo") {
        return task.status.trim().is_empty();
    }
    true
}

fn typed_value(kind: Option<&str>, raw: &str) -> Value {
    match kind.unwrap_or("text") {
        "number" => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(raw.to_string())),
        "checkbox" => Value::Bool(matches!(raw, "true" | "1" | "yes" | "on")),
        "list" => {
            if let Ok(value) = serde_json::from_str::<Value>(raw) {
                value
            } else {
                Value::Array(
                    raw.split(',')
                        .map(|value| Value::String(value.trim().to_string()))
                        .collect(),
                )
            }
        }
        _ => Value::String(raw.to_string()),
    }
}

fn flatten_bookmarks(value: &Value, lines: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                flatten_bookmarks(item, lines);
            }
        }
        Value::Object(map) => {
            if let Some(path) = map.get("path").and_then(Value::as_str) {
                lines.push(path.to_string());
            } else if let Some(url) = map.get("url").and_then(Value::as_str) {
                lines.push(url.to_string());
            }
            for value in map.values() {
                flatten_bookmarks(value, lines);
            }
        }
        _ => {}
    }
}

fn read_json_string_list(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    serde_json::from_str::<Vec<String>>(&fs::read_to_string(path)?).map_err(Into::into)
}

fn write_json_string_list(path: &Path, values: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(values)?)?;
    Ok(())
}

fn read_appearance(vault: &VaultContext) -> Result<AppearanceConfig> {
    let path = vault.obsidian_dir.join("appearance.json");
    if !path.exists() {
        return Ok(AppearanceConfig::default());
    }
    serde_json::from_str::<AppearanceConfig>(&fs::read_to_string(path)?).map_err(Into::into)
}

fn write_appearance(vault: &VaultContext, appearance: &AppearanceConfig) -> Result<()> {
    let path = vault.obsidian_dir.join("appearance.json");
    fs::write(path, serde_json::to_string_pretty(appearance)?)?;
    Ok(())
}

fn collect_snippets(vault: &VaultContext) -> Result<Vec<SnippetInfo>> {
    let appearance = read_appearance(vault)?;
    let snippets_dir = vault.obsidian_dir.join("snippets");
    let mut snippets = Vec::new();
    if !snippets_dir.exists() {
        return Ok(snippets);
    }
    for entry in fs::read_dir(snippets_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        snippets.push(SnippetInfo {
            name: stem.to_string(),
            enabled: appearance.enabled_css_snippets.iter().any(|item| item == stem),
        });
    }
    snippets.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(snippets)
}

fn is_text_candidate(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|value| value.to_str()),
        Some("txt" | "md" | "markdown" | "json" | "yaml" | "yml" | "csv" | "tsv")
    )
}

fn super_to_millis(time: std::time::SystemTime) -> u128 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}

fn copy_to_clipboard(text: &str) -> Result<()> {
    for program in ["termux-clipboard-set", "pbcopy", "clip"] {
        let mut child = match Command::new(program)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => continue,
        };
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write as _;
            stdin.write_all(text.as_bytes())?;
        }
        let _ = child.wait();
        return Ok(());
    }
    bail!("no se encontró un backend de clipboard compatible")
}

fn open_url(url: &str) -> Result<()> {
    let commands: &[(&str, &[&str])] = &[
        ("termux-open-url", &[]),
        ("xdg-open", &[]),
        ("explorer", &[]),
    ];
    for (program, prefix) in commands {
        let mut command = Command::new(program);
        for arg in *prefix {
            command.arg(arg);
        }
        if command.arg(url).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok() {
            return Ok(());
        }
    }
    bail!("no se pudo abrir la URL en este entorno")
}

impl App {
    fn cmd_help(&self, invocation: &Invocation) -> Result<String> {
        let topic = invocation
            .positionals
            .first()
            .map(String::as_str)
            .or_else(|| invocation.param("command"));
        Ok(match topic {
            Some(topic) => command_help(topic).unwrap_or_else(|| format!("sin ayuda para `{topic}`")),
            None => overview(),
        })
    }

    fn cmd_version(&self) -> String {
        format!(
            "obsidian-termux-cli {}\ncompat-profile: Obsidian CLI 1.12-style syntax",
            env!("CARGO_PKG_VERSION")
        )
    }

    fn cmd_vault(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let files = vault.list_files(None, None)?;
        let folders = vault.list_folders(None)?;
        let info = json!({
            "name": vault.name,
            "path": vault.root,
            "files": files.len(),
            "folders": folders.len(),
            "size": count_bytes(&files),
        });

        Ok(match invocation.param("info") {
            Some("path") => vault.root.to_string_lossy().to_string(),
            Some("files") => files.len().to_string(),
            Some("folders") => folders.len().to_string(),
            Some("size") => count_bytes(&files).to_string(),
            _ if invocation.param("format") == Some("json") => serde_json::to_string_pretty(&info)?,
            _ => key_value_block(&[
                ("name", vault.name),
                ("path", vault.root.to_string_lossy().to_string()),
                ("files", files.len().to_string()),
                ("folders", folders.len().to_string()),
                ("size", count_bytes(&files).to_string()),
            ]),
        })
    }

    fn cmd_vaults(&self, invocation: &Invocation) -> Result<String> {
        let mut vaults = self.workspace.known_vaults.clone();
        if vaults.is_empty() {
            if let Ok(current) = self.workspace.resolve_vault(None) {
                vaults.push(current);
            }
        }

        if invocation.has_flag("total") {
            return Ok(vaults.len().to_string());
        }

        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string_pretty(&vaults)?);
        }

        if invocation.has_flag("verbose") {
            return Ok(vaults
                .iter()
                .map(|vault| format!("{}\t{}", vault.name, vault.path))
                .collect::<Vec<_>>()
                .join("\n"));
        }

        Ok(vaults
            .iter()
            .map(|vault| vault.name.clone())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn cmd_vault_open(&mut self, invocation: &Invocation) -> Result<String> {
        let selector = invocation
            .param("name")
            .or_else(|| invocation.positionals.first().map(String::as_str))
            .ok_or_else(|| anyhow!("`vault:open` requiere `name=<vault>`"))?;
        let vault = self.workspace.open_vault(Some(selector))?;
        self.workspace.set_active_vault(&vault);
        self.workspace.state.active_file = None;
        Ok(vault.root.to_string_lossy().to_string())
    }

    fn cmd_file(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let abs = vault.rel_to_abs(&rel)?;
        let meta = fs::metadata(&abs)?;
        let info = json!({
            "path": rel,
            "absolute_path": abs,
            "size": meta.len(),
            "modified_ms": meta.modified().ok().map(super_to_millis),
            "markdown": abs.extension().and_then(|ext| ext.to_str()).map(|ext| ext.eq_ignore_ascii_case("md")).unwrap_or(false),
        });
        Ok(if invocation.param("format") == Some("json") {
            serde_json::to_string_pretty(&info)?
        } else {
            key_value_block(&[
                ("path", rel),
                ("absolute_path", abs.to_string_lossy().to_string()),
                ("size", meta.len().to_string()),
                (
                    "modified_ms",
                    meta.modified()
                        .ok()
                        .map(super_to_millis)
                        .unwrap_or_default()
                        .to_string(),
                ),
            ])
        })
    }

    fn cmd_files(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let files = vault.list_files(invocation.param("folder"), invocation.param("ext"))?;
        if invocation.has_flag("total") {
            return Ok(files.len().to_string());
        }
        render_files(&files, invocation.param("format"))
    }

    fn cmd_folder(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let folder = invocation
            .param("path")
            .ok_or_else(|| anyhow!("`folder` requiere `path=<carpeta>`"))?;
        let files = vault.list_files(Some(folder), None)?;
        let folders = vault.list_folders(Some(folder))?;
        let size = count_bytes(&files);
        Ok(match invocation.param("info") {
            Some("files") => files.len().to_string(),
            Some("folders") => folders.len().to_string(),
            Some("size") => size.to_string(),
            _ => key_value_block(&[
                ("path", folder.to_string()),
                ("files", files.len().to_string()),
                ("folders", folders.len().to_string()),
                ("size", size.to_string()),
            ]),
        })
    }

    fn cmd_folders(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let folders = vault.list_folders(invocation.param("folder"))?;
        if invocation.has_flag("total") {
            return Ok(folders.len().to_string());
        }
        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string_pretty(&folders)?);
        }
        Ok(folders.join("\n"))
    }

    fn cmd_open(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        self.workspace.set_active_file(&vault, &rel);
        Ok(rel)
    }

    fn cmd_create(&mut self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let rel = create_target_path(invocation)?;
        let mut content = invocation.param("content").unwrap_or_default().to_string();
        if let Some(template) = invocation.param("template") {
            let template_content = self.load_template_text(&vault, template, invocation.param("name"))?;
            if content.is_empty() {
                content = template_content;
            } else {
                content = format!("{template_content}\n{content}");
            }
        }
        vault.write_text(&rel, &content, invocation.has_flag("overwrite"))?;
        if invocation.has_flag("open") {
            self.workspace.set_active_file(&vault, &rel);
        }
        Ok(rel)
    }

    fn cmd_read(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        vault.read_text(&rel)
    }

    fn cmd_append(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let content = required_param(invocation, "content")?;
        vault.append_text(&rel, content, invocation.has_flag("inline"))?;
        self.workspace.set_active_file(&vault, &rel);
        Ok(rel)
    }

    fn cmd_prepend(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let content = required_param(invocation, "content")?;
        vault.prepend_text(&rel, content, invocation.has_flag("inline"))?;
        self.workspace.set_active_file(&vault, &rel);
        Ok(rel)
    }

    fn cmd_move(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let from = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let to = required_param(invocation, "to")?;
        let next = vault.move_path(&from, to)?;
        if active_file.as_deref() == Some(from.as_str()) {
            self.workspace.set_active_file(&vault, &next);
        }
        Ok(next)
    }

    fn cmd_rename(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let from = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let name = required_param(invocation, "name")?;
        let next = vault.rename_path(&from, name)?;
        if active_file.as_deref() == Some(from.as_str()) {
            self.workspace.set_active_file(&vault, &next);
        }
        Ok(next)
    }

    fn cmd_delete(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let status = vault.delete_path(&rel, invocation.has_flag("permanent"))?;
        self.workspace.clear_active_file(&vault, &rel);
        Ok(format!("{status}: {rel}"))
    }

    fn cmd_links(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let links = index
            .resolved_links
            .get(&rel)
            .map(|map| map.iter().map(|(path, count)| (path.clone(), *count)).collect::<Vec<_>>())
            .unwrap_or_default();
        if invocation.has_flag("total") {
            return Ok(links.len().to_string());
        }
        render_path_counts(&links, invocation)
    }

    fn cmd_backlinks(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let links = index
            .backlinks
            .get(&rel)
            .map(|map| map.iter().map(|(path, count)| (path.clone(), *count)).collect::<Vec<_>>())
            .unwrap_or_default();
        if invocation.has_flag("total") {
            return Ok(links.len().to_string());
        }
        render_path_counts(&links, invocation)
    }

    fn cmd_unresolved(&self, invocation: &Invocation) -> Result<String> {
        let (_, index, _) = self.open_local(invocation)?;
        let mut rows = Vec::new();
        let mut aggregated = HashMap::<String, usize>::new();
        for (source, unresolved) in &index.unresolved_links {
            for (target, count) in unresolved {
                *aggregated.entry(target.clone()).or_default() += count;
                rows.push(json!({
                    "source": source,
                    "target": target,
                    "count": count,
                }));
            }
        }

        if invocation.has_flag("total") {
            return Ok(if invocation.has_flag("verbose") {
                rows.len()
            } else {
                aggregated.len()
            }
            .to_string());
        }

        if invocation.has_flag("verbose") {
            return render_json_rows(
                rows,
                invocation.param("format"),
                Some(&["source", "target", "count"]),
            );
        }

        let mut compact = aggregated.into_iter().collect::<Vec<_>>();
        compact.sort_by(|left, right| left.0.cmp(&right.0));
        render_counts("target", &compact, invocation)
    }

    fn cmd_orphans(&self, invocation: &Invocation) -> Result<String> {
        let (_, index, _) = self.open_local(invocation)?;
        let mut orphans = index
            .markdown
            .keys()
            .filter(|path| !index.backlinks.contains_key(*path))
            .cloned()
            .collect::<Vec<_>>();
        orphans.sort();
        if invocation.has_flag("total") {
            return Ok(orphans.len().to_string());
        }
        render_strings(&orphans, invocation.param("format"))
    }

    fn cmd_deadends(&self, invocation: &Invocation) -> Result<String> {
        let (_, index, _) = self.open_local(invocation)?;
        let mut deadends = index
            .markdown
            .iter()
            .filter(|(path, meta)| {
                meta.links.is_empty()
                    || (!index.resolved_links.contains_key(*path)
                        && !index.unresolved_links.contains_key(*path))
            })
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        deadends.sort();
        if invocation.has_flag("total") {
            return Ok(deadends.len().to_string());
        }
        render_strings(&deadends, invocation.param("format"))
    }

    fn cmd_outline(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let meta = index
            .markdown
            .get(&rel)
            .ok_or_else(|| anyhow!("solo aplica a archivos Markdown"))?;
        if invocation.has_flag("total") {
            return Ok(meta.headings.len().to_string());
        }
        let format = invocation.param("format").unwrap_or("tree");
        Ok(match format {
            "json" => serde_json::to_string_pretty(&meta.headings)?,
            "md" => meta
                .headings
                .iter()
                .map(|heading| format!("{} {}", "#".repeat(heading.level), heading.text))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => meta
                .headings
                .iter()
                .map(|heading| format!("{}{}", "  ".repeat(heading.level.saturating_sub(1)), heading.text))
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }

    fn cmd_daily_path(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        vault.ensure_daily_note_path()
    }

    fn cmd_daily(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, path) = self.ensure_daily_exists(invocation)?;
        self.workspace.set_active_file(&vault, &path);
        Ok(path)
    }

    fn cmd_daily_read(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let path = vault.ensure_daily_note_path()?;
        vault.read_text(&path)
    }

    fn cmd_daily_append(&mut self, invocation: &Invocation) -> Result<String> {
        let content = required_param(invocation, "content")?;
        let (vault, path) = self.ensure_daily_exists(invocation)?;
        vault.append_text(&path, content, invocation.has_flag("inline"))?;
        if invocation.has_flag("open") {
            self.workspace.set_active_file(&vault, &path);
        }
        Ok(path)
    }

    fn cmd_daily_prepend(&mut self, invocation: &Invocation) -> Result<String> {
        let content = required_param(invocation, "content")?;
        let (vault, path) = self.ensure_daily_exists(invocation)?;
        vault.prepend_text(&path, content, invocation.has_flag("inline"))?;
        if invocation.has_flag("open") {
            self.workspace.set_active_file(&vault, &path);
        }
        Ok(path)
    }

    fn cmd_search(&self, invocation: &Invocation, with_context: bool) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let query = required_param(invocation, "query")?;
        let limit = invocation
            .param("limit")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(usize::MAX);
        let case_sensitive = invocation.has_flag("case");
        let search_query = if case_sensitive {
            query.to_string()
        } else {
            query.to_ascii_lowercase()
        };
        let scope = invocation.param("path").map(normalize_rel_path);
        let mut hits = Vec::<Value>::new();
        let mut seen_files = BTreeSet::new();

        for file in vault.list_files(None, None)? {
            if hits.len() >= limit {
                break;
            }
            if !file.is_markdown && !is_text_candidate(&file.rel_path) {
                continue;
            }
            if let Some(scope) = scope.as_deref() {
                if !file.rel_path.starts_with(scope) {
                    continue;
                }
            }
            let Ok(text) = vault.read_text(&file.rel_path) else {
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                if !contains_query(line, &search_query, case_sensitive) {
                    continue;
                }
                if with_context {
                    hits.push(json!({
                        "path": file.rel_path,
                        "line": index + 1,
                        "text": line,
                    }));
                } else if seen_files.insert(file.rel_path.clone()) {
                    hits.push(json!({ "path": file.rel_path }));
                }
                if hits.len() >= limit {
                    break;
                }
            }
        }

        if invocation.has_flag("total") {
            return Ok(hits.len().to_string());
        }

        if with_context {
            render_json_rows(hits, invocation.param("format"), Some(&["path", "line", "text"]))
        } else if invocation.param("format") == Some("json") {
            Ok(serde_json::to_string_pretty(&hits)?)
        } else {
            Ok(hits
                .iter()
                .filter_map(|value| value.get("path").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"))
        }
    }

    fn cmd_tags(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        if invocation.has_flag("active") || invocation.param("file").is_some() || invocation.param("path").is_some() {
            let rel = vault.resolve_target(
                &index,
                invocation.param("file"),
                invocation.param("path"),
                active_file.as_deref(),
            )?;
            let meta = index
                .markdown
                .get(&rel)
                .ok_or_else(|| anyhow!("solo aplica a archivos Markdown"))?;
            if invocation.has_flag("total") {
                return Ok(meta.tags.len().to_string());
            }
            return render_strings(&meta.tags, invocation.param("format"));
        }

        let mut counts = HashMap::<String, usize>::new();
        for meta in index.markdown.values() {
            for tag in &meta.tags {
                if let Some(count) = counts.get_mut(tag) {
                    *count += 1;
                } else {
                    counts.insert(tag.clone(), 1);
                }
            }
        }
        let mut rows = counts.into_iter().collect::<Vec<_>>();
        if invocation.param("sort") == Some("count") {
            rows.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        } else {
            rows.sort_by(|left, right| left.0.cmp(&right.0));
        }
        if invocation.has_flag("total") {
            return Ok(rows.len().to_string());
        }
        render_counts("tag", &rows, invocation)
    }

    fn cmd_tag(&self, invocation: &Invocation) -> Result<String> {
        let (_, index, _) = self.open_local(invocation)?;
        let mut target = required_param(invocation, "name")?.to_string();
        if !target.starts_with('#') {
            target = format!("#{target}");
        }
        let mut rows = Vec::new();
        for (path, meta) in &index.markdown {
            let count = meta.tags.iter().filter(|tag| *tag == &target).count();
            if count > 0 {
                rows.push((path.clone(), count));
            }
        }
        if invocation.has_flag("total") {
            return Ok(rows.len().to_string());
        }
        render_path_counts(&rows, invocation)
    }

    fn cmd_tasks(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let current_daily = if invocation.has_flag("daily") {
            Some(vault.ensure_daily_note_path()?)
        } else {
            None
        };
        let selected_path = if invocation.has_flag("active")
            || invocation.param("file").is_some()
            || invocation.param("path").is_some()
        {
            Some(vault.resolve_target(
                &index,
                invocation.param("file"),
                invocation.param("path"),
                active_file.as_deref(),
            )?)
        } else {
            None
        };

        let mut rows = Vec::new();
        for (path, meta) in &index.markdown {
            if let Some(selected) = selected_path.as_deref() {
                if path != selected {
                    continue;
                }
            }
            if let Some(daily) = current_daily.as_deref() {
                if path != daily {
                    continue;
                }
            }
            for task in &meta.tasks {
                if !task_matches(task, invocation) {
                    continue;
                }
                rows.push(json!({
                    "path": path,
                    "line": task.line,
                    "status": task.status,
                    "text": task.text,
                    "ref": format!("{path}:{}", task.line),
                }));
            }
        }

        if invocation.has_flag("total") {
            return Ok(rows.len().to_string());
        }

        if invocation.has_flag("verbose") || invocation.param("format").is_some() {
            return render_json_rows(rows, invocation.param("format"), Some(&["ref", "status", "text"]));
        }

        Ok(rows
            .iter()
            .map(|row| {
                let path = row.get("path").and_then(Value::as_str).unwrap_or_default();
                let line = row.get("line").and_then(Value::as_u64).unwrap_or_default();
                let text = row.get("text").and_then(Value::as_str).unwrap_or_default();
                format!("{path}:{line} {text}")
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn cmd_task(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let (path, task) = if let Some(reference) = invocation.param("ref") {
            index.task_by_ref(reference)?
        } else {
            let rel = vault.resolve_target(
                &index,
                invocation.param("file"),
                invocation.param("path"),
                active_file.as_deref(),
            )?;
            let line = required_param(invocation, "line")?.parse::<usize>()?;
            let task = index
                .markdown
                .get(&rel)
                .and_then(|meta| meta.tasks.iter().find(|task| task.line == line))
                .cloned()
                .ok_or_else(|| anyhow!("no se encontró la tarea"))?;
            (rel, task)
        };

        let desired_status = if invocation.has_flag("toggle") {
            Some(if task.status.trim().eq_ignore_ascii_case("x") {
                " "
            } else {
                "x"
            })
        } else if invocation.has_flag("done") {
            Some("x")
        } else if invocation.has_flag("todo") {
            Some(" ")
        } else {
            invocation.param("status")
        };

        if let Some(status) = desired_status {
            let current = vault.read_text(&path)?;
            let next = replace_task_status(&current, task.line, status)?;
            vault.write_text(&path, &next, true)?;
            self.workspace.set_active_file(&vault, &path);
            return Ok(format!("{path}:{} [{status}] {}", task.line, task.text));
        }

        Ok(format!("{path}:{} [{}] {}", task.line, task.status, task.text))
    }

    fn cmd_aliases(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        if invocation.has_flag("active") || invocation.param("file").is_some() || invocation.param("path").is_some() {
            let rel = vault.resolve_target(
                &index,
                invocation.param("file"),
                invocation.param("path"),
                active_file.as_deref(),
            )?;
            let meta = index
                .markdown
                .get(&rel)
                .ok_or_else(|| anyhow!("solo aplica a archivos Markdown"))?;
            return render_json_rows(alias_rows(meta), invocation.param("format"), Some(&["value"]));
        }

        let mut rows = Vec::new();
        for (path, meta) in &index.markdown {
            for alias in &meta.aliases {
                rows.push(json!({ "path": path, "alias": alias }));
            }
        }
        if invocation.has_flag("total") {
            return Ok(rows.len().to_string());
        }
        render_json_rows(rows, invocation.param("format"), Some(&["path", "alias"]))
    }

    fn cmd_properties(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        if invocation.has_flag("active") || invocation.param("file").is_some() || invocation.param("path").is_some() {
            let rel = vault.resolve_target(
                &index,
                invocation.param("file"),
                invocation.param("path"),
                active_file.as_deref(),
            )?;
            let meta = index
                .markdown
                .get(&rel)
                .ok_or_else(|| anyhow!("solo aplica a archivos Markdown"))?;
            if invocation.param("format") == Some("yaml") {
                return Ok(serde_yaml::to_string(&meta.properties)?);
            }
            return render_json_rows(property_rows(meta), invocation.param("format"), Some(&["name", "value"]));
        }

        let name_filter = invocation.param("name");
        let mut counts = HashMap::<String, usize>::new();
        for meta in index.markdown.values() {
            for name in meta.properties.keys() {
                if name_filter.is_none_or(|target| target == name) {
                    if let Some(count) = counts.get_mut(name) {
                        *count += 1;
                    } else {
                        counts.insert(name.clone(), 1);
                    }
                }
            }
        }
        let mut rows = counts.into_iter().collect::<Vec<_>>();
        if invocation.param("sort") == Some("count") {
            rows.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        } else {
            rows.sort_by(|left, right| left.0.cmp(&right.0));
        }
        if invocation.has_flag("total") {
            return Ok(rows.len().to_string());
        }
        render_counts("name", &rows, invocation)
    }

    fn cmd_property_set(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let name = required_param(invocation, "name")?;
        let value = required_param(invocation, "value")?;
        let abs = vault.rel_to_abs(&rel)?;
        let original = fs::read_to_string(&abs)?;
        let (_, body) = split_frontmatter(&original);
        let mut properties = read_frontmatter(&abs)?;
        properties.insert(name.to_string(), typed_value(invocation.param("type"), value));
        write_frontmatter(&abs, &properties, body)?;
        self.workspace.set_active_file(&vault, &rel);
        Ok(rel)
    }

    fn cmd_property_remove(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let name = required_param(invocation, "name")?;
        let abs = vault.rel_to_abs(&rel)?;
        let original = fs::read_to_string(&abs)?;
        let (_, body) = split_frontmatter(&original);
        let mut properties = read_frontmatter(&abs)?;
        properties.remove(name);
        write_frontmatter(&abs, &properties, body)?;
        self.workspace.set_active_file(&vault, &rel);
        Ok(rel)
    }

    fn cmd_property_read(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let name = required_param(invocation, "name")?;
        let meta = index
            .markdown
            .get(&rel)
            .ok_or_else(|| anyhow!("solo aplica a archivos Markdown"))?;
        let value = meta
            .properties
            .get(name)
            .ok_or_else(|| anyhow!("propiedad no encontrada"))?;
        Ok(if invocation.param("format") == Some("json") {
            serde_json::to_string_pretty(value)?
        } else {
            json_string(value)
        })
    }

    fn cmd_templates(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let templates = self.list_templates(&vault)?;
        if invocation.has_flag("total") {
            return Ok(templates.len().to_string());
        }
        render_strings(&templates, invocation.param("format"))
    }

    fn cmd_template_read(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let name = required_param(invocation, "name")?;
        let mut template = self.load_template_text(&vault, name, invocation.param("title"))?;
        if invocation.has_flag("resolve") {
            template = apply_template_tokens(&template, invocation.param("title"));
        }
        Ok(template)
    }

    fn cmd_template_insert(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let active_file = active_file.ok_or_else(|| anyhow!("no hay archivo activo"))?;
        let _ = index
            .markdown
            .get(&active_file)
            .ok_or_else(|| anyhow!("el archivo activo debe ser Markdown"))?;
        let name = required_param(invocation, "name")?;
        let template = self.load_template_text(&vault, name, None)?;
        vault.append_text(&active_file, &template, false)?;
        Ok(active_file)
    }

    fn cmd_bases(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let bases = vault.list_bases()?;
        render_strings(&bases, invocation.param("format"))
    }

    fn cmd_random(&mut self, invocation: &Invocation, read: bool) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let files = vault.list_files(invocation.param("folder"), Some("md"))?;
        let mut rng = rand::rng();
        let target = files
            .choose(&mut rng)
            .cloned()
            .ok_or_else(|| anyhow!("no hay notas Markdown en el vault"))?;
        self.workspace.set_active_file(&vault, &target.rel_path);
        if read {
            let content = vault.read_text(&target.rel_path)?;
            Ok(format!("{}\n\n{}", target.rel_path, content))
        } else {
            Ok(target.rel_path)
        }
    }

    fn cmd_recents(&self, invocation: &Invocation) -> Result<String> {
        let rows = self
            .workspace
            .state
            .recents
            .iter()
            .map(|entry| {
                json!({
                    "vault": entry.vault_path,
                    "path": entry.path,
                    "opened_at": entry.opened_at,
                })
            })
            .collect::<Vec<_>>();
        if invocation.has_flag("total") {
            return Ok(rows.len().to_string());
        }
        render_json_rows(rows, invocation.param("format"), Some(&["path", "opened_at"]))
    }

    fn cmd_wordcount(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let rel = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let meta = index
            .markdown
            .get(&rel)
            .ok_or_else(|| anyhow!("solo aplica a archivos Markdown"))?;
        if invocation.has_flag("words") {
            return Ok(meta.word_count.to_string());
        }
        if invocation.has_flag("characters") {
            return Ok(meta.character_count.to_string());
        }
        Ok(key_value_block(&[
            ("path", rel),
            ("words", meta.word_count.to_string()),
            ("characters", meta.character_count.to_string()),
        ]))
    }

    fn cmd_bookmarks(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let path = vault.obsidian_dir.join("bookmarks.json");
        if !path.exists() {
            return Ok(String::new());
        }
        let text = fs::read_to_string(path)?;
        let value: Value = serde_json::from_str(&text)?;
        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string_pretty(&value)?);
        }
        let mut lines = Vec::new();
        flatten_bookmarks(&value, &mut lines);
        if invocation.has_flag("total") {
            return Ok(lines.len().to_string());
        }
        Ok(lines.join("\n"))
    }

    fn cmd_bookmark(&self, _invocation: &Invocation) -> Result<String> {
        bail!("`bookmark` todavía no escribe `bookmarks.json`; el esquema varía entre versiones de Obsidian")
    }

    fn cmd_plugins(&self, invocation: &Invocation, enabled_only: bool) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let mut plugins = self.collect_plugins(&vault)?;
        if let Some(filter) = invocation.param("filter") {
            plugins.retain(|plugin| plugin.kind == filter);
        }
        if enabled_only {
            plugins.retain(|plugin| plugin.enabled);
        }
        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string_pretty(&plugins)?);
        }
        if invocation.has_flag("versions") {
            return Ok(plugins
                .iter()
                .map(|plugin| {
                    format!(
                        "{}\t{}\t{}\t{}",
                        plugin.id,
                        plugin.kind,
                        plugin.enabled,
                        plugin.version.clone().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"));
        }
        Ok(plugins
            .iter()
            .map(|plugin| plugin.id.clone())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn cmd_plugin(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let id = required_param(invocation, "id")?;
        let plugin = self
            .collect_plugins(&vault)?
            .into_iter()
            .find(|plugin| plugin.id == id)
            .ok_or_else(|| anyhow!("plugin no encontrado"))?;
        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string_pretty(&plugin)?);
        }
        Ok(key_value_block(&[
            ("id", plugin.id),
            ("kind", plugin.kind),
            ("enabled", plugin.enabled.to_string()),
            ("version", plugin.version.unwrap_or_default()),
            ("name", plugin.name.unwrap_or_default()),
        ]))
    }

    fn cmd_plugin_toggle(&self, invocation: &Invocation, enabled: bool) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let id = required_param(invocation, "id")?;
        let filter = invocation.param("filter").unwrap_or("community");
        let path = if filter == "core" {
            vault.obsidian_dir.join("core-plugins.json")
        } else {
            vault.obsidian_dir.join("community-plugins.json")
        };
        let mut values = read_json_string_list(&path)?;
        if enabled {
            if !values.iter().any(|value| value == id) {
                values.push(id.to_string());
            }
        } else {
            values.retain(|value| value != id);
        }
        write_json_string_list(&path, &values)?;
        Ok(id.to_string())
    }

    fn cmd_plugin_uninstall(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let id = required_param(invocation, "id")?;
        if id.is_empty() || id.contains('/') || id.contains('\\') || id == ".." || id == "." {
            bail!("invalid plugin id");
        }
        let plugin_dir = vault.obsidian_dir.join("plugins").join(id);
        if plugin_dir.exists() {
            fs::remove_dir_all(&plugin_dir)?;
        }
        let path = vault.obsidian_dir.join("community-plugins.json");
        let mut values = read_json_string_list(&path)?;
        values.retain(|value| value != id);
        write_json_string_list(&path, &values)?;
        Ok(id.to_string())
    }

    fn cmd_themes(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let themes = self.collect_themes(&vault)?;
        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string_pretty(&themes)?);
        }
        if invocation.has_flag("versions") {
            return Ok(themes
                .iter()
                .map(|theme| format!("{}\t{}", theme.name, theme.version.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        Ok(themes
            .iter()
            .map(|theme| theme.name.clone())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn cmd_theme(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let appearance = read_appearance(&vault)?;
        if let Some(name) = invocation.param("name") {
            let theme = self
                .collect_themes(&vault)?
                .into_iter()
                .find(|theme| theme.name == name)
                .ok_or_else(|| anyhow!("tema no encontrado"))?;
            if invocation.param("format") == Some("json") {
                return Ok(serde_json::to_string_pretty(&theme)?);
            }
            return Ok(key_value_block(&[
                ("name", theme.name),
                ("version", theme.version.unwrap_or_default()),
                ("active", (appearance.css_theme.as_deref() == Some(name)).to_string()),
            ]));
        }
        Ok(appearance.css_theme.unwrap_or_default())
    }

    fn cmd_theme_set(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let name = required_param(invocation, "name")?;
        let mut appearance = read_appearance(&vault)?;
        appearance.css_theme = if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        };
        write_appearance(&vault, &appearance)?;
        Ok(name.to_string())
    }

    fn cmd_theme_uninstall(&self, invocation: &Invocation) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let name = required_param(invocation, "name")?;
        if name.is_empty() || name.contains('/') || name.contains('\\') || name == ".." || name == "." {
            bail!("invalid theme name");
        }
        let theme_dir = vault.obsidian_dir.join("themes").join(name);
        if theme_dir.exists() {
            fs::remove_dir_all(&theme_dir)?;
        }
        let mut appearance = read_appearance(&vault)?;
        if appearance.css_theme.as_deref() == Some(name) {
            appearance.css_theme = None;
            write_appearance(&vault, &appearance)?;
        }
        Ok(name.to_string())
    }

    fn cmd_snippets(&self, invocation: &Invocation, enabled_only: bool) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let appearance = read_appearance(&vault)?;
        let mut snippets = collect_snippets(&vault)?;
        if enabled_only {
            snippets.retain(|snippet| appearance.enabled_css_snippets.contains(&snippet.name));
        }
        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string_pretty(&snippets)?);
        }
        Ok(snippets
            .iter()
            .map(|snippet| snippet.name.clone())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    fn cmd_snippet_toggle(&self, invocation: &Invocation, enabled: bool) -> Result<String> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let name = required_param(invocation, "name")?;
        let mut appearance = read_appearance(&vault)?;
        if enabled {
            if !appearance.enabled_css_snippets.iter().any(|item| item == name) {
                appearance.enabled_css_snippets.push(name.to_string());
            }
        } else {
            appearance.enabled_css_snippets.retain(|item| item != name);
        }
        write_appearance(&vault, &appearance)?;
        Ok(name.to_string())
    }

    fn cmd_web(&self, invocation: &Invocation) -> Result<String> {
        let url = required_param(invocation, "url")?;
        open_url(url)?;
        Ok(url.to_string())
    }

    fn open_local(&self, invocation: &Invocation) -> Result<(VaultContext, VaultIndex, Option<String>)> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let index = vault.load_index()?;
        let active_file = self.workspace.active_file_for(&vault);
        Ok((vault, index, active_file))
    }

    fn ensure_daily_exists(&self, invocation: &Invocation) -> Result<(VaultContext, String)> {
        let vault = self.workspace.open_vault(invocation.global.vault.as_deref())?;
        let path = vault.ensure_daily_note_path()?;
        if !vault.rel_to_abs(&path)?.exists() {
            let settings: DailySettings = vault.daily_settings()?;
            let content = if let Some(template) = settings.template.as_deref() {
                self.load_template_text(&vault, template, Some(&path))?
            } else {
                String::new()
            };
            vault.write_text(&path, &content, true)?;
        }
        Ok((vault, path))
    }

    fn list_templates(&self, vault: &VaultContext) -> Result<Vec<String>> {
        let folder = vault
            .templates_folder()?
            .ok_or_else(|| anyhow!("no hay carpeta de templates configurada en `.obsidian/templates.json`"))?;
        let mut templates = vault
            .list_files(Some(&folder), Some("md"))?
            .into_iter()
            .map(|file| file.rel_path)
            .collect::<Vec<_>>();
        templates.sort();
        Ok(templates)
    }

    fn load_template_text(&self, vault: &VaultContext, name: &str, title: Option<&str>) -> Result<String> {
        let folder = vault
            .templates_folder()?
            .ok_or_else(|| anyhow!("no hay carpeta de templates configurada"))?;
        let rel = if name.ends_with(".md") {
            normalize_rel_path(&format!("{folder}/{name}"))
        } else {
            normalize_rel_path(&format!("{folder}/{name}.md"))
        };
        let template = vault.read_text(&rel)?;
        Ok(apply_template_tokens(&template, title))
    }

    fn collect_plugins(&self, vault: &VaultContext) -> Result<Vec<PluginInfo>> {
        let community_enabled = read_json_string_list(&vault.obsidian_dir.join("community-plugins.json"))?;
        let core_enabled = read_json_string_list(&vault.obsidian_dir.join("core-plugins.json"))?;
        let mut plugins = Vec::new();

        for id in core_enabled {
            plugins.push(PluginInfo {
                id,
                kind: "core".to_string(),
                enabled: true,
                version: None,
                name: None,
            });
        }

        let community_dir = vault.obsidian_dir.join("plugins");
        if community_dir.exists() {
            for entry in fs::read_dir(community_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let id = entry.file_name().to_string_lossy().to_string();
                let manifest_path = entry.path().join("manifest.json");
                let manifest = if manifest_path.exists() {
                    serde_json::from_str::<PluginManifest>(&fs::read_to_string(manifest_path)?).ok()
                } else {
                    None
                };
                plugins.push(PluginInfo {
                    id: id.clone(),
                    kind: "community".to_string(),
                    enabled: community_enabled.iter().any(|value| value == &id),
                    version: manifest.as_ref().and_then(|value| value.version.clone()),
                    name: manifest.as_ref().and_then(|value| value.name.clone()),
                });
            }
        }

        plugins.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(plugins)
    }

    fn collect_themes(&self, vault: &VaultContext) -> Result<Vec<ThemeInfo>> {
        let theme_dir = vault.obsidian_dir.join("themes");
        let mut themes = Vec::new();
        if !theme_dir.exists() {
            return Ok(themes);
        }
        for entry in fs::read_dir(theme_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let manifest_path = entry.path().join("manifest.json");
            let manifest = if manifest_path.exists() {
                serde_json::from_str::<ThemeManifest>(&fs::read_to_string(manifest_path)?).ok()
            } else {
                None
            };
            themes.push(ThemeInfo {
                name,
                version: manifest.and_then(|value| value.version),
            });
        }
        themes.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(themes)
    }
}
