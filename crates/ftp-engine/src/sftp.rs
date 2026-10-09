//! SFTP sources (SSH file transfer).

use std::io::SeekFrom;
use std::sync::Arc;

use parking_lot::Mutex;
use russh::client::{self, Handle};
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::error::Error as SftpErr;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::StatusCode;
use tokio::io::AsyncSeekExt;

use crate::hostkeys;
use crate::{body_stream, FtpError, FtpResult, Opened, RemoteConfig, RemoteInfo, Target};

pub struct SftpSource {
    target: Target,
    cfg: RemoteConfig,
}

struct HostCheck {
    host: String,
    port: u16,
    app_known_hosts: Option<std::path::PathBuf>,
    rejection: Arc<Mutex<Option<String>>>,
}

impl client::Handler for HostCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            *self.rejection.lock() = Some("SSH host certificates are not supported".into());
            return Ok(false);
        };
        let system = hostkeys::system_known_hosts();
        match hostkeys::verify(
            &self.host,
            self.port,
            key,
            system.as_deref(),
            self.app_known_hosts.as_deref(),
        ) {
            Ok(_) => Ok(true),
            Err(msg) => {
                *self.rejection.lock() = Some(msg);
                Ok(false)
            }
        }
    }
}

/// The SSH connection and SFTP session of one transfer.
struct Session {
    _ssh: Handle<HostCheck>,
    sftp: SftpSession,
}

fn map_sftp(e: SftpErr, path: &str) -> FtpError {
    match e {
        SftpErr::Status(s) => match s.status_code {
            StatusCode::NoSuchFile => FtpError::NotFound(path.to_string()),
            StatusCode::PermissionDenied => FtpError::Auth(format!("permission denied: {path}")),
            StatusCode::ConnectionLost | StatusCode::NoConnection => {
                FtpError::Network(s.error_message)
            }
            _ => FtpError::Protocol(format!("{}: {}", s.status_code, s.error_message)),
        },
        SftpErr::Timeout => FtpError::Timeout,
        SftpErr::IO(m) => FtpError::Network(m),
        other => FtpError::Protocol(other.to_string()),
    }
}

impl SftpSource {
    pub fn new(target: Target, cfg: RemoteConfig) -> Self {
        Self { target, cfg }
    }

    /// `/~/file` is relative to the home directory, anything else absolute.
    fn path(&self) -> &str {
        self.target
            .path
            .strip_prefix("/~/")
            .unwrap_or(&self.target.path)
    }

    async fn connect(&self) -> FtpResult<Session> {
        let t = &self.target;
        let rejection = Arc::new(Mutex::new(None));
        let handler = HostCheck {
            host: t.host.clone(),
            port: t.port,
            app_known_hosts: self.cfg.known_hosts.clone(),
            rejection: rejection.clone(),
        };
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(self.cfg.read_timeout * 4),
            ..Default::default()
        });
        let connect = client::connect(config, (t.host.as_str(), t.port), handler);
        let mut ssh = match tokio::time::timeout(self.cfg.connect_timeout, connect).await {
            Err(_) => return Err(FtpError::Timeout),
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                return Err(match rejection.lock().take() {
                    Some(msg) => FtpError::HostKey(msg),
                    None => FtpError::Network(e.to_string()),
                })
            }
        };

        let user = t
            .user
            .clone()
            .or_else(|| std::env::var("USER").ok())
            .or_else(|| std::env::var("USERNAME").ok())
            .unwrap_or_default();
        let mut authed = false;
        if let Some(pw) = &t.password {
            authed = ssh
                .authenticate_password(user.clone(), pw.clone())
                .await
                .map_err(|e| FtpError::Network(e.to_string()))?
                .success();
        }
        if !authed {
            // Unencrypted default keys (password-protected keys need an agent).
            let home = dirs::home_dir().unwrap_or_default().join(".ssh");
            for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
                let Ok(key) = load_secret_key(home.join(name), None) else {
                    continue;
                };
                let hash = if key.algorithm().is_rsa() {
                    ssh.best_supported_rsa_hash().await.ok().flatten().flatten()
                } else {
                    None
                };
                let res = ssh
                    .authenticate_publickey(
                        user.clone(),
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await
                    .map_err(|e| FtpError::Network(e.to_string()))?;
                if res.success() {
                    authed = true;
                    break;
                }
            }
        }
        if !authed {
            return Err(FtpError::Auth(format!(
                "the SSH server rejected the login of {user:?}"
            )));
        }
        let channel = ssh
            .channel_open_session()
            .await
            .map_err(|e| FtpError::Network(e.to_string()))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| FtpError::Protocol(format!("the server has no SFTP subsystem: {e}")))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| map_sftp(e, self.path()))?;
        sftp.set_timeout(self.cfg.read_timeout.as_secs().max(1));
        Ok(Session { _ssh: ssh, sftp })
    }

    pub async fn stat(&self) -> FtpResult<RemoteInfo> {
        let s = self.connect().await?;
        let meta = s
            .sftp
            .metadata(self.path())
            .await
            .map_err(|e| map_sftp(e, self.path()))?;
        let _ = s.sftp.close().await;
        Ok(RemoteInfo {
            size: meta.size,
            modified: meta.mtime.map(|t| t.to_string()),
            resumable: true,
        })
    }

    pub async fn open(&self, offset: u64, len: Option<u64>) -> FtpResult<Opened> {
        let s = self.connect().await?;
        let path = self.path().to_string();
        let mut file = s.sftp.open(&path).await.map_err(|e| map_sftp(e, &path))?;
        let meta = file.metadata().await.map_err(|e| map_sftp(e, &path))?;
        if offset > 0 {
            file.seek(SeekFrom::Start(offset))
                .await
                .map_err(|e| FtpError::Network(e.to_string()))?;
        }
        Ok(Opened {
            info: RemoteInfo {
                size: meta.size,
                modified: meta.mtime.map(|t| t.to_string()),
                resumable: true,
            },
            at_offset: true,
            body: body_stream(s, Box::pin(file), len, self.cfg.read_timeout),
        })
    }
}
