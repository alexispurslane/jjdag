# jjdag

![](screenshot.png)

A Rust TUI to manipulate the [Jujutsu](https://github.com/jj-vcs/jj) DAG.

Inspired by the great UX of [Magit](https://magit.vc/).

Very much a work in progress, consider this a pre-alpha release. But I already use it personally for almost all jj operations.

Once you run the program you can press `?` to show the help info. Most of the commands you can see by running `jj help` in the terminal are implemented.

## Features

- Browse the jj log tree with dynamic folding/unfolding of commits and file diffs.
- Multi-key command sequences with transient-menu style help popups. For example type `gpa` to run `jj git push --all`, or `gpt` to run `jj git push --tracked`, or `ss` to squash the selected revision into its parent.
- Output from jj commands is displayed in the bottom panel.
- Mouse support: left click to select, right click to toggle folding, and scroll wheel to scroll.

## Supported jj commands

- `jj abandon`
- `jj absorb`
- `jj bookmark create`
- `jj bookmark delete`
- `jj bookmark forget`
- `jj bookmark move`
- `jj bookmark rename`
- `jj bookmark set`
- `jj bookmark track`
- `jj bookmark untrack`
- `jj commit`
- `jj describe`
- `jj diff`
- `jj duplicate`
- `jj edit`
- `jj evolog`
- `jj file track`
- `jj file untrack`
- `jj git fetch`
- `jj git push`
- `jj interdiff`
- `jj metaedit`
- `jj new`
- `jj next`
- `jj parallelize`
- `jj prev`
- `jj rebase`
- `jj redo`
- `jj restore`
- `jj revert`
- `jj sign`
- `jj simplify-parents`
- `jj squash`
- `jj status`
- `jj undo`
- `jj unsign`

## Installation

With cargo: 
```sh
cargo install --git https://github.com/anthrofract/jjdag
```

Or with the nix flake:
```nix
inputs.jjdag.url = "github:anthrofract/jjdag";
```

Missing Features in jjdag

### 1. **Config Management** (`jj config`)
- `config edit`, `get`, `list`, `path`, `set`, `unset`
- Essential for users wanting to tweak jj settings from within the TUI

### 2. **Bookmark List** (`jj bookmark list`)
- View all bookmarks with their targets, tracking status, and conflicts
- Currently only has create/delete/move/rename/etc, but no way to *list* bookmarks in a popup

### 3. **File Operations** (`jj file` subcommands)
- **`annotate`** - Show source change for each line (git blame equivalent)
- **`chmod`** - Set/remove executable bit
- **`list`** - List files in a revision
- **`search`** - Search for content in files
- **`show`** - Print file contents

### 4. **Git Operations**
- **`git clone`** - Clone a new repo
- **`git init`** - Initialize a new Git-backed repo
- **`git remote`** - Manage remotes (add, list, remove, rename, set-url)
- **`git export/import`** - Sync with underlying Git repo
- **`git colocation`** - Enable/disable colocation status
- **`git root`** - Show underlying Git directory

### 5. **Operation Log** (`jj operation`)
- **`op log`** - View operation history (undo/redo log)
- **`op show`** - Show changes made by a specific operation
- **`op diff`** - Compare repo state between operations
- **`op restore`** - Restore repo to an earlier operation state
- **`op revert`** - Revert a specific operation
- **`op abandon`** - Discard old operation history
- **`op integrate`** - Integrate orphaned operations

### 6. **Conflict Resolution**
- **`resolve`** - Launch external merge tool for conflicted files
- Currently only has `next/prev --conflict` to navigate to conflicts, but no resolution UI

### 7. **Tag Management** (`jj tag`)
- **`tag list`** - List tags and their targets
- **`tag set`** - Create/update tags
- **`tag delete`** - Delete tags

### 8. **Sparse Checkouts** (`jj sparse`)
- **`sparse list/edit/set/reset`** - Manage which paths are present in the working copy

### 9. **Workspace Management** (`jj workspace`)
- **`workspace add`** - Add additional working copies
- **`workspace list`** - List workspaces
- **`workspace forget`** - Stop tracking a workspace
- **`workspace rename`** - Rename current workspace
- **`workspace root`** - Show workspace root
- **`workspace update-stale`** - Update stale workspaces

### 10. **Bisect** (`jj bisect`)
- **`bisect run`** - Binary search to find first bad revision

### 11. **Code Formatting** (`jj fix`)
- **`fix`** - Apply formatting fixes or other transformations to revisions

### 12. **Gerrit Integration** (`jj gerrit`)
- **`gerrit upload`** - Upload changes to Gerrit for code review

### 13. **Utility Commands** (`jj util`)
- **`util completion`** - Shell completion scripts
- **`util gc`** - Garbage collection
- **`util exec`** - Execute external commands
- Others: `config-schema`, `install-man-pages`, `markdown-help`

### 14. **Diff Editing**
- **`diffedit`** - Touch up content changes with a diff editor (distinct from the TUI's fold/unfold view)

### 15. **Misc**
- **`show`** - Show commit description and changes (jjdag has its own view, but `show` has different formatting options)
- **`root`** - Show workspace root directory
- **`version`** - Display version

---

## Priority Assessment

**High-value additions for a TUI:**
1. **`bookmark list`** - Would fit naturally in a popup like the existing bookmark delete popup
2. **`op log`** - Critical for understanding undo/redo history; could be a new panel
3. **`resolve`** - Essential for conflict workflows (currently requires leaving the TUI)
4. **`tag list/set/delete`** - Tags are commonly used alongside bookmarks
5. **`file annotate`** - Git blame is a common TUI operation
6. **`workspace list`** - Useful for multi-workspace workflows

**Medium priority:**
- `config` commands (usually one-time setup)
- `file list/show` (partially covered by the diff view)
- `git remote` management

**Lower priority (niche/advanced):**
- `bisect` (complex interactive workflow)
- `gerrit` (specific to Gerrit users)
- `fix` (usually run via hooks/CI)
- `util` commands
