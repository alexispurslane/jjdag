use crate::model::Model;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, Paragraph},
};

pub const SELECTION_COLOR: Color = Color::Rgb(40, 42, 54);
pub const SAVED_SELECTION_COLOR: Color = Color::Rgb(33, 35, 45);

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
    if let Some(popup) = &model.current_popup {
        render_popup(model, frame, popup, frame.area());
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
        Span::styled(&model.revset, Style::default().fg(Color::Green)),
    ];
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
    apply_saved_selection_highlights(model, &mut log_items);
    List::new(log_items)
        .highlight_style(Style::new().bold().bg(SELECTION_COLOR))
        .scroll_padding(model.log_list_scroll_padding)
}

/// When bookmark editing is active, inject the virtual bookmark into the selected commit's line
fn inject_virtual_bookmark(model: &Model, log_items: &mut [ratatui::text::Text<'static>]) {
    let Some(ref editing_change_id) = model.bookmark_edit_change_id else {
        return;
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
        // Add the virtual bookmark span at the end
        first_line.spans.push(Span::raw(" "));
        first_line.spans.push(Span::styled(
            format!("[{}█]", model.bookmark_edit_name),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
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
fn render_popup(model: &Model, frame: &mut Frame, popup: &crate::update::Popup, area: Rect) {
    use ratatui::widgets::{Clear, Wrap};

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
