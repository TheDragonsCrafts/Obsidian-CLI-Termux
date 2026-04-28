mod app;
mod parser;
mod registry;
mod tui;
mod updater;
mod vault;

use std::io::IsTerminal;
use std::process::{Command, ExitCode};

use anyhow::Result;

use crate::app::App;
use crate::parser::{Request, parse};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
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
        return relaunch_after_update(&args);
    }

    let mut app = App::load()?;

    match parse(&args)? {
        Request::Interactive => tui::run(&mut app)?,
        Request::Invocation(invocation) => {
            let output = app.execute(invocation)?;
            if !output.is_empty() {
                println!("{output}");
            }
        }
    }

    Ok(())
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
    if args.iter().any(|arg| arg == "--no-update") {
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
    }
}
