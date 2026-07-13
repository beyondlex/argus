use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::app::{App, AppMode, FilterFocus, Focus, SearchMode, TreeNode, DELTA_UNIT_LABELS};
use crate::components::{command_bar, file_tree, help_popup, metadata, status_bar, time_help};
use crate::handler;
use crate::util::{format_size, key_hints};
use argus_core::ROOT_NODE;
use crossterm::event::{self, Event};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
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

    let time_to_cursor = if app.search_mode == SearchMode::Input || app.mode == AppMode::Command {
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

    if let Some(clear_at) = app.error_clear_at {
        if Instant::now() >= clear_at {
            app.last_error = None;
            app.error_clear_at = None;
            dirty = true;
        }
    }

    let should_blink = app.search_mode == SearchMode::Input || app.mode == AppMode::Command;
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
    render_header(f, chunks[0]);
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
        },
    );
}

fn render_status_bar(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let error_str = app.last_error.as_deref();
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
        app.server_connected,
        app.sort_mode,
    );
}

fn render_overlays(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    match app.mode {
        AppMode::DeletePrompt => render_delete_prompt(f, area, app, false),
        AppMode::DeletePermanentPrompt => render_delete_prompt(f, area, app, true),
        AppMode::Help => help_popup::render(f, area),
        AppMode::TimeHelp => time_help::render(f, area, &mut app.time_help_scroll),
        AppMode::Command => command_bar::render(
            f,
            area,
            &app.command_input,
            &app.command_matches,
            app.command_selected,
        ),
        AppMode::Browsing => {}
    }

    if let Some((path, meta)) = &app.info_data {
        metadata::render(f, area, path, meta);
    }

    if let Some(ref state) = app.delta_detail {
        crate::components::delta_detail::render(f, area, state);
    }
}

/// Render header bar
fn render_header(f: &mut Frame, area: ratatui::layout::Rect) {
    let mut header_spans: Vec<Span> = vec![
        Span::styled(
            " Argus v0.1.0 ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];
    header_spans.extend(key_hints(&[("?", "Help"), ("q", "Quit")]));
    let line = Line::from(header_spans);
    f.render_widget(Paragraph::new(line), area);
}

fn focus_highlight_style(active: bool, inactive_fg: Color) -> Style {
    if active {
        Style::default().fg(Color::Black).bg(Color::LightYellow)
    } else {
        Style::default().fg(inactive_fg).bg(Color::Black)
    }
}

/// Render the filter pane (time range + delta filter)
fn render_filter_pane(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let is_focused = app.focus == Focus::FilterPane;

    let border_style = Style::default().fg(if is_focused {
        Color::Magenta
    } else {
        Color::DarkGray
    });

    let hint: Vec<Span> = if is_focused {
        key_hints(&[("Tab", "cycle"), ("Esc", "Files"), ("c", "Clear")])
    } else {
        key_hints(&[("f", "Focus"), ("c", "Clear")])
    };

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(border_style)
        .title(Line::from(hint))
        .title_alignment(Alignment::Right);
    let inner = block.inner(area);

    let default_bg = Color::Black;

    if !app.server_connected {
        let line = Line::from(vec![
            Span::styled(" ", Style::default().bg(default_bg)),
            Span::styled(
                "Press R to connect to daemon",
                Style::default().fg(Color::DarkGray).bg(default_bg),
            ),
        ]);
        f.render_widget(
            Paragraph::new(line).style(Style::default().bg(default_bg)),
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
        Color::White,
    );

    let delta_value_style = focus_highlight_style(
        is_focused && app.filter_focus == FilterFocus::DeltaValue,
        Color::Yellow,
    );

    let delta_unit_style = focus_highlight_style(
        is_focused && app.filter_focus == FilterFocus::DeltaUnit,
        Color::Cyan,
    );

    let delta_prefix_style = Style::default().fg(Color::DarkGray).bg(default_bg);

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
        left.push(Span::styled(right_text, Style::default().fg(Color::Green)));
    }

    f.render_widget(
        Paragraph::new(Line::from(left)).style(Style::default().bg(default_bg)),
        inner,
    );
    f.render_widget(block, area);
}
fn render_delete_prompt(f: &mut Frame, area: ratatui::layout::Rect, app: &App, permanent: bool) {
    let popup = crate::components::help_popup::centered_rect(50, 40, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Delete Confirmation ")
        .style(Style::default().fg(Color::Red).bg(Color::Black));

    let path_display = app
        .delete_target_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let action_text = if permanent {
        "This will permanently delete the item."
    } else {
        "This will move the item to trash."
    };

    let confirm_label = if permanent {
        "Permanently delete"
    } else {
        "Confirm delete"
    };

    let text = Paragraph::new(vec![
        Line::from(vec![Span::styled(
            "WARNING:",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![
            Span::styled("Path: ", Style::default().fg(Color::Gray)),
            Span::styled(&path_display, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![Span::styled(
            action_text,
            Style::default().fg(Color::White),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(key_hints(&[("y", confirm_label), ("n", "Cancel")])),
    ])
    .block(block)
    .alignment(Alignment::Center);
    f.render_widget(text, popup);
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
        let backend = TestBackend::new(80, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = ratatui::layout::Rect::new(0, 0, 80, 1);
                render_header(f, area);
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
        let backend = TestBackend::new(80, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = ratatui::layout::Rect::new(0, 0, 80, 1);
                render_header(f, area);
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
