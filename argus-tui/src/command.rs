use crate::app::App;
use crate::time_utils::*;
use crate::types::*;
use ratatui::widgets::BorderType;
use ratatui_finder::{FinderColors, FinderConfig, FinderMode, FinderState};

impl App {
    pub const COMMANDS: &'static [&'static str] = &[
        "Clean",
        "Consolidate",
        "Delta",
        "Finder",
        "Help",
        "Purge",
        "Scan",
        "Sort",
        "Time",
        "Uninstall",
    ];

    pub fn update_command_matches(&mut self) {
        if self.command_input.is_empty() {
            self.command_matches = Self::COMMANDS.to_vec();
        } else {
            let lower = self.command_input.to_lowercase();
            self.command_matches = Self::COMMANDS
                .iter()
                .filter(|&c| crate::search::fuzzy_match(&lower, &c.to_lowercase()))
                .copied()
                .collect();
        }
        if self.command_selected >= self.command_matches.len() {
            self.command_selected = 0;
        }
    }

    pub fn clear_command_state(&mut self) {
        self.command_input.clear();
        self.command_matches.clear();
        self.command_selected = 0;
        self.command_scroll = 0;
    }

    pub fn push_command_history(&mut self, cmd: &str) {
        let cmd = cmd.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        if self.command_history.last().map(|s: &String| s.as_str()) == Some(cmd.as_str()) {
            return;
        }
        self.command_history.push(cmd);
        if self.command_history.len() > 50 {
            self.command_history.remove(0);
        }
        self.command_history_idx = None;
    }

    pub fn execute_command(&mut self, cmd: &str) -> Result<String, String> {
        let trimmed = cmd.trim();
        if trimmed.is_empty() {
            return Err("empty command".into());
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let name = parts[0];
        let name_lower = name.to_lowercase();

        match name_lower.as_str() {
            "help" => self.cmd_help(),
            "delta" => self.cmd_delta(&parts),
            "time" => self.cmd_time(&parts),
            "sort" | "s" => self.cmd_sort(&parts),
            "sd" => self.cmd_sort_quick(SortMode::Delta, "Delta"),
            "ss" => self.cmd_sort_quick(SortMode::Size, "Size"),
            "sn" => self.cmd_sort_quick(SortMode::Name, "Name"),
            "finder" => self.cmd_finder(),
            "scan" => self.cmd_scan(),
            "consolidate" => self.cmd_consolidate(),
            _ => Err(format!("unknown command: {name}")),
        }
    }

    pub(crate) fn cmd_help(&mut self) -> Result<String, String> {
        self.mode = AppMode::Help;
        Ok("help opened".into())
    }

    pub(crate) fn cmd_delta(&mut self, parts: &[&str]) -> Result<String, String> {
        if !self.server_mode {
            return Err("not in server mode".into());
        }
        let (num_str, unit) = match parts.get(1).copied() {
            Some(arg) if arg.ends_with('k') || arg.ends_with('K') => {
                (&arg[..arg.len() - 1], 0usize)
            }
            Some(arg) if arg.ends_with('m') || arg.ends_with('M') => {
                (&arg[..arg.len() - 1], 1usize)
            }
            Some(arg) if arg.ends_with('g') || arg.ends_with('G') => {
                (&arg[..arg.len() - 1], 2usize)
            }
            Some(arg) => (arg, 1usize),
            None => ("0", 0usize),
        };
        let value: u64 = num_str
            .parse()
            .map_err(|_| format!("invalid number: {num_str}"))?;
        self.delta_filter_active = true;
        self.delta_filter_value = value;
        self.delta_filter_unit = unit;
        self.refresh_current_filtered();
        Ok(format!(
            "delta filter set to {}{}",
            value,
            ["KB", "MB", "GB"][unit]
        ))
    }

    pub(crate) fn cmd_time(&mut self, parts: &[&str]) -> Result<String, String> {
        if !self.server_mode {
            return Err("not in server mode".into());
        }
        let arg = match parts.get(1) {
            Some(a) => *a,
            None => {
                self.mode = AppMode::TimeHelp;
                return Ok("".into());
            }
        };
        let rest: String = parts[1..].join(" ");
        let to_lower = rest.to_lowercase();
        let to_marker = " to ";
        if let Some(pos) = to_lower.find(to_marker) {
            let left = rest[..pos].trim();
            let right = rest[pos + to_marker.len()..].trim();
            if left.is_empty() || right.is_empty() {
                return Err("invalid time range: empty side".into());
            }
            let left_parsed = parse_single_time_arg(left)?;
            let (to_ms, right_label) = if is_time_only(right) {
                let date = left_parsed
                    .date
                    .ok_or("cannot inherit date for time-only right side")?;
                let parts: Vec<&str> = right.split(':').collect();
                let h: u32 = parts[0].parse().unwrap_or(0);
                let min: u32 = parts[1].parse().unwrap_or(0);
                (
                    datetime_to_millis(date.0, date.1, h, min),
                    format!("{:02}:{:02}", h, min),
                )
            } else {
                let parsed = parse_single_time_arg(right)?;
                (parsed.ms, parsed.label)
            };
            self.time_from = left_parsed.ms;
            self.time_to = to_ms;
            self.time_custom = true;
            self.time_custom_label = format_time_label(&left_parsed.label, &right_label);
            self.request_delta_refresh();
            Ok(format!("time range: {}", self.time_custom_label))
        } else {
            let parsed = parse_single_time_arg(arg)?;
            let now = now_in_millis();
            self.time_from = parsed.ms;
            self.time_to = now;
            self.time_custom = true;
            if parsed.date.is_some() {
                self.time_custom_label = format!("{} ~ now", parsed.label);
            } else {
                self.time_custom_label = parsed.label;
            }
            self.request_delta_refresh();
            if parsed.date.is_some() {
                Ok(format!("time range: {}", self.time_custom_label))
            } else {
                Ok(format!("time range: in {}", self.time_custom_label))
            }
        }
    }

    pub(crate) fn cmd_sort(&mut self, parts: &[&str]) -> Result<String, String> {
        let sub = parts.get(1).copied().unwrap_or("");
        match sub.to_lowercase().as_str() {
            "d" | "delta" => self.sort_mode = SortMode::Delta,
            "s" | "size" => self.sort_mode = SortMode::Size,
            "n" | "name" => self.sort_mode = SortMode::Name,
            "" => self.sort_mode = self.sort_mode.toggle(),
            _ => return Err(format!("unknown sort mode: {sub}")),
        }
        self.load_current_children();
        Ok(format!("Sort: {}", self.sort_mode.label()))
    }

    pub(crate) fn cmd_sort_quick(&mut self, mode: SortMode, label: &str) -> Result<String, String> {
        self.sort_mode = mode;
        self.load_current_children();
        Ok(format!("Sort: {label}"))
    }

    pub fn cmd_scan(&mut self) -> Result<String, String> {
        if self.scanning {
            return Err("already scanning".into());
        }
        Ok("scan started".into())
    }

    pub(crate) fn cmd_finder(&mut self) -> Result<String, String> {
        self.finder_state = Some(FinderState::new(FinderConfig {
            mode: FinderMode::Dir,
            initial_path: self.view_root_path.to_string_lossy().to_string(),
            title: " Go to Path ".to_string(),
            border_type: BorderType::Plain,
            colors: FinderColors {
                border_fg: self.theme.popup_border_normal,
                border_bg: self.theme.bg,
                input_fg: self.theme.text,
                input_bg: self.theme.bg,
                hint_fg: self.theme.text_tertiary,
                hint_bg: self.theme.bg,
                selected_bg: self.theme.success,
                selected_fg: self.theme.focus_fg,
                normal_bg: self.theme.popup_bg,
                normal_fg: self.theme.text,
                match_fg: self.theme.success,
                path_fg: self.theme.text_tertiary,
                separator_fg: self.theme.text_tertiary,
                title_fg: self.theme.success,
            },
            ..Default::default()
        }));
        self.mode = AppMode::Finder;
        Ok("finder opened".into())
    }

    pub fn cmd_consolidate(&mut self) -> Result<String, String> {
        if !self.server_mode {
            return Err("not in server mode".into());
        }
        Ok("consolidation requested".into())
    }
}
