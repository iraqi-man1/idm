use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::media::MediaRequest;

/// Unique identifier of a download.
pub type DownloadId = Uuid;

/// Lifecycle state of a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DownloadStatus {
    /// Waiting in a queue to be started by the scheduler.
    Queued,
    /// Establishing the first connection / probing the server.
    Connecting,
    /// Transferring data.
    Downloading,
    /// Stopped by the user (or by the app on exit); partial data is kept.
    Paused,
    /// Waiting before an automatic retry after a recoverable error.
    Retrying,
    /// Post-processing (merging media streams, verifying checksums).
    Processing,
    /// Finished successfully.
    Completed,
    /// Stopped because of an unrecoverable error.
    Failed,
    /// Stopped by the user and partial data discarded.
    Cancelled,
}

impl DownloadStatus {
    /// A task is currently running for this download.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Connecting | Self::Downloading | Self::Retrying | Self::Processing
        )
    }

    /// The download will not progress without user action.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Connecting => "connecting",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Retrying => "retrying",
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "connecting" => Self::Connecting,
            "downloading" => Self::Downloading,
            "paused" => Self::Paused,
            "retrying" => Self::Retrying,
            "processing" => Self::Processing,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// Transfer mechanism used for a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DownloadKind {
    /// HTTP or HTTPS file transfer (segmented when the server allows it).
    Http,
    /// FTP / FTPS file transfer.
    Ftp,
    /// SFTP (SSH) file transfer.
    Sftp,
    /// HTTP Live Streaming playlist downloaded natively and remuxed by FFmpeg.
    Hls,
    /// MPEG-DASH manifest downloaded natively and remuxed by FFmpeg.
    Dash,
    /// Media page handled by the bundled yt-dlp extractor.
    Extractor,
}

impl DownloadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Ftp => "ftp",
            Self::Sftp => "sftp",
            Self::Hls => "hls",
            Self::Dash => "dash",
            Self::Extractor => "extractor",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "http" => Self::Http,
            "ftp" => Self::Ftp,
            "sftp" => Self::Sftp,
            "hls" => Self::Hls,
            "dash" => Self::Dash,
            "extractor" => Self::Extractor,
            _ => return None,
        })
    }

    pub fn is_media(self) -> bool {
        matches!(self, Self::Hls | Self::Dash | Self::Extractor)
    }
}

/// File category used for sidebar filters and optional category folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Category {
    Video,
    Music,
    Document,
    Archive,
    Program,
    Image,
    Other,
}

const VIDEO_EXT: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "flv", "mpg", "mpeg", "3gp", "ts", "m2ts",
    "ogv", "vob", "rm", "rmvb", "asf", "f4v",
];
const MUSIC_EXT: &[&str] = &[
    "mp3", "m4a", "aac", "flac", "wav", "ogg", "oga", "opus", "wma", "aiff", "aif", "ape", "mka",
    "mid", "midi", "ra", "amr",
];
const DOCUMENT_EXT: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "csv",
    "epub", "mobi", "djvu", "md", "pps", "ppsx", "xps", "tex", "chm",
];
const ARCHIVE_EXT: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "lz", "lzma", "cab",
    "iso", "img", "arj", "z", "lzh", "ace", "001",
];
const PROGRAM_EXT: &[&str] = &[
    "exe",
    "msi",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "dmg",
    "pkg",
    "deb",
    "rpm",
    "appimage",
    "apk",
    "aab",
    "jar",
    "bat",
    "cmd",
    "sh",
    "run",
    "bin",
    "flatpakref",
    "snap",
    "xpi",
    "crx",
];
const IMAGE_EXT: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "tif", "tiff", "ico", "heic", "heif",
    "avif", "psd", "raw", "cr2", "nef", "jxl",
];

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Video,
        Category::Music,
        Category::Document,
        Category::Archive,
        Category::Program,
        Category::Image,
        Category::Other,
    ];

    /// Classify by file extension (case-insensitive, without the dot).
    pub fn from_extension(ext: &str) -> Category {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        let ext = ext.as_str();
        if VIDEO_EXT.contains(&ext) {
            Category::Video
        } else if MUSIC_EXT.contains(&ext) {
            Category::Music
        } else if DOCUMENT_EXT.contains(&ext) {
            Category::Document
        } else if ARCHIVE_EXT.contains(&ext) {
            Category::Archive
        } else if PROGRAM_EXT.contains(&ext) {
            Category::Program
        } else if IMAGE_EXT.contains(&ext) {
            Category::Image
        } else {
            Category::Other
        }
    }

    /// Classify from a MIME type such as `video/mp4`.
    pub fn from_mime(mime: &str) -> Category {
        let mime = mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime.starts_with("video/")
            || mime == "application/vnd.apple.mpegurl"
            || mime == "application/x-mpegurl"
            || mime == "application/dash+xml"
        {
            Category::Video
        } else if mime.starts_with("audio/") {
            Category::Music
        } else if mime.starts_with("image/") {
            Category::Image
        } else if mime.starts_with("text/")
            || mime == "application/pdf"
            || mime.contains("officedocument")
            || mime.contains("msword")
            || mime.contains("ms-excel")
            || mime.contains("ms-powerpoint")
            || mime.contains("opendocument")
            || mime == "application/epub+zip"
        {
            Category::Document
        } else if mime.contains("zip")
            || mime.contains("rar")
            || mime.contains("7z")
            || mime.contains("tar")
            || mime.contains("gzip")
            || mime.contains("bzip")
            || mime.contains("xz")
            || mime.contains("iso9660")
        {
            Category::Archive
        } else if mime.contains("msdownload")
            || mime.contains("msi")
            || mime.contains("x-executable")
            || mime.contains("apple-diskimage")
            || mime.contains("debian-package")
            || mime.contains("x-rpm")
            || mime.contains("android.package-archive")
        {
            Category::Program
        } else {
            Category::Other
        }
    }

    /// Classify using the file name first, falling back to the MIME type.
    pub fn detect(file_name: &str, mime: Option<&str>) -> Category {
        let by_ext = file_name
            .rsplit_once('.')
            .map(|(_, ext)| Category::from_extension(ext))
            .unwrap_or(Category::Other);
        if by_ext != Category::Other {
            return by_ext;
        }
        mime.map(Category::from_mime).unwrap_or(Category::Other)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Video => "video",
            Category::Music => "music",
            Category::Document => "document",
            Category::Archive => "archive",
            Category::Program => "program",
            Category::Image => "image",
            Category::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Category {
        match s {
            "video" => Category::Video,
            "music" => Category::Music,
            "document" => Category::Document,
            "archive" => Category::Archive,
            "program" => Category::Program,
            "image" => Category::Image,
            _ => Category::Other,
        }
    }

    /// Default sub-folder name used when category folders are enabled.
    pub fn default_folder(self) -> &'static str {
        match self {
            Category::Video => "Video",
            Category::Music => "Music",
            Category::Document => "Documents",
            Category::Archive => "Compressed",
            Category::Program => "Programs",
            Category::Image => "Images",
            Category::Other => "General",
        }
    }
}

/// Classification of a download failure, used by the UI to offer recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorKind {
    /// Connection, DNS or timeout problem. Usually recoverable.
    Network,
    /// The server replied with an unexpected HTTP status.
    Http,
    /// The link appears to have expired (403/404/410 or redirected to a page).
    LinkExpired,
    /// The remote file changed since the download began (ETag/size mismatch).
    RemoteChanged,
    /// The server does not allow resuming; continuing would corrupt the file.
    ResumeUnsupported,
    /// Authentication is required or the credentials were rejected.
    Auth,
    /// A local disk error (permissions, missing directory, I/O failure).
    Disk,
    /// Not enough free disk space.
    DiskFull,
    /// The server sent a response the engine cannot interpret safely.
    InvalidResponse,
    /// A bundled tool (FFmpeg / yt-dlp) failed.
    Tool,
    /// The URL or media type is not supported.
    Unsupported,
    /// The checksum of the finished file did not match.
    ChecksumMismatch,
    Other,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Http => "http",
            Self::LinkExpired => "link_expired",
            Self::RemoteChanged => "remote_changed",
            Self::ResumeUnsupported => "resume_unsupported",
            Self::Auth => "auth",
            Self::Disk => "disk",
            Self::DiskFull => "disk_full",
            Self::InvalidResponse => "invalid_response",
            Self::Tool => "tool",
            Self::Unsupported => "unsupported",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "network" => Self::Network,
            "http" => Self::Http,
            "link_expired" => Self::LinkExpired,
            "remote_changed" => Self::RemoteChanged,
            "resume_unsupported" => Self::ResumeUnsupported,
            "auth" => Self::Auth,
            "disk" => Self::Disk,
            "disk_full" => Self::DiskFull,
            "invalid_response" => Self::InvalidResponse,
            "tool" => Self::Tool,
            "unsupported" => Self::Unsupported,
            "checksum_mismatch" => Self::ChecksumMismatch,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl ChecksumAlgorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().replace('-', "").as_str() {
            "md5" => Self::Md5,
            "sha1" => Self::Sha1,
            "sha256" => Self::Sha256,
            "sha512" => Self::Sha512,
            _ => return None,
        })
    }

    /// Guess the algorithm from the length of a hex digest.
    pub fn from_hex_len(len: usize) -> Option<Self> {
        match len {
            32 => Some(Self::Md5),
            40 => Some(Self::Sha1),
            64 => Some(Self::Sha256),
            128 => Some(Self::Sha512),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChecksumSpec {
    pub algorithm: ChecksumAlgorithm,
    /// Lower-case hex digest.
    pub expected: String,
}

/// A custom HTTP header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HeaderPair {
    pub name: String,
    pub value: String,
}

/// Username / password for HTTP Basic, FTP or SFTP authentication.
/// Never persisted in plaintext; see `velox-persistence::secrets`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// How a newly added download should be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StartMode {
    /// Start immediately, independent of queue limits ("Start Download").
    #[default]
    Now,
    /// Add to a queue; the scheduler starts it when a slot is free.
    Queue,
    /// Add without starting ("Download Later").
    Paused,
}

/// Where an add request came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AddSource {
    #[default]
    User,
    Browser,
    ContextMenu,
    VideoButton,
    Clipboard,
    Batch,
}

/// What to do when the target file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ConflictPolicy {
    /// Pick a free name: `file (1).ext`.
    #[default]
    Rename,
    /// Replace the existing file once the download completes.
    Overwrite,
}

/// Request to add a new download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct AddDownloadRequest {
    pub url: String,
    /// Desired file name; detected from the server when omitted.
    pub file_name: Option<String>,
    /// Target directory; derived from settings and category when omitted.
    pub save_dir: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub headers: Vec<HeaderPair>,
    /// Value of the `Cookie` header supplied by the browser (sensitive).
    pub cookies: Option<String>,
    pub credentials: Option<Credentials>,
    /// Maximum parallel connections for this download (1..=32).
    pub connections: Option<u8>,
    pub queue_id: Option<String>,
    pub start: StartMode,
    /// Milliseconds since epoch at which the download should start.
    #[ts(type = "number | null")]
    pub scheduled_at: Option<i64>,
    pub checksum: Option<ChecksumSpec>,
    pub media: Option<MediaRequest>,
    pub source: AddSource,
    /// Size reported by the browser, if any.
    #[ts(type = "number | null")]
    pub expected_size: Option<u64>,
    pub mime: Option<String>,
    pub priority: Option<i32>,
    /// Per-download speed limit in bytes/second (0 or None = unlimited).
    #[ts(type = "number | null")]
    pub speed_limit: Option<u64>,
    pub conflict: Option<ConflictPolicy>,
    /// Page on which the link was found (for display and referer fallback).
    pub page_url: Option<String>,
}

/// Information about a URL obtained by probing it before adding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UrlInfo {
    pub url: String,
    pub final_url: String,
    pub file_name: String,
    #[ts(type = "number | null")]
    pub total_size: Option<u64>,
    pub resumable: bool,
    pub mime: Option<String>,
    pub category: Category,
    pub kind: DownloadKind,
    /// Directory the file would be saved to with current settings.
    pub suggested_dir: String,
    /// Another download with the same URL already exists.
    pub duplicate_of: Option<DownloadId>,
}

/// Persistent and live information about a download, as shown in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DownloadInfo {
    pub id: DownloadId,
    pub url: String,
    pub final_url: Option<String>,
    pub page_url: Option<String>,
    pub file_name: String,
    pub save_dir: String,
    pub kind: DownloadKind,
    pub status: DownloadStatus,
    pub category: Category,
    #[ts(type = "number | null")]
    pub total_size: Option<u64>,
    #[ts(type = "number")]
    pub downloaded: u64,
    /// `None` until the server has been contacted.
    pub resumable: Option<bool>,
    /// Maximum connections configured for this download.
    pub max_connections: u8,
    /// Connections currently transferring data.
    pub active_connections: u8,
    /// Current speed in bytes/second.
    #[ts(type = "number")]
    pub speed: u64,
    /// Average speed over the active transfer time, bytes/second.
    #[ts(type = "number")]
    pub avg_speed: u64,
    #[ts(type = "number | null")]
    pub eta_secs: Option<u64>,
    pub error: Option<String>,
    pub error_kind: Option<ErrorKind>,
    pub mime: Option<String>,
    pub referer: Option<String>,
    pub queue_id: Option<String>,
    pub priority: i32,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub started_at: Option<i64>,
    #[ts(type = "number | null")]
    pub completed_at: Option<i64>,
    #[ts(type = "number | null")]
    pub scheduled_at: Option<i64>,
    #[ts(type = "number | null")]
    pub next_retry_at: Option<i64>,
    pub retry_count: u32,
    #[ts(type = "number")]
    pub speed_limit: u64,
    pub checksum: Option<ChecksumSpec>,
    /// Result of the last checksum verification: `Some(true)` = matched.
    pub checksum_ok: Option<bool>,
    pub media: Option<MediaRequest>,
    /// Total active transfer time in milliseconds.
    #[ts(type = "number")]
    pub elapsed_ms: u64,
    /// Whether authentication/cookie data is stored for this download.
    pub has_secrets: bool,
}

impl DownloadInfo {
    /// Full path of the final file.
    pub fn file_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.save_dir).join(&self.file_name)
    }

    /// Progress fraction 0.0..=1.0 if the size is known.
    pub fn fraction(&self) -> Option<f64> {
        self.total_size.map(|t| {
            if t == 0 {
                1.0
            } else {
                (self.downloaded as f64 / t as f64).min(1.0)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        assert_eq!(Category::detect("movie.MKV", None), Category::Video);
        assert_eq!(Category::detect("song.mp3", None), Category::Music);
        assert_eq!(Category::detect("setup.exe", None), Category::Program);
        assert_eq!(Category::detect("a.tar.gz", None), Category::Archive);
        assert_eq!(
            Category::detect("report", Some("application/pdf")),
            Category::Document
        );
        assert_eq!(
            Category::detect("stream", Some("video/mp4; codecs=avc1")),
            Category::Video
        );
        assert_eq!(Category::detect("unknown.xyz", None), Category::Other);
    }

    #[test]
    fn status_roundtrip() {
        for s in [
            DownloadStatus::Queued,
            DownloadStatus::Connecting,
            DownloadStatus::Downloading,
            DownloadStatus::Paused,
            DownloadStatus::Retrying,
            DownloadStatus::Processing,
            DownloadStatus::Completed,
            DownloadStatus::Failed,
            DownloadStatus::Cancelled,
        ] {
            assert_eq!(DownloadStatus::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn credentials_debug_is_redacted() {
        let c = Credentials {
            username: "u".into(),
            password: "hunter2".into(),
        };
        assert!(!format!("{c:?}").contains("hunter2"));
    }
}
