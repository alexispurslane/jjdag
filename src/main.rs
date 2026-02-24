mod cli;
mod command_tree;
mod log_tree;
mod logger;
mod model;
mod shell_out;
mod terminal;
mod update;
mod view;

use crate::model::{Model, State};
use crate::update::update;
use crate::view::view;

use anyhow::Result;
use clap::Parser;
use cli::Args;
use log::Level;
use shell_out::JjCommand;
use terminal::Term;

fn main() {
    let _ = logger::FileLogger::init(Level::Debug);
    log::info!("jjdag starting up");

    let result = run();
    if let Err(err) = result {
        log::error!("Fatal error: {}", err);
        // Avoids a redundant message "Error: Error:"
        eprintln!("{err}");
        std::process::exit(1);
    }
    log::info!("jjdag shutting down normally");
}

fn run() -> Result<()> {
    let args = Args::parse();
    log::info!("CLI args parsed, repository: {:?}", args.repository);
    let repository = match JjCommand::ensure_valid_repo(&args.repository) {
        Ok(repo) => repo,
        Err(_) => {
            // Launch detection: check for subdirectory with .jj/ (power workspace post-scoop case)
            let cwd = std::env::current_dir()?;
            let entries: Vec<_> = std::fs::read_dir(&cwd)?.filter_map(|e| e.ok()).collect();

            log::info!("Attempting power workspace recovery in: {:?}", cwd);
            // Find subdirectory containing .jj/
            let mut recovered_path: Option<std::path::PathBuf> = None;
            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    let jj_path = path.join(".jj");
                    if jj_path.exists() && jj_path.is_dir() {
                        recovered_path = Some(path);
                        break;
                    }
                }
            }

            match recovered_path {
                Some(path) => {
                    log::info!("Power workspace recovered, changing to: {:?}", path);
                    // Found power workspace subdirectory - change into it
                    std::env::set_current_dir(&path)?;
                    JjCommand::ensure_valid_repo(".")?
                }
                None => {
                    // No recovery possible - propagate error by retrying
                    JjCommand::ensure_valid_repo(&args.repository)?
                }
            }
        }
    };
    log::info!("Repository validated: {}", repository);
    let model = Model::new(repository, args.revisions)?;
    log::info!(
        "Model initialized with {} revisions",
        model.jj_log.log_tree.len()
    );

    let terminal = terminal::init_terminal()?;
    log::info!("Starting TUI loop");
    let result = tui_loop(model, terminal);
    log::info!("TUI loop ended");
    terminal::relinquish_terminal()?;

    result
}

fn tui_loop(mut model: Model, terminal: Term) -> Result<()> {
    log::debug!("Entering TUI loop");
    while model.state != State::Quit {
        terminal.borrow_mut().draw(|f| view(&mut model, f))?;
        update(terminal.clone(), &mut model)?;
    }
    log::debug!("TUI loop exiting, state: {:?}", model.state);
    Ok(())
}
