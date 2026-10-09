//! Describe the renditions available for a media URL.
//!
//! Format ids are stable references used by the download step:
//! * HLS: `v<n>` = n-th variant, `a<n>` = n-th rendition, `m0` = media playlist
//! * DASH: `r:<representation id>`
//! * direct: `direct`; pages: yt-dlp format ids.

use std::path::Path;

use url::Url;
use velox_http::{build_client, fetch_bytes, ClientOptions, Validators};
use velox_types::{MediaFormat, MediaProbeResult, MediaSourceKind, SubtitleTrack};

use crate::dash::{self, TrackKind};
use crate::hls::{self, Playlist};
use crate::{ytdlp, MediaError, RequestInfo, Tools, MAX_MANIFEST};

pub(crate) async fn fetch_text(
    client: &velox_http::reqwest::Client,
    info: &RequestInfo,
    url: &str,
) -> Result<(String, Url), MediaError> {
    let f = fetch_bytes(client, &info.context_for(url), None, MAX_MANIFEST).await?;
    let base = Url::parse(&f.final_url).map_err(|e| MediaError::Parse(e.to_string()))?;
    Ok((String::from_utf8_lossy(&f.body).into_owned(), base))
}

fn hls_audio_codec(codecs: Option<&str>) -> bool {
    codecs.is_none_or(|c| {
        c.split(',').any(|x| {
            let x = x.trim();
            x.starts_with("mp4a")
                || x.starts_with("ac-3")
                || x.starts_with("ec-3")
                || x.starts_with("opus")
                || x.starts_with("mp3")
        })
    })
}

fn hls_video_codec(codecs: Option<&str>) -> bool {
    codecs.is_none_or(|c| {
        c.split(',').any(|x| {
            let x = x.trim();
            x.starts_with("avc")
                || x.starts_with("hvc")
                || x.starts_with("hev")
                || x.starts_with("av01")
                || x.starts_with("vp")
        })
    })
}

fn video_label(height: Option<u32>, bitrate: Option<u64>, codec: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(h) = height {
        parts.push(format!("{h}p"));
    }
    if let Some(b) = bitrate {
        parts.push(format!("{:.1} Mbps", b as f64 / 1_000_000.0));
    }
    if let Some(c) = codec {
        parts.push(
            c.split(',')
                .next()
                .unwrap_or(c)
                .split('.')
                .next()
                .unwrap_or(c)
                .to_string(),
        );
    }
    parts.join(" · ")
}

async fn probe_hls(
    client: &velox_http::reqwest::Client,
    info: &RequestInfo,
) -> Result<MediaProbeResult, MediaError> {
    let (text, base) = fetch_text(client, info, &info.url).await?;
    let pl = hls::parse(&text, &base).map_err(|e| MediaError::Parse(e.to_string()))?;
    let mut result = MediaProbeResult {
        kind: MediaSourceKind::Hls,
        url: info.url.clone(),
        title: None,
        duration_secs: None,
        thumbnail: None,
        formats: vec![],
        subtitles: vec![],
        extractor: None,
        is_live: false,
        drm_protected: false,
    };
    match pl {
        Playlist::Media(m) => {
            result.drm_protected = m.drm;
            result.is_live = !m.ended;
            result.duration_secs = Some(m.duration());
            result.formats.push(MediaFormat {
                id: "m0".into(),
                label: "Default".into(),
                has_video: true,
                has_audio: true,
                ext: Some(
                    if m.segments.first().is_some_and(|s| s.map.is_some()) {
                        "mp4"
                    } else {
                        "ts"
                    }
                    .into(),
                ),
                width: None,
                height: None,
                fps: None,
                bitrate: None,
                vcodec: None,
                acodec: None,
                filesize: None,
                language: None,
                url: Some(info.url.clone()),
            });
        }
        Playlist::Master(m) => {
            result.drm_protected = m.drm;
            // Fetch the best variant's media playlist for duration / DRM / live.
            let best = m.variants.iter().max_by_key(|v| v.bandwidth.unwrap_or(0));
            if let Some(best) = best {
                if let Ok((t, b)) = fetch_text(client, info, best.uri.as_str()).await {
                    if let Ok(Playlist::Media(mp)) = hls::parse(&t, &b) {
                        result.duration_secs = Some(mp.duration());
                        result.is_live = !mp.ended;
                        result.drm_protected |= mp.drm;
                    }
                }
            }
            let audio_renditions: Vec<(usize, &hls::Rendition)> = m
                .renditions
                .iter()
                .enumerate()
                .filter(|(_, r)| r.kind == "AUDIO" && r.uri.is_some())
                .collect();
            for (i, v) in m.variants.iter().enumerate() {
                let separate_audio = v
                    .audio_group
                    .as_ref()
                    .is_some_and(|g| audio_renditions.iter().any(|(_, r)| &r.group_id == g));
                let has_video = hls_video_codec(v.codecs.as_deref()) || v.height.is_some();
                result.formats.push(MediaFormat {
                    id: format!("v{i}"),
                    label: video_label(v.height, v.bandwidth, v.codecs.as_deref()),
                    has_video,
                    has_audio: !separate_audio && hls_audio_codec(v.codecs.as_deref()),
                    ext: Some("mp4".into()),
                    width: v.width,
                    height: v.height,
                    fps: v.frame_rate,
                    bitrate: v.average_bandwidth.or(v.bandwidth),
                    vcodec: v.codecs.as_ref().and_then(|c| {
                        c.split(',')
                            .find(|x| hls_video_codec(Some(x)))
                            .map(|x| x.trim().to_string())
                    }),
                    acodec: v.codecs.as_ref().and_then(|c| {
                        c.split(',')
                            .find(|x| hls_audio_codec(Some(x)) && !hls_video_codec(Some(x)))
                            .map(|x| x.trim().to_string())
                    }),
                    filesize: match (v.average_bandwidth.or(v.bandwidth), result.duration_secs) {
                        (Some(b), Some(d)) => Some((b as f64 * d / 8.0) as u64),
                        _ => None,
                    },
                    language: None,
                    url: None,
                });
            }
            for (i, r) in audio_renditions {
                result.formats.push(MediaFormat {
                    id: format!("a{i}"),
                    label: r.name.clone().unwrap_or_else(|| "Audio".into()),
                    has_video: false,
                    has_audio: true,
                    ext: Some("m4a".into()),
                    width: None,
                    height: None,
                    fps: None,
                    // Prefer the default rendition when bitrates are unknown.
                    bitrate: Some(if r.default { 2 } else { 1 }),
                    vcodec: None,
                    acodec: None,
                    filesize: None,
                    language: r.language.clone(),
                    url: None,
                });
            }
            for r in m.renditions.iter().filter(|r| r.kind == "SUBTITLES") {
                if let Some(lang) = &r.language {
                    result.subtitles.push(SubtitleTrack {
                        language: lang.clone(),
                        name: r.name.clone(),
                        ext: Some("vtt".into()),
                        automatic: false,
                        url: r.uri.as_ref().map(|u| u.to_string()),
                    });
                }
            }
        }
    }
    Ok(result)
}

async fn probe_dash(
    client: &velox_http::reqwest::Client,
    info: &RequestInfo,
) -> Result<MediaProbeResult, MediaError> {
    let (text, base) = fetch_text(client, info, &info.url).await?;
    let m = dash::parse(&text, &base).map_err(|e| MediaError::Parse(e.to_string()))?;
    let mut formats = Vec::new();
    let mut subtitles = Vec::new();
    for r in &m.representations {
        let size = match (r.bandwidth, m.duration) {
            (Some(b), Some(d)) => Some((b as f64 * d / 8.0) as u64),
            _ => None,
        };
        match r.kind {
            TrackKind::Video | TrackKind::Audio => formats.push(MediaFormat {
                id: format!("r:{}", r.id),
                label: if r.kind == TrackKind::Video {
                    video_label(r.height, r.bandwidth, r.codecs.as_deref())
                } else {
                    format!(
                        "{} {}",
                        r.lang.as_deref().unwrap_or(""),
                        r.codecs.as_deref().unwrap_or("audio")
                    )
                    .trim()
                    .to_string()
                },
                has_video: r.kind == TrackKind::Video,
                has_audio: r.kind == TrackKind::Audio,
                ext: r.mime.as_deref().map(|m| {
                    if m.contains("webm") {
                        "webm".into()
                    } else {
                        "mp4".into()
                    }
                }),
                width: r.width,
                height: r.height,
                fps: r.frame_rate,
                bitrate: r.bandwidth,
                vcodec: (r.kind == TrackKind::Video)
                    .then(|| r.codecs.clone())
                    .flatten(),
                acodec: (r.kind == TrackKind::Audio)
                    .then(|| r.codecs.clone())
                    .flatten(),
                filesize: size,
                language: r.lang.clone(),
                url: None,
            }),
            TrackKind::Text => subtitles.push(SubtitleTrack {
                language: r.lang.clone().unwrap_or_else(|| r.id.clone()),
                name: None,
                ext: r.mime.clone(),
                automatic: false,
                url: r.segments.first().map(|s| s.url.to_string()),
            }),
            TrackKind::Other => {}
        }
    }
    Ok(MediaProbeResult {
        kind: MediaSourceKind::Dash,
        url: info.url.clone(),
        title: None,
        duration_secs: m.duration,
        thumbnail: None,
        formats,
        subtitles,
        extractor: None,
        is_live: m.is_live,
        drm_protected: m.drm,
    })
}

async fn probe_direct(
    client: &velox_http::reqwest::Client,
    info: &RequestInfo,
) -> Result<MediaProbeResult, MediaError> {
    let p = velox_http::probe(
        client,
        &info.context_for(&info.url),
        0,
        &Validators::default(),
    )
    .await?;
    let mime = p.mime.clone().unwrap_or_default().to_ascii_lowercase();
    let ext = velox_http::headers::extension_for_mime(&mime)
        .map(str::to_string)
        .or_else(|| {
            velox_http::headers::filename_from_url(&p.final_url)
                .and_then(|n| n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
        });
    let has_video = mime.starts_with("video/")
        || matches!(
            ext.as_deref(),
            Some("mp4" | "webm" | "mkv" | "mov" | "m4v" | "ogv")
        );
    drop(p.body);
    Ok(MediaProbeResult {
        kind: MediaSourceKind::Direct,
        url: info.url.clone(),
        title: None,
        duration_secs: None,
        thumbnail: None,
        formats: vec![MediaFormat {
            id: "direct".into(),
            label: ext
                .clone()
                .map(|e| e.to_uppercase())
                .unwrap_or_else(|| "File".into()),
            has_video,
            has_audio: true,
            ext,
            width: None,
            height: None,
            fps: None,
            bitrate: None,
            vcodec: None,
            acodec: None,
            filesize: p.total_size,
            language: None,
            url: Some(p.final_url),
        }],
        subtitles: vec![],
        extractor: None,
        is_live: false,
        drm_protected: false,
    })
}

/// Probe a media resource.
pub async fn probe(
    kind: MediaSourceKind,
    info: &RequestInfo,
    opts: &ClientOptions,
    tools: &Tools,
    work_dir: &Path,
) -> Result<MediaProbeResult, MediaError> {
    let client = build_client(opts)?;
    match kind {
        MediaSourceKind::Hls => probe_hls(&client, info).await,
        MediaSourceKind::Dash => probe_dash(&client, info).await,
        MediaSourceKind::Direct => probe_direct(&client, info).await,
        MediaSourceKind::Page => {
            let y = tools.ytdlp.as_ref().ok_or_else(|| {
                MediaError::Unsupported(
                    "the yt-dlp extractor is not available in this build".into(),
                )
            })?;
            ytdlp::probe(y, info, work_dir).await
        }
    }
}
