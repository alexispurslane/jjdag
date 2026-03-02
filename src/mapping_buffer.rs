use std::collections::HashMap;
use std::fmt;

use log::debug;

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
    /// Reverse mapping: tree index -> [start, end) line range
    tree_index_to_line_range: HashMap<usize, (usize, usize)>,
}

impl MappingBuffer {
    /// Create an empty MappingBuffer with no mappings.
    ///
    /// This is useful for initializing a buffer before loading data.
    pub fn new_empty() -> Self {
        Self {
            line_to_tree_pos: Vec::new(),
            tree_index_to_line_range: HashMap::new(),
        }
    }

    /// Rebuild the buffer with new mappings, validating all invariants.
    ///
    /// This replaces all existing mappings with the provided ones.
    /// Returns an error if the new mappings violate any invariants.
    ///
    /// - `line_to_tree_pos`: New forward mapping from line index to tree position
    /// - `tree_index_to_line_range`: New reverse mapping from tree index to line range
    pub fn rebuild(
        &mut self,
        line_to_tree_pos: Vec<TreePosition>,
        tree_index_to_line_range: HashMap<usize, (usize, usize)>,
    ) -> Result<(), MappingError> {
        // Validate using the same logic as new()
        // Validate invariant 2: Commit Range Validity (start < end)
        for (tree_index, (start, end)) in &tree_index_to_line_range {
            if start >= end {
                return Err(MappingError::InvariantViolation {
                    which: format!(
                        "Tree index {} has invalid range: start ({}) >= end ({})",
                        tree_index, start, end
                    ),
                });
            }
        }

        // Validate invariant 5: Check for overlapping ranges
        let mut ranges: Vec<(usize, usize, usize)> = tree_index_to_line_range
            .iter()
            .map(|(idx, (start, end))| (*start, *end, *idx))
            .collect();
        ranges.sort_by_key(|(start, _, _)| *start);

        for i in 1..ranges.len() {
            let (prev_start, prev_end, prev_idx) = &ranges[i - 1];
            let (curr_start, curr_end, curr_idx) = &ranges[i];
            if prev_end > curr_start {
                return Err(MappingError::InvariantViolation {
                    which: format!(
                        "Overlapping ranges: tree index {} range [{}, {}) overlaps with tree index {} range [{}, {})",
                        prev_idx, prev_start, prev_end, curr_idx, curr_start, curr_end
                    ),
                });
            }
        }

        // Validate invariant 3: Tree Position Validity (commit_idx at position 0)
        for (line_idx, tree_pos) in line_to_tree_pos.iter().enumerate() {
            if tree_pos.is_empty() {
                debug!(
                    "line {} has empty TreePosition (missing commit_idx)",
                    line_idx
                );
            }
        }

        // Validate invariant 4: Order Preservation
        let mut prev_end_line: Option<usize> = None;
        for &(start, end, _) in &ranges {
            if let Some(prev) = prev_end_line {
                if start < prev {
                    return Err(MappingError::InvariantViolation {
                        which: format!(
                            "Order preservation violated: range [{}, {}) starts before previous range ended at {}",
                            start, end, prev
                        ),
                    });
                }
            }
            prev_end_line = Some(end);
        }

        // Validate invariant 1: Mapping Consistency
        for (line_idx, tree_pos) in line_to_tree_pos.iter().enumerate() {
            if tree_pos.is_empty() {
                continue;
            }
            let tree_index = tree_pos[0];
            let (start, end) = tree_index_to_line_range.get(&tree_index).ok_or_else(|| {
                MappingError::InvariantViolation {
                    which: format!(
                        "No line range for tree_index {} at line {}",
                        tree_index, line_idx
                    ),
                }
            })?;

            if line_idx < *start || line_idx >= *end {
                return Err(MappingError::InvariantViolation {
                    which: format!(
                        "Mapping consistency violated: line {} (tree_index {}) is not in range [{}, {})",
                        line_idx, tree_index, start, end
                    ),
                });
            }
        }

        // All validations passed - replace the mappings
        self.line_to_tree_pos = line_to_tree_pos;
        self.tree_index_to_line_range = tree_index_to_line_range;

        Ok(())
    }

    /// Get the line range for a given tree index.
    pub fn get_line_range(&self, tree_index: usize) -> Option<&(usize, usize)> {
        self.tree_index_to_line_range.get(&tree_index)
    }

    /// Get the TreePosition for a given line index.
    pub fn get_tree_position(&self, line_idx: usize) -> Option<&TreePosition> {
        self.line_to_tree_pos.get(line_idx)
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
            for (line_idx, pos) in self.line_to_tree_pos.iter().enumerate() {
                if pos == &current_pos {
                    return Some(line_idx);
                }
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
    pub fn get_child_line_range(&self, parent_tree_pos: &TreePosition) -> Option<(usize, usize)> {
        // Find the parent's line index first
        let parent_line = self.get_line_for_tree_position(parent_tree_pos)?;

        let mut children_start: Option<usize> = None;
        let mut children_end = parent_line + 1;

        for i in (parent_line + 1)..self.line_count() {
            if let Some(pos) = self.get_tree_position(i) {
                // Check if this line is a child (starts with parent_tree_pos)
                if pos.len() > parent_tree_pos.len()
                    && pos[..parent_tree_pos.len()] == parent_tree_pos[..]
                {
                    if children_start.is_none() {
                        children_start = Some(i);
                    }
                    children_end = i + 1;
                } else {
                    // No longer a child, stop scanning
                    break;
                }
            }
        }

        children_start.map(|start| (start, children_end))
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
        // If children exist, insert after them
        if let Some((_, child_end)) = self.get_child_line_range(parent_tree_pos) {
            return Some(child_end);
        }

        // No children yet, insert right after parent
        self.get_line_for_tree_position(parent_tree_pos)
            .map(|line| line + 1)
    }

    /// Notify that lines have been inserted (e.g., when unfolding a commit or file).
    ///
    /// # Arguments
    /// * `at_idx` - line index where insertion starts
    /// * `count` - number of lines inserted
    /// * `parent_tree_pos` - tree position of parent (commit/file)
    /// * `tree_index` - tree index of the commit being unfolded
    pub fn notify_inserted(
        &mut self,
        at_idx: usize,
        count: usize,
        parent_tree_pos: &TreePosition,
        tree_index: usize,
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

        println!("{:?}", self.line_to_tree_pos);

        // 4. Expand the tree_index's range by count
        // This is the commit/file being unfolded, so its range grows
        let mut prev_end = 0;
        if let Some((start, end)) = self.tree_index_to_line_range.get_mut(&tree_index) {
            prev_end = *end;
            *end += count;
            println!(
                "Incremented tree_index range: [{}, {}) -> [{}, {})",
                start.clone(),
                end.clone(),
                *start,
                *end
            );
        }

        // 5. Update reverse mappings for commits after the end of the original commit
        for (_, (start, end)) in self.tree_index_to_line_range.iter_mut() {
            // Skip the range that STRICTLY contains at_idx (start < at_idx < end)
            // Boundary positions (at_idx == start or at_idx == end) are shifted, not skipped
            if *start >= prev_end {
                *start += count;
                *end += count;
            }
        }

        // 6. Verify invariants (in debug mode)
        #[cfg(debug_assertions)]
        self.verify_invariants()?;

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
        count: usize,
        tree_index: usize,
    ) -> Result<(), MappingError> {
        // 1. Validate inputs
        let current_line_count = self.line_to_tree_pos.len();
        if start_idx + count > current_line_count {
            return Err(MappingError::InvariantViolation {
                which: format!(
                    "Removal range [{}, {}) exceeds line count {}",
                    start_idx,
                    start_idx + count,
                    current_line_count
                ),
            });
        }
        if count == 0 {
            return Err(MappingError::InvariantViolation {
                which: "Removal count must be greater than 0".to_string(),
            });
        }

        // 2. Validate that all removed lines are children of the specified tree_index
        for i in start_idx..start_idx + count {
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

        // 3. Remove forward mappings
        // Remove count elements starting at start_idx
        for _ in 0..count {
            self.line_to_tree_pos.remove(start_idx);
        }

        // 4. Update reverse mappings for commits with start_line >= start_idx + count
        // Decrement both start and end by count
        let removal_end = start_idx + count;
        let mut to_remove = None;
        for (idx, (start, end)) in self.tree_index_to_line_range.iter_mut() {
            if *idx == tree_index {
                // The collapsed commit's range shrinks
                if *start >= removal_end {
                    // Range is entirely after removal, shift it
                    *start -= count;
                    *end -= count;
                } else {
                    // Range contains the removal, shrink the end
                    *end -= count;
                }
                // If range becomes empty, mark for removal
                if *start == *end {
                    to_remove = Some(*idx);
                }
            } else if *start >= removal_end {
                // Other ranges entirely after removal get shifted
                *start -= count;
                *end -= count;
            }
        }

        // Remove empty tree indices
        if let Some(idx) = to_remove {
            self.tree_index_to_line_range.remove(&idx);
        }

        // 5. Verify invariants (in debug mode)
        #[cfg(debug_assertions)]
        self.verify_invariants()?;

        Ok(())
    }

    /// Notify that lines have been appended (e.g., when paginating more commits).
    ///
    /// # Arguments
    /// * `count` - number of lines appended
    /// * `tree_index` - tree index that the new lines belong to
    /// * `parent_tree_pos` - parent tree position (typically the commit being expanded)
    /// * `new_tree_index_to_line_range` - ranges for newly added tree indices
    pub fn notify_appended(
        &mut self,
        new_line_to_tree_pos: Vec<TreePosition>,
        new_tree_index_to_line_range: HashMap<usize, (usize, usize)>,
    ) -> Result<(), MappingError> {
        // 1. Validate inputs
        if new_line_to_tree_pos.len() == 0 {
            return Err(MappingError::InvariantViolation {
                which: "Append count must be greater than 0".to_string(),
            });
        }

        let start_line = self.line_to_tree_pos.len();
        // 2. Generate TreePositions for new lines
        self.line_to_tree_pos.extend(new_line_to_tree_pos);

        // 3. Merge new tree_index_to_line_range entries
        for (tree_index, (start, end)) in new_tree_index_to_line_range {
            // Validate that new ranges start at or after current line count
            if start < start_line {
                return Err(MappingError::InvariantViolation {
                    which: format!(
                        "New range for tree_index {} starts at {} but must be >= {}",
                        tree_index, start, start_line
                    ),
                });
            }
            self.tree_index_to_line_range
                .insert(tree_index, (start, end));
        }

        // 4. Verify invariants (in debug mode)
        #[cfg(debug_assertions)]
        self.verify_invariants()?;

        Ok(())
    }

    /// Debug helper to verify all invariants.
    #[cfg(debug_assertions)]
    fn verify_invariants(&self) -> Result<(), MappingError> {
        // Validate invariant 1: Mapping Consistency
        for (line_idx, tree_pos) in self.line_to_tree_pos.iter().enumerate() {
            let tree_index = tree_pos[0];
            let (start, end) = self
                .tree_index_to_line_range
                .get(&tree_index)
                .ok_or_else(|| MappingError::InvariantViolation {
                    which: format!(
                        "Invariant check failed: No line range for tree_index {} at line {}",
                        tree_index, line_idx
                    ),
                })?;

            if line_idx < *start || line_idx >= *end {
                return Err(MappingError::InvariantViolation {
                    which: format!(
                        "Invariant check failed: line {} (tree_index {}) is not in range [{}, {})",
                        line_idx, tree_index, start, end
                    ),
                });
            }
        }

        // Validate invariant 2: Commit Range Validity
        for (tree_index, (start, end)) in &self.tree_index_to_line_range {
            if start >= end {
                return Err(MappingError::InvariantViolation {
                    which: format!(
                        "Invariant check failed: Tree index {} has invalid range: start ({}) >= end ({})",
                        tree_index, start, end
                    ),
                });
            }
        }

        Ok(())
    }
}
