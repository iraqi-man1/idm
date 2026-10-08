//! Commands for the browser integration settings screen and the capture dialog.

use tauri::{AppHandle, Manager, State};
use velox_nm::manifest;
use velox_types::{
    AddDownloadRequest, BrowserInfo, BrowserIntegrationStatus, DownloadInfo, ExtensionSeen,
    PendingCapture, UrlInfo,
};

use super::{capture::Captures, host_path, register, Bridge};
use crate::error::{CmdResult, CommandError};
use crate::AppState;

fn store_url(_browser: &str) -> Option<String> {
    // Filled in once the extension is published in the respective store.
    None
}

fn bundled_extension_dir(app: &AppHandle, which: &str) -> Option<String> {
    let dir = app
        .path()
        .resource_dir()
        .ok()?
        .join("extensions")
        .join(which);
    dir.join("manifest.json")
        .exists()
        .then(|| dir.to_string_lossy().to_string())
}

fn to_info(s: manifest::BrowserStatus) -> BrowserInfo {
    BrowserInfo {
        store_url: store_url(&s.id),
        id: s.id,
        name: s.name,
        family: match s.family {
            manifest::Family::Chromium => "chromium".into(),
            manifest::Family::Firefox => "firefox".into(),
        },
        installed: s.installed,
        registered: s.registered,
        manifest_path: s.manifest_path,
        error: s.error,
    }
}

fn status(
    app: &AppHandle,
    state: &AppState,
    results: Option<Vec<manifest::BrowserStatus>>,
) -> BrowserIntegrationStatus {
    let host = host_path();
    let browsers = match (results, &host) {
        (Some(r), _) => r,
        (None, Some(h)) => manifest::status(h),
        (None, None) => manifest::status(std::path::Path::new("")),
    };
    let bridge = app.state::<Bridge>();
    let last = bridge.last_extension.lock().clone().or_else(|| {
        state
            .manager
            .db()
            .get_setting::<ExtensionSeen>("browser_extension")
            .ok()
            .flatten()
    });
    let connected = bridge
        .server
        .lock()
        .as_ref()
        .map(|s| s.connection_count() as u32)
        .unwrap_or(0);
    BrowserIntegrationStatus {
        browsers: browsers.into_iter().map(to_info).collect(),
        host_path: host.map(|h| h.to_string_lossy().to_string()),
        connected,
        last_extension: last,
        chromium_extension_dir: bundled_extension_dir(app, "chromium"),
        firefox_extension_dir: bundled_extension_dir(app, "firefox"),
        chromium_extension_id: manifest::CHROMIUM_DEV_EXTENSION_ID.into(),
        firefox_extension_id: manifest::FIREFOX_EXTENSION_ID.into(),
    }
}

#[tauri::command]
pub fn browser_integration_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> BrowserIntegrationStatus {
    status(&app, &state, None)
}

/// Re-register the native messaging host for all installed browsers.
#[tauri::command]
pub fn repair_browser_integration(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<BrowserIntegrationStatus> {
    if host_path().is_none() {
        return Err(CommandError::new("host_missing", "the native messaging host (velox-nmh) was not found next to the application; reinstall Velox"));
    }
    let results = register(&state);
    Ok(status(&app, &state, Some(results)))
}

/// The captured request for the dialog, without cookies or sensitive
/// headers (they never enter the web view).
#[tauri::command]
pub fn get_pending_capture(captures: State<'_, Captures>, id: String) -> CmdResult<PendingCapture> {
    let mut p = captures.get(&id).ok_or_else(|| {
        CommandError::new("not_found", "this download request is no longer available")
    })?;
    p.request.cookies = None;
    p.request.headers.clear();
    p.request.credentials = None;
    Ok(p)
}

/// Probe the captured URL with the browser's cookies.
#[tauri::command]
pub async fn probe_capture(
    state: State<'_, AppState>,
    captures: State<'_, Captures>,
    id: String,
) -> CmdResult<UrlInfo> {
    let p = captures.get(&id).ok_or_else(|| {
        CommandError::new("not_found", "this download request is no longer available")
    })?;
    Ok(state.manager.probe_url(&p.request).await?)
}

/// Accept (with the possibly edited request) or discard a captured download.
#[tauri::command]
pub async fn resolve_capture(
    state: State<'_, AppState>,
    captures: State<'_, Captures>,
    id: String,
    request: Option<AddDownloadRequest>,
) -> CmdResult<Option<DownloadInfo>> {
    let Some(pending) = captures.take(&id) else {
        return Err(CommandError::new(
            "not_found",
            "this download request is no longer available",
        ));
    };
    let Some(mut req) = request else {
        return Ok(None);
    };
    // Secrets and browser context come from the original capture; the
    // window never sees them.
    req.url = pending.request.url.clone();
    req.cookies = pending.request.cookies.clone();
    req.headers = pending.request.headers.clone();
    req.user_agent = pending.request.user_agent.clone();
    req.source = pending.request.source;
    Ok(Some(state.manager.add(req).await?))
}

/// Open the folder holding the bundled unpacked extension ("chromium" or "firefox").
#[tauri::command]
pub fn open_extension_folder(app: AppHandle, family: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let which = match family.as_str() {
        "chromium" | "firefox" => family.as_str(),
        _ => return Err(CommandError::new("invalid", "unknown extension family")),
    };
    let dir = bundled_extension_dir(&app, which).ok_or_else(|| {
        CommandError::new(
            "missing",
            "the extension files are not bundled with this build",
        )
    })?;
    app.opener()
        .open_path(dir, None::<&str>)
        .map_err(|e| CommandError::new("open_failed", e.to_string()))
}
