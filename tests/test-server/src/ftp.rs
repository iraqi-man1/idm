//! A minimal FTP server for tests (passive mode, binary transfers).
//!
//! Files are virtual: `/files/<size>/<name>` serves `size` bytes of
//! [`crate::expected_content`] seeded from `name` (and its version, see
//! [`FtpServer::bump`]). Options simulate connection limits, servers
//! without `REST`, and transfers that break after some bytes.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use crate::{default_seed, fill};

#[derive(Debug, Clone, Default)]
pub struct FtpOptions {
    /// Required `user`/`password`; `None` accepts any login.
    pub login: Option<(String, String)>,
    /// Answer `421` beyond this many simultaneous control connections.
    pub max_connections: Option<usize>,
    /// Refuse `REST` (no resume support).
    pub no_rest: bool,
    /// Close the data connection after this many bytes ...
    pub fail_after: Option<u64>,
    /// ... for the first `fail_times` transfers (0 = always).
    pub fail_times: u32,
    /// Throttle each transfer to this many bytes per second.
    pub rate: Option<u64>,
}

#[derive(Default)]
pub struct FtpState {
    opts: FtpOptions,
    active: AtomicUsize,
    pub max_active: AtomicUsize,
    pub retr: AtomicU32,
    pub refused: AtomicU32,
    /// Offsets requested with `REST`, per file name.
    pub rest_offsets: Mutex<HashMap<String, Vec<u64>>>,
    versions: Mutex<HashMap<String, u64>>,
}

impl FtpState {
    fn seed(&self, name: &str) -> u64 {
        default_seed(name).wrapping_add(*self.versions.lock().get(name).unwrap_or(&0))
    }
}

pub struct FtpServer {
    pub addr: SocketAddr,
    pub state: Arc<FtpState>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for FtpServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// Parse `/files/<size>/<name>`.
fn parse_path(path: &str) -> Option<(u64, String)> {
    let p = path.trim_start_matches('/');
    let rest = p.strip_prefix("files/")?;
    let (size, name) = rest.split_once('/')?;
    Some((size.parse().ok()?, name.to_string()))
}

impl FtpServer {
    pub async fn start(opts: FtpOptions) -> Self {
        Self::start_on(SocketAddr::from(([127, 0, 0, 1], 0)), opts).await
    }

    pub async fn start_on(addr: SocketAddr, opts: FtpOptions) -> Self {
        let listener = TcpListener::bind(addr).await.expect("bind ftp");
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(FtpState {
            opts,
            ..Default::default()
        });
        let (tx, mut rx) = oneshot::channel();
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    a = listener.accept() => {
                        if let Ok((sock, _)) = a {
                            tokio::spawn(session(sock, st.clone()));
                        }
                    }
                }
            }
        });
        Self {
            addr,
            state,
            shutdown: Some(tx),
        }
    }

    /// `ftp://[user:pass@]127.0.0.1:<port>/files/<size>/<name>`.
    pub fn url(&self, size: u64, name: &str) -> String {
        let auth = match &self.state.opts.login {
            Some((u, p)) => format!("{u}:{p}@"),
            None => String::new(),
        };
        format!("ftp://{auth}{}/files/{size}/{name}", self.addr)
    }

    pub fn seed_for(&self, name: &str) -> u64 {
        self.state.seed(name)
    }

    /// Change the content and modification time of `name`.
    pub fn bump(&self, name: &str) {
        *self
            .state
            .versions
            .lock()
            .entry(name.to_string())
            .or_insert(0) += 1;
    }
}

struct Active(Arc<FtpState>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn session(sock: TcpStream, st: Arc<FtpState>) {
    let n = st.active.fetch_add(1, Ordering::SeqCst) + 1;
    let _active = Active(st.clone());
    st.max_active.fetch_max(n, Ordering::SeqCst);
    let (r, mut w) = sock.into_split();
    if st.opts.max_connections.is_some_and(|m| n > m) {
        st.refused.fetch_add(1, Ordering::SeqCst);
        let _ = w
            .write_all(b"421 Too many connections, try again later\r\n")
            .await;
        return;
    }
    let _ = w.write_all(b"220 Velox test FTP server\r\n").await;
    let mut lines = BufReader::new(r).lines();
    let mut user = String::new();
    let mut logged_in = false;
    let mut passive: Option<TcpListener> = None;
    let mut rest: u64 = 0;
    macro_rules! reply {
        ($($t:tt)*) => {
            if w.write_all(format!($($t)*).as_bytes()).await.is_err() { return; }
        };
    }
    while let Ok(Some(line)) = lines.next_line().await {
        let (cmd, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let cmd = cmd.to_ascii_uppercase();
        let needs_login = !matches!(
            cmd.as_str(),
            "USER" | "PASS" | "QUIT" | "FEAT" | "SYST" | "NOOP" | "OPTS"
        );
        if needs_login && !logged_in {
            reply!("530 Please log in\r\n");
            continue;
        }
        match cmd.as_str() {
            "USER" => {
                user = arg.to_string();
                reply!("331 Password required\r\n");
            }
            "PASS" => match &st.opts.login {
                Some((u, p)) if (u.as_str(), p.as_str()) != (user.as_str(), arg) => {
                    reply!("530 Login incorrect\r\n")
                }
                _ => {
                    logged_in = true;
                    reply!("230 Logged in\r\n");
                }
            },
            "SYST" => reply!("215 UNIX Type: L8\r\n"),
            "FEAT" => {
                let rest_feat = if st.opts.no_rest {
                    ""
                } else {
                    " REST STREAM\r\n"
                };
                reply!("211-Features:\r\n SIZE\r\n MDTM\r\n{rest_feat}211 End\r\n");
            }
            "OPTS" | "NOOP" => reply!("200 OK\r\n"),
            "TYPE" => reply!("200 Type set to {arg}\r\n"),
            "PWD" => reply!("257 \"/\" is the current directory\r\n"),
            "CWD" => reply!("250 OK\r\n"),
            "PASV" => match TcpListener::bind("127.0.0.1:0").await {
                Ok(l) => {
                    let port = l.local_addr().unwrap().port();
                    passive = Some(l);
                    reply!(
                        "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                        port >> 8,
                        port & 0xff
                    );
                }
                Err(_) => reply!("425 Cannot open data connection\r\n"),
            },
            "EPSV" => match TcpListener::bind("127.0.0.1:0").await {
                Ok(l) => {
                    let port = l.local_addr().unwrap().port();
                    passive = Some(l);
                    reply!("229 Entering Extended Passive Mode (|||{port}|)\r\n");
                }
                Err(_) => reply!("425 Cannot open data connection\r\n"),
            },
            "SIZE" => match parse_path(arg) {
                Some((size, _)) => reply!("213 {size}\r\n"),
                None => reply!("550 {arg}: No such file\r\n"),
            },
            "MDTM" => match parse_path(arg) {
                Some((_, name)) => {
                    let v = *st.versions.lock().get(&name).unwrap_or(&0);
                    reply!("213 202601010000{:02}\r\n", v % 60);
                }
                None => reply!("550 {arg}: No such file\r\n"),
            },
            "REST" => {
                if st.opts.no_rest {
                    reply!("502 REST not implemented\r\n");
                } else {
                    match arg.parse::<u64>() {
                        Ok(n) => {
                            rest = n;
                            reply!("350 Restarting at {n}\r\n");
                        }
                        Err(_) => reply!("501 Bad REST argument\r\n"),
                    }
                }
            }
            "RETR" => {
                let Some((size, name)) = parse_path(arg) else {
                    reply!("550 {arg}: No such file\r\n");
                    continue;
                };
                let Some(listener) = passive.take() else {
                    reply!("425 Use PASV first\r\n");
                    continue;
                };
                let offset = std::mem::take(&mut rest).min(size);
                st.rest_offsets
                    .lock()
                    .entry(name.clone())
                    .or_default()
                    .push(offset);
                let nth = st.retr.fetch_add(1, Ordering::SeqCst) + 1;
                reply!("150 Opening BINARY mode data connection for {name} ({size} bytes)\r\n");
                let data =
                    match tokio::time::timeout(Duration::from_secs(10), listener.accept()).await {
                        Ok(Ok((d, _))) => d,
                        _ => {
                            reply!("425 Data connection failed\r\n");
                            continue;
                        }
                    };
                let fail = st
                    .opts
                    .fail_after
                    .filter(|_| st.opts.fail_times == 0 || nth <= st.opts.fail_times);
                let ok = send_file(data, st.seed(&name), offset, size, fail, st.opts.rate).await;
                if ok {
                    reply!("226 Transfer complete\r\n");
                } else {
                    reply!("426 Connection closed; transfer aborted\r\n");
                }
            }
            "ABOR" => reply!("226 Abort successful\r\n"),
            "QUIT" => {
                reply!("221 Goodbye\r\n");
                return;
            }
            _ => reply!("502 Command not implemented\r\n"),
        }
    }
}

/// Write `[offset, size)` of the file; `false` when stopped deliberately or
/// when the client went away.
async fn send_file(
    mut data: TcpStream,
    seed: u64,
    offset: u64,
    size: u64,
    fail_after: Option<u64>,
    rate: Option<u64>,
) -> bool {
    let mut pos = offset;
    let mut sent = 0u64;
    let mut buf = vec![0u8; 32 * 1024];
    while pos < size {
        let mut n = (size - pos).min(buf.len() as u64);
        if let Some(f) = fail_after {
            if sent >= f {
                return false;
            }
            n = n.min(f - sent);
        }
        fill(seed, pos, &mut buf[..n as usize]);
        if data.write_all(&buf[..n as usize]).await.is_err() {
            return false;
        }
        pos += n;
        sent += n;
        if let Some(r) = rate.filter(|r| *r > 0) {
            tokio::time::sleep(Duration::from_secs_f64(n as f64 / r as f64)).await;
        }
    }
    let _ = data.shutdown().await;
    true
}
