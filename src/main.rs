//! forster-todo: a terminal task manager implementing Mark Forster's
//! Final Version Perfected (FVP) methodology.

mod app;
mod fvp;
mod session;
mod storage;
mod task;
mod ui;
mod web;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event};
use directories::ProjectDirs;
use ratatui::DefaultTerminal;

use app::App;
use session::Session;

const DEFAULT_WEB_PORT: u16 = 9000;

struct Args {
    file: Option<PathBuf>,
    web: Option<u16>,
    auth: Option<String>,
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let path = match args.file {
        Some(p) => p,
        None => default_path()?,
    };
    let session = Arc::new(Mutex::new(Session::load(path)?));

    // Start the web view (if requested) before touching the terminal, so a
    // port-in-use error prints normally.
    if let Some(port) = args.web {
        web::spawn(session.clone(), port, args.auth)?;
    }

    let mut app = App::new(session.clone());
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &session, &mut app);
    ratatui::restore();
    result
}

/// The main event loop. Uses a polling timeout rather than a blocking read so
/// changes made through the web view show up without a keypress.
fn run(terminal: &mut DefaultTerminal, session: &Arc<Mutex<Session>>, app: &mut App) -> Result<()> {
    while !app.should_quit {
        {
            let session = session.lock().expect("session lock poisoned");
            terminal.draw(|frame| ui::draw(frame, &session, app))?;
        }
        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
        {
            app.on_key(key)?;
        }
    }
    Ok(())
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        file: None,
        web: None,
        auth: None,
    };
    let mut it = std::env::args().skip(1).peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--file" | "-f" => {
                let value = it.next().context("--file requires a path argument")?;
                args.file = Some(PathBuf::from(value));
            }
            "--web" | "-w" => {
                // Optional port; defaults to DEFAULT_WEB_PORT.
                let port = match it.peek().and_then(|p| p.parse::<u16>().ok()) {
                    Some(p) => {
                        it.next();
                        p
                    }
                    None => DEFAULT_WEB_PORT,
                };
                args.web = Some(port);
            }
            "--auth" => {
                let value = it.next().context("--auth requires a user:pass argument")?;
                anyhow::ensure!(
                    value.contains(':'),
                    "--auth expects user:pass (missing ':')"
                );
                args.auth = Some(value);
            }
            "-h" | "--help" => {
                println!(
                    "forster-todo — Mark Forster's FVP in your terminal\n\n\
                     USAGE:\n    forster-todo [--file <path>] [--web [port]] [--auth user:pass]\n\n\
                     OPTIONS:\n    \
                     -f, --file <path>     Task file (default: platform data directory)\n    \
                     -w, --web [port]      Also serve a web view on 0.0.0.0:PORT, reachable from the LAN (default {DEFAULT_WEB_PORT})\n    \
                     --auth <user:pass>    Require HTTP Basic auth on the web view"
                );
                std::process::exit(0);
            }
            other => {
                anyhow::bail!("unknown argument: {other} (try --help)");
            }
        }
    }
    Ok(args)
}

/// The default task file in the platform data directory.
fn default_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "forster-todo")
        .context("could not determine a data directory for this platform")?;
    Ok(dirs.data_dir().join("tasks.txt"))
}
