use crate::commit_data::{CommitData, FileDiff, FileDiffStatus};
use crate::mapping_buffer::MappingBuffer;
use crate::model::GlobalArgs;
use crate::shell_out::{JjCommand, build_display_mappings};
use ansi_to_tui::IntoText;
use anyhow::{Result, anyhow};
use ratatui::text::Text;
use regex::Regex;
use std::sync::{Arc, Mutex, OnceLock};

/// Strip ANSI escape codes from a string.
///
/// Removes all ANSI escape sequences (e.g., color codes, cursor movement)
/// from the input string and returns the cleaned text.
pub fn strip_ansi(s: &str) -> String {
    // ANSI escape sequences: ESC [ ... m (color codes) or ESC [ ... other commands
    static RE_ANSI: OnceLock<Regex> = OnceLock::new();
    let re = RE_ANSI.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*m").unwrap());
    re.replace_all(s, "").to_string()
}

/// Result of toggling a node's fold state.
pub enum ToggleFoldResult {
    /// The node was folded. Contains the line range (start, end) that should be removed from display.
    Folded((usize, usize)),
    /// The node was unfolded. Contains the lines to insert and the insertion point.
    Unfolded {
        lines: Vec<Text<'static>>,
        insertion_point: usize,
    },
}

const INITIAL_LOAD_COUNT: usize = 200;
const LOAD_BATCH_SIZE: usize = 200;

#[derive(Debug)]
pub struct JjLog {
    pub commits: Vec<CommitData>,
    revset: String,
    global_args: GlobalArgs,
    mapping_buffer: Arc<Mutex<MappingBuffer>>,
}

impl JjLog {
    /// Create a new JjLog with a shared MappingBuffer.
    ///
    /// - `mapping_buffer`: Shared Arc<Mutex<MappingBuffer>> for population
    ///
    /// Returns (Self, display_lines) where display_lines are the raw log output lines.
    pub fn new(mapping_buffer: Arc<Mutex<MappingBuffer>>) -> Result<(Self, Vec<String>)> {
        let jj_log = JjLog {
            commits: Vec::new(),
            revset: String::new(),
            global_args: GlobalArgs {
                repository: String::new(),
                ignore_immutable: false,
            },
            mapping_buffer,
        };
        Ok((jj_log, Vec::new()))
    }

    pub fn load_log_tree(&mut self, global_args: &GlobalArgs, revset: &str) -> Result<Vec<String>> {
        self.global_args = global_args.clone();
        self.revset = revset.to_string();

        // Use dual-source loading: JSON for data, template for display
        let (commits, display_lines) =
            JjCommand::log_dual(revset, INITIAL_LOAD_COUNT, global_args.clone())?;

        // Build display mappings from display_lines and commits
        let (line_to_tree_pos, _) = build_display_mappings(&display_lines, &commits, 0);

        // Populate the shared MappingBuffer
        {
            let mut buffer = self
                .mapping_buffer
                .lock()
                .map_err(|e| anyhow!("Failed to lock mapping buffer: {}", e))?;
            buffer.rebuild(line_to_tree_pos);
        }

        // Store the structured commits
        self.commits = commits;

        Ok(display_lines)
    }

    pub fn load_more(&mut self, line_offset: usize) -> Result<(Vec<String>, bool)> {
        // Get the last commit's change_id to use as offset
        let last_id = self.commits.last().map(|c| c.change_id.clone());
        let last_id = match last_id {
            Some(id) => id,
            None => return Ok((Vec::new(), false)),
        };

        // Use revset to get commits older than last_change_id
        let revset = format!("..{}-", last_id);
        let (commits, display_lines) =
            JjCommand::log_dual(&revset, LOAD_BATCH_SIZE, self.global_args.clone())?;

        let new_count = commits.len();
        if new_count > 0 {
            self.commits.extend(commits);

            let (line_to_tree_pos, _) =
                build_display_mappings(&display_lines, &self.commits, line_offset);

            let mut buffer = self
                .mapping_buffer
                .lock()
                .map_err(|e| anyhow!("Failed to lock mapping buffer: {}", e))?;
            buffer.notify_appended(line_to_tree_pos);
        }

        Ok((display_lines, new_count > 0))
    }

    /// Get commit data by change_id
    pub fn get_commit(&self, change_id: &str) -> Option<&CommitData> {
        self.commits
            .iter()
            .find(|c| c.change_id == change_id || c.change_id.starts_with(change_id))
    }

    pub fn get_tree_node(&mut self, tree_pos: &TreePosition) -> Result<&mut dyn LogTreeNode> {
        self.get_tree_node_mut(tree_pos)
            .ok_or_else(|| anyhow::anyhow!("Node not found at tree position {:?}", tree_pos))
    }

    pub fn get_tree_commit(&self, tree_pos: &TreePosition) -> Option<&CommitData> {
        let commit_idx = tree_pos.first()?;
        self.commits.get(*commit_idx)
    }

    pub fn get_tree_file_diff(&self, tree_pos: &TreePosition) -> Option<&FileDiff> {
        let commit_idx = tree_pos.first()?;
        let commit = self.commits.get(*commit_idx)?;
        let file_idx = tree_pos.get(1)?;
        let file_diffs = commit.file_diffs.as_ref()?;
        file_diffs.get(*file_idx)
    }

    pub fn get_current_commit(&self) -> Option<&CommitData> {
        self.commits.iter().find(|c| c.is_working_copy)
    }

    /// Get a mutable reference to a node at the given tree position.
    ///
    /// tree_pos format: [commit_idx] or [commit_idx, file_idx] or [commit_idx, file_idx, hunk_idx]
    fn get_tree_node_mut(&mut self, tree_pos: &TreePosition) -> Option<&mut dyn LogTreeNode> {
        let commit_idx = tree_pos.first().copied()?;
        let commit = self.commits.get_mut(commit_idx)?;

        if tree_pos.len() == 1 {
            return Some(commit);
        }

        let file_idx = tree_pos.get(1).copied()?;
        let file_diffs = commit.file_diffs.as_mut()?;
        let file_diff = file_diffs.get_mut(file_idx)?;

        if tree_pos.len() == 2 {
            return Some(file_diff);
        }

        // Only support commit and file levels, no hunks
        Some(file_diff)
    }

    /// Toggle fold state at the given tree position and return display lines for any newly visible content.
    ///
    /// This is called by Model when user toggles fold on a line.
    /// - If unfolding: loads children via JSON if needed, renders them, returns display lines
    /// - If folding: cascades fold to descendants, returns empty (Model removes lines via MappingBuffer)
    ///
    /// Note: node.render() returns display lines for the node's children, not the node itself.
    /// For example, commit.render() returns file diff lines via `jj diff --name-only`.
    pub fn toggle_fold(
        &mut self,
        global_args: &GlobalArgs,
        tree_pos: &TreePosition,
        mapping_buffer: &Arc<Mutex<MappingBuffer>>,
    ) -> Result<ToggleFoldResult> {
        // Get insertion point and child range BEFORE toggle
        let (insertion_point, child_range) = {
            let buffer = mapping_buffer
                .lock()
                .map_err(|e| anyhow::anyhow!("Failed to lock mapping buffer: {}", e))?;
            let insert = buffer.get_insertion_point(tree_pos);
            let range = buffer.get_child_line_range(tree_pos);
            (insert, range)
        };

        let node = self
            .get_tree_node_mut(tree_pos)
            .ok_or_else(|| anyhow::anyhow!("Node not found at tree position {:?}", tree_pos))?;

        // Toggle the node's fold state (this loads children if unfolding)
        node.toggle_fold(global_args)?;

        // If now unfolded, render and notify MappingBuffer
        if !node.is_folded() {
            let new_lines = node.render(global_args)?;
            let insert_idx = insertion_point.unwrap_or(0);
            let count = new_lines.len();

            // Notify MappingBuffer of insertion
            let mut buffer = mapping_buffer
                .lock()
                .map_err(|e| anyhow::anyhow!("Failed to lock mapping buffer: {}", e))?;
            buffer.notify_inserted(insert_idx, count, tree_pos)?;

            return Ok(ToggleFoldResult::Unfolded {
                lines: new_lines,
                insertion_point: insert_idx,
            });
        }

        // If folded, notify MappingBuffer of removal and return empty
        if let Some((start, end)) = child_range {
            let tree_index = tree_pos.first().copied().unwrap_or(0);
            let mut buffer = mapping_buffer
                .lock()
                .map_err(|e| anyhow::anyhow!("Failed to lock mapping buffer: {}", e))?;
            buffer.notify_removed(start, end, tree_index)?;
            return Ok(ToggleFoldResult::Folded((start, end)));
        }

        Ok(ToggleFoldResult::Folded((0, 0)))
    }
}

/// Trait for nodes in the display tree hierarchy (commit → file diff → diff hunk → diff line).
///
/// This represents the *visual* tree structure shown in the UI, NOT the commit
/// parent/child DAG relationships. For DAG navigation (parent commits, siblings),
/// use `CommitData.parent_change_ids` directly.
pub trait LogTreeNode {
    /// Render this node's children to displayable Text lines using ANSI templates.
    ///
    /// For example:
    /// - CommitData.render() calls `jj diff --name-only` and returns file list lines
    /// - FileDiff.render() calls `jj diff <file>` and returns diff content lines
    /// - DiffHunk.render() returns the diff lines for that hunk
    ///
    /// Returns empty Vec if folded or no children loaded.
    fn render(&self, global_args: &GlobalArgs) -> Result<Vec<Text<'static>>>;

    /// Toggle the folded state of this node.
    /// - If folding: cascades fold to all descendants
    /// - If unfolding: populates children via JSON templates
    /// Returns unit - no display work. JjLog orchestrates rendering.
    fn toggle_fold(&mut self, global_args: &GlobalArgs) -> Result<()>;

    /// Check if this node is currently folded.
    fn is_folded(&self) -> bool;

    /// Get child nodes that are currently loaded.
    /// Returns empty vec if children not loaded or if leaf node.
    fn children(&self) -> Vec<&dyn LogTreeNode>;

    /// Load children from jj using JSON templates.
    /// Called by toggle_fold when unfolding and children not loaded.
    fn load_children(&mut self, global_args: &GlobalArgs) -> Result<()>;
}

pub type TreePosition = Vec<usize>;
const COMMIT_OR_TEXT_IDX: usize = 0;
const FILE_DIFF_IDX: usize = 1;
const DIFF_HUNK_IDX: usize = 2;
pub const DIFF_HUNK_LINE_IDX: usize = 3;

pub fn get_parent_tree_position(tree_pos: &TreePosition) -> Option<TreePosition> {
    let mut tree_pos = tree_pos.clone();
    if tree_pos.len() > 1 {
        tree_pos.pop();
        Some(tree_pos)
    } else {
        None
    }
}

// ============================================================================
// LogTreeNode implementations for the display hierarchy
// ============================================================================

impl LogTreeNode for CommitData {
    fn render(&self, global_args: &GlobalArgs) -> Result<Vec<Text<'static>>> {
        // If folded or no file diffs loaded, return empty
        if self.folded || self.file_diffs.is_none() {
            return Ok(Vec::new());
        }

        // Render file diffs by calling jj diff --name-only
        // This returns the list of changed files as display lines
        let output = JjCommand::diff_summary(&self.change_id, global_args.clone())
            .run()
            .map_err(|e| anyhow::anyhow!("Failed to get diff summary: {}", e))?;

        let lines: Vec<Text<'static>> = output
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| format!("    {}", line).into_text())
            .collect::<Result<Vec<_>, _>>()?;

        Ok(lines)
    }

    fn toggle_fold(&mut self, global_args: &GlobalArgs) -> Result<()> {
        self.folded = !self.folded;

        if self.folded {
            // Cascading fold: fold all descendants
            if let Some(ref mut file_diffs) = self.file_diffs {
                for file_diff in file_diffs {
                    file_diff.folded = true;
                }
            }
        } else {
            // Unfolding: load file diffs if not already loaded
            self.load_children(global_args)?;
        }

        Ok(())
    }

    fn is_folded(&self) -> bool {
        self.folded
    }

    fn children(&self) -> Vec<&dyn LogTreeNode> {
        if self.folded {
            return Vec::new();
        }
        if let Some(ref file_diffs) = self.file_diffs {
            file_diffs.iter().map(|fd| fd as &dyn LogTreeNode).collect()
        } else {
            Vec::new()
        }
    }

    fn load_children(&mut self, global_args: &GlobalArgs) -> Result<()> {
        if self.file_diffs.is_some() {
            // Already loaded
            return Ok(());
        }

        // Load file diffs via jj diff --summary with JSON template
        let output = JjCommand::diff_summary_json(&self.change_id, global_args.clone())
            .run()
            .map_err(|e| anyhow::anyhow!("Failed to load diff summary: {}", e))?;

        // Parse JSON lines into DiffSummaryEntry structs
        let mut loaded_diffs = Vec::new();
        for line in output.lines().filter(|l| !l.is_empty()) {
            let entry: crate::commit_data::DiffSummaryEntry = serde_json::from_str(line)
                .map_err(|e| anyhow::anyhow!("Failed to parse diff summary JSON: {}", e))?;

            // Map status string to FileDiffStatus
            let status = match entry.status.as_str() {
                "added" => FileDiffStatus::Added,
                "deleted" => FileDiffStatus::Deleted,
                "modified" => FileDiffStatus::Modified,
                "renamed" => FileDiffStatus::Renamed,
                "copied" => FileDiffStatus::Copied,
                _ => FileDiffStatus::Modified, // Default for unknown statuses
            };

            loaded_diffs.push(FileDiff {
                change_id: self.change_id.clone(),
                path: entry.path,
                status,
                source_path: entry.source_path,
                folded: true, // Children start folded
            });
        }

        self.file_diffs = Some(loaded_diffs);
        Ok(())
    }
}

impl LogTreeNode for FileDiff {
    fn render(&self, global_args: &GlobalArgs) -> Result<Vec<Text<'static>>> {
        // If folded, return empty
        if self.folded {
            return Ok(Vec::new());
        }

        // Render diff for this file by calling jj diff
        let output = JjCommand::diff_file(&self.change_id, &self.path, global_args.clone())
            .run()
            .map_err(|e| anyhow::anyhow!("Failed to get diff for {}: {}", self.path, e))?;

        let lines: Vec<Text<'static>> = output
            .lines()
            .map(|line| format!("      {}", line).into_text())
            .collect::<Result<Vec<_>, _>>()?;

        Ok(lines)
    }

    fn toggle_fold(&mut self, _global_args: &GlobalArgs) -> Result<()> {
        self.folded = !self.folded;
        Ok(())
    }

    fn is_folded(&self) -> bool {
        self.folded
    }

    fn children(&self) -> Vec<&dyn LogTreeNode> {
        // Files are leaf nodes now - no hunks
        Vec::new()
    }

    fn load_children(&mut self, _global_args: &GlobalArgs) -> Result<()> {
        // Files are leaf nodes - no children to load
        Ok(())
    }
}
