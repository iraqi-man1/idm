//! Integration with the bundled yt-dlp extractor for media pages.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;
use velox_types::{
    MediaFormat, MediaProbeResult, MediaRequest, MediaSourceKind, OutputContainer, SubtitleTrack,
};

use crate::tools::{command, tail};
use crate::{MediaError, RequestInfo};

/// Netscape cookie file for yt-dlp built from a `Cookie` header, scoped to
/// the URL's host. Created with owner-only permissions and removed by the
/// caller (see [`TempCookies`]).
pub struct TempCookies {
    path: PathBuf,
}

impl TempCookies {
    pub fn create(dir: &Path, url: &str, cookie_header: &str) -> std::io::Result<Self> {
        let u = url::Url::parse(url)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let host = u.host_str().unwrap_or("").to_string();
        let secure = if u.scheme() == "https" {
            "TRUE"
        } else {
            "FALSE"
        };
        let mut body = String::from("# Netscape HTTP Cookie File\n");
        for pair in cookie_header.split(';') {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if name.is_empty() || name.contains(['\t', '\n']) || value.contains(['\t', '\n']) {
                continue;
            }
            body.push_str(&format!(".{host}\tTRUE\t/\t{secure}\t0\t{name}\t{value}\n"));
        }
        std::fs::create_dir_all(dir)?;
        let path = dir.join("cookies.txt");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        opts.open(&path)?.write_all(body.as_bytes())?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempCookies {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn common_args(info: &RequestInfo, cookies: Option<&TempCookies>) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--ignore-config".into(),
        "--no-playlist".into(),
        "--no-warnings".into(),
        "--no-colors".into(),
        "--socket-timeout".into(),
        "30".into(),
    ];
    if let Some(ua) = info.user_agent.as_deref().filter(|u| !u.is_empty()) {
        a.extend(["--user-agent".into(), ua.into()]);
    }
    if let Some(r) = info.referer.as_deref().filter(|r| !r.is_empty()) {
        a.extend(["--referer".into(), r.into()]);
    }
    if let Some(c) = cookies {
        a.extend(["--cookies".into(), c.path().to_string_lossy().to_string()]);
    }
    a
}

/// First `ERROR:` line from yt-dlp's stderr, or its tail.
pub fn error_message(stderr: &str) -> String {
    stderr
        .lines()
        .find(|l| l.starts_with("ERROR:"))
        .map(|l| l.trim_start_matches("ERROR:").trim().to_string())
        .unwrap_or_else(|| tail(stderr, 600))
}

fn as_u64(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))
}

/// Convert yt-dlp's `-J` output into a probe result.
pub fn parse_info(json: &Value, url: &str) -> MediaProbeResult {
    let mut formats = Vec::new();
    let mut drm_count = 0;
    let list = json
        .get("formats")
        .and_then(|f| f.as_array())
        .cloned()
        .unwrap_or_default();
    let total = list.len();
    for f in list {
        if f.get("has_drm").and_then(|v| v.as_bool()).unwrap_or(false) {
            drm_count += 1;
            continue;
        }
        let field = |k: &str| f.get(k).and_then(|v| v.as_str());
        let vcodec = field("vcodec").filter(|v| *v != "none").map(str::to_string);
        let acodec = field("acodec").filter(|v| *v != "none").map(str::to_string);
        // "none" means absent; a missing codec means unknown (e.g. a plain
        // <video> file), decided from yt-dlp's video_ext or the extension.
        let has_video = match field("vcodec") {
            Some(v) => v != "none",
            None => match field("video_ext") {
                Some(e) => e != "none",
                None => {
                    f.get("height").is_some_and(|h| !h.is_null())
                        || matches!(
                            field("ext"),
                            Some(
                                "mp4"
                                    | "webm"
                                    | "mkv"
                                    | "mov"
                                    | "m4v"
                                    | "flv"
                                    | "ts"
                                    | "3gp"
                                    | "ogv"
                            )
                        )
                }
            },
        };
        // yt-dlp sets audio_ext only for audio-only formats, so an unknown
        // audio codec is assumed present.
        let has_audio = field("acodec").is_none_or(|a| a != "none");
        let protocol = f.get("protocol").and_then(|v| v.as_str()).unwrap_or("");
        if protocol.contains("mhtml") {
            continue; // storyboards
        }
        let id = f
            .get("format_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let height = f.get("height").and_then(as_u64).map(|h| h as u32);
        let note = f.get("format_note").and_then(|v| v.as_str()).unwrap_or("");
        formats.push(MediaFormat {
            label: format!(
                "{} {}",
                f.get("format").and_then(|v| v.as_str()).unwrap_or(&id),
                note
            )
            .trim()
            .to_string(),
            id,
            has_video,
            has_audio,
            ext: f.get("ext").and_then(|v| v.as_str()).map(str::to_string),
            width: f.get("width").and_then(as_u64).map(|w| w as u32),
            height,
            fps: f.get("fps").and_then(|v| v.as_f64()).map(|v| v as f32),
            bitrate: f
                .get("tbr")
                .and_then(|v| v.as_f64())
                .map(|v| (v * 1000.0) as u64),
            vcodec,
            acodec,
            filesize: f
                .get("filesize")
                .and_then(as_u64)
                .or_else(|| f.get("filesize_approx").and_then(as_u64)),
            language: f
                .get("language")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            url: None,
        });
    }
    let mut subtitles = Vec::new();
    for (key, auto) in [("subtitles", false), ("automatic_captions", true)] {
        if let Some(map) = json.get(key).and_then(|s| s.as_object()) {
            for (lang, entries) in map {
                let first = entries.as_array().and_then(|a| a.first());
                subtitles.push(SubtitleTrack {
                    language: lang.clone(),
                    name: first
                        .and_then(|e| e.get("name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    ext: first
                        .and_then(|e| e.get("ext"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    automatic: auto,
                    url: None,
                });
            }
        }
    }
    MediaProbeResult {
        kind: MediaSourceKind::Page,
        url: json
            .get("webpage_url")
            .and_then(|v| v.as_str())
            .unwrap_or(url)
            .to_string(),
        title: json
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        duration_secs: json.get("duration").and_then(|v| v.as_f64()),
        thumbnail: json
            .get("thumbnail")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        formats,
        subtitles,
        extractor: json
            .get("extractor_key")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        is_live: json
            .get("is_live")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        drm_protected: total > 0 && drm_count == total,
    }
}

pub async fn probe(
    ytdlp: &Path,
    info: &RequestInfo,
    work_dir: &Path,
) -> Result<MediaProbeResult, MediaError> {
    let cookies = match info.cookies.as_deref().filter(|c| !c.is_empty()) {
        Some(c) => Some(
            TempCookies::create(work_dir, &info.url, c)
                .map_err(|e| MediaError::Io(e.to_string()))?,
        ),
        None => None,
    };
    let mut args = common_args(info, cookies.as_ref());
    args.extend(["-J".into(), "--".into(), info.url.clone()]);
    let mut c = command(ytdlp);
    c.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = tokio::time::timeout(std::time::Duration::from_secs(120), c.output())
        .await
        .map_err(|_| MediaError::Tool("yt-dlp timed out while reading the page".into()))?
        .map_err(|e| MediaError::Tool(format!("cannot run yt-dlp: {e}")))?;
    if !out.status.success() {
        return Err(MediaError::Tool(error_message(&String::from_utf8_lossy(
            &out.stderr,
        ))));
    }
    let json: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| MediaError::Tool(format!("unexpected yt-dlp output: {e}")))?;
    Ok(parse_info(&json, &info.url))
}

/// yt-dlp `-f` expression for a selection.
pub fn format_spec(sel: &MediaRequest) -> String {
    let audio_only = sel.container.is_audio_only();
    match (&sel.format_id, &sel.audio_format_id) {
        (Some(v), Some(a)) => format!("{v}+{a}"),
        (Some(v), None) => v.clone(),
        (None, _) if audio_only => "ba/b".into(),
        (None, _) => match sel.max_height {
            Some(h) => format!("bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b"),
            None => "bv*+ba/b".into(),
        },
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    pub part: String,
}

const PROGRESS_PREFIX: &str = "VELOXPROG";
const FILE_PREFIX: &str = "VELOXFILE";

/// Parse one line printed by our `--progress-template`.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let rest = line.trim().strip_prefix(PROGRESS_PREFIX)?.trim();
    let f: Vec<&str> = rest.split_whitespace().collect();
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
    Some(Progress {
        downloaded: num(0)? as u64,
        total: num(1).or(num(2)).map(|v| v as u64),
        speed: num(3),
        eta: num(4).map(|v| v as u64),
        part: f.get(5).map(|s| s.to_string()).unwrap_or_default(),
    })
}

/// Download with yt-dlp into `dir` as `<stem>.<ext>`. Returns the final file.
#[allow(clippy::too_many_arguments)]
pub async fn download(
    ytdlp: &Path,
    ffmpeg: Option<&Path>,
    info: &RequestInfo,
    sel: &MediaRequest,
    dir: &Path,
    stem: &str,
    work_dir: &Path,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(&Progress),
) -> Result<PathBuf, MediaError> {
    let cookies = match info.cookies.as_deref().filter(|c| !c.is_empty()) {
        Some(c) => Some(
            TempCookies::create(work_dir, &info.url, c)
                .map_err(|e| MediaError::Io(e.to_string()))?,
        ),
        None => None,
    };
    let mut args = common_args(info, cookies.as_ref());
    args.extend([
        "--newline".into(),
        "--continue".into(),
        "--no-mtime".into(),
        "--progress-template".into(),
        format!(
            "download:{PROGRESS_PREFIX} %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s %(progress.speed)s %(progress.eta)s %(info.format_id)s"
        ),
        "--print".into(),
        format!("after_move:{FILE_PREFIX} %(filepath)s"),
        "-f".into(),
        format_spec(sel),
        "-P".into(),
        dir.to_string_lossy().to_string(),
        "-P".into(),
        format!("temp:{}", work_dir.to_string_lossy()),
        "-o".into(),
        format!("{}.%(ext)s", stem.replace('%', "%%")),
    ]);
    if let Some(ff) = ffmpeg {
        args.extend(["--ffmpeg-location".into(), ff.to_string_lossy().to_string()]);
    }
    match sel.container {
        OutputContainer::Mp4 => args.extend([
            "--merge-output-format".into(),
            "mp4".into(),
            "--remux-video".into(),
            "mp4".into(),
        ]),
        OutputContainer::Mkv => args.extend([
            "--merge-output-format".into(),
            "mkv".into(),
            "--remux-video".into(),
            "mkv".into(),
        ]),
        OutputContainer::M4a => args.extend(["-x".into(), "--audio-format".into(), "m4a".into()]),
        OutputContainer::Mp3 => args.extend([
            "-x".into(),
            "--audio-format".into(),
            "mp3".into(),
            "--audio-quality".into(),
            "2".into(),
        ]),
        OutputContainer::Original => {}
    }
    if !sel.subtitle_languages.is_empty() {
        args.extend([
            "--write-subs".into(),
            "--sub-langs".into(),
            sel.subtitle_languages.join(","),
        ]);
        if sel.embed_subtitles && !sel.container.is_audio_only() {
            args.push("--embed-subs".into());
        }
    }
    args.extend(["--".into(), info.url.clone()]);

    let mut c = command(ytdlp);
    c.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c
        .spawn()
        .map_err(|e| MediaError::Tool(format!("cannot run yt-dlp: {e}")))?;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let err_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            buf.push_str(&l);
            buf.push('\n');
            if buf.len() > 64 * 1024 {
                buf.drain(..32 * 1024);
            }
        }
        buf
    });
    let mut out_file: Option<PathBuf> = None;
    let mut lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(MediaError::Cancelled);
            }
            line = lines.next_line() => match line {
                Ok(Some(l)) => {
                    if let Some(p) = parse_progress(&l) {
                        on_progress(&p);
                    } else if let Some(f) = l.trim().strip_prefix(FILE_PREFIX) {
                        out_file = Some(PathBuf::from(f.trim()));
                    }
                }
                _ => break,
            }
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|e| MediaError::Tool(e.to_string()))?;
    let stderr = err_task.await.unwrap_or_default();
    if !status.success() {
        return Err(MediaError::Tool(error_message(&stderr)));
    }
    out_file
        .filter(|p| p.is_file())
        .ok_or_else(|| MediaError::Tool("yt-dlp finished without producing a file".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines() {
        let p = parse_progress("VELOXPROG 1048576 4194304 NA 524288.5 6 137").unwrap();
        assert_eq!(p.downloaded, 1_048_576);
        assert_eq!(p.total, Some(4_194_304));
        assert_eq!(p.eta, Some(6));
        assert_eq!(p.part, "137");
        let p = parse_progress("VELOXPROG 10 NA 900 NA NA 18").unwrap();
        assert_eq!(p.total, Some(900));
        assert_eq!(p.speed, None);
        assert!(parse_progress("[download] 10%").is_none());
    }

    #[test]
    fn format_specs() {
        let mut s = MediaRequest {
            format_id: Some("137".into()),
            audio_format_id: Some("140".into()),
            ..Default::default()
        };
        assert_eq!(format_spec(&s), "137+140");
        s.audio_format_id = None;
        assert_eq!(format_spec(&s), "137");
        s.format_id = None;
        s.max_height = Some(720);
        assert!(format_spec(&s).starts_with("bv*[height<=720]+ba"));
        s.container = OutputContainer::Mp3;
        assert_eq!(format_spec(&s), "ba/b");
    }

    #[test]
    fn info_json() {
        let j: Value = serde_json::json!({
            "title": "Clip", "duration": 12.5, "extractor_key": "Generic", "webpage_url": "https://e.com/p",
            "formats": [
                {"format_id": "sb0", "protocol": "mhtml", "ext": "mhtml"},
                {"format_id": "18", "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a", "height": 360, "tbr": 500.0, "filesize": 1000},
                {"format_id": "137", "ext": "mp4", "vcodec": "avc1", "acodec": "none", "height": 1080, "filesize_approx": 9000},
                {"format_id": "140", "ext": "m4a", "vcodec": "none", "acodec": "mp4a", "tbr": 128.0},
                {"format_id": "drm1", "has_drm": true, "vcodec": "avc1"}
            ],
            "subtitles": {"ar": [{"ext": "vtt", "name": "Arabic"}]}
        });
        let r = parse_info(&j, "https://e.com/p");
        assert_eq!(r.formats.len(), 3);
        assert!(!r.drm_protected);
        let f137 = r.formats.iter().find(|f| f.id == "137").unwrap();
        assert!(f137.has_video && !f137.has_audio);
        assert_eq!(f137.filesize, Some(9000));
        let f140 = r.formats.iter().find(|f| f.id == "140").unwrap();
        assert!(!f140.has_video && f140.has_audio);
        assert_eq!(r.subtitles[0].language, "ar");
        let only_drm = serde_json::json!({"formats": [{"format_id": "x", "has_drm": true}]});
        assert!(parse_info(&only_drm, "u").drm_protected);
    }

    #[test]
    fn cookie_file_is_scoped_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path;
        {
            let c =
                TempCookies::create(dir.path(), "https://video.example.com/x", "a=1; b=two; bad")
                    .unwrap();
            path = c.path().to_path_buf();
            let body = std::fs::read_to_string(&path).unwrap();
            assert!(body.contains(".video.example.com\tTRUE\t/\tTRUE\t0\ta\t1"));
            assert!(body.contains("\tb\ttwo"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        assert!(!path.exists());
    }
}
