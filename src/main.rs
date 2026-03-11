mod app;
mod parser;
mod registry;
mod tui;
mod updater;
mod vault;

use std::process::ExitCode;

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
    if let Err(error) = updater::check_and_auto_update() {
        eprintln!("Auto-update omitido: {error:#}");
    }

    let args = std::env::args().skip(1).collect::<Vec<_>>();
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
