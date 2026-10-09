//! Actions after a queue finishes (quit, sleep, hibernate, shut down),
//! always preceded by a cancellable countdown shown in the main window.

use std::path::PathBuf;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;
use velox_types::PostAction;

use crate::AppState;

/// Seconds the user has to cancel.
pub const COUNTDOWN_SECS: u64 = 60;

#[derive(Default)]
pub struct PostActions {
    /// `true` = run now, `false` (or dropped) = cancel.
    pending: Mutex<Option<oneshot::Sender<bool>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostActionNotice {
    pub action: PostAction,
    pub queue: String,
    pub seconds: u64,
}

/// What the operating system is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OsAction {
    Run {
        program: PathBuf,
        args: Vec<String>,
    },
    /// `SetSuspendState` (Windows).
    Suspend {
        hibernate: bool,
    },
}

/// The OS command for a power action on this platform.
pub fn os_action(action: PostAction) -> Result<Option<OsAction>, String> {
    let run = |program: PathBuf, args: &[&str]| {
        Ok(Some(OsAction::Run {
            program,
            args: args.iter().map(|s| s.to_string()).collect(),
        }))
    };
    match action {
        PostAction::None | PostAction::Exit => Ok(None),
        _ if cfg!(windows) => match action {
            PostAction::Shutdown => {
                let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
                run(
                    PathBuf::from(root).join("System32").join("shutdown.exe"),
                    &["/s", "/t", "0"],
                )
            }
            PostAction::Sleep => Ok(Some(OsAction::Suspend { hibernate: false })),
            _ => Ok(Some(OsAction::Suspend { hibernate: true })),
        },
        _ if cfg!(target_os = "macos") => match action {
            PostAction::Shutdown => run(
                "/usr/bin/osascript".into(),
                &["-e", "tell application \"System Events\" to shut down"],
            ),
            PostAction::Sleep => run("/usr/bin/pmset".into(), &["sleepnow"]),
            _ => Err("hibernation is not supported on macOS".into()),
        },
        _ => {
            let systemctl = ["/usr/bin/systemctl", "/bin/systemctl"]
                .iter()
                .map(PathBuf::from)
                .find(|p| p.is_file())
                .ok_or("systemctl was not found")?;
            let verb = match action {
                PostAction::Shutdown => "poweroff",
                PostAction::Sleep => "suspend",
                _ => "hibernate",
            };
            run(systemctl, &[verb])
        }
    }
}

fn execute(os: OsAction) -> Result<(), String> {
    match os {
        OsAction::Run { program, args } => {
            let out = std::process::Command::new(&program)
                .args(&args)
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|e| format!("{}: {e}", program.display()))?;
            if out.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        }
        #[cfg(windows)]
        OsAction::Suspend { hibernate } => {
            // SAFETY: plain Win32 call without pointers.
            let ok = unsafe {
                windows_sys::Win32::System::Power::SetSuspendState(hibernate, false, false)
            };
            if ok {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error().to_string())
            }
        }
        #[cfg(not(windows))]
        OsAction::Suspend { .. } => Err("not supported on this platform".into()),
    }
}

/// Start the countdown for `action` after queue `queue` finished.
pub fn begin(app: &AppHandle, action: PostAction, queue: &str) {
    if action == PostAction::None {
        return;
    }
    let (tx, rx) = oneshot::channel();
    // A newer countdown replaces (cancels) an older one.
    *app.state::<PostActions>().pending.lock() = Some(tx);
    crate::show_main_window(app);
    let _ = app.emit(
        "app://post-action",
        PostActionNotice {
            action,
            queue: queue.to_string(),
            seconds: COUNTDOWN_SECS,
        },
    );
    tracing::info!(?action, queue, "post-completion action scheduled");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let go = tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(COUNTDOWN_SECS)) => true,
            r = rx => r.unwrap_or(false),
        };
        if !go {
            tracing::info!("post-completion action cancelled");
            let _ = app.emit("app://post-action-cancelled", ());
            return;
        }
        app.state::<PostActions>().pending.lock().take();
        run(&app, action).await;
    });
}

async fn run(app: &AppHandle, action: PostAction) {
    tracing::info!(?action, "running post-completion action");
    if action == PostAction::Exit {
        crate::quit(app);
        return;
    }
    if action == PostAction::Shutdown {
        // Checkpoint and stop everything before the system goes down.
        app.state::<AppState>().manager.shutdown().await;
    }
    let result = os_action(action).and_then(|os| os.map(execute).unwrap_or(Ok(())));
    if let Err(e) = result {
        tracing::error!(?action, error = %e, "post-completion action failed");
        let _ = app.emit("app://post-action-failed", e);
    }
}

#[tauri::command]
pub fn cancel_post_action(app: AppHandle) {
    if let Some(tx) = app.state::<PostActions>().pending.lock().take() {
        let _ = tx.send(false);
    }
}

#[tauri::command]
pub fn run_post_action_now(app: AppHandle) {
    if let Some(tx) = app.state::<PostActions>().pending.lock().take() {
        let _ = tx.send(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_os_action_for_none_and_exit() {
        assert_eq!(os_action(PostAction::None), Ok(None));
        assert_eq!(os_action(PostAction::Exit), Ok(None));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_uses_systemctl() {
        if !std::path::Path::new("/usr/bin/systemctl").is_file()
            && !std::path::Path::new("/bin/systemctl").is_file()
        {
            assert!(os_action(PostAction::Shutdown).is_err());
            return;
        }
        for (a, verb) in [
            (PostAction::Shutdown, "poweroff"),
            (PostAction::Sleep, "suspend"),
            (PostAction::Hibernate, "hibernate"),
        ] {
            match os_action(a).unwrap().unwrap() {
                OsAction::Run { program, args } => {
                    assert!(program.ends_with("systemctl"));
                    assert_eq!(args, [verb]);
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_actions() {
        match os_action(PostAction::Shutdown).unwrap().unwrap() {
            OsAction::Run { program, args } => {
                assert!(program.ends_with("System32\\shutdown.exe"));
                assert_eq!(args, ["/s", "/t", "0"]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            os_action(PostAction::Sleep),
            Ok(Some(OsAction::Suspend { hibernate: false }))
        );
        assert_eq!(
            os_action(PostAction::Hibernate),
            Ok(Some(OsAction::Suspend { hibernate: true }))
        );
    }
}
