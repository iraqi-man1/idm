//! FTP / FTPS (implicit and explicit TLS) sources.

use std::pin::Pin;
use std::sync::Arc;

use suppaftp::tokio::{AsyncFtpStream, AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::{FtpError as Raw, Status};
use tokio::io::AsyncRead;

use crate::{body_stream, FtpError, FtpResult, Opened, RemoteConfig, RemoteInfo, Scheme, Target};

pub struct FtpSource {
    target: Target,
    cfg: RemoteConfig,
}

enum Conn {
    Plain(AsyncFtpStream),
    Tls(AsyncRustlsFtpStream),
}

macro_rules! on {
    ($c:expr, $s:ident => $e:expr) => {
        match $c {
            Conn::Plain($s) => $e,
            Conn::Tls($s) => $e,
        }
    };
}

fn map(e: Raw) -> FtpError {
    match e {
        Raw::ConnectionError(io) if io.kind() == std::io::ErrorKind::TimedOut => FtpError::Timeout,
        Raw::ConnectionError(io) => FtpError::Network(io.to_string()),
        Raw::SecureError(m) => FtpError::Tls(m),
        Raw::UnexpectedResponse(r) => {
            let text = r.as_string().unwrap_or_default();
            let msg = format!("{} {}", r.status.code(), text.trim());
            match r.status.code() {
                421 => FtpError::Refused(msg),
                530 if text.to_ascii_lowercase().contains("too many") => FtpError::Refused(msg),
                530 | 331 | 332 => FtpError::Auth(msg),
                550 | 553 => FtpError::NotFound(msg),
                425 | 426 | 450 | 451 => FtpError::Network(msg),
                _ => FtpError::Protocol(format!("unexpected FTP reply: {msg}")),
            }
        }
        Raw::BadResponse => FtpError::Protocol("malformed FTP reply".into()),
        Raw::InvalidAddress(e) => FtpError::Protocol(format!("invalid passive address: {e}")),
        Raw::DataConnectionAlreadyOpen => FtpError::Protocol("data connection already open".into()),
    }
}

fn tls_connector() -> FtpResult<AsyncRustlsConnector> {
    use rustls_platform_verifier::ConfigVerifierExt;
    // Same provider as the HTTP engine; installing twice is harmless.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config =
        rustls::ClientConfig::with_platform_verifier().map_err(|e| FtpError::Tls(e.to_string()))?;
    Ok(AsyncRustlsConnector::from(
        tokio_rustls::TlsConnector::from(Arc::new(config)),
    ))
}

impl FtpSource {
    pub fn new(target: Target, cfg: RemoteConfig) -> Self {
        Self { target, cfg }
    }

    /// Path as sent to the server: relative to the login directory, or
    /// absolute when the URL encodes it (`ftp://host/%2Fetc/file`).
    fn path(&self) -> &str {
        self.target
            .path
            .strip_prefix('/')
            .unwrap_or(&self.target.path)
    }

    async fn connect(&self) -> FtpResult<Conn> {
        let t = &self.target;
        let addr = (t.host.as_str(), t.port);
        let timeout = self.cfg.connect_timeout;
        let fut = async {
            let mut c = match t.scheme {
                Scheme::Ftp => Conn::Plain(AsyncFtpStream::connect(addr).await.map_err(map)?),
                Scheme::Ftpes => {
                    let plain = suppaftp::tokio::ImplAsyncFtpStream::connect(addr)
                        .await
                        .map_err(map)?;
                    Conn::Tls(
                        plain
                            .into_secure(tls_connector()?, &t.host)
                            .await
                            .map_err(map)?,
                    )
                }
                Scheme::Ftps => Conn::Tls(
                    AsyncRustlsFtpStream::connect_secure_implicit(addr, tls_connector()?, &t.host)
                        .await
                        .map_err(map)?,
                ),
                Scheme::Sftp => return Err(FtpError::InvalidUrl("not an FTP address".into())),
            };
            let user = t.user.clone().unwrap_or_else(|| "anonymous".into());
            let pass = t.password.clone().unwrap_or_else(|| "anonymous@".into());
            on!(&mut c, s => s.login(user.as_str(), pass.as_str()).await).map_err(map)?;
            on!(&mut c, s => s.transfer_type(FileType::Binary).await).map_err(map)?;
            Ok(c)
        };
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| FtpError::Timeout)?
    }

    async fn info(&self, c: &mut Conn) -> FtpResult<RemoteInfo> {
        let path = self.path().to_string();
        let size = match on!(c, s => s.size(&path).await) {
            Ok(n) => Some(n as u64),
            Err(e) => match map(e) {
                FtpError::NotFound(m) => return Err(FtpError::NotFound(m)),
                // SIZE is optional (RFC 3659); the transfer still works.
                _ => None,
            },
        };
        let modified = on!(c, s => s.mdtm(&path).await)
            .ok()
            .map(|t| t.format("%Y%m%d%H%M%S").to_string());
        let resumable = match on!(c, s => s.feat().await) {
            Ok(f) if f.keys().any(|k| k.eq_ignore_ascii_case("REST")) => true,
            _ => on!(c, s => s.resume_transfer(0).await).is_ok(),
        };
        Ok(RemoteInfo {
            size,
            modified,
            resumable,
        })
    }

    pub async fn stat(&self) -> FtpResult<RemoteInfo> {
        let mut c = self.connect().await?;
        let info = self.info(&mut c).await;
        let _ = on!(&mut c, s => s.quit().await);
        info
    }

    pub async fn open(&self, offset: u64, len: Option<u64>) -> FtpResult<Opened> {
        let mut c = self.connect().await?;
        let info = self.info(&mut c).await?;
        let mut at_offset = offset == 0;
        if offset > 0 {
            match on!(&mut c, s => s.resume_transfer(offset as usize).await) {
                Ok(()) => at_offset = true,
                Err(Raw::UnexpectedResponse(r)) if r.status != Status::RequestFilePending => {
                    if len.is_some() {
                        return Err(FtpError::NoResume(r.as_string().unwrap_or_default()));
                    }
                }
                Err(e) => return Err(map(e)),
            }
        }
        let path = self.path().to_string();
        let reader: Pin<Box<dyn AsyncRead + Send>> = match &mut c {
            Conn::Plain(s) => Box::pin(s.retr_as_stream(&path).await.map_err(map)?),
            Conn::Tls(s) => Box::pin(s.retr_as_stream(&path).await.map_err(map)?),
        };
        let remaining = if at_offset { len } else { None };
        Ok(Opened {
            info,
            at_offset,
            body: body_stream(c, reader, remaining, self.cfg.read_timeout),
        })
    }
}
