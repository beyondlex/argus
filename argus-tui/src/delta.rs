use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use argus_core::DeltaEntry;

use crate::app::{App, AppMessage};
use crate::ipc_client::IpcClient;
use crate::util::log_msg;

fn build_delta_cache(view_root: &Path, entries: &[DeltaEntry]) -> HashMap<Vec<String>, i64> {
    let mut file_deltas: HashMap<PathBuf, i64> = HashMap::new();
    for entry in entries {
        *file_deltas.entry(entry.path.clone()).or_insert(0) += entry.delta_size;
    }

    let mut deltas: HashMap<Vec<String>, i64> = HashMap::new();
    for (abs_path, delta) in &file_deltas {
        let Some(relative) = abs_path.strip_prefix(view_root).ok() else {
            continue;
        };

        let mut components: Vec<String> = Vec::new();
        components.push(
            view_root
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        for comp in relative.components() {
            components.push(comp.as_os_str().to_string_lossy().to_string());
        }

        // This is a subtree accumulator, not a "visible rows" sum.
        // If a directory is covered by an aggregate DB row, that row still
        // contributes to every ancestor here, but it must not be combined with
        // a second, independent aggregation of the same descendant paths.
        for i in 1..=components.len() {
            let ancestor = components[..i].to_vec();
            *deltas.entry(ancestor).or_insert(0) += delta;
        }
    }

    deltas
}

impl App {
    /// Request delta data from daemon — single query to root path, build full map locally
    pub fn request_delta_refresh(&mut self) {
        if self.delta_pending {
            return;
        }
        let view_root = self.view_root_path.clone();
        if !view_root.exists() {
            return;
        }
        let from = self.time_from;
        let to = self.time_to;
        let tx = self.tx.clone();
        let uds_path = crate::config::TuiConfig::default().daemon.uds_path;
        let log_path = self.log_path.clone();

        // Take existing daemon client to avoid a new UDS connection
        let client = self.daemon_client.take();

        self.delta_pending = true;
        log_msg(
            &log_path,
            &format!(
                "request_delta_refresh: from={from} to={to} root={}",
                view_root.display()
            ),
        );
        tokio::spawn(async move {
            let t0 = Instant::now();
            match Self::fetch_deltas(&uds_path, client, &view_root, from, to, &log_path).await {
                Some((deltas, returned_client)) => {
                    log_msg(
                        &log_path,
                        &format!(
                            "fetch_deltas done: {} paths in {:?}",
                            deltas.len(),
                            t0.elapsed()
                        ),
                    );
                    let _ = tx
                        .send(AppMessage::DeltaData(deltas, Some(returned_client)))
                        .await;
                }
                None => {
                    let _ = tx.send(AppMessage::DaemonDisconnected).await;
                }
            }
        });
    }

    async fn fetch_deltas(
        uds: &str,
        client: Option<IpcClient>,
        view_root: &Path,
        from: u64,
        to: u64,
        log_path: &Path,
    ) -> Option<(HashMap<Vec<String>, i64>, IpcClient)> {
        let t0 = Instant::now();
        let mut client = match client {
            Some(c) => {
                log_msg(log_path, "fetch_deltas: reusing existing connection");
                c
            }
            None => {
                log_msg(log_path, "fetch_deltas: no existing client, connecting...");
                match IpcClient::connect(uds).await {
                    Ok(c) => c,
                    Err(e) => {
                        log_msg(log_path, &format!("fetch_deltas: connect failed: {e}"));
                        return None;
                    }
                }
            }
        };
        let t1 = Instant::now();
        log_msg(
            log_path,
            &format!("fetch_deltas: connected in {:?}", t1 - t0),
        );
        let (_total, entries) = match client.get_delta(view_root, from, to).await {
            Ok(r) => r,
            Err(e) => {
                log_msg(log_path, &format!("fetch_deltas: query failed: {e}"));
                return None;
            }
        };
        let t2 = Instant::now();
        log_msg(
            log_path,
            &format!(
                "fetch_deltas: query returned {} entries in {:?}",
                entries.len(),
                t2 - t1
            ),
        );
        let deltas = build_delta_cache(view_root, &entries);
        log_msg(
            log_path,
            &format!(
                "fetch_deltas: map build done, {} paths in {:?}",
                deltas.len(),
                t2.elapsed()
            ),
        );
        Some((deltas, client))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::{self, File};
    use std::io::Write;
    use std::sync::atomic::AtomicBool;

    use argus_core::{insert_events, open_db, query_delta_detail, scan_path, ROOT_NODE};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    fn touch_with_size(path: &Path, size: u64) {
        let mut file = File::create(path).expect("create file");
        file.write_all(&[0u8]).expect("write seed byte");
        file.set_len(size).expect("set file length");
    }

    fn add_delta_events(root: &Path) -> Vec<DeltaEntry> {
        vec![
            DeltaEntry {
                path: root.join("target/debug/deps/liba.rlib"),
                delta_size: 10 * 1024 * 1024,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/debug/deps/libb.rlib"),
                delta_size: 9 * 1024 * 1024,
                event_type: "create".into(),
                timestamp: 1100,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/debug/incremental/cache.bin"),
                delta_size: 800 * 1024,
                event_type: "modify".into(),
                timestamp: 1200,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/debug/build/argus-tui"),
                delta_size: 15 * 1024 * 1024,
                event_type: "create".into(),
                timestamp: 1300,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/debug/build/libargus_tui.rlib"),
                delta_size: 17 * 1024 * 1024,
                event_type: "create".into(),
                timestamp: 1400,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/debug/build/argusd"),
                delta_size: 15 * 1024 * 1024,
                event_type: "create".into(),
                timestamp: 1500,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/debug/build/argus-cli"),
                delta_size: 12 * 1024 * 1024,
                event_type: "create".into(),
                timestamp: 1600,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/debug/build/libargus_core.rlib"),
                delta_size: 6 * 1024 * 1024,
                event_type: "create".into(),
                timestamp: 1700,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/debug/build/cache/hidden.dat"),
                delta_size: 8 * 1024 * 1024,
                event_type: "modify".into(),
                timestamp: 1750,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/release/ignored.bin"),
                delta_size: 99 * 1024 * 1024,
                event_type: "create".into(),
                timestamp: 1800,
                is_agg: false,
            },
        ]
    }

    fn preaggregated_delta_events(root: &Path) -> Vec<DeltaEntry> {
        vec![
            DeltaEntry {
                path: root.join("target/debug/build/leaf-a.bin"),
                delta_size: 100,
                event_type: "create".into(),
                timestamp: 1000,
                is_agg: false,
            },
            DeltaEntry {
                path: root.join("target/debug/build/nested/leaf-b.bin"),
                delta_size: 200,
                event_type: "create".into(),
                timestamp: 1100,
                is_agg: false,
            },
        ]
    }

    #[test]
    fn test_delta_tree_simulation_report() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("argus");

        fs::create_dir_all(root.join("target/debug/deps")).expect("deps dir");
        fs::create_dir_all(root.join("target/debug/incremental")).expect("incremental dir");
        fs::create_dir_all(root.join("target/debug/build/cache")).expect("build cache dir");
        fs::create_dir_all(root.join("target/release")).expect("release dir");

        touch_with_size(&root.join("target/debug/deps/liba.rlib"), 111);
        touch_with_size(&root.join("target/debug/deps/libb.rlib"), 222);
        touch_with_size(&root.join("target/debug/incremental/cache.bin"), 333);
        touch_with_size(&root.join("target/debug/build/argus-tui"), 444);
        touch_with_size(&root.join("target/debug/build/libargus_tui.rlib"), 555);
        touch_with_size(&root.join("target/debug/build/argusd"), 666);
        touch_with_size(&root.join("target/debug/build/argus-cli"), 777);
        touch_with_size(&root.join("target/debug/build/libargus_core.rlib"), 888);
        touch_with_size(&root.join("target/debug/build/cache/hidden.dat"), 999);
        touch_with_size(&root.join("target/release/ignored.bin"), 1);

        let cancel = AtomicBool::new(false);
        let snapshot = scan_path(&root, &cancel, None, &[]).expect("scan snapshot");
        assert_eq!(snapshot.root_path, root);

        let db = temp.path().join("delta.db");
        let conn = open_db(&db).expect("open db");
        let events = add_delta_events(&root);
        insert_events(&conn, &events).expect("insert events");

        let entries = query_delta_detail(&conn, &root, 0, i64::MAX as u64).expect("query deltas");
        assert_eq!(entries.len(), events.len());

        let delta_cache = build_delta_cache(&root, &entries);
        let root_name = root
            .file_name()
            .expect("root name")
            .to_string_lossy()
            .to_string();
        let debug_path = vec![root_name.clone(), "target".into(), "debug".into()];
        let build_path = vec![
            root_name.clone(),
            "target".into(),
            "debug".into(),
            "build".into(),
        ];
        let deps_path = vec![
            root_name.clone(),
            "target".into(),
            "debug".into(),
            "deps".into(),
        ];

        let debug_total = delta_cache.get(&debug_path).copied().unwrap_or_default();
        let build_total = delta_cache.get(&build_path).copied().unwrap_or_default();
        let deps_total = delta_cache.get(&deps_path).copied().unwrap_or_default();

        let visible_leaf_sum: i64 = entries
            .iter()
            .filter(|entry| {
                entry
                    .path
                    .strip_prefix(root.join("target/debug/build"))
                    .is_ok()
                    && entry.path.components().count()
                        == root.join("target/debug/build").components().count() + 1
            })
            .map(|entry| entry.delta_size)
            .sum();

        eprintln!("delta simulation:");
        eprintln!(
            "  root  = {:?}",
            delta_cache.get(vec![root_name.clone()].as_slice()).copied()
        );
        eprintln!("  debug = {debug_total}");
        eprintln!("  deps  = {deps_total}");
        eprintln!("  build = {build_total}");
        eprintln!("  visible build files sum = {visible_leaf_sum}");

        assert!(debug_total > build_total);
        assert!(build_total > visible_leaf_sum);
        assert_eq!(
            debug_total,
            deps_total
                + delta_cache
                    .get(
                        vec![
                            root_name.clone(),
                            "target".into(),
                            "debug".into(),
                            "incremental".into(),
                        ]
                        .as_slice(),
                    )
                    .copied()
                    .unwrap_or_default()
                + build_total
        );

        let (tx, rx) = mpsc::channel(1);
        let mut app = App::new(crate::config::TuiConfig::default(), tx, rx);
        app.view_root_path = root.clone();
        app.tree_root = Some(crate::app::TreeNode::Snapshot(
            std::sync::Arc::new(snapshot),
            ROOT_NODE,
        ));
        app.expanded
            .insert(vec![root_name.clone(), "target".into()]);
        app.expanded
            .insert(vec![root_name.clone(), "target".into(), "debug".into()]);
        app.expanded.insert(build_path.clone());
        app.delta_cache = delta_cache;
        app.update_tree_lines();

        let debug_line = app
            .tree_lines
            .iter()
            .find(|line| line.path == debug_path)
            .expect("debug line");
        let build_line = app
            .tree_lines
            .iter()
            .find(|line| line.path == build_path)
            .expect("build line");

        assert!(debug_line.node.is_dir());
        assert!(build_line.node.is_dir());
        assert_eq!(debug_line.path, debug_path);
        assert_eq!(build_line.path, build_path);
    }

    #[test]
    fn test_preaggregated_rows_double_count_subtree() {
        // Regression guard:
        // An aggregate row at the parent path must cover the subtree by itself.
        // If future changes start summing the parent row together with the child
        // rows again, this test will fail and catch the double-count regression.
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("argus");

        fs::create_dir_all(root.join("target/debug/build/nested")).expect("build dir");
        touch_with_size(&root.join("target/debug/build/leaf-a.bin"), 100);
        touch_with_size(&root.join("target/debug/build/nested/leaf-b.bin"), 200);

        let db = temp.path().join("preagg.db");
        let conn = open_db(&db).expect("open db");
        let events = preaggregated_delta_events(&root);
        insert_events(&conn, &events).expect("insert events");
        let agg_path = root.join("target/debug/build");
        let agg_path_str = agg_path.to_string_lossy().to_string();
        let agg_path_ref = agg_path_str.as_str();
        conn.execute(
            "INSERT INTO delta_events (path, delta_size, event_type, timestamp, is_agg) VALUES (?1, ?2, ?3, ?4, 1)",
            &[agg_path_ref, "300", "agg", "1200"],
        )
        .expect("insert agg row");

        let query_total = argus_core::query_delta_total(
            &conn,
            &root.join("target/debug/build"),
            0,
            i64::MAX as u64,
        )
        .expect("query total");
        let entries = query_delta_detail(&conn, &root, 0, i64::MAX as u64).expect("query detail");
        let delta_cache = build_delta_cache(&root, &entries);

        let root_name = root
            .file_name()
            .expect("root name")
            .to_string_lossy()
            .to_string();
        let build_path = vec![
            root_name.clone(),
            "target".into(),
            "debug".into(),
            "build".into(),
        ];
        let nested_path = vec![
            root_name.clone(),
            "target".into(),
            "debug".into(),
            "build".into(),
            "nested".into(),
        ];

        let leaf_sum: i64 = events
            .iter()
            .filter(|event| !event.is_agg)
            .map(|event| event.delta_size)
            .sum();
        let build_total = delta_cache.get(&build_path).copied().unwrap_or_default();
        let nested_total = delta_cache.get(&nested_path).copied().unwrap_or_default();

        eprintln!("pre-aggregated simulation:");
        eprintln!("  leaf sum    = {leaf_sum}");
        eprintln!("  query total  = {query_total}");
        eprintln!("  build delta  = {build_total}");
        eprintln!("  nested delta = {nested_total}");

        assert_eq!(leaf_sum, 300);
        assert_eq!(query_total, 300);
        assert_eq!(build_total, 300);
        assert_eq!(nested_total, 0);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_agg);
    }
}
