//! FTP, FTPS and SFTP sources for Velox Download Manager.
//!
//! A [`Remote`] opens a remote file at any offset, optionally for a limited
//! length, which is all the segmented download task needs: every
//! connection of a download is one remote session reading one byte range.
//!
//! * `ftp://` – plain FTP (passive mode, binary, `REST` for offsets).
//! * `ftps://` – FTP over implicit TLS (port 990 by default).
//! * `ftpes://` – FTP with explicit TLS (`AUTH TLS`, port 21 by default).
//! * `sftp://` – SFTP over SSH (password or `~/.ssh` key authentication).
//!
//! TLS certificates are verified against the operating system's trust
//! store. SSH host keys are checked against `~/.ssh/known_hosts` and the
//! application's own known-hosts file ([`hostkeys`]); a changed key is
//! always refused.

pub mod ftp;
pub mod hostkeys;
pub mod sftp;

use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures::Stream;
use percent_encoding::percent_decode_str;
use velox_types::Credentials;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum FtpError {
    #[error("network error: {0}")]
    Network(String),
    #[error("connection timed out")]
    Timeout,
    /// The server refused another connection (e.g. FTP 421, too many users).
    #[error("the server refused the connection: {0}")]
    Refused(String),
    #[error("login failed: {0}")]
    Auth(String),
    #[error("the file was not found on the server: {0}")]
    NotFound(String),
    #[error("the server does not allow resuming at an offset: {0}")]
    NoResume(String),
    #[error("the remote file changed since the download started")]
    RemoteChanged,
    #[error("secure connection failed: {0}")]
    Tls(String),
    #[error("{0}")]
    HostKey(String),
    #[error("invalid address: {0}")]
    InvalidUrl(String),
    #[error("{0}")]
    Protocol(String),
}

impl FtpError {
    /// Worth retrying after a delay.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            FtpError::Network(_) | FtpError::Timeout | FtpError::Refused(_)
        )
    }

    /// The server limits concurrent connections.
    pub fn is_connection_limit(&self) -> bool {
        matches!(self, FtpError::Refused(_))
    }

    pub fn is_network(&self) -> bool {
        matches!(self, FtpError::Network(_) | FtpError::Timeout)
    }
}

pub type FtpResult<T> = Result<T, FtpError>;
pub type ByteStream = Pin<Box<dyn Stream<Item = FtpResult<Bytes>> + Send>>;

/// What is known about the remote file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteInfo {
    pub size: Option<u64>,
    /// Modification time as reported by the server (validator).
    pub modified: Option<String>,
    /// Reading from an offset is supported.
    pub resumable: bool,
}

pub struct Opened {
    pub info: RemoteInfo,
    /// The body starts at the requested offset (otherwise at 0).
    pub at_offset: bool,
    pub body: ByteStream,
}

#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    /// Application known-hosts file for SSH (new hosts are recorded here).
    /// `None` accepts only hosts already in `~/.ssh/known_hosts`.
    pub known_hosts: Option<PathBuf>,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(20),
            read_timeout: Duration::from_secs(30),
            known_hosts: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Ftp,
    /// Implicit TLS.
    Ftps,
    /// Explicit TLS (`AUTH TLS`).
    Ftpes,
    Sftp,
}

/// A parsed remote address with its credentials.
#[derive(Clone, PartialEq, Eq)]
pub struct Target {
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    /// Decoded path as written in the URL (leading `/` included).
    pub path: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("scheme", &self.scheme)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("path", &self.path)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Target {
    /// Parse `url`; explicit `credentials` take precedence over user info
    /// embedded in the URL.
    pub fn parse(url: &str, credentials: Option<&Credentials>) -> FtpResult<Self> {
        let u = url::Url::parse(url).map_err(|e| FtpError::InvalidUrl(e.to_string()))?;
        let (scheme, default_port) = match u.scheme() {
            "ftp" => (Scheme::Ftp, 21),
            "ftps" => (Scheme::Ftps, 990),
            "ftpes" => (Scheme::Ftpes, 21),
            "sftp" => (Scheme::Sftp, 22),
            s => return Err(FtpError::InvalidUrl(format!("unsupported scheme {s}"))),
        };
        let host = u
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| FtpError::InvalidUrl("missing host".into()))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let path = percent_decode_str(u.path()).decode_utf8_lossy().to_string();
        if path.is_empty() || path == "/" || path.ends_with('/') {
            return Err(FtpError::InvalidUrl(
                "the address does not name a file".into(),
            ));
        }
        if path.contains(['\r', '\n', '\0']) {
            return Err(FtpError::InvalidUrl(
                "invalid characters in the path".into(),
            ));
        }
        let decode = |s: &str| percent_decode_str(s).decode_utf8_lossy().to_string();
        let mut user = (!u.username().is_empty()).then(|| decode(u.username()));
        let mut password = u.password().map(decode);
        if let Some(c) = credentials.filter(|c| !c.username.is_empty()) {
            user = Some(c.username.clone());
            password = Some(c.password.clone());
        }
        Ok(Target {
            scheme,
            host,
            port: u.port().unwrap_or(default_port),
            path,
            user,
            password,
        })
    }
}

/// An FTP/FTPS/SFTP file.
pub enum Remote {
    Ftp(ftp::FtpSource),
    Sftp(sftp::SftpSource),
}

impl Remote {
    pub fn new(url: &str, credentials: Option<&Credentials>, cfg: RemoteConfig) -> FtpResult<Self> {
        let t = Target::parse(url, credentials)?;
        Ok(match t.scheme {
            Scheme::Sftp => Remote::Sftp(sftp::SftpSource::new(t, cfg)),
            _ => Remote::Ftp(ftp::FtpSource::new(t, cfg)),
        })
    }

    /// Open the file at `offset`; the body ends after `len` bytes when given.
    pub async fn open(&self, offset: u64, len: Option<u64>) -> FtpResult<Opened> {
        match self {
            Remote::Ftp(s) => s.open(offset, len).await,
            Remote::Sftp(s) => s.open(offset, len).await,
        }
    }

    /// Size, modification time and resume support, without transferring.
    pub async fn stat(&self) -> FtpResult<RemoteInfo> {
        match self {
            Remote::Ftp(s) => s.stat().await,
            Remote::Sftp(s) => s.stat().await,
        }
    }
}

/// Chunk size of body reads.
pub(crate) const CHUNK: usize = 64 * 1024;

/// Turn an async reader into a body stream that ends after `remaining`
/// bytes (when given) and fails if the transfer ends early. `keep` stays
/// alive as long as the stream (control connection / SSH session).
pub(crate) fn body_stream<K: Send + 'static>(
    keep: K,
    reader: Pin<Box<dyn tokio::io::AsyncRead + Send>>,
    remaining: Option<u64>,
    read_timeout: Duration,
) -> ByteStream {
    use tokio::io::AsyncReadExt;
    struct St<K> {
        _keep: K,
        reader: Pin<Box<dyn tokio::io::AsyncRead + Send>>,
        remaining: Option<u64>,
        timeout: Duration,
        done: bool,
    }
    let st = St {
        _keep: keep,
        reader,
        remaining,
        timeout: read_timeout,
        done: false,
    };
    Box::pin(futures::stream::unfold(st, |mut st| async move {
        if st.done || st.remaining == Some(0) {
            return None;
        }
        let want = st.remaining.map_or(CHUNK, |r| r.min(CHUNK as u64) as usize);
        let mut buf = vec![0u8; want];
        let res = tokio::time::timeout(st.timeout, st.reader.read(&mut buf)).await;
        let item = match res {
            Err(_) => Err(FtpError::Timeout),
            Ok(Err(e)) => Err(FtpError::Network(e.to_string())),
            Ok(Ok(0)) => {
                if st.remaining.is_some_and(|r| r > 0) {
                    Err(FtpError::Network(
                        "the transfer ended before the requested range was complete".into(),
                    ))
                } else {
                    return None;
                }
            }
            Ok(Ok(n)) => {
                buf.truncate(n);
                if let Some(r) = st.remaining.as_mut() {
                    *r -= n as u64;
                }
                Ok(Bytes::from(buf))
            }
        };
        if item.is_err() {
            st.done = true;
        }
        Some((item, st))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_addresses() {
        let t = Target::parse("ftp://user:p%40ss@example.com/pub/file%20name.iso", None).unwrap();
        assert_eq!(t.scheme, Scheme::Ftp);
        assert_eq!((t.host.as_str(), t.port), ("example.com", 21));
        assert_eq!(t.path, "/pub/file name.iso");
        assert_eq!(
            (t.user.as_deref(), t.password.as_deref()),
            (Some("user"), Some("p@ss"))
        );
        assert!(
            !format!("{t:?}").contains("p@ss"),
            "password must not be printed"
        );

        let t = Target::parse("ftps://h/f.bin", None).unwrap();
        assert_eq!((t.scheme, t.port), (Scheme::Ftps, 990));
        let t = Target::parse("ftpes://h:2121/f.bin", None).unwrap();
        assert_eq!((t.scheme, t.port), (Scheme::Ftpes, 2121));
        let creds = Credentials {
            username: "a".into(),
            password: "b".into(),
        };
        let t = Target::parse("sftp://u:x@[::1]/home/u/f.bin", Some(&creds)).unwrap();
        assert_eq!(
            (t.scheme, t.host.as_str(), t.port),
            (Scheme::Sftp, "::1", 22)
        );
        assert_eq!(
            (t.user.as_deref(), t.password.as_deref()),
            (Some("a"), Some("b"))
        );

        assert!(Target::parse("http://h/f", None).is_err());
        assert!(Target::parse("ftp://h/", None).is_err());
        assert!(Target::parse("ftp://h/dir/", None).is_err());
        assert!(
            Target::parse("ftp://h/a%0D%0ADELE%20x", None).is_err(),
            "no command injection"
        );
    }

    #[tokio::test]
    async fn body_stream_limits_and_detects_short_transfers() {
        use futures::StreamExt;
        let data: &'static [u8] = &[7u8; 200_000];
        let s = body_stream((), Box::pin(data), Some(150_000), Duration::from_secs(5));
        let got: Vec<Bytes> = s.map(|r| r.unwrap()).collect().await;
        assert_eq!(got.iter().map(|b| b.len()).sum::<usize>(), 150_000);
        let short: &'static [u8] = &[1u8; 10];
        let mut s = body_stream((), Box::pin(short), Some(20), Duration::from_secs(5));
        assert!(s.next().await.unwrap().is_ok());
        assert!(matches!(s.next().await, Some(Err(FtpError::Network(_)))));
        assert!(s.next().await.is_none());
    }
}
