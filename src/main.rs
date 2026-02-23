mod cli;
mod command_tree;
mod log_tree;
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
use shell_out::JjCommand;
use terminal::Term;

fn main() {
    let result = run();
    if let Err(err) = result {
        // Avoids a redundant message "Error: Error:"
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let repository = match JjCommand::ensure_valid_repo(&args.repository) {
        Ok(repo) => repo,
        Err(_) => {
            // Launch detection: check for subdirectory with .jj/ (power workspace post-scoop case)
            let cwd = std::env::current_dir()?;
            let entries: Vec<_> = std::fs::read_dir(&cwd)?.filter_map(|e| e.ok()).collect();

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
    let model = Model::new(repository, args.revisions)?;

    let terminal = terminal::init_terminal()?;
    let result = tui_loop(model, terminal);
    terminal::relinquish_terminal()?;

    result
}

fn tui_loop(mut model: Model, terminal: Term) -> Result<()> {
    while model.state != State::Quit {
        terminal.borrow_mut().draw(|f| view(&mut model, f))?;
        update(terminal.clone(), &mut model)?;
    }
    Ok(())
}
