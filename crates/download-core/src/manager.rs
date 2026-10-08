//! The download manager: registry of downloads, command entry point for
//! the UI / browser integration, and owner of running tasks.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use velox_http::{headers, ClientOptions, RequestContext, Validators};
use velox_persistence::secrets::is_sensitive_header;
use velox_persistence::{Database, DownloadRecord, DownloadSecrets, LogEntry, SecretBox};
use velox_types::{
    AddDownloadRequest, AppSettings, Category, ChecksumSpec, DownloadId, DownloadInfo,
    DownloadKind, DownloadStatus, EngineEvent, ProgressSnapshot, StartMode, UrlInfo, MAIN_QUEUE_ID,
};

use crate::error::{EngineError, EngineResult};
use crate::naming;
use crate::ratelimit::RateLimiter;
use crate::task::{
    run_file_task, LiveState, StopReason, TaskEnv, TaskHooks, TaskOutcome, TaskShared,
};

/// Runs media (HLS / DASH / extractor) downloads. Implemented by
/// `velox-media` and registered by the application.
pub trait MediaRunner: Send + Sync {
    fn run(
        &self,
        shared: Arc<TaskShared>,
        env: TaskEnv,
    ) -> Pin<Box<dyn Future<Output = TaskOutcome> + Send>>;
}

pub struct ManagerConfig {
    pub db: Database,
    /// Key for encrypting cookies/credentials; `None` = keep them in memory only.
    pub secret_box: Option<SecretBox>,
    pub proxy_password: Option<String>,
}

struct Running {
    shared: Arc<TaskShared>,
    handle: Option<JoinHandle<()>>,
}

struct Inner {
    db: Database,
    secret_box: Option<SecretBox>,
    settings: RwLock<AppSettings>,
    proxy_password: RwLock<Option<String>>,
    records: Mutex<HashMap<DownloadId, Arc<Mutex<DownloadRecord>>>>,
    running: Mutex<HashMap<DownloadId, Running>>,
    memory_secrets: Mutex<HashMap<DownloadId, DownloadSecrets>>,
    events: broadcast::Sender<EngineEvent>,
    global_limiter: Arc<RateLimiter>,
    reserved: Arc<Mutex<HashSet<PathBuf>>>,
    media_runner: RwLock<Option<Arc<dyn MediaRunner>>>,
    shutdown: CancellationToken,
}

/// Handle to the download manager. Cheap to clone.
#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Inner>,
}

struct Hooks(std::sync::Weak<Inner>);

impl TaskHooks for Hooks {
    fn record_changed(&self, record: &DownloadRecord) {
        if let Some(inner) = self.0.upgrade() {
            let info = inner.info_for(record);
            let _ = inner.events.send(EngineEvent::Updated { download: info });
        }
    }

    fn log(&self, record: &DownloadRecord, level: &str, message: &str) {
        if let Some(inner) = self.0.upgrade() {
            match level {
                "error" => tracing::error!(id = %record.id, "{message}"),
                "warn" => tracing::warn!(id = %record.id, "{message}"),
                _ => tracing::info!(id = %record.id, "{message}"),
            }
            let _ = inner.db.append_log(record.id, level, message);
        }
    }
}

impl Inner {
    fn info_for(&self, r: &DownloadRecord) -> DownloadInfo {
        let live = self
            .running
            .lock()
            .get(&r.id)
            .map(|run| run.shared.live.lock().clone());
        to_info(r, live.as_ref(), self.has_secrets(r))
    }

    fn has_secrets(&self, r: &DownloadRecord) -> bool {
        r.secret_id.is_some() || self.memory_secrets.lock().contains_key(&r.id)
    }

    fn emit(&self, e: EngineEvent) {
        let _ = self.events.send(e);
    }

    fn client_options(&self) -> ClientOptions {
        let s = self.settings.read();
        ClientOptions {
            user_agent: s.network.user_agent.clone(),
            connect_timeout: Duration::from_secs(s.network.connect_timeout_secs as u64),
            read_timeout: Duration::from_secs(s.network.read_timeout_secs as u64),
            max_redirects: s.network.max_redirects,
            proxy: s.network.proxy.clone(),
            proxy_password: self.proxy_password.read().clone(),
        }
    }

    fn load_secrets(&self, r: &DownloadRecord) -> DownloadSecrets {
        if let Some(s) = self.memory_secrets.lock().get(&r.id) {
            return s.clone();
        }
        match (&self.secret_box, &r.secret_id) {
            (Some(sb), Some(sid)) => match self.db.get_secrets(sb, sid) {
                Ok(Some(s)) => s,
                Ok(None) => DownloadSecrets::default(),
                Err(e) => {
                    tracing::warn!(id = %r.id, error = %e, "cannot decrypt stored secrets");
                    DownloadSecrets::default()
                }
            },
            _ => DownloadSecrets::default(),
        }
    }
}

fn to_info(r: &DownloadRecord, live: Option<&LiveState>, has_secrets: bool) -> DownloadInfo {
    let (downloaded, speed, avg, eta, active, elapsed, status) = match live {
        Some(l) => (
            l.downloaded.max(r.downloaded),
            l.speed,
            l.avg_speed,
            l.eta_secs,
            l.active_connections,
            l.elapsed_ms.max(r.elapsed_ms),
            l.status.unwrap_or(r.status),
        ),
        None => (
            r.downloaded,
            0,
            if r.elapsed_ms > 0 {
                r.downloaded * 1000 / r.elapsed_ms.max(1)
            } else {
                0
            },
            None,
            0,
            r.elapsed_ms,
            r.status,
        ),
    };
    DownloadInfo {
        id: r.id,
        url: r.url.clone(),
        final_url: r.final_url.clone(),
        page_url: r.page_url.clone(),
        file_name: r.file_name.clone(),
        save_dir: r.save_dir.clone(),
        kind: r.kind,
        status,
        category: r.category,
        total_size: r.total_size,
        downloaded,
        resumable: r.resumable,
        max_connections: r.max_connections,
        active_connections: active,
        speed,
        avg_speed: avg,
        eta_secs: eta,
        error: r.error.clone(),
        error_kind: r.error_kind,
        mime: r.mime.clone(),
        referer: r.referer.clone(),
        queue_id: r.queue_id.clone(),
        priority: r.priority,
        created_at: r.created_at,
        started_at: r.started_at,
        completed_at: r.completed_at,
        scheduled_at: r.scheduled_at,
        next_retry_at: None,
        retry_count: r.retry_count,
        speed_limit: r.speed_limit,
        checksum: r.checksum.clone(),
        checksum_ok: r.checksum_ok,
        media: r.media.clone(),
        elapsed_ms: elapsed,
        has_secrets,
    }
}

/// Classify a URL by scheme / path.
pub fn detect_kind(url: &str) -> EngineResult<DownloadKind> {
    let parsed = velox_http::url::Url::parse(url.trim())
        .map_err(|e| EngineError::InvalidUrl(e.to_string()))?;
    match parsed.scheme() {
        "http" | "https" => {
            let path = parsed.path().to_ascii_lowercase();
            if path.ends_with(".m3u8") {
                Ok(DownloadKind::Hls)
            } else if path.ends_with(".mpd") {
                Ok(DownloadKind::Dash)
            } else {
                Ok(DownloadKind::Http)
            }
        }
        "ftp" | "ftps" => Ok(DownloadKind::Ftp),
        "sftp" => Ok(DownloadKind::Sftp),
        s => Err(EngineError::InvalidUrl(format!(
            "unsupported URL scheme \"{s}\""
        ))),
    }
}

impl DownloadManager {
    /// Load state from the database and start background tasks.
    ///
    /// Downloads that were running when the application last stopped
    /// (normal exit, crash or power loss) are returned so the caller can
    /// resume them; their persisted progress is always backed by fsynced
    /// data.
    pub async fn open(cfg: ManagerConfig) -> EngineResult<(Self, Vec<DownloadId>)> {
        velox_http::init_crypto();
        let settings = cfg.db.load_settings()?;
        let (events, _) = broadcast::channel(1024);
        let inner = Arc::new(Inner {
            db: cfg.db,
            secret_box: cfg.secret_box,
            global_limiter: Arc::new(RateLimiter::new(settings.downloads.speed_limit)),
            settings: RwLock::new(settings),
            proxy_password: RwLock::new(cfg.proxy_password),
            records: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            memory_secrets: Mutex::new(HashMap::new()),
            events,
            reserved: Arc::new(Mutex::new(HashSet::new())),
            media_runner: RwLock::new(None),
            shutdown: CancellationToken::new(),
        });
        let mut interrupted = Vec::new();
        let records = inner.db.list_downloads()?;
        {
            let mut map = inner.records.lock();
            let mut reserved = inner.reserved.lock();
            for mut r in records {
                if r.status.is_active() {
                    interrupted.push(r.id);
                    r.status = DownloadStatus::Paused;
                    inner.db.save_download(&r)?;
                }
                if !r.status.is_terminal() {
                    reserved.insert(Path::new(&r.save_dir).join(&r.file_name));
                }
                map.insert(r.id, Arc::new(Mutex::new(r)));
            }
        }
        let mgr = Self { inner };
        mgr.spawn_progress_ticker();
        let resume = if mgr.settings().downloads.resume_on_startup {
            interrupted
        } else {
            Vec::new()
        };
        Ok((mgr, resume))
    }

    fn spawn_progress_ticker(&self) {
        let weak = Arc::downgrade(&self.inner);
        let shutdown = self.inner.shutdown.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(500));
            loop {
                tokio::select! {
                    _ = iv.tick() => {}
                    _ = shutdown.cancelled() => break,
                }
                let Some(inner) = weak.upgrade() else { break };
                let items = DownloadManager { inner }.progress_snapshots();
                if !items.is_empty() {
                    if let Some(inner) = weak.upgrade() {
                        inner.emit(EngineEvent::Progress { items });
                    }
                }
            }
        });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    pub fn db(&self) -> &Database {
        &self.inner.db
    }

    pub fn set_media_runner(&self, runner: Arc<dyn MediaRunner>) {
        *self.inner.media_runner.write() = Some(runner);
    }

    // ----- settings -------------------------------------------------------

    pub fn settings(&self) -> AppSettings {
        self.inner.settings.read().clone()
    }

    pub fn update_settings(&self, s: AppSettings) -> EngineResult<AppSettings> {
        let s = s.normalized();
        self.inner.db.save_settings(&s)?;
        self.inner.global_limiter.set_rate(s.downloads.speed_limit);
        *self.inner.settings.write() = s.clone();
        Ok(s)
    }

    pub fn set_proxy_password(&self, password: Option<String>) {
        *self.inner.proxy_password.write() = password;
    }

    pub fn set_global_speed_limit(&self, bytes_per_sec: u64) -> EngineResult<()> {
        let mut s = self.settings();
        s.downloads.speed_limit = bytes_per_sec;
        self.update_settings(s)?;
        Ok(())
    }

    // ----- queries --------------------------------------------------------

    pub fn list(&self) -> Vec<DownloadInfo> {
        let records: Vec<_> = self.inner.records.lock().values().cloned().collect();
        let mut out: Vec<DownloadInfo> = records
            .iter()
            .map(|r| self.inner.info_for(&r.lock()))
            .collect();
        out.sort_by_key(|d| std::cmp::Reverse(d.created_at));
        out
    }

    pub fn get(&self, id: DownloadId) -> Option<DownloadInfo> {
        let rec = self.inner.records.lock().get(&id).cloned()?;
        let r = rec.lock();
        Some(self.inner.info_for(&r))
    }

    pub fn record(&self, id: DownloadId) -> Option<DownloadRecord> {
        self.inner.records.lock().get(&id).map(|r| r.lock().clone())
    }

    pub fn is_running(&self, id: DownloadId) -> bool {
        self.inner.running.lock().contains_key(&id)
    }

    pub fn running_ids(&self) -> Vec<DownloadId> {
        self.inner.running.lock().keys().copied().collect()
    }

    pub fn running_count(&self) -> usize {
        self.inner.running.lock().len()
    }

    pub fn log(&self, id: DownloadId) -> EngineResult<Vec<LogEntry>> {
        Ok(self.inner.db.read_log(id)?)
    }

    pub fn speed_history(&self, id: DownloadId) -> Vec<u64> {
        self.inner
            .running
            .lock()
            .get(&id)
            .map(|r| r.shared.live.lock().history.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Snapshots of all running downloads.
    pub fn progress_snapshots(&self) -> Vec<ProgressSnapshot> {
        let running: Vec<(DownloadId, Arc<TaskShared>)> = self
            .inner
            .running
            .lock()
            .iter()
            .map(|(id, r)| (*id, r.shared.clone()))
            .collect();
        running
            .into_iter()
            .map(|(id, sh)| {
                let rec_status = sh.record.lock().status;
                let l = sh.live.lock();
                ProgressSnapshot {
                    id,
                    status: l.status.unwrap_or(rec_status),
                    downloaded: l.downloaded,
                    total_size: l.total,
                    speed: l.speed,
                    avg_speed: l.avg_speed,
                    eta_secs: l.eta_secs,
                    active_connections: l.active_connections,
                    elapsed_ms: l.elapsed_ms,
                    segments: l.segments.clone(),
                    stage: l.stage.clone(),
                }
            })
            .collect()
    }

    /// Existing downloads with the same URL.
    pub fn find_duplicates(&self, url: &str) -> Vec<DownloadId> {
        let url = url.trim();
        self.inner
            .records
            .lock()
            .values()
            .filter_map(|r| {
                let r = r.lock();
                (r.url == url || r.final_url.as_deref() == Some(url)).then_some(r.id)
            })
            .collect()
    }

    /// Directory a file of `category` is saved to with current settings.
    pub fn target_dir(&self, category: Category) -> PathBuf {
        let s = self.inner.settings.read();
        let base = if s.general.download_dir.trim().is_empty() {
            naming::default_download_dir()
        } else {
            PathBuf::from(s.general.download_dir.trim())
        };
        if s.general.category_folders {
            base.join(category.default_folder())
        } else {
            base
        }
    }

    /// Contact the server to learn the file name, size and resumability
    /// before adding (used by the "add download" dialog).
    pub async fn probe_url(&self, req: &AddDownloadRequest) -> EngineResult<UrlInfo> {
        let kind = detect_kind(&req.url)?;
        let url = req.url.trim().to_string();
        if kind != DownloadKind::Http {
            let name = headers::filename_from_url(&url).unwrap_or_else(|| "download".into());
            let name = naming::sanitize_file_name(&name);
            let category = Category::detect(&name, None);
            return Ok(UrlInfo {
                duplicate_of: self.find_duplicates(&url).first().copied(),
                url: url.clone(),
                final_url: url,
                file_name: name,
                total_size: None,
                resumable: kind == DownloadKind::Ftp || kind == DownloadKind::Sftp,
                mime: None,
                category,
                kind,
                suggested_dir: self.target_dir(category).to_string_lossy().to_string(),
            });
        }
        let client = velox_http::build_client(&self.inner.client_options())?;
        let mut hdrs = req.headers.clone();
        hdrs.retain(|h| !h.name.eq_ignore_ascii_case("cookie"));
        let ctx = RequestContext {
            url: url.clone(),
            referer: req.referer.clone(),
            user_agent: req.user_agent.clone(),
            headers: hdrs,
            cookies: req.cookies.clone(),
            credentials: req.credentials.clone(),
        };
        let p = velox_http::probe(&client, &ctx, 0, &Validators::default()).await?;
        let raw = req
            .file_name
            .clone()
            .or(p.disposition_name.clone())
            .or_else(|| headers::filename_from_url(&p.final_url))
            .or_else(|| headers::filename_from_url(&url))
            .unwrap_or_else(|| "download".into());
        let name = naming::ensure_extension(&naming::sanitize_file_name(&raw), p.mime.as_deref());
        let category = Category::detect(&name, p.mime.as_deref());
        let kind = match p.mime.as_deref().map(str::to_ascii_lowercase) {
            Some(m) if m.contains("mpegurl") => DownloadKind::Hls,
            Some(m) if m.contains("dash+xml") => DownloadKind::Dash,
            _ => DownloadKind::Http,
        };
        Ok(UrlInfo {
            duplicate_of: self.find_duplicates(&url).first().copied(),
            url,
            final_url: p.final_url.clone(),
            file_name: name,
            total_size: if p.status == 416 {
                Some(0)
            } else {
                p.total_size
            },
            resumable: p.ranges != velox_http::RangeSupport::No && p.total_size.is_some(),
            mime: p.mime.clone(),
            category,
            kind,
            suggested_dir: self.target_dir(category).to_string_lossy().to_string(),
        })
    }

    // ----- commands -------------------------------------------------------

    /// Create a download.
    pub async fn add(&self, req: AddDownloadRequest) -> EngineResult<DownloadInfo> {
        let url = req.url.trim().to_string();
        let mut kind = detect_kind(&url)?;
        if let Some(m) = &req.media {
            kind = match m.kind {
                velox_types::MediaSourceKind::Hls => DownloadKind::Hls,
                velox_types::MediaSourceKind::Dash => DownloadKind::Dash,
                velox_types::MediaSourceKind::Page => DownloadKind::Extractor,
                velox_types::MediaSourceKind::Direct => DownloadKind::Http,
            };
        }
        if kind.is_media() && self.inner.media_runner.read().is_none() {
            return Err(EngineError::Unsupported(
                "media downloads are not available".into(),
            ));
        }
        let settings = self.settings();

        let name_locked = req
            .file_name
            .as_deref()
            .is_some_and(|n| !n.trim().is_empty());
        let raw_name = req
            .file_name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .or_else(|| req.media.as_ref().and_then(|m| m.title.clone()))
            .or_else(|| headers::filename_from_url(&url))
            .unwrap_or_else(|| "download".into());
        let mut file_name = naming::sanitize_file_name(&raw_name);
        if kind.is_media() {
            if let Some(ext) = req.media.as_ref().and_then(|m| m.container.extension()) {
                if naming::split_ext(&file_name)
                    .1
                    .map(|e| e.to_ascii_lowercase())
                    != Some(ext.to_string())
                {
                    file_name = format!("{file_name}.{ext}");
                }
            }
        } else {
            file_name = naming::ensure_extension(&file_name, req.mime.as_deref());
        }
        let category = if kind.is_media() {
            if req
                .media
                .as_ref()
                .is_some_and(|m| m.container.is_audio_only())
            {
                Category::Music
            } else {
                Category::Video
            }
        } else {
            Category::detect(&file_name, req.mime.as_deref())
        };
        let dir = match req
            .save_dir
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            Some(d) => PathBuf::from(d),
            None => self.target_dir(category),
        };
        if !dir.is_absolute() {
            return Err(EngineError::Io(
                "the download folder must be an absolute path".into(),
            ));
        }
        let final_path = {
            let mut reserved = self.inner.reserved.lock();
            let p = naming::unique_path(&dir, &file_name, &reserved);
            reserved.insert(p.clone());
            p
        };
        let file_name = final_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(file_name);

        // Split sensitive data off the record.
        let mut secrets = DownloadSecrets {
            cookies: req.cookies.clone().filter(|c| !c.trim().is_empty()),
            credentials: req.credentials.clone().filter(|c| !c.username.is_empty()),
            headers: Vec::new(),
        };
        let mut plain_headers = Vec::new();
        for h in &req.headers {
            if h.name.eq_ignore_ascii_case("cookie") {
                secrets.cookies = Some(h.value.clone());
            } else if is_sensitive_header(&h.name) {
                secrets.headers.push(h.clone());
            } else {
                plain_headers.push(h.clone());
            }
        }

        let mut r = DownloadRecord::new(url.clone(), file_name, dir.to_string_lossy().to_string());
        r.kind = kind;
        r.category = category;
        r.page_url = req.page_url.clone();
        r.referer = req.referer.clone().or_else(|| req.page_url.clone());
        r.user_agent = req.user_agent.clone();
        r.headers = plain_headers;
        r.mime = req.mime.clone();
        r.total_size = req.expected_size.filter(|s| *s > 0);
        if kind != DownloadKind::Http {
            r.total_size = None;
        }
        r.max_connections = req
            .connections
            .unwrap_or(settings.downloads.connections_per_download)
            .clamp(1, 32);
        r.queue_id = Some(
            req.queue_id
                .clone()
                .unwrap_or_else(|| MAIN_QUEUE_ID.to_string()),
        );
        r.priority = req.priority.unwrap_or(0);
        r.position = self.inner.db.next_position()?;
        r.scheduled_at = req.scheduled_at;
        r.speed_limit = req.speed_limit.unwrap_or(0);
        r.checksum = req.checksum.clone().map(|c| ChecksumSpec {
            expected: c.expected.trim().to_ascii_lowercase(),
            ..c
        });
        r.media = req.media.clone();
        r.conflict = req.conflict.unwrap_or(settings.general.conflict_policy);
        r.name_locked = name_locked || kind.is_media();
        r.status = match req.start {
            StartMode::Now => DownloadStatus::Paused,
            StartMode::Queue => DownloadStatus::Queued,
            StartMode::Paused => DownloadStatus::Paused,
        };
        if req.scheduled_at.is_some() && req.start != StartMode::Now {
            r.status = DownloadStatus::Queued;
        }

        if !secrets.is_empty() {
            match &self.inner.secret_box {
                Some(sb) => {
                    let sid = format!("dl:{}", r.id);
                    self.inner.db.put_secrets(sb, &sid, &secrets)?;
                    r.secret_id = Some(sid);
                }
                None => {
                    self.inner.memory_secrets.lock().insert(r.id, secrets);
                }
            }
        }
        self.inner.db.save_download(&r)?;
        let id = r.id;
        let info = to_info(&r, None, self.inner.has_secrets(&r));
        self.inner
            .records
            .lock()
            .insert(id, Arc::new(Mutex::new(r)));
        let _ = self
            .inner
            .db
            .append_log(id, "info", &format!("added {url}"));
        self.inner.emit(EngineEvent::Added {
            download: info.clone(),
        });

        if req.start == StartMode::Now && req.scheduled_at.is_none() {
            self.start(id)?;
            return Ok(self.get(id).unwrap_or(info));
        }
        Ok(info)
    }

    /// Start (or resume / retry) a download immediately.
    pub fn start(&self, id: DownloadId) -> EngineResult<()> {
        let rec = self
            .inner
            .records
            .lock()
            .get(&id)
            .cloned()
            .ok_or_else(|| EngineError::Other("unknown download".into()))?;
        let mut running = self.inner.running.lock();
        if running.contains_key(&id) {
            return Ok(());
        }
        let (kind, speed_limit) = {
            let mut r = rec.lock();
            if r.status == DownloadStatus::Completed {
                return Ok(());
            }
            if r.status == DownloadStatus::Cancelled {
                r.downloaded = 0;
                r.temp_path = None;
            }
            r.status = DownloadStatus::Connecting;
            r.error = None;
            r.error_kind = None;
            r.checksum_ok = None;
            r.completed_at = None;
            r.scheduled_at = None;
            self.inner.db.save_download(&r)?;
            self.inner
                .reserved
                .lock()
                .insert(Path::new(&r.save_dir).join(&r.file_name));
            (r.kind, r.speed_limit)
        };
        let secrets = self.inner.load_secrets(&rec.lock());
        let shared = Arc::new(TaskShared {
            record: rec.clone(),
            live: Arc::new(Mutex::new(LiveState {
                status: Some(DownloadStatus::Connecting),
                ..Default::default()
            })),
            cancel: self.inner.shutdown.child_token(),
            stop: Mutex::new(StopReason::None),
            limiter: Arc::new(RateLimiter::new(speed_limit)),
        });
        let env = TaskEnv {
            db: self.inner.db.clone(),
            settings: self.settings(),
            client_opts: self.inner.client_options(),
            global_limiter: self.inner.global_limiter.clone(),
            secrets,
            reserved: self.inner.reserved.clone(),
            hooks: Arc::new(Hooks(Arc::downgrade(&self.inner))),
        };
        let media = if kind.is_media() {
            Some(self.inner.media_runner.read().clone().ok_or_else(|| {
                EngineError::Unsupported("media downloads are not available".into())
            })?)
        } else {
            None
        };
        let inner = self.inner.clone();
        let sh2 = shared.clone();
        running.insert(
            id,
            Running {
                shared: shared.clone(),
                handle: None,
            },
        );
        drop(running);
        let handle = tokio::spawn(async move {
            let outcome = match media {
                Some(m) => m.run(sh2.clone(), env).await,
                None => run_file_task(sh2.clone(), env).await,
            };
            DownloadManager { inner }.on_finished(id, sh2, outcome);
        });
        if let Some(r) = self.inner.running.lock().get_mut(&id) {
            r.handle = Some(handle);
        }
        let info = self.inner.info_for(&rec.lock());
        self.inner.emit(EngineEvent::Updated { download: info });
        Ok(())
    }

    fn on_finished(&self, id: DownloadId, shared: Arc<TaskShared>, outcome: TaskOutcome) {
        self.inner.running.lock().remove(&id);
        let db = &self.inner.db;
        let rec = shared.record.clone();
        let mut r = rec.lock();
        let final_path = Path::new(&r.save_dir).join(&r.file_name);
        match outcome {
            TaskOutcome::Completed => {
                r.status = DownloadStatus::Completed;
                r.completed_at = Some(velox_types::now_ms());
                r.error = None;
                r.error_kind = None;
                r.retry_count = 0;
                let _ = db.save_download(&r);
                let _ = db.add_finished(true);
                let _ = db.append_log(id, "info", "download complete");
                self.inner.reserved.lock().remove(&final_path);
                let info = to_info(&r, None, self.inner.has_secrets(&r));
                self.inner.emit(EngineEvent::Updated {
                    download: info.clone(),
                });
                self.inner.emit(EngineEvent::Completed { download: info });
            }
            TaskOutcome::Stopped(reason) => match reason {
                StopReason::Shutdown => {
                    // Keep the active status so the next start resumes it.
                    let _ = db.save_download(&r);
                }
                StopReason::Pause | StopReason::None => {
                    r.status = DownloadStatus::Paused;
                    let _ = db.save_download(&r);
                    let info = to_info(&r, None, self.inner.has_secrets(&r));
                    self.inner.emit(EngineEvent::Updated { download: info });
                }
                StopReason::Cancel => {
                    if let Some(t) = r.temp_path.take() {
                        let _ = std::fs::remove_file(t);
                    }
                    let _ = db.clear_segments(id);
                    r.status = DownloadStatus::Cancelled;
                    r.downloaded = 0;
                    self.inner.reserved.lock().remove(&final_path);
                    let _ = db.save_download(&r);
                    let info = to_info(&r, None, self.inner.has_secrets(&r));
                    self.inner.emit(EngineEvent::Updated { download: info });
                }
                StopReason::Remove => {
                    // Cleanup is done by `remove` after the task exits.
                }
            },
            TaskOutcome::Failed(e) => {
                r.status = DownloadStatus::Failed;
                r.error = Some(e.to_string());
                r.error_kind = Some(e.kind());
                let _ = db.save_download(&r);
                let _ = db.add_finished(false);
                let _ = db.append_log(id, "error", &e.to_string());
                let info = to_info(&r, None, self.inner.has_secrets(&r));
                self.inner.emit(EngineEvent::Updated {
                    download: info.clone(),
                });
                self.inner.emit(EngineEvent::Failed { download: info });
            }
        }
    }

    fn stop_task(&self, id: DownloadId, reason: StopReason) -> Option<JoinHandle<()>> {
        let mut running = self.inner.running.lock();
        let r = running.get_mut(&id)?;
        *r.shared.stop.lock() = reason;
        r.shared.cancel.cancel();
        r.handle.take()
    }

    async fn wait(handle: Option<JoinHandle<()>>) {
        if let Some(h) = handle {
            let _ = tokio::time::timeout(Duration::from_secs(30), h).await;
        }
    }

    /// Pause a running or queued download. Returns once it has stopped.
    pub async fn pause(&self, id: DownloadId) -> EngineResult<()> {
        if let Some(h) = self.stop_task(id, StopReason::Pause) {
            Self::wait(Some(h)).await;
            return Ok(());
        }
        // Not running: a queued download becomes paused.
        if let Some(rec) = self.inner.records.lock().get(&id).cloned() {
            let mut r = rec.lock();
            if r.status == DownloadStatus::Queued {
                r.status = DownloadStatus::Paused;
                self.inner.db.save_download(&r)?;
                let info = to_info(&r, None, self.inner.has_secrets(&r));
                self.inner.emit(EngineEvent::Updated { download: info });
            }
        }
        Ok(())
    }

    /// Stop and discard partial data.
    pub async fn cancel(&self, id: DownloadId) -> EngineResult<()> {
        if let Some(h) = self.stop_task(id, StopReason::Cancel) {
            Self::wait(Some(h)).await;
            return Ok(());
        }
        let rec = self.inner.records.lock().get(&id).cloned();
        if let Some(rec) = rec {
            let mut r = rec.lock();
            if r.status != DownloadStatus::Completed {
                if let Some(t) = r.temp_path.take() {
                    let _ = std::fs::remove_file(t);
                }
                self.inner.db.clear_segments(id)?;
                r.status = DownloadStatus::Cancelled;
                r.downloaded = 0;
                self.inner.db.save_download(&r)?;
                let info = to_info(&r, None, self.inner.has_secrets(&r));
                self.inner.emit(EngineEvent::Updated { download: info });
            }
        }
        Ok(())
    }

    /// Discard progress and download again from the beginning.
    pub async fn restart(&self, id: DownloadId) -> EngineResult<()> {
        self.cancel(id).await?;
        {
            let rec = self.inner.records.lock().get(&id).cloned();
            if let Some(rec) = rec {
                let mut r = rec.lock();
                r.status = DownloadStatus::Paused;
                r.downloaded = 0;
                r.total_size = None;
                r.resumable = None;
                r.etag = None;
                r.last_modified = None;
                r.retry_count = 0;
                r.elapsed_ms = 0;
                r.started_at = None;
                self.inner.db.save_download(&r)?;
            }
        }
        self.start(id)
    }

    /// Remove from the list; optionally delete the downloaded file.
    pub async fn remove(&self, id: DownloadId, delete_file: bool) -> EngineResult<()> {
        if let Some(h) = self.stop_task(id, StopReason::Remove) {
            Self::wait(Some(h)).await;
        }
        let rec = self.inner.records.lock().remove(&id);
        if let Some(rec) = rec {
            let r = rec.lock().clone();
            let final_path = Path::new(&r.save_dir).join(&r.file_name);
            if let Some(t) = &r.temp_path {
                let _ = std::fs::remove_file(t);
            }
            if delete_file && r.status == DownloadStatus::Completed && final_path.is_file() {
                std::fs::remove_file(&final_path).map_err(EngineError::from_io)?;
            }
            self.inner.reserved.lock().remove(&final_path);
            self.inner.memory_secrets.lock().remove(&id);
            self.inner.db.delete_download(id)?;
            self.inner.emit(EngineEvent::Removed { id });
        }
        Ok(())
    }

    pub async fn pause_all(&self) {
        let ids = self.running_ids();
        let handles: Vec<_> = ids
            .iter()
            .filter_map(|id| self.stop_task(*id, StopReason::Pause))
            .collect();
        for h in handles {
            Self::wait(Some(h)).await;
        }
    }

    /// Stop everything for application exit. Running downloads keep their
    /// active status in the database and are resumed on the next start
    /// (if enabled in settings).
    pub async fn shutdown(&self) {
        let ids = self.running_ids();
        let handles: Vec<_> = ids
            .iter()
            .filter_map(|id| self.stop_task(*id, StopReason::Shutdown))
            .collect();
        for h in handles {
            Self::wait(Some(h)).await;
        }
        self.inner.shutdown.cancel();
    }

    /// Change the address of a download (e.g. after the link expired).
    /// Progress is kept; the next start validates that the new address
    /// serves the same file (size and validators) before resuming.
    pub fn update_url(&self, id: DownloadId, url: &str) -> EngineResult<()> {
        let kind = detect_kind(url)?;
        let rec = self.record_handle(id)?;
        let mut r = rec.lock();
        if self.is_running(id) {
            return Err(EngineError::Other(
                "pause the download before changing its address".into(),
            ));
        }
        if kind != r.kind && !(r.kind.is_media()) {
            return Err(EngineError::InvalidUrl(
                "the new address uses a different protocol".into(),
            ));
        }
        r.url = url.trim().to_string();
        r.final_url = None;
        if r.error_kind == Some(velox_types::ErrorKind::LinkExpired) {
            r.error = None;
            r.error_kind = None;
            r.status = DownloadStatus::Paused;
        }
        self.inner.db.save_download(&r)?;
        let _ = self.inner.db.append_log(id, "info", "address updated");
        let info = to_info(&r, None, self.inner.has_secrets(&r));
        self.inner.emit(EngineEvent::Updated { download: info });
        Ok(())
    }

    fn record_handle(&self, id: DownloadId) -> EngineResult<Arc<Mutex<DownloadRecord>>> {
        self.inner
            .records
            .lock()
            .get(&id)
            .cloned()
            .ok_or_else(|| EngineError::Other("unknown download".into()))
    }

    /// Apply a change to a record that is not running and persist it.
    pub fn modify(
        &self,
        id: DownloadId,
        f: impl FnOnce(&mut DownloadRecord),
    ) -> EngineResult<DownloadInfo> {
        let rec = self.record_handle(id)?;
        let mut r = rec.lock();
        f(&mut r);
        self.inner.db.save_download(&r)?;
        drop(r);
        let info = self.inner.info_for(&rec.lock());
        self.inner.emit(EngineEvent::Updated {
            download: info.clone(),
        });
        Ok(info)
    }

    /// Per-download speed limit (0 = unlimited). Applies immediately.
    pub fn set_speed_limit(&self, id: DownloadId, bytes_per_sec: u64) -> EngineResult<()> {
        self.modify(id, |r| r.speed_limit = bytes_per_sec)?;
        if let Some(run) = self.inner.running.lock().get(&id) {
            run.shared.limiter.set_rate(bytes_per_sec);
        }
        Ok(())
    }

    /// Rename a download that is not running.
    pub fn rename(&self, id: DownloadId, new_name: &str) -> EngineResult<DownloadInfo> {
        if self.is_running(id) {
            return Err(EngineError::Other(
                "pause the download before renaming it".into(),
            ));
        }
        let name = naming::sanitize_file_name(new_name);
        let rec = self.record_handle(id)?;
        let (old, completed, dir) = {
            let r = rec.lock();
            (
                Path::new(&r.save_dir).join(&r.file_name),
                r.status == DownloadStatus::Completed,
                PathBuf::from(&r.save_dir),
            )
        };
        let new_path = dir.join(&name);
        if new_path != old && (new_path.exists() || self.inner.reserved.lock().contains(&new_path))
        {
            return Err(EngineError::Io(format!(
                "{} already exists",
                new_path.display()
            )));
        }
        if completed && old.exists() {
            std::fs::rename(&old, &new_path).map_err(EngineError::from_io)?;
        }
        {
            let mut res = self.inner.reserved.lock();
            if res.remove(&old) {
                res.insert(new_path);
            }
        }
        self.modify(id, |r| {
            r.file_name = name;
            r.name_locked = true;
            r.category = Category::detect(&r.file_name, r.mime.as_deref());
        })
    }

    /// Move a completed file to another folder.
    pub async fn move_completed(
        &self,
        id: DownloadId,
        new_dir: &str,
    ) -> EngineResult<DownloadInfo> {
        let rec = self.record_handle(id)?;
        let (src, name) = {
            let r = rec.lock();
            if r.status != DownloadStatus::Completed {
                return Err(EngineError::Other(
                    "only completed downloads can be moved".into(),
                ));
            }
            (
                Path::new(&r.save_dir).join(&r.file_name),
                r.file_name.clone(),
            )
        };
        let dir = PathBuf::from(new_dir.trim());
        if !dir.is_absolute() {
            return Err(EngineError::Io(
                "the destination must be an absolute path".into(),
            ));
        }
        let dest = naming::unique_path(&dir, &name, &HashSet::new());
        let dest2 = dest.clone();
        tokio::task::spawn_blocking(move || crate::fsutil::move_file(&src, &dest2))
            .await
            .map_err(|e| EngineError::Other(e.to_string()))?
            .map_err(EngineError::from_io)?;
        self.modify(id, |r| {
            r.save_dir = dir.to_string_lossy().to_string();
            r.file_name = dest
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(name);
        })
    }

    /// Hash a completed file and compare it with `spec` (or the stored one).
    pub async fn verify_checksum(
        &self,
        id: DownloadId,
        spec: Option<ChecksumSpec>,
    ) -> EngineResult<(bool, String)> {
        let rec = self.record_handle(id)?;
        let (path, stored) = {
            let r = rec.lock();
            if r.status != DownloadStatus::Completed {
                return Err(EngineError::Other("the download is not complete".into()));
            }
            (
                Path::new(&r.save_dir).join(&r.file_name),
                r.checksum.clone(),
            )
        };
        let spec = spec
            .or(stored)
            .ok_or_else(|| EngineError::Other("no checksum given".into()))?;
        let algo = spec.algorithm;
        let actual = tokio::task::spawn_blocking(move || crate::checksum::hash_file(&path, algo))
            .await
            .map_err(|e| EngineError::Other(e.to_string()))?
            .map_err(EngineError::from_io)?;
        let ok = actual.eq_ignore_ascii_case(spec.expected.trim());
        self.modify(id, |r| {
            r.checksum = Some(ChecksumSpec {
                algorithm: algo,
                expected: spec.expected.trim().to_ascii_lowercase(),
            });
            r.checksum_ok = Some(ok);
        })?;
        Ok((ok, actual))
    }

    /// Generate a fresh id (exposed for tests and the UI's optimistic adds).
    pub fn new_id() -> DownloadId {
        Uuid::new_v4()
    }
}
