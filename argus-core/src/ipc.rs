use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::DeltaEntry;

#[derive(Serialize, Deserialize, Debug)]
pub enum DaemonRequest {
    GetDelta {
        path: PathBuf,
        from_ms: u64,
        to_ms: u64,
    },
    GetDeltaDetail {
        path: PathBuf,
        from_ms: u64,
        to_ms: u64,
    },
    Ping,
    GetStatus,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DaemonResponse {
    Delta {
        total_delta: i64,
        entries: Vec<DeltaEntry>,
    },
    DeltaDetail {
        entries: Vec<DeltaEntry>,
    },
    Pong,
    Status {
        version: String,
        watch_dirs: Vec<PathBuf>,
        uptime_secs: u64,
    },
    Error {
        message: String,
    },
}

pub const DEFAULT_UDS_PATH: &str = "/tmp/argusd.sock";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ping_roundtrip() {
        let req = DaemonRequest::Ping;
        let encoded = bincode::serialize(&req).unwrap();
        let decoded: DaemonRequest = bincode::deserialize(&encoded).unwrap();
        assert!(matches!(decoded, DaemonRequest::Ping));
    }

    #[test]
    fn test_pong_roundtrip() {
        let resp = DaemonResponse::Pong;
        let encoded = bincode::serialize(&resp).unwrap();
        let decoded: DaemonResponse = bincode::deserialize(&encoded).unwrap();
        assert!(matches!(decoded, DaemonResponse::Pong));
    }

    #[test]
    fn test_get_delta_roundtrip() {
        let req = DaemonRequest::GetDelta {
            path: PathBuf::from("/tmp/test"),
            from_ms: 1000,
            to_ms: 2000,
        };
        let encoded = bincode::serialize(&req).unwrap();
        let decoded: DaemonRequest = bincode::deserialize(&encoded).unwrap();
        match decoded {
            DaemonRequest::GetDelta {
                path,
                from_ms,
                to_ms,
            } => {
                assert_eq!(path, PathBuf::from("/tmp/test"));
                assert_eq!(from_ms, 1000);
                assert_eq!(to_ms, 2000);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_get_status_roundtrip() {
        let resp = DaemonResponse::Status {
            version: "0.1.0".into(),
            watch_dirs: vec![PathBuf::from("/tmp")],
            uptime_secs: 42,
        };
        let encoded = bincode::serialize(&resp).unwrap();
        let decoded: DaemonResponse = bincode::deserialize(&encoded).unwrap();
        match decoded {
            DaemonResponse::Status {
                version,
                watch_dirs,
                uptime_secs,
            } => {
                assert_eq!(version, "0.1.0");
                assert_eq!(watch_dirs, vec![PathBuf::from("/tmp")]);
                assert_eq!(uptime_secs, 42);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_error_roundtrip() {
        let resp = DaemonResponse::Error {
            message: "something went wrong".into(),
        };
        let encoded = bincode::serialize(&resp).unwrap();
        let decoded: DaemonResponse = bincode::deserialize(&encoded).unwrap();
        match decoded {
            DaemonResponse::Error { message } => {
                assert_eq!(message, "something went wrong");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_delta_detail_roundtrip() {
        let req = DaemonRequest::GetDeltaDetail {
            path: PathBuf::from("/tmp/test"),
            from_ms: 1000,
            to_ms: 2000,
        };
        let encoded = bincode::serialize(&req).unwrap();
        let decoded: DaemonRequest = bincode::deserialize(&encoded).unwrap();
        match decoded {
            DaemonRequest::GetDeltaDetail {
                path,
                from_ms,
                to_ms,
            } => {
                assert_eq!(path, PathBuf::from("/tmp/test"));
                assert_eq!(from_ms, 1000);
                assert_eq!(to_ms, 2000);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_delta_response_roundtrip() {
        let resp = DaemonResponse::Delta {
            total_delta: 1024,
            entries: vec![DeltaEntry {
                path: PathBuf::from("/tmp/test.txt"),
                delta_size: 1024,
                event_type: "create".into(),
                timestamp: 1000,
            }],
        };
        let encoded = bincode::serialize(&resp).unwrap();
        let decoded: DaemonResponse = bincode::deserialize(&encoded).unwrap();
        match decoded {
            DaemonResponse::Delta {
                total_delta,
                entries,
            } => {
                assert_eq!(total_delta, 1024);
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].path, PathBuf::from("/tmp/test.txt"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_delta_detail_response_roundtrip() {
        let resp = DaemonResponse::DeltaDetail {
            entries: vec![
                DeltaEntry {
                    path: PathBuf::from("/tmp/a.txt"),
                    delta_size: 100,
                    event_type: "create".into(),
                    timestamp: 1000,
                },
                DeltaEntry {
                    path: PathBuf::from("/tmp/a.txt"),
                    delta_size: 50,
                    event_type: "modify".into(),
                    timestamp: 2000,
                },
            ],
        };
        let encoded = bincode::serialize(&resp).unwrap();
        let decoded: DaemonResponse = bincode::deserialize(&encoded).unwrap();
        match decoded {
            DaemonResponse::DeltaDetail { entries } => {
                assert_eq!(entries.len(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }
}
