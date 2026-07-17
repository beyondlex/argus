use super::*;
use crate::config::TuiConfig;
use argus_core::{FileType, Snapshot, SnapshotBuilder, ROOT_NODE};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

#[test]
fn test_unlisted_child_dir_keeps_dash() {
    let root_path = PathBuf::from("/tmp/test");

    let mut live = SnapshotBuilder::new("test");
    let t = live.push_dir(ROOT_NODE, "target");
    live.push_dir(t, "debug");
    live.push_dir(t, "build");
    let live_snap = live.finish(root_path.clone(), 0, 0);

    let mut scan = SnapshotBuilder::new("test");
    let t = scan.push_dir(ROOT_NODE, "target");
    scan.push_dir(t, "debug");
    let scan_snap = scan.finish(root_path.clone(), 0, 0);

    let (tx, rx) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, rx);
    app.view_root_path = root_path.clone();
    app.tree_root = Some(TreeNode::Snapshot(Arc::new(live_snap), ROOT_NODE));
    app.scan_cache
        .insert(root_path.clone(), Arc::new(scan_snap));

    app.current_dir_path = vec!["test".to_string(), "target".to_string()];
    app.load_current_children();

    let build_entry = app
        .current_children
        .iter()
        .find(|entry| entry.node.name() == "build")
        .expect("build entry should exist");
    assert!(build_entry.is_dir);
    assert!(!build_entry.has_scan_data);
    assert_eq!(build_entry.size, 0);
}

#[test]
fn test_execute_empty_command() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert_eq!(app.execute_command(""), Err("empty command".into()));
}

#[test]
fn test_execute_unknown_command() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    let result = app.execute_command("foobar");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unknown command"));
}

#[test]
fn test_execute_help() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert!(app.execute_command("help").is_ok());
    assert_eq!(app.mode, AppMode::Help);
}

#[test]
fn test_execute_time_not_in_server_mode() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert_eq!(
        app.execute_command("time 2h"),
        Err("not in server mode".into())
    );
}

#[test]
fn test_execute_time_no_arg_opens_help() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    assert!(app.execute_command("time").is_ok());
    assert_eq!(app.mode, AppMode::TimeHelp);
}

#[test]
fn test_execute_time_duration() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    app.delta_pending = true;
    let result = app.execute_command("time 2h");
    assert!(result.is_ok());
    assert!(result.unwrap().contains("2h"));
    assert!(app.time_custom);
}

#[test]
fn test_execute_time_time_only() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    app.delta_pending = true;
    let result = app.execute_command("time 14:30");
    assert!(result.is_ok());
    assert!(app.time_custom);
    assert!(app.time_custom_label.contains("14:30"));
}

#[test]
fn test_execute_time_absolute() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    app.delta_pending = true;
    let result = app.execute_command("time 07-04");
    assert!(result.is_ok());
    assert!(app.time_custom);
}

#[test]
fn test_execute_time_range_time_to_time() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    app.delta_pending = true;
    let result = app.execute_command("time 09:00 to 17:00");
    assert!(result.is_ok());
    assert!(app.time_custom);
    assert!(app.time_custom_label.contains("09:00"));
    assert!(app.time_custom_label.contains("17:00"));
}

#[test]
fn test_execute_time_range_absolute_to_absolute() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    app.delta_pending = true;
    let result = app.execute_command("time 07-04 to 07-05");
    assert!(result.is_ok());
    assert!(app.time_custom);
    assert!(app.time_custom_label.contains("07-04"));
    assert!(app.time_custom_label.contains("07-05"));
}

#[test]
fn test_execute_time_range_duration_to_date_errors() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    let result = app.execute_command("time 2h to 09:00");
    assert!(result.is_err());
}

#[test]
fn test_execute_time_invalid_duration() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    let result = app.execute_command("time 2x");
    assert!(result.is_err());
}

#[test]
fn test_execute_sort_name() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert!(app.execute_command("sort n").is_ok());
    assert_eq!(app.sort_mode, SortMode::Name);
}

#[test]
fn test_execute_sort_toggle() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.sort_mode = SortMode::Name;
    assert!(app.execute_command("sort").is_ok());
    assert_eq!(app.sort_mode, SortMode::Size);
}

#[test]
fn test_execute_sort_unknown() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    let result = app.execute_command("sort x");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unknown sort mode"));
}

#[test]
fn test_execute_delta_with_unit() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    assert!(app.execute_command("delta 500k").is_ok());
    assert_eq!(app.delta_filter_value, 500);
    assert_eq!(app.delta_filter_unit, 0);
    assert!(app.delta_filter_active);
}

#[test]
fn test_execute_delta_not_in_server_mode() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert_eq!(
        app.execute_command("delta 100m"),
        Err("not in server mode".into())
    );
}

#[test]
fn test_delta_unit_multiplier_values() {
    assert_eq!(delta_unit_multiplier(0), 1024);
    assert_eq!(delta_unit_multiplier(1), 1024 * 1024);
    assert_eq!(delta_unit_multiplier(2), 1024 * 1024 * 1024);
    assert_eq!(delta_unit_multiplier(3), 1);
}

#[test]
fn test_delta_filter_inc_basic() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.delta_filter_value = 5;
    app.delta_filter_inc();
    assert_eq!(app.delta_filter_value, 6);
}

#[test]
fn test_delta_filter_inc_unit_level_up() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.delta_filter_value = 1024;
    app.delta_filter_unit = 0;
    app.delta_filter_inc();
    assert_eq!(app.delta_filter_value, 1);
    assert_eq!(app.delta_filter_unit, 1);
}

#[test]
fn test_delta_filter_inc_unit_level_up_max_unit() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.delta_filter_value = 1024;
    app.delta_filter_unit = 2;
    app.delta_filter_inc();
    assert_eq!(app.delta_filter_value, 1025);
    assert_eq!(app.delta_filter_unit, 2);
}

#[test]
fn test_delta_filter_dec_basic() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.delta_filter_value = 5;
    app.delta_filter_dec();
    assert_eq!(app.delta_filter_value, 4);
}

#[test]
fn test_delta_filter_dec_min_zero() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.delta_filter_value = 0;
    app.delta_filter_dec();
    assert_eq!(app.delta_filter_value, 0);
}

#[test]
fn test_delta_filter_cycle_unit() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert_eq!(app.delta_filter_unit, 1);
    app.delta_filter_cycle_unit();
    assert_eq!(app.delta_filter_unit, 2);
    app.delta_filter_cycle_unit();
    assert_eq!(app.delta_filter_unit, 0);
    app.delta_filter_cycle_unit();
    assert_eq!(app.delta_filter_unit, 1);
}

#[test]
fn test_delta_filter_inc_unit_level_up_stays_at_gb() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.delta_filter_value = 1024;
    app.delta_filter_unit = 2;
    app.delta_filter_inc();
    assert_eq!(app.delta_filter_value, 1025);
    assert_eq!(app.delta_filter_unit, 2);
}

#[test]
fn test_push_command_history_empty() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.push_command_history("");
    assert!(app.command_history.is_empty());
}

#[test]
fn test_push_command_history_trimmed() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.push_command_history("  scan  ");
    assert_eq!(app.command_history, vec!["scan"]);
}

#[test]
fn test_push_command_history_dedup() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.push_command_history("scan");
    app.push_command_history("scan");
    assert_eq!(app.command_history.len(), 1);
}

#[test]
fn test_push_command_history_cap() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    for i in 0..60 {
        app.push_command_history(&format!("cmd{i}"));
    }
    assert_eq!(app.command_history.len(), 50);
    assert_eq!(app.command_history[0], "cmd10");
    assert_eq!(app.command_history[49], "cmd59");
}

#[test]
fn test_push_command_history_resets_idx() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.command_history_idx = Some(3);
    app.push_command_history("scan");
    assert_eq!(app.command_history_idx, None);
}

#[test]
fn test_clear_command_state() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.command_input = "scan".into();
    app.command_matches = vec!["Scan"];
    app.command_selected = 1;
    app.clear_command_state();
    assert!(app.command_input.is_empty());
    assert!(app.command_matches.is_empty());
    assert_eq!(app.command_selected, 0);
}

#[test]
fn test_update_command_matches_empty() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.update_command_matches();
    assert_eq!(app.command_matches.len(), App::COMMANDS.len());
}

#[test]
fn test_update_command_matches_fuzzy() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.command_input = "sc".into();
    app.update_command_matches();
    assert!(app.command_matches.contains(&"Scan"));
}

#[test]
fn test_cmd_scan_not_scanning() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert!(app.cmd_scan().is_ok());
}

#[test]
fn test_cmd_scan_already_scanning() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.scanning = true;
    assert_eq!(app.cmd_scan(), Err("already scanning".into()));
}

#[test]
fn test_cmd_consolidate_not_server_mode() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert_eq!(app.cmd_consolidate(), Err("not in server mode".into()));
}

#[test]
fn test_cmd_sort_quick_delta() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.sort_mode = SortMode::Name;
    let result = app.cmd_sort_quick(SortMode::Delta, "Delta");
    assert!(result.is_ok());
    assert_eq!(app.sort_mode, SortMode::Delta);
}

#[test]
fn test_cmd_sort_quick_size() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.sort_mode = SortMode::Name;
    let result = app.cmd_sort_quick(SortMode::Size, "Size");
    assert!(result.is_ok());
    assert_eq!(app.sort_mode, SortMode::Size);
}

#[test]
fn test_default_log_path_returns_non_empty() {
    let path = default_log_path();
    assert!(path.ends_with("argus.log"));
}

#[test]
fn test_set_time_preset_0() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.set_time_preset(0);
    assert!(!app.time_custom);
    assert!(app.time_from < app.time_to);
    assert!(app.time_to - app.time_from <= 3_600_000);
}

#[test]
fn test_set_time_preset_7d() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.set_time_preset(5);
    assert_eq!(app.time_preset, 5);
    let diff = app.time_to - app.time_from;
    assert!(diff >= 604_800_000 - 1000);
    assert!(diff <= 604_800_000 + 1000);
}

#[test]
fn test_time_preset_label_all() {
    assert_eq!(App::time_preset_label(0), "1h");
    assert_eq!(App::time_preset_label(1), "6h");
    assert_eq!(App::time_preset_label(2), "12h");
    assert_eq!(App::time_preset_label(3), "1d");
    assert_eq!(App::time_preset_label(4), "3d");
    assert_eq!(App::time_preset_label(5), "7d");
    assert_eq!(App::time_preset_label(99), "1h");
}

#[test]
fn test_default_time_range() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.default_time_range();
    assert_eq!(app.time_preset, 0);
    assert!(!app.time_custom);
}

#[test]
fn test_execute_delta_no_unit_uses_mb() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    assert!(app.execute_command("delta 200").is_ok());
    assert_eq!(app.delta_filter_value, 200);
    assert_eq!(app.delta_filter_unit, 1);
}

#[test]
fn test_execute_delta_m_unit() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.server_mode = true;
    assert!(app.execute_command("delta 50m").is_ok());
    assert_eq!(app.delta_filter_value, 50);
    assert_eq!(app.delta_filter_unit, 1);
}

#[test]
fn test_execute_sort_shortcuts() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert!(app.execute_command("sd").is_ok());
    assert_eq!(app.sort_mode, SortMode::Delta);
    assert!(app.execute_command("ss").is_ok());
    assert_eq!(app.sort_mode, SortMode::Size);
    assert!(app.execute_command("sn").is_ok());
    assert_eq!(app.sort_mode, SortMode::Name);
}

#[test]
fn test_execute_scan() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert!(app.execute_command("scan").is_ok());
}

#[test]
fn test_sort_mode_toggle() {
    assert_eq!(SortMode::Name.toggle(), SortMode::Size);
    assert_eq!(SortMode::Size.toggle(), SortMode::Delta);
    assert_eq!(SortMode::Delta.toggle(), SortMode::Name);
}

#[test]
fn test_sort_mode_label() {
    assert_eq!(SortMode::Name.label(), "Name");
    assert_eq!(SortMode::Size.label(), "Size");
    assert_eq!(SortMode::Delta.label(), "Delta");
}

#[test]
fn test_tree_node_snapshot_basics() {
    let mut b = SnapshotBuilder::new("root");
    b.push_file(ROOT_NODE, "file.txt", FileType::File, 1024, 0);
    let snap = Arc::new(b.finish(PathBuf::from("/tmp"), 0, 0));
    let root = TreeNode::Snapshot(snap.clone(), ROOT_NODE);
    assert!(root.is_dir());
    assert_eq!(root.name(), "root");
    assert_eq!(root.file_type(), FileType::Directory);
    assert_eq!(root.current_size(), 0);

    let file = TreeNode::Snapshot(snap.clone(), 1);
    assert!(!file.is_dir());
    assert_eq!(file.name(), "file.txt");
    assert_eq!(root.file_type(), FileType::Directory);
    assert_eq!(file.current_size(), 1024);
}

#[test]
fn test_selected_node_full_path_empty_tree() {
    let (tx, _) = mpsc::channel(1);
    let app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    assert!(app.selected_node_full_path().is_none());
}

#[test]
fn test_set_error_sets_last_error() {
    let (tx, _) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, mpsc::channel(1).1);
    app.set_error("test error".into(), 5);
    assert_eq!(app.last_error.as_deref(), Some("test error"));
    assert!(app.error_clear_at.is_some());
}

fn make_flat_app() -> App {
    let mut b = SnapshotBuilder::new("test");
    let src = b.push_dir(ROOT_NODE, "src");
    let docs = b.push_dir(ROOT_NODE, "docs");
    b.push_file(ROOT_NODE, "readme.md", FileType::File, 50, 50);
    b.nodes[src as usize].set_size(100);
    b.nodes[src as usize].set_disk_usage(100);
    b.nodes[docs as usize].set_size(50);
    b.nodes[docs as usize].set_disk_usage(50);
    b.nodes[0].set_size(200);
    b.nodes[0].set_disk_usage(200);
    let snap = b.finish(PathBuf::from("/tmp/test"), 200, 200);
    let (tx, rx) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, rx);
    app.view_root_path = PathBuf::from("/tmp/test");
    app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
    app.current_dir_path = vec![String::from("test")];
    app.load_current_children();
    app
}

#[test]
fn test_load_current_children_basic() {
    let app = make_flat_app();
    assert_eq!(app.current_children.len(), 3);
    assert_eq!(app.current_dir_total, 200);
    assert_eq!(app.current_children[0].node.name(), "src");
    assert_eq!(app.current_children[1].node.name(), "docs");
    assert_eq!(app.current_children[2].node.name(), "readme.md");
}

#[test]
fn test_selected_entry_returns_correct_item() {
    let mut app = make_flat_app();
    app.cursor = 0;
    let entry = app.selected_entry().unwrap();
    assert_eq!(entry.node.name(), "src");
    assert!(entry.is_dir);

    app.cursor = 2;
    let entry = app.selected_entry().unwrap();
    assert_eq!(entry.node.name(), "readme.md");
    assert!(!entry.is_dir);
}

#[test]
fn test_selected_entry_out_of_bounds() {
    let mut app = make_flat_app();
    app.cursor = 100;
    assert!(app.selected_entry().is_none());
}

#[test]
fn test_enter_directory_into_subdir() {
    let mut app = make_flat_app();
    app.cursor = 0;

    app.enter_directory();
    assert_eq!(
        app.current_dir_path,
        vec![String::from("test"), String::from("src")]
    );
    assert_eq!(app.current_children.len(), 0);
    assert_eq!(app.dir_stack.len(), 1);
    assert_eq!(app.cursor, 0);
}

#[test]
fn test_enter_directory_non_dir_does_nothing() {
    let mut app = make_flat_app();
    let readme_idx = app
        .current_children
        .iter()
        .position(|e| e.node.name() == "readme.md")
        .unwrap();
    app.cursor = readme_idx;

    app.enter_directory();
    assert_eq!(app.current_dir_path, vec![String::from("test")]);
    assert!(app.dir_stack.is_empty());
}

#[test]
fn test_go_to_parent_restores_previous() {
    let mut app = make_flat_app();
    app.cursor = 0;
    app.enter_directory();
    assert_eq!(
        app.current_dir_path,
        vec![String::from("test"), String::from("src")]
    );

    app.go_to_parent();
    assert_eq!(app.current_dir_path, vec![String::from("test")]);
    assert!(app.dir_stack.is_empty());
}

#[test]
fn test_go_to_parent_at_root_does_nothing() {
    let mut app = make_flat_app();
    assert_eq!(app.current_dir_path, vec![String::from("test")]);

    app.go_to_parent();
    assert_eq!(app.current_dir_path, vec![String::from("test")]);
}

#[test]
fn test_go_to_root_clears_stack() {
    let mut app = make_flat_app();
    app.cursor = 0;
    app.enter_directory();
    assert_eq!(app.dir_stack.len(), 1);

    app.go_to_root();
    assert_eq!(app.current_dir_path, vec![String::from("test")]);
    assert!(app.dir_stack.is_empty());
}

#[test]
fn test_apply_search_filters_children() {
    let mut app = make_flat_app();
    app.search_word = "src".into();
    app.apply_search();
    assert_eq!(app.current_filtered.len(), 3);
    assert_eq!(app.search_match_indices.len(), 1);
    assert_eq!(app.search_match_indices[0], 0);
    assert_eq!(app.current_children[0].node.name(), "src");
}

#[test]
fn test_apply_search_empty_query_restores_all() {
    let mut app = make_flat_app();
    app.search_word = "nonexistent".into();
    app.apply_search();
    assert_eq!(app.current_filtered.len(), 3);
    assert!(app.search_match_indices.is_empty());

    app.search_word = "".into();
    app.apply_search();
    assert_eq!(app.current_filtered.len(), 3);
    assert!(app.search_match_indices.is_empty());
}

#[test]
fn test_cycle_match_forward() {
    let mut app = make_flat_app();
    app.search_word = "readme".into();
    app.apply_search();
    assert_eq!(app.search_match_indices.len(), 1);

    app.cursor = 0;
    app.cycle_match(true);
    assert_eq!(app.cursor, 2);
}

#[test]
fn test_cycle_match_backward() {
    let mut app = make_flat_app();
    app.search_word = "readme".into();
    app.apply_search();
    assert_eq!(app.search_match_indices.len(), 1);

    app.cursor = 0;
    app.cycle_match(false);
    assert_eq!(app.cursor, 2);
}

#[test]
fn test_sort_by_name() {
    let mut app = make_flat_app();
    app.sort_mode = SortMode::Name;
    sort_children(&mut app.current_children, app.sort_mode, &app.delta_cache);
    app.refresh_current_filtered();
    assert_eq!(app.current_children[0].node.name(), "docs");
    assert_eq!(app.current_children[1].node.name(), "readme.md");
    assert_eq!(app.current_children[2].node.name(), "src");
}

#[test]
fn test_sort_by_size() {
    let mut app = make_flat_app();
    app.sort_mode = SortMode::Size;
    sort_children(&mut app.current_children, app.sort_mode, &app.delta_cache);
    app.refresh_current_filtered();
    assert_eq!(app.current_children[0].node.name(), "src");
    assert_eq!(app.current_children[1].node.name(), "docs");
    assert_eq!(app.current_children[2].node.name(), "readme.md");
}

#[test]
#[allow(unused_mut)]
fn test_hidden_files_toggle_in_load() {
    let mut b = SnapshotBuilder::new("test");
    b.push_file(ROOT_NODE, ".hidden", FileType::File, 50, 0);
    b.push_file(ROOT_NODE, "visible.txt", FileType::File, 50, 0);
    b.nodes[0].set_size(100);
    let snap = b.finish(PathBuf::from("/tmp/test"), 100, 0);
    let (tx, rx) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, rx);
    app.view_root_path = PathBuf::from("/tmp/test");
    app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
    app.current_dir_path = vec![String::from("test")];

    app.show_hidden = false;
    app.load_current_children();
    assert_eq!(app.current_children.len(), 1);
    assert_eq!(app.current_children[0].node.name(), "visible.txt");

    app.show_hidden = true;
    app.load_current_children();
    assert_eq!(app.current_children.len(), 2);
}

#[test]
fn test_dir_stack_depth_multiple_entries() {
    let mut b = SnapshotBuilder::new("root");
    let a = b.push_dir(ROOT_NODE, "a");
    b.push_dir(ROOT_NODE, "b");
    b.push_dir(a, "deep");
    b.nodes[0].set_size(300);
    b.nodes[a as usize].set_size(200);
    let snap = b.finish(PathBuf::from("/tmp/deep"), 300, 0);
    let (tx, rx) = mpsc::channel(1);
    let mut app = App::new(TuiConfig::default(), tx, rx);
    app.view_root_path = PathBuf::from("/tmp/deep");
    app.tree_root = Some(TreeNode::Snapshot(Arc::new(snap), ROOT_NODE));
    app.current_dir_path = vec!["root".into()];
    app.load_current_children();

    app.cursor = 0;
    app.enter_directory();
    assert_eq!(
        app.current_dir_path,
        vec![String::from("root"), String::from("a")]
    );

    app.cursor = 0;
    app.enter_directory();
    assert_eq!(
        app.current_dir_path,
        vec![
            String::from("root"),
            String::from("a"),
            String::from("deep")
        ]
    );
    assert_eq!(app.dir_stack.len(), 2);

    app.go_to_parent();
    assert_eq!(
        app.current_dir_path,
        vec![String::from("root"), String::from("a")]
    );
    assert_eq!(app.dir_stack.len(), 1);

    app.go_to_parent();
    assert_eq!(app.current_dir_path, vec![String::from("root")]);
    assert!(app.dir_stack.is_empty());
}

#[test]
fn test_selected_node_full_path_flat_mode() {
    let mut app = make_flat_app();
    let idx = app
        .current_children
        .iter()
        .position(|e| e.node.name() == "readme.md")
        .unwrap();
    app.cursor = app
        .current_filtered
        .iter()
        .position(|&i| i == idx)
        .unwrap_or(0);
    let path = app.selected_node_full_path();
    assert_eq!(path, Some(PathBuf::from("/tmp/test/readme.md")));
}

#[test]
fn test_refresh_current_filtered_delta_active() {
    let mut app = make_flat_app();
    app.delta_filter_active = true;
    app.delta_filter_value = 80;
    app.delta_filter_unit = 0;
    app.delta_cache = std::collections::HashMap::from([
        (vec![String::from("test"), String::from("src")], 100_000i64),
        (vec![String::from("test"), String::from("docs")], 50_000i64),
        (vec![String::from("test"), String::from("readme.md")], 0i64),
    ]);
    app.refresh_current_filtered();
    assert_eq!(app.current_filtered.len(), 1);
    let idx = app.current_filtered[0];
    assert_eq!(app.current_children[idx].node.name(), "src");
}
