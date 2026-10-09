//! The media download task run by the download manager.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use velox_core::speed::SpeedMeter;
use velox_core::{naming, EngineError, MediaRunner, StopReason, TaskEnv, TaskOutcome, TaskShared};
use velox_http::{build_client, ClientOptions};
use velox_types::{
    DownloadKind, DownloadStatus, MediaProbeResult, MediaRequest, MediaSourceKind, OutputContainer,
};

use crate::dash::{self, TrackKind};
use crate::hls::{self, KeyMethod, MediaPlaylist, Playlist};
use crate::mux::{self, MuxInput};
use crate::probe::fetch_text;
use crate::segments::{self, Downloader, KeyRef, Progress, SegmentJob, TrackPlan};
use crate::tools::Tools;
use crate::{ytdlp, MediaError, RequestInfo};

/// Runs HLS, DASH and extractor downloads for the manager.
#[derive(Clone)]
pub struct MediaEngine {
    tools: Tools,
    secret_dir: Option<PathBuf>,
}

impl MediaEngine {
    pub fn new(tools: Tools) -> Self {
        Self {
            tools,
            secret_dir: None,
        }
    }

    /// Keep files that carry secrets (yt-dlp cookie files) in `dir`, a
    /// per-user application directory, instead of next to the download.
    /// Files left behind by a crash are removed now.
    pub fn with_secret_dir(mut self, dir: PathBuf) -> Self {
        ytdlp::remove_cookie_files(&dir);
        self.secret_dir = Some(dir);
        self
    }

    pub fn tools(&self) -> &Tools {
        &self.tools
    }

    /// Probe a media resource (used by the browser bridge).
    pub async fn probe(
        &self,
        kind: MediaSourceKind,
        info: &RequestInfo,
        opts: &ClientOptions,
        work_dir: &Path,
    ) -> Result<MediaProbeResult, MediaError> {
        crate::probe::probe(kind, info, opts, &self.tools, work_dir).await
    }
}

impl MediaRunner for MediaEngine {
    fn run(
        &self,
        shared: Arc<TaskShared>,
        env: TaskEnv,
    ) -> Pin<Box<dyn Future<Output = TaskOutcome> + Send>> {
        let tools = self.tools.clone();
        let secret_dir = self.secret_dir.clone();
        Box::pin(run(tools, secret_dir, shared, env))
    }
}

fn set_status(sh: &TaskShared, env: &TaskEnv, status: DownloadStatus, stage: Option<String>) {
    {
        let mut r = sh.record.lock();
        if r.status != status {
            r.status = status;
            if status == DownloadStatus::Downloading {
                r.error = None;
                r.error_kind = None;
            }
            let _ = env.db.save_download(&r);
            env.hooks.record_changed(&r);
        }
    }
    let mut l = sh.live.lock();
    l.status = Some(status);
    l.stage = stage;
}

fn log(sh: &TaskShared, env: &TaskEnv, level: &str, msg: &str) {
    env.hooks.log(&sh.record.lock(), level, msg);
}

async fn run(
    tools: Tools,
    secret_dir: Option<PathBuf>,
    sh: Arc<TaskShared>,
    env: TaskEnv,
) -> TaskOutcome {
    let max_retries = env.settings.downloads.max_retries;
    let base_delay = env.settings.downloads.retry_delay_secs.max(1) as u64;
    let mut failures = 0u32;
    loop {
        if sh.cancel.is_cancelled() {
            return TaskOutcome::Stopped(sh.stop_reason());
        }
        let before = sh.live.lock().downloaded;
        match attempt(&tools, secret_dir.as_deref(), &sh, &env).await {
            Ok(()) => return TaskOutcome::Completed,
            Err(_) if sh.cancel.is_cancelled() => return TaskOutcome::Stopped(sh.stop_reason()),
            Err(MediaError::Cancelled) => return TaskOutcome::Stopped(StopReason::Pause),
            Err(e) => {
                let expired = segments::is_expired(&e);
                let ee: EngineError = if expired {
                    EngineError::LinkExpired(e.to_string())
                } else {
                    e.into()
                };
                if sh.live.lock().downloaded > before + 256 * 1024 {
                    failures = 0;
                }
                if ee.is_transient() && failures < max_retries {
                    failures += 1;
                    let delay = Duration::from_secs(
                        base_delay
                            .saturating_mul(1 << (failures - 1).min(5))
                            .min(120),
                    );
                    {
                        let mut r = sh.record.lock();
                        r.status = DownloadStatus::Retrying;
                        r.retry_count = failures;
                        r.error = Some(ee.to_string());
                        r.error_kind = Some(ee.kind());
                        let _ = env.db.save_download(&r);
                        env.hooks.log(
                            &r,
                            "warn",
                            &format!(
                                "{ee}; retry {failures}/{max_retries} in {}s",
                                delay.as_secs()
                            ),
                        );
                        env.hooks.record_changed(&r);
                    }
                    sh.live.lock().status = Some(DownloadStatus::Retrying);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => continue,
                        _ = sh.cancel.cancelled() => return TaskOutcome::Stopped(sh.stop_reason()),
                    }
                }
                return TaskOutcome::Failed(ee);
            }
        }
    }
}

fn request_info(sh: &TaskShared, env: &TaskEnv, url: &str) -> RequestInfo {
    let r = sh.record.lock();
    let mut headers = r.headers.clone();
    headers.extend(env.secrets.headers.iter().cloned());
    RequestInfo {
        url: url.to_string(),
        referer: r.referer.clone().or_else(|| r.page_url.clone()),
        cookies: env.secrets.cookies.clone(),
        user_agent: r
            .user_agent
            .clone()
            .or_else(|| Some(env.client_opts.user_agent.clone())),
        headers,
        credentials: env.secrets.credentials.clone(),
    }
}

/// Live progress reporting while segments download.
fn spawn_ticker(
    sh: Arc<TaskShared>,
    progress: Arc<Progress>,
    label: String,
    stop: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut meter = SpeedMeter::new(Duration::from_secs(3));
        let started = Instant::now();
        let base_elapsed = sh.record.lock().elapsed_ms;
        let mut last_hist = Instant::now();
        loop {
            let bytes = progress.bytes.load(Ordering::Relaxed);
            let done = progress.done.load(Ordering::Relaxed);
            let total = progress.total.load(Ordering::Relaxed);
            let now = Instant::now();
            meter.record(now, bytes);
            {
                let mut l = sh.live.lock();
                l.downloaded = bytes;
                l.total = if done > 0 && total > 0 {
                    Some(bytes / done * total)
                } else {
                    None
                };
                l.speed = meter.speed();
                let secs = started.elapsed().as_secs_f64();
                l.avg_speed = if secs > 1.0 {
                    (bytes as f64 / secs) as u64
                } else {
                    l.speed
                };
                l.eta_secs = match (l.total, l.speed) {
                    (Some(t), s) if s > 0 => Some(t.saturating_sub(bytes) / s),
                    _ => None,
                };
                l.elapsed_ms = base_elapsed + started.elapsed().as_millis() as u64;
                l.stage = Some(format!("{label} {done}/{total}"));
                l.active_connections = 1;
                if now.duration_since(last_hist) >= Duration::from_secs(1) {
                    let s = l.speed;
                    l.history.push_back(s);
                    while l.history.len() > 300 {
                        l.history.pop_front();
                    }
                    last_hist = now;
                }
            }
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis(400)) => {}
            }
        }
    })
}

fn work_dir_for(sh: &TaskShared) -> PathBuf {
    let mut r = sh.record.lock();
    if let Some(t) = &r.temp_path {
        return PathBuf::from(t);
    }
    let dir = Path::new(&r.save_dir).join(format!(".velox-{}", r.id.simple()));
    r.temp_path = Some(dir.to_string_lossy().to_string());
    dir
}

fn seg_ext(url: &url::Url, has_init: bool) -> String {
    if has_init {
        return "mp4".into();
    }
    let p = url.path().to_ascii_lowercase();
    if p.ends_with(".aac") {
        "aac".into()
    } else if p.ends_with(".vtt") || p.ends_with(".webvtt") {
        "vtt".into()
    } else if p.ends_with(".mp3") {
        "mp3".into()
    } else {
        "ts".into()
    }
}

fn hls_track(name: &str, mp: &MediaPlaylist) -> Result<TrackPlan, MediaError> {
    if mp.drm {
        return Err(MediaError::Drm);
    }
    if !mp.ended {
        return Err(MediaError::Live);
    }
    let first = mp
        .segments
        .first()
        .ok_or_else(|| MediaError::Parse("empty media playlist".into()))?;
    let init = first.map.as_ref().map(|(u, r)| {
        (
            u.clone(),
            r.as_ref().map(|r| (r.offset, r.offset + r.length - 1)),
        )
    });
    let ext = seg_ext(&first.uri, init.is_some());
    let segments = mp
        .segments
        .iter()
        .map(|s| SegmentJob {
            url: s.uri.clone(),
            range: s
                .byte_range
                .as_ref()
                .map(|r| (r.offset, r.offset + r.length - 1)),
            key: s
                .key
                .as_ref()
                .filter(|k| k.method == KeyMethod::Aes128)
                .and_then(|k| k.uri.clone().map(|uri| KeyRef { uri, iv: k.iv })),
            sequence: s.sequence,
        })
        .collect();
    Ok(TrackPlan {
        name: name.into(),
        ext,
        init,
        segments,
    })
}

async fn fetch_media_playlist(
    client: &velox_http::reqwest::Client,
    info: &RequestInfo,
    url: &str,
) -> Result<MediaPlaylist, MediaError> {
    let (text, base) = fetch_text(client, info, url).await?;
    match hls::parse(&text, &base).map_err(|e| MediaError::Parse(e.to_string()))? {
        Playlist::Media(m) => Ok(m),
        Playlist::Master(_) => Err(MediaError::Parse("expected a media playlist".into())),
    }
}

/// Downloaded video, audio and (subtitle file, language) tracks.
type Downloaded = (Option<PathBuf>, Option<PathBuf>, Vec<(PathBuf, String)>);

struct Plans {
    video: Option<TrackPlan>,
    audio: Option<TrackPlan>,
    subtitles: Vec<(TrackPlan, String)>,
}

async fn hls_plans(
    client: &velox_http::reqwest::Client,
    info: &RequestInfo,
    sel: &MediaRequest,
) -> Result<Plans, MediaError> {
    let (text, base) = fetch_text(client, info, &info.url).await?;
    let pl = hls::parse(&text, &base).map_err(|e| MediaError::Parse(e.to_string()))?;
    let m = match pl {
        Playlist::Media(mp) => {
            return Ok(Plans {
                video: Some(hls_track("video", &mp)?),
                audio: None,
                subtitles: vec![],
            })
        }
        Playlist::Master(m) => m,
    };
    if m.drm {
        return Err(MediaError::Drm);
    }
    let audio_only = sel.container.is_audio_only();
    let idx = |id: &Option<String>, prefix: char| -> Option<usize> {
        id.as_deref()?.strip_prefix(prefix)?.parse().ok()
    };

    // Audio-only selection of a separate rendition.
    let explicit_audio = idx(&sel.audio_format_id, 'a').or_else(|| idx(&sel.format_id, 'a'));
    let variant = match idx(&sel.format_id, 'v') {
        Some(i) => m.variants.get(i).cloned(),
        None => {
            let max_h = sel.max_height.unwrap_or(u32::MAX);
            m.variants
                .iter()
                .filter(|v| v.height.is_none_or(|h| h <= max_h))
                .max_by_key(|v| v.bandwidth.unwrap_or(0))
                .or_else(|| m.variants.iter().min_by_key(|v| v.height.unwrap_or(0)))
                .cloned()
        }
    };
    let audio_rendition = explicit_audio
        .and_then(|i| m.renditions.get(i).cloned())
        .or_else(|| {
            let group = variant.as_ref()?.audio_group.as_ref()?;
            let candidates: Vec<&hls::Rendition> = m
                .renditions
                .iter()
                .filter(|r| r.kind == "AUDIO" && &r.group_id == group && r.uri.is_some())
                .collect();
            candidates
                .iter()
                .find(|r| r.default)
                .or(candidates.first())
                .map(|r| (*r).clone())
        });

    let mut plans = Plans {
        video: None,
        audio: None,
        subtitles: vec![],
    };
    let only_audio_rendition = sel.format_id.as_deref().is_some_and(|f| f.starts_with('a'));
    if let Some(a) = audio_rendition.as_ref().and_then(|r| r.uri.clone()) {
        plans.audio = Some(hls_track(
            "audio",
            &fetch_media_playlist(client, info, a.as_str()).await?,
        )?);
    }
    if !(only_audio_rendition || audio_only && plans.audio.is_some()) {
        let v =
            variant.ok_or_else(|| MediaError::Parse("no variant matches the selection".into()))?;
        plans.video = Some(hls_track(
            "video",
            &fetch_media_playlist(client, info, v.uri.as_str()).await?,
        )?);
    }
    for (n, lang) in sel.subtitle_languages.iter().enumerate() {
        let r = m.renditions.iter().find(|r| {
            r.kind == "SUBTITLES"
                && r.language
                    .as_deref()
                    .is_some_and(|l| l.eq_ignore_ascii_case(lang))
        });
        if let Some(uri) = r.and_then(|r| r.uri.clone()) {
            let mp = fetch_media_playlist(client, info, uri.as_str()).await?;
            let mut t = hls_track(&format!("sub{n}"), &mp)?;
            t.ext = "vtt".into();
            plans.subtitles.push((t, lang.clone()));
        }
    }
    Ok(plans)
}

async fn dash_plans(
    client: &velox_http::reqwest::Client,
    info: &RequestInfo,
    sel: &MediaRequest,
) -> Result<Plans, MediaError> {
    let (text, base) = fetch_text(client, info, &info.url).await?;
    let m = dash::parse(&text, &base).map_err(|e| MediaError::Parse(e.to_string()))?;
    if m.drm {
        return Err(MediaError::Drm);
    }
    if m.is_live {
        return Err(MediaError::Live);
    }
    let by_id = |id: &Option<String>| -> Option<&dash::Representation> {
        let id = id.as_deref()?.strip_prefix("r:")?;
        m.representations.iter().find(|r| r.id == id)
    };
    let max_h = sel.max_height.unwrap_or(u32::MAX);
    let chosen = by_id(&sel.format_id);
    let video = match chosen {
        Some(r) if r.kind == TrackKind::Video => Some(r),
        Some(_) => None,
        None => m
            .representations
            .iter()
            .filter(|r| r.kind == TrackKind::Video && r.height.is_none_or(|h| h <= max_h))
            .max_by_key(|r| r.bandwidth.unwrap_or(0)),
    };
    let audio = by_id(&sel.audio_format_id)
        .or(chosen.filter(|r| r.kind == TrackKind::Audio))
        .or_else(|| {
            m.representations
                .iter()
                .filter(|r| r.kind == TrackKind::Audio)
                .max_by_key(|r| r.bandwidth.unwrap_or(0))
        });
    let to_plan = |name: &str, r: &dash::Representation| TrackPlan {
        name: name.into(),
        ext: if r.mime.as_deref().is_some_and(|m| m.contains("webm")) {
            "webm".into()
        } else {
            "mp4".into()
        },
        init: r.init.as_ref().map(|i| (i.url.clone(), i.range)),
        segments: r
            .segments
            .iter()
            .enumerate()
            .map(|(i, s)| SegmentJob {
                url: s.url.clone(),
                range: s.range,
                key: None,
                sequence: i as u64,
            })
            .collect(),
    };
    let audio_only = sel.container.is_audio_only();
    Ok(Plans {
        video: if audio_only && audio.is_some() {
            None
        } else {
            video.map(|r| to_plan("video", r))
        },
        audio: audio.map(|r| to_plan("audio", r)),
        subtitles: vec![],
    })
}

/// Pick the final path and move the produced file there.
fn place_output(
    sh: &TaskShared,
    env: &TaskEnv,
    produced: &Path,
    ext: &str,
) -> Result<PathBuf, MediaError> {
    let (dir, name) = {
        let r = sh.record.lock();
        (PathBuf::from(&r.save_dir), r.file_name.clone())
    };
    let stem = match naming::split_ext(&name) {
        (s, Some(e))
            if e.eq_ignore_ascii_case(ext)
                || ["mp4", "mkv", "m4a", "mp3", "webm", "ts"]
                    .contains(&e.to_ascii_lowercase().as_str()) =>
        {
            s.to_string()
        }
        _ => name.clone(),
    };
    let wanted = naming::sanitize_file_name(&format!("{stem}.{ext}"));
    let mut reserved = env.reserved.lock().clone();
    reserved.remove(&dir.join(&name));
    let target = naming::unique_path(&dir, &wanted, &reserved);
    velox_core::fsutil::move_file(produced, &target).map_err(|e| MediaError::Io(e.to_string()))?;
    {
        let mut r = sh.record.lock();
        let mut res = env.reserved.lock();
        res.remove(&dir.join(&r.file_name));
        res.insert(target.clone());
        r.file_name = target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(wanted);
        let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        r.total_size = Some(size);
        r.downloaded = size;
    }
    Ok(target)
}

async fn attempt(
    tools: &Tools,
    secret_dir: Option<&Path>,
    sh: &Arc<TaskShared>,
    env: &TaskEnv,
) -> Result<(), MediaError> {
    let (kind, sel) = {
        let r = sh.record.lock();
        let mut sel = r.media.clone().unwrap_or_default();
        if sel.url.is_empty() {
            sel.url = r.url.clone();
        }
        (r.kind, sel)
    };
    let info = request_info(sh, env, &sel.url);
    let work = work_dir_for(sh);
    std::fs::create_dir_all(&work).map_err(|e| MediaError::Io(e.to_string()))?;
    {
        let mut r = sh.record.lock();
        if r.started_at.is_none() {
            r.started_at = Some(velox_types::now_ms());
        }
        let _ = env.db.save_download(&r);
    }
    set_status(
        sh,
        env,
        DownloadStatus::Connecting,
        Some("Reading manifest".into()),
    );

    if kind == DownloadKind::Extractor {
        return extractor(
            tools,
            secret_dir.unwrap_or(&work),
            sh,
            env,
            &info,
            &sel,
            &work,
        )
        .await;
    }

    let client = build_client(&env.client_opts)?;
    let plans = match kind {
        DownloadKind::Hls => hls_plans(&client, &info, &sel).await?,
        DownloadKind::Dash => dash_plans(&client, &info, &sel).await?,
        other => {
            return Err(MediaError::Unsupported(format!(
                "{} is not a media download",
                other.as_str()
            )))
        }
    };
    let tracks: Vec<&TrackPlan> = plans
        .video
        .iter()
        .chain(plans.audio.iter())
        .chain(plans.subtitles.iter().map(|(t, _)| t))
        .collect();
    let total: usize = tracks.iter().map(|t| t.segments.len()).sum();
    log(
        sh,
        env,
        "info",
        &format!("{} tracks, {total} segments", tracks.len()),
    );

    let concurrency = env.settings.media.segment_concurrency as usize;
    let dl = Downloader::new(
        client,
        info,
        concurrency,
        sh.cancel.child_token(),
        vec![env.global_limiter.clone(), sh.limiter.clone()],
    );
    dl.progress.total.store(total as u64, Ordering::Relaxed);
    let stop_ticker = CancellationToken::new();
    set_status(
        sh,
        env,
        DownloadStatus::Downloading,
        Some("Downloading segments".into()),
    );
    let ticker = spawn_ticker(
        sh.clone(),
        dl.progress.clone(),
        "Segments".into(),
        stop_ticker.clone(),
    );

    let result: Result<Downloaded, MediaError> = async {
        let video = match &plans.video {
            Some(p) => Some(dl.download_track(p, &work).await?),
            None => None,
        };
        let audio = match &plans.audio {
            Some(p) => Some(dl.download_track(p, &work).await?),
            None => None,
        };
        let mut subs = Vec::new();
        for (p, lang) in &plans.subtitles {
            let path = dl.download_track(p, &work).await?;
            let text = std::fs::read_to_string(&path).map_err(|e| MediaError::Io(e.to_string()))?;
            std::fs::write(&path, segments::merge_webvtt(&text))
                .map_err(|e| MediaError::Io(e.to_string()))?;
            subs.push((path, lang.clone()));
        }
        Ok((video, audio, subs))
    }
    .await;
    stop_ticker.cancel();
    let _ = ticker.await;
    {
        let mut r = sh.record.lock();
        r.downloaded = dl.progress.bytes.load(Ordering::Relaxed);
        r.elapsed_ms = sh.live.lock().elapsed_ms;
        let _ = env.db.save_download(&r);
    }
    let _ = env
        .db
        .add_transferred(dl.progress.bytes.load(Ordering::Relaxed));
    let (video, audio, subs) = result?;

    // Mux.
    set_status(
        sh,
        env,
        DownloadStatus::Processing,
        Some("Merging with FFmpeg".into()),
    );
    let single = video.as_ref().xor(audio.as_ref()).cloned();
    let source_ext = single
        .as_ref()
        .and_then(|p| p.extension())
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "mkv".into());
    let copy_as_is = single
        .as_ref()
        .filter(|_| sel.container == OutputContainer::Original && subs.is_empty());
    let target = if let Some(only) = copy_as_is {
        place_output(sh, env, only, &source_ext)?
    } else {
        let ffmpeg = tools
            .ffmpeg
            .as_ref()
            .ok_or_else(|| MediaError::Tool("FFmpeg is not available in this build".into()))?;
        let ext = mux::output_ext(
            sel.container,
            if single.is_some() { &source_ext } else { "mkv" },
        );
        let out = work.join(format!("output.{ext}"));
        let embed = sel.embed_subtitles;
        let input = MuxInput {
            video: video.clone(),
            audio: audio.clone(),
            subtitles: if embed { subs.clone() } else { vec![] },
        };
        mux::mux(ffmpeg, &input, sel.container, &out, &sh.cancel).await?;
        place_output(sh, env, &out, &ext)?
    };
    // Subtitles not embedded are saved next to the video.
    if !sel.embed_subtitles || sel.container.is_audio_only() {
        for (path, lang) in &subs {
            let stem = target
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let dest =
                target.with_file_name(naming::sanitize_file_name(&format!("{stem}.{lang}.vtt")));
            let _ = velox_core::fsutil::move_file(path, &dest);
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    {
        let mut r = sh.record.lock();
        r.temp_path = None;
        let _ = env.db.save_download(&r);
    }
    log(sh, env, "info", &format!("saved {}", target.display()));
    Ok(())
}

async fn extractor(
    tools: &Tools,
    cookie_dir: &Path,
    sh: &Arc<TaskShared>,
    env: &TaskEnv,
    info: &RequestInfo,
    sel: &MediaRequest,
    work: &Path,
) -> Result<(), MediaError> {
    let y = tools.ytdlp.as_ref().ok_or_else(|| {
        MediaError::Unsupported("the yt-dlp extractor is not available in this build".into())
    })?;
    let (dir, stem) = {
        let r = sh.record.lock();
        let stem = naming::split_ext(&r.file_name).0.to_string();
        (PathBuf::from(&r.save_dir), stem)
    };
    set_status(
        sh,
        env,
        DownloadStatus::Downloading,
        Some("Extracting".into()),
    );
    let started = Instant::now();
    let base_elapsed = sh.record.lock().elapsed_ms;
    let mut done_parts: u64 = 0;
    let mut current_part = String::new();
    let mut last_bytes = 0u64;
    let mut last_hist = Instant::now();
    let path = ytdlp::download(
        y,
        tools.ffmpeg.as_deref(),
        info,
        sel,
        &dir,
        &stem,
        work,
        cookie_dir,
        &sh.cancel,
        |p| {
            if p.part != current_part {
                done_parts += last_bytes;
                current_part = p.part.clone();
            }
            last_bytes = p.downloaded;
            let mut l = sh.live.lock();
            l.status = Some(DownloadStatus::Downloading);
            l.downloaded = done_parts + p.downloaded;
            l.total = p.total.map(|t| done_parts + t);
            l.speed = p.speed.unwrap_or(0.0) as u64;
            l.eta_secs = p.eta;
            l.active_connections = 1;
            l.elapsed_ms = base_elapsed + started.elapsed().as_millis() as u64;
            let secs = started.elapsed().as_secs_f64();
            l.avg_speed = if secs > 1.0 {
                (l.downloaded as f64 / secs) as u64
            } else {
                l.speed
            };
            l.stage = Some(format!("yt-dlp · format {}", p.part));
            if last_hist.elapsed() >= Duration::from_secs(1) {
                let s = l.speed;
                l.history.push_back(s);
                last_hist = Instant::now();
            }
        },
    )
    .await?;
    let _ = env.db.add_transferred(sh.live.lock().downloaded);
    {
        let mut r = sh.record.lock();
        let mut res = env.reserved.lock();
        res.remove(&dir.join(&r.file_name));
        res.insert(path.clone());
        r.file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| r.file_name.clone());
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        r.total_size = Some(size);
        r.downloaded = size;
        r.temp_path = None;
        r.elapsed_ms = base_elapsed + started.elapsed().as_millis() as u64;
        let _ = env.db.save_download(&r);
    }
    let _ = std::fs::remove_dir_all(work);
    log(sh, env, "info", &format!("saved {}", path.display()));
    Ok(())
}
