mod app;
mod parser;
mod registry;
mod tui;
mod updater;
mod vault;

use std::io::{self, ErrorKind, IsTerminal, Write};
use std::process::{Command, ExitCode};

use anyhow::Result;

use crate::app::App;
use crate::parser::{Request, parse};

fn main() -> ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            if std::env::args().any(|arg| arg == "--agent") {
                let message = format!("{error:#}");
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "ok": false,
                        "error": { "message": message }
                    })
                );
            } else {
                eprintln!("{error:#}");
            }
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut auto_updated = false;
    if should_check_auto_update(&args) {
        match updater::check_and_auto_update() {
            Ok(updater::AutoUpdateOutcome::Updated) => auto_updated = true,
            Ok(updater::AutoUpdateOutcome::NoChange) => {}
            Err(error) => {
                eprintln!("Auto-update omitido: {error:#}");
            }
        }
    }

    if auto_updated {
        relaunch_after_update(&args)?;
        return Ok(ExitCode::SUCCESS);
    }

    let mut app = App::load()?;

    match parse(&args)? {
        Request::Interactive => tui::run(&mut app)?,
        Request::Invocation(invocation) => {
            let agent = invocation.global.agent;
            let command = invocation.command.clone();
            let output = app.execute(invocation)?;
            if agent {
                let data = serde_json::from_str::<serde_json::Value>(&output)
                    .unwrap_or(serde_json::Value::String(output));
                let ok = data
                    .get("ok")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                write_stdout_line(
                    &serde_json::json!({
                        "ok": ok,
                        "command": command,
                        "data": data,
                    })
                    .to_string(),
                )?;
                if !ok {
                    return Ok(ExitCode::from(2));
                }
            } else if !output.is_empty() {
                write_stdout_line(&output)?;
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn write_stdout_line(output: &str) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match writeln!(stdout, "{output}") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn relaunch_after_update(args: &[String]) -> Result<()> {
    let invoked_exe = std::env::args()
        .next()
        .unwrap_or_else(|| "obsidian".to_string());
    let status = Command::new(invoked_exe).args(args).status()?;
    if status.success() {
        return Ok(());
    }
    std::process::exit(status.code().unwrap_or(1));
}

fn should_check_auto_update(args: &[String]) -> bool {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--no-update" | "--agent"))
    {
        return false;
    }

    if args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "help" | "--help" | "-h" | "version" | "--version" | "-V"
        )
    }) {
        return false;
    }

    std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::should_check_auto_update;

    #[test]
    fn skips_auto_update_for_help_version_and_explicit_opt_out() {
        assert!(!should_check_auto_update(&["--help".to_string()]));
        assert!(!should_check_auto_update(&["version".to_string()]));
        assert!(!should_check_auto_update(&[
            "--no-update".to_string(),
            "files".to_string()
        ]));
        assert!(!should_check_auto_update(&[
            "--agent".to_string(),
            "files".to_string()
        ]));
    }
}
