use crate::app::App;
use crate::handler::brew::format_brew_time;
use crate::render::SPINNER_FRAMES;
use crate::theme::ColorTheme;
use crate::types::{BrewFilterType, BrewSortMode, BrewState};
use crate::util::{format_size, key_hints};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render_brew(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(ref state) = app.brew_state.clone() else {
        return;
    };
    let theme = &app.theme;

    let title = if state.scanning {
        format!(
            " Argus Brew [{}] ",
            SPINNER_FRAMES[app.scan_spinner as usize % SPINNER_FRAMES.len()]
        )
    } else if state.report.is_some() {
        " Argus Brew (complete) ".to_string()
    } else {
        " Argus Brew ".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border_normal))
        .style(Style::default().bg(theme.popup_bg))
        .title_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
        .title_alignment(Alignment::Center)
        .title(title);

    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    if state.scanning {
        let spinner = SPINNER_FRAMES[app.scan_spinner as usize % SPINNER_FRAMES.len()];
        let target = if state.current_scan_target.is_empty() {
            String::new()
        } else {
            format!(" {}", state.current_scan_target)
        };
        let progress = if state.scan_progress_total > 0 {
            let pct = state.scan_progress_current * 100 / state.scan_progress_total;
            format!(" [{}/{}]{}%", state.scan_progress_current, state.scan_progress_total, pct)
        } else {
            String::new()
        };
        let lines = vec![
            Line::from(Span::styled(
                format!("{} Scanning{}", spinner, target),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
            Line::from(""),
            Line::from(Span::styled(
                progress,
                Style::default().fg(theme.text_tertiary),
            ))
            .alignment(Alignment::Center),
        ];
        let [_, center, _] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(3),
            Constraint::Fill(1),
        ])
        .areas(inner);
        f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), center);
        return;
    }

    if state.uninstalling {
        let spinner = SPINNER_FRAMES[app.scan_spinner as usize % SPINNER_FRAMES.len()];
        let pkg_name = state
            .selected_pkg
            .and_then(|i| state.filtered.get(i).copied())
            .and_then(|i| state.packages.get(i))
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        let lines = vec![
            Line::from(Span::styled(
                format!("{} Uninstalling {}...", spinner, pkg_name),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center),
        ];
        let [_, center, _] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(inner);
        f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), center);
        return;
    }

    if let Some(ref report) = state.report {
        render_brew_report(f, inner, report, theme);
        return;
    }

    if state.confirm_pending {
        let pkg_idx = state.selected_pkg.unwrap_or(0);
        let pkg = state
            .filtered
            .get(pkg_idx)
            .and_then(|&i| state.packages.get(i));
        let pkg_name = pkg.map(|p| p.name.as_str()).unwrap_or("?");
        let confirm_text = format!("Uninstall {}?", pkg_name);

        let mut lines = vec![
            Line::from(Span::styled(confirm_text, Style::default().fg(theme.text))),
        ];

        let confirm_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.danger))
            .style(Style::default().bg(theme.popup_bg))
            .title(" Confirm ")
            .title_alignment(Alignment::Center)
            .title_bottom(
                Line::from(key_hints(&[("y", "Yes"), ("n", "Cancel")], theme)).centered(),
            );
        let confirm_area = centered_rect(inner, 70, 50);
        f.render_widget(Clear, confirm_area);
        f.render_widget(
            Paragraph::new(lines).block(confirm_block).alignment(Alignment::Center),
            confirm_area,
        );
        return;
    }

    // Main list view
    let [header_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    let prefix = argus_core::brew_prefix();
    let analyzed: std::collections::HashSet<usize> = state
        .packages
        .iter()
        .enumerate()
        .filter(|(_, pkg)| {
            let path = argus_core::keg_path(&prefix, &pkg.name, &pkg.package_type)
                .join(&pkg.version);
            app.ai_analyzed.contains_key(&path)
        })
        .map(|(i, _)| i)
        .collect();

    render_brew_header(f, header_area, &state, theme);
    render_brew_list(f, list_area, &state, theme, &analyzed);
    render_brew_footer(f, footer_area, &state, theme);
}

fn render_brew_header(f: &mut Frame, area: Rect, state: &BrewState, theme: &ColorTheme) {
    let filter_label = match state.filter_type {
        BrewFilterType::All => "All",
        BrewFilterType::Formula => "Formula",
        BrewFilterType::Cask => "Cask",
    };
    let sort_label = match state.sort_mode {
        BrewSortMode::Time => "time",
        BrewSortMode::Size => "size",
        BrewSortMode::Name => "name",
        BrewSortMode::Type => "type",
    };

    let search_text = if state.filter_mode {
        format!(" /{}", state.search_word)
    } else if state.search_word.is_empty() {
        " Search: press / to filter".to_string()
    } else {
        format!(" Search: {}", state.search_word)
    };
    let search_style = if state.filter_mode {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else if !state.search_word.is_empty() {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.text_tertiary)
    };

    let [search_area, stats_area] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(30)]).areas(area);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(search_text, search_style))),
        search_area,
    );

    let stats = format!(
        "Cache: {}  Sort: {}  Type: {}",
        format_size(state.cache_size),
        sort_label,
        filter_label,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            stats,
            Style::default().fg(theme.text_secondary),
        )))
        .alignment(Alignment::Right),
        stats_area,
    );
}

fn render_brew_list(
    f: &mut Frame,
    area: Rect,
    state: &BrewState,
    theme: &ColorTheme,
    analyzed: &std::collections::HashSet<usize>,
) {
    if state.filtered.is_empty() {
        let msg = if state.packages.is_empty() {
            " No brew packages found "
        } else {
            " No packages match filter "
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg,
                Style::default().fg(theme.text_tertiary),
            )))
            .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let scroll = state.cursor.saturating_sub(area.height as usize / 2);

    let items: Vec<Line> = state
        .filtered
        .iter()
        .enumerate()
        .skip(scroll)
        .take(area.height as usize)
        .map(|(display_i, &pkg_i)| {
            let pkg = &state.packages[pkg_i];
            let is_cursor = display_i == state.cursor;
            let prefix = if is_cursor { ">" } else { " " };
            let is_selected = state.multi_select && state.selected_pkgs.contains(&pkg_i);
            let style = if is_cursor {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            let prefix_span = if state.multi_select {
                Span::styled(
                    if is_selected { "● " } else { "○ " },
                    if is_selected {
                        Style::default().fg(theme.success)
                    } else {
                        Style::default().fg(theme.text_tertiary)
                    },
                )
            } else {
                Span::styled(format!("{} ", prefix), style)
            };

            let type_label = pkg.package_type.label();
            let time_str = format_brew_time(pkg.last_used);
            let size_str = format_size(pkg.size);
            let size_style = match crate::util::size_unit_index(pkg.size) {
                0 | 1 => Style::default().fg(theme.success),
                2 => Style::default().fg(theme.warning),
                _ => Style::default().fg(theme.danger),
            };

            // Color code the time: never = red, old = yellow, recent = green
            let time_style = match pkg.last_used {
                None => Style::default().fg(theme.danger),
                Some(dt) => {
                    let days = (chrono::Utc::now() - dt).num_days();
                    if days > 180 {
                        Style::default().fg(theme.danger)
                    } else if days > 30 {
                        Style::default().fg(theme.warning)
                    } else {
                        Style::default().fg(theme.success)
                    }
                }
            };

            let desc_display = if pkg.description.len() > 30 {
                format!("{}...", &pkg.description[..27])
            } else {
                pkg.description.clone()
            };
            let has_ai = analyzed.contains(&pkg_i);

            Line::from(vec![
                prefix_span,
                Span::styled(format!("{:>12}", time_str), time_style),
                Span::raw(" "),
                Span::styled(
                    format!("{:>9}", size_str),
                    size_style,
                ),
                Span::raw("  "),
                Span::styled(
                    format!("[{}]", type_label),
                    Style::default().fg(theme.text_tertiary),
                ),
                Span::raw(" "),
                Span::styled(pkg.name.clone(), style),
                Span::raw(" "),
                Span::styled(
                    if has_ai { "⚡" } else { "" },
                    Style::default().fg(theme.warning),
                ),
                Span::raw("  "),
                Span::styled(desc_display, Style::default().fg(theme.text_tertiary)),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(items), area);
}

fn render_brew_footer(f: &mut Frame, area: Rect, state: &BrewState, theme: &ColorTheme) {
    let total_size: u64 = state.filtered.iter().map(|&i| state.packages[i].size).sum();
    let mut spans = vec![Span::styled(
        format!(
            " {} pkg(s) {}  |  ",
            state.filtered.len(),
            format_size(total_size)
        ),
        Style::default().fg(theme.text_tertiary),
    )];
    if state.multi_select {
        let sel_count = state.selected_pkgs.len();
        let sel_size: u64 = state
            .selected_pkgs
            .iter()
            .filter_map(|&i| state.packages.get(i))
            .map(|p| p.size)
            .sum();
        spans.push(Span::styled(
            format!(" ● {}({})  |  ", sel_count, format_size(sel_size)),
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        ));
        spans.extend(key_hints(
            &[
                ("Space", "Select"),
                ("a", "AI"),
                ("Esc", "Exit"),
            ],
            theme,
        ));
    } else {
        spans.extend(key_hints(
            &[
                ("j/k", "Move"),
                ("Space", "M-Select"),
                ("Enter", "Uninstall"),
                ("o", "Sort"),
                ("t", "Type"),
                ("/", "Filter"),
                ("Esc", "Back"),
            ],
            theme,
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_brew_report(
    f: &mut Frame,
    area: Rect,
    report: &argus_core::CleanReport,
    theme: &ColorTheme,
) {
    let has_errors = report.total_failed > 0;
    let title = if has_errors {
        " Brew uninstall failed "
    } else {
        " Brew uninstall complete "
    };
    let title_color = if has_errors {
        theme.danger
    } else {
        theme.success
    };

    let mut lines = vec![
        Line::from(Span::styled(
            title,
            Style::default()
                .fg(title_color)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Freed: ", Style::default().fg(theme.text_secondary)),
            Span::styled(
                format_size(report.freed_bytes),
                Style::default()
                    .fg(theme.text_highlight)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Succeeded: ", Style::default().fg(theme.text_secondary)),
            Span::styled(
                report.total_succeeded.to_string(),
                Style::default().fg(theme.success),
            ),
            Span::raw("  "),
            Span::styled("Failed: ", Style::default().fg(theme.text_secondary)),
            Span::styled(
                report.total_failed.to_string(),
                if has_errors {
                    Style::default().fg(theme.danger)
                } else {
                    Style::default().fg(theme.success)
                },
            ),
        ]),
    ];

    for (_, err_msg) in &report.errors {
        lines.push(Line::from(""));
        for wrapped_line in word_wrap(err_msg, 60) {
            lines.push(Line::from(Span::styled(
                wrapped_line,
                Style::default()
                    .fg(theme.danger)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Press Esc to return ",
        Style::default().fg(theme.text_tertiary),
    )));

    let content_height = lines.len() as u16;
    let [_, center, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(content_height),
        Constraint::Fill(1),
    ])
    .areas(area);
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), center);
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

fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.len() + word.len() + 1 > max_width && !line.is_empty() {
            result.push(line.clone());
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        result.push(line);
    }
    result
}
