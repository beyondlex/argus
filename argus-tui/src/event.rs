use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use ratatui::Frame;

use crate::app::{App, AppMode, FilterMode};
use crate::components::{file_tree, filter_bar, help_popup, metadata, status_bar};
use crate::handler;

/// Main event loop
pub async fn run(app: &mut App) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(16); // ~60fps
    let mut cursor_visible = true;
    let mut last_cursor_tick = Instant::now();
    let cursor_blink_rate = Duration::from_millis(500);

    loop {
        terminal.draw(|f| render(f, app, cursor_visible))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_default();

        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => handler::handle_key(key, app),
                Event::Resize(..) => {}
                _ => {}
            }
        }

        // Process background messages
        while let Ok(msg) = app.rx.try_recv() {
            app.handle_message(msg);
        }

        // Clear transient errors
        if let Some(clear_at) = app.error_clear_at {
            if Instant::now() >= clear_at {
                app.last_error = None;
                app.error_clear_at = None;
            }
        }

        // Cursor blink for filter input
        if app.filter_mode == FilterMode::Input && last_cursor_tick.elapsed() >= cursor_blink_rate {
            cursor_visible = !cursor_visible;
            last_cursor_tick = Instant::now();
        }
        if app.filter_mode != FilterMode::Input {
            cursor_visible = true;
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
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

    // Layout: Header(1) / FilterBar(1) / Main / StatusBar(1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Length(3), // Filter bar
            Constraint::Min(1),    // Main content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Header
    render_header(f, chunks[0]);

    // Filter bar
    let has_enough = app.available_snapshots.len() >= 2;
    let filter_focused = app.focus == crate::app::Focus::FilterBar && app.mode == AppMode::Browsing;
    filter_bar::render(
        f,
        chunks[1],
        &app.filter_state,
        &app.available_snapshots,
        filter_focused,
        app.filter_state.sub_focus,
        has_enough,
    );

    // Main content: split into tree (70%) and metadata (30%)
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(chunks[2]);

    // File tree
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
    );

    // Metadata panel
    let has_scan = app.scan_cache.contains_key(&app.view_root_path);
    let last_scan = app.scan_cache.get(&app.view_root_path).map(|s| s.timestamp);
    metadata::render(
        f,
        main_chunks[1],
        app.selected_line(),
        app.has_delta_column(),
        has_scan,
        last_scan,
    );

    // Status bar
    let error_str = app.last_error.as_deref();
    let file_count = app.tree_lines.len();
    status_bar::render(
        f,
        chunks[3],
        app.mode,
        app.focus,
        file_count,
        app.scan_progress,
        error_str,
    );

    // Overlays
    match app.mode {
        AppMode::DeletePrompt => render_delete_prompt(f, area, app),
        AppMode::Help => help_popup::render(f, area),
        AppMode::Browsing => {}
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
fn render_delete_prompt(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    use ratatui::{
        layout::Alignment,
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Clear, Paragraph},
    };

    let popup = crate::components::help_popup::centered_rect(50, 25, area);
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
            "This will move the item to trash.",
            Style::default().fg(Color::White),
        )]),
        Line::from(vec![Span::raw("")]),
        Line::from(vec![
            Span::styled(
                "y",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" - Confirm delete  ", Style::default().fg(Color::DarkGray)),
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
