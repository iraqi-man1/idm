use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;
use velox_types::{AppInfo, StatsSummary};

use crate::error::{CmdResult, CommandError};
use crate::AppState;

#[tauri::command]
pub fn get_app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: velox_types::APP_NAME.into(),
        version: app.package_info().version.to_string(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        data_dir: state.data_dir.to_string_lossy().to_string(),
        log_dir: state.log_dir.to_string_lossy().to_string(),
        secure_storage: state.secret_box.is_some(),
        updater_configured: state.updater_configured,
        protocol_version: velox_types::PROTOCOL_VERSION,
    }
}

#[tauri::command]
pub fn get_stats(state: State<'_, AppState>) -> CmdResult<StatsSummary> {
    Ok(state.manager.db().stats_summary()?)
}

#[tauri::command]
pub fn default_download_dir(state: State<'_, AppState>) -> String {
    state
        .manager
        .target_dir(velox_types::Category::Other)
        .to_string_lossy()
        .to_string()
}

/// Open (or focus) the detailed progress window of a download.
#[tauri::command]
pub fn open_progress_window(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<()> {
    let id = Uuid::parse_str(&id)?;
    let info = state
        .manager
        .get(id)
        .ok_or_else(|| CommandError::new("not_found", "download not found"))?;
    let label = format!("progress-{}", id.simple());
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(
        &app,
        label,
        WebviewUrl::App(format!("index.html#/progress/{id}").into()),
    )
    .title(info.file_name)
    .inner_size(640.0, 560.0)
    .min_inner_size(520.0, 460.0)
    .resizable(true)
    .center()
    .build()?;
    Ok(())
}

#[tauri::command]
pub fn show_main(app: AppHandle) {
    crate::show_main_window(&app);
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    crate::quit(&app);
}
