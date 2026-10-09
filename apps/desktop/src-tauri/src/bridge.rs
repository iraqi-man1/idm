//! Forwards engine events to the UI and raises desktop notifications.

use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;
use velox_core::DownloadManager;
use velox_types::EngineEvent;

/// Name of the event carrying [`EngineEvent`] payloads to the UI.
pub const ENGINE_EVENT: &str = "engine://event";

pub fn spawn(app: AppHandle, manager: DownloadManager) {
    let mut rx = manager.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    notify(&app, &manager, &ev);
                    if let EngineEvent::Progress { .. } = &ev {
                        crate::tray::update_tooltip(&app, &manager);
                    }
                    let _ = app.emit(ENGINE_EVENT, &ev);
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::debug!(skipped = n, "UI event bridge lagged");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn notify(app: &AppHandle, manager: &DownloadManager, ev: &EngineEvent) {
    let s = manager.settings();
    let ar = s.appearance.language == "ar";
    let (title, body) = match ev {
        EngineEvent::Completed { download } if s.general.notify_on_complete => (
            if ar {
                "اكتمل التنزيل"
            } else {
                "Download complete"
            },
            download.file_name.clone(),
        ),
        EngineEvent::Failed { download } if s.general.notify_on_error => (
            if ar {
                "فشل التنزيل"
            } else {
                "Download failed"
            },
            format!(
                "{}: {}",
                download.file_name,
                download.error.clone().unwrap_or_default()
            ),
        ),
        _ => return,
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::debug!(error = %e, "notification failed");
    }
}
