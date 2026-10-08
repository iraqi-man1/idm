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

/// One browser's integration state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BrowserInfo {
    pub id: String,
    pub name: String,
    /// "chromium" or "firefox".
    pub family: String,
    pub installed: bool,
    /// The native messaging host is registered for this browser.
    pub registered: bool,
    pub manifest_path: Option<String>,
    pub error: Option<String>,
    /// Extension store page, once the extension is published there.
    pub store_url: Option<String>,
}

/// The most recent extension that connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExtensionSeen {
    pub browser: String,
    pub version: String,
    pub origin: String,
    #[ts(type = "number")]
    pub last_seen: i64,
    pub compatible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BrowserIntegrationStatus {
    pub browsers: Vec<BrowserInfo>,
    /// Path of the native messaging host binary, if found.
    pub host_path: Option<String>,
    /// Extensions currently connected.
    pub connected: u32,
    pub last_extension: Option<ExtensionSeen>,
    /// Folders with the unpacked extensions shipped with the app.
    pub chromium_extension_dir: Option<String>,
    pub firefox_extension_dir: Option<String>,
    pub chromium_extension_id: String,
    pub firefox_extension_id: String,
}

/// A download handed over by the browser, waiting for the user's decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingCapture {
    pub id: String,
    pub request: crate::download::AddDownloadRequest,
    pub duplicate_of: Option<crate::download::DownloadId>,
}
