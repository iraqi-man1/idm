//! Media downloads for Velox Download Manager.
//!
//! * [`hls`] / [`dash`] – native manifest parsers with DRM detection.
//! * [`probe`] – list renditions of a manifest, direct file or page.
//! * [`segments`] – parallel, resumable segment downloader (AES-128 HLS).
//! * [`mux`] – FFmpeg remuxing / audio extraction.
//! * [`ytdlp`] – the bundled yt-dlp extractor for media pages.
//! * [`task`] – [`MediaEngine`], the `MediaRunner` used by the manager.
//!
//! DRM-protected media (Widevine, PlayReady, FairPlay, SAMPLE-AES) is never
//! downloaded; probes report `drm_protected` and downloads fail with a clear
//! message.

pub mod dash;
pub mod hls;
pub mod mux;
pub mod probe;
pub mod segments;
pub mod task;
pub mod tools;
pub mod ytdlp;

pub use task::MediaEngine;
pub use tools::Tools;

use velox_http::RequestContext;
use velox_types::HeaderPair;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum MediaError {
    #[error(transparent)]
    Http(#[from] velox_http::HttpError),
    #[error("{0}")]
    Parse(String),
    #[error("this media is protected by DRM and cannot be downloaded")]
    Drm,
    #[error("live streams cannot be downloaded")]
    Live,
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Tool(String),
    #[error("disk error: {0}")]
    Io(String),
    #[error("cancelled")]
    Cancelled,
}

impl From<MediaError> for velox_core::EngineError {
    fn from(e: MediaError) -> Self {
        use velox_core::EngineError as E;
        match e {
            MediaError::Http(h) => E::Http(h),
            MediaError::Parse(m) => E::Other(m),
            MediaError::Drm => {
                E::Unsupported("this media is protected by DRM and cannot be downloaded".into())
            }
            MediaError::Live => E::Unsupported("live streams cannot be downloaded".into()),
            MediaError::Unsupported(m) => E::Unsupported(m),
            MediaError::Tool(m) => E::Tool(m),
            MediaError::Io(m) => E::Io(m),
            MediaError::Cancelled => E::Cancelled,
        }
    }
}

/// What is needed to fetch the resources of a media download.
#[derive(Clone, Default)]
pub struct RequestInfo {
    pub url: String,
    pub referer: Option<String>,
    pub cookies: Option<String>,
    pub user_agent: Option<String>,
    pub headers: Vec<HeaderPair>,
}

impl std::fmt::Debug for RequestInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestInfo")
            .field("url", &self.url)
            .field("referer", &self.referer)
            .field("cookies", &self.cookies.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl RequestInfo {
    pub fn context_for(&self, url: &str) -> RequestContext {
        RequestContext {
            url: url.to_string(),
            referer: self.referer.clone(),
            user_agent: self.user_agent.clone(),
            headers: self.headers.clone(),
            cookies: self.cookies.clone(),
            credentials: None,
        }
    }
}

/// Playlists/manifests larger than this are rejected.
pub const MAX_MANIFEST: usize = 16 * 1024 * 1024;
