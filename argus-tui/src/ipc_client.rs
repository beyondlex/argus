use std::fmt;
use std::path::Path;

use argus_core::{DaemonRequest, DaemonResponse, DeltaEntry};

pub struct IpcClient {
    stream: tokio::net::UnixStream,
}

impl fmt::Debug for IpcClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IpcClient").finish_non_exhaustive()
    }
}

impl IpcClient {
    pub async fn connect(uds_path: &str) -> Result<Self, String> {
        let stream = tokio::net::UnixStream::connect(uds_path)
            .await
            .map_err(|e| format!("failed to connect to daemon: {e}"))?;
        Ok(Self { stream })
    }

    pub async fn ping(&mut self) -> Result<(), String> {
        let resp = self.send_request(&DaemonRequest::Ping).await?;
        match resp {
            DaemonResponse::Pong => Ok(()),
            _ => Err("unexpected response".into()),
        }
    }

    pub async fn get_delta(
        &mut self,
        path: &Path,
        from_ms: u64,
        to_ms: u64,
    ) -> Result<(i64, Vec<DeltaEntry>), String> {
        let resp = self
            .send_request(&DaemonRequest::GetDelta {
                path: path.to_path_buf(),
                from_ms,
                to_ms,
            })
            .await?;
        match resp {
            DaemonResponse::Delta {
                total_delta,
                entries,
            } => Ok((total_delta, entries)),
            DaemonResponse::Error { message } => Err(message),
            _ => Err("unexpected response".into()),
        }
    }

    pub async fn request_consolidation(&mut self) -> Result<u64, String> {
        let resp = self
            .send_request(&DaemonRequest::RequestConsolidation)
            .await?;
        match resp {
            DaemonResponse::ConsolidationDone { consolidated_count } => Ok(consolidated_count),
            DaemonResponse::Error { message } => Err(message),
            _ => Err("unexpected response".into()),
        }
    }

    async fn send_request(&mut self, req: &DaemonRequest) -> Result<DaemonResponse, String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let payload = bincode::serialize(req).map_err(|e| format!("serialize: {e}"))?;
        let len = (payload.len() as u32).to_be_bytes();
        self.stream
            .write_all(&len)
            .await
            .map_err(|e| format!("write len: {e}"))?;
        self.stream
            .write_all(&payload)
            .await
            .map_err(|e| format!("write payload: {e}"))?;

        let mut len_buf = [0u8; 4];
        self.stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| format!("read len: {e}"))?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        self.stream
            .read_exact(&mut resp_buf)
            .await
            .map_err(|e| format!("read payload: {e}"))?;

        bincode::deserialize(&resp_buf).map_err(|e| format!("deserialize: {e}"))
    }
}
