use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::app::{App, AppMode, FilterFocus, Focus, TreeNode, DELTA_UNIT_LABELS};
use crate::components::{
    command_bar, file_tree, help_popup, metadata, popup, status_bar, time_help,
};
use crate::handler;
use crate::util::{format_size, key_hints};
use argus_core::ROOT_NODE;
use crossterm::event::{self, Event};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph},
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
            Constraint::Length(2), // Filter pane (border + content)
            Constraint::Min(1),    // Main content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    render_chrome(f, app, &chunks, cursor_visible);
    render_overlays(f, app, area);
}

fn render_chrome(f: &mut Frame, app: &App, chunks: &[ratatui::layout::Rect], cursor_visible: bool) {
    render_header(f, chunks[0], &app.theme);
    render_filter_pane(f, chunks[1], app);
    render_main_content(f, app, chunks[2], cursor_visible);
    render_status_bar(f, app, chunks[3]);
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

    let file_tree_focused = app.focus == Focus::Tree && app.mode == AppMode::Browsing;
    let delta_cache = if app.server_connected {
        Some(&app.delta_cache)
    } else {
        None
    };
    let root_total_size = app.tree_root.as_ref().map_or(0, |tr| match tr {
        TreeNode::Snapshot(snap, _) => snap.node(ROOT_NODE).size,
    });
    file_tree::render(
        f,
        main_chunks[0],
        file_tree::TreeRenderCtx {
            tree_lines: &app.tree_lines,
            filtered_indices: &app.filtered_tree_lines,
            cursor: app.cursor,
            scroll_offset: app.scroll_offset,
            view_root_path: &app.view_root_path,
            search_word: &app.search_word,
            search_mode: app.search_mode,
            match_indices: &app.match_indices,
            current_match: app.current_match,
            cursor_visible,
            focus: file_tree_focused,
            delta_cache,
            root_total_size,
            multi_select: app.multi_select,
            selected_paths: &app.selected_paths,
            theme: &app.theme,
        },
    );
}

fn render_status_bar(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let error_str = app.last_error.as_deref();
    let status_is_error = app.status_is_error;
    let scan_elapsed = app.scan_started_at.map(|started| started.elapsed());
    status_bar::render(
        f,
        area,
        app.mode,
        &app.view_root_path,
        app.scanning,
        app.scan_progress,
        app.scan_spinner,
        scan_elapsed,
        app.last_scan_summary.as_ref(),
        error_str,
        status_is_error,
        app.server_connected,
        app.sort_mode,
        app.multi_select,
        &app.theme,
    );
}

fn render_overlays(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
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
fn render_header(f: &mut Frame, area: ratatui::layout::Rect, theme: &crate::theme::ColorTheme) {
    let mut header_spans: Vec<Span> = vec![
        Span::styled(
            " Argus v0.1.0 ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];
    header_spans.extend(key_hints(&[("?", "Help"), ("q", "Quit")], theme));
    let line = Line::from(header_spans);
    f.render_widget(Paragraph::new(line), area);
}

fn focus_highlight_style(active: bool, theme: &crate::theme::ColorTheme) -> Style {
    if active {
        Style::default().fg(theme.focus_fg).bg(theme.focus_bg)
    } else {
        Style::default().fg(theme.text).bg(theme.bg)
    }
}

/// Render the filter pane (time range + delta filter)
fn render_filter_pane(f: &mut Frame, area: Rect, app: &App) {
    let is_focused = app.focus == Focus::FilterPane;

    let border_style = Style::default().fg(if is_focused {
        app.theme.accent
    } else {
        app.theme.border_unfocused
    });

    let hint: Vec<Span> = if is_focused {
        key_hints(
            &[("Tab", "cycle"), ("Esc", "Files"), ("c", "Clear")],
            &app.theme,
        )
    } else {
        key_hints(&[("f", "Focus"), ("c", "Clear")], &app.theme)
    };

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(border_style)
        .title(Line::from(hint))
        .title_alignment(Alignment::Right);
    let inner = block.inner(area);

    if !app.server_connected {
        let line = Line::from(vec![
            Span::styled(" ", Style::default().bg(app.theme.bg)),
            Span::styled(
                "Press R to connect to daemon",
                Style::default()
                    .fg(app.theme.text_tertiary)
                    .bg(app.theme.bg),
            ),
        ]);
        f.render_widget(
            Paragraph::new(line).style(Style::default().bg(app.theme.bg)),
            inner,
        );
        f.render_widget(block, area);
        return;
    }

    let time_label = if app.time_custom {
        format!(" Time: {} ", app.time_custom_label)
    } else {
        format!(" Time: in {} ", App::time_preset_label(app.time_preset))
    };
    let time_style = focus_highlight_style(
        is_focused && app.filter_focus == FilterFocus::TimePreset && !app.time_custom,
        &app.theme,
    );

    let delta_value_style = focus_highlight_style(
        is_focused && app.filter_focus == FilterFocus::DeltaValue,
        &app.theme,
    );

    let delta_unit_style = focus_highlight_style(
        is_focused && app.filter_focus == FilterFocus::DeltaUnit,
        &app.theme,
    );

    let delta_prefix_style = Style::default()
        .fg(app.theme.text_tertiary)
        .bg(app.theme.bg);

    let mut left: Vec<Span> = vec![
        Span::styled(time_label, time_style),
        Span::raw("  "),
        Span::styled("+Size: >=", delta_prefix_style),
        Span::styled(
            if app.delta_filter_active {
                app.delta_filter_value.to_string()
            } else {
                "-".to_string()
            },
            delta_value_style,
        ),
        Span::raw(" "),
        Span::styled(
            if app.delta_filter_active {
                DELTA_UNIT_LABELS
                    .get(app.delta_filter_unit)
                    .copied()
                    .unwrap_or("--")
                    .to_string()
            } else {
                "-".to_string()
            },
            delta_unit_style,
        ),
    ];

    // Right-aligned deleted-space counter (only if > 0)
    if app.deleted_bytes > 0 {
        let right_text = format!(" Deleted: {} ", format_size(app.deleted_bytes));
        let left_width: usize = left.iter().map(|s| s.content.len()).sum();
        let padding = inner
            .width
            .saturating_sub((left_width + right_text.len()) as u16);
        if padding > 0 {
            left.push(Span::raw(" ".repeat(padding as usize)));
        }
        left.push(Span::styled(
            right_text,
            Style::default().fg(app.theme.success),
        ));
    }

    f.render_widget(
        Paragraph::new(Line::from(left)).style(Style::default().bg(app.theme.bg)),
        inner,
    );
    f.render_widget(block, area);
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
                render_header(f, area, &app.theme);
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
                render_header(f, area, &app.theme);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(content.contains('?'), "header should have help hint");
        assert!(content.contains('q'), "header should have quit hint");
    }

    #[test]
    fn test_render_filter_pane_not_connected_shows_hint() {
        let mut app = make_app();
        app.server_connected = false;

        let backend = TestBackend::new(80, 2);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = ratatui::layout::Rect::new(0, 0, 80, 2);
                render_filter_pane(f, area, &app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("Press R to connect"),
            "disconnected hint not found in: {content:?}"
        );
    }

    #[test]
    fn test_render_filter_pane_connected_shows_time() {
        let mut app = make_app();
        app.server_connected = true;

        let backend = TestBackend::new(80, 2);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = ratatui::layout::Rect::new(0, 0, 80, 2);
                render_filter_pane(f, area, &app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("Time:"),
            "time label not found in: {content:?}"
        );
    }

    #[test]
    fn test_render_filter_pane_shows_size_filter() {
        let mut app = make_app();
        app.server_connected = true;
        app.delta_filter_active = true;
        app.delta_filter_value = 500;
        app.delta_filter_unit = 0;

        let backend = TestBackend::new(80, 2);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = ratatui::layout::Rect::new(0, 0, 80, 2);
                render_filter_pane(f, area, &app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("500"),
            "delta filter value not found in: {content:?}"
        );
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
