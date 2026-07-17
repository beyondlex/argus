use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::app::{App, AppMode};
use crate::render;
use crossterm::event::{self, Event};

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

    terminal.draw(|f| render::render(f, app, cursor_visible))?;

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
            terminal.draw(|f| render::render(f, app, cursor_visible))?;
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
            crate::handler::handle_key(key, app);
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
