use crate::app::{App, AppMode};
use crate::components::{
    ai_review, command_bar, flat_tree, help_popup, metadata, popup, status_bar, time_help,
};
use crate::util::{display_path, format_count, format_duration, format_size, key_hints};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Gauge, Paragraph},
    Frame,
};

/// Render the entire TUI
pub fn render(f: &mut Frame, app: &mut App, cursor_visible: bool) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(1),    // Main content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    render_chrome(f, app, &chunks, cursor_visible);
    render_overlays(f, app, area);
}

fn render_chrome(f: &mut Frame, app: &App, chunks: &[Rect], cursor_visible: bool) {
    render_header(f, chunks[0], app);
    render_main_content(f, app, chunks[1], cursor_visible);
    render_status_bar(f, app, chunks[2]);
}

fn render_main_content(f: &mut Frame, app: &App, area: Rect, cursor_visible: bool) {
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(100)])
        .split(area);

    let file_tree_focused = app.mode == AppMode::Browsing;
    let delta_cache = if app.server_connected {
        Some(&app.delta_cache)
    } else {
        None
    };

    flat_tree::render(
        f,
        main_chunks[0],
        flat_tree::FlatRenderCtx {
            children: &app.current_children,
            filtered_indices: &app.current_filtered,
            cursor: app.cursor,
            scroll_offset: app.scroll_offset,
            view_root_path: &app.view_root_path,
            current_dir_path: &app.current_dir_path,
            search_word: &app.search_word,
            search_mode: app.search_mode,
            cursor_visible,
            focus: file_tree_focused,
            delta_cache,
            current_dir_disk_usage: app.current_dir_disk_usage,
            multi_select: app.multi_select,
            selected_paths: &app.selected_paths,
            theme: &app.theme,
        },
    );
}

fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let error_str = app.last_error.as_deref();
    let status_is_error = app.status_is_error;
    status_bar::render(
        f,
        area,
        app.mode,
        error_str,
        status_is_error,
        app.sort_mode,
        app.multi_select,
        app.selected_paths.len(),
        &app.theme,
        app.time_custom,
        app.time_preset,
        &app.time_custom_label,
        app.delta_filter_active,
        app.delta_filter_value,
        app.delta_filter_unit,
        app.current_dir_disk_usage,
        app.current_dir_total,
        app.current_dir_items,
    );
}

fn render_scan_popup(f: &mut Frame, area: Rect, app: &App) {
    const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

    let width = (area.width * 70 / 100).max(40).min(area.width).min(120);
    let height: u16 = 7.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect {
        x,
        y,
        width,
        height,
    };

    let root_display = display_path(&app.current_scan_path());
    let title = format!(" Scanning {} ", root_display);
    let block = popup::popup_block(title, popup::PopupStyle::Normal, &app.theme)
        .title_bottom(Line::from(key_hints(&[("Esc", "Cancel")], &app.theme)).right_aligned());

    let inner = block.inner(popup_area);

    let current_path = app.scan_current_path.as_deref().unwrap_or("");

    let mut path_lines: Vec<Line> = Vec::new();
    path_lines.push(Line::from(""));

    let max_width = inner.width.saturating_sub(4) as usize;
    let chars: Vec<char> = current_path.chars().collect();
    let display_path_str = if chars.len() > max_width {
        let ellipsis = "...";
        let ellipsis_len = ellipsis.chars().count();
        let available = max_width.saturating_sub(ellipsis_len);
        let left_len = available / 2;
        let right_len = available - left_len;
        let left: String = chars[..left_len].iter().collect();
        let right: String = chars[chars.len() - right_len..].iter().collect();
        format!("{}{}{}", left, ellipsis, right)
    } else {
        current_path.to_string()
    };
    path_lines.push(Line::from(Span::styled(
        format!("  {}", display_path_str),
        Style::default().fg(app.theme.text_secondary),
    )));
    path_lines.push(Line::from(""));

    let mut stats_spans: Vec<Span> = Vec::new();
    if let Some((current, total_bytes)) = app.scan_progress {
        stats_spans.push(Span::styled(
            "Size:",
            Style::default().fg(app.theme.text_secondary),
        ));
        stats_spans.push(Span::styled(
            format!(" {}", format_size(total_bytes)),
            Style::default().fg(app.theme.text_highlight),
        ));
        stats_spans.push(Span::raw("  "));
        stats_spans.push(Span::styled(
            "Items:",
            Style::default().fg(app.theme.text_secondary),
        ));
        stats_spans.push(Span::styled(
            format!(" {}", format_count(current)),
            Style::default().fg(app.theme.text_highlight),
        ));
    }
    stats_spans.push(Span::raw("  "));
    stats_spans.push(Span::styled(
        "Took:",
        Style::default().fg(app.theme.text_secondary),
    ));
    stats_spans.push(Span::styled(
        format!(
            " {}",
            format_duration(
                app.scan_started_at
                    .map(|started| started.elapsed())
                    .unwrap_or_default()
            )
        ),
        Style::default().fg(app.theme.text_highlight),
    ));
    stats_spans.push(Span::raw("  "));
    stats_spans.push(Span::styled(
        format!(
            "{} ",
            SPINNER_FRAMES[app.scan_spinner as usize % SPINNER_FRAMES.len()]
        ),
        Style::default().fg(app.theme.spinner),
    ));

    let [path_area, stats_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);

    f.render_widget(Clear, popup_area);
    f.render_widget(block, popup_area);
    f.render_widget(Paragraph::new(path_lines), path_area);
    f.render_widget(
        Paragraph::new(Line::from(stats_spans)).alignment(Alignment::Center),
        stats_area,
    );
}

fn render_overlays(f: &mut Frame, app: &mut App, area: Rect) {
    if app.scanning {
        render_scan_popup(f, area, app);
        return;
    }

    match app.mode {
        AppMode::DeletePrompt => render_delete_prompt(f, area, app, false),
        AppMode::DeletePermanentPrompt => render_delete_prompt(f, area, app, true),
        AppMode::Deleting => render_delete_progress(f, area, app),
        AppMode::Help => help_popup::render(f, area, &app.theme),
        AppMode::TimeHelp => time_help::render(f, area, &mut app.time_help_scroll, &app.theme),
        AppMode::Command => command_bar::render(
            f,
            area,
            &app.command_input,
            &app.command_matches,
            app.command_selected,
            &app.theme,
        ),
        AppMode::Browsing => {}
        AppMode::Finder => {
            if let Some(finder) = app.finder_state.as_mut() {
                ratatui_finder::render_finder_popup(f, area, finder);
            }
        }
        AppMode::AiReview => {
            ai_review::render(f, area, app);
        }
    }

    if let Some((path, meta)) = &app.info_data {
        metadata::render(f, area, path, meta, app.info_ai.as_ref(), &app.theme);
    }

    if let Some(ref state) = app.delta_detail {
        crate::components::delta_detail::render(f, area, state, &app.theme);
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let mut header_spans: Vec<Span> = vec![
        Span::styled(
            " Argus v0.1.0 ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];
    header_spans.extend(key_hints(&[("?", "Help"), ("q", "Quit")], &app.theme));

    let mut summary_spans: Vec<Span> = Vec::new();
    if let Some(summary) = &app.last_scan_summary {
        summary_spans.push(Span::styled(
            "Last scan: ",
            Style::default().fg(app.theme.text_tertiary),
        ));
        summary_spans.push(Span::styled(
            "Size:",
            Style::default().fg(app.theme.text_secondary),
        ));
        summary_spans.push(Span::styled(
            format!(" {}", format_size(summary.total_disk_usage)),
            Style::default().fg(app.theme.text_highlight),
        ));
        summary_spans.push(Span::styled(
            " Items:",
            Style::default().fg(app.theme.text_secondary),
        ));
        summary_spans.push(Span::styled(
            format!(" {}", format_count(summary.total_files)),
            Style::default().fg(app.theme.text_highlight),
        ));
        summary_spans.push(Span::styled(
            " Took:",
            Style::default().fg(app.theme.text_secondary),
        ));
        summary_spans.push(Span::styled(
            format!(" {}", format_duration(summary.duration)),
            Style::default().fg(app.theme.text_highlight),
        ));
    }

    if app.deleted_bytes > 0 {
        summary_spans.push(Span::styled(
            format!(" {} ", format_size(app.deleted_bytes)),
            Style::default()
                .fg(app.theme.success)
                .add_modifier(Modifier::BOLD),
        ));
        summary_spans.push(Span::styled(
            "Freed",
            Style::default().fg(app.theme.text_tertiary),
        ));
    }

    let (daemon_text, daemon_color) = if app.server_connected {
        ("● Daemon", app.theme.success)
    } else {
        ("○ Daemon", app.theme.text_tertiary)
    };
    let daemon_span = Span::styled(daemon_text, Style::default().fg(daemon_color));
    let daemon_line = Line::from(vec![Span::raw(" "), daemon_span, Span::raw(" ")]);

    let summary_width: u16 = summary_spans.iter().map(|s| s.content.len() as u16).sum();
    let daemon_width: u16 = daemon_text.len() as u16 + 2;
    let right_total = summary_width + daemon_width;

    if right_total + 4 < area.width {
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(right_total)])
                .flex(Flex::SpaceBetween)
                .areas(area);
        f.render_widget(Paragraph::new(Line::from(header_spans)), left_area);

        let mut right_spans = summary_spans;
        right_spans.push(Span::raw("  "));
        right_spans.extend(daemon_line.spans);
        f.render_widget(Paragraph::new(Line::from(right_spans)), right_area);
    } else {
        f.render_widget(Paragraph::new(Line::from(header_spans)), area);
    }
}

fn render_delete_prompt(f: &mut Frame, area: Rect, app: &App, permanent: bool) {
    let height_fixed: u16 = 11;
    let popup_area = popup::centered_rect(50, 70, area);
    let height = height_fixed.min(area.height);
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect {
        x: popup_area.x,
        y,
        width: popup_area.width,
        height,
    };

    f.render_widget(Clear, popup);

    let is_batch = !app.delete_target_paths.is_empty();

    let (title, path_display, action_text, confirm_label) = if is_batch {
        let count = app.delete_target_paths.len();
        (
            format!(" Delete {} items? ", count),
            format!("{} items selected for deletion", count),
            if permanent {
                "This will permanently delete all selected items."
            } else {
                "This will move all selected items to trash."
            },
            if permanent {
                "Permanently delete"
            } else {
                "Confirm delete"
            },
        )
    } else {
        let path_str = app
            .delete_target_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        (
            " Delete Confirmation ".to_string(),
            format!("Path: {}", path_str),
            if permanent {
                "This will permanently delete the item."
            } else {
                "This will move the item to trash."
            },
            if permanent {
                "Permanently delete"
            } else {
                "Confirm delete"
            },
        )
    };

    let block = popup::popup_block(title, popup::PopupStyle::Danger, &app.theme).title_bottom(
        Line::from(key_hints(
            &[("y", confirm_label), ("n", "Cancel")],
            &app.theme,
        ))
        .centered(),
    );

    let text = Paragraph::new(vec![
        Line::from(vec![Span::styled(
            "WARNING:",
            Style::default()
                .fg(app.theme.danger)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            &path_display,
            Style::default().fg(app.theme.text),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            action_text,
            Style::default().fg(app.theme.text),
        )]),
    ])
    .block(block)
    .alignment(Alignment::Center);
    f.render_widget(text, popup);
}

fn render_delete_progress(f: &mut Frame, area: Rect, app: &App) {
    let height_fixed: u16 = 3;
    let popup_area = popup::centered_rect(50, 20, area);
    let height = height_fixed.min(area.height);
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect {
        x: popup_area.x,
        y,
        width: popup_area.width,
        height,
    };

    f.render_widget(Clear, popup);

    let (current, total) = app.delete_progress.unwrap_or((0, 1));
    let label = if app.delete_permanent {
        "Permanently Deleting..."
    } else {
        "Moving to Trash..."
    };

    let block = popup::popup_block(
        format!(" {} ({}/{}) ", label, current, total),
        popup::PopupStyle::Danger,
        &app.theme,
    );

    let ratio = if total > 0 {
        current as f64 / total as f64
    } else {
        0.0
    };

    let gauge = Gauge::default()
        .block(block)
        .gauge_style(
            Style::default()
                .fg(app.theme.danger)
                .bg(app.theme.bg)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(ratio)
        .label(format!("{:.0}%", ratio * 100.0));

    f.render_widget(gauge, popup);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use ratatui::backend::TestBackend;
    use tokio::sync::mpsc;

    fn make_app() -> App {
        let config = crate::config::TuiConfig::default();
        let (tx, _rx) = mpsc::channel(100);
        App::new(config, tx, _rx)
    }

    #[test]
    fn test_render_header_contains_title() {
        let app = make_app();
        let backend = TestBackend::new(80, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 80, 1);
                render_header(f, area, &app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("Argus"),
            "header should contain Argus, got: {content:?}"
        );
    }

    #[test]
    fn test_render_header_shows_help_and_quit_hints() {
        let app = make_app();
        let backend = TestBackend::new(80, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 80, 1);
                render_header(f, area, &app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(content.contains('?'), "header should have help hint");
        assert!(content.contains('q'), "header should have quit hint");
    }

    #[test]
    fn test_render_delete_prompt_shows_path() {
        let mut app = make_app();
        app.delete_target_path = Some(std::path::PathBuf::from("/tmp/test_file"));
        app.mode = AppMode::DeletePrompt;

        let backend = TestBackend::new(80, 25);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 80, 25);
                render_delete_prompt(f, area, &app, false);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("test_file"),
            "delete prompt should contain path, got: {content:?}"
        );
        assert!(
            content.contains("trash"),
            "non-permanent should mention trash, got: {content:?}"
        );
    }

    #[test]
    fn test_render_delete_prompt_permanent() {
        let mut app = make_app();
        app.delete_target_path = Some(std::path::PathBuf::from("/tmp/test_file"));
        app.mode = AppMode::DeletePermanentPrompt;

        let backend = TestBackend::new(80, 25);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 80, 25);
                render_delete_prompt(f, area, &app, true);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("permanently delete"),
            "permanent prompt should say permanently delete, got: {content:?}"
        );
    }
}
