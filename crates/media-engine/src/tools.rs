//! Locating and running the bundled FFmpeg / ffprobe / yt-dlp binaries.
//!
//! Release builds only use binaries shipped with the application (Tauri
//! sidecars next to the executable). Debug builds additionally honour
//! `VELOX_FFMPEG`, `VELOX_FFPROBE`, `VELOX_YTDLP` and the system `PATH` so
//! developers can work without the bundled tools; the tool status reports
//! which kind is in use. Tools are always executed with an argument vector
//! (never through a shell) and with stdin closed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use velox_types::ToolStatus;

#[derive(Debug, Clone, Default)]
pub struct Tools {
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    pub ytdlp: Option<PathBuf>,
    /// Which of the tools came from the application bundle.
    pub bundled: [bool; 3],
}

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

fn find_in(dirs: &[PathBuf], base: &str) -> Option<PathBuf> {
    let name = exe_name(base);
    dirs.iter().map(|d| d.join(&name)).find(|p| p.is_file())
}

fn find_on_path(base: &str) -> Option<PathBuf> {
    let name = exe_name(base);
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(&name))
            .find(|p| p.is_file())
    })
}

impl Tools {
    /// Discover tools in `bundle_dirs` (normally the executable's directory).
    pub fn discover(bundle_dirs: &[PathBuf]) -> Self {
        let mut t = Tools::default();
        let names = ["ffmpeg", "ffprobe", "yt-dlp"];
        let envs = ["VELOX_FFMPEG", "VELOX_FFPROBE", "VELOX_YTDLP"];
        for (i, base) in names.iter().enumerate() {
            let mut found = find_in(bundle_dirs, base).map(|p| (p, true));
            if found.is_none() && cfg!(debug_assertions) {
                found = std::env::var_os(envs[i])
                    .map(PathBuf::from)
                    .filter(|p| p.is_file())
                    .or_else(|| find_on_path(base))
                    .map(|p| (p, false));
            }
            if let Some((p, bundled)) = found {
                t.bundled[i] = bundled;
                match i {
                    0 => t.ffmpeg = Some(p),
                    1 => t.ffprobe = Some(p),
                    _ => t.ytdlp = Some(p),
                }
            }
        }
        t
    }

    /// Directory of the running executable (where Tauri places sidecars).
    pub fn exe_dir() -> Vec<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
            .into_iter()
            .collect()
    }

    pub async fn status(&self) -> Vec<ToolStatus> {
        let mut out = Vec::new();
        let entries = [
            (
                "FFmpeg",
                &self.ffmpeg,
                vec!["-hide_banner", "-version"],
                self.bundled[0],
            ),
            (
                "ffprobe",
                &self.ffprobe,
                vec!["-hide_banner", "-version"],
                self.bundled[1],
            ),
            ("yt-dlp", &self.ytdlp, vec!["--version"], self.bundled[2]),
        ];
        for (name, path, args, bundled) in entries {
            let mut st = ToolStatus {
                name: name.to_string(),
                available: false,
                path: path.as_ref().map(|p| p.to_string_lossy().to_string()),
                version: None,
                error: None,
                bundled,
            };
            match path {
                None => st.error = Some("not found in the application bundle".into()),
                Some(p) => match run_capture(p, &args, Duration::from_secs(20)).await {
                    Ok(o) => {
                        st.available = true;
                        st.version = o.lines().next().map(|l| {
                            l.trim()
                                .trim_start_matches("ffmpeg version ")
                                .trim_start_matches("ffprobe version ")
                                .chars()
                                .take(80)
                                .collect()
                        });
                    }
                    Err(e) => st.error = Some(e),
                },
            }
            out.push(st);
        }
        out
    }
}

/// A command for a bundled tool: no shell, stdin closed, killed on drop,
/// no console window on Windows.
pub fn command(program: &Path) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    c.stdin(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Run a tool and return its stdout (or the error with stderr's tail).
pub async fn run_capture(
    program: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let mut c = command(program);
    c.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let out = tokio::time::timeout(timeout, c.output())
        .await
        .map_err(|_| format!("{} timed out", program.display()))?
        .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(tail(&err, 1200))
    }
}

/// Last `max` bytes of a message (UTF-8 safe), for error reporting.
pub fn tail(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}
