use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use velox_types::AppSettings;

use crate::error::{CmdResult, CommandError};
use crate::AppState;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.manager.settings()
}

/// Validate, persist and apply settings. Returns the normalized document.
#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> CmdResult<AppSettings> {
    let dir = settings.general.download_dir.trim();
    if !dir.is_empty() && !std::path::Path::new(dir).is_absolute() {
        return Err(CommandError::new(
            "invalid_setting",
            "the download folder must be an absolute path",
        ));
    }
    let temp = settings.downloads.temp_dir.trim();
    if !temp.is_empty() && !std::path::Path::new(temp).is_absolute() {
        return Err(CommandError::new(
            "invalid_setting",
            "the temporary folder must be an absolute path",
        ));
    }
    let previous = state.manager.settings();
    let saved = state.manager.update_settings(settings)?;

    if previous.general.launch_at_startup != saved.general.launch_at_startup {
        let al = app.autolaunch();
        let res = if saved.general.launch_at_startup {
            al.enable()
        } else {
            al.disable()
        };
        if let Err(e) = res {
            tracing::warn!(error = %e, "cannot change autostart");
        }
    }
    if previous.appearance.language != saved.appearance.language
        || previous.downloads.speed_limit != saved.downloads.speed_limit
    {
        crate::tray::refresh(&app);
    }
    if previous.browser != saved.browser {
        crate::integration::push_config(&app, &saved);
        if previous.browser.extra_allowed_extension_ids != saved.browser.extra_allowed_extension_ids
        {
            crate::integration::register(&state);
        }
    }
    // Limits and power policy may allow (or hold) queued downloads now.
    app.state::<velox_scheduler::Scheduler>().wake();
    let _ = app.emit("app://settings-changed", &saved);
    Ok(saved)
}

/// Store (or clear) the proxy password in the encrypted secret store.
#[tauri::command]
pub fn set_proxy_password(
    state: State<'_, AppState>,
    password: Option<String>,
) -> CmdResult<AppSettings> {
    let password = password.filter(|p| !p.is_empty());
    let db = state.manager.db().clone();
    match (&state.secret_box, &password) {
        (Some(sb), Some(p)) => db.put_app_secret(sb, "proxy", p)?,
        (Some(_), None) => db.delete_secrets("app:proxy")?,
        (None, _) => {}
    }
    state.manager.set_proxy_password(password.clone());
    let mut s = state.manager.settings();
    s.network.proxy.has_password = password.is_some();
    Ok(state.manager.update_settings(s)?)
}
