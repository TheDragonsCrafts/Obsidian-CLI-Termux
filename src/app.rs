use std::collections::{BTreeSet, HashMap};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use csv::WriterBuilder;
use rand::prelude::IndexedRandom;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::parser::{Invocation, Request, parse_line};
use crate::registry::{
    COMMANDS, SupportLevel, command_aliases, command_help, command_usage, find, localize_category,
    overview,
};
use crate::updater;
use crate::vault::{
    DailySettings, FileRecord, VaultContext, VaultIndex, Workspace, alias_rows,
    apply_template_tokens, atomic_write_bytes, count_bytes, json_string, normalize_rel_path,
    property_rows, read_frontmatter, replace_task_status, split_frontmatter, write_frontmatter,
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
        if let Some(format) = invocation.param("format")
            && !matches!(
                format,
                "text" | "json" | "jsonl" | "csv" | "tsv" | "yaml" | "tree" | "md"
            )
        {
            bail!("formato no soportado: {format}");
        }
        let output = match invocation.command.as_str() {
            "help" => self.cmd_help(&invocation)?,
            "version" => self.cmd_version(),
            "update" => self.cmd_update(&invocation)?,
            "language" => self.cmd_language(&invocation)?,
            "commands" => self.cmd_commands(&invocation)?,
            "doctor" => self.cmd_doctor(&invocation)?,
            "batch" => self.cmd_batch(&invocation)?,
            "vault" => self.cmd_vault(&invocation)?,
            "vault:init" => self.cmd_vault_init(&invocation)?,
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

        self.workspace.save_if_dirty()?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{create_target_path, required_param_any, typed_value};
    use crate::parser::{GlobalOptions, Invocation};

    #[test]
    fn create_accepts_file_alias_for_name() {
        let invocation = Invocation {
            command: "create".to_string(),
            params: BTreeMap::from([("file".to_string(), "Inbox".to_string())]),
            flags: BTreeSet::new(),
            positionals: Vec::new(),
            global: GlobalOptions::default(),
        };
        let path = create_target_path(&invocation).unwrap();
        assert_eq!(path, "Inbox.md");
    }

    #[test]
    fn required_param_any_accepts_aliases() {
        let invocation = Invocation {
            command: "rename".to_string(),
            params: BTreeMap::from([("to".to_string(), "Nuevo".to_string())]),
            flags: BTreeSet::new(),
            positionals: Vec::new(),
            global: GlobalOptions::default(),
        };

        assert_eq!(
            required_param_any(&invocation, &["name", "to"]).unwrap(),
            "Nuevo"
        );
    }

    #[test]
    fn typed_property_values_follow_documented_contract() {
        assert_eq!(
            typed_value(Some("bool"), "true").unwrap(),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            typed_value(Some("json"), r#"{"nested":1}"#).unwrap(),
            serde_json::json!({"nested": 1})
        );
        assert!(typed_value(Some("number"), "not-a-number").is_err());
        assert!(typed_value(Some("unknown"), "value").is_err());
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

fn param_any<'a>(invocation: &'a Invocation, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| invocation.param(key))
}

fn required_param_any<'a>(invocation: &'a Invocation, keys: &[&str]) -> Result<&'a str> {
    param_any(invocation, keys).ok_or_else(|| {
        let expected = keys
            .iter()
            .map(|key| format!("`{key}=...`"))
            .collect::<Vec<_>>()
            .join(" o ");
        anyhow!("faltó {expected}")
    })
}

fn warn_frontmatter_error(path: &str, meta: &crate::vault::MarkdownMeta) {
    if let Some(error) = meta.frontmatter_error.as_deref() {
        eprintln!("[warn] frontmatter inválido en {path}: {error}");
    }
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
    render_json_rows(
        rows,
        format,
        Some(&["path", "size", "modified_ms", "markdown"]),
    )
}

fn render_counts(
    label: &str,
    values: &[(String, usize)],
    invocation: &Invocation,
) -> Result<String> {
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
        return render_table(
            &[label, "count"],
            &rows,
            if invocation.param("format") == Some("csv") {
                b','
            } else {
                b'\t'
            },
        );
    }
    render_strings(
        &values
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>(),
        invocation.param("format"),
    )
}

fn render_path_counts(values: &[(String, usize)], invocation: &Invocation) -> Result<String> {
    render_counts("path", values, invocation)
}

fn render_json_rows(
    rows: Vec<Value>,
    format: Option<&str>,
    headers: Option<&[&str]>,
) -> Result<String> {
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
                                    object.get(*header).map(display_value).unwrap_or_default()
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
                .map(|header| value.get(*header).map(display_value).unwrap_or_default())
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
        Value::Array(values) => values
            .iter()
            .map(display_value)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn create_target_path(invocation: &Invocation) -> Result<String> {
    if let Some(path) = invocation.param("path") {
        return Ok(normalize_rel_path(path));
    }
    let name = invocation
        .param("name")
        .or_else(|| invocation.param("file"))
        .ok_or_else(|| anyhow!("faltó `name=...` (también acepta `file=...`)"))?;
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

fn typed_value(kind: Option<&str>, raw: &str) -> Result<Value> {
    match kind.unwrap_or("text") {
        "text" | "string" => Ok(Value::String(raw.to_string())),
        "number" => raw
            .parse::<serde_json::Number>()
            .map(Value::Number)
            .map_err(|_| anyhow!("valor numérico inválido: {raw}")),
        "checkbox" | "bool" | "boolean" => match raw.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Value::Bool(true)),
            "false" | "0" | "no" | "off" => Ok(Value::Bool(false)),
            _ => bail!("valor booleano inválido: {raw}"),
        },
        "json" => serde_json::from_str(raw).map_err(Into::into),
        "list" => {
            if let Ok(value) = serde_json::from_str::<Value>(raw) {
                if value.is_array() {
                    Ok(value)
                } else {
                    bail!("type=list requiere un array JSON o valores separados por coma")
                }
            } else {
                Ok(Value::Array(
                    raw.split(',')
                        .map(|value| Value::String(value.trim().to_string()))
                        .collect(),
                ))
            }
        }
        other => bail!("tipo de propiedad no soportado: {other}"),
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
    atomic_write_bytes(path, serde_json::to_string_pretty(values)?.as_bytes())?;
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
    atomic_write_bytes(&path, serde_json::to_string_pretty(appearance)?.as_bytes())?;
    Ok(())
}

fn command_exists(program: &str) -> bool {
    if program.contains('/') || program.contains('\\') {
        return is_executable_file(Path::new(program));
    }

    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|dir| {
        if is_executable_file(&dir.join(program)) {
            return true;
        }
        if cfg!(windows) {
            return ["exe", "cmd", "bat", "com"]
                .iter()
                .any(|extension| is_executable_file(&dir.join(format!("{program}.{extension}"))));
        }
        false
    })
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn dir_writable(path: &Path) -> bool {
    if !path.exists() && fs::create_dir_all(path).is_err() {
        return false;
    }
    tempfile::NamedTempFile::new_in(path).is_ok()
}

fn is_termux_environment() -> bool {
    env::var("TERMUX_VERSION").is_ok()
        || env::var("PREFIX")
            .map(|value| value.contains("/com.termux/files/usr"))
            .unwrap_or(false)
        || command_exists("termux-open")
        || command_exists("termux-open-url")
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
            enabled: appearance
                .enabled_css_snippets
                .iter()
                .any(|item| item == stem),
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
        if command
            .arg(url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
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
            Some(topic) => command_help(topic, self.workspace.language())
                .unwrap_or_else(|| format!("sin ayuda para `{topic}`")),
            None => overview(self.workspace.language()),
        })
    }

    fn cmd_version(&self) -> String {
        if self.workspace.language() == "en" {
            format!(
                "obsidian-termux-cli {}\ncompat-profile: Obsidian CLI 1.12-style syntax",
                env!("CARGO_PKG_VERSION")
            )
        } else {
            format!(
                "obsidian-termux-cli {}\nperfil-de-compatibilidad: sintaxis estilo Obsidian CLI 1.12",
                env!("CARGO_PKG_VERSION")
            )
        }
    }

    fn cmd_update(&self, invocation: &Invocation) -> Result<String> {
        let force = invocation.has_flag("force");
        updater::manual_update(force, self.workspace.language())
    }

    fn cmd_language(&mut self, invocation: &Invocation) -> Result<String> {
        let selected = invocation
            .param("set")
            .or_else(|| invocation.param("lang"))
            .or_else(|| invocation.positionals.first().map(String::as_str));

        if let Some(value) = selected {
            let normalized = value.trim().to_lowercase();
            let language = match normalized.as_str() {
                "es" | "espanol" | "español" | "spanish" => "es",
                "en" | "english" | "ingles" | "inglés" => "en",
                _ => bail!("idioma no soportado: {value}. Usa `es` o `en`"),
            };
            self.workspace.set_language(language);
        }

        let language = self.workspace.language();
        Ok(if language == "en" {
            format!("language: {language}")
        } else {
            format!("idioma: {language}")
        })
    }

    fn cmd_commands(&self, invocation: &Invocation) -> Result<String> {
        let support_filter = invocation.param("support");
        let category_filter = invocation.param("category");
        let rows = COMMANDS
            .iter()
            .filter(|spec| support_filter.is_none_or(|support| spec.support.label() == support))
            .filter(|spec| category_filter.is_none_or(|category| spec.category == category))
            .filter(|spec| !invocation.has_flag("available") || command_is_available(spec.name))
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "category": spec.category,
                    "category_label": localize_category(spec.category, self.workspace.language()),
                    "support": spec.support.label(),
                    "summary": spec.summary,
                    "usage": command_usage(spec.name),
                    "aliases": command_aliases(spec.name),
                    "available": command_is_available(spec.name),
                })
            })
            .collect::<Vec<_>>();

        if invocation.has_flag("total") {
            return Ok(rows.len().to_string());
        }

        render_json_rows(
            rows,
            invocation.param("format"),
            Some(&[
                "name",
                "category",
                "support",
                "available",
                "summary",
                "usage",
                "aliases",
            ]),
        )
    }

    fn cmd_doctor(&mut self, invocation: &Invocation) -> Result<String> {
        let mut repairs = Vec::new();
        if invocation.has_flag("fix") {
            fs::create_dir_all(&self.workspace.runtime.base_dir)?;
            fs::create_dir_all(&self.workspace.runtime.cache_dir)?;
            self.workspace.refresh_known_vaults()?;
            repairs.push("runtime_directories_ensured");
            repairs.push("vault_discovery_refreshed");
        }

        let prefix = env::var("PREFIX").unwrap_or_default();
        let config_home = dirs::config_dir()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        let runtime = &self.workspace.runtime;
        let vaults = self
            .workspace
            .known_vaults
            .iter()
            .map(|vault| {
                let path = vault.path_buf();
                json!({
                    "name": vault.name,
                    "path": vault.path,
                    "exists": path.join(".obsidian").is_dir(),
                })
            })
            .collect::<Vec<_>>();
        let active_context = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())
            .ok();
        let active_vault = active_context.as_ref().map(|vault| {
            json!({
                "name": vault.name,
                "path": vault.root,
            })
        });
        let storage_paths = [
            dirs::home_dir().map(|home| home.join("storage").join("shared").join("Documents")),
            Some(Path::new("/storage/emulated/0/Documents").to_path_buf()),
            Some(Path::new("/sdcard/Documents").to_path_buf()),
        ];
        let storage = storage_paths
            .into_iter()
            .flatten()
            .map(|path| json!({ "path": path, "exists": path.exists() }))
            .collect::<Vec<_>>();
        let termux_tools = [
            "pkg",
            "termux-open",
            "termux-open-url",
            "termux-clipboard-set",
        ]
        .into_iter()
        .map(|program| json!({ "program": program, "available": command_exists(program) }))
        .collect::<Vec<_>>();
        let cargo_tools = ["cargo", "rustc", "clang", "git"]
            .into_iter()
            .map(|program| json!({ "program": program, "available": command_exists(program) }))
            .collect::<Vec<_>>();

        let base_writable = dir_writable(&runtime.base_dir);
        let cache_writable = dir_writable(&runtime.cache_dir);
        let termux = is_termux_environment();
        let mut checks = Vec::new();
        let mut recommendations = Vec::new();

        checks.push(json!({
            "id": "runtime.base_writable",
            "status": if base_writable { "ok" } else { "error" },
            "message": if base_writable { "runtime directory is writable" } else { "runtime directory is not writable" },
            "fix": if base_writable { Value::Null } else { json!("check storage permissions and OBSIDIAN_CLI_HOME") },
        }));
        checks.push(json!({
            "id": "runtime.cache_writable",
            "status": if cache_writable { "ok" } else { "error" },
            "message": if cache_writable { "cache directory is writable" } else { "cache directory is not writable" },
            "fix": if cache_writable { Value::Null } else { json!("run doctor fix or select a writable OBSIDIAN_CLI_HOME") },
        }));

        let invalid_vaults = vaults
            .iter()
            .filter(|vault| vault["exists"] == Value::Bool(false))
            .count();
        checks.push(json!({
            "id": "vault.discovery",
            "status": if invalid_vaults == 0 { "ok" } else { "warning" },
            "message": if invalid_vaults == 0 {
                format!("{} known vault(s) are valid", vaults.len())
            } else {
                format!("{invalid_vaults} stale vault path(s) found")
            },
            "fix": if invalid_vaults == 0 { Value::Null } else { json!("run doctor fix") },
        }));

        if active_vault.is_some() {
            checks.push(json!({
                "id": "vault.active",
                "status": "ok",
                "message": "an active vault can be resolved",
            }));
        } else {
            checks.push(json!({
                "id": "vault.active",
                "status": "warning",
                "message": "no active vault can be resolved",
                "fix": "run vault:init path=<path> or pass --vault <name>"
            }));
            recommendations.push(
                "Selecciona un vault con `--vault <name>` o inicializa uno con `vault:init`."
                    .to_string(),
            );
        }

        if termux && !command_exists("pkg") {
            checks.push(json!({
                "id": "termux.pkg",
                "status": "error",
                "message": "Termux was detected but pkg is unavailable",
                "fix": "repair the Termux package environment"
            }));
        }
        if termux && !command_exists("termux-open-url") {
            checks.push(json!({
                "id": "termux.api",
                "status": "warning",
                "message": "termux-open-url is unavailable",
                "fix": "pkg install termux-api and install the Termux:API app"
            }));
            recommendations.push(
                "Instala Termux:API para habilitar apertura de URLs y clipboard.".to_string(),
            );
        }

        let mut index_report = Value::Null;
        if (invocation.has_flag("deep") || invocation.has_flag("fix"))
            && let Some(vault) = active_context
        {
            let started = Instant::now();
            match vault.load_index() {
                Ok(index) => {
                    let elapsed_ms = started.elapsed().as_millis();
                    index_report = json!({
                        "status": "ok",
                        "duration_ms": elapsed_ms,
                        "files": index.files.len(),
                        "markdown_files": index.markdown.len(),
                        "resolved_link_sources": index.resolved_links.len(),
                        "unresolved_link_sources": index.unresolved_links.len(),
                    });
                    checks.push(json!({
                        "id": "vault.index",
                        "status": "ok",
                        "message": format!("vault index loaded in {elapsed_ms} ms"),
                    }));
                    if invocation.has_flag("fix") {
                        repairs.push("active_vault_index_verified");
                    }
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    index_report = json!({ "status": "error", "message": message });
                    checks.push(json!({
                        "id": "vault.index",
                        "status": "error",
                        "message": message,
                        "fix": "check unreadable files and storage permissions"
                    }));
                }
            }
        }

        let errors = checks
            .iter()
            .filter(|check| check["status"] == "error")
            .count();
        let warnings = checks
            .iter()
            .filter(|check| check["status"] == "warning")
            .count();
        let passed = checks.len() - errors - warnings;
        let status = if errors > 0 {
            "error"
        } else if warnings > 0 {
            "warning"
        } else {
            "ok"
        };

        let report = json!({
            "schema_version": 1,
            "ok": errors == 0,
            "status": status,
            "summary": { "passed": passed, "warnings": warnings, "errors": errors },
            "version": env!("CARGO_PKG_VERSION"),
            "termux": termux,
            "cwd": self.workspace.cwd.to_string_lossy(),
            "prefix": prefix,
            "config_home": config_home,
            "auto_update_enabled": !env::var("OBSIDIAN_CLI_AUTO_UPDATE")
                .map(|value| value == "0" || value.eq_ignore_ascii_case("false"))
                .unwrap_or(false),
            "runtime": {
                "base_dir": runtime.base_dir.to_string_lossy(),
                "cache_dir": runtime.cache_dir.to_string_lossy(),
                "state_file": runtime.state_file.to_string_lossy(),
                "history_file": runtime.history_file.to_string_lossy(),
                "base_writable": base_writable,
                "cache_writable": cache_writable,
            },
            "active_vault": active_vault,
            "vaults": vaults,
            "storage": storage,
            "termux_tools": termux_tools,
            "build_tools": cargo_tools,
            "index": index_report,
            "checks": checks,
            "recommendations": recommendations,
            "repairs": repairs,
        });

        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string_pretty(&report)?);
        }

        let mut lines = Vec::new();
        lines.push(format!("status: {status}"));
        lines.push(format!(
            "checks: {passed} passed, {warnings} warnings, {errors} errors"
        ));
        lines.push(format!("version: {}", env!("CARGO_PKG_VERSION")));
        lines.push(format!("termux: {}", report["termux"]));
        lines.push(format!("cwd: {}", self.workspace.cwd.to_string_lossy()));
        lines.push(format!(
            "prefix: {}",
            report["prefix"].as_str().unwrap_or("")
        ));
        lines.push(format!(
            "runtime_base: {}",
            runtime.base_dir.to_string_lossy()
        ));
        lines.push(format!(
            "base_writable: {}",
            report["runtime"]["base_writable"]
        ));
        lines.push(format!(
            "cache_writable: {}",
            report["runtime"]["cache_writable"]
        ));
        lines.push(format!(
            "auto_update_enabled: {}",
            report["auto_update_enabled"]
        ));
        lines.push(format!(
            "vaults: {}",
            report["vaults"]
                .as_array()
                .map(Vec::len)
                .unwrap_or_default()
        ));
        if let Some(active) = report["active_vault"].as_object() {
            lines.push(format!(
                "active_vault: {}",
                active
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            ));
        }
        lines.push("termux_tools:".to_string());
        for tool in report["termux_tools"].as_array().into_iter().flatten() {
            lines.push(format!(
                "  {}\t{}",
                tool["program"].as_str().unwrap_or_default(),
                tool["available"]
            ));
        }
        lines.push("build_tools:".to_string());
        for tool in report["build_tools"].as_array().into_iter().flatten() {
            lines.push(format!(
                "  {}\t{}",
                tool["program"].as_str().unwrap_or_default(),
                tool["available"]
            ));
        }
        lines.push("diagnostics:".to_string());
        for check in report["checks"].as_array().into_iter().flatten() {
            lines.push(format!(
                "  [{}] {}: {}",
                check["status"].as_str().unwrap_or("unknown"),
                check["id"].as_str().unwrap_or("unknown"),
                check["message"].as_str().unwrap_or_default()
            ));
        }
        for recommendation in report["recommendations"].as_array().into_iter().flatten() {
            lines.push(format!(
                "recommendation: {}",
                recommendation.as_str().unwrap_or_default()
            ));
        }
        Ok(lines.join("\n"))
    }

    fn cmd_batch(&mut self, invocation: &Invocation) -> Result<String> {
        let input = if let Some(input) = invocation.param("input") {
            input.to_string()
        } else if let Some(path) = invocation.param("file") {
            fs::read_to_string(path)?
        } else {
            let mut stdin = io::stdin();
            if stdin.is_terminal() {
                bail!("batch necesita `file=<ruta>`, `input=<comandos>` o stdin redirigido");
            }
            let mut input = String::new();
            stdin.read_to_string(&mut input)?;
            input
        };

        let mut results = Vec::new();
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        for (offset, source) in input.lines().enumerate() {
            let source = source.trim();
            if source.is_empty() || source.starts_with('#') {
                continue;
            }
            let line = offset + 1;
            let result = match parse_line(source) {
                Ok(Request::Invocation(mut nested)) if nested.command != "batch" => {
                    if nested.global.vault.is_none() {
                        nested.global.vault = invocation.global.vault.clone();
                    }
                    nested.global.no_update = true;
                    nested.global.agent = false;
                    if invocation.global.agent {
                        nested
                            .params
                            .entry("format".to_string())
                            .or_insert_with(|| "json".to_string());
                    }
                    let command = nested.command.clone();
                    match self.execute(nested) {
                        Ok(output) => {
                            succeeded += 1;
                            let data = serde_json::from_str::<Value>(&output)
                                .unwrap_or(Value::String(output));
                            json!({
                                "ok": true,
                                "line": line,
                                "command": command,
                                "data": data,
                            })
                        }
                        Err(error) => {
                            failed += 1;
                            json!({
                                "ok": false,
                                "line": line,
                                "command": command,
                                "error": { "message": format!("{error:#}") },
                            })
                        }
                    }
                }
                Ok(Request::Invocation(_)) => {
                    failed += 1;
                    json!({
                        "ok": false,
                        "line": line,
                        "error": { "message": "batch anidado no está permitido" },
                    })
                }
                Ok(Request::Interactive) => continue,
                Err(error) => {
                    failed += 1;
                    json!({
                        "ok": false,
                        "line": line,
                        "error": { "message": format!("{error:#}") },
                    })
                }
            };
            let failed_result = result["ok"] == Value::Bool(false);
            results.push(result);
            if failed_result && invocation.has_flag("fail-fast") {
                break;
            }
        }

        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string(&json!({
                "ok": failed == 0,
                "summary": {
                    "total": results.len(),
                    "succeeded": succeeded,
                    "failed": failed,
                },
                "results": results,
            }))?);
        }
        results
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|lines| lines.join("\n"))
            .map_err(Into::into)
    }

    fn cmd_vault(&self, invocation: &Invocation) -> Result<String> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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

    fn cmd_vault_init(&mut self, invocation: &Invocation) -> Result<String> {
        let selector = invocation
            .param("path")
            .or(invocation.param("name"))
            .or(invocation.global.vault.as_deref());
        let vault = self.workspace.open_or_init_vault(selector)?;
        self.workspace.set_active_vault(&vault);
        self.workspace.state.active_file = None;
        Ok(vault.root.to_string_lossy().to_string())
    }

    fn cmd_vaults(&mut self, invocation: &Invocation) -> Result<String> {
        if invocation.has_flag("refresh") {
            self.workspace.refresh_known_vaults()?;
        }

        let mut vaults = self.workspace.known_vaults.clone();
        if vaults.is_empty()
            && let Ok(current) = self.workspace.resolve_vault(None)
        {
            vaults.push(current);
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
        let files = vault.list_files(invocation.param("folder"), invocation.param("ext"))?;
        if invocation.has_flag("total") {
            return Ok(files.len().to_string());
        }
        render_files(&files, invocation.param("format"))
    }

    fn cmd_folder(&self, invocation: &Invocation) -> Result<String> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let vault = self
            .workspace
            .open_or_init_vault(invocation.global.vault.as_deref())?;
        let rel = create_target_path(invocation)?;
        let mut content = invocation.param("content").unwrap_or_default().to_string();
        if let Some(template) = invocation.param("template") {
            let template_content =
                self.load_template_text(&vault, template, invocation.param("name"))?;
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
        let name = required_param_any(invocation, &["name", "to"])?;
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
            .map(|map| {
                map.iter()
                    .map(|(path, count)| (path.clone(), *count))
                    .collect::<Vec<_>>()
            })
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
            .map(|map| {
                map.iter()
                    .map(|(path, count)| (path.clone(), *count))
                    .collect::<Vec<_>>()
            })
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
                .map(|heading| {
                    format!(
                        "{}{}",
                        "  ".repeat(heading.level.saturating_sub(1)),
                        heading.text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }

    fn cmd_daily_path(&self, invocation: &Invocation) -> Result<String> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
        vault.ensure_daily_note_path()
    }

    fn cmd_daily(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, path) = self.ensure_daily_exists(invocation)?;
        self.workspace.set_active_file(&vault, &path);
        Ok(path)
    }

    fn cmd_daily_read(&self, invocation: &Invocation) -> Result<String> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let index = vault.load_index()?;

        if !with_context && !case_sensitive {
            for (rel_path, file) in &index.files {
                if hits.len() >= limit {
                    break;
                }
                if !file.is_markdown && !is_text_candidate(rel_path) {
                    continue;
                }
                if let Some(scope) = scope.as_deref()
                    && !rel_path.starts_with(scope)
                {
                    continue;
                }

                let matched = if file.is_markdown {
                    index
                        .markdown
                        .get(rel_path)
                        .map(|meta| meta.search_blob.contains(&search_query))
                        .unwrap_or(false)
                } else if let Ok(text) = vault.read_text(rel_path) {
                    text.to_ascii_lowercase().contains(&search_query)
                } else {
                    false
                };

                if matched {
                    hits.push(json!({ "path": rel_path }));
                }
            }

            if invocation.has_flag("total") {
                return Ok(hits.len().to_string());
            }
            if invocation.param("format") == Some("json") {
                return Ok(serde_json::to_string_pretty(&hits)?);
            }
            return Ok(hits
                .iter()
                .filter_map(|value| value.get("path").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"));
        }

        for file in index.files.values() {
            if hits.len() >= limit {
                break;
            }
            if !file.is_markdown && !is_text_candidate(&file.rel_path) {
                continue;
            }
            if let Some(scope) = scope.as_deref()
                && !file.rel_path.starts_with(scope)
            {
                continue;
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
            render_json_rows(
                hits,
                invocation.param("format"),
                Some(&["path", "line", "text"]),
            )
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
        if invocation.has_flag("active")
            || invocation.param("file").is_some()
            || invocation.param("path").is_some()
        {
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
            warn_frontmatter_error(&rel, meta);
            if invocation.has_flag("total") {
                return Ok(meta.tags.len().to_string());
            }
            return render_strings(&meta.tags, invocation.param("format"));
        }

        let mut counts = HashMap::<String, usize>::new();
        for (path, meta) in &index.markdown {
            warn_frontmatter_error(path, meta);
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
            if let Some(selected) = selected_path.as_deref()
                && path != selected
            {
                continue;
            }
            if let Some(daily) = current_daily.as_deref()
                && path != daily
            {
                continue;
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
            return render_json_rows(
                rows,
                invocation.param("format"),
                Some(&["ref", "status", "text"]),
            );
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

        Ok(format!(
            "{path}:{} [{}] {}",
            task.line, task.status, task.text
        ))
    }

    fn cmd_aliases(&self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        if invocation.has_flag("active")
            || invocation.param("file").is_some()
            || invocation.param("path").is_some()
        {
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
            warn_frontmatter_error(&rel, meta);
            return render_json_rows(
                alias_rows(meta),
                invocation.param("format"),
                Some(&["value"]),
            );
        }

        let mut rows = Vec::new();
        for (path, meta) in &index.markdown {
            warn_frontmatter_error(path, meta);
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
        if invocation.has_flag("active")
            || invocation.param("file").is_some()
            || invocation.param("path").is_some()
        {
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
            warn_frontmatter_error(&rel, meta);
            if invocation.param("format") == Some("yaml") {
                return Ok(serde_yaml::to_string(&meta.properties)?);
            }
            return render_json_rows(
                property_rows(meta),
                invocation.param("format"),
                Some(&["name", "value"]),
            );
        }

        let name_filter = param_any(invocation, &["name", "key"]);
        let mut counts = HashMap::<String, usize>::new();
        for (path, meta) in &index.markdown {
            warn_frontmatter_error(path, meta);
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
        let name = required_param_any(invocation, &["name", "key"])?;
        let value = required_param(invocation, "value")?;
        let abs = vault.rel_to_abs(&rel)?;
        let original = fs::read_to_string(&abs)?;
        let (_, body) = split_frontmatter(&original);
        let mut properties = read_frontmatter(&abs)?;
        properties.insert(
            name.to_string(),
            typed_value(invocation.param("type"), value)?,
        );
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
        let name = required_param_any(invocation, &["name", "key"])?;
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
        let name = required_param_any(invocation, &["name", "key"])?;
        let meta = index
            .markdown
            .get(&rel)
            .ok_or_else(|| anyhow!("solo aplica a archivos Markdown"))?;
        warn_frontmatter_error(&rel, meta);
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
        let templates = self.list_templates(&vault)?;
        if invocation.has_flag("total") {
            return Ok(templates.len().to_string());
        }
        render_strings(&templates, invocation.param("format"))
    }

    fn cmd_template_read(&self, invocation: &Invocation) -> Result<String> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
        let name = required_param(invocation, "name")?;
        let mut template = self.load_template_text(&vault, name, invocation.param("title"))?;
        if invocation.has_flag("resolve") {
            template = apply_template_tokens(&template, invocation.param("title"));
        }
        Ok(template)
    }

    fn cmd_template_insert(&mut self, invocation: &Invocation) -> Result<String> {
        let (vault, index, active_file) = self.open_local(invocation)?;
        let target = vault.resolve_target(
            &index,
            invocation.param("file"),
            invocation.param("path"),
            active_file.as_deref(),
        )?;
        let _ = index
            .markdown
            .get(&target)
            .ok_or_else(|| anyhow!("el archivo objetivo debe ser Markdown"))?;
        let name = required_param(invocation, "name")?;
        let template = self.load_template_text(&vault, name, invocation.param("title"))?;
        vault.append_text(&target, &template, false)?;
        Ok(target)
    }

    fn cmd_bases(&self, invocation: &Invocation) -> Result<String> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
        let bases = vault.list_bases()?;
        render_strings(&bases, invocation.param("format"))
    }

    fn cmd_random(&mut self, invocation: &Invocation, read: bool) -> Result<String> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        render_json_rows(
            rows,
            invocation.param("format"),
            Some(&["path", "opened_at"]),
        )
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        bail!(
            "`bookmark` todavía no escribe `bookmarks.json`; el esquema varía entre versiones de Obsidian"
        )
    }

    fn cmd_plugins(&self, invocation: &Invocation, enabled_only: bool) -> Result<String> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
        let themes = self.collect_themes(&vault)?;
        if invocation.param("format") == Some("json") {
            return Ok(serde_json::to_string_pretty(&themes)?);
        }
        if invocation.has_flag("versions") {
            return Ok(themes
                .iter()
                .map(|theme| {
                    format!(
                        "{}\t{}",
                        theme.name,
                        theme.version.clone().unwrap_or_default()
                    )
                })
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
                (
                    "active",
                    (appearance.css_theme.as_deref() == Some(name)).to_string(),
                ),
            ]));
        }
        Ok(appearance.css_theme.unwrap_or_default())
    }

    fn cmd_theme_set(&self, invocation: &Invocation) -> Result<String> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
        let name = required_param(invocation, "name")?;
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name == ".."
            || name == "."
        {
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
        let name = required_param(invocation, "name")?;
        let mut appearance = read_appearance(&vault)?;
        if enabled {
            if !appearance
                .enabled_css_snippets
                .iter()
                .any(|item| item == name)
            {
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

    fn open_local(
        &self,
        invocation: &Invocation,
    ) -> Result<(VaultContext, VaultIndex, Option<String>)> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
        let index = vault.load_index()?;
        let active_file = self.workspace.active_file_for(&vault);
        Ok((vault, index, active_file))
    }

    fn ensure_daily_exists(&self, invocation: &Invocation) -> Result<(VaultContext, String)> {
        let vault = self
            .workspace
            .open_vault(invocation.global.vault.as_deref())?;
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
        let folder = vault.templates_folder()?.ok_or_else(|| {
            anyhow!("no hay carpeta de templates configurada en `.obsidian/templates.json`")
        })?;
        let mut templates = vault
            .list_files(Some(&folder), Some("md"))?
            .into_iter()
            .map(|file| file.rel_path)
            .collect::<Vec<_>>();
        templates.sort();
        Ok(templates)
    }

    fn load_template_text(
        &self,
        vault: &VaultContext,
        name: &str,
        title: Option<&str>,
    ) -> Result<String> {
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
        let community_enabled =
            read_json_string_list(&vault.obsidian_dir.join("community-plugins.json"))?;
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

fn command_is_available(name: &str) -> bool {
    find(name).is_some_and(|spec| {
        spec.support != SupportLevel::BridgeOnly
            && !matches!(name, "base:views" | "base:query" | "bookmark")
    })
}
