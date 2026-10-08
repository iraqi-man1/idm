use velox_http::HttpError;
use velox_types::ErrorKind;

/// Errors produced by the download engine.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum EngineError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("disk error: {0}")]
    Io(String),
    #[error("not enough free disk space (need {needed} bytes, {available} available)")]
    DiskFull { needed: u64, available: u64 },
    #[error("the download link appears to have expired ({0}); refresh the address to continue")]
    LinkExpired(String),
    #[error("the remote file changed since the download started; restart the download to get the new version")]
    RemoteChanged,
    #[error("the server does not support resuming this download; restart it to download from the beginning")]
    ResumeUnsupported,
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("database error: {0}")]
    Db(String),
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("{0}")]
    Tool(String),
    #[error("cancelled")]
    Cancelled,
    #[error("the server closed the connection before the segment was complete")]
    PrematureEof,
    #[error("{0}")]
    Other(String),
}

impl EngineError {
    pub fn kind(&self) -> ErrorKind {
        match self {
            EngineError::Http(h) => match h {
                HttpError::Network(_) | HttpError::Timeout => ErrorKind::Network,
                HttpError::Status {
                    status: 401 | 407, ..
                } => ErrorKind::Auth,
                HttpError::Status { .. } => ErrorKind::Http,
                HttpError::RemoteChanged => ErrorKind::RemoteChanged,
                HttpError::RangeIgnored => ErrorKind::ResumeUnsupported,
                HttpError::BadContentRange(_) => ErrorKind::InvalidResponse,
                HttpError::InvalidUrl(_) => ErrorKind::Unsupported,
                HttpError::Tls(_) => ErrorKind::Network,
                HttpError::TooManyRedirects => ErrorKind::Http,
                HttpError::Other(_) => ErrorKind::Other,
            },
            EngineError::Io(_) => ErrorKind::Disk,
            EngineError::DiskFull { .. } => ErrorKind::DiskFull,
            EngineError::LinkExpired(_) => ErrorKind::LinkExpired,
            EngineError::RemoteChanged => ErrorKind::RemoteChanged,
            EngineError::ResumeUnsupported => ErrorKind::ResumeUnsupported,
            EngineError::InvalidUrl(_) | EngineError::Unsupported(_) => ErrorKind::Unsupported,
            EngineError::Db(_) => ErrorKind::Other,
            EngineError::ChecksumMismatch { .. } => ErrorKind::ChecksumMismatch,
            EngineError::Tool(_) => ErrorKind::Tool,
            EngineError::Cancelled => ErrorKind::Other,
            EngineError::PrematureEof => ErrorKind::Network,
            EngineError::Other(_) => ErrorKind::Other,
        }
    }

    /// Worth an automatic retry.
    pub fn is_transient(&self) -> bool {
        match self {
            EngineError::Http(h) => h.is_transient(),
            EngineError::PrematureEof => true,
            _ => false,
        }
    }

    pub fn from_io(e: std::io::Error) -> Self {
        if is_disk_full(&e) {
            EngineError::DiskFull {
                needed: 0,
                available: 0,
            }
        } else {
            EngineError::Io(e.to_string())
        }
    }
}

impl From<velox_persistence::DbError> for EngineError {
    fn from(e: velox_persistence::DbError) -> Self {
        EngineError::Db(e.to_string())
    }
}

/// ENOSPC / ERROR_DISK_FULL / ERROR_HANDLE_DISK_FULL.
pub fn is_disk_full(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::StorageFull {
        return true;
    }
    match e.raw_os_error() {
        #[cfg(unix)]
        Some(code) => code == libc::ENOSPC || code == libc::EDQUOT,
        #[cfg(windows)]
        Some(code) => code == 112 || code == 39,
        #[cfg(not(any(unix, windows)))]
        Some(_) => false,
        None => false,
    }
}

pub type EngineResult<T> = Result<T, EngineError>;
