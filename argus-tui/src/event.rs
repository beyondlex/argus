use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::app::{App, AppMode};
use crate::components::{
    command_bar, flat_tree, help_popup, metadata, popup, status_bar, time_help,
};
use crate::handler;
use crate::util::{display_path, format_count, format_duration, format_size, key_hints};
use crossterm::event::{self, Event};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Gauge, Paragraph},
    Frame,
};

pub static SHOULD_QUIT: AtomicBool = AtomicBool::new(false);

/// Main event loop
pub async fn run(app: &mut App) -> anyhow::Result<()> {
    let _ = ctrlc::set_handler(|| {
        SHOULD_QUIT.store(true, Ordering::Relaxed);
    });
    let mut terminal = ratatui::init();
    let spinner_rate = Duration::from_millis(120);
    let mut cursor_visible = true;
    let mut last_cursor_tick = Instant::now();
    let cursor_blink_rate = Duration::from_millis(500);

    terminal.draw(|f| render(f, app, cursor_visible))?;

    loop {
        if SHOULD_QUIT.swap(false, Ordering::Relaxed) {
            app.should_quit = true;
        }

        let poll_timeout =
            next_poll_timeout(app, last_cursor_tick, spinner_rate, cursor_blink_rate);
        let got_event = event::poll(poll_timeout)?;
        let mut dirty = false;

        if got_event {
            dirty |= handle_input_event(event::read()?, app);
            if SHOULD_QUIT.load(Ordering::Relaxed) {
                app.should_quit = true;
            }
        }

        dirty |= drain_messages(app);
        dirty |= advance_timers(
            app,
            &mut cursor_visible,
            &mut last_cursor_tick,
            spinner_rate,
            cursor_blink_rate,
        );

        if dirty {
            terminal.draw(|f| render(f, app, cursor_visible))?;
        }

        if app.should_quit {
            break;
        }
    }

    ratatui::restore();
    Ok(())
}

fn next_poll_timeout(
    app: &App,
    last_cursor_tick: Instant,
    spinner_rate: Duration,
    cursor_blink_rate: Duration,
) -> Duration {
    let time_to_spinner = if app.scanning {
        spinner_rate.saturating_sub(app.scan_spinner_tick.elapsed())
    } else {
        Duration::MAX
    };

    let time_to_cursor = if app.search_mode == crate::types::SearchMode::Input
        || app.mode == AppMode::Command
        || app.mode == AppMode::Finder
    {
        cursor_blink_rate.saturating_sub(last_cursor_tick.elapsed())
    } else {
        Duration::MAX
    };

    let time_to_error = app
        .error_clear_at
        .map(|clear_at| clear_at.saturating_duration_since(Instant::now()))
        .unwrap_or(Duration::MAX);

    time_to_spinner
        .min(time_to_cursor)
        .min(time_to_error)
        .min(Duration::from_millis(100))
}

fn handle_input_event(event: Event, app: &mut App) -> bool {
    match event {
        Event::Key(key) => {
            handler::handle_key(key, app);
            true
        }
        Event::Resize(..) => true,
        _ => false,
    }
}

fn drain_messages(app: &mut App) -> bool {
    let mut dirty = false;
    while let Ok(msg) = app.rx.try_recv() {
        app.handle_message(msg);
        dirty = true;
    }
    dirty
}

fn advance_timers(
    app: &mut App,
    cursor_visible: &mut bool,
    last_cursor_tick: &mut Instant,
    spinner_rate: Duration,
    cursor_blink_rate: Duration,
) -> bool {
    let mut dirty = false;

    if app.scanning && app.scan_spinner_tick.elapsed() >= spinner_rate {
        app.scan_spinner = (app.scan_spinner + 1) % 10;
        app.scan_spinner_tick = Instant::now();
        dirty = true;
    }

    if app.deleting {
        dirty = true;
    }

    if let Some(clear_at) = app.error_clear_at {
        if Instant::now() >= clear_at {
            app.last_error = None;
            app.error_clear_at = None;
            dirty = true;
        }
    }

    let should_blink = app.search_mode == crate::types::SearchMode::Input
        || app.mode == AppMode::Command
        || app.mode == AppMode::Finder;
    if should_blink && last_cursor_tick.elapsed() >= cursor_blink_rate {
        *cursor_visible = !*cursor_visible;
        *last_cursor_tick = Instant::now();
        dirty = true;
    } else if !should_blink {
        *cursor_visible = true;
    }

    dirty
}

/// Render the entire TUI
fn render(f: &mut Frame, app: &mut App, cursor_visible: bool) {
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

fn render_chrome(f: &mut Frame, app: &App, chunks: &[ratatui::layout::Rect], cursor_visible: bool) {
    render_header(f, chunks[0], app);
    render_main_content(f, app, chunks[1], cursor_visible);
    render_status_bar(f, app, chunks[2]);
}

fn render_main_content(
    f: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    cursor_visible: bool,
) {
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

fn render_status_bar(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
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

    // Fixed-size popup: 7 rows (1 spacer + path + 1 spacer + stats + 1 spacer + 2 borders)
    let width = ((area.width * 70 / 100) as u16)
        .max(40)
        .min(area.width)
        .min(120);
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

    // Current file path being scanned
    let current_path = app.scan_current_path.as_deref().unwrap_or("");

    let mut path_lines: Vec<Line> = Vec::new();

    // Spacer
    path_lines.push(Line::from(""));

    // Current file path (truncated in the middle to fit, char-aware, 2-char padding)
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

    // Spacer
    path_lines.push(Line::from(""));

    // Stats row: Size, Items, Took, Spinner
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

    // Split inner area: top part for path, bottom part for centered stats
    let [path_area, stats_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);

    f.render_widget(Clear, popup_area);
    f.render_widget(block, popup_area);
    f.render_widget(ratatui::widgets::Paragraph::new(path_lines), path_area);
    f.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(stats_spans))
            .alignment(ratatui::layout::Alignment::Center),
        stats_area,
    );
}

fn render_overlays(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    // Render scanning popup on top of everything if a scan is in progress
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
    }

    if let Some((path, meta)) = &app.info_data {
        metadata::render(f, area, path, meta, &app.theme);
    }

    if let Some(ref state) = app.delta_detail {
        crate::components::delta_detail::render(f, area, state, &app.theme);
    }
}

/// Render header bar
fn render_header(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    // Left side: title + key hints
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

    // Middle: last scan summary (if available)
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

    // Right side: daemon status
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
                let area = ratatui::layout::Rect::new(0, 0, 80, 1);
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
                let area = ratatui::layout::Rect::new(0, 0, 80, 1);
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
                let area = ratatui::layout::Rect::new(0, 0, 80, 25);
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
                let area = ratatui::layout::Rect::new(0, 0, 80, 25);
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
