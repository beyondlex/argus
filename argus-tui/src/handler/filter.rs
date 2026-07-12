use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, FilterFocus, Focus};

pub(crate) fn handle_filter_pane_key(key: KeyEvent, app: &mut App) {
    let skip_time = app.time_custom;
    match key.code {
        KeyCode::Tab | KeyCode::Char('\t') => {
            app.filter_focus = cycle_filter_focus(app.filter_focus, skip_time, true);
        }
        KeyCode::BackTab => {
            app.filter_focus = cycle_filter_focus(app.filter_focus, skip_time, false);
        }
        KeyCode::Char(ch) if ch.is_ascii_digit() && app.filter_focus == FilterFocus::DeltaValue => {
            let digit = (ch as u8 - b'0') as u64;
            if app.delta_filter_active {
                app.delta_filter_value = app
                    .delta_filter_value
                    .saturating_mul(10)
                    .saturating_add(digit);
            } else {
                app.delta_filter_active = true;
                app.delta_filter_value = digit;
            }
            app.refresh_filtered_lines();
        }
        KeyCode::Backspace
            if app.filter_focus == FilterFocus::DeltaValue && app.delta_filter_active =>
        {
            app.delta_filter_value /= 10;
            if app.delta_filter_value == 0 {
                app.delta_filter_active = false;
            }
            app.refresh_filtered_lines();
        }
        KeyCode::Char('j') | KeyCode::Down => adjust_filter_focus(app, true),
        KeyCode::Char('k') | KeyCode::Up => adjust_filter_focus(app, false),
        KeyCode::Char('c') => {
            app.clear_filter_pane();
        }
        KeyCode::Enter => {
            if app.filter_focus == FilterFocus::DeltaValue && app.delta_filter_active {
                app.refresh_filtered_lines();
            }
            app.focus = Focus::Tree;
        }
        KeyCode::Esc => {
            app.focus = Focus::Tree;
        }
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Char(':') => {
            app.mode = crate::app::AppMode::Command;
            app.clear_command_state();
            app.command_history_idx = None;
            app.update_command_matches();
        }
        _ => {}
    }
}

pub(crate) fn cycle_filter_focus(
    current: FilterFocus,
    skip_time: bool,
    forward: bool,
) -> FilterFocus {
    if skip_time {
        match current {
            FilterFocus::DeltaValue => FilterFocus::DeltaUnit,
            _ => FilterFocus::DeltaValue,
        }
    } else {
        let order = [
            FilterFocus::TimePreset,
            FilterFocus::DeltaValue,
            FilterFocus::DeltaUnit,
        ];
        let pos = order.iter().position(|f| *f == current).unwrap_or(0);
        let next = if forward {
            (pos + 1) % 3
        } else {
            (pos + 2) % 3
        };
        order[next]
    }
}

pub(crate) fn adjust_filter_focus(app: &mut App, forward: bool) {
    let skip_time = app.time_custom;
    match app.filter_focus {
        FilterFocus::TimePreset if !skip_time => {
            let count = crate::app::TIME_PRESET_COUNT;
            let next = if forward {
                (app.time_preset + 1) % count
            } else {
                (app.time_preset + count - 1) % count
            };
            let label = App::time_preset_label(next);
            crate::util::log_msg(
                &app.log_path,
                &format!(
                    "filter: {} key, preset={next} ({label})",
                    if forward { "j" } else { "k" }
                ),
            );
            app.set_time_preset(next);
            app.request_delta_refresh();
        }
        FilterFocus::TimePreset => {}
        FilterFocus::DeltaValue => {
            app.delta_filter_active = true;
            if forward {
                app.delta_filter_inc();
            } else {
                app.delta_filter_dec();
            }
            app.refresh_filtered_lines();
        }
        FilterFocus::DeltaUnit => {
            app.delta_filter_active = true;
            if forward {
                app.delta_filter_cycle_unit();
            } else {
                app.delta_filter_unit = (app.delta_filter_unit + 2) % 3;
            }
            app.refresh_filtered_lines();
        }
    }
}
