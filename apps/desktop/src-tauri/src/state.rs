use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use velox_core::DownloadManager;
use velox_persistence::SecretBox;

/// Application-wide state managed by Tauri.
pub struct AppState {
    pub manager: DownloadManager,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub secret_box: Option<SecretBox>,
    pub updater_configured: bool,
    /// Set once the user chose "Quit" so window/exit handlers let go.
    pub quitting: AtomicBool,
    _log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

impl AppState {
    pub fn new(
        manager: DownloadManager,
        data_dir: PathBuf,
        log_dir: PathBuf,
        secret_box: Option<SecretBox>,
        updater_configured: bool,
        log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
    ) -> Self {
        Self {
            manager,
            data_dir,
            log_dir,
            secret_box,
            updater_configured,
            quitting: AtomicBool::new(false),
            _log_guard: log_guard,
        }
    }
}
