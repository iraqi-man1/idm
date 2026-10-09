//! `velox-nmh` — the native messaging host started by the browser.
//!
//! The browser launches this process when the extension calls
//! `runtime.connectNative("com.veloxdm.host")` and talks to it over
//! stdin/stdout. The host
//!
//! 1. verifies the calling extension (origin passed by the browser on the
//!    command line) against the allow-list,
//! 2. validates every message against the protocol schema,
//! 3. connects to the running desktop app over authenticated local IPC,
//!    starting the app in the background when an action needs it,
//! 4. relays requests and responses.
//!
//! Nothing but protocol frames is ever written to stdout.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::AsyncWrite;
use tokio::sync::Mutex;
use velox_nm::framing::{read_frame, read_native, write_ipc, write_native, MAX_IPC};
use velox_nm::ipc::{self, ConnectError, EndpointInfo};
use velox_nm::manifest;
use velox_types::protocol::{ExtMessage, ExtRequest, ExtResponse};

struct Logger {
    path: Option<PathBuf>,
}

impl Logger {
    fn new(data_dir: Option<&Path>) -> Self {
        let path = data_dir.map(|d| d.join("logs").join("velox-nmh.log"));
        if let Some(p) = &path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // Keep the log small.
            if std::fs::metadata(p)
                .map(|m| m.len() > 512 * 1024)
                .unwrap_or(false)
            {
                let _ = std::fs::remove_file(p);
            }
        }
        Self { path }
    }

    fn log(&self, msg: &str) {
        let line = format!("{} [{}] {msg}\n", velox_types::now_ms(), std::process::id());
        if let Some(p) = &self.path {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
            {
                let _ = f.write_all(line.as_bytes());
            }
        }
        // stderr goes to the browser's log on most platforms.
        eprint!("velox-nmh: {line}");
    }
}

/// Identify the caller from the browser-provided arguments.
/// Chromium: `velox-nmh chrome-extension://<id>/ [--parent-window=N]`
/// Firefox:  `velox-nmh <manifest path> <extension id>`
fn caller(args: &[String]) -> Option<(String, String)> {
    if let Some(o) = args
        .iter()
        .skip(1)
        .find(|a| a.starts_with("chrome-extension://"))
    {
        return Some((o.clone(), "chromium".into()));
    }
    if args.len() >= 3 && !args[2].starts_with("--") {
        return Some((args[2].clone(), "firefox".into()));
    }
    None
}

fn extra_allowed_ids(data_dir: Option<&Path>) -> Vec<String> {
    data_dir
        .and_then(|d| std::fs::read(d.join("allowed-extensions.json")).ok())
        .and_then(|raw| serde_json::from_slice::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

fn app_binary(info: Option<&EndpointInfo>) -> Option<PathBuf> {
    if let Some(p) = info.and_then(|i| i.app_path.as_ref()).map(PathBuf::from) {
        if p.exists() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "velox-desktop.exe"
    } else {
        "velox-desktop"
    };
    let p = exe.parent()?.join(name);
    p.exists().then_some(p)
}

/// Start the desktop app detached from the browser's process tree.
fn launch_app(path: &Path, log: &Logger) -> bool {
    let mut cmd = std::process::Command::new(path);
    cmd.arg("--background")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        // Browsers run hosts inside a job object that is killed with the
        // host; break away so the app keeps running.
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        if cmd.spawn().is_ok() {
            log.log(&format!("launched {}", path.display()));
            return true;
        }
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    match cmd.spawn() {
        Ok(_) => {
            log.log(&format!("launched {}", path.display()));
            true
        }
        Err(e) => {
            log.log(&format!("cannot launch {}: {e}", path.display()));
            false
        }
    }
}

type Stdout = Arc<Mutex<tokio::io::Stdout>>;

async fn send(stdout: &Stdout, resp: &ExtResponse) {
    let mut out = stdout.lock().await;
    let _ = write_native(&mut *out, resp).await;
}

struct AppLink {
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    alive: Arc<std::sync::atomic::AtomicBool>,
}

async fn open_link(
    data_dir: &Path,
    origin: &str,
    browser: &str,
    launch: bool,
    stdout: &Stdout,
    pending: &Arc<Mutex<HashSet<u64>>>,
    log: &Logger,
) -> Result<AppLink, (String, String)> {
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut launched = false;
    loop {
        let info = ipc::read_endpoint_file(data_dir).ok();
        let attempt = match &info {
            Some(i) => ipc::connect(i, origin, browser).await,
            None => Err(ConnectError::NotRunning),
        };
        match attempt {
            Ok(client) => {
                log.log(&format!("connected to app {}", client.app_version));
                let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
                let alive2 = alive.clone();
                let stdout = stdout.clone();
                let pending = pending.clone();
                let mut reader = client.reader;
                tokio::spawn(async move {
                    while let Ok(Some(raw)) = read_frame(&mut reader, MAX_IPC, true).await {
                        // A frame this host does not understand (newer app) is
                        // skipped rather than dropping the connection.
                        if let Ok(resp) = serde_json::from_slice::<ExtResponse>(&raw) {
                            pending.lock().await.remove(&resp.id);
                            send(&stdout, &resp).await;
                        }
                    }
                    alive2.store(false, std::sync::atomic::Ordering::SeqCst);
                    // Fail requests that will never be answered.
                    let ids: Vec<u64> = pending.lock().await.drain().collect();
                    for id in ids {
                        send(
                            &stdout,
                            &ExtResponse::error(
                                id,
                                "app_disconnected",
                                "the desktop app closed the connection",
                            ),
                        )
                        .await;
                    }
                });
                return Ok(AppLink {
                    writer: client.writer,
                    alive,
                });
            }
            Err(ConnectError::NotRunning) | Err(ConnectError::Io(_)) => {
                if !launch {
                    return Err((
                        "app_not_running".into(),
                        "Velox Download Manager is not running".into(),
                    ));
                }
                if !launched {
                    match app_binary(info.as_ref()) {
                        Some(p) => launched = launch_app(&p, log),
                        None => {
                            return Err((
                                "app_not_found".into(),
                                "the Velox application was not found next to the host".into(),
                            ))
                        }
                    }
                    if !launched {
                        return Err((
                            "app_launch_failed".into(),
                            "could not start Velox Download Manager".into(),
                        ));
                    }
                }
                if Instant::now() > deadline {
                    return Err((
                        "app_start_timeout".into(),
                        "Velox Download Manager did not start in time".into(),
                    ));
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Err(e) => {
                log.log(&format!("connect failed: {e}"));
                return Err(("app_unavailable".into(), e.to_string()));
            }
        }
    }
}

fn needs_app_launch(m: &ExtMessage) -> bool {
    matches!(
        m,
        ExtMessage::AddDownload { .. }
            | ExtMessage::AddBatch { .. }
            | ExtMessage::ProbeMedia { .. }
            | ExtMessage::DownloadMedia { .. }
            | ExtMessage::ShowApp
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data_dir = std::env::var_os("VELOX_DATA_DIR")
        .map(PathBuf::from)
        .or_else(ipc::app_data_dir);
    let log = Logger::new(data_dir.as_deref());
    let Some((origin, browser)) = caller(&args) else {
        log.log("started without a browser caller; this program is launched by web browsers");
        std::process::exit(2);
    };
    let extra = extra_allowed_ids(data_dir.as_deref());
    if !manifest::origin_allowed(&origin, &extra) {
        log.log(&format!("rejected caller {origin}"));
        std::process::exit(3);
    }
    let Some(data_dir) = data_dir else {
        log.log("cannot determine the application data directory");
        std::process::exit(4);
    };
    log.log(&format!("started by {browser} ({origin})"));

    let stdout: Stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let pending: Arc<Mutex<HashSet<u64>>> = Arc::default();
    let mut stdin = tokio::io::stdin();
    let mut link: Option<AppLink> = None;

    loop {
        let raw = match read_native(&mut stdin).await {
            Ok(Some(r)) => r,
            Ok(None) => break, // browser closed the port
            Err(e) => {
                log.log(&format!("stdin error: {e}"));
                break;
            }
        };
        let value: serde_json::Value = match serde_json::from_slice(&raw) {
            Ok(v) => v,
            Err(e) => {
                send(
                    &stdout,
                    &ExtResponse::error(0, "invalid_json", e.to_string()),
                )
                .await;
                continue;
            }
        };
        let id = value.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let req: ExtRequest = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => {
                send(
                    &stdout,
                    &ExtResponse::error(id, "invalid_request", e.to_string()),
                )
                .await;
                continue;
            }
        };
        if let Err(e) = req.validate() {
            send(&stdout, &ExtResponse::error(id, "invalid_request", e.0)).await;
            continue;
        }

        let mut delivered = false;
        for _ in 0..2 {
            if link
                .as_ref()
                .is_none_or(|l| !l.alive.load(std::sync::atomic::Ordering::SeqCst))
            {
                match open_link(
                    &data_dir,
                    &origin,
                    &browser,
                    needs_app_launch(&req.message),
                    &stdout,
                    &pending,
                    &log,
                )
                .await
                {
                    Ok(l) => link = Some(l),
                    Err((code, msg)) => {
                        send(&stdout, &ExtResponse::error(id, &code, msg)).await;
                        delivered = true;
                        break;
                    }
                }
            }
            let l = link.as_mut().expect("link");
            pending.lock().await.insert(id);
            match write_ipc(&mut l.writer, &req).await {
                Ok(()) => {
                    delivered = true;
                    break;
                }
                Err(e) => {
                    pending.lock().await.remove(&id);
                    log.log(&format!("write to app failed: {e}; reconnecting"));
                    link = None;
                }
            }
        }
        if !delivered {
            send(
                &stdout,
                &ExtResponse::error(id, "app_unavailable", "could not reach the desktop app"),
            )
            .await;
        }
    }
    log.log("browser disconnected; exiting");
}
