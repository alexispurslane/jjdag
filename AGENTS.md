# Jjdag Agent Documentation

This is a list of instructions to correct common mistakes / misconceptions agents tend to have about the codebase.

**To agents**: if anything you find in the codebase confuses you, or takes a long time to figure out, add it to the list.

## Instructions

- use jujutsu's cli help commands to look at what commands can and cannot do, what arguments they accept, what sub-commands they have, and how they work, before implementing the equivalent in the TUI
- **tmux**: Use tmux for interactive testing of TUI workflows:
  - Create session: `tmux new-session -d -s <name>`
  - Send keys: `tmux send-keys -t <name> "<keys>"`
  - Capture screen: `tmux capture-pane -t <name> -p` (capture whole pane, don't use `tail` which cuts off text prompts)
  - Kill session: `tmux kill-session -t <name>`
- **test_env**: A test environment exists at `jjdag/test_env/` for manual testing. Use `reset_test_env.sh` to reset it to a fresh jj repo state.
- **Power Workspaces**: The "power workspace workflow" is a jjdag TUI feature layered on top of jj's native workspace system. It adds:
  - "Scoop up" - moving the initial workspace into a `default/` subdirectory when adding the first additional workspace
  - "Un-scoop" - restoring to standard structure when only `default` workspace remains after a forget
  - These are NOT native jj commands and can only be done through the TUI
- **Directory Structure Awareness**: When working with power-workflow repos:
  - After scoop: all workspaces are in subdirectories (`default/`, `other/`, etc.)
  - `global_args.repository` points to the current workspace directory, not the project root
  - Path calculations must account for this nesting
- **Agent Working Directory**: The agent's shell always starts at the project root (`jjdag/`), not inside `test_env/`. Use explicit paths like `jjdag/test_env/` when referencing test files.
- **Early Exit State Tracking**: When using early exits (break, return, continue) in loops, pay attention to post-loop invariants:
  - Never early break/return until AFTER verifying subsequent logic won't be affected
  - Audit every variable left behind after an early exit and verify assumptions made by code after the break still hold
  - **Pattern**: If you must break out of a loop early, set a loop-body-local exit flag in the condition that should trigger the break, then break on that condition flag at the end of that loop iteration, after it's done all of its post-loop-iteration updates:
  ```rust
  for (idx, line) in lines_vec.iter().enumerate() {
    let line_end = current_pos + line.len();

    let mut cursor_found = false; // loop exit condition flag!
    if self.text_cursor <= line_end {
        // we need to exit, but we DON'T BREAK HERE
        cursor_found = true; // isntead, we set the flag
    }
    
    current_pos = line_end + 1; // now, we let our post-loop-iteration exit code run
    
    if cursor_found {
        break; // then we break
    }
  }
  ```
