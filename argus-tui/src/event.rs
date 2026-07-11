use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use ratatui::Frame;

use crate::app::{App, AppMode, FilterMode};
use crate::components::{file_tree, help_popup, metadata, status_bar};
use crate::handler;

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

        // Compute how long to sleep — just long enough for next pending timer
        let time_to_spinner = if app.scanning {
            let elapsed = app.scan_spinner_tick.elapsed();
            spinner_rate.saturating_sub(elapsed)
        } else {
            Duration::MAX
        };

        let time_to_cursor = if app.filter_mode == FilterMode::Input {
            let elapsed = last_cursor_tick.elapsed();
            cursor_blink_rate.saturating_sub(elapsed)
        } else {
            Duration::MAX
        };

        let time_to_error = app
            .error_clear_at
            .map(|clear_at| clear_at.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::MAX);

        let poll_timeout = time_to_spinner
            .min(time_to_cursor)
            .min(time_to_error)
            .min(Duration::from_millis(100)); // cap idle wakeup

        let got_event = event::poll(poll_timeout)?;
        let mut dirty = false;

        if got_event {
            match event::read()? {
                Event::Key(key) => {
                    handler::handle_key(key, app);
                    if SHOULD_QUIT.load(Ordering::Relaxed) {
                        app.should_quit = true;
                    }
                    dirty = true;
                }
                Event::Resize(..) => dirty = true,
                _ => {}
            }
        }

        // Process background messages
        while let Ok(msg) = app.rx.try_recv() {
            app.handle_message(msg);
            dirty = true;
        }

        // Advance scan spinner
        if app.scanning && app.scan_spinner_tick.elapsed() >= spinner_rate {
            app.scan_spinner = (app.scan_spinner + 1) % 10;
            app.scan_spinner_tick = Instant::now();
            dirty = true;
        }

        // Clear transient errors
        if let Some(clear_at) = app.error_clear_at {
            if Instant::now() >= clear_at {
                app.last_error = None;
                app.error_clear_at = None;
                dirty = true;
            }
        }

        // Cursor blink for filter input
        if app.filter_mode == FilterMode::Input && last_cursor_tick.elapsed() >= cursor_blink_rate {
            cursor_visible = !cursor_visible;
            last_cursor_tick = Instant::now();
            dirty = true;
        }
        if app.filter_mode != FilterMode::Input {
            cursor_visible = true;
        }

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

/// Render the entire TUI
fn render(f: &mut Frame, app: &mut App, cursor_visible: bool) {
    use ratatui::layout::{Constraint, Direction, Layout};

    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(1),    // Main content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Header
    render_header(f, chunks[0]);

    // Main content: tree takes full width
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(100)])
        .split(chunks[1]);

    // File tree
    let file_tree_focused = app.focus == crate::app::Focus::Tree && app.mode == AppMode::Browsing;
    let delta_cache = if app.server_mode {
        Some(&app.delta_cache)
    } else {
        None
    };
    file_tree::render(
        f,
        main_chunks[0],
        &app.tree_lines,
        app.cursor,
        app.scroll_offset,
        app.sort_mode,
        &app.view_root_path,
        &app.filter_word,
        app.filter_mode,
        &app.match_indices,
        app.current_match,
        cursor_visible,
        file_tree_focused,
        delta_cache,
    );

    // Status bar
    let error_str = app.last_error.as_deref();
    let scan_elapsed = app.scan_started_at.map(|started| started.elapsed());
    status_bar::render(
        f,
        chunks[2],
        app.mode,
        &app.view_root_path,
        app.scanning,
        app.scan_progress,
        app.scan_spinner,
        scan_elapsed,
        app.last_scan_summary.as_ref(),
        error_str,
        app.server_mode,
        app.server_connected,
    );

    // Overlays
    match app.mode {
        AppMode::DeletePrompt => render_delete_prompt(f, area, app, false),
        AppMode::DeletePermanentPrompt => render_delete_prompt(f, area, app, true),
        AppMode::Help => help_popup::render(f, area),
        AppMode::Browsing => {}
    }

    // Info popup (on top of everything, including overlays)
    if let Some((path, meta)) = &app.info_data {
        metadata::render(f, area, path, meta);
    }
}

/// Render header bar
fn render_header(f: &mut Frame, area: ratatui::layout::Rect) {
    use ratatui::{
        style::{Color, Style},
        text::{Line, Span},
        widgets::Paragraph,
    };

    let line = Line::from(vec![
        Span::styled(
            " Argus v0.1.0 ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("[?] Help", Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled("[Q] Quit", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

/// Render delete confirmation prompt
fn render_delete_prompt(f: &mut Frame, area: ratatui::layout::Rect, app: &App, permanent: bool) {
    use ratatui::{
        layout::Alignment,
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

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
        Line::from(vec![
            Span::styled(
                "y",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" - {}  ", confirm_label),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "n",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" - Cancel", Style::default().fg(Color::DarkGray)),
        ]),
    ])
    .block(block)
    .alignment(Alignment::Center);
    f.render_widget(text, popup);
}
