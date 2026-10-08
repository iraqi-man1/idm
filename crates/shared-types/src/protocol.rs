//! Messages exchanged between the browser extension and the desktop app.
//!
//! Transport: browser <-(native messaging)-> `velox-nmh` <-(local IPC)-> app.
//! Every request carries a numeric `id` that is echoed in the response.
//! Unknown fields are rejected to keep the attack surface small, and
//! [`ExtRequest::validate`] enforces size limits and URL schemes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::download::{DownloadId, HeaderPair};
use crate::media::{MediaProbeResult, MediaRequest, MediaSourceKind};

pub const MAX_URL_LEN: usize = 16 * 1024;
pub const MAX_COOKIE_LEN: usize = 64 * 1024;
pub const MAX_HEADERS: usize = 64;
pub const MAX_HEADER_LEN: usize = 8 * 1024;
pub const MAX_BATCH: usize = 2000;
pub const MAX_TEXT_LEN: usize = 4 * 1024;

/// A download handed over by the browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, rename = "ExtBrowserDownload")]
pub struct BrowserDownload {
    pub url: String,
    #[serde(default)]
    pub referrer: Option<String>,
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub file_size: Option<u64>,
    /// `Cookie` header value for the URL, read with the browser's cookies API.
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub headers: Vec<HeaderPair>,
    /// Ask the user (show the add dialog) instead of starting silently.
    #[serde(default = "default_true")]
    pub interactive: bool,
    #[serde(default)]
    pub origin: CaptureOrigin,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export, rename = "ExtCaptureOrigin")]
pub enum CaptureOrigin {
    /// A download the browser started and the extension intercepted.
    #[default]
    Capture,
    /// "Download with Velox" from the context menu.
    ContextMenu,
    /// "Download all links".
    Batch,
    /// The floating "Download This Video" button or popup media list.
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, rename = "ExtMediaContext")]
pub struct MediaContext {
    pub url: String,
    pub kind: MediaSourceKind,
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub referrer: Option<String>,
    #[serde(default)]
    pub cookies: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export, rename = "ExtMessage")]
pub enum ExtMessage {
    Hello {
        extension_version: String,
        browser: String,
        protocol_version: u32,
    },
    Ping,
    GetConfig,
    AddDownload {
        download: BrowserDownload,
    },
    AddBatch {
        items: Vec<BrowserDownload>,
        #[serde(default)]
        page_url: Option<String>,
    },
    /// List the renditions of an HLS/DASH/direct media URL or a page.
    ProbeMedia {
        media: MediaContext,
    },
    DownloadMedia {
        media: MediaContext,
        selection: MediaRequest,
        #[serde(default = "default_true")]
        interactive: bool,
    },
    /// Bring the main window to the front.
    ShowApp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, rename = "ExtRequest")]
pub struct ExtRequest {
    #[ts(type = "number")]
    pub id: u64,
    #[serde(flatten)]
    pub message: ExtMessage,
}

/// Capture configuration pushed to the extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, rename = "ExtBrowserConfig")]
pub struct BrowserConfig {
    pub capture_downloads: bool,
    pub capture_extensions: Vec<String>,
    #[ts(type = "number")]
    pub min_capture_size: u64,
    pub excluded_sites: Vec<String>,
    pub video_detection: bool,
    pub floating_button: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export, rename = "ExtReply")]
pub enum ExtReply {
    Hello {
        app_version: String,
        protocol_version: u32,
        compatible: bool,
        /// Shown to the user when `compatible` is false.
        message: Option<String>,
        config: BrowserConfig,
    },
    Pong,
    Config {
        config: BrowserConfig,
    },
    Added {
        accepted: bool,
        /// Present when the download was created immediately.
        download_id: Option<DownloadId>,
        /// The user is being asked (add dialog shown).
        pending: bool,
        reason: Option<String>,
    },
    BatchAdded {
        count: u32,
    },
    Media {
        result: MediaProbeResult,
    },
    Ok,
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, rename = "ExtResponse")]
pub struct ExtResponse {
    /// Request id; 0 for unsolicited notifications.
    #[ts(type = "number")]
    pub id: u64,
    #[serde(flatten)]
    pub reply: ExtReply,
}

impl ExtResponse {
    pub fn error(id: u64, code: &str, message: impl Into<String>) -> Self {
        Self {
            id,
            reply: ExtReply::Error {
                code: code.into(),
                message: message.into(),
            },
        }
    }
}

/// Error returned by request validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValidationError {}

fn check_len(field: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.len() > max {
        return Err(ValidationError(format!("{field} exceeds {max} bytes")));
    }
    if value.chars().any(|c| c == '\0') {
        return Err(ValidationError(format!("{field} contains NUL")));
    }
    Ok(())
}

fn check_opt(field: &str, value: &Option<String>, max: usize) -> Result<(), ValidationError> {
    match value {
        Some(v) => check_len(field, v, max),
        None => Ok(()),
    }
}

/// Only network URLs may be handed to the downloader.
pub fn check_download_url(field: &str, url: &str) -> Result<(), ValidationError> {
    check_len(field, url, MAX_URL_LEN)?;
    let lower = url.trim_start().to_ascii_lowercase();
    let allowed = ["http://", "https://", "ftp://", "ftps://", "sftp://"];
    if !allowed.iter().any(|p| lower.starts_with(p)) {
        return Err(ValidationError(format!(
            "{field} must be an http(s), ftp(s) or sftp URL"
        )));
    }
    Ok(())
}

fn check_page_url(field: &str, url: &Option<String>) -> Result<(), ValidationError> {
    if let Some(u) = url {
        check_len(field, u, MAX_URL_LEN)?;
    }
    Ok(())
}

fn check_headers(headers: &[HeaderPair]) -> Result<(), ValidationError> {
    if headers.len() > MAX_HEADERS {
        return Err(ValidationError("too many headers".into()));
    }
    for h in headers {
        check_len("header name", &h.name, 256)?;
        check_len("header value", &h.value, MAX_HEADER_LEN)?;
        if h.name.is_empty()
            || !h
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        {
            return Err(ValidationError(format!("invalid header name {:?}", h.name)));
        }
        if h.value.contains(['\r', '\n']) {
            return Err(ValidationError("header value contains a line break".into()));
        }
    }
    Ok(())
}

impl BrowserDownload {
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_download_url("url", &self.url)?;
        check_page_url("referrer", &self.referrer)?;
        check_page_url("page_url", &self.page_url)?;
        check_opt("file_name", &self.file_name, 1024)?;
        check_opt("mime", &self.mime, 256)?;
        check_opt("cookies", &self.cookies, MAX_COOKIE_LEN)?;
        check_opt("user_agent", &self.user_agent, 1024)?;
        if let Some(c) = &self.cookies {
            if c.contains(['\r', '\n']) {
                return Err(ValidationError("cookies contain a line break".into()));
            }
        }
        check_headers(&self.headers)
    }
}

impl MediaContext {
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_download_url("url", &self.url)?;
        check_page_url("page_url", &self.page_url)?;
        check_page_url("referrer", &self.referrer)?;
        check_opt("cookies", &self.cookies, MAX_COOKIE_LEN)?;
        check_opt("user_agent", &self.user_agent, 1024)?;
        check_opt("title", &self.title, MAX_TEXT_LEN)?;
        if let Some(c) = &self.cookies {
            if c.contains(['\r', '\n']) {
                return Err(ValidationError("cookies contain a line break".into()));
            }
        }
        Ok(())
    }
}

impl ExtRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match &self.message {
            ExtMessage::Hello {
                extension_version,
                browser,
                ..
            } => {
                check_len("extension_version", extension_version, 64)?;
                check_len("browser", browser, 64)
            }
            ExtMessage::Ping | ExtMessage::GetConfig | ExtMessage::ShowApp => Ok(()),
            ExtMessage::AddDownload { download } => download.validate(),
            ExtMessage::AddBatch { items, page_url } => {
                if items.is_empty() || items.len() > MAX_BATCH {
                    return Err(ValidationError(format!(
                        "batch must have 1..={MAX_BATCH} items"
                    )));
                }
                check_page_url("page_url", page_url)?;
                items.iter().try_for_each(BrowserDownload::validate)
            }
            ExtMessage::ProbeMedia { media } => media.validate(),
            ExtMessage::DownloadMedia {
                media, selection, ..
            } => {
                media.validate()?;
                check_len("selection.url", &selection.url, MAX_URL_LEN)?;
                check_opt("format_id", &selection.format_id, 256)?;
                check_opt("audio_format_id", &selection.audio_format_id, 256)?;
                check_opt("title", &selection.title, MAX_TEXT_LEN)?;
                if selection.subtitle_languages.len() > 32 {
                    return Err(ValidationError("too many subtitle languages".into()));
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_add_download() {
        let json = r#"{"id":7,"type":"add_download","download":{"url":"https://example.com/a.zip","cookies":"a=b"}}"#;
        let req: ExtRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, 7);
        req.validate().unwrap();
        match req.message {
            ExtMessage::AddDownload { download } => {
                assert!(download.interactive);
                assert_eq!(download.cookies.as_deref(), Some("a=b"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn rejects_unknown_fields_and_bad_schemes() {
        let json = r#"{"id":1,"type":"add_download","download":{"url":"https://e.com","evil":1}}"#;
        assert!(serde_json::from_str::<ExtRequest>(json).is_err());

        let json = r#"{"id":1,"type":"add_download","download":{"url":"file:///etc/passwd"}}"#;
        let req: ExtRequest = serde_json::from_str(json).unwrap();
        assert!(req.validate().is_err());

        let json = r#"{"id":1,"type":"add_download","download":{"url":"https://e.com/x","headers":[{"name":"X","value":"a\r\nInjected: 1"}]}}"#;
        let req: ExtRequest = serde_json::from_str(json).unwrap();
        assert!(req.validate().is_err());
    }

    /// Every reply variant must survive a round trip through the flattened
    /// envelope (no field may collide with the envelope's `id`).
    #[test]
    fn every_reply_roundtrips() {
        use crate::media::MediaProbeResult;
        let cfg = BrowserConfig {
            capture_downloads: true,
            capture_extensions: vec![],
            min_capture_size: 0,
            excluded_sites: vec![],
            video_detection: true,
            floating_button: true,
        };
        let replies = vec![
            ExtReply::Hello {
                app_version: "1".into(),
                protocol_version: 1,
                compatible: true,
                message: None,
                config: cfg.clone(),
            },
            ExtReply::Pong,
            ExtReply::Config { config: cfg },
            ExtReply::Added {
                accepted: true,
                download_id: Some(uuid::Uuid::nil()),
                pending: false,
                reason: None,
            },
            ExtReply::BatchAdded { count: 3 },
            ExtReply::Media {
                result: MediaProbeResult {
                    kind: MediaSourceKind::Hls,
                    url: "https://e.com/a.m3u8".into(),
                    title: None,
                    duration_secs: None,
                    thumbnail: None,
                    formats: vec![],
                    subtitles: vec![],
                    extractor: None,
                    is_live: false,
                    drm_protected: false,
                },
            },
            ExtReply::Ok,
            ExtReply::Error {
                code: "x".into(),
                message: "y".into(),
            },
        ];
        for reply in replies {
            let r = ExtResponse { id: 42, reply };
            let json = serde_json::to_string(&r).unwrap();
            assert_eq!(
                json.matches("\"id\":").count(),
                1,
                "duplicate id key in {json}"
            );
            let back: ExtResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(back, r);
        }
    }

    #[test]
    fn reply_serialization_shape() {
        let r = ExtResponse {
            id: 3,
            reply: ExtReply::Pong,
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"id":3,"type":"pong"}"#
        );
    }
}
