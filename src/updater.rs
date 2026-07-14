use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use fs2::FileExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DEFAULT_GITHUB_REPO: &str = "TheDragonsCrafts/Obsidian-CLI-Termux";
const CHECK_INTERVAL_SECS: u64 = 60 * 60 * 12;
const FETCH_TIMEOUT_SECS: u64 = 12;
const FETCH_RETRIES: u8 = 3;
const FETCH_BACKOFF_BASE_MS: u64 = 250;
const BREAKER_THRESHOLD: u8 = 2;
const MAX_BINARY_BYTES: usize = 64 * 1024 * 1024;

static UPDATE_CIRCUIT_OPEN: AtomicBool = AtomicBool::new(false);
static UPDATE_FAIL_STREAK: AtomicU8 = AtomicU8::new(0);

#[derive(Debug, Clone, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UpdateState {
    last_check_unix: u64,
    last_seen_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoUpdateOutcome {
    NoChange,
    Updated,
}

pub fn check_and_auto_update() -> Result<AutoUpdateOutcome> {
    if env_flag("OBSIDIAN_CLI_AUTO_UPDATE", false) {
        return Ok(AutoUpdateOutcome::NoChange);
    }
    if !should_check_now()? || UPDATE_CIRCUIT_OPEN.load(Ordering::Relaxed) {
        return Ok(AutoUpdateOutcome::NoChange);
    }

    let repo = configured_repo();
    let Some(release) = fetch_release_with_fallback(&repo, None)? else {
        write_state(None)?;
        eprintln!("No hay releases publicadas; update no compilara codigo fuente automaticamente.");
        return Ok(AutoUpdateOutcome::NoChange);
    };
    let version = release_version(&release)?;

    if !is_newer(&version, env!("CARGO_PKG_VERSION"))? {
        write_state(Some(version))?;
        return Ok(AutoUpdateOutcome::NoChange);
    }
    if !auto_apply_enabled() {
        write_state(Some(version.clone()))?;
        eprintln!(
            "Nueva version detectada ({version}). Modo seguro activo: ejecuta `obsidian update` para instalarla."
        );
        return Ok(AutoUpdateOutcome::NoChange);
    }

    eprintln!("Nueva version detectada ({version}). Descargando binario precompilado...");
    install_release_binary(&release)?;
    write_state(Some(version))?;
    eprintln!("Auto-update completado y verificado. Reiniciando...");
    Ok(AutoUpdateOutcome::Updated)
}

pub fn manual_update(force: bool, language: &str) -> Result<String> {
    let configured = configured_repo();
    let pin = pinned_tag();
    let release = fetch_release_with_fallback(&configured, pin.as_deref())?.ok_or_else(|| {
        anyhow!("no hay una release con binarios precompilados disponible en GitHub")
    })?;
    let version = release_version(&release)?;

    if !force && !is_newer(&version, env!("CARGO_PKG_VERSION"))? {
        write_state(Some(version.clone()))?;
        return Ok(if language == "en" {
            format!("CLI is already up to date ({version}).")
        } else {
            format!("La CLI ya esta actualizada ({version}).")
        });
    }

    let asset = install_release_binary(&release)?;
    write_state(Some(version.clone()))?;
    let progress = render_update_progress(language);
    Ok(if language == "en" {
        format!(
            "{progress}\nManual update to {version} completed from {asset}. Restart the command to use it."
        )
    } else {
        format!(
            "{progress}\nActualizacion manual a {version} completada desde {asset}. Reinicia el comando para usarla."
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

fn fetch_release_with_fallback(repo: &str, tag: Option<&str>) -> Result<Option<GitHubRelease>> {
    match fetch_release(repo, tag) {
        Ok(release) => Ok(release),
        Err(_error) if repo != DEFAULT_GITHUB_REPO => {
            eprintln!("No se pudo consultar {repo}; reintentando con {DEFAULT_GITHUB_REPO}.");
            fetch_release(DEFAULT_GITHUB_REPO, tag)
        }
        Err(error) => Err(error),
    }
}

fn fetch_release(repo: &str, tag: Option<&str>) -> Result<Option<GitHubRelease>> {
    let endpoint = match tag {
        Some(tag) => format!("https://api.github.com/repos/{repo}/releases/tags/{tag}"),
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    let mut last_error = None;
    for attempt in 1..=FETCH_RETRIES {
        match github_get(&endpoint) {
            Ok(Some(bytes)) => {
                reset_circuit_breaker();
                let release = serde_json::from_slice(&bytes)
                    .context("no se pudo parsear la release de GitHub")?;
                return Ok(Some(release));
            }
            Ok(None) => {
                reset_circuit_breaker();
                return Ok(None);
            }
            Err(error) => last_error = Some(error),
        }
        if attempt < FETCH_RETRIES {
            thread::sleep(Duration::from_millis(
                FETCH_BACKOFF_BASE_MS * u64::from(attempt),
            ));
        }
    }
    register_fetch_failure();
    Err(last_error.unwrap_or_else(|| anyhow!("error desconocido consultando GitHub")))
}

fn github_get(url: &str) -> Result<Option<Vec<u8>>> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(FETCH_TIMEOUT_SECS)))
        .build();
    let agent: ureq::Agent = config.into();
    match agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "obsidian-termux-cli-updater")
        .call()
    {
        Ok(response) => response
            .into_body()
            .with_config()
            .limit(MAX_BINARY_BYTES as u64)
            .read_to_vec()
            .map(Some)
            .with_context(|| format!("no se pudo descargar {url}")),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(error) => Err(anyhow!(error).context(format!("no se pudo consultar GitHub ({url})"))),
    }
}

fn install_release_binary(release: &GitHubRelease) -> Result<String> {
    let _update_lock = acquire_update_lock()?;
    let asset_name = platform_asset_name()?;
    let checksum_name = format!("{asset_name}.sha256");
    let binary_asset = find_asset(release, asset_name)?;
    let checksum_asset = find_asset(release, &checksum_name)?;

    let binary = github_get(&binary_asset.browser_download_url)?
        .ok_or_else(|| anyhow!("GitHub no devolvio el asset {asset_name}"))?;
    let checksum_bytes = github_get(&checksum_asset.browser_download_url)?
        .ok_or_else(|| anyhow!("GitHub no devolvio el checksum {checksum_name}"))?;
    verify_sha256(&binary, &checksum_bytes, asset_name)?;
    replace_current_executable(&binary, &release_version(release)?)?;
    Ok(asset_name.to_string())
}

fn find_asset<'a>(release: &'a GitHubRelease, name: &str) -> Result<&'a ReleaseAsset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| {
            anyhow!(
                "la release {} no contiene el asset {name}",
                release.tag_name
            )
        })
}

fn verify_sha256(binary: &[u8], checksum_file: &[u8], asset_name: &str) -> Result<()> {
    let checksum_text = std::str::from_utf8(checksum_file).context("checksum no es UTF-8")?;
    let expected = checksum_text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("archivo de checksum vacio"))?;
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("checksum SHA-256 invalido para {asset_name}");
    }
    let actual = crate::encoding::lowercase_hex(Sha256::digest(binary));
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("checksum SHA-256 no coincide para {asset_name}");
    }
    Ok(())
}

fn replace_current_executable(binary: &[u8], expected_version: &str) -> Result<()> {
    let executable =
        std::env::current_exe().context("no se pudo localizar el ejecutable actual")?;
    let parent = executable
        .parent()
        .ok_or_else(|| anyhow!("ruta de ejecutable invalida: {}", executable.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("no se puede escribir en {}", parent.display()))?;
    temporary.write_all(binary)?;
    temporary.as_file().sync_all()?;
    set_executable_permissions(temporary.path())?;
    verify_downloaded_executable(temporary.path(), expected_version)?;
    temporary
        .persist(&executable)
        .map_err(|error| anyhow!(error.error))
        .with_context(|| format!("no se pudo reemplazar {}", executable.display()))?;
    Ok(())
}

fn verify_downloaded_executable(path: &Path, expected_version: &str) -> Result<()> {
    let output = Command::new(path)
        .arg("--no-update")
        .arg("version")
        .output()
        .context("el binario descargado no se pudo ejecutar")?;
    if !output.status.success() {
        bail!("el binario descargado fallo su comprobacion de arranque");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = format!("obsidian-termux-cli {expected_version}");
    if !stdout.lines().any(|line| line.trim() == expected) {
        bail!("el asset no reporta la version esperada {expected_version}; instalacion cancelada");
    }
    Ok(())
}

fn acquire_update_lock() -> Result<fs::File> {
    let path = state_path()?.with_file_name("update.lock");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("no se pudo abrir el lock de update: {}", path.display()))?;
    FileExt::try_lock_exclusive(&file).context("ya hay otra actualizacion en curso")?;
    Ok(file)
}

#[cfg(unix)]
fn set_executable_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable_permissions(_path: &Path) -> Result<()> {
    bail!("el auto-update precompilado solo esta soportado actualmente en Termux/Android")
}

fn platform_asset_name() -> Result<&'static str> {
    #[cfg(all(target_os = "android", target_arch = "aarch64"))]
    return Ok("obsidian-aarch64-linux-android");
    #[cfg(all(target_os = "android", target_arch = "x86_64"))]
    return Ok("obsidian-x86_64-linux-android");
    #[cfg(not(any(
        all(target_os = "android", target_arch = "aarch64"),
        all(target_os = "android", target_arch = "x86_64")
    )))]
    {
        bail!(
            "no hay binario precompilado para {}-{}; usa Termux AArch64 o x86_64",
            std::env::consts::ARCH,
            std::env::consts::OS
        )
    }
}

fn release_version(release: &GitHubRelease) -> Result<String> {
    let value = release.tag_name.trim_start_matches('v').trim();
    Version::parse(value).with_context(|| format!("version invalida en GitHub: {value}"))?;
    Ok(value.to_string())
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

fn is_newer(candidate: &str, current: &str) -> Result<bool> {
    Ok(Version::parse(candidate)? > Version::parse(current)?)
}

fn auto_apply_enabled() -> bool {
    std::env::var("OBSIDIAN_CLI_AUTO_UPDATE_APPLY")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn env_flag(name: &str, truthy_disables: bool) -> bool {
    std::env::var(name)
        .map(|value| {
            if truthy_disables {
                value == "1" || value.eq_ignore_ascii_case("true")
            } else {
                value == "0" || value.eq_ignore_ascii_case("false")
            }
        })
        .unwrap_or(false)
}

fn pinned_tag() -> Option<String> {
    std::env::var("OBSIDIAN_CLI_UPDATE_PIN")
        .ok()
        .map(|value| value.trim().trim_start_matches("tag:").to_string())
        .filter(|value| !value.is_empty())
}

fn reset_circuit_breaker() {
    UPDATE_FAIL_STREAK.store(0, Ordering::Relaxed);
    UPDATE_CIRCUIT_OPEN.store(false, Ordering::Relaxed);
}

fn register_fetch_failure() {
    let streak = UPDATE_FAIL_STREAK
        .fetch_add(1, Ordering::Relaxed)
        .saturating_add(1);
    if streak >= BREAKER_THRESHOLD {
        UPDATE_CIRCUIT_OPEN.store(true, Ordering::Relaxed);
    }
}

fn render_update_progress(language: &str) -> String {
    let labels = if language == "en" {
        [
            "Release checked",
            "Binary downloaded",
            "SHA-256 verified",
            "Binary installed",
        ]
    } else {
        [
            "Release comprobada",
            "Binario descargado",
            "SHA-256 verificado",
            "Binario instalado",
        ]
    };
    labels
        .iter()
        .enumerate()
        .map(|(index, label)| format!("[{}] {label}", "#".repeat((index + 1) * 6)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_state() -> Result<UpdateState> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(UpdateState::default());
    }
    serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("estado de updater invalido: {}", path.display()))
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
    atomic_write_bytes(&path, &serde_json::to_vec_pretty(&state)?)
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow!("ruta invalida"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| anyhow!(error.error))?;
    Ok(())
}

fn state_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or_else(|| anyhow!("no se pudo resolver XDG_CONFIG_HOME"))?
        .join("obsidian-termux-cli")
        .join("auto-update-state.json"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_GITHUB_REPO, GitHubRelease, configured_repo, release_version, verify_sha256,
    };

    #[test]
    fn uses_default_repo_when_env_is_missing_or_blank() {
        unsafe { std::env::remove_var("OBSIDIAN_CLI_GITHUB_REPO") };
        assert_eq!(configured_repo(), DEFAULT_GITHUB_REPO);
        unsafe { std::env::set_var("OBSIDIAN_CLI_GITHUB_REPO", "   ") };
        assert_eq!(configured_repo(), DEFAULT_GITHUB_REPO);
    }

    #[test]
    fn validates_release_version() {
        let release = GitHubRelease {
            tag_name: "v1.2.3".into(),
            assets: vec![],
        };
        assert_eq!(release_version(&release).unwrap(), "1.2.3");
    }

    #[test]
    fn verifies_sha256_before_installing() {
        let checksum =
            b"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  obsidian\n";
        verify_sha256(b"hello", checksum, "obsidian").unwrap();
        assert!(verify_sha256(b"tampered", checksum, "obsidian").is_err());
    }
}
