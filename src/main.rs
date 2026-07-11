//! forster-todo: a terminal task manager implementing Mark Forster's
//! Final Version Perfected (FVP) methodology.

mod app;
mod fvp;
mod storage;
mod task;
mod ui;

use std::path::PathBuf;

use anyhow::{Context, Result};
use crossterm::event::{self, Event};
use directories::ProjectDirs;
use ratatui::DefaultTerminal;

use app::App;

fn main() -> Result<()> {
    let path = resolve_path()?;
    let tasks = storage::load(&path)?;
    let mut app = App::new(path, tasks);

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

/// The main event loop: draw, wait for a key, dispatch, repeat.
fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if let Event::Key(key) = event::read()? {
            app.on_key(key)?;
        }
    }
    Ok(())
}

/// Determine the task file: `--file <path>` / `-f <path>` if given, otherwise
/// the platform data directory (e.g. ~/Library/Application Support/forster-todo).
fn resolve_path() -> Result<PathBuf> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => default_path(),
        Some("--file" | "-f") => {
            let value = args.next().context("--file requires a path argument")?;
            Ok(PathBuf::from(value))
        }
        Some("-h" | "--help") => {
            println!(
                "forster-todo — Mark Forster's FVP in your terminal\n\n\
                 USAGE:\n    forster-todo [--file <path>]\n\n\
                 Without --file, tasks are stored in the platform data directory."
            );
            std::process::exit(0);
        }
        Some(other) => anyhow::bail!("unknown argument: {other} (try --help)"),
    }
}

/// The default task file in the platform data directory.
fn default_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "forster-todo")
        .context("could not determine a data directory for this platform")?;
    Ok(dirs.data_dir().join("tasks.txt"))
}
