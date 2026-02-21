# jjdag - Agent Documentation

## Project Purpose

`jjdag` is a Terminal User Interface (TUI) for the [Jujutsu](https://github.com/jj-vcs/jj) version control system. It provides an interactive, keyboard-driven interface for visualizing and manipulating the Jujutsu DAG (Directed Acyclic Graph) of commits.

**Key Features:**
- Browse the jj log tree with dynamic folding/unfolding of commits and file diffs
- Multi-key command sequences (Magit transient-style, e.g., `gpa` → `jj git push --all`)
- Transient-menu style help popups showing available commands
- Mouse support: click to select, right-click to toggle folding, scroll wheel
- Real-time output display from jj commands

## Architecture Overview

The project follows the **Model-Update-View (MUV)** pattern, inspired by the Elm Architecture.

### Core Flow

```
┌─────────┐    ┌─────────┐    ┌─────────┐
│  Model  │◄───│ Update  │◄───│ Events  │
│ (State) │    │(Handler)│    │(Key/Mouse)│
└────┬────┘    └─────────┘    └─────────┘
     │
     ▼
┌─────────┐
│  View   │───► Terminal (ratatui + crossterm)
└─────────┘
```

### State Management

- **Single Source of Truth**: All application state lives in `Model` (`src/model.rs`)
- **Message-Driven Updates**: Events are translated into `Message` enums (`src/update.rs`), which are processed to mutate the Model
- **Immutable Transitions**: State changes are explicit and centralized

## Module Structure

| Module | Purpose |
|--------|---------|
| `main.rs` | Entry point, initializes terminal, runs main TUI loop |
| `model.rs` | Central state container (~1800 lines) - contains all app state and business logic methods |
| `update.rs` | Event handling, keybinding dispatch, message processing |
| `view.rs` | Rendering logic using ratatui widgets |
| `command_tree.rs` | Emacs transient-style keybinding tree definition (~1700 lines of command definitions) |
| `log_tree.rs` | DAG tree structure, commit/file/diff rendering and folding |
| `shell_out.rs` | External process execution (jj binary wrapper) |
| `terminal.rs` | Terminal initialization, raw mode, panic hooks |
| `cli.rs` | CLI argument parsing (clap) |

## Code Patterns

### 1. Model-Update-View Pattern

```jjdag/src/main.rs#L32-42
fn tui_loop(mut model: Model, terminal: Term) -> Result<()> {
    while model.state != State::Quit {
        terminal.borrow_mut().draw(|f| view(&mut model, f))?;
        update(terminal.clone(), &mut model)?;
    }
    Ok(())
}
```

The main loop is simple: draw the view, then process updates.

### 2. Message-Driven State Changes

All state modifications flow through the `Message` enum:

```jjdag/src/update.rs#L78-217
pub enum Message {
    Abandon { mode: AbandonMode },
    GitPush { mode: GitPushMode },
    // ... 40+ variants
    ToggleLogListFold,
    Quit,
}
```

### 3. Command Tree Keybindings

A tree structure enables multi-key commands inspired by Magit transients:

```jjdag/src/command_tree.rs#L69-72
pub struct CommandTreeNode {
    pub children: Option<CommandTreeNodeChildren>,
    pub action: Option<Message>,
}
```

Example: `g` → `p` → `a` executes `jj git push --all`

### 4. Log Tree Traversal

The DAG is rendered using a trait-based tree structure:

```jjdag/src/log_tree.rs#L124-135
pub trait LogTreeNode {
    fn render(&self) -> Text<'static>;
    fn flatten(&self, depth: usize, items: &mut Vec<Text<'static>>);
    fn flat_log_idx(&self) -> usize;
    fn children(&self) -> Option<&Vec<Box<dyn LogTreeNode>>>;
    fn toggle_fold(&mut self);
}
```

Implementors: `Commit`, `FileDiff`, `DiffHunk`, `DiffHunkLine`, `InfoText`

### 5. Shell Abstraction

All jj command execution is encapsulated:

```jjdag/src/shell_out.rs#L14-20
pub struct JjCommand {
    args: Vec<String>,
    global_args: GlobalArgs,
    interactive_term: bool,
    return_output: bool,
    sync: bool,
}
```

### 6. Popup System

Modal dialogs with fuzzy filtering for interactive selection:

```jjdag/src/update.rs#L10-43
pub enum Popup {
    BookmarkDelete { bookmarks: Vec<String> },
    GitPushBookmark { bookmarks: Vec<String>, change_id: String, is_named_mode: bool },
    // ...
}
```

### 7. Terminal Lifecycle Management

Proper cleanup on panic via custom hook:

```jjdag/src/terminal.rs#L36-42
pub fn install_panic_hook() {
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        relinquish_terminal().unwrap();
        original_hook(panic_info);
    }));
}
```

## Key Design Decisions

1. **Single-File Model**: All state and business logic lives in `model.rs` (~1800 lines). This centralizes domain logic at the cost of file size.

2. **Command Queue**: Jj commands are queued and processed sequentially rather than async/await, ensuring sequential consistency with the VCS.

3. **Selection Persistence**: The model saves selection state (change_id, file_path) so the view can restore position after refresh.

4. **ANSI Stripping**: Output from jj commands has ANSI codes stripped before storage to ensure clean rendering.

5. **Folding State**: Each tree node tracks its own `unfolded` boolean, enabling granular control over what's visible.

## Dependencies

| Category | Crates |
|----------|--------|
| CLI | `clap` |
| TUI | `ratatui`, `crossterm`, `ansi-to-tui` |
| Error Handling | `anyhow` |
| Collections | `indexmap` |
| Parsing | `regex` |
| Testing | `tempfile` |

## Nix Development

The project includes a `flake.nix` for reproducible development environments using Nix.
