use crate::{model::Model, terminal::Term};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers, MouseEventKind};
use std::time::Duration;

const EVENT_POLL_DURATION: Duration = Duration::from_millis(200);

/// A fuzzy searchable popup for selecting from a list of options
#[derive(Debug, Clone)]
pub enum Popup {
    BookmarkDelete {
        bookmarks: Vec<String>,
    },
    BookmarkForget {
        bookmarks: Vec<String>,
        include_remotes: bool,
    },
    BookmarkRenameSelect {
        bookmarks: Vec<String>,
    },
    BookmarkSet {
        bookmarks: Vec<String>,
    },
    BookmarkTrack {
        remote_bookmarks: Vec<String>,
    },
    BookmarkUntrack {
        tracked_bookmarks: Vec<String>,
    },
    FileTrack {
        untracked_files: Vec<String>,
    },
    GitFetchRemote {
        remotes: Vec<String>,
        select_for_branches: bool,
    },
    GitFetchRemoteBranches {
        remote: String,
        branches: Vec<String>,
    },
    GitPushBookmark {
        bookmarks: Vec<String>,
        change_id: String,
        is_named_mode: bool,
    },
}

/// Action to take when text prompt is submitted
#[derive(Debug, Clone, PartialEq)]
pub enum TextPromptAction {
    BookmarkRenameSubmit {
        old_name: String,
    },
    MetaeditSetAuthor {
        change_id: String,
    },
    MetaeditSetTimestamp {
        change_id: String,
    },
    ParallelizeRevset,
    NextPrev {
        direction: NextPrevDirection,
        mode: NextPrevMode,
    },
}

/// Location where text input is currently active
#[derive(Debug, Clone, PartialEq)]
pub enum TextInputLocation {
    /// No text input active
    None,
    /// Text input in a popup prompt
    Popup {
        prompt: &'static str,
        placeholder: &'static str,
        action: TextPromptAction,
    },
    /// Inline revset editing in header
    Revset { original: String },
    /// Inline bookmark creation at selected commit
    Bookmark { change_id: String },
    /// Inline description editing at selected commit
    Description {
        change_id: String,
        mode: DescribeMode,
    },
}

impl Popup {
    /// Get the title to display in the popup
    pub fn title(&self) -> &'static str {
        match self {
            Popup::BookmarkDelete { .. } => "Delete Bookmark",
            Popup::BookmarkForget { .. } => "Forget Bookmark",
            Popup::BookmarkRenameSelect { .. } => "Select Bookmark to Rename",
            Popup::BookmarkSet { .. } => "Set Bookmark",
            Popup::BookmarkTrack { .. } => "Track Remote Bookmark",
            Popup::BookmarkUntrack { .. } => "Untrack Remote Bookmark",
            Popup::FileTrack { .. } => "Track File",
            Popup::GitFetchRemote { .. } => "Select Remote",
            Popup::GitFetchRemoteBranches { .. } => "Select Branch to Fetch",
            Popup::GitPushBookmark { .. } => "Select Bookmark to Push",
        }
    }

    /// Get the items to display in the popup
    pub fn items(&self) -> &[String] {
        match self {
            Popup::BookmarkDelete { bookmarks } => bookmarks,
            Popup::BookmarkForget { bookmarks, .. } => bookmarks,
            Popup::BookmarkRenameSelect { bookmarks } => bookmarks,
            Popup::BookmarkSet { bookmarks } => bookmarks,
            Popup::BookmarkTrack { remote_bookmarks } => remote_bookmarks,
            Popup::BookmarkUntrack { tracked_bookmarks } => tracked_bookmarks,
            Popup::FileTrack { untracked_files } => untracked_files,
            Popup::GitFetchRemote { remotes, .. } => remotes,
            Popup::GitFetchRemoteBranches { branches, .. } => branches,
            Popup::GitPushBookmark { bookmarks, .. } => bookmarks,
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Message {
    Abandon {
        mode: AbandonMode,
    },
    Absorb {
        mode: AbsorbMode,
    },
    BookmarkDelete,
    BookmarkForget {
        include_remotes: bool,
    },
    BookmarkMove {
        mode: BookmarkMoveMode,
    },
    BookmarkRename,
    BookmarkSet,
    BookmarkTrack,
    BookmarkUntrack,
    /// Start editing a bookmark name inline for the selected commit
    BookmarkEditStart,
    /// Start editing description inline for the selected commit
    DescriptionEditStart {
        mode: DescribeMode,
    },
    /// Add a character to the popup filter
    PopupFilterChar {
        ch: char,
    },
    /// Remove last character from popup filter
    PopupFilterBackspace,
    /// Select the currently highlighted popup item
    PopupSelect,
    /// Cancel the popup without selecting
    PopupCancel,
    /// Move selection down in popup
    PopupNext,
    /// Move selection up in popup
    PopupPrev,
    /// Add a character to the text input at cursor position
    TextInputChar {
        ch: char,
    },
    /// Delete character before cursor in text input
    TextInputBackspace,
    /// Delete character at cursor in text input
    TextInputDelete,
    /// Move cursor left in text input
    TextInputMoveLeft,
    /// Move cursor right in text input
    TextInputMoveRight,
    /// Move cursor to start of text input
    TextInputMoveHome,
    /// Move cursor to end of text input
    TextInputMoveEnd,
    /// Submit the text input
    TextInputSubmit,
    /// Cancel the text input
    TextInputCancel,
    Clear,
    Commit,

    Duplicate {
        destination_type: DuplicateDestinationType,
        destination: DuplicateDestination,
    },
    Edit,
    Evolog {
        patch: bool,
    },
    FileTrack,
    FileUntrack,
    GitFetch {
        mode: GitFetchMode,
    },
    GitPush {
        mode: GitPushMode,
    },
    Interdiff {
        mode: InterdiffMode,
    },
    LeftMouseClick {
        row: u16,
        column: u16,
    },
    Metaedit {
        action: MetaeditAction,
    },
    New {
        mode: NewMode,
    },
    NewAfterTrunkSync,
    RebaseSelectedBranchOntoTrunk,
    RebaseSelectedBranchOntoTrunkSync,
    NextPrev {
        direction: NextPrevDirection,
        mode: NextPrevMode,
        offset: bool,
    },
    Parallelize {
        source: ParallelizeSource,
    },
    Quit,
    Rebase {
        source_type: RebaseSourceType,
        destination_type: RebaseDestinationType,
        destination: RebaseDestination,
    },
    Redo,
    Refresh,
    Restore {
        mode: RestoreMode,
    },
    Revert {
        revision: RevertRevision,
        destination_type: RevertDestinationType,
        destination: RevertDestination,
    },
    RightMouseClick {
        row: u16,
        column: u16,
    },
    SaveSelection,
    ScrollDown,
    ScrollDownPage,
    ScrollUp,
    ScrollUpPage,
    SelectCurrentWorkingCopy,
    SelectNextNode,
    SelectNextSiblingNode,
    SelectParentNode,
    SelectPrevNode,
    SelectPrevSiblingNode,
    SetRevset,
    ShowHelp,
    Sign {
        action: SignAction,
        range: bool,
    },
    SimplifyParents {
        mode: SimplifyParentsMode,
    },
    Split,
    Squash {
        mode: SquashMode,
    },
    Status,
    /// Move the nearest bookmark ancestor to the current commit
    Tug,
    ToggleIgnoreImmutable,
    ToggleLogListFold,
    Undo,
    View {
        mode: ViewMode,
    },
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum AbandonMode {
    Default,
    RetainBookmarks,
    RestoreDescendants,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum AbsorbMode {
    Default,
    Into,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum BookmarkMoveMode {
    AllowBackwards,
    Default,
    Tug,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DuplicateDestination {
    Default,
    Selection,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DuplicateDestinationType {
    Default,
    InsertAfter,
    InsertBefore,
    Onto,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum GitFetchMode {
    Default,
    AllRemotes,
    Branch,
    Remote,
    Tracked,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum GitPushMode {
    Default,
    All,
    Bookmark,
    Change,
    Deleted,
    Named,
    Revision,
    Tracked,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum InterdiffMode {
    FromSelection,
    FromSelectionToDestination,
    ToSelection,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MetaeditAction {
    ForceRewrite,
    SetAuthor,
    SetAuthorTimestamp,
    UpdateAuthor,
    UpdateAuthorTimestamp,
    UpdateChangeId,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DescribeMode {
    Default,
    IgnoreImmutable,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum NewMode {
    AfterTrunk,
    Before,
    Default,
    InsertAfter,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum NextPrevDirection {
    Next,
    Prev,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum NextPrevMode {
    Conflict,
    Default,
    Edit,
    NoEdit,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ParallelizeSource {
    Range,
    Revset,
    Selection,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RebaseDestination {
    Current,
    Selection,
    Trunk,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RebaseDestinationType {
    InsertAfter,
    InsertBefore,
    Onto,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RebaseSourceType {
    Branch,
    Revisions,
    Source,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RestoreMode {
    ChangesIn,
    ChangesInRestoreDescendants,
    From,
    FromInto,
    Into,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RevertDestination {
    Current,
    Selection,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RevertDestinationType {
    InsertAfter,
    InsertBefore,
    Onto,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RevertRevision {
    Saved,
    Selection,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SignAction {
    Sign,
    Unsign,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SimplifyParentsMode {
    Revisions,
    Source,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SquashMode {
    Default,
    Into,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ViewMode {
    Default,
    FromSelection,
    FromSelectionToDestination,
    FromTrunkToSelection,
    ToSelection,
}

pub fn update(terminal: Term, model: &mut Model) -> Result<()> {
    model.process_jj_command_queue()?;

    let mut current_msg = handle_event(model)?;
    while let Some(msg) = current_msg {
        current_msg = handle_msg(terminal.clone(), model, msg)?;
    }

    Ok(())
}

fn handle_event(model: &mut Model) -> Result<Option<Message>> {
    if event::poll(EVENT_POLL_DURATION)? {
        match event::read()? {
            Event::Key(key) => {
                if key.kind == event::KeyEventKind::Press {
                    return Ok(handle_key(model, key));
                }
            }
            Event::Mouse(mouse) => {
                return Ok(handle_mouse(mouse));
            }
            _ => {}
        }
    }
    Ok(None)
}

fn handle_key(model: &mut Model, key: event::KeyEvent) -> Option<Message> {
    // When text input is active (single source of truth)
    // When text input is active, dispatch to unified TextInput messages
    if model.text_input_location != crate::update::TextInputLocation::None {
        return match key.code {
            KeyCode::Enter => Some(Message::TextInputSubmit),
            KeyCode::Esc => Some(Message::TextInputCancel),
            KeyCode::Backspace => Some(Message::TextInputBackspace),
            KeyCode::Delete => Some(Message::TextInputDelete),
            KeyCode::Left => Some(Message::TextInputMoveLeft),
            KeyCode::Right => Some(Message::TextInputMoveRight),
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Message::TextInputMoveLeft)
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Message::TextInputMoveRight)
            }
            KeyCode::Home => Some(Message::TextInputMoveHome),
            KeyCode::End => Some(Message::TextInputMoveEnd),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Message::TextInputMoveHome)
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Message::TextInputMoveEnd)
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Message::TextInputDelete)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Message::TextInputBackspace)
            }
            KeyCode::Char(c) => Some(Message::TextInputChar { ch: c }),
            _ => None,
        };
    }

    // When a selection popup is active (not text input)
    if model.current_popup.is_some() {
        return match key.code {
            KeyCode::Enter => Some(Message::PopupSelect),
            KeyCode::Esc => Some(Message::PopupCancel),
            KeyCode::Backspace => Some(Message::PopupFilterBackspace),
            KeyCode::Down | KeyCode::Char('j') => Some(Message::PopupNext),
            KeyCode::Up | KeyCode::Char('k') => Some(Message::PopupPrev),
            KeyCode::Char(c) => Some(Message::PopupFilterChar { ch: c }),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('q') => Some(Message::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Message::Quit),
        KeyCode::Down | KeyCode::Char('j') => Some(Message::SelectNextNode),
        KeyCode::Up | KeyCode::Char('k') => Some(Message::SelectPrevNode),
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Message::SelectNextNode)
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Message::SelectPrevNode)
        }
        KeyCode::PageDown => Some(Message::ScrollDownPage),
        KeyCode::PageUp => Some(Message::ScrollUpPage),
        KeyCode::Left | KeyCode::Char('h') => Some(Message::SelectPrevSiblingNode),
        KeyCode::Right | KeyCode::Char('l') => Some(Message::SelectNextSiblingNode),
        KeyCode::Char('K') => Some(Message::SelectParentNode),
        KeyCode::Char(' ') => Some(Message::Refresh),
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Message::Refresh)
        }
        KeyCode::Tab => Some(Message::ToggleLogListFold),
        KeyCode::Esc => Some(Message::Clear),
        KeyCode::Char('@') => Some(Message::SelectCurrentWorkingCopy),
        KeyCode::Char('L') => Some(Message::SetRevset),
        KeyCode::Char('I') => Some(Message::ToggleIgnoreImmutable),
        KeyCode::Char('?') => Some(Message::ShowHelp),
        _ => model.handle_command_key(key.code),
    }
}

fn handle_mouse(mouse: event::MouseEvent) -> Option<Message> {
    match mouse.kind {
        MouseEventKind::ScrollDown => Some(Message::ScrollDown),
        MouseEventKind::ScrollUp => Some(Message::ScrollUp),
        MouseEventKind::Down(event::MouseButton::Left) => Some(Message::LeftMouseClick {
            row: mouse.row,
            column: mouse.column,
        }),
        MouseEventKind::Down(event::MouseButton::Right) => Some(Message::RightMouseClick {
            row: mouse.row,
            column: mouse.column,
        }),
        _ => None,
    }
}

fn handle_msg(term: Term, model: &mut Model, msg: Message) -> Result<Option<Message>> {
    match msg {
        // General
        Message::Clear => model.clear(),
        Message::Quit => model.quit(),
        Message::Refresh => model.refresh()?,
        Message::SetRevset => model.set_revset(term)?,

        Message::ShowHelp => model.show_help(),
        Message::ToggleIgnoreImmutable => model.toggle_ignore_immutable(),

        // Navigation
        Message::ScrollDownPage => model.scroll_down_page(),
        Message::ScrollUpPage => model.scroll_up_page(),
        Message::SelectCurrentWorkingCopy => model.select_current_working_copy(),
        Message::SelectNextNode => model.select_next_node(),
        Message::SelectNextSiblingNode => model.select_current_next_sibling_node()?,
        Message::SelectParentNode => model.select_parent_node()?,
        Message::SelectPrevNode => model.select_prev_node(),
        Message::SelectPrevSiblingNode => model.select_current_prev_sibling_node()?,
        Message::ToggleLogListFold => model.toggle_current_fold()?,

        // Mouse
        Message::LeftMouseClick { row, column } => model.handle_mouse_click(row, column),
        Message::RightMouseClick { row, column } => {
            model.handle_mouse_click(row, column);
            model.toggle_current_fold()?;
        }
        Message::ScrollDown => model.scroll_down_once(),
        Message::ScrollUp => model.scroll_up_once(),

        // Commands
        Message::Abandon { mode } => model.jj_abandon(mode)?,
        Message::Absorb { mode } => model.jj_absorb(mode)?,
        Message::BookmarkDelete => model.jj_bookmark_delete(term)?,
        Message::BookmarkForget { include_remotes } => {
            model.jj_bookmark_forget(include_remotes, term)?
        }
        Message::BookmarkMove { mode } => model.jj_bookmark_move(mode)?,
        Message::BookmarkRename => model.jj_bookmark_rename(term)?,
        Message::BookmarkSet => model.jj_bookmark_set(term)?,
        Message::BookmarkTrack => model.jj_bookmark_track(term)?,
        Message::BookmarkUntrack => model.jj_bookmark_untrack(term)?,
        // Bookmark editing
        Message::BookmarkEditStart => model.bookmark_edit_start()?,
        // Description editing
        Message::DescriptionEditStart { mode } => model.description_edit_start(mode)?,
        // Popup messages
        Message::PopupFilterChar { ch } => model.popup_filter_char(ch),
        Message::PopupFilterBackspace => model.popup_filter_backspace(),
        Message::PopupNext => model.popup_next(),
        Message::PopupPrev => model.popup_prev(),
        Message::PopupSelect => model.popup_select(term)?,
        Message::PopupCancel => model.popup_cancel(),
        // Text input messages
        Message::TextInputChar { ch } => model.text_input_char(ch),
        Message::TextInputBackspace => model.text_input_backspace(),
        Message::TextInputDelete => model.text_input_delete(),
        Message::TextInputMoveLeft => model.text_input_move_left(),
        Message::TextInputMoveRight => model.text_input_move_right(),
        Message::TextInputMoveHome => model.text_input_move_home(),
        Message::TextInputMoveEnd => model.text_input_move_end(),
        Message::TextInputSubmit => model.text_input_submit(term)?,
        Message::TextInputCancel => model.text_input_cancel(),
        Message::Commit => model.jj_commit(term)?,

        Message::Duplicate {
            destination_type,
            destination,
        } => model.jj_duplicate(destination_type, destination)?,
        Message::Edit => model.jj_edit()?,
        Message::Evolog { patch } => model.jj_evolog(patch, term)?,
        Message::FileTrack => model.jj_file_track(term)?,
        Message::FileUntrack => model.jj_file_untrack()?,
        Message::GitFetch { mode } => model.jj_git_fetch(mode, term)?,
        Message::GitPush { mode } => model.jj_git_push(mode, term)?,
        Message::Interdiff { mode } => model.jj_interdiff(mode, term)?,
        Message::Metaedit { action } => model.jj_metaedit(action, term)?,
        Message::New { mode } => model.jj_new(mode)?,
        Message::NewAfterTrunkSync => model.jj_new_after_trunk_sync()?,
        Message::RebaseSelectedBranchOntoTrunk => model.jj_rebase_selected_branch_onto_trunk()?,
        Message::RebaseSelectedBranchOntoTrunkSync => {
            model.jj_rebase_selected_branch_onto_trunk_sync()?
        }
        Message::NextPrev {
            direction,
            mode,
            offset,
        } => model.jj_next_prev(direction, mode, offset, term)?,
        Message::Parallelize { source } => model.jj_parallelize(source, term)?,
        Message::Rebase {
            source_type,
            destination_type,
            destination,
        } => model.jj_rebase(source_type, destination_type, destination)?,
        Message::Redo => model.jj_redo()?,
        Message::Restore { mode } => model.jj_restore(mode)?,
        Message::Revert {
            revision,
            destination_type,
            destination,
        } => model.jj_revert(revision, destination_type, destination)?,
        Message::SaveSelection => model.save_selection()?,
        Message::Sign { action, range } => model.jj_sign(action, range)?,
        Message::SimplifyParents { mode } => model.jj_simplify_parents(mode)?,
        Message::Split => model.jj_split(term)?,
        Message::Squash { mode } => model.jj_squash(mode, term)?,
        Message::Status => model.jj_status(term)?,
        Message::Tug => model.jj_tug()?,
        Message::Undo => model.jj_undo()?,
        Message::View { mode } => model.jj_view(mode, term)?,
    };

    Ok(None)
}
