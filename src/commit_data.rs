//! Pure data structures for commit information parsed from JSON.
//!
//! These structures are separate from the display/rendering logic in log_tree.rs
//! and contain only the raw data needed for commit visualization.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Default fold state for commits and file diffs (start folded).
fn default_folded() -> bool {
    true
}

/// The status of a file in a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileDiffStatus {
    /// File was modified.
    Modified,
    /// File was added.
    Added,
    /// File was deleted.
    Deleted,
    /// File was renamed.
    Renamed,
    /// File was copied.
    Copied,
}

/// Entry from jj diff --summary --template JSON output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSummaryEntry {
    pub path: String,
    pub status: String,
    #[serde(default)]
    pub source_path: Option<String>,
}

/// A diff for a single file within a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    /// The change ID of the commit this file diff belongs to.
    pub change_id: String,
    /// The path of the file (or target path for renames/copies).
    pub path: String,
    /// The status of this file in the diff.
    pub status: FileDiffStatus,
    /// The source path for renames/copies (None for other statuses).
    pub source_path: Option<String>,
    /// Whether this file diff is folded (collapsed).
    #[serde(default = "default_folded")]
    pub folded: bool,
    /// Graph prefix for rendering unfolded content.
    ///
    /// Inherited from the parent commit's graph_prefix with additional
    /// indentation. Not serialized — set at runtime.
    #[serde(skip)]
    pub graph_prefix: String,
}

/// Complete data for a single commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitData {
    /// The change ID (e.g., "sktvpxlq" or a full ID).
    pub change_id: String,
    /// The commit ID (SHA).
    pub commit_id: String,
    /// The commit message/description.
    pub description: String,
    /// The author of the commit.
    pub author: String,
    /// The timestamp of the commit (Unix timestamp milliseconds).
    pub timestamp: i64,
    /// Change IDs of parent commits.
    pub parent_change_ids: Vec<String>,
    /// Whether this is the working copy.
    pub is_working_copy: bool,
    /// Whether the commit has no changes.
    pub is_empty: bool,
    /// Whether the commit has conflicts.
    pub has_conflict: bool,
    /// The file diffs for this commit, if loaded.
    pub file_diffs: Option<Vec<FileDiff>>,
    /// Whether this commit is folded (collapsed).
    #[serde(default = "default_folded")]
    pub folded: bool,
    /// Extra headers from jj log output (e.g., "git_refs", "branches").
    #[serde(flatten)]
    pub extra_headers: HashMap<String, String>,
    /// Graph prefix for rendering unfolded content.
    ///
    /// Extracted from the jj log display line's description line and
    /// transformed into a continuation prefix. For example, if the
    /// description line is `"├─╯  fixing some bugs"`, the graph_prefix
    /// would be `"│    "` (the continuation of the main line).
    /// Not serialized — set at runtime after parsing display lines.
    #[serde(skip)]
    pub graph_prefix: String,
}
