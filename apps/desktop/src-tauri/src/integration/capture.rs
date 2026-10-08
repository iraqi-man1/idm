//! "Download file info" dialog for downloads handed over by the browser.
//!
//! The captured request (including cookies) is kept in memory only, keyed
//! by a random id that the dialog window uses to fetch and resolve it.

use std::collections::HashMap;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use velox_types::{AddDownloadRequest, DownloadId, PendingCapture};

#[derive(Default)]
pub struct Captures {
    pending: Mutex<HashMap<String, PendingCapture>>,
}

impl Captures {
    pub fn get(&self, id: &str) -> Option<PendingCapture> {
        self.pending.lock().get(id).cloned()
    }

    pub fn take(&self, id: &str) -> Option<PendingCapture> {
        self.pending.lock().remove(id)
    }
}

/// Show the dialog for a captured download.
pub fn open(
    app: &AppHandle,
    request: AddDownloadRequest,
    duplicate_of: Option<DownloadId>,
) -> tauri::Result<()> {
    let id = velox_nm::ipc::random_hex(8);
    app.state::<Captures>().pending.lock().insert(
        id.clone(),
        PendingCapture {
            id: id.clone(),
            request,
            duplicate_of,
        },
    );
    let label = format!("capture-{id}");
    let window = WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::App(format!("index.html#/capture/{id}").into()),
    )
    .title("Velox — Download file info")
    .inner_size(600.0, 420.0)
    .min_inner_size(520.0, 380.0)
    .resizable(true)
    .always_on_top(true)
    .center()
    .focused(true)
    .build()?;
    let app2 = app.clone();
    let id2 = id.clone();
    window.on_window_event(move |e| {
        if let tauri::WindowEvent::Destroyed = e {
            // Closing the window without choosing discards the capture.
            app2.state::<Captures>().take(&id2);
        }
    });
    Ok(())
}
