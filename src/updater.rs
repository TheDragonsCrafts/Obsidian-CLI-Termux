use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use semver::Version;
use serde::{Deserialize, Serialize};

const DEFAULT_GITHUB_REPO: &str = "TheDragonsCrafts/Obsidian-CLI-Termux";
const CHECK_INTERVAL_SECS: u64 = 60 * 60 * 12;

#[derive(Debug, Deserialize)]
struct LatestRelease {
    tag_name: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UpdateState {
    last_check_unix: u64,
    last_seen_version: Option<String>,
}

pub fn check_and_auto_update() -> Result<()> {
    if std::env::var("OBSIDIAN_CLI_AUTO_UPDATE")
        .map(|value| value == "0" || value.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
    {
        return Ok(());
    }

    if !should_check_now()? {
        return Ok(());
    }

    let configured_repo = configured_repo();
    let (repo, latest) = match fetch_latest_version(&configured_repo) {
        Ok(latest) => (configured_repo.clone(), latest),
        Err(_error) if configured_repo != DEFAULT_GITHUB_REPO => {
            eprintln!(
                "No se pudo consultar releases en {configured_repo}. Reintentando con repo por defecto {DEFAULT_GITHUB_REPO}..."
            );
            let latest = fetch_latest_version(DEFAULT_GITHUB_REPO).with_context(|| {
                format!(
                    "falló la consulta del repo configurado ({configured_repo}) y también del repo por defecto"
                )
            })?;
            (DEFAULT_GITHUB_REPO.to_string(), latest)
        }
        Err(error) => return Err(error),
    };
    let Some(latest) = latest else {
        write_state(None)?;
        eprintln!(
            "No hay releases publicadas en {repo}. Intentando auto-update desde la rama por defecto..."
        );
        run_self_update(&repo)?;
        eprintln!("Auto-update completado. Reinicia el comando para usar la versión nueva.");
        return Ok(());
    };

    write_state(Some(latest.clone()))?;

    if !is_newer(&latest, env!("CARGO_PKG_VERSION"))? {
        return Ok(());
    }

    eprintln!("Nueva versión detectada ({latest}). Intentando auto-update desde GitHub...");
    run_self_update(&repo)?;
    eprintln!("Auto-update completado. Reinicia el comando para usar la versión nueva.");

    Ok(())
}

pub fn manual_update(force: bool, language: &str) -> Result<String> {
    let configured_repo = configured_repo();
    let (repo, latest) = match fetch_latest_version(&configured_repo) {
        Ok(latest) => (configured_repo.clone(), latest),
        Err(_error) if configured_repo != DEFAULT_GITHUB_REPO => {
            eprintln!(
                "No se pudo consultar releases en {configured_repo}. Reintentando con repo por defecto {DEFAULT_GITHUB_REPO}..."
            );
            let latest = fetch_latest_version(DEFAULT_GITHUB_REPO).with_context(|| {
                format!(
                    "falló la consulta del repo configurado ({configured_repo}) y también del repo por defecto"
                )
            })?;
            (DEFAULT_GITHUB_REPO.to_string(), latest)
        }
        Err(error) => return Err(error),
    };

    if let Some(latest) = latest {
        write_state(Some(latest.clone()))?;
        if !force && !is_newer(&latest, env!("CARGO_PKG_VERSION"))? {
            return Ok(if language == "en" {
                format!("CLI is already up to date ({latest}).")
            } else {
                format!("La CLI ya está actualizada ({latest}).")
            });
        }
    } else {
        write_state(None)?;
    }

    let install_output = run_self_update(&repo)?;

    let progress = render_update_progress(language);
    let details = if install_output.trim().is_empty() {
        String::new()
    } else {
        let snippet = install_output
            .lines()
            .rev()
            .find(|line| {
                line.contains("Finished")
                    || line.contains("Installing")
                    || line.contains("Replacing")
            })
            .unwrap_or_default();
        if snippet.is_empty() {
            String::new()
        } else {
            format!("\n\n{snippet}")
        }
    };

    Ok(if language == "en" {
        format!(
            "{progress}\nManual update completed. Restart the command to use the new version.{details}"
        )
    } else {
        format!(
            "{progress}\nActualización manual completada. Reinicia el comando para usar la nueva versión.{details}"
        )
    })
}

fn configured_repo() -> String {
    std::env::var("OBSIDIAN_CLI_GITHUB_REPO")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_GITHUB_REPO.to_string())
}

fn should_check_now() -> Result<bool> {
    let state = read_state()?;
    let now = now_unix();
    if state.last_check_unix == 0
        || now.saturating_sub(state.last_check_unix) >= CHECK_INTERVAL_SECS
    {
        write_state(state.last_seen_version)?;
        return Ok(true);
    }
    Ok(false)
}

fn fetch_latest_version(repo: &str) -> Result<Option<String>> {
    let endpoint = format!("https://api.github.com/repos/{repo}/releases/latest");
    let response = match ureq::get(&endpoint)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "obsidian-termux-cli-auto-updater")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => {
            return Ok(None);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("no se pudo consultar GitHub ({endpoint})"));
        }
    };

    let payload: LatestRelease = response
        .into_body()
        .read_json()
        .context("no se pudo parsear la respuesta de GitHub")?;

    Ok(Some(
        payload.tag_name.trim_start_matches('v').trim().to_string(),
    ))
}

fn run_self_update(repo: &str) -> Result<String> {
    let install_url = format!("https://github.com/{repo}.git");
    let root = std::env::var("PREFIX").unwrap_or_else(|_| {
        dirs::home_dir()
            .map(|path| path.join(".cargo").to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string())
    });

    let output = Command::new("cargo")
        .arg("install")
        .arg("--git")
        .arg(install_url)
        .arg("--bin")
        .arg("obsidian")
        .arg("--locked")
        .arg("--force")
        .arg("--root")
        .arg(root)
        .output()
        .context("falló la ejecución de `cargo install` para el auto-update")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("`cargo install` terminó con error: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(format!("{stdout}\n{stderr}").trim().to_string())
}

fn render_update_progress(language: &str) -> String {
    let stages = [
        (
            12,
            if language == "en" {
                "Checking latest release"
            } else {
                "Consultando última release"
            },
        ),
        (
            38,
            if language == "en" {
                "Downloading source"
            } else {
                "Descargando fuente"
            },
        ),
        (
            72,
            if language == "en" {
                "Compiling"
            } else {
                "Compilando"
            },
        ),
        (
            100,
            if language == "en" {
                "Installing binary"
            } else {
                "Instalando binario"
            },
        ),
    ];

    stages
        .iter()
        .map(|(percent, label)| format!("{} {}", progress_bar(*percent), label))
        .collect::<Vec<_>>()
        .join("\n")
}

fn progress_bar(percent: u8) -> String {
    let total = 24;
    let filled = ((percent as usize) * total) / 100;
    let empty = total.saturating_sub(filled);
    format!(
        "[{}{}] {:>3}%",
        "█".repeat(filled),
        "░".repeat(empty),
        percent
    )
}

fn is_newer(candidate: &str, current: &str) -> Result<bool> {
    let candidate = Version::parse(candidate)
        .with_context(|| format!("versión inválida en GitHub: {candidate}"))?;
    let current =
        Version::parse(current).with_context(|| format!("versión local inválida: {current}"))?;
    Ok(candidate > current)
}

fn read_state() -> Result<UpdateState> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(UpdateState::default());
    }
    let bytes = fs::read(&path)
        .with_context(|| format!("no se pudo leer estado de updater: {}", path.display()))?;
    let state = serde_json::from_slice::<UpdateState>(&bytes)
        .with_context(|| format!("estado de updater inválido: {}", path.display()))?;
    Ok(state)
}

fn write_state(last_seen_version: Option<String>) -> Result<()> {
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let state = UpdateState {
        last_check_unix: now_unix(),
        last_seen_version,
    };
    fs::write(&path, serde_json::to_vec_pretty(&state)?)?;
    Ok(())
}

fn state_path() -> Result<PathBuf> {
    let config_base =
        dirs::config_dir().ok_or_else(|| anyhow!("no se pudo resolver XDG_CONFIG_HOME"))?;
    Ok(config_base
        .join("obsidian-termux-cli")
        .join("auto-update-state.json"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_GITHUB_REPO, configured_repo};

    #[test]
    fn uses_default_repo_when_env_is_missing() {
        unsafe { std::env::remove_var("OBSIDIAN_CLI_GITHUB_REPO") };
        assert_eq!(configured_repo(), DEFAULT_GITHUB_REPO);
    }

    #[test]
    fn uses_default_repo_when_env_is_blank() {
        unsafe { std::env::set_var("OBSIDIAN_CLI_GITHUB_REPO", "   ") };
        assert_eq!(configured_repo(), DEFAULT_GITHUB_REPO);
    }

    #[test]
    fn uses_env_repo_when_present() {
        unsafe { std::env::set_var("OBSIDIAN_CLI_GITHUB_REPO", "CustomOrg/CustomRepo") };
        assert_eq!(configured_repo(), "CustomOrg/CustomRepo");
    }
}
