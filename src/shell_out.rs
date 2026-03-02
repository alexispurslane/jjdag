use crate::commit_data::CommitData;
use crate::log_tree::strip_ansi;
use crate::model::GlobalArgs;
use crate::terminal::{self, Term};
use anyhow::{Result, anyhow};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use regex::Regex;
use std::{
    env,
    io::{Read, Write},
    process::Command,
};

#[derive(Debug)]
pub struct JjCommand {
    args: Vec<String>,
    global_args: GlobalArgs,
    interactive_term: Option<Term>,
    return_output: ReturnOutput,
    sync: bool,
}

impl JjCommand {
    fn _new(
        args: &[&str],
        global_args: GlobalArgs,
        interactive_term: Option<Term>,
        return_output: ReturnOutput,
    ) -> Self {
        Self {
            args: args.iter().map(|a| a.to_string()).collect(),
            global_args,
            interactive_term,
            return_output,
            sync: true,
        }
    }

    fn _new_skip_sync(
        args: &[&str],
        global_args: GlobalArgs,
        interactive_term: Option<Term>,
        return_output: ReturnOutput,
    ) -> Self {
        Self {
            args: args.iter().map(|a| a.to_string()).collect(),
            global_args,
            interactive_term,
            return_output,
            sync: false,
        }
    }

    pub fn sync(&self) -> bool {
        self.sync
    }

    pub fn to_lines(&self) -> Vec<Line<'static>> {
        let line = Line::from(vec![
            Span::styled("❯", Style::default().fg(Color::Yellow)),
            Span::raw(" jj "),
            Span::raw(self.args.join(" ")),
        ]);
        let blank_line = Line::raw("");
        vec![line, blank_line]
    }

    pub fn run(&self) -> Result<String, JjCommandError> {
        let output = match &self.interactive_term {
            None => self.run_noninteractive(),
            Some(term) => self.run_interactive(term),
        }?;
        match self.return_output {
            ReturnOutput::Stdout => Ok(output.stdout),
            ReturnOutput::Stderr => Ok(output.stderr),
        }
    }

    fn run_noninteractive(&self) -> Result<JjCommandOutput, JjCommandError> {
        log::info!("Running jj command: {}", self.args.join(" "));
        let mut command = self.base_command();
        command.args(self.args.clone());
        let output = command.output().map_err(JjCommandError::new_other)?;

        let stderr = String::from_utf8_lossy(&output.stderr).into();
        if output.status.success() {
            log::debug!("Command succeeded: {}", self.args.join(" "));
            let stdout = String::from_utf8_lossy(&output.stdout).into();
            Ok(JjCommandOutput { stdout, stderr })
        } else {
            log::error!("Command failed: {} - {}", self.args.join(" "), stderr);
            Err(JjCommandError::new_failed(stderr))
        }
    }

    fn run_interactive(&self, term: &Term) -> Result<JjCommandOutput, JjCommandError> {
        log::info!("Running interactive jj command: {}", self.args.join(" "));
        let mut command = self.base_command();
        command.args(self.args.clone());
        command.stderr(std::process::Stdio::piped());

        terminal::relinquish_terminal().map_err(JjCommandError::new_other)?;

        let mut child = command.spawn().map_err(JjCommandError::new_other)?;
        let mut stderr_handle = child
            .stderr
            .take()
            .ok_or_else(|| JjCommandError::new_other(anyhow!("No stderr handle")))?;
        let mut buf = Vec::new();
        stderr_handle
            .read_to_end(&mut buf)
            .map_err(JjCommandError::new_other)?;
        let stderr = strip_non_style_ansi(&String::from_utf8_lossy(&buf));
        let status = child.wait().map_err(JjCommandError::new_other)?;

        terminal::takeover_terminal(term).map_err(JjCommandError::new_other)?;

        if status.success() {
            log::debug!("Interactive command succeeded: {}", self.args.join(" "));
            Ok(JjCommandOutput {
                stdout: "".to_string(),
                stderr,
            })
        } else {
            log::error!(
                "Interactive command failed: {} - {}",
                self.args.join(" "),
                stderr
            );
            Err(JjCommandError::new_failed(stderr))
        }
    }

    fn base_command(&self) -> Command {
        let mut command = Command::new("jj");
        let args = [
            "--color",
            "always",
            "--config",
            "ui.pager=:builtin",
            "--config",
            "ui.streampager.interface=full-screen-clear-output",
            "--config",
            r#"templates.log_node=
            coalesce(
              if(!self, label("elided", "~")),
              label(
                separate(" ",
                  if(current_working_copy, "working_copy"),
                  if(immutable, "immutable"),
                  if(conflict, "conflict"),
                ),
                coalesce(
                  if(current_working_copy, "@"),
                  if(root, "┴"),
                  if(immutable, "●"),
                  if(conflict, "⊗"),
                  "○",
                )
              )
            )
        "#,
            "--repository",
            &self.global_args.repository,
        ];
        command.args(args);

        if self.global_args.ignore_immutable {
            command.arg("--ignore-immutable");
        }

        command
    }

    pub fn log(revset: &str, limit: usize, global_args: GlobalArgs) -> Self {
        let args = [
            "log",
            "--template",
            "builtin_log_compact",
            "--revisions",
            revset,
            "--limit",
            &limit.to_string(),
        ];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn diff_summary(change_id: &str, global_args: GlobalArgs) -> Self {
        let args = ["diff", "--summary", "--revisions", change_id];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    /// Get diff summary as JSON for structured parsing.
    /// Returns JSON lines with path, status, and source_path.
    pub fn diff_summary_json(change_id: &str, global_args: GlobalArgs) -> Self {
        let template = r#"{"path": path, "status": status, "source_path": source_path}"#;
        let args = [
            "diff",
            "--summary",
            "--revisions",
            change_id,
            "--template",
            template,
            "--color",
            "never",
        ];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn diff_file(change_id: &str, file: &str, global_args: GlobalArgs) -> Self {
        let args = ["diff", "--color-words", "--revisions", change_id, file];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn diff_file_interactive(
        change_id: &str,
        file: &str,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let args = ["diff", "--revisions", change_id, file];
        Self::_new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn diff_from_to_interactive(
        from: &str,
        to: &str,
        file: Option<&str>,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let mut args = vec!["diff", "--from", from, "--to", to];
        if let Some(file) = file {
            args.push(file);
        }
        Self::_new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn describe(
        change_id: &str,
        ignore_immutable: bool,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let mut args = vec!["describe", change_id];
        if ignore_immutable {
            args.push("--ignore-immutable");
        }
        Self::_new(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn describe_with_message(
        change_id: &str,
        message: &str,
        ignore_immutable: bool,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["describe", change_id, "--message", message];
        if ignore_immutable {
            args.push("--ignore-immutable");
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    /// Get the full description of a change
    pub fn get_description(change_id: &str, global_args: GlobalArgs) -> Self {
        let args = vec![
            "log",
            "-r",
            change_id,
            "-T",
            "description",
            "--no-graph",
            "--no-pager",
        ];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn duplicate(
        change_id: &str,
        destination_type: Option<&str>,
        destination: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["duplicate", change_id];
        if let (Some(destination_type), Some(destination)) = (destination_type, destination) {
            args.push(destination_type);
            args.push(destination);
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn new(target: &str, flags: &[&str], global_args: GlobalArgs) -> Self {
        let mut args = vec!["new"];
        args.extend_from_slice(flags);
        args.push(target);
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn parallelize(revset: &str, global_args: GlobalArgs) -> Self {
        let args = ["parallelize", revset];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn next_prev(
        direction: &str,
        mode: Option<&str>,
        offset: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec![direction];
        if let Some(mode) = mode {
            args.push(mode);
        }
        if let Some(offset) = offset {
            args.push(offset);
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn abandon(change_id: &str, mode: Option<&str>, global_args: GlobalArgs) -> Self {
        let mut args = vec!["abandon"];
        if let Some(mode) = mode {
            args.push(mode);
        }
        args.push(change_id);
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn absorb(
        from_change_id: &str,
        maybe_into_change_id: Option<&str>,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["absorb", "--from", from_change_id];
        if let Some(into_change_id) = maybe_into_change_id {
            args.push("--into");
            args.push(into_change_id);
        }
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn revert(
        revision: &str,
        destination_type: &str,
        destination: &str,
        global_args: GlobalArgs,
    ) -> Self {
        let args = ["revert", "-r", revision, destination_type, destination];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn sign(action: &str, revset: &str, global_args: GlobalArgs) -> Self {
        let args = [action, "-r", revset];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn show(change_id: &str, global_args: GlobalArgs, term: Term) -> Self {
        let args = ["show", change_id];
        Self::_new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn status(global_args: GlobalArgs, term: Term) -> Self {
        let args = ["status"];
        Self::_new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn simplify_parents(revision: &str, mode: &str, global_args: GlobalArgs) -> Self {
        let args = ["simplify-parents", mode, revision];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn split(change_id: &str, message: &str, global_args: GlobalArgs, term: Term) -> Self {
        let args = ["split", "-r", change_id, "-m", message];
        Self::_new(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn undo(global_args: GlobalArgs) -> Self {
        let args = ["undo"];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn redo(global_args: GlobalArgs) -> Self {
        let args = ["redo"];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn commit(maybe_file_path: Option<&str>, global_args: GlobalArgs, term: Term) -> Self {
        let mut args = vec!["commit"];
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::_new(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn rebase(
        source_type: &str,
        source: &str,
        destination_type: &str,
        destination: &str,
        global_args: GlobalArgs,
    ) -> Self {
        let args = vec!["rebase", source_type, source, destination_type, destination];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn restore(flags: &[&str], maybe_file_path: Option<&str>, global_args: GlobalArgs) -> Self {
        let mut args = vec!["restore"];
        args.extend_from_slice(flags);
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn squash_noninteractive(
        change_id: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["squash", "--revision", change_id];
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn squash_interactive(
        change_id: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let mut args = vec!["squash", "--revision", change_id];
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::_new(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn squash_into_interactive(
        from_change_id: &str,
        into_change_id: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let mut args = vec!["squash", "--from", from_change_id, "--into", into_change_id];
        if let Some(file_path) = maybe_file_path {
            args.push(file_path);
        }
        Self::_new(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn tug(global_args: GlobalArgs) -> Self {
        let args = [
            "bookmark",
            "move",
            "--from",
            "heads(::@- & bookmarks())",
            "--to",
            "@",
        ];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn edit(change_id: &str, ignore_immutable: bool, global_args: GlobalArgs) -> Self {
        let mut args = vec!["edit", change_id];
        if ignore_immutable {
            args.push("--ignore-immutable");
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn resolve(change_id: &str, global_args: GlobalArgs, term: Term) -> Self {
        let args = ["resolve", "-r", change_id];
        Self::_new(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn evolog(change_id: &str, patch: bool, global_args: GlobalArgs, term: Term) -> Self {
        let mut args = vec!["evolog", "-r", change_id];
        if patch {
            args.push("--patch");
        }
        Self::_new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn interdiff(
        from: &str,
        to: &str,
        maybe_file_path: Option<&str>,
        global_args: GlobalArgs,
        term: Term,
    ) -> Self {
        let mut args = vec!["interdiff", "--from", from, "--to", to];
        if let Some(path) = maybe_file_path {
            args.push(path);
        }
        Self::_new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn file_track(file_path: &str, global_args: GlobalArgs) -> Self {
        let args = ["file", "track", file_path];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn file_untrack(file_path: &str, global_args: GlobalArgs) -> Self {
        let args = ["file", "untrack", file_path];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn file_list_untracked(global_args: GlobalArgs) -> Self {
        let args = ["file", "list", "--untracked"];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn metaedit(
        change_id: &str,
        flag: &str,
        value: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["metaedit", flag];
        if let Some(value) = value {
            args.push(value);
        }
        args.push(change_id);
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn git_fetch(flag: Option<&str>, value: Option<&str>, global_args: GlobalArgs) -> Self {
        let mut args = vec!["git", "fetch"];
        if let Some(flag) = flag {
            args.push(flag);
        }
        if let Some(value) = value {
            args.push(value);
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn git_remote_list(global_args: GlobalArgs) -> Self {
        let args = ["git", "remote", "list"];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn git_branch_list(remote: Option<&str>, global_args: GlobalArgs) -> Self {
        let mut args = vec!["git", "branch", "list"];
        if let Some(remote) = remote {
            args.push("-r");
            args.push(remote);
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn git_push(flag: Option<&str>, value: Option<&str>, global_args: GlobalArgs) -> Self {
        let mut args = vec!["git", "push"];
        if let Some(flag) = flag {
            args.push(flag);
        }
        if let Some(value) = value {
            args.push(value);
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    /// Fetch from a specific remote, optionally filtering by branch
    pub fn git_fetch_from_remote(
        remote: &str,
        branch: Option<&str>,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["git", "fetch", "--remote", remote];
        if let Some(branch) = branch {
            args.push("-b");
            args.push(branch);
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn bookmark_create(bookmark_names: &str, change_id: &str, global_args: GlobalArgs) -> Self {
        let args = [
            "bookmark",
            "create",
            "--revision",
            change_id,
            bookmark_names,
        ];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn bookmark_list(global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "list"];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn bookmark_list_with_args(args: &[&str], global_args: GlobalArgs) -> Self {
        Self::_new(args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn bookmark_delete(bookmark_names: &str, global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "delete", bookmark_names];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn bookmark_forget(
        bookmark_names: &str,
        include_remotes: bool,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec!["bookmark", "forget"];
        if include_remotes {
            args.push("--include-remotes");
        }
        args.push(bookmark_names);
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn bookmark_move(
        from_change_id: &str,
        to_change_id: &str,
        allow_backwards: bool,
        global_args: GlobalArgs,
    ) -> Self {
        let mut args = vec![
            "bookmark",
            "move",
            "--from",
            from_change_id,
            "--to",
            to_change_id,
        ];
        if allow_backwards {
            args.push("--allow-backwards");
        }
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn bookmark_rename(
        old_bookmark_name: &str,
        new_bookmark_name: &str,
        global_args: GlobalArgs,
    ) -> Self {
        let args = ["bookmark", "rename", old_bookmark_name, new_bookmark_name];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn bookmark_set(bookmark_names: &str, change_id: &str, global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "set", bookmark_names, "--revision", change_id];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn bookmark_track(bookmark_at_remote: &str, global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "track", bookmark_at_remote];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn bookmark_untrack(bookmark_at_remote: &str, global_args: GlobalArgs) -> Self {
        let args = ["bookmark", "untrack", bookmark_at_remote];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn workspace_list(global_args: GlobalArgs) -> Self {
        let args = ["workspace", "list"];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn workspace_root(global_args: GlobalArgs) -> Self {
        let args = ["workspace", "root"];
        Self::_new(&args, global_args, None, ReturnOutput::Stdout)
    }

    pub fn workspace_forget(name: &str, global_args: GlobalArgs) -> Self {
        let args = ["workspace", "forget", name];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn workspace_rename(new_name: &str, global_args: GlobalArgs) -> Self {
        let args = ["workspace", "rename", new_name];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn workspace_update_stale(global_args: GlobalArgs) -> Self {
        let args = ["workspace", "update-stale", "--all"];
        Self::_new(&args, global_args, None, ReturnOutput::Stderr)
    }

    pub fn workspace_add(path: &str, global_args: GlobalArgs, term: Term) -> Self {
        let args = ["workspace", "add", path];
        Self::_new_skip_sync(&args, global_args, Some(term), ReturnOutput::Stderr)
    }

    pub fn ensure_valid_repo(repository: &str) -> Result<String, JjCommandError> {
        log::debug!("Validating repository: {}", repository);
        let args = [
            "--repository",
            repository,
            "workspace",
            "root",
            "--color",
            "always",
        ];
        let output = Command::new("jj")
            .args(args)
            .output()
            .map_err(JjCommandError::new_other)?;

        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout)
                .to_string()
                .trim()
                .to_string();
            log::debug!("Repository validated at: {}", root);
            Ok(root)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).into();
            log::warn!(
                "Repository validation failed for '{}': {}",
                repository,
                stderr
            );
            Err(JjCommandError::new_failed(stderr))
        }
    }

    /// Run dual log commands to get both display and structured data.
    ///
    /// This runs two jj log commands simultaneously:
    /// - One with builtin_log_compact template for display output
    /// - One with JSON template for structured data parsing
    ///
    /// # Arguments
    /// - `revset`: The revision set to query
    /// - `limit`: Maximum number of commits to fetch
    /// - `global_args`: Global arguments for jj commands
    ///
    /// # Returns
    /// - `Ok((Vec<CommitData>, Vec<String>))`: Structured commit data and display lines
    /// - `Err(JjCommandError)`: If either command fails
    pub fn log_dual(
        revset: &str,
        limit: usize,
        global_args: GlobalArgs,
    ) -> Result<(Vec<CommitData>, Vec<String>), JjCommandError> {
        // Run display log command
        let display_cmd = Self::log(revset, limit, global_args.clone());
        let display_output = display_cmd.run()?;
        let display_lines: Vec<String> = display_output.lines().map(|s| s.to_string()).collect();

        // Build JSON template for structured data
        // Note: parents field omitted - jj template list mapping syntax varies by version
        let json_template = r###"
              "{" ++
                "\"change_id\": " ++ json(change_id) ++ ", " ++
                "\"commit_id\": " ++ json(commit_id) ++ ", " ++
                "\"description\": " ++ json(description) ++ ", " ++
                "\"author\": " ++ json(author.email()) ++ ", " ++
                "\"timestamp\": " ++ stringify(author.timestamp().format("%s")) ++ ", " ++
                "\"parent_change_ids\": [" ++ parents.map(|c| json(c.change_id())).join(", ") ++ "], " ++
                "\"is_working_copy\": " ++ json(current_working_copy) ++ ", " ++
                "\"is_empty\": " ++ json(empty) ++ ", " ++
                "\"has_conflict\": " ++ json(conflict) ++
              "}\n"
        "###;

        // Run JSON log command
        let json_args = [
            "log",
            "--no-graph",
            "--template",
            json_template,
            "--revisions",
            revset,
            "--limit",
            &limit.to_string(),
        ];
        let json_cmd = Self::_new(&json_args, global_args, None, ReturnOutput::Stdout);
        let json_output = json_cmd.run()?;

        // Parse JSON lines into CommitData
        let mut commits = Vec::new();
        let mut parse_failures = 0;
        let mut empty_lines = 0;
        for line in json_output.lines() {
            let line = line.trim();
            if line.is_empty() {
                empty_lines += 1;
                continue;
            }
            match serde_json::from_str::<CommitData>(line) {
                Ok(commit) => {
                    log::debug!(
                        "log_dual: parsed commit {} with change_id {}",
                        commits.len(),
                        commit.change_id
                    );
                    commits.push(commit);
                }
                Err(e) => {
                    parse_failures += 1;
                    log::warn!(
                        "log_dual: Failed to parse commit JSON: {} - line: {}",
                        e,
                        line
                    );
                    continue;
                }
            }
        }

        log::info!(
            "log_dual: parsed {} commits, {} parse failures, {} empty lines from {} display lines",
            commits.len(),
            parse_failures,
            empty_lines,
            display_lines.len()
        );

        Ok((commits, display_lines))
    }
}

/// Build display mappings by correlating display lines to commit indices.
///
/// Analyzes display lines to find which commit each line belongs to using
/// change_id pattern matching. Returns the mappings needed for MappingBuffer::new().
///
/// # Arguments
/// - `display_lines`: The display output lines from jj log
/// - `commits`: The structured commit data with change_ids
///
/// # Returns
/// A tuple of (line_to_tree_pos, tree_index_to_line_range) for MappingBuffer
/// Extract change_id from a display line.
///
/// Skips leading non-alphabetic characters (graph chars, symbols, whitespace),
/// then reads exactly 8 alphabetical characters.
///
/// - `line`: The display line from jj log output
fn extract_change_id(line: &str) -> Option<String> {
    // Step 1: Strip ANSI codes
    let stripped = strip_ansi(line);

    // Step 2: Strip all leading non-alphanumeric characters (graph symbols, spaces, etc.)
    // This handles @, ○, ◆, │, ~, and any amount of whitespace
    let cleaned: String = stripped
        .chars()
        .skip_while(|c| !c.is_ascii_alphanumeric())
        .collect();

    // Step 3: Extract the first 8 alphabetic characters (the change_id)
    // Skip any remaining spaces after the graph symbols
    let content = cleaned.trim_start();
    if content.len() < 8 {
        log::debug!(
            "extract_change_id: too short after cleaning ({} chars)",
            content.len()
        );
        return None;
    }

    // Read exactly 8 characters for change_id candidate
    let candidate = &content[..8];

    // Verify all 8 chars are alphabetical (jj change_ids are lowercase letters)
    let all_alpha = candidate.chars().all(|c| c.is_ascii_alphabetic());
    if !all_alpha {
        return None;
    }

    // Verify 9th char is whitespace or end of string (jj uses space after change_id)
    let ninth_space =
        content.len() == 8 || content.chars().nth(8).map_or(true, |c| c.is_whitespace());
    if !ninth_space {
        return None;
    }

    Some(candidate.to_string())
}

/// Build display mappings by correlating display lines to commit indices.
///
/// Analyzes display lines to find which commit each line belongs to by
/// extracting change_ids at known positions. Returns the mappings needed
/// for MappingBuffer::new().
///
/// # Arguments
/// - `display_lines`: The display output lines from jj log
/// - `commits`: The structured commit data with change_ids
/// - `line_offset`: Offset to add to line indices (for appending)
///
/// # Returns
/// A tuple of (line_to_tree_pos, tree_index_to_line_range) for MappingBuffer
pub fn build_display_mappings(
    display_lines: &[String],
    commits: &[CommitData],
    line_offset: usize,
) -> (
    Vec<Vec<usize>>,
    std::collections::HashMap<usize, (usize, usize)>,
) {
    use std::collections::HashMap;

    log::debug!(
        "build_display_mappings: {} display_lines, {} commits, offset {}",
        display_lines.len(),
        commits.len(),
        line_offset
    );
    log::debug!(
        "build_display_mappings: commit change_ids: {:?}",
        commits.iter().map(|c| &c.change_id).collect::<Vec<_>>()
    );

    let mut line_to_tree_pos: Vec<Vec<usize>> = Vec::new();
    let mut tree_index_to_line_range: HashMap<usize, (usize, usize)> = HashMap::new();

    let mut current_commit_idx: Option<usize> = None;
    let mut commit_start_line: Option<usize> = None;
    let mut unmatched_lines: Vec<usize> = Vec::new();

    for (line_idx, line) in display_lines.iter().enumerate() {
        // Try to extract change_id from this line
        if let Some(found_change_id) = extract_change_id(line) {
            // Find which commit this change_id belongs to
            if let Some(commit_idx) = commits.iter().position(|c| {
                c.change_id.starts_with(&found_change_id) || c.change_id == found_change_id
            }) {
                log::debug!(
                    "build_display_mappings: line {} matched commit idx {} (change_id {:?})",
                    line_idx,
                    commit_idx,
                    commits[commit_idx].change_id
                );
                // Close previous commit range if we were tracking one
                if let Some(prev_idx) = current_commit_idx {
                    if let Some(start) = commit_start_line {
                        tree_index_to_line_range
                            .insert(prev_idx, (start + line_offset, line_idx + line_offset));
                    }
                }

                // Start tracking new commit
                current_commit_idx = Some(commit_idx);
                commit_start_line = Some(line_idx);
            } else {
                log::warn!(
                    "build_display_mappings: line {} extracted change_id {:?} but no matching commit found",
                    line_idx,
                    found_change_id
                );
            }
        } else {
            log::debug!(
                "build_display_mappings: line {} - no change_id extracted (line content: {:?})",
                line_idx,
                line
            );
        }

        // Map this line to the current commit's tree position
        if let Some(commit_idx) = current_commit_idx {
            line_to_tree_pos.push(vec![commit_idx]);
        } else {
            line_to_tree_pos.push(vec![]);
            unmatched_lines.push(line_idx);
        }
    }

    if !unmatched_lines.is_empty() {
        log::warn!(
            "build_display_mappings: {} lines with empty TreePosition: {:?}",
            unmatched_lines.len(),
            &unmatched_lines[..unmatched_lines.len().min(10)]
        );
    }

    // Close the last commit's range
    if let Some(commit_idx) = current_commit_idx {
        if let Some(start) = commit_start_line {
            tree_index_to_line_range.insert(
                commit_idx,
                (start + line_offset, display_lines.len() + line_offset),
            );
        }
    }

    log::debug!(
        "build_display_mappings: completed - line_to_tree_pos has {} entries, tree_index_to_line_range has {} entries",
        line_to_tree_pos.len(),
        tree_index_to_line_range.len()
    );

    (line_to_tree_pos, tree_index_to_line_range)
}

#[derive(Debug)]
enum ReturnOutput {
    Stdout,
    Stderr,
}

#[derive(Debug)]
pub enum JjCommandError {
    Failed { stderr: String },
    Other { err: anyhow::Error },
}

impl JjCommandError {
    fn new_failed(stderr: String) -> Self {
        Self::Failed {
            stderr: stderr.trim().to_string(),
        }
    }

    fn new_other(err: impl Into<anyhow::Error>) -> Self {
        Self::Other { err: err.into() }
    }
}

impl std::fmt::Display for JjCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed { stderr } => {
                write!(f, "{stderr}")
            }
            Self::Other { err } => err.fmt(f),
        }
    }
}

impl std::error::Error for JjCommandError {}

/// Parse the workspace_store/index file to find a workspace's path.
/// The file uses a simple protobuf-like format where each entry is:
///   0a <total_len> 0a <name_len> <name> 12 <path_len> <path>
/// Returns None if workspace not found or file cannot be read.
pub fn get_workspace_path(repo_root: &str, workspace_name: &str) -> Option<String> {
    let index_path = std::path::Path::new(repo_root)
        .parent()?
        .join("default")
        .join(".jj/repo/workspace_store/index");
    let contents = std::fs::read(index_path).ok()?;

    let mut i = 0;
    while i < contents.len() {
        // Each entry starts with 0a (field 1, wire type 2) followed by total length
        if contents.get(i)? != &0x0a {
            break;
        }
        i += 1;

        // Read total entry length (varint, but for reasonable sizes it's 1 byte)
        let entry_len = *contents.get(i)? as usize;
        i += 1;
        let entry_end = i + entry_len;

        // Parse name field: 0a <len> <bytes>
        if contents.get(i)? != &0x0a {
            break;
        }
        i += 1;
        let name_len = *contents.get(i)? as usize;
        i += 1;
        let name_bytes = contents.get(i..i + name_len)?;
        i += name_len;
        let name = String::from_utf8_lossy(name_bytes);

        // Parse path field: 12 <len> <bytes>
        if contents.get(i)? != &0x12 {
            break;
        }
        i += 1;
        let path_len = *contents.get(i)? as usize;
        i += 1;
        let path_bytes = contents.get(i..i + path_len)?;

        if name == workspace_name {
            return Some(String::from_utf8_lossy(path_bytes).to_string());
        }

        // Skip to next entry (in case we had padding or miscalculated)
        i = entry_end;
    }

    None
}

/// Update the path for a workspace in jj's workspace_store/index file.
/// This is necessary after renaming a workspace directory to keep jj's store in sync.
pub fn update_workspace_path(
    global_args: &GlobalArgs,
    workspace_name: &str,
    new_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::path::PathBuf::from(&global_args.repository)
        .parent()
        .unwrap()
        .join("default");
    log::debug!(
        "repo_root='{}' workspace='{}' new_path='{}'",
        repo_root.to_string_lossy(),
        workspace_name,
        new_path
    );
    let index_path = repo_root.join(".jj/repo/workspace_store/index");
    log::debug!("index_path='{}'", index_path.display());
    let contents = std::fs::read(&index_path)?;
    log::debug!("read {} bytes", contents.len());

    let new_path_bytes = new_path.as_bytes();
    let workspace_name_bytes = workspace_name.as_bytes();

    // Calculate new entry size
    // Format: 0a <total_len> 0a <name_len> <name> 12 <path_len> <path>
    let new_name_field_len = 2 + workspace_name_bytes.len(); // 0a <len> <bytes>
    let new_path_field_len = 2 + new_path_bytes.len(); // 12 <len> <bytes>
    let new_entry_len = new_name_field_len + new_path_field_len;

    let mut new_contents = Vec::new();
    let mut i = 0;
    let mut found = false;

    while i < contents.len() {
        log::debug!("parsing at i={}", i);
        // Check entry start
        if contents.get(i) != Some(&0x0a) {
            log::debug!("no more entries at i={}", i);
            // Copy remaining bytes as-is
            new_contents.extend_from_slice(&contents[i..]);
            break;
        }
        i += 1;

        // Read entry length
        let entry_len = *contents.get(i).ok_or("Unexpected end of file")? as usize;
        i += 1;
        let entry_start = i;
        let entry_end = entry_start + entry_len;

        log::debug!("entry_len={} end={}", entry_len, entry_end);
        // Parse name field
        if contents.get(i) != Some(&0x0a) {
            new_contents.extend_from_slice(&contents[entry_start - 2..entry_end]);
            i = entry_end;
            continue;
        }
        i += 1;
        let name_len = *contents.get(i).ok_or("Unexpected end of file")? as usize;
        i += 1;
        let name_bytes = &contents
            .get(i..i + name_len)
            .ok_or("Unexpected end of file")?;
        let name = String::from_utf8_lossy(name_bytes);
        log::debug!("name='{}'", name);
        i += name_len;

        // Parse path field (skip for now)
        if contents.get(i) != Some(&0x12) {
            new_contents.extend_from_slice(&contents[entry_start - 2..entry_end]);
            i = entry_end;
            continue;
        }
        i += 1;
        let path_len = *contents.get(i).ok_or("Unexpected end of file")? as usize;
        i += 1;
        let old_path_start = i;
        i += path_len; // Skip old path bytes
        let old_path =
            String::from_utf8_lossy(&contents[old_path_start..old_path_start + path_len]);
        log::debug!("old_path='{}'", old_path);

        log::debug!("found name: '{}'", workspace_name);
        if name == workspace_name {
            log::debug!("FOUND '{}', updating path", workspace_name);
            // Found the workspace - write updated entry
            found = true;
            log::debug!("writing new entry len={}", new_entry_len + 2);
            new_contents.push(0x0a); // Entry start tag
            new_contents.push(new_entry_len as u8); // Entry length
            new_contents.push(0x0a); // Name field tag
            new_contents.push(workspace_name_bytes.len() as u8); // Name length
            new_contents.extend_from_slice(workspace_name_bytes); // Name bytes
            new_contents.push(0x12); // Path field tag
            new_contents.push(new_path_bytes.len() as u8); // Path length
            new_contents.extend_from_slice(new_path_bytes); // Path bytes
        } else {
            // Copy original entry
            new_contents.extend_from_slice(&contents[entry_start - 2..entry_end]);
        }

        i = entry_end;
    }

    if found {
        log::debug!("writing {} bytes to index", new_contents.len());
        std::fs::write(&index_path, new_contents)?;
        log::debug!("SUCCESS");
        Ok(())
    } else {
        log::debug!("FAILED: workspace '{}' not found", workspace_name);
        Err(format!("Workspace '{}' not found in store", workspace_name).into())
    }
}

struct JjCommandOutput {
    stdout: String,
    stderr: String,
}

pub fn get_input_from_editor(
    interactive_term: Term,
    starting_text: Option<&str>,
    help_text: Option<&str>,
) -> Result<Option<String>> {
    // Create temp file
    let mut temp_file = tempfile::Builder::new()
        .suffix(".jjdescription")
        .tempfile()?;
    if let Some(text) = starting_text {
        writeln!(temp_file, "{text}")?;
        temp_file.flush()?;
    }
    if let Some(text) = help_text {
        writeln!(temp_file, "\n\nJJ: {text}")?;
        writeln!(
            temp_file,
            "JJ: Lines starting with \"JJ:\" (like this one) will be removed."
        )?;

        temp_file.flush()?;
    }
    let temp_path = temp_file.path().to_path_buf();

    // Open editor in temp file
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    terminal::relinquish_terminal()?;
    let status = Command::new(&editor).arg(&temp_path).status()?;
    terminal::takeover_terminal(&interactive_term)?;
    if !status.success() {
        anyhow::bail!("Editor exited with non-zero status");
    }

    // Remove all lines starting with "JJ: "
    let contents = std::fs::read_to_string(&temp_path)?;
    let contents: String = contents
        .lines()
        .filter(|line| !line.starts_with("JJ:"))
        .collect::<Vec<&str>>()
        .join("\n")
        .trim()
        .to_string();
    if contents.is_empty() {
        Ok(None)
    } else {
        Ok(Some(contents))
    }
}

fn strip_non_style_ansi(str: &str) -> String {
    let non_style_ansi_regex =
        Regex::new(r"\x1b(\[[0-9;?]*[ -/]*([@-l]|[n-~])|\].*?(\x07|\x1b\\)|P.*?\x1b\\)").unwrap();
    non_style_ansi_regex.replace_all(str, "").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_change_id_simple() {
        // Plain line without ANSI
        let line = "@  qvmzlrxu alexispurslane@pm.me 2026-03-02 17:43:34 ffaac8d7";
        assert_eq!(extract_change_id(line), Some("qvmzlrxu".to_string()));
    }

    #[test]
    fn test_extract_change_id_with_graph_chars() {
        // Different graph characters
        assert_eq!(
            extract_change_id("○  lmomxzrx alexispurslane@pm.me"),
            Some("lmomxzrx".to_string())
        );
        assert_eq!(
            extract_change_id("◆  vstovwow main 38dd4dc4"),
            Some("vstovwow".to_string())
        );
        assert_eq!(
            extract_change_id("│  (no description set)"),
            None // Should fail - no change_id
        );
    }

    #[test]
    fn test_extract_change_id_with_ansi() {
        // Line with ANSI color codes (simulating what jj outputs)
        let line = "\u{1b}[1m\u{1b}[38;5;14m●\u{1b}[0m  \u{1b}[1m\u{1b}[38;5;5mkrx\u{1b}[0m\u{1b}[38;5;8momvwm\u{1b}[39m \u{1b}[38;5;3malexispurslane@pm.me\u{1b}[39m";
        let result = extract_change_id(line);
        println!("ANSI line result: {:?}", result);
        // After stripping ANSI: "●  krxomvwm alexispurslane@pm.me"
        // Should extract "krxomvwm"
        assert_eq!(result, Some("krxomvwm".to_string()));
    }

    #[test]
    fn test_extract_change_id_debug_real_cases() {
        // Real cases from the log that were failing
        let cases = vec![
            ("@  qvmzlrxu alexispurslane@pm.me", Some("qvmzlrxu")),
            ("○  lmomxzrx alexispurslane@pm.me", Some("lmomxzrx")),
            ("◆  vstovwow alexispurslane@pm.me", Some("vstovwow")),
            ("│  (no description set)", None),
            ("│  mapping buffer! :)", None),
        ];

        for (line, expected) in cases {
            let result = extract_change_id(line);
            println!("Input: {:?}", line);
            println!("Expected: {:?}, Got: {:?}\n", expected, result);
            assert_eq!(result, expected.map(|s| s.to_string()));
        }
    }
}
