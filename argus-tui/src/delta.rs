use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::{App, AppMessage};
use crate::ipc_client::IpcClient;
use crate::util::log_msg;

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
            match Self::fetch_deltas(&uds_path, &view_root, from, to, &log_path).await {
                Some(deltas) => {
                    log_msg(
                        &log_path,
                        &format!(
                            "fetch_deltas done: {} paths in {:?}",
                            deltas.len(),
                            t0.elapsed()
                        ),
                    );
                    let _ = tx.send(AppMessage::DeltaData(deltas)).await;
                }
                None => {
                    let _ = tx.send(AppMessage::DaemonDisconnected).await;
                }
            }
        });
    }

    async fn fetch_deltas(
        uds: &str,
        view_root: &Path,
        from: u64,
        to: u64,
        log_path: &Path,
    ) -> Option<HashMap<Vec<String>, i64>> {
        let t0 = Instant::now();
        let mut client = match IpcClient::connect(uds).await {
            Ok(c) => c,
            Err(e) => {
                log_msg(log_path, &format!("fetch_deltas: connect failed: {e}"));
                return None;
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
        let mut file_deltas: HashMap<PathBuf, i64> = HashMap::new();
        for entry in &entries {
            *file_deltas.entry(entry.path.clone()).or_insert(0) += entry.delta_size;
        }
        let mut deltas: HashMap<Vec<String>, i64> = HashMap::new();
        for (abs_path, delta) in &file_deltas {
            let relative = abs_path.strip_prefix(view_root).ok();
            let Some(relative) = relative else { continue };
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
            for i in 1..=components.len() {
                let ancestor = components[..i].to_vec();
                *deltas.entry(ancestor).or_insert(0) += delta;
            }
        }
        log_msg(
            log_path,
            &format!(
                "fetch_deltas: map build done, {} paths in {:?}",
                deltas.len(),
                t2.elapsed()
            ),
        );
        Some(deltas)
    }
}
