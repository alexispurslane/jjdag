# Jjdag Agent Documentation

This is a list of instructions to correct common mistakes / misconceptions agents tend to have about the codebase.

**To agents**: if anything you find in the codebase confuses you, or takes a long time to figure out, add it to the list.

## Instructions

- **Testing Requirement**: All major building blocks of the codebase must have tests
- **Docstring Requirement**: All functions, struct members, and enum arms must have docstrings
- **Function Docstrings**: Must use a bullet-point list to specify the semantics of each argument
- **td Task Descriptions**: When creating tasks with `td`, descriptions must follow this structure:
  - **Context**: One-two sentence big picture context/background
  - **Technical Description**: What will be done, using nested bullet points or numbered lists for algorithms, type signatures, etc.
  - **Deliverables**: Specific artifacts that must be complete to close the ticket
  - **Acceptance Criteria**: Must include tests for the thing in question
  - Keep descriptions concise but complete
  - **All td tasks must have BOTH a title AND a description**
- use jujutsu's cli help commands to look at what commands can and cannot do, what arguments they accept, what sub-commands they have, and how they work, before implementing the equivalent in the TUI
- **Power Workspaces**: A jjdag TUI feature layered on top of jj's native workspace system:
  - "Scoop up" - moving the initial workspace into a `default/` subdirectory when adding the first additional workspace
  - "Un-scoop" - restoring to standard structure when only `default` workspace remains after a forget
  - These are NOT native jj commands and can only be done through the TUI
  - After scoop: all workspaces are in subdirectories (`default/`, `other/`, etc.)
  - `global_args.repository` points to the current workspace directory, not the project root
  - Path calculations must account for this nesting
- **Agent Working Directory**: The agent's shell always starts at the project root (`jjdag/`), not inside `test_env/`. Use explicit paths like `jjdag/test_env/` when referencing test files.
- **Early Exit State Tracking**: When using early exits (break, return, continue) in loops, pay attention to post-loop invariants:
  - Never early break/return until AFTER verifying subsequent logic won't be affected
  - Audit every variable left behind after an early exit and verify assumptions made by code after the break still hold
  - **Pattern**: If you must break out of a loop early, set a loop-body-local exit flag in the condition that should trigger the break, then break on that condition flag at the end of that loop iteration, after it's done all of its post-loop-iteration updates:
  ```rust
  loop {
    let mut should_break = false;
    if condition { should_break = true; } // set flag, don't break yet
    do_post_loop_work(); // complete iteration
    if should_break { break; }
  }
  ```
