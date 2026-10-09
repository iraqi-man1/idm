//! System tray icon and menu.

use std::sync::atomic::{AtomicI64, Ordering};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use velox_core::DownloadManager;

use crate::AppState;

const TRAY_ID: &str = "velox-tray";
const LIMITS: &[(u64, &str)] = &[
    (0, "limit-0"),
    (256 * 1024, "limit-256k"),
    (1024 * 1024, "limit-1m"),
    (4 * 1024 * 1024, "limit-4m"),
    (10 * 1024 * 1024, "limit-10m"),
];

struct Labels {
    open: &'static str,
    pause_all: &'static str,
    resume_all: &'static str,
    limit: &'static str,
    unlimited: &'static str,
    quit: &'static str,
}

fn labels(lang: &str) -> Labels {
    if lang == "ar" {
        Labels {
            open: "فتح Velox",
            pause_all: "إيقاف الكل مؤقتاً",
            resume_all: "استئناف الكل",
            limit: "حد السرعة",
            unlimited: "بلا حد",
            quit: "خروج",
        }
    } else {
        Labels {
            open: "Open Velox",
            pause_all: "Pause all",
            resume_all: "Resume all",
            limit: "Speed limit",
            unlimited: "Unlimited",
            quit: "Quit",
        }
    }
}

fn limit_label(bytes: u64, unlimited: &str) -> String {
    match bytes {
        0 => unlimited.to_string(),
        b if b >= 1024 * 1024 => format!("{} MB/s", b / (1024 * 1024)),
        b => format!("{} KB/s", b / 1024),
    }
}

fn build_menu(app: &AppHandle, lang: &str, current_limit: u64) -> tauri::Result<Menu<Wry>> {
    let l = labels(lang);
    let open = MenuItem::with_id(app, "open", l.open, true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause-all", l.pause_all, true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume-all", l.resume_all, true, None::<&str>)?;
    let mut checks = Vec::new();
    for (bytes, id) in LIMITS {
        checks.push(CheckMenuItem::with_id(
            app,
            *id,
            limit_label(*bytes, l.unlimited),
            true,
            *bytes == current_limit,
            None::<&str>,
        )?);
    }
    let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = checks
        .iter()
        .map(|c| c as &dyn tauri::menu::IsMenuItem<Wry>)
        .collect();
    let limit = Submenu::with_items(app, l.limit, true, &refs)?;
    let quit = MenuItem::with_id(app, "quit", l.quit, true, None::<&str>)?;
    Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &resume,
            &limit,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let s = app.state::<AppState>().manager.settings();
    let menu = build_menu(app, &s.appearance.language, s.downloads.speed_limit)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Velox Download Manager")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Rebuild the menu (language or speed limit changed).
pub fn refresh(app: &AppHandle) {
    let s = app.state::<AppState>().manager.settings();
    if let (Some(tray), Ok(menu)) = (
        app.tray_by_id(TRAY_ID),
        build_menu(app, &s.appearance.language, s.downloads.speed_limit),
    ) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn handle_menu(app: &AppHandle, id: &str) {
    let mgr = app.state::<AppState>().manager.clone();
    match id {
        "open" => crate::show_main_window(app),
        "pause-all" => {
            tauri::async_runtime::spawn(async move { mgr.pause_all().await });
        }
        "resume-all" => crate::commands::downloads::resume_all_impl(&mgr),
        "quit" => crate::quit(app),
        other => {
            if let Some((bytes, _)) = LIMITS.iter().find(|(_, lid)| *lid == other) {
                if let Err(e) = mgr.set_global_speed_limit(*bytes) {
                    tracing::warn!(error = %e, "cannot set speed limit");
                }
                refresh(app);
                use tauri::Emitter;
                let _ = app.emit("app://settings-changed", mgr.settings());
            }
        }
    }
}

static LAST_TOOLTIP: AtomicI64 = AtomicI64::new(0);

/// Update the tray tooltip with the aggregate speed (at most every 2 s).
pub fn update_tooltip(app: &AppHandle, manager: &DownloadManager) {
    let now = velox_types::now_ms();
    if now - LAST_TOOLTIP.load(Ordering::Relaxed) < 2000 {
        return;
    }
    LAST_TOOLTIP.store(now, Ordering::Relaxed);
    let snaps = manager.progress_snapshots();
    let speed: u64 = snaps.iter().map(|s| s.speed).sum();
    let text = if snaps.is_empty() {
        "Velox Download Manager".to_string()
    } else {
        format!("Velox — {} active · {}/s", snaps.len(), human_bytes(speed))
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(text));
    }
}

fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}
