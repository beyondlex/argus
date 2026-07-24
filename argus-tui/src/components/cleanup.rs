use crate::app::App;
use crate::render::SPINNER_FRAMES;
use crate::theme::ColorTheme;
use crate::types::{CleanupMode, CleanupState, UninstallPhase, UninstallState};
use crate::util::{format_relative_time, format_size, key_hints};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn render_cleanup(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(ref state) = app.cleanup_state.clone() else { return };
    let theme = &app.theme;

    let mode_label = match state.mode {
        CleanupMode::Clean => "Clean",
        CleanupMode::Purge => "Purge",
    };

    let title = if state.scanning {
        format!(" Argus {} [{}] ", mode_label, SPINNER_FRAMES[app.scan_spinner as usize % SPINNER_FRAMES.len()])
    } else if state.report.is_some() {
        format!(" Argus {} (complete) ", mode_label)
    } else {
        format!(" Argus {} ", mode_label)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border_normal))
        .style(Style::default().bg(theme.popup_bg))
        .title_style(Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))
        .title_alignment(Alignment::Center)
        .title(title);

    let footer = if state.scanning {
        Line::from(Span::styled(
            " Scanning... ",
            Style::default().fg(theme.text_tertiary),
        ))
    } else if state.report.is_some() {
        let r = state.report.as_ref().unwrap();
        let status = if r.total_failed > 0 {
            format!(" {} succeeded, {} failed, {} freed ",
                r.total_succeeded, r.total_failed, format_size(r.freed_bytes))
        } else {
            format!(" {} succeeded, {} freed ",
                r.total_succeeded, format_size(r.freed_bytes))
        };
        Line::from(Span::styled(status, Style::default().fg(if r.total_failed > 0 { theme.danger } else { theme.success })))
            .alignment(Alignment::Center)
    } else {
        Line::from(
            key_hints(
                &[("j/k", "Move"), ("Space", "Toggle"), ("i", "Details"), ("Enter", "Execute"), ("d", "Dry-run"), ("Esc", "Back")],
                theme,
            ),
        )
        .alignment(Alignment::Center)
    };
    let block = block.title_bottom(footer);

    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    if state.scanning {
        let spinner = SPINNER_FRAMES[app.scan_spinner as usize % SPINNER_FRAMES.len()];
        let mut scan_lines = vec![
            Line::from(Span::styled(
                format!(" {} Scanning... ", spinner),
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            )).alignment(Alignment::Center),
            Line::from(""),
        ];
        if let Some(ref path) = state.scan_current_path {
            let display_path = path.replacen(
                &std::path::Path::new("/Users").join(whoami()).to_string_lossy().to_string(),
                "~",
                1,
            );
            scan_lines.push(
                Line::from(Span::styled(
                    format!(" {}", display_path),
                    Style::default().fg(theme.text_secondary),
                )).alignment(Alignment::Center),
            );
        } else {
            scan_lines.push(
                Line::from(Span::styled(
                    " discovering targets... ",
                    Style::default().fg(theme.text_tertiary),
                )).alignment(Alignment::Center),
            );
        }
        scan_lines.push(Line::from(""));
        scan_lines.push(
            Line::from(Span::styled(
                " This may take a moment depending on the size of your caches ",
                Style::default().fg(theme.text_tertiary),
            )).alignment(Alignment::Center),
        );
        f.render_widget(
            Paragraph::new(scan_lines).alignment(Alignment::Center),
            inner,
        );
        return;
    }

    if let Some(ref report) = state.report {
        render_cleanup_report(f, inner, report, theme);
        return;
    }

    if state.detail_pending {
        let detail_items = state.detail_items.as_deref().unwrap_or(&[]);
        let item = state.items.get(state.cursor);
        let home = std::path::Path::new("/Users").join(whoami()).to_string_lossy().to_string();
        let full_path = item
            .map(|i| i.path.to_string_lossy().to_string().replacen(&home, "~", 1))
            .unwrap_or_default();
        let total_size = item.map(|i| i.size).unwrap_or(0);
        let popup_area = centered_rect(inner, 55, 60);
        let available = popup_area.height.saturating_sub(4) as usize;
        let lines: Vec<Line> = if detail_items.is_empty() {
            vec![Line::from(Span::styled(
                " Scanning... ",
                Style::default().fg(theme.text_tertiary),
            ))]
        } else {
            let mut lines = vec![
                Line::from(Span::styled(
                    format!(" {} ", full_path),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(" {} total, {} items ", format_size(total_size), detail_items.len()),
                    Style::default().fg(theme.text_tertiary),
                )),
                Line::from(""),
            ];

            let max_visible = available.saturating_sub(3);
            for (name, size) in detail_items.iter().take(max_visible) {
                let size_str = format_size(*size);
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:>10}", size_str),
                        Style::default().fg(theme.text_highlight),
                    ),
                    Span::raw("  "),
                    Span::styled(name.clone(), Style::default().fg(theme.text)),
                ]));
            }
            let omitted = detail_items.len().saturating_sub(max_visible);
            if omitted > 0 {
                lines.push(Line::from(Span::styled(
                    format!(" ... {} more items (scroll not available) ", omitted),
                    Style::default().fg(theme.text_tertiary),
                )));
            }
            lines
        };

        let detail_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border_normal))
            .style(Style::default().bg(theme.popup_bg))
            .title(" Preview ")
            .title_alignment(Alignment::Center)
            .title_bottom(
                Line::from(key_hints(&[("Esc", "Close")], theme)).centered(),
            );
        f.render_widget(Clear, popup_area);
        f.render_widget(
            Paragraph::new(lines).block(detail_block).wrap(Wrap { trim: false }),
            popup_area,
        );
        return;
    }

    if state.confirm_pending {
        let selected_count = state.selected.len();
        let selected_total: u64 = state
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| state.selected.contains(i))
            .map(|(_, item)| item.size)
            .sum();
        let home = std::path::Path::new("/Users").join(whoami()).to_string_lossy().to_string();
        let mut confirm_lines = vec![
            Line::from(Span::styled(
                format!("Delete {} item(s) ({} {})?", selected_count, format_size(selected_total), if state.dry_run { "dry-run" } else { "to trash" }),
                Style::default().fg(theme.text),
            )),
            Line::from(""),
        ];
        let selected_items: Vec<_> = state
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| state.selected.contains(i))
            .collect();
        for (_, item) in selected_items.iter().take(3) {
            let path_str = item.path.to_string_lossy().to_string().replacen(&home, "~", 1);
            confirm_lines.push(Line::from(vec![
                Span::styled(
                    format!("{:>10}", format_size(item.size)),
                    Style::default().fg(theme.text_highlight),
                ),
                Span::raw("  "),
                Span::styled(path_str, Style::default().fg(theme.text)),
            ]));
        }
        let omitted = selected_items.len().saturating_sub(3);
        if omitted > 0 {
            confirm_lines.push(Line::from(Span::styled(
                format!("... {} more", omitted),
                Style::default().fg(theme.text_tertiary),
            )));
        }

        let confirm_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.danger))
            .style(Style::default().bg(theme.popup_bg))
            .title(" Confirm ")
            .title_alignment(Alignment::Center)
            .title_bottom(
                Line::from(key_hints(&[("y", "Yes"), ("n", "Cancel")], theme)).centered(),
            );
        let confirm_area = centered_rect(inner, 55, 35);
        f.render_widget(Clear, confirm_area);
        f.render_widget(
            Paragraph::new(confirm_lines).block(confirm_block),
            confirm_area,
        );
        return;
    }

    let [summary_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);

    render_cleanup_summary(f, summary_area, state, theme);
    render_cleanup_list(f, list_area, state, app, theme);
}

fn render_cleanup_summary(f: &mut Frame, area: Rect, state: &CleanupState, theme: &ColorTheme) {
    let selected_total: u64 = state
        .items
        .iter()
        .enumerate()
        .filter(|(i, _)| state.selected.contains(i))
        .map(|(_, item)| item.size)
        .sum();
    let dry_run_label = if state.dry_run { " [DRY-RUN] " } else { "" };
    let text = format!(
        " Total: {}  |  Selected: {} ({}){}  |  {} target(s)",
        format_size(state.total_bytes),
        state.selected.len(),
        format_size(selected_total),
        dry_run_label,
        state.items.len(),
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(text, Style::default().fg(theme.text_secondary)))),
        area,
    );
}

fn render_cleanup_list(f: &mut Frame, area: Rect, state: &CleanupState, _app: &App, theme: &ColorTheme) {
    let visible_count = state.items.len();
    let max_scroll = visible_count.saturating_sub(area.height as usize);
    let scroll = state.scroll_offset.min(max_scroll);

    let home = std::path::Path::new("/Users")
        .join(whoami())
        .to_string_lossy()
        .to_string();

    let items: Vec<Line> = state
        .items
        .iter()
        .enumerate()
        .skip(scroll)
        .take(area.height as usize)
        .map(|(i, item)| {
            let is_selected = state.selected.contains(&i);
            let is_cursor = i == state.cursor;
            let checkbox = if is_selected { "[x]" } else { "[ ]" };
            let prefix = if is_cursor { ">" } else { " " };
            let path_str = item.path.to_string_lossy().to_string();
            let display_path = path_str.replacen(&home, "~", 1);
            let size_str = format_size(item.size);

            let style = if is_cursor {
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            let checkbox_style = if is_selected {
                Style::default().fg(theme.success)
            } else {
                Style::default().fg(theme.text_tertiary)
            };

            Line::from(vec![
                Span::styled(format!("{} ", prefix), style),
                Span::styled(checkbox, checkbox_style),
                Span::raw(" "),
                Span::styled(
                    format!("{:>10}", size_str),
                    Style::default().fg(theme.text_highlight),
                ),
                Span::raw("  "),
                Span::styled(display_path, style),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(items), area);
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".to_string())
}

fn render_cleanup_report(f: &mut Frame, area: Rect, report: &argus_core::CleanReport, _theme: &ColorTheme) {
    let text = format!(
        "Clean complete!\n\nAttempted: {}\nSucceeded: {}\nFailed: {}\nFreed: {}",
        report.total_attempted,
        report.total_succeeded,
        report.total_failed,
        format_size(report.freed_bytes),
    );
    f.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── Uninstall Panel ─────────────────────────────────────────────────

pub fn render_uninstall(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(ref state) = app.uninstall_state.clone() else { return };
    let theme = &app.theme;

    let title = if state.scanning {
        format!(" Argus Uninstall [{}] ", SPINNER_FRAMES[app.scan_spinner as usize % SPINNER_FRAMES.len()])
    } else if state.report.is_some() {
        " Argus Uninstall (complete) ".to_string()
    } else {
        " Argus Uninstall ".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border_normal))
        .style(Style::default().bg(theme.popup_bg))
        .title_style(Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))
        .title_alignment(Alignment::Center)
        .title(title);

    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    if state.scanning {
        if let Some(ref path) = state.scan_current_path {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" Scanning {} ", path),
                    Style::default().fg(theme.text_secondary),
                )))
                .alignment(Alignment::Center),
                inner,
            );
        }
        return;
    }

    if let Some(ref report) = state.report {
        render_cleanup_report(f, inner, report, theme);
        return;
    }

    if state.confirm_pending {
        let app_idx = state.selected_app.unwrap_or(0);
        let app_name = state.apps.get(app_idx).map(|a| a.name.as_str()).unwrap_or("?");
        let confirm_text = format!("Uninstall {} and remove leftovers?", app_name);
        let confirm_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.danger))
            .style(Style::default().bg(theme.popup_bg))
            .title(" Confirm ")
            .title_alignment(Alignment::Center)
            .title_bottom(
                Line::from(key_hints(&[("y", "Yes"), ("n", "Cancel")], theme)).centered(),
            );
        let confirm_area = centered_rect(inner, 50, 30);
        f.render_widget(Clear, confirm_area);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                confirm_text,
                Style::default().fg(theme.text),
            )))
            .block(confirm_block)
            .alignment(Alignment::Center),
            confirm_area,
        );
        return;
    }

    match state.phase {
        UninstallPhase::SelectApp => render_uninstall_select(f, inner, state, app, theme),
        UninstallPhase::Confirm => render_uninstall_confirm(f, inner, state, app, theme),
    }
}

fn render_uninstall_select(f: &mut Frame, area: Rect, state: &UninstallState, _app: &App, theme: &ColorTheme) {
    let [search_area, list_area, footer_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);

    let search_text = if state.filter_mode {
        format!(" /{} ", state.search_word)
    } else if state.search_word.is_empty() {
        " Search: press / to filter ".to_string()
    } else {
        format!(" Search: {} ", state.search_word)
    };
    let search_style = if state.filter_mode {
        Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
    } else if !state.search_word.is_empty() {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.text_tertiary)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(search_text, search_style))),
        search_area,
    );

    let scroll = state.cursor.saturating_sub(area.height as usize / 2);

    let items: Vec<Line> = state
        .filtered
        .iter()
        .enumerate()
        .skip(scroll)
        .take(list_area.height as usize)
        .map(|(display_i, &app_i)| {
            let app_info = &state.apps[app_i];
            let is_cursor = display_i == state.cursor;
            let is_top = Some(app_i) == state.filtered.first().copied();
            let prefix = if is_cursor { ">" } else { " " };
            let size_str = format_size(app_info.size);
            let time_str = format_relative_time(app_info.last_used);

            let style = if is_cursor {
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            let sort_highlight = Style::default().fg(theme.success);

            let time_style = if is_top && state.sort_mode == 1 {
                sort_highlight
            } else {
                Style::default().fg(theme.text_tertiary)
            };
            let size_style = if is_top && state.sort_mode == 0 {
                sort_highlight
            } else {
                Style::default().fg(theme.text_highlight)
            };
            let name_style = if is_top && state.sort_mode == 2 {
                sort_highlight
            } else {
                style
            };

            Line::from(vec![
                Span::styled(format!("{} ", prefix), style),
                Span::styled(time_str, time_style),
                Span::raw(" "),
                Span::styled(format!("{:>10}", size_str), size_style),
                Span::raw("  "),
                Span::styled(app_info.name.clone(), name_style),
                Span::raw("  "),
                Span::styled(
                    app_info.id.clone(),
                    Style::default().fg(theme.text_tertiary),
                ),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(items), list_area);

    let sort_label = match state.sort_mode {
        0 => "size",
        1 => "time",
        _ => "name",
    };
    let total_size: u64 = state.apps.iter().map(|a| a.size).sum();
    let footer_text = format!(
        " {} app(s) {}  |  sort: {}  |  j/k move  o toggle sort  Enter select  / filter  Esc back ",
        state.apps.len(),
        format_size(total_size),
        sort_label,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(footer_text, Style::default().fg(theme.text_tertiary)))),
        footer_area,
    );
}

fn render_uninstall_confirm(f: &mut Frame, area: Rect, state: &UninstallState, _app: &App, theme: &ColorTheme) {
    let Some(app_idx) = state.selected_app else { return };
    let Some(ref app_info) = state.apps.get(app_idx) else { return };

    let [detail_area, leftover_label_area, leftover_list_area, toggle_area, footer_area] =
        Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

    // App detail
    let detail = format!(
        "App:  {}  ({})\nID:   {}\nSize: {}",
        app_info.name,
        format_size(app_info.size),
        app_info.id,
        format_size(app_info.size),
    );
    f.render_widget(
        Paragraph::new(detail).wrap(Wrap { trim: false }),
        detail_area,
    );

    if let Some(ref leftovers) = state.leftovers {
        let leftover_size = format_size(leftovers.total_leftover_bytes);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" Leftovers: {}  ", leftover_size),
                Style::default().fg(theme.text_secondary),
            ))),
            leftover_label_area,
        );

        let scroll = state.cursor.saturating_sub(leftover_list_area.height as usize / 2);
        let items: Vec<Line> = leftovers
            .leftover_paths
            .iter()
            .enumerate()
            .skip(scroll)
            .take(leftover_list_area.height as usize)
            .map(|(i, path)| {
                let is_selected = state.selected_leftovers.contains(&i);
                let is_cursor = i == state.cursor;
                let checkbox = if is_selected { "[x]" } else { "[ ]" };
                let prefix = if is_cursor { ">" } else { " " };

                let style = if is_cursor {
                    Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                };

                Line::from(vec![
                    Span::styled(format!("{} ", prefix), style),
                    Span::styled(checkbox, if is_selected {
                        Style::default().fg(theme.success)
                    } else {
                        Style::default().fg(theme.text_tertiary)
                    }),
                    Span::raw(" "),
                    Span::styled(path.to_string_lossy(), style),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(items), leftover_list_area);
    } else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " Scanning for leftovers... ",
                Style::default().fg(theme.text_tertiary),
            ))),
            leftover_label_area,
        );
    }


    // Remove leftovers toggle
    let toggle_label = if state.remove_leftovers {
        "[x] Remove leftovers (recommended)"
    } else {
        "[ ] Remove leftovers"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            toggle_label,
            Style::default().fg(if state.remove_leftovers { theme.success } else { theme.text }),
        ))),
        toggle_area,
    );

    // Footer
    let footer_hints = key_hints(
        &[("j/k", "Move"), ("Space", "Toggle"), ("Enter", "Uninstall"), ("Esc", "Back")],
        theme,
    );
    f.render_widget(
        Paragraph::new(Line::from(footer_hints)).alignment(Alignment::Center),
        footer_area,
    );
}

fn centered_rect(parent: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let popup_area = crate::components::popup::centered_rect(percent_x, percent_y, parent);
    Rect {
        x: popup_area.x,
        y: popup_area.y,
        width: popup_area.width.min(parent.width),
        height: popup_area.height.min(parent.height),
    }
}
