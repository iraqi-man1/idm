use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::download::{DownloadId, DownloadInfo, DownloadStatus};

/// State of one byte-range segment as reported by the engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SegmentProgress {
    pub index: u32,
    /// First byte of the segment.
    #[ts(type = "number")]
    pub start: u64,
    /// One past the last byte (`null` while the size is unknown).
    #[ts(type = "number | null")]
    pub end: Option<u64>,
    /// Bytes of this segment written to disk.
    #[ts(type = "number")]
    pub written: u64,
    /// A connection is currently assigned to the segment.
    pub active: bool,
    /// Current speed of the connection serving this segment, bytes/second.
    #[ts(type = "number")]
    pub speed: u64,
    pub done: bool,
}

/// Periodic progress report for a running download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProgressSnapshot {
    pub id: DownloadId,
    pub status: DownloadStatus,
    #[ts(type = "number")]
    pub downloaded: u64,
    #[ts(type = "number | null")]
    pub total_size: Option<u64>,
    #[ts(type = "number")]
    pub speed: u64,
    #[ts(type = "number")]
    pub avg_speed: u64,
    #[ts(type = "number | null")]
    pub eta_secs: Option<u64>,
    pub active_connections: u8,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
    pub segments: Vec<SegmentProgress>,
    /// Free-form stage description for media jobs ("Downloading video 3/120").
    pub stage: Option<String>,
}

/// Events published by the engine and forwarded to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum EngineEvent {
    /// A download was created.
    Added { download: DownloadInfo },
    /// Persistent fields or status of a download changed.
    Updated { download: DownloadInfo },
    /// A download was removed from the list.
    Removed { id: DownloadId },
    /// Batched progress of all running downloads.
    Progress { items: Vec<ProgressSnapshot> },
    /// A download finished successfully.
    Completed { download: DownloadInfo },
    /// A download stopped with an error.
    Failed { download: DownloadInfo },
}
