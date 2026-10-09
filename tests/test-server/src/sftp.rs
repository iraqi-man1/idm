//! A minimal SFTP server for tests (password login, read-only virtual
//! files `/files/<size>/<name>` like the FTP server).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{Auth, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::{default_seed, fill};

#[derive(Default)]
pub struct SftpState {
    pub opens: AtomicU32,
    pub reads: AtomicU32,
    active: AtomicUsize,
    pub max_active: AtomicUsize,
    versions: Mutex<HashMap<String, u64>>,
}

pub struct SftpServer {
    pub addr: SocketAddr,
    pub state: Arc<SftpState>,
    pub host_key: PublicKey,
    user: String,
    password: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for SftpServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

fn parse_path(path: &str) -> Option<(u64, String)> {
    let rest = path.trim_start_matches('/').strip_prefix("files/")?;
    let (size, name) = rest.split_once('/')?;
    Some((size.parse().ok()?, name.to_string()))
}

impl SftpServer {
    /// Start with a fresh random host key (or `key` to keep one across restarts).
    pub async fn start(user: &str, password: &str, key: Option<PrivateKey>) -> Self {
        Self::start_on(SocketAddr::from(([127, 0, 0, 1], 0)), user, password, key).await
    }

    pub async fn start_on(
        addr: SocketAddr,
        user: &str,
        password: &str,
        key: Option<PrivateKey>,
    ) -> Self {
        let key = key
            .unwrap_or_else(|| PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap());
        let host_key = key.public_key().clone();
        let config = Arc::new(russh::server::Config {
            keys: vec![key],
            auth_rejection_time: std::time::Duration::from_millis(100),
            auth_rejection_time_initial: Some(std::time::Duration::ZERO),
            ..Default::default()
        });
        let listener = TcpListener::bind(addr).await.expect("bind sftp");
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(SftpState::default());
        let (tx, mut rx) = oneshot::channel();
        let (st, u, p) = (state.clone(), user.to_string(), password.to_string());
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    a = listener.accept() => {
                        let Ok((sock, _)) = a else { continue };
                        let handler = SshHandler { state: st.clone(), user: u.clone(), password: p.clone(), channels: Default::default() };
                        let config = config.clone();
                        tokio::spawn(async move {
                            if let Ok(running) = russh::server::run_stream(config, sock, handler).await {
                                let _ = running.await;
                            }
                        });
                    }
                }
            }
        });
        Self {
            addr,
            state,
            host_key,
            user: user.into(),
            password: password.into(),
            shutdown: Some(tx),
        }
    }

    /// `sftp://user:pass@127.0.0.1:<port>/files/<size>/<name>`.
    pub fn url(&self, size: u64, name: &str) -> String {
        format!(
            "sftp://{}:{}@{}/files/{size}/{name}",
            self.user, self.password, self.addr
        )
    }

    pub fn seed_for(&self, name: &str) -> u64 {
        default_seed(name).wrapping_add(*self.state.versions.lock().get(name).unwrap_or(&0))
    }
}

struct SshHandler {
    state: Arc<SftpState>,
    user: String,
    password: String,
    channels: Arc<tokio::sync::Mutex<HashMap<ChannelId, Channel<Msg>>>>,
}

impl russh::server::Handler for SshHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == self.user && password == self.password {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.lock().await.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let channel = self.channels.lock().await.remove(&id);
        match channel {
            Some(channel) if name == "sftp" => {
                session.channel_success(id)?;
                let handler = SftpHandler {
                    state: self.state.clone(),
                    handles: HashMap::new(),
                    next: 0,
                };
                let state = self.state.clone();
                tokio::spawn(async move {
                    let n = state.active.fetch_add(1, Ordering::SeqCst) + 1;
                    state.max_active.fetch_max(n, Ordering::SeqCst);
                    russh_sftp::server::run(channel.into_stream(), handler).await;
                });
            }
            _ => session.channel_failure(id)?,
        }
        Ok(())
    }
}

struct SftpHandler {
    state: Arc<SftpState>,
    handles: HashMap<String, (u64, String)>,
    next: u64,
}

impl Drop for SftpHandler {
    fn drop(&mut self) {
        self.state.active.fetch_sub(1, Ordering::SeqCst);
    }
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

impl SftpHandler {
    fn attrs(&self, size: u64, name: &str) -> FileAttributes {
        let v = *self.state.versions.lock().get(name).unwrap_or(&0) as u32;
        FileAttributes {
            size: Some(size),
            mtime: Some(1_767_225_600 + v),
            permissions: Some(0o100644),
            ..Default::default()
        }
    }
}

impl russh_sftp::server::Handler for SftpHandler {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _ext: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        if pflags.intersects(
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::APPEND,
        ) {
            return Err(StatusCode::PermissionDenied);
        }
        let file = parse_path(&filename).ok_or(StatusCode::NoSuchFile)?;
        self.state.opens.fetch_add(1, Ordering::SeqCst);
        self.next += 1;
        let handle = format!("h{}", self.next);
        self.handles.insert(handle.clone(), file);
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle);
        Ok(ok(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let (size, name) = self
            .handles
            .get(&handle)
            .cloned()
            .ok_or(StatusCode::Failure)?;
        if offset >= size {
            return Err(StatusCode::Eof);
        }
        self.state.reads.fetch_add(1, Ordering::SeqCst);
        let n = (size - offset).min(len as u64) as usize;
        let mut data = vec![0u8; n];
        let seed =
            default_seed(&name).wrapping_add(*self.state.versions.lock().get(&name).unwrap_or(&0));
        fill(seed, offset, &mut data);
        Ok(Data { id, data })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let (size, name) = self
            .handles
            .get(&handle)
            .cloned()
            .ok_or(StatusCode::Failure)?;
        Ok(Attrs {
            id,
            attrs: self.attrs(size, &name),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let (size, name) = parse_path(&path).ok_or(StatusCode::NoSuchFile)?;
        Ok(Attrs {
            id,
            attrs: self.attrs(size, &name),
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat(id, path).await
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        Ok(Name {
            id,
            files: vec![File::dummy(path)],
        })
    }
}
