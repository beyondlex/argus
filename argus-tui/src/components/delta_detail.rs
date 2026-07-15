use std::collections::HashMap;
use std::path::Path;

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Clear, Row, Table},
    Frame,
};

use argus_core::DeltaEntry;

use crate::app::App;
use crate::components::popup::{popup_block, PopupStyle};
use crate::theme::ColorTheme;
use crate::types::*;
use crate::util::{display_path, format_delta};

/// Load delta detail data for the selected path.
/// Groups raw events by direct child: exact-match → individual rows, deeper → aggregated.
pub fn load_delta_detail(app: &mut App, path: &Path) {
    if !app.server_connected {
        app.delta_detail = Some(DeltaDetailState {
            path: path.to_path_buf(),
            entries: vec![],
            scroll: 0,
        });
        return;
    }

    app.set_info("loading delta detail...".into(), 1);
    let uds_path = crate::config::TuiConfig::default().daemon.uds_path;
    let from = app.time_from;
    let to = app.time_to;
    let tx = app.tx.clone();
    let path_clone = path.to_path_buf();
    let log_path = app.log_path.clone();

    tokio::spawn(async move {
        match fetch_delta_detail(&uds_path, &path_clone, from, to, &log_path).await {
            Some(entries) => {
                let state = build_detail_state(&path_clone, &entries);
                let _ = tx
                    .send(crate::app::AppMessage::DeltaDetailLoaded(state))
                    .await;
            }
            None => {
                let _ = tx
                    .send(crate::app::AppMessage::Error(
                        "failed to fetch delta detail".into(),
                    ))
                    .await;
            }
        }
    });
}

async fn fetch_delta_detail(
    uds: &str,
    path: &Path,
    from: u64,
    to: u64,
    _log_path: &Path,
) -> Option<Vec<DeltaEntry>> {
    let mut client = crate::ipc_client::IpcClient::connect(uds).await.ok()?;
    client.get_delta_detail(path, from, to).await.ok()
}

fn build_detail_state(path: &Path, entries: &[DeltaEntry]) -> DeltaDetailState {
    // Group entries by direct child (first component after selected path)
    let mut exact_events: Vec<DeltaEntry> = Vec::new();
    let mut agg_children: HashMap<String, (i64, u64)> = HashMap::new();

    for entry in entries {
        let Ok(relative) = entry.path.strip_prefix(path) else {
            continue;
        };
        let mut components = relative.components();
        let Some(first) = components.next() else {
            continue;
        };
        let child_name = first.as_os_str().to_string_lossy().to_string();

        if components.next().is_none() {
            exact_events.push(entry.clone());
        } else {
            let (sum, latest) = agg_children.entry(child_name).or_insert((0, 0));
            *sum += entry.delta_size;
            if entry.timestamp > *latest {
                *latest = entry.timestamp;
            }
        }
    }

    let mut rows: Vec<DeltaDetailRow> = Vec::new();

    for entry in &exact_events {
        let child_name = entry
            .path
            .strip_prefix(path)
            .ok()
            .and_then(|r| r.components().next())
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .unwrap_or_default();
        rows.push(DeltaDetailRow {
            timestamp: format_timestamp(entry.timestamp),
            child_name,
            delta_size: entry.delta_size,
            delta_display: format_delta(entry.delta_size),
            is_aggregated: false,
        });
    }

    for (child_name, (sum, ts)) in &agg_children {
        rows.push(DeltaDetailRow {
            timestamp: format_timestamp(*ts),
            child_name: child_name.clone(),
            delta_size: *sum,
            delta_display: format_delta(*sum),
            is_aggregated: true,
        });
    }

    rows.sort_by(|a, b| {
        b.timestamp
            .cmp(&a.timestamp)
            .then_with(|| a.child_name.cmp(&b.child_name))
    });

    DeltaDetailState {
        path: path.to_path_buf(),
        entries: rows,
        scroll: 0,
    }
}

fn format_timestamp(ts_ms: u64) -> String {
    let secs = (ts_ms / 1000) as i64;
    let nanos = ((ts_ms % 1000) * 1_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// Render the delta detail popup
pub fn render(f: &mut Frame, area: Rect, state: &DeltaDetailState, theme: &ColorTheme) {
    let popup = crate::components::popup::centered_rect(70, 65, area);
    f.render_widget(Clear, popup);

    let entry_count = state.entries.len();
    let total_title = format!(" Delta Events for: {} ", display_path(&state.path));
    let visible_rows = (popup.height as usize).saturating_sub(4);
    let needs_scroll = entry_count > visible_rows;
    let footer = if state.entries.is_empty() {
        " [Esc close] ".into()
    } else if needs_scroll {
        let bottom = (state.scroll + visible_rows).min(entry_count);
        let pct = bottom * 100 / entry_count.max(1);
        format!(
            " {} entries ({pct}%) · [j/k scroll · Esc close] ",
            entry_count
        )
    } else {
        format!(" {} entries · [Esc close] ", entry_count)
    };
    let block = popup_block(total_title, PopupStyle::Normal, theme)
        .title_bottom(Line::from(footer).right_aligned());

    let inner = block.inner(popup);
    let scroll = state.scroll;

    let widths = [
        Constraint::Length(22),
        Constraint::Fill(1),
        Constraint::Length(15),
    ];

    let header = Row::new(vec![
        Cell::from(Span::styled(
            "Time",
            Style::default()
                .fg(theme.text_tertiary)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Path",
            Style::default()
                .fg(theme.text_tertiary)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "      Delta",
            Style::default()
                .fg(theme.text_tertiary)
                .add_modifier(Modifier::BOLD),
        )),
    ])
    .style(Style::default().bg(theme.popup_bg))
    .height(1);

    let mut rows: Vec<Row> = Vec::new();

    if !state.entries.is_empty() {
        let visible_count = (inner.height as usize).saturating_sub(2);
        for i in scroll..(scroll + visible_count).min(state.entries.len()) {
            let row = &state.entries[i];
            let prefix = if row.is_aggregated { "- " } else { "  " };
            let ts_style = Style::default().fg(theme.text_secondary);
            let path_style = Style::default().fg(theme.text_highlight);
            let delta_style = if row.delta_size >= 0 {
                Style::default().fg(theme.danger)
            } else {
                Style::default().fg(theme.success)
            };

            rows.push(
                Row::new(vec![
                    Cell::from(Line::from(vec![
                        Span::styled(prefix, Style::default().fg(theme.text_tertiary)),
                        Span::styled(&row.timestamp, ts_style),
                    ])),
                    Cell::from(Span::styled(&row.child_name, path_style)),
                    Cell::from(Span::styled(
                        format!("{:>12  }", &row.delta_display),
                        delta_style,
                    )),
                ])
                .style(Style::default().bg(theme.popup_bg))
                .height(1),
            );
        }
    }

    let table = Table::new(rows, widths).header(header).block(block);
    f.render_widget(table, popup);
}
