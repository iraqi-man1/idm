//! Browser integration on the app side: the IPC endpoint used by the
//! native messaging host, request handling, the "download file info"
//! dialog for captured downloads, and native host registration.

pub mod capture;
pub mod commands;
mod handler;

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};
use velox_nm::ipc::{self, EndpointInfo, ServerHandle};
use velox_nm::manifest;
use velox_types::protocol::{BrowserConfig, ExtReply};
use velox_types::{AppSettings, ExtensionSeen};

use crate::AppState;

/// Runtime state of the browser bridge.
#[derive(Default)]
pub struct Bridge {
    pub server: Mutex<Option<ServerHandle>>,
    pub last_extension: Mutex<Option<ExtensionSeen>>,
}

pub fn browser_config(s: &AppSettings) -> BrowserConfig {
    BrowserConfig {
        capture_downloads: s.browser.capture_downloads,
        capture_extensions: s.browser.capture_extensions.clone(),
        min_capture_size: s.browser.min_capture_size,
        excluded_sites: s.browser.excluded_sites.clone(),
        video_detection: s.browser.video_detection,
        floating_button: s.browser.floating_button,
    }
}

/// Path of the bundled native messaging host.
pub fn host_path() -> Option<PathBuf> {
    manifest::host_binary_next_to_current_exe()
}

fn manifests_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("native-messaging")
}

/// Write the extension allow-list read by the host and (re)register the
/// host for the current user.
pub fn register(state: &AppState) -> Vec<manifest::BrowserStatus> {
    let s = state.manager.settings();
    let extra = &s.browser.extra_allowed_extension_ids;
    if let Ok(json) = serde_json::to_vec(extra) {
        let _ = std::fs::write(state.data_dir.join("allowed-extensions.json"), json);
    }
    match host_path() {
        Some(host) => manifest::register_user(&host, &manifests_dir(state), extra),
        None => {
            tracing::warn!("native messaging host binary not found next to the application");
            Vec::new()
        }
    }
}

/// Start the IPC server and publish its endpoint. Called once at startup.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let data_dir = state.data_dir.clone();
        let endpoint = ipc::new_endpoint_name(&data_dir);
        let token = ipc::random_hex(32);
        let handler = Arc::new(handler::AppBridge::new(app.clone()));
        match ipc::serve(&endpoint, token.clone(), handler).await {
            Ok(server) => {
                let info = EndpointInfo {
                    endpoint,
                    token,
                    pid: std::process::id(),
                    app_version: app.package_info().version.to_string(),
                    app_path: std::env::current_exe()
                        .ok()
                        .map(|p| p.to_string_lossy().to_string()),
                };
                if let Err(e) = ipc::write_endpoint_file(&data_dir, &info) {
                    tracing::error!(error = %e, "cannot publish the browser integration endpoint");
                }
                *app.state::<Bridge>().server.lock() = Some(server);
                tracing::info!("browser integration endpoint ready");
            }
            Err(e) => tracing::error!(error = %e, "cannot start the browser integration endpoint"),
        }
        let results = register(&state);
        for r in results.iter().filter(|r| r.installed) {
            tracing::info!(browser = %r.id, registered = r.registered, error = ?r.error, "native host registration");
        }
    });
}

/// Remove the endpoint file on exit so hosts do not try a dead endpoint.
pub fn shutdown(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        ipc::remove_endpoint_file(&state.data_dir);
    }
}

/// Push the current capture configuration to connected extensions.
pub fn push_config(app: &AppHandle, s: &AppSettings) {
    if let Some(server) = app.state::<Bridge>().server.lock().as_ref() {
        server.notify_all(ExtReply::Config {
            config: browser_config(s),
        });
    }
}
