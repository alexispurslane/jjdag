use crate::{
    command_tree::{CommandTree, display_unbound_error_lines},
    log_tree::{
        DIFF_HUNK_LINE_IDX, JjLog, LogTreeNode, TreePosition, get_parent_tree_position, strip_ansi,
    },
    shell_out::{JjCommand, JjCommandError},
    terminal::Term,
    update::{
        AbandonMode, AbsorbMode, BookmarkMoveMode, DuplicateDestination, DuplicateDestinationType,
        EditMode, GitFetchMode, GitPushMode, InterdiffMode, Message, MetaeditAction, NewMode,
        NextPrevDirection, NextPrevMode, ParallelizeSource, RebaseDestination,
        RebaseDestinationType, RebaseSourceType, RestoreMode, RevertDestination,
        RevertDestinationType, RevertRevision, SignAction, SimplifyParentsMode, SquashMode,
        TextPromptAction, ViewMode,
    },
};
use ansi_to_tui::IntoText;
use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::{
    layout::Rect,
    text::{Line, Text},
    widgets::ListState,
};

/// Simple debug logging to file
fn debug_log(msg: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::time::SystemTime;

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/jjdag.log")
    {
        let _ = writeln!(file, "[{}] {}", timestamp, msg);
    }
}

const LOG_LIST_SCROLL_PADDING: usize = 0;

#[derive(Default, Debug, PartialEq, Eq)]
pub enum State {
    #[default]
    Running,
    Quit,
}

#[derive(Debug, Clone)]
pub struct GlobalArgs {
    pub repository: String,
    pub ignore_immutable: bool,
}

#[derive(Debug)]
pub struct Model {
    pub global_args: GlobalArgs,
    pub display_repository: String,
    pub revset: String,
    pub state: State,
    pub command_tree: CommandTree,
    command_keys: Vec<KeyCode>,
    queued_jj_commands: Vec<JjCommand>,
    accumulated_command_output: Vec<Line<'static>>,
    saved_change_id: Option<String>,
    saved_file_path: Option<String>,
    saved_tree_position: Option<TreePosition>,
    jj_log: JjLog,
    pub log_list: Vec<Text<'static>>,
    pub log_list_state: ListState,
    log_list_tree_positions: Vec<TreePosition>,
    pub log_list_layout: Rect,
    pub log_list_scroll_padding: usize,
    pub info_list: Option<Text<'static>>,
    /// Current fuzzy searchable popup for selection lists
    pub current_popup: Option<crate::update::Popup>,
    /// Where text input is currently active (source of truth)
    pub text_input_location: crate::update::TextInputLocation,
    /// Filter text for fuzzy searching in popups
    pub popup_filter: String,
    /// Selected index in the current popup's filtered list
    pub popup_selection: usize,
    /// Text input buffer for text prompt popups
    pub text_input: String,
    /// Cursor position in text input (byte index)
    pub text_cursor: usize,
    /// Track last click for double-click detection
    last_click_time: Option<std::time::Instant>,
    last_click_pos: Option<(u16, u16)>,
}

#[derive(Debug)]
enum ScrollDirection {
    Up,
    Down,
}

impl Model {
    pub fn new(repository: String, revset: String) -> Result<Self> {
        let mut model = Self {
            state: State::default(),
            command_tree: CommandTree::new(),
            command_keys: Vec::new(),
            queued_jj_commands: Vec::new(),
            accumulated_command_output: Vec::new(),
            saved_tree_position: None,
            saved_change_id: None,
            saved_file_path: None,
            jj_log: JjLog::new()?,
            log_list: Vec::new(),
            log_list_state: ListState::default(),
            log_list_tree_positions: Vec::new(),
            log_list_layout: Rect::ZERO,
            log_list_scroll_padding: LOG_LIST_SCROLL_PADDING,
            info_list: None,
            current_popup: None,
            text_input_location: crate::update::TextInputLocation::None,
            popup_filter: String::new(),
            popup_selection: 0,
            text_input: String::new(),
            text_cursor: 0,
            last_click_time: None,
            last_click_pos: None,
            display_repository: format_repository_for_display(&repository),
            global_args: GlobalArgs {
                repository,
                ignore_immutable: false,
            },
            revset,
        };

        model.sync()?;
        Ok(model)
    }

    pub fn quit(&mut self) {
        self.state = State::Quit;
    }

    fn reset_log_list_selection(&mut self) -> Result<()> {
        // Start with @ selected and unfolded
        let list_idx = match self.jj_log.get_current_commit() {
            None => 0,
            Some(commit) => commit.flat_log_idx,
        };
        self.log_select(list_idx);
        self.toggle_current_fold()
    }

    pub fn sync(&mut self) -> Result<()> {
        self.jj_log.load_log_tree(&self.global_args, &self.revset)?;
        self.sync_log_list()?;
        self.reset_log_list_selection()?;
        Ok(())
    }

    fn sync_log_list(&mut self) -> Result<()> {
        (self.log_list, self.log_list_tree_positions) = self.jj_log.flatten_log()?;
        Ok(())
    }

    pub fn refresh(&mut self) -> Result<()> {
        // Add periods for visual feedback on repeated refreshes
        let periods = self
            .info_list
            .as_ref()
            .map(|t| t.to_string())
            .filter(|s| s.starts_with("Refreshed"))
            .map_or(0, |s| s.matches('.').count() + 3);
        self.clear();
        self.sync()?;
        self.info_list = Some(format!("Refreshed{}", ".".repeat(periods)).into());
        Ok(())
    }

    pub fn toggle_ignore_immutable(&mut self) {
        self.global_args.ignore_immutable = !self.global_args.ignore_immutable;
    }

    fn log_offset(&self) -> usize {
        self.log_list_state.offset()
    }

    fn log_selected(&self) -> usize {
        self.log_list_state.selected().unwrap()
    }

    fn log_select(&mut self, idx: usize) {
        self.log_list_state.select(Some(idx));
    }

    fn get_selected_tree_position(&self) -> TreePosition {
        self.log_list_tree_positions[self.log_selected()].clone()
    }

    fn get_selected_change_id(&self) -> Option<&str> {
        let tree_pos = self.get_selected_tree_position();
        self.get_change_id(tree_pos)
    }

    fn get_saved_change_id(&self) -> Option<&str> {
        self.saved_change_id.as_deref()
    }

    fn get_change_id(&self, tree_pos: TreePosition) -> Option<&str> {
        match self.jj_log.get_tree_commit(&tree_pos) {
            None => None,
            Some(commit) => Some(&commit.change_id),
        }
    }

    fn get_selected_file_path(&self) -> Option<&str> {
        let tree_pos = self.get_selected_tree_position();
        self.get_file_path(tree_pos)
    }

    fn get_saved_file_path(&self) -> Option<&str> {
        self.saved_file_path.as_deref()
    }

    fn get_file_path(&self, tree_pos: TreePosition) -> Option<&str> {
        match self.jj_log.get_tree_file_diff(&tree_pos) {
            None => None,
            Some(file_diff) => Some(&file_diff.path),
        }
    }

    pub fn get_saved_selection_flat_log_idxs(&self) -> (Option<usize>, Option<usize>) {
        let Some(saved_tree_position) = self.saved_tree_position.as_ref() else {
            return (None, None);
        };

        let commit_idx = self
            .jj_log
            .get_tree_commit(saved_tree_position)
            .map(|commit| commit.flat_log_idx);
        let file_diff_idx = self
            .jj_log
            .get_tree_file_diff(saved_tree_position)
            .map(|file_diff| file_diff.flat_log_idx());

        (commit_idx, file_diff_idx)
    }

    fn is_selected_working_copy(&self) -> bool {
        let tree_pos = self.get_selected_tree_position();
        match self.jj_log.get_tree_commit(&tree_pos) {
            None => false,
            Some(commit) => commit.current_working_copy,
        }
    }

    pub fn select_next_node(&mut self) {
        if self.log_list_state.selected().unwrap() < self.log_list.len() - 1 {
            self.log_list_state.select_next();
        }
    }

    pub fn select_prev_node(&mut self) {
        if self.log_list_state.selected().unwrap() > 0 {
            self.log_list_state.select_previous();
        }
    }

    pub fn select_current_working_copy(&mut self) {
        if let Some(commit) = self.jj_log.get_current_commit() {
            self.log_select(commit.flat_log_idx);
        }
    }

    pub fn select_parent_node(&mut self) -> Result<()> {
        let tree_pos = self.get_selected_tree_position();
        if let Some(parent_pos) = get_parent_tree_position(&tree_pos) {
            let parent_node_idx = self.jj_log.get_tree_node(&parent_pos)?.flat_log_idx();
            self.log_select(parent_node_idx);
        }
        Ok(())
    }

    pub fn select_current_next_sibling_node(&mut self) -> Result<()> {
        let tree_pos = self.get_selected_tree_position();
        self.select_next_sibling_node(tree_pos)
    }

    fn select_next_sibling_node(&mut self, tree_pos: TreePosition) -> Result<()> {
        let mut tree_pos = tree_pos;
        if tree_pos.len() == DIFF_HUNK_LINE_IDX + 1 {
            tree_pos = get_parent_tree_position(&tree_pos).unwrap();
        }
        let idx = tree_pos[tree_pos.len() - 1];

        match get_parent_tree_position(&tree_pos) {
            Some(parent_pos) => {
                let parent_node = self.jj_log.get_tree_node(&parent_pos)?;
                let children = parent_node.children();

                if idx == children.len() - 1 {
                    self.select_next_sibling_node(parent_pos)?;
                } else {
                    let sibling_idx = (idx + 1).min(children.len() - 1);
                    self.log_list_state
                        .select(Some(children[sibling_idx].flat_log_idx()));
                }
            }
            None => {
                let sibling_idx = (idx + 1).min(self.jj_log.log_tree.len() - 1);
                self.log_list_state
                    .select(Some(self.jj_log.log_tree[sibling_idx].flat_log_idx()));
            }
        };

        Ok(())
    }

    pub fn select_current_prev_sibling_node(&mut self) -> Result<()> {
        let tree_pos = self.get_selected_tree_position();
        self.select_prev_sibling_node(tree_pos)
    }

    fn select_prev_sibling_node(&mut self, tree_pos: TreePosition) -> Result<()> {
        if tree_pos.len() == DIFF_HUNK_LINE_IDX + 1 {
            let parent_pos = get_parent_tree_position(&tree_pos).unwrap();
            let parent_node_idx = self.jj_log.get_tree_node(&parent_pos)?.flat_log_idx();
            self.log_select(parent_node_idx);
            return Ok(());
        }
        let idx = tree_pos[tree_pos.len() - 1];

        match get_parent_tree_position(&tree_pos) {
            Some(parent_pos) => {
                let parent_node = self.jj_log.get_tree_node(&parent_pos)?;
                let children = parent_node.children();

                if idx == 0 {
                    let parent_node_idx = parent_node.flat_log_idx();
                    self.log_select(parent_node_idx);
                } else {
                    let sibling_idx = idx - 1;
                    self.log_list_state
                        .select(Some(children[sibling_idx].flat_log_idx()));
                }
            }
            None => {
                let sibling_idx = idx.saturating_sub(1);
                self.log_list_state
                    .select(Some(self.jj_log.log_tree[sibling_idx].flat_log_idx()));
            }
        };

        Ok(())
    }

    pub fn toggle_current_fold(&mut self) -> Result<()> {
        let tree_pos = self.get_selected_tree_position();
        let log_list_selected_idx = self.jj_log.toggle_fold(&self.global_args, &tree_pos)?;
        self.sync_log_list()?;
        self.log_select(log_list_selected_idx);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.info_list = None;
        self.saved_tree_position = None;
        self.saved_change_id = None;
        self.saved_file_path = None;
        self.command_keys.clear();
        self.queued_jj_commands.clear();
        self.accumulated_command_output.clear();
    }

    /// User cancelled an action (e.g., closed editor without entering input).
    /// The command key sequence is automatically cleared by `handle_command_key`
    /// when the action is triggered, so we don't need to clear it here.
    fn cancelled(&mut self) -> Result<()> {
        self.info_list = Some(Text::from("Cancelled"));
        Ok(())
    }

    /// The selected or saved change is invalid for this operation (e.g., no
    /// change selected, or the saved selection from a two-step command is missing).
    /// The command key sequence is automatically cleared by `handle_command_key`
    /// when the action is triggered, so we don't need to clear it here.
    fn invalid_selection(&mut self) -> Result<()> {
        self.info_list = Some(Text::from("Invalid selection"));
        Ok(())
    }

    fn display_error_lines(&mut self, err: &anyhow::Error) {
        self.info_list = Some(err.to_string().into_text().unwrap());
    }

    pub fn set_revset(&mut self, _term: Term) -> Result<()> {
        // Enter inline revset editing mode
        self.text_input_location = crate::update::TextInputLocation::Revset {
            original: self.revset.clone(),
        };
        self.text_input = self.revset.clone();
        self.text_cursor = self.text_input.len();
        Ok(())
    }

    /// Submit new revset
    pub fn revset_edit_submit(&mut self) -> Result<()> {
        let new_revset = std::mem::take(&mut self.text_input);
        self.text_cursor = 0;

        let old_revset = match &self.text_input_location {
            crate::update::TextInputLocation::Revset { original } => original.clone(),
            _ => self.revset.clone(),
        };
        self.text_input_location = crate::update::TextInputLocation::None;
        self.revset = new_revset.clone();

        match self.sync() {
            Err(err) => {
                self.display_error_lines(&err);
                self.revset = old_revset;
            }
            Ok(()) => {
                self.info_list = Some(Text::from(format!("Revset set to '{}'", self.revset)));
            }
        }
        Ok(())
    }

    pub fn show_help(&mut self) {
        self.info_list = Some(self.command_tree.get_help());
    }

    pub fn handle_command_key(&mut self, key_code: KeyCode) -> Option<Message> {
        self.command_keys.push(key_code);

        let node = match self.command_tree.get_node(&self.command_keys) {
            None => {
                self.command_keys.pop();
                display_unbound_error_lines(&mut self.info_list, &key_code);
                return None;
            }
            Some(node) => node,
        };
        if let Some(children) = &node.children {
            self.info_list = Some(children.get_help());
        }
        if let Some(message) = node.action {
            if node.children.is_none() {
                self.command_keys.clear();
            }
            return Some(message);
        }
        None
    }

    /// Returns true if there are pending command keys in a multi-key sequence
    pub fn has_pending_command_keys(&self) -> bool {
        !self.command_keys.is_empty()
    }

    pub fn scroll_down_once(&mut self) {
        if self.log_selected() <= self.log_offset() + self.log_list_scroll_padding {
            self.select_next_node();
        }
        *self.log_list_state.offset_mut() = self.log_offset() + 1;
    }

    pub fn scroll_up_once(&mut self) {
        if self.log_offset() == 0 {
            return;
        }
        let last_node_visible = self.line_dist_to_dest_node(
            self.log_list_layout.height as usize - 1,
            self.log_offset(),
            &ScrollDirection::Down,
        );
        if self.log_selected() >= last_node_visible - 1 - self.log_list_scroll_padding {
            self.select_prev_node();
        }
        *self.log_list_state.offset_mut() = self.log_offset().saturating_sub(1);
    }

    pub fn scroll_down_page(&mut self) {
        self.scroll_lines(self.log_list_layout.height as usize, &ScrollDirection::Down);
    }

    pub fn scroll_up_page(&mut self) {
        self.scroll_lines(self.log_list_layout.height as usize, &ScrollDirection::Up);
    }

    fn scroll_lines(&mut self, num_lines: usize, direction: &ScrollDirection) {
        let selected_node_dist_from_offset = self.log_selected() - self.log_offset();
        let mut target_offset =
            self.line_dist_to_dest_node(num_lines, self.log_offset(), direction);
        let mut target_node = target_offset + selected_node_dist_from_offset;
        match direction {
            ScrollDirection::Down => {
                if target_offset == self.log_list.len() - 1 {
                    target_node = target_offset;
                    target_offset = self.log_offset();
                }
            }
            ScrollDirection::Up => {
                // If we're already at the top of the page, then move selection to the top as well
                if target_offset == 0 && target_offset == self.log_offset() {
                    target_node = 0;
                }
            }
        }
        self.log_select(target_node);
        *self.log_list_state.offset_mut() = target_offset;
    }

    pub fn handle_mouse_click(&mut self, row: u16, column: u16) {
        use std::time::{Duration, Instant};

        const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(300);

        // Check for double-click
        let is_double_click = if let Some(last_time) = self.last_click_time {
            let elapsed = Instant::now().duration_since(last_time);
            let pos_matches = self.last_click_pos == Some((row, column));
            elapsed < DOUBLE_CLICK_THRESHOLD && pos_matches
        } else {
            false
        };

        // Update last click tracking
        self.last_click_time = Some(Instant::now());
        self.last_click_pos = Some((row, column));

        // Handle double-click - treat like Enter
        if is_double_click {
            let _ = self.enter_pressed();
            return;
        }

        let Rect {
            x,
            y,
            width,
            height,
        } = self.log_list_layout;

        // Check if inside log list
        if row < y || row >= y + height || column < x || column >= x + width {
            return;
        }

        let target_node = self.line_dist_to_dest_node(
            row as usize - y as usize,
            self.log_offset(),
            &ScrollDirection::Down,
        );
        self.log_select(target_node);
    }

    // Since some nodes contain multiple lines, we need a way to determine the destination node
    // which is n lines away from the starting node.
    fn line_dist_to_dest_node(
        &self,
        line_dist: usize,
        starting_node: usize,
        direction: &ScrollDirection,
    ) -> usize {
        let mut current_node = starting_node;
        let mut lines_traversed = 0;
        loop {
            let lines_in_node = self.log_list[current_node].lines.len();
            lines_traversed += lines_in_node;

            // Stop if we've found the dest node or have no further to traverse
            if match direction {
                ScrollDirection::Down => current_node == self.log_list.len() - 1,
                ScrollDirection::Up => current_node == 0,
            } || lines_traversed > line_dist
            {
                break;
            }

            match direction {
                ScrollDirection::Down => current_node += 1,
                ScrollDirection::Up => current_node -= 1,
            }
        }

        current_node
    }

    pub fn save_selection(&mut self) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            self.clear();
            return self.invalid_selection();
        };
        self.saved_change_id = Some(change_id.to_string());
        self.saved_file_path = self.get_selected_file_path().map(String::from);
        self.saved_tree_position = Some(self.get_selected_tree_position());

        Ok(())
    }

    pub fn jj_abandon(&mut self, mode: AbandonMode) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };
        let mode = match mode {
            AbandonMode::Default => None,
            AbandonMode::RetainBookmarks => Some("--retain-bookmarks"),
            AbandonMode::RestoreDescendants => Some("--restore-descendants"),
        };
        let cmd = JjCommand::abandon(change_id, mode, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_absorb(&mut self, mode: AbsorbMode) -> Result<()> {
        let (from_change_id, maybe_into_change_id, maybe_file_path) = match mode {
            AbsorbMode::Default => {
                let Some(from_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (from_change_id, None, self.get_selected_file_path())
            }
            AbsorbMode::Into => {
                let Some(from_change_id) = self.get_saved_change_id() else {
                    return self.invalid_selection();
                };
                let Some(into_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (
                    from_change_id,
                    Some(into_change_id),
                    self.get_saved_file_path(),
                )
            }
        };

        let cmd = JjCommand::absorb(
            from_change_id,
            maybe_into_change_id,
            maybe_file_path,
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    /// Start inline bookmark editing for the selected commit
    pub fn bookmark_edit_start(&mut self) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };
        let change_id = change_id.to_string();
        self.text_input.clear();
        self.text_cursor = 0;
        self.text_input_location = crate::update::TextInputLocation::Bookmark { change_id };
        Ok(())
    }

    /// Cancel bookmark editing
    pub fn bookmark_edit_cancel(&mut self) {
        self.text_input_location = crate::update::TextInputLocation::None;
        self.text_input.clear();
        self.text_cursor = 0;
    }

    /// Submit the bookmark creation from inline edit
    pub fn bookmark_edit_submit(&mut self, _term: Term) -> Result<()> {
        let change_id = match &self.text_input_location {
            crate::update::TextInputLocation::Bookmark { change_id } => change_id.clone(),
            _ => return Ok(()),
        };
        let bookmark_name = self.text_input.clone();
        self.bookmark_edit_cancel(); // Clear editing state first

        let cmd = JjCommand::bookmark_create(&bookmark_name, &change_id, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    // ===== Description Editing Methods =====

    /// Start inline description editing for the selected commit
    pub fn description_edit_start(&mut self, mode: crate::update::DescribeMode) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };
        let change_id = change_id.to_string();

        // Get the existing description to pre-fill
        let tree_pos = self.get_selected_tree_position();
        let existing_desc = self
            .jj_log
            .get_tree_commit(&tree_pos)
            .and_then(|c| c.description_first_line.clone())
            .unwrap_or_default();

        self.text_input = existing_desc;
        self.text_cursor = self.text_input.len();
        self.text_input_location =
            crate::update::TextInputLocation::Description { change_id, mode };
        Ok(())
    }

    /// Submit the description edit using jj describe
    pub fn description_edit_submit(&mut self, _term: Term) -> Result<()> {
        let (change_id, mode) = match &self.text_input_location {
            crate::update::TextInputLocation::Description { change_id, mode } => {
                (change_id.clone(), *mode)
            }
            _ => return Ok(()),
        };
        let message = self.text_input.clone();
        self.text_input_cancel(); // Clear editing state first

        let ignore_immutable = mode == crate::update::DescribeMode::IgnoreImmutable;
        let cmd = JjCommand::describe_with_message(
            &change_id,
            &message,
            ignore_immutable,
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    // ===== Popup Methods =====

    /// Open a fuzzy searchable popup
    pub fn open_popup(&mut self, popup: crate::update::Popup) -> Result<()> {
        self.current_popup = Some(popup);
        self.popup_filter = String::new();
        self.popup_selection = 0;
        Ok(())
    }

    /// Add a character to the popup filter
    pub fn popup_filter_char(&mut self, ch: char) {
        self.popup_filter.push(ch);
        self.popup_selection = 0; // Reset selection when filter changes
    }

    /// Remove last character from popup filter
    pub fn popup_filter_backspace(&mut self) {
        self.popup_filter.pop();
        self.popup_selection = 0; // Reset selection when filter changes
    }

    /// Move selection to next item in popup
    pub fn popup_next(&mut self) {
        if let Some(ref popup) = self.current_popup {
            let filtered_count = popup
                .items()
                .iter()
                .filter(|item| {
                    let filter_lower = self.popup_filter.to_lowercase();
                    let item_lower = item.to_lowercase();
                    filter_lower.is_empty() || item_lower.contains(&filter_lower)
                })
                .count();
            if self.popup_selection + 1 < filtered_count {
                self.popup_selection += 1;
            }
        }
    }

    /// Move selection to previous item in popup
    pub fn popup_prev(&mut self) {
        if self.popup_selection > 0 {
            self.popup_selection -= 1;
        }
    }

    /// Get the currently selected item from the popup
    fn get_popup_selection(&self) -> Option<String> {
        let popup = self.current_popup.as_ref()?;
        let filter_lower = self.popup_filter.to_lowercase();
        let filtered: Vec<&String> = popup
            .items()
            .iter()
            .filter(|item| {
                let item_lower = item.to_lowercase();
                filter_lower.is_empty() || item_lower.contains(&filter_lower)
            })
            .collect();
        filtered.get(self.popup_selection).map(|s| (*s).clone())
    }

    /// Confirm popup selection and execute the command
    pub fn popup_select(&mut self, _term: Term) -> Result<()> {
        let Some(selected) = self.get_popup_selection() else {
            self.popup_cancel();
            return Ok(());
        };

        // Take ownership of popup to avoid borrow issues
        let popup = self.current_popup.take().unwrap();
        self.popup_cancel(); // Clear state

        match popup {
            crate::update::Popup::BookmarkDelete { .. } => {
                let cmd = JjCommand::bookmark_delete(&selected, self.global_args.clone());
                self.queue_jj_command(cmd)
            }
            crate::update::Popup::BookmarkForget {
                include_remotes, ..
            } => {
                let cmd = JjCommand::bookmark_forget(
                    &selected,
                    include_remotes,
                    self.global_args.clone(),
                );
                self.queue_jj_command(cmd)
            }
            crate::update::Popup::BookmarkRenameSelect { .. } => {
                // Open text prompt for new bookmark name
                self.text_input.clear();
                self.text_cursor = 0;
                self.text_input_location = crate::update::TextInputLocation::Popup {
                    prompt: "Enter New Bookmark Name",
                    placeholder: "new-bookmark-name",
                    action: crate::update::TextPromptAction::BookmarkRenameSubmit {
                        old_name: selected,
                    },
                };
                Ok(())
            }
            crate::update::Popup::BookmarkSet { .. } => {
                if let Some(change_id) = self.get_selected_change_id() {
                    let cmd =
                        JjCommand::bookmark_set(&selected, change_id, self.global_args.clone());
                    self.queue_jj_command(cmd)
                } else {
                    self.invalid_selection()
                }
            }
            crate::update::Popup::BookmarkTrack { .. } => {
                let cmd = JjCommand::bookmark_track(&selected, self.global_args.clone());
                self.queue_jj_command(cmd)
            }
            crate::update::Popup::BookmarkUntrack { .. } => {
                let cmd = JjCommand::bookmark_untrack(&selected, self.global_args.clone());
                self.queue_jj_command(cmd)
            }
            crate::update::Popup::FileTrack { .. } => {
                let cmd = JjCommand::file_track(&selected, self.global_args.clone());
                self.queue_jj_command(cmd)
            }

            crate::update::Popup::GitFetchRemote {
                select_for_branches,
                ..
            } => {
                if select_for_branches {
                    // Fetch bookmarks/branches from this remote and show branch selection popup
                    let output = JjCommand::bookmark_list_with_args(
                        &["bookmark", "list", "--remote", &selected],
                        self.global_args.clone(),
                    )
                    .run()?;
                    let branches: Vec<String> = output
                        .lines()
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(|s| {
                            let clean = strip_ansi(s);
                            // Extract bookmark name: split by colon, then by whitespace
                            // to handle "bookmark-name (deleted): ..."
                            clean
                                .split(':')
                                .next()
                                .unwrap_or(&clean)
                                .trim()
                                .split_whitespace()
                                .next()
                                .unwrap_or(&clean)
                                .to_string()
                        })
                        .filter(|s| !s.is_empty())
                        .collect();

                    if branches.is_empty() {
                        self.info_list = Some(
                            format!("No branches found on remote '{}'", selected).into_text()?,
                        );
                        return Ok(());
                    }

                    let popup = crate::update::Popup::GitFetchRemoteBranches {
                        remote: selected,
                        branches,
                    };
                    self.open_popup(popup)
                } else {
                    // Fetch all from this remote
                    let cmd =
                        JjCommand::git_fetch_from_remote(&selected, None, self.global_args.clone());
                    self.queue_jj_command(cmd)
                }
            }
            crate::update::Popup::GitFetchRemoteBranches { remote, .. } => {
                // Fetch specific branch from specific remote
                let cmd = JjCommand::git_fetch_from_remote(
                    &remote,
                    Some(&selected),
                    self.global_args.clone(),
                );
                self.queue_jj_command(cmd)
            }
            crate::update::Popup::GitPushBookmark {
                change_id,
                is_named_mode,
                ..
            } => {
                if is_named_mode {
                    // Named mode: create bookmark at specific revision and push
                    let value = format!("{}={}", selected, change_id);
                    let cmd = JjCommand::git_push(
                        Some("--named"),
                        Some(&value),
                        self.global_args.clone(),
                    );
                    self.queue_jj_command(cmd)
                } else {
                    // Bookmark mode: push existing bookmark
                    let cmd =
                        JjCommand::git_push(Some("-b"), Some(&selected), self.global_args.clone());
                    self.queue_jj_command(cmd)
                }
            }
            crate::update::Popup::WorkspaceForget { .. } => {
                let cmd = JjCommand::workspace_forget(&selected, self.global_args.clone());
                self.queue_jj_command(cmd)
            }
            crate::update::Popup::WorkspaceUpdateStale { .. } => {
                // Run with --all flag to update all stale workspaces
                let cmd = JjCommand::workspace_update_stale(self.global_args.clone());
                self.queue_jj_command(cmd)
            }
        }
    }

    /// Cancel and close the popup
    pub fn popup_cancel(&mut self) {
        self.current_popup = None;
        self.popup_filter = String::new();
        self.popup_selection = 0;
    }

    // ===== Text Input Methods =====

    /// Insert a character at the current cursor position
    pub fn text_input_char(&mut self, ch: char) {
        if self.text_cursor > self.text_input.len() {
            self.text_cursor = self.text_input.len();
        }
        self.text_input.insert(self.text_cursor, ch);
        self.text_cursor += ch.len_utf8();
    }

    /// Delete character before cursor (backspace)
    pub fn text_input_backspace(&mut self) {
        if self.text_cursor > 0 {
            let char_len = self.text_input[..self.text_cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.text_cursor -= char_len;
            self.text_input.remove(self.text_cursor);
        }
    }

    /// Delete character at cursor
    pub fn text_input_delete(&mut self) {
        if self.text_cursor < self.text_input.len() {
            self.text_input.remove(self.text_cursor);
        }
    }

    /// Move cursor left
    pub fn text_input_move_left(&mut self) {
        if self.text_cursor > 0 {
            let char_len = self.text_input[..self.text_cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.text_cursor -= char_len;
        }
    }

    /// Move cursor right
    pub fn text_input_move_right(&mut self) {
        if self.text_cursor < self.text_input.len() {
            let char_len = self.text_input[self.text_cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.text_cursor += char_len;
        }
    }

    /// Move cursor to beginning
    pub fn text_input_move_home(&mut self) {
        self.text_cursor = 0;
    }

    /// Move cursor to end
    pub fn text_input_move_end(&mut self) {
        self.text_cursor = self.text_input.len();
    }

    /// Cancel text input and close popup
    pub fn text_input_cancel(&mut self) {
        self.text_input_location = crate::update::TextInputLocation::None;
        self.text_input.clear();
        self.text_cursor = 0;
    }

    /// Submit text input and execute the associated action based on location
    pub fn text_input_submit(&mut self, _term: Term) -> Result<()> {
        match &self.text_input_location {
            crate::update::TextInputLocation::Popup { action, .. } => {
                let action = action.clone();
                let text = std::mem::take(&mut self.text_input);
                self.text_cursor = 0;
                self.text_input_location = crate::update::TextInputLocation::None;

                match action {
                    TextPromptAction::BookmarkRenameSubmit { old_name } => {
                        self.bookmark_rename_submit(old_name, text)
                    }
                    TextPromptAction::MetaeditSetAuthor { change_id } => {
                        self.metaedit_set_author(change_id, text)
                    }
                    TextPromptAction::MetaeditSetTimestamp { change_id } => {
                        self.metaedit_set_timestamp(change_id, text)
                    }
                    TextPromptAction::ParallelizeRevset => self.parallelize_with_revset(text),
                    TextPromptAction::NextPrev { direction, mode } => {
                        self.next_prev_with_offset(direction, mode, text)
                    }
                    TextPromptAction::WorkspaceAdd => self.jj_workspace_add(&text, _term),
                    TextPromptAction::WorkspaceRenameSubmit => self.workspace_rename_submit(text),
                }
            }
            crate::update::TextInputLocation::Revset { .. } => self.revset_edit_submit(),
            crate::update::TextInputLocation::Bookmark { .. } => self.bookmark_edit_submit(_term),
            crate::update::TextInputLocation::Description { .. } => {
                self.description_edit_submit(_term)
            }
            _ => Ok(()),
        }
    }

    fn bookmark_rename_submit(&mut self, old_name: String, new_name: String) -> Result<()> {
        let cmd = JjCommand::bookmark_rename(&old_name, &new_name, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    fn metaedit_set_author(&mut self, change_id: String, author: String) -> Result<()> {
        let cmd = JjCommand::metaedit(
            &change_id,
            "--author",
            Some(&author),
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    fn metaedit_set_timestamp(&mut self, change_id: String, timestamp: String) -> Result<()> {
        let cmd = JjCommand::metaedit(
            &change_id,
            "--author-timestamp",
            Some(&timestamp),
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    fn parallelize_with_revset(&mut self, revset: String) -> Result<()> {
        let cmd = JjCommand::parallelize(&revset, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    fn next_prev_with_offset(
        &mut self,
        direction: NextPrevDirection,
        mode: NextPrevMode,
        offset: String,
    ) -> Result<()> {
        let mode_flag = match mode {
            NextPrevMode::Conflict => Some("--conflict"),
            NextPrevMode::Default => None,
            NextPrevMode::Edit => Some("--edit"),
            NextPrevMode::NoEdit => Some("--no-edit"),
        };

        let direction = match direction {
            NextPrevDirection::Next => "next",
            NextPrevDirection::Prev => "prev",
        };

        let cmd = JjCommand::next_prev(
            direction,
            mode_flag,
            Some(&offset),
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    pub fn jj_bookmark_delete(&mut self, _term: Term) -> Result<()> {
        // Fetch bookmarks and open popup
        let output = JjCommand::bookmark_list(self.global_args.clone()).run()?;
        let bookmarks: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                // Strip ANSI color codes from jj output
                let clean = strip_ansi(s);
                // Default format: "bookmark-name: commit-id description" or "bookmark-name (deleted): ..."
                // Extract just the bookmark name (before colon, then before whitespace)
                clean
                    .split(':')
                    .next()
                    .unwrap_or(&clean)
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or(&clean)
                    .to_string()
            })
            .collect();

        if bookmarks.is_empty() {
            self.info_list = Some("No bookmarks to delete".into_text()?);
            return Ok(());
        }

        let popup = crate::update::Popup::BookmarkDelete { bookmarks };
        self.open_popup(popup)
    }

    pub fn jj_bookmark_forget(&mut self, include_remotes: bool, _term: Term) -> Result<()> {
        // Fetch bookmarks and open popup
        let mut args = vec!["bookmark", "list", "-T", "name"];
        if include_remotes {
            args.push("--all-remotes");
        }
        let output = JjCommand::bookmark_list_with_args(&args, self.global_args.clone()).run()?;
        let bookmarks: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                let clean = strip_ansi(s);
                clean
                    .split(':')
                    .next()
                    .unwrap_or(&clean)
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or(&clean)
                    .to_string()
            })
            .collect();

        if bookmarks.is_empty() {
            let msg = if include_remotes {
                "No bookmarks to forget (including remotes)"
            } else {
                "No bookmarks to forget"
            };
            self.info_list = Some(msg.into_text()?);
            return Ok(());
        }

        let popup = crate::update::Popup::BookmarkForget {
            bookmarks,
            include_remotes,
        };
        self.open_popup(popup)
    }

    pub fn jj_bookmark_move(&mut self, mode: BookmarkMoveMode) -> Result<()> {
        let (from_change_id, to_change_id, allow_backwards) = match mode {
            BookmarkMoveMode::Default => {
                let Some(from_change_id) = self.get_saved_change_id() else {
                    return self.invalid_selection();
                };
                let Some(to_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (from_change_id, to_change_id, false)
            }
            BookmarkMoveMode::AllowBackwards => {
                let Some(from_change_id) = self.get_saved_change_id() else {
                    return self.invalid_selection();
                };
                let Some(to_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (from_change_id, to_change_id, true)
            }
            BookmarkMoveMode::Tug => {
                let Some(to_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                ("heads(::@- & bookmarks())", to_change_id, false)
            }
        };
        let cmd = JjCommand::bookmark_move(
            from_change_id,
            to_change_id,
            allow_backwards,
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    pub fn jj_bookmark_rename(&mut self, _term: Term) -> Result<()> {
        // Fetch bookmarks and open popup for selection
        let output = JjCommand::bookmark_list(self.global_args.clone()).run()?;
        let bookmarks: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                let clean = strip_ansi(s);
                clean
                    .split(':')
                    .next()
                    .unwrap_or(&clean)
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or(&clean)
                    .to_string()
            })
            .collect();

        if bookmarks.is_empty() {
            return Ok(());
        }

        let popup = crate::update::Popup::BookmarkRenameSelect { bookmarks };
        self.open_popup(popup)
    }

    pub fn jj_bookmark_set(&mut self, _term: Term) -> Result<()> {
        if self.get_selected_change_id().is_none() {
            return self.invalid_selection();
        }
        // Fetch bookmarks and open popup
        let output = JjCommand::bookmark_list(self.global_args.clone()).run()?;
        let bookmarks: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                let clean = strip_ansi(s);
                clean
                    .split(':')
                    .next()
                    .unwrap_or(&clean)
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or(&clean)
                    .to_string()
            })
            .collect();

        if bookmarks.is_empty() {
            self.info_list = Some("No bookmarks to set".into_text()?);
            return Ok(());
        }

        let popup = crate::update::Popup::BookmarkSet { bookmarks };
        self.open_popup(popup)
    }

    pub fn jj_bookmark_track(&mut self, _term: Term) -> Result<()> {
        // Fetch remote bookmarks and open popup
        let output = JjCommand::bookmark_list_with_args(
            &["bookmark", "list", "--all-remotes"],
            self.global_args.clone(),
        )
        .run()?;
        let remote_bookmarks: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                let clean = strip_ansi(s);
                clean
                    .split(':')
                    .next()
                    .unwrap_or(&clean)
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or(&clean)
                    .to_string()
            })
            .collect();

        if remote_bookmarks.is_empty() {
            self.info_list = Some("No remote bookmarks to track".into_text()?);
            return Ok(());
        }

        let popup = crate::update::Popup::BookmarkTrack { remote_bookmarks };
        self.open_popup(popup)
    }

    pub fn jj_bookmark_untrack(&mut self, _term: Term) -> Result<()> {
        // Fetch tracked remote bookmarks and open popup
        let output = JjCommand::bookmark_list_with_args(
            &["bookmark", "list", "--all-remotes"],
            self.global_args.clone(),
        )
        .run()?;
        let tracked_bookmarks: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                let clean = strip_ansi(s);
                clean
                    .split(':')
                    .next()
                    .unwrap_or(&clean)
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or(&clean)
                    .to_string()
            })
            .filter(|s| s.contains('@'))
            .collect();

        if tracked_bookmarks.is_empty() {
            self.info_list = Some("No tracked remote bookmarks to untrack".into_text()?);
            return Ok(());
        }

        let popup = crate::update::Popup::BookmarkUntrack { tracked_bookmarks };
        self.open_popup(popup)
    }

    pub fn jj_commit(&mut self, term: Term) -> Result<()> {
        let maybe_file_path = self.get_selected_file_path();
        let cmd = JjCommand::commit(maybe_file_path, self.global_args.clone(), term);
        self.queue_jj_command(cmd)
    }

    pub fn jj_duplicate(
        &mut self,
        destination_type: DuplicateDestinationType,
        destination: DuplicateDestination,
    ) -> Result<()> {
        let destination_type = match destination_type {
            DuplicateDestinationType::Default => None,
            DuplicateDestinationType::Onto => Some("--onto"),
            DuplicateDestinationType::InsertAfter => Some("--insert-after"),
            DuplicateDestinationType::InsertBefore => Some("--insert-before"),
        };

        let change_id = if destination_type.is_some() {
            let Some(change_id) = self.get_saved_change_id() else {
                return self.invalid_selection();
            };
            change_id
        } else {
            let Some(change_id) = self.get_selected_change_id() else {
                return self.invalid_selection();
            };
            change_id
        };

        let destination = match destination {
            DuplicateDestination::Default => None,
            DuplicateDestination::Selection => {
                let Some(dest_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                Some(dest_change_id)
            }
        };

        let cmd = JjCommand::duplicate(
            change_id,
            destination_type,
            destination,
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    pub fn jj_edit(&mut self, mode: EditMode) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };
        let ignore_immutable = mode == EditMode::IgnoreImmutable;
        let cmd = JjCommand::edit(change_id, ignore_immutable, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn enter_pressed(&mut self) -> Result<()> {
        let tree_pos = self.get_selected_tree_position();
        debug_log(&format!(
            "enter_pressed called, tree_pos.len() = {}",
            tree_pos.len()
        ));

        // If on a commit (revision title), edit that revision
        if tree_pos.len() == 1 {
            debug_log("On commit, calling jj_edit");
            return self.jj_edit(EditMode::Default);
        }

        // If on a diff line (tree_pos.len() == 4), get line number and parent file
        let (file_path, line_num) = if tree_pos.len() == 4 {
            debug_log("On diff line (len=4), getting line number");
            // Parse line number first (requires &mut self)
            let line_num = self.get_diff_line_number(&tree_pos);
            debug_log(&format!("Got line_num: {:?}", line_num));
            // Then get file path (requires &self)
            let file_tree_pos: TreePosition = tree_pos[..2].to_vec();
            let Some(path) = self.get_file_path(file_tree_pos) else {
                debug_log("Failed to get file path");
                return self.invalid_selection();
            };
            debug_log(&format!("Got file path: {}, line: {:?}", path, line_num));
            (path.to_string(), line_num)
        } else {
            // On a file or hunk header - no specific line
            let Some(path) = self.get_selected_file_path() else {
                debug_log("Failed to get selected file path");
                return self.invalid_selection();
            };
            debug_log(&format!("On file/hunk, path: {}", path));
            (path.to_string(), None)
        };

        debug_log(&format!(
            "Final: file_path={}, line_num={:?}",
            file_path, line_num
        ));

        // Get the change_id for this file's revision
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };

        // Open the file using jj cat piped to $EDITOR
        // For the working copy (@), we can open directly; otherwise use jj cat
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());

        // Parse editor command - handle cases like "code --wait" or "vim -u NONE"
        let mut editor_parts = editor.split_whitespace();
        let editor_bin = editor_parts.next().unwrap_or("vim");
        let editor_args: Vec<&str> = editor_parts.collect();

        // Build the file argument - include line number if available
        let file_arg = if let Some(num) = line_num {
            format!("{}:{}", file_path, num)
        } else {
            file_path.to_string()
        };

        if change_id == "@" || self.is_selected_working_copy() {
            debug_log(&format!("Opening working copy file: {}", file_arg));
            // Open working copy file directly - spawn and forget (non-blocking)
            let full_path = std::path::Path::new(&self.global_args.repository).join(&file_arg);
            std::process::Command::new(editor_bin)
                .args(&editor_args)
                .arg(&full_path)
                .spawn()?;
        } else {
            // For historical revisions, use jj cat and pipe to editor
            // Since many editors don't support piping directly, we'll use a tempfile approach
            let temp_file = tempfile::NamedTempFile::with_suffix(
                std::path::Path::new(&file_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(""),
            )?;
            let temp_path = temp_file.path().to_path_buf();

            // Get file content at this revision
            let output = std::process::Command::new("jj")
                .args([
                    "cat",
                    "--color=never",
                    "--repository",
                    &self.global_args.repository,
                    "-r",
                    change_id,
                    "--",
                    &file_path,
                ])
                .output()?;

            if !output.status.success() {
                return Err(anyhow::anyhow!(
                    "Failed to get file content: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }

            std::fs::write(&temp_path, &output.stdout)?;

            // Open the temp file in editor
            debug_log(&format!("Opening temp file: {}", temp_path.display()));
            std::process::Command::new(editor_bin)
                .args(&editor_args)
                .arg(&temp_path)
                .spawn()?;
        }

        Ok(())
    }

    /// Get the line number from a diff hunk line at the given tree position.
    /// Uses the LogTreeNode::line_number trait method.
    fn get_diff_line_number(&mut self, tree_pos: &TreePosition) -> Option<u32> {
        // Get the diff hunk line node and call line_number()
        let node = self.jj_log.get_tree_node(tree_pos).ok()?;
        node.line_number()
    }

    pub fn jj_evolog(&mut self, patch: bool, term: Term) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };
        let cmd = JjCommand::evolog(change_id, patch, self.global_args.clone(), term);
        self.queue_jj_command(cmd)
    }

    pub fn jj_file_track(&mut self, _term: Term) -> Result<()> {
        // Fetch untracked files and open popup
        let output = JjCommand::file_list_untracked(self.global_args.clone()).run()?;
        let untracked_files: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| strip_ansi(s).trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if untracked_files.is_empty() {
            self.info_list = Some("No untracked files to track".into_text()?);
            return Ok(());
        }

        let popup = crate::update::Popup::FileTrack { untracked_files };
        self.open_popup(popup)
    }

    pub fn jj_file_untrack(&mut self) -> Result<()> {
        let Some(file_path) = self.get_selected_file_path() else {
            return self.invalid_selection();
        };
        if !self.is_selected_working_copy() {
            return self.invalid_selection();
        }
        let cmd = JjCommand::file_untrack(file_path, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_git_fetch(&mut self, mode: GitFetchMode, _term: Term) -> Result<()> {
        match mode {
            GitFetchMode::Default => {
                let cmd = JjCommand::git_fetch(None, None, self.global_args.clone());
                self.queue_jj_command(cmd)
            }
            GitFetchMode::AllRemotes => {
                let cmd =
                    JjCommand::git_fetch(Some("--all-remotes"), None, self.global_args.clone());
                self.queue_jj_command(cmd)
            }
            GitFetchMode::Tracked => {
                let cmd = JjCommand::git_fetch(Some("--tracked"), None, self.global_args.clone());
                self.queue_jj_command(cmd)
            }
            GitFetchMode::Branch => {
                // Show remotes first, then we'll fetch branches from selected remote
                let output = JjCommand::git_remote_list(self.global_args.clone()).run()?;
                let remotes: Vec<String> = output
                    .lines()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        // jj git remote list outputs "origin git@github.com:..."
                        // We only want the remote name (first word)
                        strip_ansi(s)
                            .split_whitespace()
                            .next()
                            .unwrap_or(s)
                            .trim()
                            .to_string()
                    })
                    .filter(|s| !s.is_empty())
                    .collect();

                if remotes.is_empty() {
                    self.info_list = Some("No remotes configured".into_text()?);
                    return Ok(());
                }

                let popup = crate::update::Popup::GitFetchRemote {
                    remotes,
                    select_for_branches: true,
                };
                self.open_popup(popup)
            }
            GitFetchMode::Remote => {
                // Fetch remotes and show popup
                let output = JjCommand::git_remote_list(self.global_args.clone()).run()?;
                let remotes: Vec<String> = output
                    .lines()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        // jj git remote list outputs "origin git@github.com:..."
                        // We only want the remote name (first word)
                        strip_ansi(s)
                            .split_whitespace()
                            .next()
                            .unwrap_or(s)
                            .trim()
                            .to_string()
                    })
                    .filter(|s| !s.is_empty())
                    .collect();

                if remotes.is_empty() {
                    self.info_list = Some("No remotes configured".into_text()?);
                    return Ok(());
                }

                let popup = crate::update::Popup::GitFetchRemote {
                    remotes,
                    select_for_branches: false,
                };
                self.open_popup(popup)
            }
        }
    }

    pub fn jj_git_push(&mut self, mode: GitPushMode, _term: Term) -> Result<()> {
        let (flag, value) = match mode {
            GitPushMode::Default => (None, None),
            GitPushMode::All => (Some("--all"), None),
            GitPushMode::Tracked => (Some("--tracked"), None),
            GitPushMode::Deleted => (Some("--deleted"), None),
            GitPushMode::Revision => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (Some("-r"), Some(change_id.to_string()))
            }
            GitPushMode::Change => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (Some("-c"), Some(change_id.to_string()))
            }
            GitPushMode::Named => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                // Fetch bookmarks and open popup
                let output = JjCommand::bookmark_list(self.global_args.clone()).run()?;
                let bookmarks: Vec<String> = output
                    .lines()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        let clean = strip_ansi(s);
                        clean
                            .split(':')
                            .next()
                            .unwrap_or(&clean)
                            .trim()
                            .split_whitespace()
                            .next()
                            .unwrap_or(&clean)
                            .to_string()
                    })
                    .collect();

                if bookmarks.is_empty() {
                    self.info_list = Some("No bookmarks to push".into_text()?);
                    return Ok(());
                }

                let popup = crate::update::Popup::GitPushBookmark {
                    bookmarks,
                    change_id: change_id.to_string(),
                    is_named_mode: true,
                };
                return self.open_popup(popup);
            }
            GitPushMode::Bookmark => {
                // Fetch bookmarks and open popup
                let output = JjCommand::bookmark_list(self.global_args.clone()).run()?;
                let bookmarks: Vec<String> = output
                    .lines()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        let clean = strip_ansi(s);
                        clean
                            .split(':')
                            .next()
                            .unwrap_or(&clean)
                            .trim()
                            .split_whitespace()
                            .next()
                            .unwrap_or(&clean)
                            .to_string()
                    })
                    .collect();

                if bookmarks.is_empty() {
                    self.info_list = Some("No bookmarks to push".into_text()?);
                    return Ok(());
                }

                let popup = crate::update::Popup::GitPushBookmark {
                    bookmarks,
                    change_id: String::new(),
                    is_named_mode: false,
                };
                return self.open_popup(popup);
            }
        };
        let cmd = JjCommand::git_push(flag, value.as_deref(), self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_interdiff(&mut self, mode: InterdiffMode, term: Term) -> Result<()> {
        let (from, to, maybe_file_path) = match mode {
            InterdiffMode::FromSelection => {
                let Some(from_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (from_change_id, "@", self.get_selected_file_path())
            }
            InterdiffMode::FromSelectionToDestination => {
                let Some(from_change_id) = self.get_saved_change_id() else {
                    return self.invalid_selection();
                };
                let Some(to_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (from_change_id, to_change_id, self.get_saved_file_path())
            }
            InterdiffMode::ToSelection => {
                let Some(to_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                ("@", to_change_id, self.get_selected_file_path())
            }
        };

        let cmd = JjCommand::interdiff(from, to, maybe_file_path, self.global_args.clone(), term);
        self.queue_jj_command(cmd)
    }

    pub fn jj_metaedit(&mut self, action: MetaeditAction, _term: Term) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };

        match action {
            MetaeditAction::UpdateChangeId => {
                let cmd = JjCommand::metaedit(
                    change_id,
                    "--update-change-id",
                    None,
                    self.global_args.clone(),
                );
                self.queue_jj_command(cmd)
            }
            MetaeditAction::UpdateAuthorTimestamp => {
                let cmd = JjCommand::metaedit(
                    change_id,
                    "--update-author-timestamp",
                    None,
                    self.global_args.clone(),
                );
                self.queue_jj_command(cmd)
            }
            MetaeditAction::UpdateAuthor => {
                let cmd = JjCommand::metaedit(
                    change_id,
                    "--update-author",
                    None,
                    self.global_args.clone(),
                );
                self.queue_jj_command(cmd)
            }
            MetaeditAction::ForceRewrite => {
                let cmd = JjCommand::metaedit(
                    change_id,
                    "--force-rewrite",
                    None,
                    self.global_args.clone(),
                );
                self.queue_jj_command(cmd)
            }
            MetaeditAction::SetAuthor => {
                let change_id = change_id.to_string();
                self.text_input.clear();
                self.text_cursor = 0;
                self.text_input_location = crate::update::TextInputLocation::Popup {
                    prompt: "Set Author",
                    placeholder: "Name <email@example.com>",
                    action: crate::update::TextPromptAction::MetaeditSetAuthor { change_id },
                };
                Ok(())
            }
            MetaeditAction::SetAuthorTimestamp => {
                let change_id = change_id.to_string();
                self.text_input.clear();
                self.text_cursor = 0;
                self.text_input_location = crate::update::TextInputLocation::Popup {
                    prompt: "Set Author Timestamp",
                    placeholder: "2000-01-23T01:23:45-08:00",
                    action: crate::update::TextPromptAction::MetaeditSetTimestamp { change_id },
                };
                Ok(())
            }
        }
    }

    pub fn jj_new(&mut self, mode: NewMode) -> Result<()> {
        let cmd = match mode {
            NewMode::Default => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                JjCommand::new(change_id, &[], self.global_args.clone())
            }
            NewMode::AfterTrunk => JjCommand::new("trunk()", &[], self.global_args.clone()),
            NewMode::Before => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                JjCommand::new(
                    change_id,
                    &["--no-edit", "--insert-before"],
                    self.global_args.clone(),
                )
            }
            NewMode::InsertAfter => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                JjCommand::new(change_id, &["--insert-after"], self.global_args.clone())
            }
        };
        self.queue_jj_command(cmd)
    }

    pub fn jj_new_after_trunk_sync(&mut self) -> Result<()> {
        let fetch_cmd = JjCommand::git_fetch(None, None, self.global_args.clone());
        let new_cmd = JjCommand::new("trunk()", &[], self.global_args.clone());
        self.queue_jj_commands(vec![fetch_cmd, new_cmd])
    }

    pub fn jj_new_on_branch(&mut self) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };
        let new_cmd = JjCommand::new(change_id, &[], self.global_args.clone());
        let tug_cmd = JjCommand::tug(self.global_args.clone());
        self.queue_jj_commands(vec![new_cmd, tug_cmd])
    }

    pub fn jj_next_prev(
        &mut self,
        direction: NextPrevDirection,
        mode: NextPrevMode,
        offset: bool,
        _term: Term,
    ) -> Result<()> {
        if offset {
            self.text_input.clear();
            self.text_cursor = 0;
            self.text_input_location = crate::update::TextInputLocation::Popup {
                prompt: "Enter Offset",
                placeholder: "positive integer",
                action: crate::update::TextPromptAction::NextPrev { direction, mode },
            };
            Ok(())
        } else {
            let mode_flag = match mode {
                NextPrevMode::Conflict => Some("--conflict"),
                NextPrevMode::Default => None,
                NextPrevMode::Edit => Some("--edit"),
                NextPrevMode::NoEdit => Some("--no-edit"),
            };

            let direction = match direction {
                NextPrevDirection::Next => "next",
                NextPrevDirection::Prev => "prev",
            };
            let cmd = JjCommand::next_prev(direction, mode_flag, None, self.global_args.clone());
            self.queue_jj_command(cmd)
        }
    }

    pub fn jj_parallelize(&mut self, source: ParallelizeSource, _term: Term) -> Result<()> {
        match source {
            ParallelizeSource::Range => {
                let Some(from_change_id) = self.get_saved_change_id() else {
                    return self.invalid_selection();
                };
                let Some(to_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                let revset = format!("{}::{}", from_change_id, to_change_id);
                let cmd = JjCommand::parallelize(&revset, self.global_args.clone());
                self.queue_jj_command(cmd)
            }
            ParallelizeSource::Revset => {
                self.text_input.clear();
                self.text_cursor = 0;
                self.text_input_location = crate::update::TextInputLocation::Popup {
                    prompt: "Parallelize Revset",
                    placeholder: "Enter revset expression",
                    action: crate::update::TextPromptAction::ParallelizeRevset,
                };
                Ok(())
            }
            ParallelizeSource::Selection => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                let revset = format!("{}-::{}", change_id, change_id);
                let cmd = JjCommand::parallelize(&revset, self.global_args.clone());
                self.queue_jj_command(cmd)
            }
        }
    }

    pub fn jj_rebase(
        &mut self,
        source_type: RebaseSourceType,
        destination_type: RebaseDestinationType,
        destination: RebaseDestination,
    ) -> Result<()> {
        let Some(source_change_id) = self.get_saved_change_id() else {
            return self.invalid_selection();
        };
        let source_type = match source_type {
            RebaseSourceType::Branch => "--branch",
            RebaseSourceType::Source => "--source",
            RebaseSourceType::Revisions => "--revisions",
        };
        let destination_type = match destination_type {
            RebaseDestinationType::InsertAfter => "--insert-after",
            RebaseDestinationType::InsertBefore => "--insert-before",
            RebaseDestinationType::Onto => "--onto",
        };
        let destination = match destination {
            RebaseDestination::Selection => {
                let Some(dest_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                dest_change_id
            }
            RebaseDestination::Trunk => "trunk()",
            RebaseDestination::Current => "@",
        };

        let cmd = JjCommand::rebase(
            source_type,
            source_change_id,
            destination_type,
            destination,
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    pub fn jj_rebase_selected_branch_onto_trunk(&mut self) -> Result<()> {
        let Some(source_change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };

        let cmd = JjCommand::rebase(
            "--branch",
            source_change_id,
            "--onto",
            "trunk()",
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    pub fn jj_rebase_selected_branch_onto_trunk_sync(&mut self) -> Result<()> {
        let Some(source_change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };

        let fetch_cmd = JjCommand::git_fetch(None, None, self.global_args.clone());
        let rebase_cmd = JjCommand::rebase(
            "--branch",
            source_change_id,
            "--onto",
            "trunk()",
            self.global_args.clone(),
        );
        self.queue_jj_commands(vec![fetch_cmd, rebase_cmd])
    }

    pub fn jj_redo(&mut self) -> Result<()> {
        let cmd = JjCommand::redo(self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_restore(&mut self, mode: RestoreMode) -> Result<()> {
        let (flags, maybe_file_path) = match mode {
            RestoreMode::ChangesIn => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (
                    vec!["--changes-in", change_id],
                    self.get_selected_file_path(),
                )
            }
            RestoreMode::ChangesInRestoreDescendants => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (
                    vec!["--changes-in", change_id, "--restore-descendants"],
                    self.get_selected_file_path(),
                )
            }
            RestoreMode::From => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (vec!["--from", change_id], self.get_selected_file_path())
            }
            RestoreMode::Into => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (vec!["--into", change_id], self.get_selected_file_path())
            }
            RestoreMode::FromInto => {
                let Some(from_change_id) = self.get_saved_change_id() else {
                    return self.invalid_selection();
                };
                let Some(into_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                (
                    vec!["--from", from_change_id, "--into", into_change_id],
                    self.get_saved_file_path(),
                )
            }
        };

        let cmd = JjCommand::restore(&flags, maybe_file_path, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_revert(
        &mut self,
        revision: RevertRevision,
        destination_type: RevertDestinationType,
        destination: RevertDestination,
    ) -> Result<()> {
        let revision = match revision {
            RevertRevision::Saved => {
                let Some(revision) = self.get_saved_change_id() else {
                    return self.invalid_selection();
                };
                revision
            }
            RevertRevision::Selection => {
                let Some(revision) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                revision
            }
        };
        let destination_type = match destination_type {
            RevertDestinationType::Onto => "--onto",
            RevertDestinationType::InsertAfter => "--insert-after",
            RevertDestinationType::InsertBefore => "--insert-before",
        };
        let destination = match destination {
            RevertDestination::Current => "@",
            RevertDestination::Selection => {
                let Some(destination) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                destination
            }
        };

        let cmd = JjCommand::revert(
            revision,
            destination_type,
            destination,
            self.global_args.clone(),
        );
        self.queue_jj_command(cmd)
    }

    pub fn jj_resolve(&mut self, term: Term) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };
        let cmd = JjCommand::resolve(&change_id, self.global_args.clone(), term);
        self.queue_jj_command(cmd)
    }

    pub fn jj_sign(&mut self, action: SignAction, range: bool) -> Result<()> {
        let revset = if range {
            let Some(from_change_id) = self.get_saved_change_id() else {
                return self.invalid_selection();
            };
            let Some(to_change_id) = self.get_selected_change_id() else {
                return self.invalid_selection();
            };
            format!("{}::{}", from_change_id, to_change_id)
        } else {
            let Some(change_id) = self.get_selected_change_id() else {
                return self.invalid_selection();
            };
            change_id.to_string()
        };

        let action = match action {
            SignAction::Sign => "sign",
            SignAction::Unsign => "unsign",
        };
        let cmd = JjCommand::sign(action, &revset, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_simplify_parents(&mut self, mode: SimplifyParentsMode) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };
        let mode = match mode {
            SimplifyParentsMode::Revisions => "-r",
            SimplifyParentsMode::Source => "-s",
        };
        let cmd = JjCommand::simplify_parents(change_id, mode, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_split(&mut self, term: Term) -> Result<()> {
        let Some(change_id) = self.get_selected_change_id() else {
            return self.invalid_selection();
        };
        let cmd = JjCommand::split(change_id, "Split: part 1", self.global_args.clone(), term);
        self.queue_jj_command(cmd)
    }

    pub fn jj_tug(&mut self) -> Result<()> {
        let cmd = JjCommand::tug(self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_tug_and_git_push(&mut self) -> Result<()> {
        // Find bookmarks at the parent commit that will be tugged
        let output = JjCommand::bookmark_list_with_args(
            &[
                "bookmark",
                "list",
                "-r",
                "heads(::@- & bookmarks())",
                "-T",
                "name",
            ],
            self.global_args.clone(),
        )
        .run()?;

        let bookmarks: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        if bookmarks.is_empty() {
            self.info_list = Some("No bookmarks to tug and push".into_text()?);
            return Ok(());
        }

        // Queue tug command first
        let tug_cmd = JjCommand::tug(self.global_args.clone());

        // Then queue git push for each bookmark
        let mut cmds = vec![tug_cmd];
        for bookmark in &bookmarks {
            let push_cmd =
                JjCommand::git_push(Some("-b"), Some(bookmark), self.global_args.clone());
            cmds.push(push_cmd);
        }

        self.queue_jj_commands(cmds)
    }

    pub fn jj_squash(&mut self, mode: SquashMode, term: Term) -> Result<()> {
        let cmd = match mode {
            SquashMode::Default => {
                let tree_pos = self.get_selected_tree_position();
                let Some(commit) = self.jj_log.get_tree_commit(&tree_pos) else {
                    return self.invalid_selection();
                };
                let maybe_file_path = self.get_selected_file_path();

                if commit.description_first_line.is_none() {
                    JjCommand::squash_noninteractive(
                        &commit.change_id,
                        maybe_file_path,
                        self.global_args.clone(),
                    )
                } else {
                    JjCommand::squash_interactive(
                        &commit.change_id,
                        maybe_file_path,
                        self.global_args.clone(),
                        term,
                    )
                }
            }
            SquashMode::Into => {
                let Some(from_change_id) = self.get_saved_change_id() else {
                    return self.invalid_selection();
                };
                let maybe_file_path = self.get_saved_file_path();
                let Some(into_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                JjCommand::squash_into_interactive(
                    from_change_id,
                    into_change_id,
                    maybe_file_path,
                    self.global_args.clone(),
                    term,
                )
            }
        };

        self.queue_jj_command(cmd)
    }

    pub fn jj_status(&mut self, term: Term) -> Result<()> {
        let cmd = JjCommand::status(self.global_args.clone(), term);
        self.queue_jj_command(cmd)
    }

    pub fn jj_undo(&mut self) -> Result<()> {
        let cmd = JjCommand::undo(self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_view(&mut self, mode: ViewMode, term: Term) -> Result<()> {
        let cmd = match mode {
            ViewMode::Default => {
                let Some(change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                match self.get_selected_file_path() {
                    Some(file_path) => JjCommand::diff_file_interactive(
                        change_id,
                        file_path,
                        self.global_args.clone(),
                        term,
                    ),
                    None => JjCommand::show(change_id, self.global_args.clone(), term),
                }
            }
            ViewMode::FromSelection => {
                let Some(from_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                let file = self.get_selected_file_path();
                JjCommand::diff_from_to_interactive(
                    from_change_id,
                    "@",
                    file,
                    self.global_args.clone(),
                    term,
                )
            }
            ViewMode::FromSelectionToDestination => {
                let Some(from_change_id) = self.get_saved_change_id() else {
                    return self.invalid_selection();
                };
                let Some(to_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                let file = self.get_selected_file_path();
                JjCommand::diff_from_to_interactive(
                    from_change_id,
                    to_change_id,
                    file,
                    self.global_args.clone(),
                    term,
                )
            }
            ViewMode::FromTrunkToSelection => {
                let Some(to_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                let file = self.get_selected_file_path();
                JjCommand::diff_from_to_interactive(
                    "trunk()",
                    to_change_id,
                    file,
                    self.global_args.clone(),
                    term,
                )
            }
            ViewMode::ToSelection => {
                let Some(to_change_id) = self.get_selected_change_id() else {
                    return self.invalid_selection();
                };
                let file = self.get_selected_file_path();
                JjCommand::diff_from_to_interactive(
                    "@",
                    to_change_id,
                    file,
                    self.global_args.clone(),
                    term,
                )
            }
        };
        self.queue_jj_command(cmd)
    }

    pub fn jj_workspace_list(&mut self) -> Result<()> {
        let cmd = JjCommand::workspace_list(self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn jj_workspace_root(&mut self) -> Result<()> {
        let cmd = JjCommand::workspace_root(self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    pub fn workspace_add_start(&mut self) -> Result<()> {
        // Get parent directory of current repository to prefill
        let repo_path = std::path::Path::new(&self.global_args.repository);
        let parent_path = repo_path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        self.text_input = parent_path;
        self.text_cursor = self.text_input.len();
        self.text_input_location = crate::update::TextInputLocation::Popup {
            prompt: "Enter Workspace Path",
            placeholder: "/path/to/new-workspace",
            action: crate::update::TextPromptAction::WorkspaceAdd,
        };
        Ok(())
    }

    pub fn jj_workspace_add(&mut self, path: &str, term: Term) -> Result<()> {
        let cmd = JjCommand::workspace_add(path, self.global_args.clone(), term);
        self.queue_jj_command(cmd)
    }

    pub fn jj_workspace_forget(&mut self) -> Result<()> {
        // Fetch workspaces and open popup
        let output = JjCommand::workspace_list(self.global_args.clone()).run()?;
        let workspaces: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                // Parse "workspace-name: /path/to/workspace" format
                // Split by colon and take the first part (workspace name)
                s.split(':').next().unwrap_or(s).trim().to_string()
            })
            .collect();

        if workspaces.is_empty() {
            self.info_list = Some("No workspaces to forget".into_text()?);
            return Ok(());
        }

        let popup = crate::update::Popup::WorkspaceForget { workspaces };
        self.open_popup(popup)
    }

    pub fn jj_workspace_update_stale_start(&mut self) -> Result<()> {
        // Fetch workspaces and open popup
        let output = JjCommand::workspace_list(self.global_args.clone()).run()?;
        let workspaces: Vec<String> = output
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                // Parse "workspace-name: /path/to/workspace" format
                // Split by colon and take the first part (workspace name)
                s.split(':').next().unwrap_or(s).trim().to_string()
            })
            .collect();

        if workspaces.is_empty() {
            self.info_list = Some("No workspaces to update".into_text()?);
            return Ok(());
        }

        let popup = crate::update::Popup::WorkspaceUpdateStale { workspaces };
        self.open_popup(popup)
    }

    pub fn workspace_rename_current_start(&mut self) -> Result<()> {
        self.text_input.clear();
        self.text_cursor = 0;
        self.text_input_location = crate::update::TextInputLocation::Popup {
            prompt: "Enter New Workspace Name",
            placeholder: "new-workspace-name",
            action: crate::update::TextPromptAction::WorkspaceRenameSubmit,
        };
        Ok(())
    }

    fn workspace_rename_submit(&mut self, new_name: String) -> Result<()> {
        let cmd = JjCommand::workspace_rename(&new_name, self.global_args.clone());
        self.queue_jj_command(cmd)
    }

    fn queue_jj_command(&mut self, cmd: JjCommand) -> Result<()> {
        self.queue_jj_commands(vec![cmd])
    }

    fn queue_jj_commands(&mut self, cmds: Vec<JjCommand>) -> Result<()> {
        self.accumulated_command_output.clear();
        self.queued_jj_commands = cmds;
        self.update_info_list_for_queue();
        Ok(())
    }

    fn update_info_list_for_queue(&mut self) {
        let mut lines = self.accumulated_command_output.clone();
        if let Some(cmd) = self.queued_jj_commands.first() {
            lines.extend(cmd.to_lines());
            lines.push(Line::raw("Running..."));
        }
        self.info_list = Some(Text::from(lines));
    }

    pub fn process_jj_command_queue(&mut self) -> Result<()> {
        if self.queued_jj_commands.is_empty() {
            return Ok(());
        }

        let cmd = self.queued_jj_commands.remove(0);
        let result = cmd.run();

        // Accumulate output from this command (with blank line separator)
        if !self.accumulated_command_output.is_empty() {
            self.accumulated_command_output.push(Line::raw(""));
        }
        self.accumulated_command_output.extend(cmd.to_lines());

        match result {
            Ok(output) => {
                self.accumulated_command_output
                    .extend(output.into_text()?.lines);

                if self.queued_jj_commands.is_empty() {
                    // All commands done, show final output and sync
                    let final_output = self.accumulated_command_output.clone();
                    self.clear();
                    self.info_list = Some(Text::from(final_output));
                    if cmd.sync() {
                        self.sync()?;
                    }
                } else {
                    // More commands to run, update info_list to show next command
                    self.update_info_list_for_queue();
                }
            }
            Err(err) => match err {
                JjCommandError::Other { err } => return Err(err),
                JjCommandError::Failed { stderr } => {
                    // Command failed, show error with accumulated output
                    self.accumulated_command_output
                        .extend(stderr.into_text()?.lines);
                    let final_output = self.accumulated_command_output.clone();
                    self.clear();
                    self.info_list = Some(Text::from(final_output));
                }
            },
        }

        Ok(())
    }
}

fn format_repository_for_display(repository: &str) -> String {
    let Ok(home_dir) = std::env::var("HOME") else {
        return repository.to_string();
    };

    if repository == home_dir {
        return "~".to_string();
    }

    let home_prefix = format!("{home_dir}/");
    match repository.strip_prefix(&home_prefix) {
        Some(relative_path) => format!("~/{relative_path}"),
        None => repository.to_string(),
    }
}
