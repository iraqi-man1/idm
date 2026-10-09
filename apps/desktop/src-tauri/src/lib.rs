//! Velox Download Manager — Tauri shell.
//!
//! Responsibilities: open the database and engine, bridge engine events to
//! the UI, expose typed commands, own the tray icon and windows, and run the
//! browser-integration IPC endpoint. All download logic lives in the
//! engine crates.

mod bridge;
mod clipboard;
mod commands;
mod error;
mod integration;
mod media_bridge;
mod post_action;
mod queues;
mod security;
mod state;
mod tray;

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use velox_core::{DownloadManager, ManagerConfig};
use velox_persistence::{Database, SecretBox};

pub use state::AppState;

/// Command-line flags understood by the app.
#[derive(Debug, Default, Clone)]
pub struct LaunchArgs {
    /// Started by the browser's native host or autostart: stay in the tray.
    pub background: bool,
    /// URLs passed on the command line to add.
    pub urls: Vec<String>,
}

impl LaunchArgs {
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Self {
        let mut out = LaunchArgs::default();
        for a in args.into_iter().skip(1) {
            match a.as_str() {
                "--background" | "--minimized" | "--hidden" => out.background = true,
                s if s.starts_with("http://")
                    || s.starts_with("https://")
                    || s.starts_with("ftp://") =>
                {
                    out.urls.push(s.to_string())
                }
                _ => {}
            }
        }
        out
    }
}

fn init_logging(log_dir: &PathBuf) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};
    std::fs::create_dir_all(log_dir).ok()?;
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("velox")
        .filename_suffix("log")
        .max_log_files(7)
        .build(log_dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(file);
    let filter = EnvFilter::try_from_env("VELOX_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let registry = tracing_subscriber::registry().with(filter).with(
        fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .with_target(true),
    );
    if cfg!(debug_assertions) {
        let _ = registry
            .with(fmt::layer().with_writer(std::io::stderr))
            .try_init();
    } else {
        let _ = registry.try_init();
    }
    Some(guard)
}

/// Show and focus the main window.
pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn handle_launch_args(app: &AppHandle, args: &LaunchArgs, first_launch: bool) {
    let start_minimized = app
        .try_state::<AppState>()
        .map(|s| s.manager.settings().general.start_minimized)
        .unwrap_or(false);
    let stay_hidden = args.background || (first_launch && start_minimized);
    if !stay_hidden || !args.urls.is_empty() {
        show_main_window(app);
    }
    for url in &args.urls {
        let _ = app.emit("app://add-url", url);
    }
}

/// Quit for real: stop downloads gracefully, then exit.
pub fn quit(app: &AppHandle) {
    let app = app.clone();
    if let Some(state) = app.try_state::<AppState>() {
        if state.quitting.swap(true, Ordering::SeqCst) {
            return;
        }
        let mgr = state.manager.clone();
        tauri::async_runtime::spawn(async move {
            mgr.shutdown().await;
            integration::shutdown(&app);
            app.exit(0);
        });
    } else {
        app.exit(0);
    }
}

fn updater_configured(ctx: &tauri::Context<tauri::Wry>) -> bool {
    ctx.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .is_some_and(|k| !k.trim().is_empty())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let launch = LaunchArgs::parse(std::env::args());
    let context = tauri::generate_context!();
    let has_updater = updater_configured(&context);

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let args = LaunchArgs::parse(argv);
            handle_launch_args(app, &args, false);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ));
    if has_updater {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    let launch_for_setup = launch.clone();
    let app = builder
        .setup(move |app| {
            let handle = app.handle().clone();
            let data_dir = app.path().app_data_dir()?;
            let log_dir = app.path().app_log_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let log_guard = init_logging(&log_dir);
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "starting Velox Download Manager"
            );

            let db = Database::open(&data_dir.join("velox.sqlite"))
                .map_err(|e| format!("cannot open database: {e}"))?;
            if !db.quick_check().unwrap_or(false) {
                tracing::error!("database integrity check failed");
            }
            let key = security::keystore::load_master_key();
            let secret_box = key.map(|k| SecretBox::new(&k));
            let proxy_password = secret_box
                .as_ref()
                .and_then(|sb| db.get_app_secret(sb, "proxy").ok().flatten());

            let (manager, interrupted) =
                tauri::async_runtime::block_on(DownloadManager::open(ManagerConfig {
                    db,
                    secret_box: secret_box.clone(),
                    proxy_password,
                }))
                .map_err(|e| format!("cannot start the download engine: {e}"))?;

            // The media runner must be in place before media downloads resume.
            let media = media_bridge::Media::new(&data_dir);
            manager.set_media_runner(media.engine.clone());
            app.manage(media);

            for id in &interrupted {
                if let Err(e) = manager.start(*id) {
                    tracing::warn!(%id, error = %e, "could not resume interrupted download");
                }
            }

            app.manage(AppState::new(
                manager.clone(),
                data_dir,
                log_dir,
                secret_box,
                has_updater,
                log_guard,
            ));
            let scheduler = tauri::async_runtime::block_on(async {
                velox_scheduler::Scheduler::start(manager.clone(), Default::default())
            });
            queues::spawn_events(handle.clone(), &scheduler);
            app.manage(scheduler);
            app.manage(post_action::PostActions::default());
            clipboard::spawn(handle.clone());
            app.manage(integration::Bridge::default());
            app.manage(integration::capture::Captures::default());
            bridge::spawn(handle.clone(), manager.clone());
            integration::start(&handle);
            tray::create(&handle)?;
            handle_launch_args(&handle, &launch_for_setup, true);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let app = window.app_handle();
                    let quitting = app.state::<AppState>().quitting.load(Ordering::SeqCst);
                    let to_tray = app
                        .state::<AppState>()
                        .manager
                        .settings()
                        .general
                        .close_to_tray;
                    if !quitting {
                        api.prevent_close();
                        if to_tray {
                            let _ = window.hide();
                        } else {
                            quit(app);
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::downloads::list_downloads,
            commands::downloads::get_download,
            commands::downloads::probe_url,
            commands::downloads::add_download,
            commands::downloads::add_batch,
            commands::downloads::start_download,
            commands::downloads::pause_download,
            commands::downloads::cancel_download,
            commands::downloads::restart_download,
            commands::downloads::remove_downloads,
            commands::downloads::start_many,
            commands::downloads::pause_many,
            commands::downloads::pause_all,
            commands::downloads::resume_all,
            commands::downloads::open_file,
            commands::downloads::open_folder,
            commands::downloads::move_download,
            commands::downloads::rename_download,
            commands::downloads::update_download_url,
            commands::downloads::set_download_speed_limit,
            commands::downloads::set_download_connections,
            commands::downloads::verify_checksum,
            commands::downloads::get_download_log,
            commands::downloads::get_speed_history,
            commands::downloads::get_progress,
            commands::downloads::find_duplicates,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::set_proxy_password,
            commands::system::get_app_info,
            commands::system::get_stats,
            commands::system::open_progress_window,
            commands::system::show_main,
            commands::system::quit_app,
            commands::system::default_download_dir,
            integration::commands::browser_integration_status,
            integration::commands::repair_browser_integration,
            integration::commands::get_pending_capture,
            integration::commands::probe_capture,
            integration::commands::resolve_capture,
            integration::commands::open_extension_folder,
            media_bridge::get_media_tools,
            media_bridge::probe_media,
            queues::list_queues,
            queues::create_queue,
            queues::update_queue,
            queues::delete_queue,
            queues::start_queue,
            queues::stop_queue,
            queues::set_download_queue,
            queues::get_power_hold,
            post_action::cancel_post_action,
            post_action::run_post_action_now,
        ])
        .build(context)
        .expect("error while building the application");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, code, .. } = &event {
            // Closing the last window must not quit while running in the tray.
            let quitting = app.state::<AppState>().quitting.load(Ordering::SeqCst);
            if code.is_none() && !quitting {
                api.prevent_exit();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::LaunchArgs;

    #[test]
    fn parses_launch_args() {
        let a = LaunchArgs::parse(
            ["velox", "--background", "https://e.com/a.zip", "--unknown"].map(String::from),
        );
        assert!(a.background);
        assert_eq!(a.urls, vec!["https://e.com/a.zip".to_string()]);
        let b = LaunchArgs::parse(["velox"].map(String::from));
        assert!(!b.background && b.urls.is_empty());
    }
}
