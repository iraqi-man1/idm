//! A local HTTP server that simulates the behaviour of real download
//! servers, including broken ones. Used by the engine integration tests and
//! available as a binary (`cargo run -p velox-test-server -- --port 8787`)
//! for manual testing of the desktop app.
//!
//! `GET /file/{name}?size=N[&options]` serves deterministic content.
//! Options (query parameters):
//!
//! | option          | effect                                                         |
//! |-----------------|----------------------------------------------------------------|
//! | `seed`          | content seed (default derived from name)                       |
//! | `norange=1`     | ignore `Range`, always answer 200 with the full body           |
//! | `rate=B`        | throttle each response to B bytes/second                       |
//! | `fail_after=N`  | abort the connection after N body bytes                        |
//! | `fail_times=K`  | only the first K responses are aborted                         |
//! | `stall_after=N` | stop sending (hang) after N body bytes                         |
//! | `stall_times=K` | only the first K responses stall                               |
//! | `maxconn=K`     | answer 503 when more than K responses are in flight            |
//! | `nolength=1`    | chunked body without `Content-Length`, ranges disabled         |
//! | `malformed=1`   | ranged responses carry a wrong `Content-Range` start           |
//! | `cd=NAME`       | send `Content-Disposition: attachment; filename*=UTF-8''NAME`  |
//! | `mime=TYPE`     | `Content-Type` (default application/octet-stream)              |
//! | `noetag=1`      | omit `ETag`                                                    |
//! | `auth=1`        | require HTTP Basic `user:pass`                                  |
//! | `cookie=V`      | require a `Cookie` header containing V                          |
//! | `expire_after=K`| after K requests answer 403 (expired link)                      |
//! | `html_after=K`  | after K requests answer 200 with an HTML page                   |
//! | `status=S`      | always answer with status S                                     |
//!
//! `POST /admin/bump/{name}` changes the content and ETag of `name`,
//! simulating a remote file that changed between sessions.
//! `GET /redirect?n=K&to=/file/...` redirects K times before reaching `to`.
//! `GET /static/{path}` serves files below the directory given to
//! [`TestServer::serve_dir`] (single `Range` requests supported); used for
//! HLS/DASH tests with real media. [`TestServer::force_status`] makes one
//! static path answer with a fixed status (e.g. 403 for an expired link);
//! [`TestServer::require_static_login`] protects them with Basic auth.

pub mod ftp;
pub mod sftp;

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use base64::Engine;
use bytes::Bytes;
use parking_lot::Mutex;
use serde::Deserialize;
use tokio::sync::oneshot;

/// Deterministic content byte at absolute offset `i`.
pub fn byte_at(seed: u64, i: u64) -> u8 {
    let x = (i ^ seed.rotate_left(17)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let x = x ^ (x >> 29);
    (x.wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 56) as u8
}

/// Fill `buf` with content starting at `offset`.
pub fn fill(seed: u64, offset: u64, buf: &mut [u8]) {
    for (k, b) in buf.iter_mut().enumerate() {
        *b = byte_at(seed, offset + k as u64);
    }
}

/// The complete expected content of a resource.
pub fn expected_content(seed: u64, size: u64) -> Vec<u8> {
    let mut v = vec![0u8; size as usize];
    fill(seed, 0, &mut v);
    v
}

/// Seed used for `name` when no explicit seed is given (version 0).
pub fn default_seed(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// Per-resource counters.
#[derive(Default, Debug)]
pub struct ResourceStats {
    pub requests: AtomicU32,
    pub range_requests: AtomicU32,
    pub active: AtomicU32,
    pub max_active: AtomicU32,
    pub bytes_sent: AtomicU64,
    pub rejected: AtomicU32,
}

#[derive(Default)]
pub struct ServerState {
    stats: Mutex<HashMap<String, Arc<ResourceStats>>>,
    versions: Mutex<HashMap<String, u64>>,
    static_root: Mutex<Option<PathBuf>>,
    forced_status: Mutex<HashMap<String, u16>>,
    /// Expected `Authorization` header for `/static/...`, if any.
    static_login: Mutex<Option<String>>,
}

impl ServerState {
    pub fn stats(&self, name: &str) -> Arc<ResourceStats> {
        self.stats
            .lock()
            .entry(name.to_string())
            .or_default()
            .clone()
    }

    pub fn version(&self, name: &str) -> u64 {
        *self.versions.lock().get(name).unwrap_or(&0)
    }

    pub fn bump(&self, name: &str) {
        *self.versions.lock().entry(name.to_string()).or_insert(0) += 1;
    }
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub state: Arc<ServerState>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestServer {
    /// Start on an ephemeral localhost port.
    pub async fn start() -> Self {
        Self::start_on(SocketAddr::from(([127, 0, 0, 1], 0))).await
    }

    pub async fn start_on(addr: SocketAddr) -> Self {
        let state = Arc::new(ServerState::default());
        let app = router(state.clone());
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("bind test server");
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Self {
            addr,
            state,
            shutdown: Some(tx),
        }
    }

    pub fn url(&self, path_and_query: &str) -> String {
        format!("http://{}{}", self.addr, path_and_query)
    }

    pub fn stats(&self, name: &str) -> Arc<ResourceStats> {
        self.state.stats(name)
    }

    /// Effective content seed of a resource with the default seed.
    pub fn seed_for(&self, name: &str) -> u64 {
        default_seed(name).wrapping_add(self.state.version(name))
    }

    pub fn bump(&self, name: &str) {
        self.state.bump(name)
    }

    /// Serve the files below `dir` at `/static/...`.
    pub fn serve_dir(&self, dir: impl Into<PathBuf>) {
        *self.state.static_root.lock() = Some(dir.into());
    }

    /// Require HTTP Basic authentication for every `/static/...` request.
    pub fn require_static_login(&self, user: &str, password: &str) {
        *self.state.static_login.lock() = Some(format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
        ));
    }

    /// Make `/static/{path}` answer `status` (or serve it normally again).
    pub fn force_status(&self, path: &str, status: Option<u16>) {
        let mut m = self.state.forced_status.lock();
        match status {
            Some(s) => m.insert(path.to_string(), s),
            None => m.remove(path),
        };
    }

    /// Request counters of a static file.
    pub fn static_stats(&self, path: &str) -> Arc<ResourceStats> {
        self.state.stats(&format!("static/{path}"))
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/file/{name}", get(serve_file))
        .route("/admin/bump/{name}", post(bump))
        .route("/redirect", get(redirect))
        .route("/page", get(page))
        .route("/static/{*path}", get(serve_static))
        .with_state(state)
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
struct FileQuery {
    size: u64,
    seed: Option<u64>,
    norange: Option<u8>,
    rate: Option<u64>,
    fail_after: Option<u64>,
    fail_times: Option<u32>,
    stall_after: Option<u64>,
    stall_times: Option<u32>,
    maxconn: Option<u32>,
    nolength: Option<u8>,
    malformed: Option<u8>,
    cd: Option<String>,
    mime: Option<String>,
    noetag: Option<u8>,
    auth: Option<u8>,
    cookie: Option<String>,
    expire_after: Option<u32>,
    html_after: Option<u32>,
    status: Option<u16>,
}

const LAST_MODIFIED_BASE: u64 = 1_700_000_000;

async fn bump(State(state): State<Arc<ServerState>>, Path(name): Path<String>) -> StatusCode {
    state.bump(&name);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct RedirectQuery {
    n: u32,
    to: String,
}

async fn redirect(Query(q): Query<RedirectQuery>) -> Response<Body> {
    let location = if q.n <= 1 {
        q.to.clone()
    } else {
        format!(
            "/redirect?n={}&to={}",
            q.n - 1,
            q.to.replace('%', "%25")
                .replace('&', "%26")
                .replace('?', "%3F")
                .replace('=', "%3D")
        )
    };
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap()
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PageQuery {
    /// Link target rendered as `<a id="link">`.
    href: Option<String>,
    /// Video source rendered as `<video id="video">`.
    video: Option<String>,
    title: Option<String>,
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `GET /page?href=URL&video=URL&title=T`: a minimal HTML page for browser tests.
async fn page(Query(q): Query<PageQuery>) -> impl IntoResponse {
    let mut body = String::from("<!doctype html><html><head><meta charset=\"utf-8\"><title>");
    body.push_str(&html_escape(
        q.title.as_deref().unwrap_or("Velox test page"),
    ));
    body.push_str("</title></head><body style=\"font-family:sans-serif;margin:24px\"><h1>Velox test page</h1>");
    if let Some(h) = &q.href {
        body.push_str(&format!(
            "<p><a id=\"link\" href=\"{}\">Download file</a></p>",
            html_escape(h)
        ));
    }
    if let Some(v) = &q.video {
        body.push_str(&format!(
            "<video id=\"video\" src=\"{}\" width=\"640\" height=\"360\" controls muted playsinline></video>",
            html_escape(v)
        ));
    }
    body.push_str("</body></html>");
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body)
}

fn static_mime(path: &str) -> &'static str {
    match path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("m3u8") => "application/vnd.apple.mpegurl",
        Some("mpd") => "application/dash+xml",
        Some("ts") => "video/mp2t",
        Some("mp4" | "m4s" | "m4v") => "video/mp4",
        Some("m4a") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("vtt") => "text/vtt",
        Some("html") => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// `GET /static/{path}`: files below the configured directory.
async fn serve_static(
    State(state): State<Arc<ServerState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Response<Body> {
    let stats = state.stats(&format!("static/{path}"));
    stats.requests.fetch_add(1, Ordering::SeqCst);
    let status = |s: StatusCode| Response::builder().status(s).body(Body::empty()).unwrap();
    if let Some(s) = state.forced_status.lock().get(&path).copied() {
        return status(StatusCode::from_u16(s).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    }
    let login = state.static_login.lock().clone();
    if let Some(expected) = login {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        if given != Some(expected.as_str()) {
            stats.rejected.fetch_add(1, Ordering::SeqCst);
            return status(StatusCode::UNAUTHORIZED);
        }
    }
    if path
        .split('/')
        .any(|c| c.is_empty() || c == "." || c == "..")
        || path.contains('\\')
    {
        return status(StatusCode::BAD_REQUEST);
    }
    let Some(root) = state.static_root.lock().clone() else {
        return status(StatusCode::NOT_FOUND);
    };
    let Ok(data) = tokio::fs::read(root.join(&path)).await else {
        return status(StatusCode::NOT_FOUND);
    };
    let size = data.len() as u64;
    let builder = Response::builder()
        .header(header::CONTENT_TYPE, static_mime(&path))
        .header(header::ACCEPT_RANGES, "bytes");
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (builder, body) = match range {
        Some(r) => match parse_range(r, size) {
            Some((a, b)) => {
                stats.range_requests.fetch_add(1, Ordering::SeqCst);
                (
                    builder
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_RANGE, format!("bytes {a}-{b}/{size}")),
                    data[a as usize..=b as usize].to_vec(),
                )
            }
            None => {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                    .body(Body::empty())
                    .unwrap()
            }
        },
        None => (builder.status(StatusCode::OK), data),
    };
    stats
        .bytes_sent
        .fetch_add(body.len() as u64, Ordering::SeqCst);
    builder
        .header(header::CONTENT_LENGTH, body.len())
        .body(Body::from(body))
        .unwrap()
}

struct ActiveGuard(Arc<ResourceStats>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

fn parse_range(h: &str, size: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    if a.is_empty() {
        // suffix range
        let n: u64 = b.parse().ok()?;
        if n == 0 {
            return None;
        }
        return Some((size.saturating_sub(n), size - 1));
    }
    let start: u64 = a.parse().ok()?;
    let end = if b.is_empty() {
        size.checked_sub(1)?
    } else {
        b.parse::<u64>().ok()?.min(size.checked_sub(1)?)
    };
    if start > end {
        return None;
    }
    Some((start, end))
}

async fn serve_file(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
    Query(q): Query<FileQuery>,
    headers: HeaderMap,
) -> Response<Body> {
    let stats = state.stats(&name);
    let req_no = stats.requests.fetch_add(1, Ordering::SeqCst) + 1;
    let version = state.version(&name);
    let seed = q
        .seed
        .unwrap_or_else(|| default_seed(&name))
        .wrapping_add(version);
    let size = q.size;

    if let Some(s) = q.status {
        return Response::builder()
            .status(StatusCode::from_u16(s).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
            .body(Body::from("forced status"))
            .unwrap();
    }
    if q.auth == Some(1) {
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("user:pass")
        );
        let ok = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            == Some(&expected);
        if !ok {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header(header::WWW_AUTHENTICATE, "Basic realm=\"test\"")
                .body(Body::from("auth required"))
                .unwrap();
        }
    }
    if let Some(c) = &q.cookie {
        let ok = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains(c.as_str()));
        if !ok {
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::from("cookie required"))
                .unwrap();
        }
    }
    if q.expire_after.is_some_and(|k| req_no > k) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from("link expired"))
            .unwrap();
    }
    if q.html_after.is_some_and(|k| req_no > k) {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from("<html><body>Please log in</body></html>"))
            .unwrap();
    }

    // Connection limit.
    let active = stats.active.fetch_add(1, Ordering::SeqCst) + 1;
    let guard = ActiveGuard(stats.clone());
    if q.maxconn.is_some_and(|m| active > m) {
        stats.rejected.fetch_add(1, Ordering::SeqCst);
        drop(guard);
        return Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(header::RETRY_AFTER, "1")
            .body(Body::from("too many connections"))
            .unwrap();
    }
    stats.max_active.fetch_max(active, Ordering::SeqCst);

    let etag = if q.noetag == Some(1) {
        None
    } else {
        Some(format!("\"{seed:x}-{size}\""))
    };
    let last_modified = httpdate::fmt_http_date(
        std::time::UNIX_EPOCH + Duration::from_secs(LAST_MODIFIED_BASE + version * 3600),
    );

    let ranges_enabled = q.norange != Some(1) && q.nolength != Some(1);
    let mut range = None;
    if ranges_enabled {
        if let Some(r) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
            stats.range_requests.fetch_add(1, Ordering::SeqCst);
            // If-Range: only honour the range when the validator matches.
            let if_range_ok = match headers.get(header::IF_RANGE).and_then(|v| v.to_str().ok()) {
                None => true,
                Some(v) if v.starts_with('"') || v.starts_with("W/") => Some(v) == etag.as_deref(),
                Some(v) => v == last_modified,
            };
            if if_range_ok {
                match parse_range(r, size) {
                    Some(rg) => range = Some(rg),
                    None => {
                        drop(guard);
                        return Response::builder()
                            .status(StatusCode::RANGE_NOT_SATISFIABLE)
                            .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                            .body(Body::empty())
                            .unwrap();
                    }
                }
            }
        }
    }

    let (status, start, end_incl) = match range {
        Some((s, e)) => (StatusCode::PARTIAL_CONTENT, s, e),
        None => (StatusCode::OK, 0, size.saturating_sub(1)),
    };
    let body_len = if size == 0 { 0 } else { end_incl - start + 1 };

    let mut builder = Response::builder().status(status).header(
        header::CONTENT_TYPE,
        q.mime
            .clone()
            .unwrap_or_else(|| "application/octet-stream".into()),
    );
    if ranges_enabled {
        builder = builder.header(header::ACCEPT_RANGES, "bytes");
    }
    if let Some(e) = &etag {
        builder = builder.header(header::ETAG, e.as_str());
    }
    builder = builder.header(header::LAST_MODIFIED, last_modified.as_str());
    if let Some(cd) = &q.cd {
        let encoded: String = cd
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b".-_".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        builder = builder.header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!(
                "attachment; filename=\"fallback.bin\"; filename*=UTF-8''{encoded}"
            ))
            .unwrap(),
        );
    }
    if status == StatusCode::PARTIAL_CONTENT {
        let reported_start = if q.malformed == Some(1) {
            start + 1
        } else {
            start
        };
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {reported_start}-{end_incl}/{size}"),
        );
    }
    if q.nolength != Some(1) {
        builder = builder.header(header::CONTENT_LENGTH, body_len);
    }

    let fail_after = match (q.fail_after, q.fail_times) {
        (Some(n), Some(k)) if req_no <= k => Some(n),
        (Some(n), None) => Some(n),
        _ => None,
    };
    let rate = q.rate.filter(|r| *r > 0);
    let chunk_size: u64 = match rate {
        Some(r) => (r / 20).clamp(1024, 64 * 1024),
        None => 64 * 1024,
    };

    struct St {
        pos: u64,
        end: u64,
        sent: u64,
        seed: u64,
        chunk: u64,
        rate: Option<u64>,
        fail_after: Option<u64>,
        stall_after: Option<u64>,
        stats: Arc<ResourceStats>,
        _guard: ActiveGuard,
        failed: bool,
    }
    let st = St {
        pos: start,
        end: start + body_len,
        sent: 0,
        seed,
        chunk: chunk_size,
        rate,
        fail_after,
        stall_after: match q.stall_times {
            Some(k) if req_no > k => None,
            _ => q.stall_after,
        },
        stats: stats.clone(),
        _guard: guard,
        failed: false,
    };
    let stream = futures::stream::unfold(st, |mut st| async move {
        if st.failed || st.pos >= st.end {
            return None;
        }
        if let Some(limit) = st.stall_after {
            if st.sent >= limit {
                futures::future::pending::<()>().await;
            }
        }
        if let Some(limit) = st.fail_after {
            if st.sent >= limit {
                st.failed = true;
                return Some((
                    Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "simulated failure",
                    )),
                    st,
                ));
            }
        }
        let mut n = st.chunk.min(st.end - st.pos);
        if let Some(limit) = st.fail_after {
            n = n.min(limit - st.sent);
        }
        if let Some(limit) = st.stall_after {
            n = n
                .min(limit - st.sent)
                .max(if limit > st.sent { 1 } else { 0 });
        }
        if let Some(rate) = st.rate {
            tokio::time::sleep(Duration::from_secs_f64(n as f64 / rate as f64)).await;
        }
        let mut buf = vec![0u8; n as usize];
        fill(st.seed, st.pos, &mut buf);
        st.pos += n;
        st.sent += n;
        st.stats.bytes_sent.fetch_add(n, Ordering::Relaxed);
        Some((Ok::<Bytes, io::Error>(Bytes::from(buf)), st))
    });
    builder.body(Body::from_stream(stream)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 10), Some((0, 9)));
        assert_eq!(parse_range("bytes=2-4", 10), Some((2, 4)));
        assert_eq!(parse_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(parse_range("bytes=2-100", 10), Some((2, 9)));
        assert_eq!(parse_range("bytes=11-", 10), None);
        assert_eq!(parse_range("bytes=0-1,3-4", 10), None);
    }

    #[test]
    fn content_is_deterministic() {
        let a = expected_content(7, 1000);
        let mut b = vec![0u8; 500];
        fill(7, 500, &mut b);
        assert_eq!(&a[500..], &b[..]);
    }
}
