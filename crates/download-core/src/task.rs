//! The segmented file-download task.
//!
//! One task runs per active download. It owns the transport, the segment
//! table, the disk writer and the connection workers, and reports progress
//! through a shared [`LiveState`].
//!
//! Flow of one attempt:
//! 1. Restore persisted segments (resume) or start fresh.
//! 2. Probe: the first `GET Range: bytes=N-` both validates the entity
//!    (`If-Range`, size) and becomes the first connection's body.
//! 3. Supervisor loop: spawns connections adaptively, each taking an
//!    unassigned segment or splitting the largest remainder; checkpoints
//!    (fsync, then SQLite) every few seconds; reacts to connection errors.
//! 4. Finalize: flush, verify size, rename the partial file, verify checksum.
//!
//! Transient failures end the attempt; the outer loop retries with
//! exponential backoff and resumes from the last durable checkpoint.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use parking_lot::Mutex;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use velox_http::{headers, ClientOptions, HttpError, RangeSupport, RequestContext, Validators};
use velox_persistence::{Database, DownloadRecord, DownloadSecrets};
use velox_segments::{Assignment, SegmentTable, WorkerId};
use velox_types::{
    AppSettings, Category, ConflictPolicy, DownloadKind, DownloadStatus, SegmentProgress,
};

use crate::checksum;
use crate::error::{EngineError, EngineResult};
use crate::fsutil;
use crate::naming;
use crate::ratelimit::RateLimiter;
use crate::speed::SpeedMeter;
use crate::transport::{ByteStream, HttpTransport, Transport};
use crate::writer::{DiskWriter, WriteCmd};

/// Why a task was asked to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopReason {
    #[default]
    None,
    Pause,
    Cancel,
    Remove,
    /// Application exit: behaves like pause but keeps the "was running" flag.
    Shutdown,
}

/// Live, non-persistent state of a running task.
#[derive(Debug, Clone, Default)]
pub struct LiveState {
    pub status: Option<DownloadStatus>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed: u64,
    pub avg_speed: u64,
    pub eta_secs: Option<u64>,
    pub active_connections: u8,
    pub segments: Vec<SegmentProgress>,
    pub stage: Option<String>,
    pub elapsed_ms: u64,
    /// One sample per second, most recent last (bytes/second).
    pub history: VecDeque<u64>,
}

const HISTORY_LEN: usize = 300;

/// State shared between the manager and a running task.
pub struct TaskShared {
    pub record: Arc<Mutex<DownloadRecord>>,
    pub live: Arc<Mutex<LiveState>>,
    pub cancel: CancellationToken,
    pub stop: Mutex<StopReason>,
    /// Per-download speed limit (0 = unlimited), adjustable at runtime.
    pub limiter: Arc<RateLimiter>,
}

impl TaskShared {
    pub fn stop_reason(&self) -> StopReason {
        *self.stop.lock()
    }
}

/// Callbacks into the manager.
pub trait TaskHooks: Send + Sync {
    /// The persistent record changed (status, name, size...).
    fn record_changed(&self, record: &DownloadRecord);
    /// Append a line to the download's log.
    fn log(&self, record: &DownloadRecord, level: &str, message: &str);
}

/// Engine services available to a task.
#[derive(Clone)]
pub struct TaskEnv {
    pub db: Database,
    pub settings: AppSettings,
    pub client_opts: ClientOptions,
    pub global_limiter: Arc<RateLimiter>,
    pub secrets: DownloadSecrets,
    /// Final paths claimed by other downloads.
    pub reserved: Arc<Mutex<HashSet<PathBuf>>>,
    pub hooks: Arc<dyn TaskHooks>,
}

pub enum TaskOutcome {
    Completed,
    Stopped(StopReason),
    Failed(EngineError),
}

/// Interval between durable checkpoints.
const CHECKPOINT_EVERY: Duration = Duration::from_secs(3);
const TICK: Duration = Duration::from_millis(250);

fn build_transport(record: &DownloadRecord, env: &TaskEnv) -> EngineResult<Transport> {
    match record.kind {
        DownloadKind::Http => {
            let mut headers = record.headers.clone();
            headers.extend(env.secrets.headers.iter().cloned());
            let ctx = RequestContext {
                url: record.url.clone(),
                referer: record.referer.clone(),
                user_agent: record.user_agent.clone(),
                headers,
                cookies: env.secrets.cookies.clone(),
                credentials: env.secrets.credentials.clone(),
            };
            Ok(Transport::Http(HttpTransport::new(
                ctx,
                env.client_opts.clone(),
            )))
        }
        other => Err(EngineError::Unsupported(format!(
            "{} downloads are not handled by the file task",
            other.as_str()
        ))),
    }
}

/// Run a file download until it completes, fails or is stopped.
pub async fn run_file_task(sh: Arc<TaskShared>, env: TaskEnv) -> TaskOutcome {
    let max_retries = env.settings.downloads.max_retries;
    let base_delay = env.settings.downloads.retry_delay_secs.max(1) as u64;
    let mut failures: u32 = 0;
    loop {
        if sh.cancel.is_cancelled() {
            return TaskOutcome::Stopped(sh.stop_reason());
        }
        let before = sh.record.lock().downloaded;
        let res = attempt(&sh, &env).await;
        match res {
            Ok(()) => return TaskOutcome::Completed,
            Err(_) if sh.cancel.is_cancelled() => return TaskOutcome::Stopped(sh.stop_reason()),
            Err(EngineError::Cancelled) => return TaskOutcome::Stopped(sh.stop_reason()),
            Err(e) => {
                let after = sh.record.lock().downloaded;
                if after > before + 256 * 1024 {
                    failures = 0;
                }
                if e.is_transient() && failures < max_retries {
                    failures += 1;
                    let retry_after = match &e {
                        EngineError::Http(HttpError::Status {
                            retry_after: Some(s),
                            ..
                        }) => Some(*s),
                        _ => None,
                    };
                    let exp = base_delay.saturating_mul(1u64 << (failures - 1).min(5));
                    let delay = Duration::from_secs(retry_after.unwrap_or(exp).clamp(1, 120));
                    {
                        let mut r = sh.record.lock();
                        r.status = DownloadStatus::Retrying;
                        r.retry_count = failures;
                        r.error = Some(e.to_string());
                        r.error_kind = Some(e.kind());
                        let _ = env.db.save_download(&r);
                        env.hooks.log(
                            &r,
                            "warn",
                            &format!(
                                "{e}; retry {failures}/{max_retries} in {}s",
                                delay.as_secs()
                            ),
                        );
                        env.hooks.record_changed(&r);
                    }
                    {
                        let mut live = sh.live.lock();
                        live.status = Some(DownloadStatus::Retrying);
                        live.speed = 0;
                        live.active_connections = 0;
                        live.stage = Some(format!("Retrying in {}s", delay.as_secs()));
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = sh.cancel.cancelled() => return TaskOutcome::Stopped(sh.stop_reason()),
                    }
                    continue;
                }
                return TaskOutcome::Failed(e);
            }
        }
    }
}

fn set_status(sh: &TaskShared, env: &TaskEnv, status: DownloadStatus) {
    let mut r = sh.record.lock();
    if r.status != status {
        r.status = status;
        if status == DownloadStatus::Downloading {
            r.error = None;
            r.error_kind = None;
        }
        let _ = env.db.save_download(&r);
        env.hooks.record_changed(&r);
    }
    sh.live.lock().status = Some(status);
}

fn log(sh: &TaskShared, env: &TaskEnv, level: &str, msg: &str) {
    let r = sh.record.lock();
    env.hooks.log(&r, level, msg);
}

/// Pick the final file name for a fresh download.
fn choose_name(
    record: &DownloadRecord,
    probe_name: Option<&str>,
    final_url: &str,
    mime: Option<&str>,
) -> String {
    if record.name_locked && !record.file_name.is_empty() {
        return naming::sanitize_file_name(&record.file_name);
    }
    let raw = probe_name
        .map(str::to_string)
        .or_else(|| headers::filename_from_url(final_url))
        .or_else(|| headers::filename_from_url(&record.url))
        .unwrap_or_else(|| record.file_name.clone());
    let name = naming::sanitize_file_name(&raw);
    naming::ensure_extension(&name, mime)
}

/// Directory used for the partial file.
fn partial_location(env: &TaskEnv, final_path: &Path) -> PathBuf {
    let temp_dir = env.settings.downloads.temp_dir.trim();
    if temp_dir.is_empty() {
        naming::partial_path(final_path)
    } else {
        let name = final_path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        naming::partial_path(&Path::new(temp_dir).join(name))
    }
}

struct Prepared {
    table: SegmentTable,
    first_body: Option<ByteStream>,
    first_from: u64,
    temp: PathBuf,
    fresh: bool,
    validators: Validators,
}

async fn attempt(sh: &Arc<TaskShared>, env: &TaskEnv) -> EngineResult<()> {
    set_status(sh, env, DownloadStatus::Connecting);
    sh.live.lock().stage = Some("Connecting".into());
    let record = sh.record.lock().clone();
    let transport = Arc::new(build_transport(&record, env)?);
    let min_split = env.settings.downloads.min_segment_size;

    let prepared = match try_resume(sh, env, &record, &transport, min_split).await? {
        Some(p) => p,
        None => start_fresh(sh, env, &transport, min_split).await?,
    };
    let Prepared {
        table,
        first_body,
        first_from,
        temp,
        fresh,
        validators,
    } = prepared;

    // Open the partial file.
    let file = fsutil::open_partial(&temp).map_err(EngineError::from_io)?;
    if fresh {
        file.set_len(0).map_err(EngineError::from_io)?;
        if let Some(total) = table.total() {
            if env.settings.downloads.preallocate && total > 0 {
                fsutil::preallocate(&file, total).map_err(EngineError::from_io)?;
            }
        }
    } else {
        // The partial file must contain every byte the table claims.
        let max_written = table
            .segments()
            .iter()
            .map(|s| s.written)
            .max()
            .unwrap_or(0);
        let len = file.metadata().map_err(EngineError::from_io)?.len();
        if len < max_written && !(table.total().is_some() && len == 0 && max_written == 0) {
            return Err(EngineError::Io(format!(
                "partial file is shorter ({len} bytes) than recorded progress ({max_written} bytes)"
            )));
        }
    }

    {
        let mut r = sh.record.lock();
        r.temp_path = Some(temp.to_string_lossy().to_string());
        if r.started_at.is_none() {
            r.started_at = Some(velox_types::now_ms());
        }
        r.downloaded = table.written_bytes();
        let _ = env.db.save_download(&r);
    }

    let table = Arc::new(Mutex::new(table));
    let writer = DiskWriter::spawn(
        file,
        table.clone(),
        sh.record.lock().id.simple().to_string(),
    );
    if fresh {
        // Persist the empty table right away so a crash before the first
        // checkpoint is still recognised as a resumable download.
        let snap = writer.checkpoint().await?;
        let r = sh.record.lock().clone();
        env.db.checkpoint(&r, &snap)?;
    }
    set_status(sh, env, DownloadStatus::Downloading);

    let result = supervise(
        sh,
        env,
        transport,
        table.clone(),
        &writer,
        first_body,
        first_from,
        validators,
    )
    .await;

    match result {
        Ok(()) => finalize(sh, env, table, writer, &temp).await,
        Err(e) => {
            // Persist whatever reached the disk, then close the file.
            if writer.error().is_none() {
                if let Ok(snap) = writer.checkpoint().await {
                    persist_checkpoint(sh, env, &snap, &table, 0);
                }
            }
            let _ = writer.finish().await;
            Err(e)
        }
    }
}

/// Try to continue from persisted segments. `Ok(None)` means start fresh.
async fn try_resume(
    sh: &Arc<TaskShared>,
    env: &TaskEnv,
    record: &DownloadRecord,
    transport: &Transport,
    min_split: u64,
) -> EngineResult<Option<Prepared>> {
    let persisted = env.db.load_segments(record.id)?;
    let temp = record.temp_path.as_ref().map(PathBuf::from);
    let Some(temp) = temp else { return Ok(None) };
    if persisted.is_empty()
        || !temp.exists()
        || record.total_size.is_none()
        || record.resumable != Some(true)
    {
        if temp.exists() && record.resumable == Some(false) && record.downloaded > 0 {
            log(
                sh,
                env,
                "warn",
                "server does not support resuming; restarting from the beginning",
            );
        }
        return Ok(None);
    }
    let table = match SegmentTable::restore(record.total_size, &persisted, min_split, true) {
        Ok(t) => t,
        Err(e) => {
            log(
                sh,
                env,
                "warn",
                &format!("discarding inconsistent progress data: {e}"),
            );
            return Ok(None);
        }
    };
    let validators = Validators {
        etag: record.etag.clone(),
        last_modified: record.last_modified.clone(),
    };
    if table.is_complete() {
        return Ok(Some(Prepared {
            table,
            first_body: None,
            first_from: 0,
            temp,
            fresh: false,
            validators,
        }));
    }
    let from = table
        .segments()
        .iter()
        .filter(|s| !s.is_done())
        .map(|s| s.written)
        .min()
        .unwrap_or(0);

    let probe = match transport.probe(0, from, &validators).await {
        Ok(p) => p,
        Err(EngineError::Http(HttpError::Status { status, .. }))
            if matches!(status, 403 | 404 | 410) =>
        {
            return Err(EngineError::LinkExpired(format!("HTTP {status}")));
        }
        Err(EngineError::Http(HttpError::RemoteChanged)) => return Err(EngineError::RemoteChanged),
        Err(e) => return Err(e),
    };
    let expected_html = headers::is_html(record.mime.as_deref());
    if !expected_html && headers::is_html(probe.mime.as_deref()) && probe.status == 200 {
        return Err(EngineError::LinkExpired(
            "the server returned a web page instead of the file".into(),
        ));
    }
    match probe.status {
        206 => {
            if probe.total_size != record.total_size
                || (!validators.is_empty() && validators.conflicts_with(&probe.validators))
            {
                return Err(EngineError::RemoteChanged);
            }
            log(
                sh,
                env,
                "info",
                &format!("resuming at {from} bytes (server confirmed the file is unchanged)"),
            );
            Ok(Some(Prepared {
                table,
                first_body: probe.body,
                first_from: from,
                temp,
                fresh: false,
                validators,
            }))
        }
        200 => {
            if probe.entity_changed
                || probe
                    .total_size
                    .is_some_and(|t| Some(t) != record.total_size)
            {
                Err(EngineError::RemoteChanged)
            } else {
                Err(EngineError::ResumeUnsupported)
            }
        }
        _ => Err(EngineError::RemoteChanged),
    }
}

async fn start_fresh(
    sh: &Arc<TaskShared>,
    env: &TaskEnv,
    transport: &Transport,
    min_split: u64,
) -> EngineResult<Prepared> {
    // Remove stale partial data from an earlier non-resumable attempt.
    let old_temp = sh.record.lock().temp_path.clone();
    if let Some(t) = old_temp {
        let _ = std::fs::remove_file(&t);
    }
    env.db.clear_segments(sh.record.lock().id)?;

    let probe = match transport.probe(0, 0, &Validators::default()).await {
        Err(EngineError::Http(HttpError::BadContentRange(detail))) => {
            log(sh, env, "warn", &format!(
                "server sent an invalid Content-Range ({detail}); falling back to a single plain connection"
            ));
            transport.probe_plain(0).await?
        }
        other => other?,
    };
    let mut total = probe.total_size;
    if probe.status == 416 {
        total = Some(0);
    }
    let ranges = match probe.ranges {
        RangeSupport::Yes => true,
        RangeSupport::No => false,
        RangeSupport::Maybe => match total {
            Some(t) => {
                let ok = transport
                    .verify_ranges(1, t, &probe.validators)
                    .await
                    .unwrap_or(false);
                log(
                    sh,
                    env,
                    "info",
                    &format!(
                        "server answered 200 to a range request; range support {}",
                        if ok { "confirmed" } else { "not available" }
                    ),
                );
                ok
            }
            None => false,
        },
    };
    let resumable = ranges && total.is_some();

    let (final_path, temp) = {
        let mut r = sh.record.lock();
        let name = choose_name(
            &r,
            probe.disposition_name.as_deref(),
            &probe.final_url,
            probe.mime.as_deref(),
        );
        let dir = PathBuf::from(&r.save_dir);
        let mut reserved = env.reserved.lock();
        let previous = Path::new(&r.save_dir).join(&r.file_name);
        reserved.remove(&previous);
        let final_path = naming::unique_path(&dir, &name, &reserved);
        reserved.insert(final_path.clone());
        drop(reserved);
        r.file_name = final_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(name);
        if r.category == Category::Other || !r.name_locked {
            r.category = Category::detect(&r.file_name, probe.mime.as_deref());
        }
        r.total_size = total;
        r.resumable = Some(resumable);
        r.etag = probe.validators.etag.clone();
        r.last_modified = probe.validators.last_modified.clone();
        r.mime = probe.mime.clone();
        r.final_url = (probe.final_url != r.url).then(|| probe.final_url.clone());
        r.downloaded = 0;
        let temp = partial_location(env, &final_path);
        r.temp_path = Some(temp.to_string_lossy().to_string());
        let _ = env.db.save_download(&r);
        env.hooks.record_changed(&r);
        (final_path, temp)
    };
    log(
        sh,
        env,
        "info",
        &format!(
            "connected: size {}, ranges {}, saving as {}",
            total
                .map(|t| t.to_string())
                .unwrap_or_else(|| "unknown".into()),
            if resumable {
                "supported"
            } else {
                "not supported"
            },
            final_path.display()
        ),
    );

    // Disk space check.
    if let Some(t) = total {
        if let Ok(avail) = fsutil::available_space(&temp) {
            if avail < t {
                return Err(EngineError::DiskFull {
                    needed: t,
                    available: avail,
                });
            }
        }
    }

    let table = SegmentTable::new(total, min_split, resumable);
    Ok(Prepared {
        table,
        first_body: probe.body,
        first_from: 0,
        temp,
        fresh: true,
        validators: probe.validators,
    })
}

#[derive(Debug)]
enum WorkerExit {
    /// The segment was fully received.
    Done,
    /// End of stream on an open-ended segment.
    Eof,
}

struct WorkerCtx {
    transport: Arc<Transport>,
    table: Arc<Mutex<SegmentTable>>,
    tx: tokio::sync::mpsc::Sender<WriteCmd>,
    global_limiter: Arc<RateLimiter>,
    limiter: Arc<RateLimiter>,
    validators: Validators,
    cancel: CancellationToken,
}

async fn run_worker(
    ctx: Arc<WorkerCtx>,
    wid: WorkerId,
    slot: usize,
    a: Assignment,
    body: Option<ByteStream>,
    bytes: Arc<AtomicU64>,
) -> Result<WorkerExit, EngineError> {
    let res = worker_loop(&ctx, wid, slot, a, body, &bytes).await;
    ctx.table.lock().release(a.index, wid);
    res
}

async fn worker_loop(
    ctx: &WorkerCtx,
    wid: WorkerId,
    slot: usize,
    a: Assignment,
    body: Option<ByteStream>,
    bytes: &AtomicU64,
) -> Result<WorkerExit, EngineError> {
    let total = ctx.table.lock().total();
    let mut stream = match body {
        Some(b) => b,
        None => {
            let end = (a.end != velox_segments::OPEN_END).then_some(a.end);
            tokio::select! {
                biased;
                _ = ctx.cancel.cancelled() => return Err(EngineError::Cancelled),
                s = ctx.transport.open(slot, a.from, end, total, &ctx.validators) => s?,
            }
        }
    };
    loop {
        let next = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => return Err(EngineError::Cancelled),
            n = stream.next() => n,
        };
        let chunk = match next {
            None => {
                let t = ctx.table.lock();
                let seg = t.segment(a.index).expect("segment exists");
                return if seg.is_open_ended() {
                    Ok(WorkerExit::Eof)
                } else if seg.is_received() {
                    Ok(WorkerExit::Done)
                } else {
                    Err(EngineError::PrematureEof)
                };
            }
            Some(Err(e)) => return Err(e),
            Some(Ok(c)) => c,
        };
        if chunk.is_empty() {
            continue;
        }
        let len = chunk.len() as u64;
        let wait = ctx
            .global_limiter
            .reserve(len)
            .max(ctx.limiter.reserve(len));
        if !wait.is_zero() {
            tokio::select! {
                biased;
                _ = ctx.cancel.cancelled() => return Err(EngineError::Cancelled),
                _ = tokio::time::sleep(wait) => {}
            }
        }
        let claim = ctx
            .table
            .lock()
            .claim(a.index, wid, len)
            .map_err(|e| EngineError::Other(e.to_string()))?;
        if claim.accepted > 0 {
            let data = chunk.slice(..claim.accepted as usize);
            let cmd = WriteCmd::Data {
                index: a.index,
                offset: claim.offset,
                data,
            };
            tokio::select! {
                biased;
                _ = ctx.cancel.cancelled() => return Err(EngineError::Cancelled),
                r = ctx.tx.send(cmd) => r.map_err(|_| EngineError::Io("disk writer stopped".into()))?,
            }
            bytes.fetch_add(claim.accepted, Ordering::Relaxed);
        }
        if claim.finished {
            return Ok(WorkerExit::Done);
        }
    }
}

struct WorkerInfo {
    slot: usize,
    index: usize,
    bytes: Arc<AtomicU64>,
    meter: SpeedMeter,
}

/// Adaptive connection controller.
///
/// Connections are added one at a time. After each addition the aggregate
/// throughput is compared with the throughput before it; when additions
/// repeatedly bring less than 5 % improvement the level is frozen (a
/// higher level is re-tested periodically). Servers that reject extra
/// connections (429/503/403 while other connections work) cap the level.
struct Controller {
    max: usize,
    level: usize,
    adaptive: bool,
    server_cap: Option<usize>,
    last_change: Instant,
    speed_before: u64,
    strikes: u32,
    frozen_until: Option<Instant>,
    respawn_after: Instant,
}

impl Controller {
    fn new(max: usize, adaptive: bool) -> Self {
        let level = if adaptive { max.min(4) } else { max };
        Self {
            max,
            level: level.max(1),
            adaptive,
            server_cap: None,
            last_change: Instant::now(),
            speed_before: 0,
            strikes: 0,
            frozen_until: None,
            respawn_after: Instant::now(),
        }
    }

    fn cap(&self) -> usize {
        self.server_cap.unwrap_or(self.max).min(self.max).max(1)
    }

    fn target(&self) -> usize {
        self.level.min(self.cap())
    }

    fn on_tick(&mut self, now: Instant, active: usize, speed_2s: u64, can_split: bool) {
        if !self.adaptive || !can_split || self.level >= self.cap() || active < self.target() {
            return;
        }
        if let Some(until) = self.frozen_until {
            if now < until {
                return;
            }
            // Re-test one more connection.
            self.frozen_until = None;
            self.strikes = 2;
        }
        if now.duration_since(self.last_change) < Duration::from_millis(1500) {
            return;
        }
        if self.speed_before > 0 {
            if speed_2s * 100 >= self.speed_before * 105 {
                self.strikes = 0;
            } else {
                self.strikes += 1;
            }
        }
        if self.strikes >= 3 {
            self.frozen_until = Some(now + Duration::from_secs(20));
            self.strikes = 0;
            return;
        }
        self.level += 1;
        self.speed_before = speed_2s;
        self.last_change = now;
    }

    fn limit_by_server(&mut self, working: usize) {
        let cap = working.max(1);
        self.server_cap = Some(self.server_cap.map_or(cap, |c| c.min(cap)));
        self.level = self.level.min(cap);
    }
}

#[allow(clippy::too_many_arguments)]
async fn supervise(
    sh: &Arc<TaskShared>,
    env: &TaskEnv,
    transport: Arc<Transport>,
    table: Arc<Mutex<SegmentTable>>,
    writer: &DiskWriter,
    first_body: Option<ByteStream>,
    first_from: u64,
    validators: Validators,
) -> EngineResult<()> {
    let worker_cancel = sh.cancel.child_token();
    let ctx = Arc::new(WorkerCtx {
        transport,
        table: table.clone(),
        tx: writer.sender(),
        global_limiter: env.global_limiter.clone(),
        limiter: sh.limiter.clone(),
        validators,
        cancel: worker_cancel.clone(),
    });
    let max_conn = sh.record.lock().max_connections.clamp(1, 32) as usize;
    let mut ctl = Controller::new(max_conn, env.settings.downloads.adaptive_connections);

    let mut join: JoinSet<(WorkerId, Result<WorkerExit, EngineError>)> = JoinSet::new();
    let mut workers: HashMap<WorkerId, WorkerInfo> = HashMap::new();
    let mut next_wid: WorkerId = 0;
    let mut free_slots: Vec<usize> = (0..32).rev().collect();

    let spawn = |join: &mut JoinSet<_>,
                 workers: &mut HashMap<WorkerId, WorkerInfo>,
                 free_slots: &mut Vec<usize>,
                 wid: WorkerId,
                 a: Assignment,
                 body: Option<ByteStream>| {
        let slot = free_slots.pop().unwrap_or(0);
        let bytes = Arc::new(AtomicU64::new(0));
        workers.insert(
            wid,
            WorkerInfo {
                slot,
                index: a.index,
                bytes: bytes.clone(),
                meter: SpeedMeter::new(Duration::from_secs(3)),
            },
        );
        let c = ctx.clone();
        join.spawn(async move { (wid, run_worker(c, wid, slot, a, body, bytes).await) });
    };

    // First connection: the probe body.
    if let Some(body) = first_body {
        let idx = {
            let t = table.lock();
            t.segments()
                .iter()
                .position(|s| !s.is_done() && s.written == first_from && s.worker.is_none())
        };
        if let Some(idx) = idx {
            let a = table
                .lock()
                .assign(idx, next_wid)
                .map_err(|e| EngineError::Other(e.to_string()))?;
            spawn(
                &mut join,
                &mut workers,
                &mut free_slots,
                next_wid,
                a,
                Some(body),
            );
            next_wid += 1;
        }
    }

    let session_start = Instant::now();
    let base_elapsed = sh.record.lock().elapsed_ms;
    let session_base_bytes = table.lock().received_bytes();
    let mut total_meter = SpeedMeter::new(Duration::from_secs(3));
    let mut last_checkpoint = Instant::now();
    let mut last_history = Instant::now();
    let mut last_written = table.lock().written_bytes();
    let mut transient_errors_in_row = 0u32;
    let mut last_error: Option<EngineError> = None;
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let outcome: EngineResult<()> = loop {
        tokio::select! {
            _ = interval.tick() => {}
            Some(joined) = join.join_next(), if !join.is_empty() => {
                let (wid, res) = match joined {
                    Ok(v) => v,
                    Err(e) => break Err(EngineError::Other(format!("connection task panicked: {e}"))),
                };
                if let Some(w) = workers.remove(&wid) {
                    free_slots.push(w.slot);
                }
                match res {
                    Ok(WorkerExit::Done) => {
                        transient_errors_in_row = 0;
                    }
                    Ok(WorkerExit::Eof) => {
                        let total = table.lock().close_at_eof();
                        let mut r = sh.record.lock();
                        r.total_size = Some(total);
                        env.hooks.record_changed(&r);
                    }
                    Err(EngineError::Cancelled) => {}
                    Err(e) => {
                        let others = workers.len();
                        match &e {
                            EngineError::Http(HttpError::RangeIgnored | HttpError::BadContentRange(_))
                                if others > 0 =>
                            {
                                table.lock().set_splittable(false);
                                ctl.limit_by_server(1);
                                log(sh, env, "warn", &format!(
                                    "unreliable range response ({e}); continuing with one connection"
                                ));
                            }
                            EngineError::Http(HttpError::Status { status, .. })
                                if others > 0 && matches!(status, 403 | 429 | 503 | 509) =>
                            {
                                ctl.limit_by_server(others);
                                ctl.respawn_after = Instant::now() + Duration::from_secs(5);
                                log(sh, env, "info", &format!(
                                    "server refused an extra connection (HTTP {status}); limiting to {}",
                                    ctl.cap()
                                ));
                            }
                            EngineError::Http(HttpError::Network(_) | HttpError::Timeout)
                                if others > 0 && ctl.target() > 1 =>
                            {
                                transient_errors_in_row += 1;
                                if transient_errors_in_row >= 3 {
                                    ctl.limit_by_server(others);
                                }
                                ctl.respawn_after = Instant::now() + Duration::from_secs(2);
                                log(sh, env, "warn", &format!("connection error: {e}; other connections continue"));
                            }
                            e if e.is_transient() => {
                                transient_errors_in_row += 1;
                                last_error = Some(e.clone());
                                if others == 0 {
                                    break Err(e.clone());
                                }
                                ctl.respawn_after = Instant::now() + Duration::from_secs(2);
                                log(sh, env, "warn", &format!("connection error: {e}; other connections continue"));
                            }
                            e => {
                                break Err(e.clone());
                            }
                        }
                    }
                }
            }
            _ = sh.cancel.cancelled() => {
                break Err(EngineError::Cancelled);
            }
        }

        if let Some(e) = writer.error() {
            break Err(e);
        }

        let now = Instant::now();
        let (received, written, complete_received, can_split, unassigned_work) = {
            let t = table.lock();
            let unassigned = t
                .segments()
                .iter()
                .any(|s| s.worker.is_none() && (s.is_open_ended() || !s.is_received()));
            (
                t.received_bytes(),
                t.written_bytes(),
                t.is_fully_received(),
                t.is_splittable(),
                unassigned,
            )
        };

        if complete_received && join.is_empty() {
            break Ok(());
        }

        // Spawn connections up to the controller's target.
        total_meter.record(now, received);
        ctl.on_tick(
            now,
            workers.len(),
            total_meter.speed_over(Duration::from_secs(2)),
            can_split,
        );
        if now >= ctl.respawn_after {
            let target = ctl.target();
            let mut spawned_this_tick = 0;
            while workers.len() < target && spawned_this_tick < 2 {
                if !can_split && !unassigned_work {
                    break;
                }
                let a = table.lock().acquire(next_wid);
                let Some(a) = a else { break };
                spawn(&mut join, &mut workers, &mut free_slots, next_wid, a, None);
                next_wid += 1;
                spawned_this_tick += 1;
            }
        }
        // Nothing running, nothing left to assign, but not complete: the
        // only possible cause is a failed attempt to spawn; surface it.
        if workers.is_empty() && !complete_received && now >= ctl.respawn_after {
            let a = table.lock().acquire(next_wid);
            match a {
                Some(a) => {
                    spawn(&mut join, &mut workers, &mut free_slots, next_wid, a, None);
                    next_wid += 1;
                }
                None => {
                    break Err(last_error.clone().unwrap_or_else(|| {
                        EngineError::Other("no work could be assigned to a connection".into())
                    }));
                }
            }
        }

        // Live progress.
        let elapsed_ms = base_elapsed + session_start.elapsed().as_millis() as u64;
        {
            let t = table.lock();
            let mut seg_speed: HashMap<usize, u64> = HashMap::new();
            for w in workers.values_mut() {
                w.meter.record(now, w.bytes.load(Ordering::Relaxed));
                seg_speed.insert(w.index, w.meter.speed());
            }
            let speed = total_meter.speed();
            let session_secs = session_start.elapsed().as_secs_f64();
            let mut live = sh.live.lock();
            live.status = Some(DownloadStatus::Downloading);
            live.downloaded = received;
            live.total = t.total();
            live.speed = speed;
            live.avg_speed = if session_secs > 1.0 {
                ((received.saturating_sub(session_base_bytes)) as f64 / session_secs) as u64
            } else {
                speed
            };
            live.eta_secs = match (t.total(), speed) {
                (Some(total), s) if s > 0 => Some(total.saturating_sub(received) / s),
                _ => None,
            };
            live.active_connections = workers.len() as u8;
            live.elapsed_ms = elapsed_ms;
            live.stage = None;
            let mut segs: Vec<(u64, SegmentProgress)> = t
                .segments()
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    (
                        s.start,
                        SegmentProgress {
                            index: i as u32,
                            start: s.start,
                            end: (!s.is_open_ended()).then_some(s.end),
                            written: s.received - s.start,
                            active: s.worker.is_some(),
                            speed: seg_speed.get(&i).copied().unwrap_or(0),
                            done: s.is_received(),
                        },
                    )
                })
                .collect();
            segs.sort_by_key(|(start, _)| *start);
            live.segments = segs.into_iter().map(|(_, s)| s).collect();
            if now.duration_since(last_history) >= Duration::from_secs(1) {
                live.history.push_back(speed);
                while live.history.len() > HISTORY_LEN {
                    live.history.pop_front();
                }
                last_history = now;
            }
        }

        // Durable checkpoint.
        if now.duration_since(last_checkpoint) >= CHECKPOINT_EVERY {
            last_checkpoint = now;
            let snap = writer.checkpoint().await?;
            let delta = persist_checkpoint(sh, env, &snap, &table, elapsed_ms);
            let _ = delta;
            let w = table.lock().written_bytes();
            let _ = env.db.add_transferred(w.saturating_sub(last_written));
            last_written = w;
        }
        let _ = written;
    };

    // Stop every connection.
    worker_cancel.cancel();
    while join.join_next().await.is_some() {}
    table.lock().release_all();
    {
        let mut r = sh.record.lock();
        r.elapsed_ms = base_elapsed + session_start.elapsed().as_millis() as u64;
    }
    let w = table.lock().written_bytes();
    let _ = env.db.add_transferred(w.saturating_sub(last_written));
    {
        let mut live = sh.live.lock();
        live.active_connections = 0;
        live.speed = 0;
    }
    outcome
}

/// Save a durable snapshot. Returns the written byte count.
fn persist_checkpoint(
    sh: &TaskShared,
    env: &TaskEnv,
    snap: &[velox_segments::PersistedSegment],
    table: &Mutex<SegmentTable>,
    elapsed_ms: u64,
) -> u64 {
    let written: u64 = snap.iter().map(|s| s.written - s.start).sum();
    let mut r = sh.record.lock();
    r.downloaded = written;
    if elapsed_ms > 0 {
        r.elapsed_ms = elapsed_ms;
    }
    if r.total_size.is_none() {
        r.total_size = table.lock().total();
    }
    if let Err(e) = env.db.checkpoint(&r, snap) {
        tracing::error!(error = %e, "checkpoint failed");
    }
    written
}

async fn finalize(
    sh: &Arc<TaskShared>,
    env: &TaskEnv,
    table: Arc<Mutex<SegmentTable>>,
    writer: DiskWriter,
    temp: &Path,
) -> EngineResult<()> {
    writer.finish().await?;
    let (complete, total) = {
        let t = table.lock();
        (t.is_complete(), t.total())
    };
    if !complete {
        return Err(EngineError::Other(
            "internal error: download ended with missing data".into(),
        ));
    }
    let total = total.unwrap_or(0);
    let len = std::fs::metadata(temp).map_err(EngineError::from_io)?.len();
    if len != total {
        if len > total {
            let f = std::fs::OpenOptions::new()
                .write(true)
                .open(temp)
                .map_err(EngineError::from_io)?;
            f.set_len(total).map_err(EngineError::from_io)?;
            f.sync_all().map_err(EngineError::from_io)?;
        } else {
            return Err(EngineError::Io(format!(
                "file size {len} does not match expected {total}"
            )));
        }
    }

    // Move into place.
    let (dir, name, conflict) = {
        let r = sh.record.lock();
        (PathBuf::from(&r.save_dir), r.file_name.clone(), r.conflict)
    };
    let mut final_path = dir.join(&name);
    if final_path.exists() {
        match conflict {
            ConflictPolicy::Overwrite => {
                std::fs::remove_file(&final_path).map_err(EngineError::from_io)?;
            }
            ConflictPolicy::Rename => {
                let reserved = env.reserved.lock().clone();
                final_path = naming::unique_path(&dir, &name, &reserved);
            }
        }
    }
    fsutil::move_file(temp, &final_path).map_err(EngineError::from_io)?;
    fsutil::sync_dir(&dir);

    let checksum_spec = {
        let mut r = sh.record.lock();
        r.file_name = final_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(name);
        r.temp_path = None;
        r.total_size = Some(total);
        r.downloaded = total;
        r.checksum.clone()
    };
    env.db.clear_segments(sh.record.lock().id)?;

    if let Some(spec) = checksum_spec {
        set_status(sh, env, DownloadStatus::Processing);
        sh.live.lock().stage = Some("Verifying checksum".into());
        let path = final_path.clone();
        let algo = spec.algorithm;
        let actual = tokio::task::spawn_blocking(move || checksum::hash_file(&path, algo))
            .await
            .map_err(|e| EngineError::Other(e.to_string()))?
            .map_err(EngineError::from_io)?;
        let ok = actual.eq_ignore_ascii_case(spec.expected.trim());
        sh.record.lock().checksum_ok = Some(ok);
        if !ok {
            return Err(EngineError::ChecksumMismatch {
                expected: spec.expected,
                actual,
            });
        }
        log(
            sh,
            env,
            "info",
            &format!("{} checksum verified", algo.as_str()),
        );
    }
    Ok(())
}
