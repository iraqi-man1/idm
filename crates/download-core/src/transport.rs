//! Protocol abstraction used by the segmented file task.
//!
//! The task only needs two operations from a protocol: a *probe* that
//! returns metadata plus a body starting at a given offset, and *open* for
//! additional byte ranges. HTTP(S) is implemented here; FTP/SFTP plug in
//! through the same enum.

use std::pin::Pin;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;
use velox_http::{ClientOptions, HttpError, RangeSupport, RequestContext, Validators};

use crate::error::{EngineError, EngineResult};

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, EngineError>> + Send>>;

/// Metadata and first body of a download.
pub struct ProbeInfo {
    pub status: u16,
    pub final_url: String,
    pub ranges: RangeSupport,
    pub total_size: Option<u64>,
    pub body_start: u64,
    pub disposition_name: Option<String>,
    pub mime: Option<String>,
    pub validators: Validators,
    pub validator_matched: bool,
    pub entity_changed: bool,
    pub body: Option<ByteStream>,
}

pub enum Transport {
    Http(HttpTransport),
    /// FTP, FTPS or SFTP.
    Remote(RemoteTransport),
}

pub struct RemoteTransport {
    url: String,
    remote: velox_ftp::Remote,
}

impl RemoteTransport {
    pub fn new(
        url: &str,
        credentials: Option<&velox_types::Credentials>,
        cfg: velox_ftp::RemoteConfig,
    ) -> EngineResult<Self> {
        Ok(Self {
            url: url.to_string(),
            remote: velox_ftp::Remote::new(url, credentials, cfg)?,
        })
    }

    async fn probe(&self, from: u64, validators: &Validators) -> EngineResult<ProbeInfo> {
        let o = self.remote.open(from, None).await?;
        let remote = Validators {
            etag: None,
            last_modified: o.info.modified.clone(),
        };
        let changed = from > 0 && !validators.is_empty() && validators.conflicts_with(&remote);
        Ok(ProbeInfo {
            status: if from > 0 && o.at_offset { 206 } else { 200 },
            final_url: self.url.clone(),
            ranges: if o.info.resumable {
                RangeSupport::Yes
            } else {
                RangeSupport::No
            },
            total_size: o.info.size,
            body_start: if o.at_offset { from } else { 0 },
            disposition_name: None,
            mime: None,
            validators: remote,
            validator_matched: from > 0 && o.at_offset && !changed,
            entity_changed: changed,
            body: Some(map_remote(o.body)),
        })
    }

    async fn open(
        &self,
        start: u64,
        end: Option<u64>,
        total: Option<u64>,
        validators: &Validators,
    ) -> EngineResult<ByteStream> {
        let o = match self.remote.open(start, end.map(|e| e - start)).await {
            Ok(o) => o,
            // An extra connection that cannot start at an offset.
            Err(velox_ftp::FtpError::NoResume(_)) => return Err(HttpError::RangeIgnored.into()),
            Err(e) => return Err(e.into()),
        };
        if !o.at_offset {
            return Err(HttpError::RangeIgnored.into());
        }
        let size_changed = total.is_some() && o.info.size.is_some() && o.info.size != total;
        let date_changed = validators.last_modified.is_some()
            && o.info.modified.is_some()
            && validators.last_modified != o.info.modified;
        if size_changed || date_changed {
            return Err(EngineError::RemoteChanged);
        }
        Ok(map_remote(o.body))
    }
}

fn map_remote(s: velox_ftp::ByteStream) -> ByteStream {
    Box::pin(s.map(|r| r.map_err(EngineError::from)))
}

pub struct HttpTransport {
    ctx: RequestContext,
    opts: ClientOptions,
    /// One client per connection slot: separate TCP connections.
    clients: Mutex<Vec<Option<velox_http::reqwest::Client>>>,
}

impl HttpTransport {
    pub fn new(ctx: RequestContext, opts: ClientOptions) -> Self {
        Self {
            ctx,
            opts,
            clients: Mutex::new(Vec::new()),
        }
    }

    fn client(&self, slot: usize) -> EngineResult<velox_http::reqwest::Client> {
        let mut clients = self.clients.lock();
        if clients.len() <= slot {
            clients.resize(slot + 1, None);
        }
        if let Some(c) = &clients[slot] {
            return Ok(c.clone());
        }
        let c = velox_http::build_client(&self.opts)?;
        clients[slot] = Some(c.clone());
        Ok(c)
    }
}

fn map_stream(s: velox_http::ByteStream) -> ByteStream {
    Box::pin(s.map(|r| r.map_err(EngineError::from)))
}

impl Transport {
    pub async fn probe(
        &self,
        slot: usize,
        from: u64,
        validators: &Validators,
    ) -> EngineResult<ProbeInfo> {
        match self {
            Transport::Http(t) => {
                let client = t.client(slot)?;
                let p = velox_http::probe(&client, &t.ctx, from, validators).await?;
                Ok(ProbeInfo {
                    status: p.status,
                    final_url: p.final_url,
                    ranges: p.ranges,
                    total_size: p.total_size,
                    body_start: p.body_start,
                    disposition_name: p.disposition_name,
                    mime: p.mime,
                    validators: p.validators,
                    validator_matched: p.validator_matched,
                    entity_changed: p.entity_changed,
                    body: p.body.map(map_stream),
                })
            }
            Transport::Remote(t) => t.probe(from, validators).await,
        }
    }

    /// Single-connection fallback without range requests.
    pub async fn probe_plain(&self, slot: usize) -> EngineResult<ProbeInfo> {
        match self {
            Transport::Http(t) => {
                let client = t.client(slot)?;
                let p = velox_http::probe_plain(&client, &t.ctx).await?;
                Ok(ProbeInfo {
                    status: p.status,
                    final_url: p.final_url,
                    ranges: p.ranges,
                    total_size: p.total_size,
                    body_start: 0,
                    disposition_name: p.disposition_name,
                    mime: p.mime,
                    validators: p.validators,
                    validator_matched: false,
                    entity_changed: false,
                    body: p.body.map(map_stream),
                })
            }
            Transport::Remote(t) => t.probe(0, &Validators::default()).await,
        }
    }

    pub async fn open(
        &self,
        slot: usize,
        start: u64,
        end: Option<u64>,
        total: Option<u64>,
        validators: &Validators,
    ) -> EngineResult<ByteStream> {
        match self {
            Transport::Http(t) => {
                let client = t.client(slot)?;
                let s =
                    velox_http::open_range(&client, &t.ctx, start, end, total, validators).await?;
                Ok(map_stream(s))
            }
            Transport::Remote(t) => t.open(start, end, total, validators).await,
        }
    }

    /// For servers that answered `bytes=0-` with 200 but advertise ranges:
    /// request one byte from the middle and see whether a 206 comes back.
    pub async fn verify_ranges(
        &self,
        slot: usize,
        total: u64,
        validators: &Validators,
    ) -> EngineResult<bool> {
        if total < 2 {
            return Ok(false);
        }
        let mid = total / 2;
        match self
            .open(slot, mid, Some(mid + 1), Some(total), validators)
            .await
        {
            Ok(mut s) => {
                // Drain the single byte so the connection can be reused.
                while let Some(chunk) = s.next().await {
                    chunk?;
                }
                Ok(true)
            }
            Err(EngineError::Http(HttpError::RangeIgnored)) => Ok(false),
            Err(EngineError::Http(HttpError::BadContentRange(_))) => Ok(false),
            Err(e) => Err(e),
        }
    }
}
