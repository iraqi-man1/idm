//! Media downloads: probing renditions and adding HLS/DASH/page downloads,
//! for the browser extension (floating video button) and the Add Download
//! dialog. Backed by the media engine, which also runs the downloads for
//! the manager.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager, State};
use velox_media::{MediaEngine, MediaError, RequestInfo, Tools};
use velox_types::protocol::{check_download_url, ExtMessage, ExtReply, MediaContext};
use velox_types::{
    AddDownloadRequest, AddSource, AppSettings, MediaProbeResult, MediaRequest, MediaSourceKind,
    StartMode, ToolStatus,
};

use crate::error::{CmdResult, CommandError};
use crate::security::redact_url;
use crate::AppState;

/// Probing a page with yt-dlp can be slow; manifests are fast.
const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

/// Managed state: the media engine and a scratch directory for probes.
pub struct Media {
    pub engine: Arc<MediaEngine>,
    work_dir: PathBuf,
}

impl Media {
    /// Locate the bundled tools (next to the executable).
    pub fn new(data_dir: &Path) -> Self {
        let tools = Tools::discover(&Tools::exe_dir());
        tracing::info!(
            ffmpeg = ?tools.ffmpeg,
            ffprobe = ?tools.ffprobe,
            ytdlp = ?tools.ytdlp,
            bundled = ?tools.bundled,
            "media tools"
        );
        let work_dir = data_dir.join("media-probe");
        // Cookie files for yt-dlp stay in the per-user data directory, and
        // leftovers of an interrupted run are deleted now.
        velox_media::ytdlp::remove_cookie_files(&work_dir);
        let engine = MediaEngine::new(tools).with_secret_dir(data_dir.join("private"));
        Self {
            engine: Arc::new(engine),
            work_dir,
        }
    }
}

fn media_error(e: MediaError) -> CommandError {
    let code = match &e {
        MediaError::Drm => "drm",
        MediaError::Live => "live",
        _ => "",
    };
    let engine: velox_core::EngineError = e.into();
    let code = if code.is_empty() {
        engine.kind().as_str()
    } else {
        code
    };
    CommandError::new(code, engine.to_string())
}

/// Describe the renditions of a media URL or page.
pub async fn probe(app: &AppHandle, ctx: &MediaContext) -> CmdResult<MediaProbeResult> {
    ctx.validate()
        .map_err(|e| CommandError::new("invalid", e.to_string()))?;
    let media = app.state::<Media>();
    let opts = app.state::<AppState>().manager.client_options();
    let info = RequestInfo {
        url: ctx.url.clone(),
        referer: ctx.referrer.clone().or_else(|| ctx.page_url.clone()),
        cookies: ctx.cookies.clone().filter(|c| !c.is_empty()),
        user_agent: ctx.user_agent.clone().filter(|u| !u.is_empty()),
        headers: Vec::new(),
        credentials: None,
    };
    tracing::info!(url = %redact_url(&ctx.url), kind = ?ctx.kind, "probing media");
    let mut result = tokio::time::timeout(
        PROBE_TIMEOUT,
        media.engine.probe(ctx.kind, &info, &opts, &media.work_dir),
    )
    .await
    .map_err(|_| CommandError::new("timeout", "reading the media formats took too long"))?
    .map_err(media_error)?;
    if result.title.as_deref().is_none_or(str::is_empty) {
        result.title = ctx.title.clone().filter(|t| !t.trim().is_empty());
    }
    Ok(result)
}

/// File name for a direct media file: the page title plus the extension
/// of the URL, when it is a media extension.
fn direct_file_name(title: Option<&str>, url: &str) -> Option<String> {
    let title = title.map(str::trim).filter(|t| !t.is_empty())?;
    let path = url::Url::parse(url).ok()?.path().to_ascii_lowercase();
    let ext = path.rsplit_once('.')?.1;
    const MEDIA: &[&str] = &[
        "mp4", "m4v", "webm", "mkv", "mov", "ogv", "m4a", "mp3", "ogg", "opus", "wav", "flac",
        "aac",
    ];
    MEDIA.contains(&ext).then(|| format!("{title}.{ext}"))
}

/// Build the download for a selection made in the browser.
pub fn browser_request(
    ctx: &MediaContext,
    mut sel: MediaRequest,
    settings: &AppSettings,
) -> Result<AddDownloadRequest, String> {
    if sel.url.trim().is_empty() {
        sel.url = ctx.url.clone();
    }
    check_download_url("selection.url", &sel.url).map_err(|e| e.to_string())?;
    if sel.title.as_deref().is_none_or(|t| t.trim().is_empty()) {
        sel.title = ctx.title.clone();
    }
    // Subtitles follow the media settings (the in-page menu has no choice).
    if sel.subtitle_languages.is_empty() && settings.media.download_subtitles {
        sel.subtitle_languages = settings.media.subtitle_languages.clone();
        sel.embed_subtitles = settings.media.embed_subtitles;
    }
    let direct = sel.kind == MediaSourceKind::Direct;
    Ok(AddDownloadRequest {
        url: sel.url.clone(),
        file_name: if direct {
            direct_file_name(sel.title.as_deref(), &sel.url)
        } else {
            None
        },
        referer: ctx.referrer.clone().or_else(|| ctx.page_url.clone()),
        user_agent: ctx.user_agent.clone().filter(|u| !u.is_empty()),
        cookies: ctx.cookies.clone().filter(|c| !c.is_empty()),
        start: StartMode::Now,
        source: AddSource::VideoButton,
        page_url: ctx.page_url.clone(),
        media: (!direct).then_some(sel),
        ..Default::default()
    })
}

/// Requests from the browser extension.
pub async fn handle(app: &AppHandle, msg: ExtMessage) -> ExtReply {
    let error = |e: CommandError| ExtReply::Error {
        code: e.code,
        message: e.message,
    };
    match msg {
        ExtMessage::ProbeMedia { media } => match probe(app, &media).await {
            Ok(result) => ExtReply::Media { result },
            Err(e) => error(e),
        },
        ExtMessage::DownloadMedia {
            media, selection, ..
        } => {
            let state = app.state::<AppState>();
            let settings = state.manager.settings();
            let req = match browser_request(&media, selection, &settings) {
                Ok(r) => r,
                Err(e) => return error(CommandError::new("invalid", e)),
            };
            tracing::info!(url = %redact_url(&req.url), "media download from browser");
            match state.manager.add(req).await {
                Ok(info) => {
                    if settings.general.show_progress_window {
                        let _ = crate::commands::system::open_progress_window(
                            app.clone(),
                            app.state::<AppState>(),
                            info.id.to_string(),
                        );
                    }
                    ExtReply::Added {
                        accepted: true,
                        download_id: Some(info.id),
                        pending: false,
                        reason: None,
                    }
                }
                Err(e) => error(e.into()),
            }
        }
        _ => ExtReply::Error {
            code: "protocol".into(),
            message: "not a media request".into(),
        },
    }
}

// ----- commands ---------------------------------------------------------------

/// Availability and versions of FFmpeg, ffprobe and yt-dlp.
#[tauri::command]
pub async fn get_media_tools(media: State<'_, Media>) -> CmdResult<Vec<ToolStatus>> {
    Ok(media.engine.tools().status().await)
}

/// Probe a media URL entered in the Add Download dialog.
#[tauri::command]
pub async fn probe_media(
    app: AppHandle,
    url: String,
    kind: MediaSourceKind,
    referer: Option<String>,
    cookies: Option<String>,
) -> CmdResult<MediaProbeResult> {
    let ctx = MediaContext {
        url,
        kind,
        page_url: None,
        referrer: referer.filter(|r| !r.trim().is_empty()),
        cookies: cookies.filter(|c| !c.trim().is_empty()),
        user_agent: None,
        title: None,
    };
    probe(&app, &ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(kind: MediaSourceKind) -> MediaContext {
        MediaContext {
            url: "https://cdn.example.com/v/master.m3u8".into(),
            kind,
            page_url: Some("https://example.com/watch".into()),
            referrer: None,
            cookies: Some("sid=1".into()),
            user_agent: Some("UA".into()),
            title: Some("A video".into()),
        }
    }

    #[test]
    fn hls_selection_becomes_a_media_download() {
        let mut s = AppSettings::default();
        s.media.download_subtitles = true;
        s.media.subtitle_languages = vec!["ar".into()];
        let sel = MediaRequest {
            kind: MediaSourceKind::Hls,
            format_id: Some("v1".into()),
            ..Default::default()
        };
        let r = browser_request(&ctx(MediaSourceKind::Hls), sel, &s).unwrap();
        assert_eq!(r.url, "https://cdn.example.com/v/master.m3u8");
        assert_eq!(r.referer.as_deref(), Some("https://example.com/watch"));
        assert_eq!(r.cookies.as_deref(), Some("sid=1"));
        assert_eq!(r.source, AddSource::VideoButton);
        let m = r.media.unwrap();
        assert_eq!(m.title.as_deref(), Some("A video"));
        assert_eq!(m.subtitle_languages, ["ar"]);
        assert_eq!(m.format_id.as_deref(), Some("v1"));
    }

    #[test]
    fn direct_selection_is_a_plain_download_named_after_the_title() {
        let sel = MediaRequest {
            kind: MediaSourceKind::Direct,
            url: "https://cdn.example.com/f/clip.MP4?token=1".into(),
            title: Some("My clip".into()),
            ..Default::default()
        };
        let r =
            browser_request(&ctx(MediaSourceKind::Direct), sel, &AppSettings::default()).unwrap();
        assert!(r.media.is_none());
        assert_eq!(r.file_name.as_deref(), Some("My clip.mp4"));
        assert_eq!(direct_file_name(Some("x"), "https://e.com/play?id=3"), None);
    }

    #[test]
    fn unsafe_selection_urls_are_rejected() {
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,x",
        ] {
            let sel = MediaRequest {
                kind: MediaSourceKind::Hls,
                url: url.into(),
                ..Default::default()
            };
            assert!(
                browser_request(&ctx(MediaSourceKind::Hls), sel, &AppSettings::default()).is_err()
            );
        }
    }
}
