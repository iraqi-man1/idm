//! Types used only between the desktop UI and its Rust shell.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::download::DownloadInfo;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub data_dir: String,
    pub log_dir: String,
    /// Secrets are persisted (OS credential store available).
    pub secure_storage: bool,
    /// The updater has a signing key configured.
    pub updater_configured: bool,
    pub protocol_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BatchAddResult {
    pub added: Vec<DownloadInfo>,
    pub failed: Vec<BatchFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BatchFailure {
    pub url: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LogLine {
    #[ts(type = "number")]
    pub ts: i64,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChecksumResult {
    pub ok: bool,
    pub actual: String,
}

/// A URL seen on the clipboard (only when the user enabled monitoring).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClipboardUrl {
    pub url: String,
}
