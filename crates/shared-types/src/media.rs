use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How a media resource was discovered / is retrieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MediaSourceKind {
    /// A direct file URL (MP4, WebM, MP3...).
    Direct,
    /// HLS master or media playlist (.m3u8).
    Hls,
    /// MPEG-DASH manifest (.mpd).
    Dash,
    /// A web page URL resolved by the bundled yt-dlp extractor.
    Page,
}

/// Container requested for the final media file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OutputContainer {
    #[default]
    Mp4,
    Mkv,
    /// Keep whatever container the source provides.
    Original,
    /// Audio only, AAC in M4A.
    M4a,
    /// Audio only, transcoded to MP3.
    Mp3,
}

impl OutputContainer {
    pub fn extension(self) -> Option<&'static str> {
        match self {
            Self::Mp4 => Some("mp4"),
            Self::Mkv => Some("mkv"),
            Self::M4a => Some("m4a"),
            Self::Mp3 => Some("mp3"),
            Self::Original => None,
        }
    }

    pub fn is_audio_only(self) -> bool {
        matches!(self, Self::M4a | Self::Mp3)
    }
}

/// One selectable rendition of a media resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaFormat {
    /// Opaque identifier (variant URL hash, DASH representation id, yt-dlp format id).
    pub id: String,
    /// Human readable label, e.g. "1080p · 4.8 Mbps · H.264".
    pub label: String,
    pub has_video: bool,
    pub has_audio: bool,
    pub ext: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
    /// Bits per second.
    #[ts(type = "number | null")]
    pub bitrate: Option<u64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    /// Exact or estimated size in bytes.
    #[ts(type = "number | null")]
    pub filesize: Option<u64>,
    pub language: Option<String>,
    /// Direct URL of this rendition when known (not exposed for extractor formats).
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SubtitleTrack {
    pub language: String,
    pub name: Option<String>,
    pub ext: Option<String>,
    /// Automatically generated captions.
    pub automatic: bool,
    pub url: Option<String>,
}

/// Result of probing a media URL or page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MediaProbeResult {
    pub kind: MediaSourceKind,
    pub url: String,
    pub title: Option<String>,
    pub duration_secs: Option<f64>,
    pub thumbnail: Option<String>,
    pub formats: Vec<MediaFormat>,
    pub subtitles: Vec<SubtitleTrack>,
    /// Name of the yt-dlp extractor, for page probes.
    pub extractor: Option<String>,
    /// The stream is live; it cannot be downloaded to completion.
    pub is_live: bool,
    /// DRM / encryption that the app refuses to handle was detected.
    pub drm_protected: bool,
}

/// A media download selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct MediaRequest {
    pub kind: MediaSourceKind,
    /// Manifest URL, direct URL or page URL depending on `kind`.
    pub url: String,
    /// Selected video (or muxed) format id; `None` = best.
    pub format_id: Option<String>,
    /// Separate audio format id (DASH / extractor), `None` = best.
    pub audio_format_id: Option<String>,
    /// Subtitle languages to download.
    pub subtitle_languages: Vec<String>,
    /// Embed subtitles into the container instead of separate files.
    pub embed_subtitles: bool,
    pub container: OutputContainer,
    pub title: Option<String>,
    /// Preferred maximum height used when `format_id` is `None`.
    pub max_height: Option<u32>,
}

impl Default for MediaRequest {
    fn default() -> Self {
        Self {
            kind: MediaSourceKind::Page,
            url: String::new(),
            format_id: None,
            audio_format_id: None,
            subtitle_languages: Vec::new(),
            embed_subtitles: false,
            container: OutputContainer::Mp4,
            title: None,
            max_height: None,
        }
    }
}

/// Availability of a bundled tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolStatus {
    pub name: String,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub error: Option<String>,
    /// Tool comes from the application bundle (as opposed to a dev fallback).
    pub bundled: bool,
}
