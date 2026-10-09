//! HTTP(S) transport for the Velox download engine.
//!
//! This crate knows how to talk to HTTP servers; it knows nothing about
//! segments, files or the database. The download engine (`velox-core`)
//! drives it through three operations:
//!
//! * [`probe`] – the first request of a download. It is a `GET` with
//!   `Range: bytes=N-`, so the same response both reveals the size / range
//!   support / validators *and* serves as the first connection's body.
//! * [`open_range`] – additional connections, each validated against the
//!   size and validators learned from the probe (`If-Range`), so a changed
//!   remote file can never be silently mixed into the partial file.
//! * header helpers ([`headers`]) for file names, content ranges and MIME.
//!
//! TLS uses rustls with the operating system's certificate store
//! (`rustls-platform-verifier`); certificate verification is never disabled.

pub mod headers;

use std::pin::Pin;
use std::sync::Once;
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Proxy, StatusCode};
use url::Url;
use velox_types::{Credentials, HeaderPair, ProxyMode, ProxySettings};

pub use reqwest;
pub use url;

/// Stream of body chunks.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, HttpError>> + Send>>;

static CRYPTO_INIT: Once = Once::new();

/// Install the process-wide rustls crypto provider (ring). Idempotent.
pub fn init_crypto() {
    CRYPTO_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum HttpError {
    #[error("network error: {0}")]
    Network(String),
    #[error("connection timed out")]
    Timeout,
    #[error("server responded with HTTP {status}")]
    Status {
        status: u16,
        retry_after: Option<u64>,
    },
    #[error("the server ignored the byte-range request")]
    RangeIgnored,
    #[error("the server sent an inconsistent Content-Range: {0}")]
    BadContentRange(String),
    #[error("the remote file changed since the download started")]
    RemoteChanged,
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("{0}")]
    Other(String),
}

impl HttpError {
    /// Worth retrying after a delay.
    pub fn is_transient(&self) -> bool {
        match self {
            HttpError::Network(_) | HttpError::Timeout => true,
            HttpError::Status { status, .. } => {
                matches!(
                    status,
                    408 | 425 | 429 | 500 | 502 | 503 | 504 | 509 | 520..=530
                )
            }
            _ => false,
        }
    }

    /// The server is refusing additional parallel connections.
    pub fn is_connection_limit(&self) -> bool {
        matches!(
            self,
            HttpError::Status {
                status: 429 | 503 | 509,
                ..
            }
        )
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            HttpError::Status { status, .. } => Some(*status),
            _ => None,
        }
    }

    fn from_reqwest(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            return HttpError::Timeout;
        }
        if e.is_redirect() {
            return HttpError::TooManyRedirects;
        }
        if e.is_builder() {
            return HttpError::InvalidUrl(e.to_string());
        }
        // Walk the source chain to detect certificate problems, which must
        // not be retried blindly.
        let mut src: Option<&dyn std::error::Error> = Some(&e);
        let mut detail = String::new();
        while let Some(s) = src {
            let msg = s.to_string();
            if msg.contains("certificate")
                || msg.contains("InvalidCertificate")
                || msg.contains("UnknownIssuer")
            {
                return HttpError::Tls(msg);
            }
            detail = msg;
            src = s.source();
        }
        HttpError::Network(if detail.is_empty() {
            e.to_string()
        } else {
            detail
        })
    }
}

impl From<reqwest::Error> for HttpError {
    fn from(e: reqwest::Error) -> Self {
        HttpError::from_reqwest(e)
    }
}

/// Settings for building HTTP clients.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub user_agent: String,
    pub connect_timeout: Duration,
    /// Maximum idle time between two body reads.
    pub read_timeout: Duration,
    pub max_redirects: u32,
    pub proxy: ProxySettings,
    pub proxy_password: Option<String>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            user_agent: velox_types::DEFAULT_USER_AGENT.into(),
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(60),
            max_redirects: 10,
            proxy: ProxySettings::default(),
            proxy_password: None,
        }
    }
}

/// Build a client. Each connection of a segmented download uses its own
/// client so that parallel segments really travel over separate TCP
/// connections (HTTP/2 would otherwise multiplex them over one).
pub fn build_client(opts: &ClientOptions) -> Result<Client, HttpError> {
    init_crypto();
    let mut b = Client::builder()
        .user_agent(opts.user_agent.clone())
        .connect_timeout(opts.connect_timeout)
        .read_timeout(opts.read_timeout)
        .redirect(reqwest::redirect::Policy::limited(
            opts.max_redirects as usize,
        ))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(30))
        .https_only(false);
    match opts.proxy.mode {
        ProxyMode::System => {}
        ProxyMode::None => b = b.no_proxy(),
        ProxyMode::Http | ProxyMode::Socks5 => {
            let scheme = if opts.proxy.mode == ProxyMode::Http {
                "http"
            } else {
                "socks5h"
            };
            let host = opts.proxy.host.trim();
            if host.is_empty() {
                return Err(HttpError::InvalidUrl("proxy host is empty".into()));
            }
            let mut p = Proxy::all(format!("{scheme}://{host}:{}", opts.proxy.port))
                .map_err(|e| HttpError::InvalidUrl(format!("proxy: {e}")))?;
            if !opts.proxy.username.is_empty() {
                p = p.basic_auth(
                    &opts.proxy.username,
                    opts.proxy_password.as_deref().unwrap_or(""),
                );
            }
            if !opts.proxy.bypass.trim().is_empty() {
                p = p.no_proxy(reqwest::NoProxy::from_string(&opts.proxy.bypass));
            }
            b = b.proxy(p);
        }
    }
    b.build()
        .map_err(|e| HttpError::Other(format!("cannot build HTTP client: {e}")))
}

/// Everything needed to issue a request for a download.
#[derive(Clone, Default)]
pub struct RequestContext {
    pub url: String,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub headers: Vec<HeaderPair>,
    pub cookies: Option<String>,
    pub credentials: Option<Credentials>,
}

impl std::fmt::Debug for RequestContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestContext")
            .field("url", &self.url)
            .field("referer", &self.referer)
            .field(
                "headers",
                &self.headers.iter().map(|h| &h.name).collect::<Vec<_>>(),
            )
            .field("cookies", &self.cookies.as_ref().map(|_| "<redacted>"))
            .field("credentials", &self.credentials)
            .finish()
    }
}

impl RequestContext {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    fn parsed_url(&self) -> Result<Url, HttpError> {
        let u = Url::parse(self.url.trim()).map_err(|e| HttpError::InvalidUrl(e.to_string()))?;
        match u.scheme() {
            "http" | "https" => Ok(u),
            s => Err(HttpError::InvalidUrl(format!("unsupported scheme {s}"))),
        }
    }

    fn header_map(&self) -> Result<HeaderMap, HttpError> {
        let mut h = HeaderMap::new();
        // Downloads must be byte-exact: never let the server compress.
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        h.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        for pair in &self.headers {
            let name = HeaderName::from_bytes(pair.name.as_bytes())
                .map_err(|_| HttpError::Other(format!("invalid header name {}", pair.name)))?;
            let lower = name.as_str();
            // The engine controls these itself.
            if matches!(
                lower,
                "range" | "if-range" | "accept-encoding" | "host" | "content-length"
            ) {
                continue;
            }
            let value = HeaderValue::from_str(&pair.value)
                .map_err(|_| HttpError::Other(format!("invalid value for header {}", pair.name)))?;
            h.insert(name, value);
        }
        if let Some(r) = self.referer.as_deref().filter(|r| !r.is_empty()) {
            if let Ok(v) = HeaderValue::from_str(r) {
                h.insert(header::REFERER, v);
            }
        }
        if let Some(ua) = self.user_agent.as_deref().filter(|u| !u.is_empty()) {
            if let Ok(v) = HeaderValue::from_str(ua) {
                h.insert(header::USER_AGENT, v);
            }
        }
        if let Some(c) = self.cookies.as_deref().filter(|c| !c.is_empty()) {
            let v = HeaderValue::from_str(c)
                .map_err(|_| HttpError::Other("invalid cookie value".into()))?;
            h.insert(header::COOKIE, v);
        }
        Ok(h)
    }

    fn request(&self, client: &Client) -> Result<reqwest::RequestBuilder, HttpError> {
        let url = self.parsed_url()?;
        let mut rb = client.get(url).headers(self.header_map()?);
        if let Some(c) = &self.credentials {
            rb = rb.basic_auth(&c.username, Some(&c.password));
        }
        Ok(rb)
    }
}

/// Entity validators used to detect remote changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validators {
    /// Value for `If-Range`. Weak ETags cannot be used with `If-Range`.
    pub fn if_range(&self) -> Option<&str> {
        match &self.etag {
            Some(e) if !e.starts_with("W/") => Some(e.as_str()),
            _ => self.last_modified.as_deref(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none()
    }

    fn from_headers(h: &HeaderMap) -> Self {
        let get = |n: HeaderName| {
            h.get(n)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
        };
        Self {
            etag: get(header::ETAG),
            last_modified: get(header::LAST_MODIFIED),
        }
    }

    /// Two validator sets describe a different entity.
    pub fn conflicts_with(&self, other: &Validators) -> bool {
        if let (Some(a), Some(b)) = (&self.etag, &other.etag) {
            return a.trim_start_matches("W/") != b.trim_start_matches("W/");
        }
        if let (Some(a), Some(b)) = (&self.last_modified, &other.last_modified) {
            return a != b;
        }
        false
    }
}

/// Whether the server honours byte ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSupport {
    /// The server answered 206 to a range request.
    Yes,
    /// The server answered 200 but advertises `Accept-Ranges: bytes`; some
    /// servers answer `bytes=0-` with 200. Verified by the next connection.
    Maybe,
    No,
}

/// Result of the first request of a download.
pub struct Probe {
    pub status: u16,
    pub final_url: String,
    pub ranges: RangeSupport,
    /// Total entity size, when known.
    pub total_size: Option<u64>,
    /// File offset of the first body byte.
    pub body_start: u64,
    /// Name suggested by `Content-Disposition`.
    pub disposition_name: Option<String>,
    pub mime: Option<String>,
    pub validators: Validators,
    /// `If-Range` was sent and the server confirmed the entity is unchanged.
    pub validator_matched: bool,
    /// `If-Range` was sent but the server sent the whole (changed) entity.
    pub entity_changed: bool,
    /// Response body; `None` when there is nothing to read (416 / empty).
    pub body: Option<ByteStream>,
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probe")
            .field("status", &self.status)
            .field("final_url", &self.final_url)
            .field("ranges", &self.ranges)
            .field("total_size", &self.total_size)
            .field("body_start", &self.body_start)
            .field("disposition_name", &self.disposition_name)
            .field("mime", &self.mime)
            .field("validators", &self.validators)
            .field("validator_matched", &self.validator_matched)
            .field("entity_changed", &self.entity_changed)
            .finish()
    }
}

fn retry_after(h: &HeaderMap) -> Option<u64> {
    let v = h
        .get(header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .to_string();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(secs);
    }
    let when = httpdate::parse_http_date(&v).ok()?;
    Some(
        when.duration_since(std::time::SystemTime::now())
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

fn status_error(resp: &reqwest::Response) -> HttpError {
    HttpError::Status {
        status: resp.status().as_u16(),
        retry_after: retry_after(resp.headers()),
    }
}

fn body_stream(resp: reqwest::Response) -> ByteStream {
    Box::pin(resp.bytes_stream().map(|r| r.map_err(HttpError::from)))
}

fn content_length(h: &HeaderMap) -> Option<u64> {
    h.get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Send the first request of a download, starting at `from`.
///
/// When `validators` is non-empty an `If-Range` header is sent so that a
/// changed remote file is detected instead of being appended to stale data.
pub async fn probe(
    client: &Client,
    ctx: &RequestContext,
    from: u64,
    validators: &Validators,
) -> Result<Probe, HttpError> {
    let mut rb = ctx
        .request(client)?
        .header(header::RANGE, format!("bytes={from}-"));
    let sent_if_range = if from > 0 {
        validators.if_range()
    } else {
        None
    };
    if let Some(v) = sent_if_range {
        rb = rb.header(header::IF_RANGE, v);
    }
    let resp = rb.send().await?;
    let status = resp.status();
    let h = resp.headers().clone();
    let final_url = resp.url().to_string();
    let mime = h
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());
    let disposition_name = h
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| headers::filename_from_content_disposition(v.as_bytes()));
    let resp_validators = Validators::from_headers(&h);

    match status {
        StatusCode::PARTIAL_CONTENT => {
            let cr = h
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| HttpError::BadContentRange("missing Content-Range on 206".into()))?;
            let cr = headers::parse_content_range(cr)
                .ok_or_else(|| HttpError::BadContentRange(cr.to_string()))?;
            let (start, _end) = cr
                .range
                .ok_or_else(|| HttpError::BadContentRange("unsatisfied range on 206".into()))?;
            if start != from {
                return Err(HttpError::BadContentRange(format!(
                    "asked for offset {from}, got {start}"
                )));
            }
            Ok(Probe {
                status: 206,
                final_url,
                ranges: RangeSupport::Yes,
                total_size: cr.total,
                body_start: start,
                disposition_name,
                mime,
                validators: resp_validators,
                validator_matched: sent_if_range.is_some(),
                entity_changed: false,
                body: Some(body_stream(resp)),
            })
        }
        StatusCode::OK => {
            let accept_ranges = h
                .get(header::ACCEPT_RANGES)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.to_ascii_lowercase().contains("bytes"));
            let entity_changed =
                sent_if_range.is_some() && validators.conflicts_with(&resp_validators);
            let ranges = if from == 0 && accept_ranges {
                RangeSupport::Maybe
            } else {
                RangeSupport::No
            };
            Ok(Probe {
                status: 200,
                final_url,
                ranges,
                total_size: content_length(&h),
                body_start: 0,
                disposition_name,
                mime,
                validators: resp_validators,
                validator_matched: false,
                entity_changed,
                body: Some(body_stream(resp)),
            })
        }
        StatusCode::RANGE_NOT_SATISFIABLE => {
            // `bytes */N`: the range starts at or after the end of the entity.
            let total = h
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(headers::parse_content_range)
                .and_then(|cr| cr.total);
            match total {
                Some(t) if t == from => Ok(Probe {
                    status: 416,
                    final_url,
                    ranges: RangeSupport::Yes,
                    total_size: Some(t),
                    body_start: from,
                    disposition_name,
                    mime,
                    validators: resp_validators,
                    validator_matched: sent_if_range.is_some(),
                    entity_changed: false,
                    body: None,
                }),
                Some(_) if from > 0 => Err(HttpError::RemoteChanged),
                _ => Err(status_error(&resp)),
            }
        }
        _ => Err(status_error(&resp)),
    }
}

/// A complete small response body (playlists, keys, media segments).
pub struct Fetched {
    pub body: Bytes,
    pub final_url: String,
    pub mime: Option<String>,
}

/// `GET` a resource (optionally an inclusive byte range) and read the whole
/// body, refusing bodies larger than `max_len`.
pub async fn fetch_bytes(
    client: &Client,
    ctx: &RequestContext,
    range: Option<(u64, u64)>,
    max_len: usize,
) -> Result<Fetched, HttpError> {
    let mut rb = ctx.request(client)?;
    if let Some((a, b)) = range {
        rb = rb.header(header::RANGE, format!("bytes={a}-{b}"));
    }
    let resp = rb.send().await?;
    let status = resp.status();
    if !(status.is_success()) {
        return Err(status_error(&resp));
    }
    if range.is_some() && status != StatusCode::PARTIAL_CONTENT {
        return Err(HttpError::RangeIgnored);
    }
    if content_length(resp.headers()).is_some_and(|l| l as usize > max_len) {
        return Err(HttpError::Other(format!(
            "response larger than {max_len} bytes"
        )));
    }
    let final_url = resp.url().to_string();
    let mime = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());
    let mut body = bytes::BytesMut::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len() + chunk.len() > max_len {
            return Err(HttpError::Other(format!(
                "response larger than {max_len} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Fetched {
        body: body.freeze(),
        final_url,
        mime,
    })
}

/// Plain `GET` without a `Range` header, for servers whose range responses
/// cannot be trusted. The download proceeds over a single connection.
pub async fn probe_plain(client: &Client, ctx: &RequestContext) -> Result<Probe, HttpError> {
    let resp = ctx.request(client)?.send().await?;
    if resp.status() != StatusCode::OK {
        return Err(status_error(&resp));
    }
    let h = resp.headers().clone();
    Ok(Probe {
        status: 200,
        final_url: resp.url().to_string(),
        ranges: RangeSupport::No,
        total_size: content_length(&h),
        body_start: 0,
        disposition_name: h
            .get(header::CONTENT_DISPOSITION)
            .and_then(|v| headers::filename_from_content_disposition(v.as_bytes())),
        mime: h
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string()),
        validators: Validators::from_headers(&h),
        validator_matched: false,
        entity_changed: false,
        body: Some(body_stream(resp)),
    })
}

/// Open an additional connection for `[start, end)`.
///
/// `expected_total` is the size learned from the probe; a response that
/// reports another size means the entity changed.
pub async fn open_range(
    client: &Client,
    ctx: &RequestContext,
    start: u64,
    end: Option<u64>,
    expected_total: Option<u64>,
    validators: &Validators,
) -> Result<ByteStream, HttpError> {
    let range = match end {
        Some(e) if e > start => format!("bytes={start}-{}", e - 1),
        _ => format!("bytes={start}-"),
    };
    let mut rb = ctx.request(client)?.header(header::RANGE, range);
    if let Some(v) = validators.if_range() {
        rb = rb.header(header::IF_RANGE, v);
    }
    let resp = rb.send().await?;
    match resp.status() {
        StatusCode::PARTIAL_CONTENT => {
            let raw = resp
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let cr = headers::parse_content_range(&raw)
                .ok_or_else(|| HttpError::BadContentRange(raw.clone()))?;
            let (s, _) = cr
                .range
                .ok_or_else(|| HttpError::BadContentRange(raw.clone()))?;
            if s != start {
                return Err(HttpError::BadContentRange(format!(
                    "asked for offset {start}, got {s}"
                )));
            }
            if let (Some(exp), Some(got)) = (expected_total, cr.total) {
                if exp != got {
                    return Err(HttpError::RemoteChanged);
                }
            }
            let got = Validators::from_headers(resp.headers());
            if validators.conflicts_with(&got) {
                return Err(HttpError::RemoteChanged);
            }
            Ok(body_stream(resp))
        }
        StatusCode::OK => {
            let got = Validators::from_headers(resp.headers());
            if validators.conflicts_with(&got) {
                Err(HttpError::RemoteChanged)
            } else {
                Err(HttpError::RangeIgnored)
            }
        }
        StatusCode::RANGE_NOT_SATISFIABLE => Err(HttpError::RemoteChanged),
        _ => Err(status_error(&resp)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn if_range_prefers_strong_etag() {
        let v = Validators {
            etag: Some("\"abc\"".into()),
            last_modified: Some("lm".into()),
        };
        assert_eq!(v.if_range(), Some("\"abc\""));
        let v = Validators {
            etag: Some("W/\"abc\"".into()),
            last_modified: Some("lm".into()),
        };
        assert_eq!(v.if_range(), Some("lm"));
        let v = Validators {
            etag: Some("W/\"abc\"".into()),
            last_modified: None,
        };
        assert_eq!(v.if_range(), None);
    }

    #[test]
    fn validators_conflict() {
        let a = Validators {
            etag: Some("\"1\"".into()),
            last_modified: None,
        };
        let b = Validators {
            etag: Some("\"2\"".into()),
            last_modified: None,
        };
        assert!(a.conflicts_with(&b));
        assert!(!a.conflicts_with(&a));
        assert!(!a.conflicts_with(&Validators::default()));
    }

    #[test]
    fn transient_classification() {
        assert!(HttpError::Timeout.is_transient());
        assert!(HttpError::Status {
            status: 503,
            retry_after: None
        }
        .is_transient());
        assert!(!HttpError::Status {
            status: 404,
            retry_after: None
        }
        .is_transient());
        assert!(!HttpError::RemoteChanged.is_transient());
        assert!(HttpError::Status {
            status: 429,
            retry_after: None
        }
        .is_connection_limit());
    }
}
