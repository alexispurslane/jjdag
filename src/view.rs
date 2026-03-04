use crate::{log_tree::strip_ansi, model::Model};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, Paragraph},
};

pub const SELECTION_COLOR: Color = Color::Rgb(40, 42, 54);
pub const SAVED_SELECTION_COLOR: Color = Color::Rgb(33, 35, 45);

/// Standard style for normal text in input fields
pub const INPUT_STYLE: Style = Style::new().fg(Color::Yellow);
/// Style for text beyond column limits (grayed out)
pub const GRAYED_OUT_STYLE: Style = Style::new().fg(Color::DarkGray);

pub fn view(model: &mut Model, frame: &mut Frame) {
    let header = render_header(model);
    let log_list = render_log_list(model);
    let layout = render_layout(model, frame.area());
    frame.render_widget(header, layout[0]);
    frame.render_widget(log_list, layout[1]);
    model.log_list_layout = layout[1];

    // Render debug overlay if RUST_LOG=debug is set
    if log::log_enabled!(log::Level::Debug) {
        render_debug_overlay(model, frame, layout[1]);
    }

    if let Some(info_list) = render_info_list(model) {
        frame.render_widget(info_list, layout[2]);
    }
    if model.current_popup.is_some()
        || matches!(
            model.text_input_location,
            crate::update::TextInputLocation::Popup { .. }
        )
    {
        render_popup(model, frame, model.current_popup.as_ref(), frame.area());
    }

    // Set the terminal cursor position for text input
    if let Some((x, y)) = model.calculate_cursor_position() {
        frame.set_cursor_position(ratatui::layout::Position::new(x, y));
    }
}

fn render_layout(model: &Model, area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
            if let Some(info_list) = &model.info_list {
                Constraint::Length(info_list.lines.len() as u16 + 2)
            } else {
                Constraint::Length(0)
            },
        ])
        .split(area)
}

fn render_header(model: &Model) -> Paragraph<'_> {
    let mut header_spans = vec![
        Span::styled("repository: ", Style::default().fg(Color::Blue)),
        Span::styled(&model.display_repository, Style::default().fg(Color::Green)),
        Span::raw("  "),
        Span::styled("revset: ", Style::default().fg(Color::Blue)),
    ];

    if matches!(
        model.text_input_location,
        crate::update::TextInputLocation::Revset { .. }
    ) {
        // Show inline editing (real cursor is rendered via frame.set_cursor_position)
        header_spans.push(Span::styled(&model.text_input, INPUT_STYLE));
    } else {
        header_spans.push(Span::styled(
            &model.revset,
            Style::default().fg(Color::Green),
        ));
    }
    if model.global_args.ignore_immutable {
        header_spans.push(Span::styled(
            "  --ignore-immutable",
            Style::default().fg(Color::LightRed),
        ));
    }
    Paragraph::new(Line::from(header_spans))
}

fn render_log_list(model: &Model) -> List<'static> {
    // Slice display_lines using scroll offset
    let start = model.scroll_offset;
    let end = (start + model.log_list_layout.height as usize).min(model.display_lines.len());

    let mut log_items: Vec<Text<'static>> = model.display_lines[start..end].to_vec();

    // Adjust cursor for visible slice
    let visible_cursor = model.cursor.saturating_sub(start);

    inject_virtual_bookmark(model, &mut log_items);
    inject_virtual_description(model, &mut log_items);
    apply_saved_selection_highlights(model, &mut log_items, visible_cursor);

    // Create list with highlighted selection
    let items_with_selection: Vec<Text<'static>> = log_items
        .into_iter()
        .enumerate()
        .map(|(idx, mut text)| {
            if idx == visible_cursor {
                // Apply selection highlight to ALL spans in the line
                if let Some(line) = text.lines.first_mut() {
                    for span in &mut line.spans {
                        // Preserve original style but add selection background
                        span.style = span.style.patch(Style::new().bg(SELECTION_COLOR));
                    }
                }
                text
            } else {
                text
            }
        })
        .collect();

    List::new(items_with_selection)
}

/// When bookmark editing is active, inject the virtual bookmark into the selected commit's line.
/// Uses the mapping buffer to find the exact line where the commit starts (same logic as
/// calculate_bookmark_cursor_position), accounting for scroll offset internally.
/// The real cursor is rendered via terminal ANSI codes, not as fake text.
fn inject_virtual_bookmark(model: &Model, log_items: &mut [Text<'static>]) {
    let editing_change_id = match &model.text_input_location {
        crate::update::TextInputLocation::Bookmark { change_id } => change_id,
        _ => return,
    };

    // Use mapping buffer to find the exact line where the commit starts (same as cursor calculation)
    let buffer = match model.mapping_buffer.lock() {
        Ok(b) => b,
        _ => return,
    };

    // Get the tree position for the current cursor
    let Some(tree_pos) = buffer.get_tree_position(model.cursor).cloned() else {
        return;
    };

    // Get the display line where that commit starts
    let Some(containing_start_line) = buffer.get_exact_line_for_tree_position(&tree_pos) else {
        return;
    };

    // Calculate visible index (same logic as cursor calculation)
    let visible_idx = containing_start_line.saturating_sub(model.scroll_offset);

    // Check if this line is in the visible range
    let Some(text) = log_items.get_mut(visible_idx) else {
        return;
    };

    // Find the change_id in the selected line to verify this is the right commit
    let text_str = text.to_string();
    if !text_str.contains(&editing_change_id[..8]) {
        return;
    }

    // Create a new line with the virtual bookmark injected
    if let Some(first_line) = text.lines.first_mut() {
        // Add the bookmark text - real cursor is rendered via ANSI codes
        let style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);

        first_line.spans.push(Span::raw(" ["));
        first_line
            .spans
            .push(Span::styled(model.text_input.clone(), style));
        first_line.spans.push(Span::styled("]", style));
    }
}

/// Strip ANSI codes from all spans in a line
fn strip_ansi_from_line(line: &Line<'_>) -> Line<'static> {
    let clean_spans: Vec<Span<'static>> = line
        .spans
        .iter()
        .map(|span| {
            let clean_content = strip_ansi(&span.content);
            Span::styled(clean_content, span.style)
        })
        .collect();
    Line::from(clean_spans)
}

/// Render a single line of description with column limit styling.
/// The real cursor is rendered via terminal ANSI codes, not inserted text.
fn render_description_line(line_text: &str, line_idx: usize) -> Vec<Span<'static>> {
    let col_limit = if line_idx == 0 { 50 } else { 72 };

    if line_text.is_empty() {
        return Vec::new();
    }

    if line_text.len() <= col_limit {
        vec![Span::styled(line_text.to_string(), INPUT_STYLE)]
    } else {
        let (within, beyond) = line_text.split_at(col_limit);
        vec![
            Span::styled(within.to_string(), INPUT_STYLE),
            Span::styled(beyond.to_string(), GRAYED_OUT_STYLE),
        ]
    }
}

/// When description editing is active, replace the description line with the user's input.
/// The description is on the line AFTER the commit line (visible_idx + 1).
/// Multi-line descriptions are rendered as multiple Lines within one Text element.
fn inject_virtual_description(model: &Model, log_items: &mut [Text<'static>]) {
    let change_id = match &model.text_input_location {
        crate::update::TextInputLocation::Description { change_id, .. } => change_id,
        _ => return,
    };

    // Use mapping buffer to find the exact line where the commit starts
    let Ok(buffer) = model.mapping_buffer.lock() else {
        return;
    };
    let Some(tree_pos) = buffer.get_tree_position(model.cursor) else {
        return;
    };
    let Some(containing_start_line) = buffer.get_exact_line_for_tree_position(&tree_pos) else {
        return;
    };

    // Description line is one line after the commit line
    let commit_line_idx = containing_start_line.saturating_sub(model.scroll_offset);
    let desc_line_idx = (containing_start_line + 1).saturating_sub(model.scroll_offset);

    // Verify this is the right commit by checking change_id on the commit line first
    let Some(commit_text) = log_items.get(commit_line_idx) else {
        return;
    };
    let commit_str = commit_text.to_string();
    if !commit_str.contains(&change_id[..8]) {
        return;
    }

    // Now get mutable reference to the description line
    let Some(text) = log_items.get_mut(desc_line_idx) else {
        return;
    };

    // Get the input text (show placeholder if empty)
    let input_text = if model.text_input.is_empty() {
        "(no description set)".to_string()
    } else {
        strip_ansi(&model.text_input)
    };

    // Get the prefix from the existing description line (for graph indentation like "│  ")
    let prefix_content = if !text.lines.is_empty() && !text.lines[0].spans.is_empty() {
        let first_span = &text.lines[0].spans[0].content;
        // Extract just the graph characters (everything before the actual description)
        let prefix_end = first_span
            .char_indices()
            .find(|(_, c)| !c.is_whitespace() && *c != '│' && *c != '|')
            .map(|(i, _)| i)
            .unwrap_or(first_span.len());
        first_span[..prefix_end].to_string()
    } else {
        "  ".to_string()
    };

    // Split input into lines for multi-line rendering
    let input_lines: Vec<&str> = input_text.split('\n').collect();

    // Build multiple Lines for multi-line description
    // All lines get the prefix (graph indentation) except we use spaces for continuation
    let mut new_lines: Vec<Line<'static>> = Vec::new();
    let base_style = if model.text_input.is_empty() {
        Style::default().fg(Color::DarkGray)
    } else {
        INPUT_STYLE
    };

    for (idx, line_content) in input_lines.iter().enumerate() {
        // First line gets the full prefix with graph char, subsequent lines get spaces
        let prefix = if idx == 0 {
            prefix_content.clone()
        } else {
            // Replace graph chars with spaces for indentation alignment
            prefix_content
                .chars()
                .map(|c| {
                    if c.is_whitespace() || c == '│' || c == '|' {
                        ' '
                    } else {
                        c
                    }
                })
                .collect()
        };

        let spans = vec![
            Span::raw(prefix),
            Span::raw(" "),
            Span::styled(line_content.to_string(), base_style),
        ];
        new_lines.push(Line::from(spans));
    }

    // Replace the Text with a new one containing all the lines
    *text = Text::from(new_lines);
}

fn apply_saved_selection_highlights(
    model: &Model,
    log_items: &mut [Text<'static>],
    _visible_cursor: usize,
) {
    let (saved_commit_idx, saved_file_diff_idx) = model.get_saved_selection_flat_log_idxs();

    if let Some(idx) = saved_commit_idx
        && let Some(item) = log_items.get_mut(idx)
    {
        apply_saved_selection_highlight(item);
    }

    if let Some(idx) = saved_file_diff_idx
        && let Some(item) = log_items.get_mut(idx)
    {
        apply_saved_selection_highlight(item);
    }
}

fn apply_saved_selection_highlight(text: &mut Text<'static>) {
    text.style = text.style.bg(SAVED_SELECTION_COLOR);
    for line in &mut text.lines {
        for span in &mut line.spans {
            span.style = span.style.bg(SAVED_SELECTION_COLOR);
        }
    }
}

/// Render a centered popup for fuzzy selection
fn render_popup(
    model: &Model,
    frame: &mut Frame,
    popup: Option<&crate::update::Popup>,
    area: Rect,
) {
    use ratatui::widgets::{Clear, Wrap};

    // Handle text input popup separately
    if let crate::update::TextInputLocation::Popup {
        prompt,
        placeholder,
        ..
    } = &model.text_input_location
    {
        render_text_prompt_popup(model, frame, *prompt, *placeholder, area);
        return;
    }

    // For selection popups, we need a popup instance
    let Some(popup) = popup else {
        return;
    };

    // Calculate popup size
    let popup_width = (area.width * 2 / 3).min(60).max(40);
    let popup_height = (area.height * 2 / 3).min(20).max(10);
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the background behind the popup
    frame.render_widget(Clear, popup_area);

    // Get items and filter them
    let items = popup.items();
    let filtered_items: Vec<&String> = items
        .iter()
        .filter(|item| {
            let filter_lower = model.popup_filter.to_lowercase();
            let item_lower = item.to_lowercase();
            filter_lower.is_empty() || item_lower.contains(&filter_lower)
        })
        .collect();

    // Build popup content
    let title = format!(" {} ", popup.title());
    let filter_line = format!("> {}", model.popup_filter);
    let help_line = "Enter: select | Esc: cancel | ↑↓: navigate";

    let mut lines = vec![
        Line::from(vec![Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![]), // spacer
        Line::from(vec![
            Span::raw(filter_line),
            Span::styled("_", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![]), // spacer
    ];

    // Add filtered items
    let max_visible_items = popup_height.saturating_sub(5) as usize;
    let selection = model
        .popup_selection
        .min(filtered_items.len().saturating_sub(1));

    // Calculate scroll offset to keep selection visible
    let scroll_offset = if selection >= max_visible_items {
        selection - max_visible_items + 1
    } else {
        0
    };

    for (idx, item) in filtered_items
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(max_visible_items)
    {
        let is_selected = idx == selection;
        let style = if is_selected {
            Style::default()
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", if is_selected { "▸" } else { " " }), style),
            Span::styled(
                format!("{:<width$}", item, width = popup_width as usize - 4),
                style,
            ),
        ]));
    }

    // Fill remaining space
    for _ in 0..max_visible_items.saturating_sub(filtered_items.len()) {
        lines.push(Line::from(vec![Span::raw("")]));
    }

    lines.push(Line::from(vec![])); // spacer
    lines.push(Line::from(vec![Span::styled(
        help_line,
        Style::default().fg(Color::DarkGray),
    )]));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue)),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, popup_area);
}

/// Render a text prompt popup for single-line input
fn render_text_prompt_popup(
    model: &Model,
    frame: &mut Frame,
    prompt: &str,
    placeholder: &str,
    area: Rect,
) {
    use ratatui::widgets::Clear;

    // Calculate popup size - fixed height for text prompt
    let popup_width = (area.width * 2 / 3).min(60).max(40);
    let popup_height = 7u16; // Fixed height: title + spacer + prompt + input + spacer + help
    let popup_x = (area.width - popup_width) / 2;
    let popup_y = (area.height - popup_height) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the background behind the popup
    frame.render_widget(Clear, popup_area);

    // Build text prompt content
    let title = format!(" {} ", prompt);
    let help_line = "Enter: confirm | Esc: cancel";

    // Build input line - real cursor is rendered via frame.set_cursor_position()
    let mut input_line = vec![Span::raw("> ")];

    if model.text_input.is_empty() {
        // Show placeholder in gray
        input_line.push(Span::styled(
            placeholder.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        // Show input text
        input_line.push(Span::styled(model.text_input.clone(), Style::default()));
    }

    let mut lines = vec![
        Line::from(vec![Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![]), // spacer
        Line::from(input_line),
        Line::from(vec![]), // spacer
    ];

    lines.push(Line::from(vec![Span::styled(
        help_line,
        Style::default().fg(Color::DarkGray),
    )]));

    let paragraph = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue)),
    );

    frame.render_widget(paragraph, popup_area);
}

/// Render a debug overlay showing tree positions for each visible line.
/// Only renders when RUST_LOG=debug is set.
fn render_debug_overlay(model: &Model, frame: &mut Frame, area: Rect) {
    let start = model.scroll_offset;
    let visible_height = area.height as usize;
    let end = (start + visible_height).min(model.display_lines.len());
    let visible_count = end.saturating_sub(start);

    // Get mapping buffer
    let buffer = match model.mapping_buffer.lock() {
        Ok(b) => b,
        Err(_) => return,
    };

    // Build lines with tree positions right-aligned
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible_count);

    for i in 0..visible_count {
        let line_idx = start + i;
        let tree_pos_str = match buffer.get_tree_position(line_idx) {
            Some(pos) if !pos.is_empty() => {
                let parts: Vec<String> = pos.iter().map(|p| p.to_string()).collect();
                format!("[{}]", parts.join(","))
            }
            _ => "[]".to_string(),
        };

        // Right-align the tree position text
        let style = Style::default().fg(Color::DarkGray);
        let span = Span::styled(tree_pos_str, style);
        lines.push(Line::from(vec![span]));
    }

    // Create overlay paragraph with transparent background
    let overlay = Paragraph::new(Text::from(lines)).alignment(ratatui::layout::Alignment::Right);

    // Render in the same area as the log list (overlay on top)
    frame.render_widget(overlay, area);
}

fn render_info_list(model: &Model) -> Option<List<'static>> {
    let info_list = model.info_list.as_ref()?;
    Some(
        List::new(info_list.clone()).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::Blue)),
        ),
    )
}
