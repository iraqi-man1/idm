//! Local IPC between the native messaging host and the desktop app.
//!
//! * Unix: a Unix domain socket inside the per-user application data
//!   directory (mode 0600).
//! * Windows: a named pipe whose name contains a random component, with
//!   remote clients rejected. The client also checks that the pipe is
//!   served by the process recorded in the endpoint file.
//!
//! The app publishes `{endpoint, token, pid}` in `nm-endpoint.json` in its
//! data directory (readable only by the user). The host must present the
//! token in its first frame; connections with a wrong token are dropped.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::broadcast;
use velox_types::protocol::{ExtReply, ExtRequest, ExtResponse};

use crate::framing::{read_ipc, write_ipc, FrameError};

/// Application identifier (matches the Tauri `identifier`).
pub const APP_IDENTIFIER: &str = "com.veloxdm.app";
const ENDPOINT_FILE: &str = "nm-endpoint.json";

/// Per-user application data directory (same as Tauri's `app_data_dir`).
pub fn app_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP_IDENTIFIER))
}

/// Published connection details.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EndpointInfo {
    pub endpoint: String,
    pub token: String,
    pub pid: u32,
    pub app_version: String,
    pub app_path: Option<String>,
}

pub fn random_hex(bytes: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut b);
    hex::encode(b)
}

/// A new endpoint name for this app instance.
pub fn new_endpoint_name(data_dir: &Path) -> String {
    #[cfg(windows)]
    {
        let _ = data_dir;
        format!(r"\\.\pipe\velox-dm-{}", random_hex(12))
    }
    #[cfg(not(windows))]
    {
        data_dir.join("ipc.sock").to_string_lossy().to_string()
    }
}

pub fn write_endpoint_file(data_dir: &Path, info: &EndpointInfo) -> io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(ENDPOINT_FILE);
    let tmp = data_dir.join(format!("{ENDPOINT_FILE}.tmp"));
    let json = serde_json::to_vec_pretty(info).map_err(io::Error::other)?;
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)
}

pub fn read_endpoint_file(data_dir: &Path) -> io::Result<EndpointInfo> {
    let raw = std::fs::read(data_dir.join(ENDPOINT_FILE))?;
    serde_json::from_slice(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn remove_endpoint_file(data_dir: &Path) {
    let _ = std::fs::remove_file(data_dir.join(ENDPOINT_FILE));
}

/// First frame sent by the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientHello {
    Hello {
        token: String,
        origin: String,
        browser: String,
        host_version: String,
    },
}

/// Server answer to the hello.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerHello {
    Welcome { app_version: String },
    Denied { reason: String },
}

/// Identity of a connected extension (as reported by the host, which
/// takes it from the browser's command line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    pub conn_id: u64,
    pub origin: String,
    pub browser: String,
    pub host_version: String,
}

pub type HandlerFuture<'a> = Pin<Box<dyn Future<Output = ExtReply> + Send + 'a>>;

/// Implemented by the desktop app to answer extension requests.
pub trait BridgeHandler: Send + Sync + 'static {
    fn handle<'a>(&'a self, peer: &'a PeerInfo, req: ExtRequest) -> HandlerFuture<'a>;
    fn connected(&self, _peer: &PeerInfo) {}
    fn disconnected(&self, _peer: &PeerInfo) {}
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Handle to a running IPC server.
#[derive(Clone)]
pub struct ServerHandle {
    notify: broadcast::Sender<ExtResponse>,
    connections: Arc<AtomicU64>,
}

impl ServerHandle {
    /// Push an unsolicited message (id 0) to every connected extension.
    pub fn notify_all(&self, reply: ExtReply) {
        let _ = self.notify.send(ExtResponse { id: 0, reply });
    }

    /// Number of currently connected hosts.
    pub fn connection_count(&self) -> u64 {
        self.connections.load(Ordering::Relaxed)
    }
}

async fn serve_connection<S: Stream + 'static>(
    mut stream: S,
    token: Arc<String>,
    handler: Arc<dyn BridgeHandler>,
    conn_id: u64,
    mut notify: broadcast::Receiver<ExtResponse>,
    connections: Arc<AtomicU64>,
) {
    let hello: Option<ClientHello> =
        match tokio::time::timeout(std::time::Duration::from_secs(10), read_ipc(&mut stream)).await
        {
            Ok(Ok(h)) => h,
            _ => return,
        };
    let Some(ClientHello::Hello {
        token: t,
        origin,
        browser,
        host_version,
    }) = hello
    else {
        return;
    };
    if !constant_time_eq(t.as_bytes(), token.as_bytes()) {
        let _ = write_ipc(
            &mut stream,
            &ServerHello::Denied {
                reason: "invalid token".into(),
            },
        )
        .await;
        return;
    }
    if write_ipc(
        &mut stream,
        &ServerHello::Welcome {
            app_version: env!("CARGO_PKG_VERSION").into(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let peer = PeerInfo {
        conn_id,
        origin,
        browser,
        host_version,
    };
    connections.fetch_add(1, Ordering::Relaxed);
    handler.connected(&peer);
    let (mut rd, mut wr) = tokio::io::split(stream);
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<ExtResponse>(64);
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(msg) = out_rx.recv() => {
                    if write_ipc(&mut wr, &msg).await.is_err() { break; }
                }
                n = notify.recv() => match n {
                    Ok(msg) => { if write_ipc(&mut wr, &msg).await.is_err() { break; } }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                else => break,
            }
        }
    });
    loop {
        let req: Result<Option<serde_json::Value>, FrameError> = read_ipc(&mut rd).await;
        let value = match req {
            Ok(Some(v)) => v,
            _ => break,
        };
        let id = value.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let parsed: Result<ExtRequest, _> = serde_json::from_value(value);
        let resp = match parsed {
            Err(e) => ExtResponse::error(id, "invalid_request", e.to_string()),
            Ok(req) => match req.validate() {
                Err(e) => ExtResponse::error(id, "invalid_request", e.0),
                Ok(()) => {
                    let id = req.id;
                    // Handle concurrently so a slow probe never blocks other requests.
                    let handler = handler.clone();
                    let peer = peer.clone();
                    let tx = out_tx.clone();
                    tokio::spawn(async move {
                        let reply = handler.handle(&peer, req).await;
                        let _ = tx.send(ExtResponse { id, reply }).await;
                    });
                    continue;
                }
            },
        };
        if out_tx.send(resp).await.is_err() {
            break;
        }
    }
    drop(out_tx);
    writer.abort();
    connections.fetch_sub(1, Ordering::Relaxed);
    handler.disconnected(&peer);
}

/// Start the IPC server on `endpoint`. Runs until the process exits.
pub async fn serve(
    endpoint: &str,
    token: String,
    handler: Arc<dyn BridgeHandler>,
) -> io::Result<ServerHandle> {
    let (notify, _) = broadcast::channel(32);
    let handle = ServerHandle {
        notify: notify.clone(),
        connections: Arc::new(AtomicU64::new(0)),
    };
    let token = Arc::new(token);
    let next_id = Arc::new(AtomicU64::new(1));
    let connections = handle.connections.clone();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = PathBuf::from(endpoint);
        if path.exists() {
            // A stale socket from a crashed instance; a live instance is
            // prevented by the single-instance lock of the app.
            let _ = std::fs::remove_file(&path);
        }
        let listener = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let id = next_id.fetch_add(1, Ordering::Relaxed);
                        tokio::spawn(serve_connection(
                            stream,
                            token.clone(),
                            handler.clone(),
                            id,
                            notify.subscribe(),
                            connections.clone(),
                        ));
                    }
                    Err(e) => {
                        eprintln!("velox ipc accept error: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }
            }
        });
    }

    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = endpoint.to_string();
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)?;
        tokio::spawn(async move {
            loop {
                if let Err(e) = server.connect().await {
                    eprintln!("velox ipc connect error: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    continue;
                }
                let connected = server;
                server = match ServerOptions::new()
                    .reject_remote_clients(true)
                    .create(&name)
                {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("velox ipc cannot create pipe instance: {e}");
                        return;
                    }
                };
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(serve_connection(
                    connected,
                    token.clone(),
                    handler.clone(),
                    id,
                    notify.subscribe(),
                    connections.clone(),
                ));
            }
        });
    }

    Ok(handle)
}

/// Client side used by the native host.
pub struct IpcClient {
    pub reader: Box<dyn AsyncRead + Unpin + Send>,
    pub writer: Box<dyn AsyncWrite + Unpin + Send>,
    pub app_version: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("the desktop app is not running")]
    NotRunning,
    #[error("connection refused by the desktop app: {0}")]
    Denied(String),
    #[error("the endpoint is not served by the expected process")]
    WrongServer,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[cfg(windows)]
fn pipe_server_pid(client: &tokio::net::windows::named_pipe::NamedPipeClient) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let mut pid: u32 = 0;
    // SAFETY: valid pipe handle owned by `client`.
    let ok = unsafe { GetNamedPipeServerProcessId(client.as_raw_handle() as _, &mut pid) };
    (ok != 0).then_some(pid)
}

/// Connect and authenticate using the published endpoint file.
pub async fn connect(
    info: &EndpointInfo,
    origin: &str,
    browser: &str,
) -> Result<IpcClient, ConnectError> {
    let (reader, writer): (
        Box<dyn AsyncRead + Unpin + Send>,
        Box<dyn AsyncWrite + Unpin + Send>,
    );
    #[cfg(unix)]
    {
        let stream = tokio::net::UnixStream::connect(&info.endpoint)
            .await
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                    ConnectError::NotRunning
                }
                _ => ConnectError::Io(e),
            })?;
        let (r, w) = stream.into_split();
        reader = Box::new(r);
        writer = Box::new(w);
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        let client =
            ClientOptions::new()
                .open(&info.endpoint)
                .map_err(|e| match e.raw_os_error() {
                    Some(2) => ConnectError::NotRunning, // ERROR_FILE_NOT_FOUND
                    _ => ConnectError::Io(e),
                })?;
        if let Some(pid) = pipe_server_pid(&client) {
            if pid != info.pid {
                return Err(ConnectError::WrongServer);
            }
        }
        let (r, w) = tokio::io::split(client);
        reader = Box::new(r);
        writer = Box::new(w);
    }
    let mut c = IpcClient {
        reader,
        writer,
        app_version: String::new(),
    };
    let hello = ClientHello::Hello {
        token: info.token.clone(),
        origin: origin.to_string(),
        browser: browser.to_string(),
        host_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    write_ipc(&mut c.writer, &hello)
        .await
        .map_err(|e| ConnectError::Protocol(e.to_string()))?;
    let answer: Option<ServerHello> =
        tokio::time::timeout(std::time::Duration::from_secs(10), read_ipc(&mut c.reader))
            .await
            .map_err(|_| ConnectError::Protocol("timeout waiting for the app".into()))?
            .map_err(|e| ConnectError::Protocol(e.to_string()))?;
    match answer {
        Some(ServerHello::Welcome { app_version }) => {
            c.app_version = app_version;
            Ok(c)
        }
        Some(ServerHello::Denied { reason }) => Err(ConnectError::Denied(reason)),
        None => Err(ConnectError::Protocol(
            "connection closed during handshake".into(),
        )),
    }
}
