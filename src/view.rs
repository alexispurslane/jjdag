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
    frame.render_stateful_widget(log_list, layout[1], &mut model.log_list_state);
    model.log_list_layout = layout[1];
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
    let mut log_items = model.log_list.clone();
    inject_virtual_bookmark(model, &mut log_items);
    inject_virtual_description(model, &mut log_items);
    apply_saved_selection_highlights(model, &mut log_items);
    List::new(log_items)
        .highlight_style(Style::new().bold().bg(SELECTION_COLOR))
        .scroll_padding(model.log_list_scroll_padding)
}

/// When bookmark editing is active, inject the virtual bookmark into the selected commit's line.
/// The real cursor is rendered via terminal ANSI codes, not as fake text.
fn inject_virtual_bookmark(model: &Model, log_items: &mut [ratatui::text::Text<'static>]) {
    let editing_change_id = match &model.text_input_location {
        crate::update::TextInputLocation::Bookmark { change_id } => change_id,
        _ => return,
    };
    let Some(selected_idx) = model.log_list_state.selected() else {
        return;
    };
    let Some(text) = log_items.get_mut(selected_idx) else {
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

/// When description editing is active, replace the description line with the user's input
/// Displays actual multi-line descriptions with proper indentation
fn inject_virtual_description(model: &Model, log_items: &mut [ratatui::text::Text<'static>]) {
    if let crate::update::TextInputLocation::Description { .. } = &model.text_input_location {
        let Some(selected_idx) = model.log_list_state.selected() else {
            return;
        };
        let Some(text) = log_items.get_mut(selected_idx) else {
            return;
        };

        // Get the input text (show placeholder if empty)
        // Strip any ANSI codes that might have been included
        let input_text = if model.text_input.is_empty() {
            "(no description set)".to_string()
        } else {
            strip_ansi(&model.text_input)
        };

        // Split input into lines
        let desc_lines: Vec<&str> = input_text.split('\n').collect();

        // Get the prefix from the existing description line (for graph indentation)
        // Strip ANSI codes to avoid rendering them as text
        let prefix_content = if text.lines.len() >= 2 && !text.lines[1].spans.is_empty() {
            strip_ansi(&text.lines[1].spans[0].content)
        } else {
            "  ".to_string()
        };
        let prefix_span = Span::raw(prefix_content);

        // Build new lines: keep line 0 (graph + change_id), then add description lines
        let mut new_lines: Vec<Line<'static>> = Vec::new();

        // Keep the first line (graph + change_id) but strip ANSI codes
        if !text.lines.is_empty() {
            let clean_line = strip_ansi_from_line(&text.lines[0]);
            new_lines.push(clean_line);
        }

        // Add description lines (real cursor is rendered via ANSI codes)
        for (line_idx, line_text) in desc_lines.iter().enumerate() {
            let desc_spans = render_description_line(line_text, line_idx);
            let mut all_spans = vec![prefix_span.clone(), Span::raw(" ")];
            all_spans.extend(desc_spans);
            new_lines.push(Line::from(all_spans));
        }

        // Replace the text lines
        text.lines = new_lines;
    }
}

fn apply_saved_selection_highlights(model: &Model, log_items: &mut [ratatui::text::Text<'static>]) {
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

fn apply_saved_selection_highlight(text: &mut ratatui::text::Text<'static>) {
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
