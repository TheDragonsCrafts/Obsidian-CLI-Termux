mod app;
mod parser;
mod registry;
mod tui;
mod updater;
mod vault;

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
    match updater::check_and_auto_update() {
        Ok(updater::AutoUpdateOutcome::Updated) => auto_updated = true,
        Ok(updater::AutoUpdateOutcome::NoChange) => {}
        Err(error) => {
            eprintln!("Auto-update omitido: {error:#}");
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
