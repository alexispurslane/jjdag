use std::fmt;

use crate::log_tree::TreePosition;

/// Errors that can occur when constructing or validating a MappingBuffer.
#[derive(Debug)]
pub enum MappingError {
    /// An invariant was violated during construction or validation.
    InvariantViolation { which: String },
}

impl fmt::Display for MappingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MappingError::InvariantViolation { which } => {
                write!(f, "Invariant violation: {}", which)
            }
        }
    }
}

impl std::error::Error for MappingError {}

/// Encapsulates bidirectional mappings between line indices and tree positions.
/// Uses tree index (TreePosition[0]) as the key for self-validation.
#[derive(Debug)]
pub struct MappingBuffer {
    /// Forward mapping: line index -> tree position
    line_to_tree_pos: Vec<TreePosition>,
}

impl MappingBuffer {
    /// Create an empty MappingBuffer with no mappings.
    ///
    /// This is useful for initializing a buffer before loading data.
    pub fn new_empty() -> Self {
        Self {
            line_to_tree_pos: Vec::new(),
        }
    }

    /// Rebuild the buffer with new forward mapping.
    ///
    /// This replaces all existing mappings with the provided ones.
    ///
    /// - `line_to_tree_pos`: New forward mapping from line index to tree position
    pub fn rebuild(&mut self, line_to_tree_pos: Vec<TreePosition>) {
        self.line_to_tree_pos = line_to_tree_pos;
    }

    /// Get the TreePosition for a given line index.
    pub fn get_tree_position(&self, line_idx: usize) -> Option<&TreePosition> {
        self.line_to_tree_pos.get(line_idx)
    }

    pub fn get_exact_line_for_tree_position(&self, tree_pos: &TreePosition) -> Option<usize> {
        self.line_to_tree_pos.iter().position(|pos| pos == tree_pos)
    }

    /// Get the total number of lines.
    pub fn line_count(&self) -> usize {
        self.line_to_tree_pos.len()
    }

    /// Find the line index for a given tree position.
    ///
    /// Scans line_to_tree_pos to find where the tree position appears.
    /// If the exact position is not found, walks up parent positions
    /// (e.g., [0,1,2] -> [0,1] -> [0]) until a match is found.
    /// Returns None if no ancestor position is found.
    pub fn get_line_for_tree_position(&self, tree_pos: &TreePosition) -> Option<usize> {
        let mut current_pos = tree_pos.clone();

        while !current_pos.is_empty() {
            // Try to find exact match for current position
            if let Some(line_idx) = self.get_exact_line_for_tree_position(&current_pos) {
                return Some(line_idx);
            }

            // No match found, try parent (truncate last element)
            current_pos.pop();
        }

        // Try root level (empty tree position)
        for (line_idx, pos) in self.line_to_tree_pos.iter().enumerate() {
            if pos == &current_pos {
                return Some(line_idx);
            }
        }

        None
    }

    /// Find the line range of children for a given parent tree position.
    ///
    /// Scans lines after the parent to find consecutive lines where
    /// the tree position starts with the parent (is a child).
    ///
    /// Returns: Some((start_line, end_line)) if children found, None otherwise
    pub fn get_child_line_range(&self, tree_pos: &TreePosition) -> Option<(usize, usize)> {
        let start_line = self.get_exact_line_for_tree_position(tree_pos)?;

        let end_line: usize = start_line
            + 1
            + self.line_to_tree_pos[start_line + 1..]
                .iter()
                .position(|pos| !pos.starts_with(tree_pos))
                .unwrap_or(self.line_to_tree_pos.len() - start_line - 1);

        Some((start_line + 1, end_line))
    }

    /// Get the insertion point for new children of a parent tree position.
    ///
    /// Returns the line index where new children should be inserted -
    /// either after existing children, or right after the parent if no children exist.
    ///
    /// # Arguments
    /// * `parent_tree_pos` - tree position of the parent node
    ///
    /// Returns: `Some(line_idx)` where insertion should happen, or `None` if parent not found
    pub fn get_insertion_point(&self, parent_tree_pos: &TreePosition) -> Option<usize> {
        self.get_child_line_range(parent_tree_pos)
            .map(|(_, end)| end)
            .or(self
                .get_line_for_tree_position(parent_tree_pos)
                .map(|line| line + 1))
    }

    /// Notify that lines have been inserted (e.g., when unfolding a commit or file).
    ///
    /// # Arguments
    /// * `at_idx` - line index where insertion starts
    /// * `count` - number of lines inserted
    /// * `parent_tree_pos` - tree position of parent (commit/file)
    pub fn notify_inserted(
        &mut self,
        at_idx: usize,
        count: usize,
        parent_tree_pos: &TreePosition,
    ) -> Result<(), MappingError> {
        // 1. Validate inputs
        let current_line_count = self.line_to_tree_pos.len();
        if at_idx > current_line_count {
            return Err(MappingError::InvariantViolation {
                which: format!(
                    "Insertion index {} is out of range (current line count: {})",
                    at_idx, current_line_count
                ),
            });
        }
        if count == 0 {
            return Err(MappingError::InvariantViolation {
                which: "Insertion count must be greater than 0".to_string(),
            });
        }

        // 2. Generate TreePositions for new lines
        let mut new_tree_positions: Vec<TreePosition> = Vec::with_capacity(count);
        for i in 0..count {
            let mut tree_pos = parent_tree_pos.clone();
            tree_pos.push(i);
            new_tree_positions.push(tree_pos);
        }

        // 3. Insert forward mappings
        // Insert at at_idx, which shifts existing entries
        let insert_pos = at_idx;
        for (i, tree_pos) in new_tree_positions.into_iter().enumerate() {
            self.line_to_tree_pos.insert(insert_pos + i, tree_pos);
        }

        Ok(())
    }

    /// Notify that lines have been removed (e.g., when collapsing a commit or file).
    ///
    /// # Arguments
    /// * `start_idx` - line index where removal starts
    /// * `count` - number of lines to remove
    /// * `tree_index` - tree index of the commit being collapsed
    pub fn notify_removed(
        &mut self,
        start_idx: usize,
        end_idx: usize,
        tree_index: usize,
    ) -> Result<(), MappingError> {
        // Validate inputs
        let current_line_count = self.line_to_tree_pos.len();
        let count = end_idx - start_idx;

        if end_idx > current_line_count {
            return Err(MappingError::InvariantViolation {
                which: format!(
                    "Removal range [{}, {}) exceeds line count {}",
                    start_idx, end_idx, current_line_count
                ),
            });
        }
        if count == 0 {
            return Err(MappingError::InvariantViolation {
                which: "Removal count must be greater than 0".to_string(),
            });
        }

        // Validate that all removed lines are children of the specified tree_index
        for i in start_idx..end_idx {
            let tree_pos = &self.line_to_tree_pos[i];
            if tree_pos[0] != tree_index {
                return Err(MappingError::InvariantViolation {
                    which: format!(
                        "Line {} has tree_index {} but expected {} (children of collapsed commit)",
                        i, tree_pos[0], tree_index
                    ),
                });
            }
        }

        // Remove forward mappings
        for _ in 0..count {
            self.line_to_tree_pos.remove(start_idx);
        }

        Ok(())
    }

    /// Notify that lines have been appended (e.g., when paginating more commits).
    ///
    /// # Arguments
    /// * `count` - number of lines appended
    /// * `tree_index` - tree index that the new lines belong to
    /// * `parent_tree_pos` - parent tree position (typically the commit being expanded)
    /// * `new_tree_index_to_line_range` - ranges for newly added tree indices
    pub fn notify_appended(&mut self, new_line_to_tree_pos: Vec<TreePosition>) {
        self.line_to_tree_pos.extend(new_line_to_tree_pos);
    }
}
