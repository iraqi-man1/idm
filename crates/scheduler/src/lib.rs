//! Queue processing for Velox Download Manager.
//!
//! * [`time`] – schedule arithmetic (start/stop times, days, windows).
//! * [`plan`] – which waiting downloads to start under queue and global
//!   limits.
//! * [`power`] – battery / metered-connection state and hold policy.
//! * [`Scheduler`] – runs the above against a [`DownloadManager`], owns the
//!   queue definitions and reports [`SchedulerEvent`]s (queue changes,
//!   finished queues for post-completion actions, power holds).
//!
//! Manual "Start" never goes through the scheduler; it only starts
//! downloads that wait in a started queue (or have a due scheduled time).

pub mod plan;
pub mod power;
pub mod time;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use parking_lot::Mutex;
use tokio::sync::{broadcast, Notify};
use velox_core::{DownloadManager, EngineError, EngineResult};
use velox_types::{
    DownloadId, DownloadStatus, EngineEvent, PowerHold, QueueInfo, QueueUpdate, Schedule,
    SchedulerEvent, MAIN_QUEUE_ID,
};

pub use power::{PowerSource, PowerState, SystemPower};

/// Wall clock (replaceable in tests).
pub trait Clock: Send + Sync {
    /// Local wall-clock time, used for schedules.
    fn now_local(&self) -> NaiveDateTime;
    /// Milliseconds since the Unix epoch, used for scheduled downloads.
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_local(&self) -> NaiveDateTime {
        chrono::Local::now().naive_local()
    }
    fn now_ms(&self) -> i64 {
        velox_types::now_ms()
    }
}

pub struct SchedulerConfig {
    /// Maximum time between two passes (events also trigger a pass).
    pub tick: Duration,
    /// How often the power state is read.
    pub power_interval: Duration,
    pub clock: Arc<dyn Clock>,
    pub power: Arc<dyn PowerSource>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(1),
            power_interval: Duration::from_secs(30),
            clock: Arc::new(SystemClock),
            power: Arc::new(SystemPower),
        }
    }
}

#[derive(Default)]
struct Batch {
    /// Downloads started by this queue since it was (re)started or last finished.
    started: Vec<DownloadId>,
}

#[derive(Default)]
struct State {
    prev_tick: Option<NaiveDateTime>,
    /// Running downloads started by queue processing, with their queue.
    started_by: HashMap<DownloadId, String>,
    batches: HashMap<String, Batch>,
    hold: Option<PowerHold>,
    power_checked: Option<Instant>,
}

struct Inner {
    manager: DownloadManager,
    cfg: SchedulerConfig,
    events: broadcast::Sender<SchedulerEvent>,
    state: Mutex<State>,
    wake: Notify,
    stopped: AtomicBool,
    /// Serializes passes with queue start/stop requests.
    pass: tokio::sync::Mutex<()>,
}

/// Handle to the running scheduler (cheap to clone).
#[derive(Clone)]
pub struct Scheduler {
    inner: Arc<Inner>,
}

fn invalid(msg: &str) -> EngineError {
    EngineError::Other(msg.to_string())
}

fn validate_schedule(s: &Schedule) -> EngineResult<()> {
    for t in [&s.start_time, &s.stop_time].into_iter().flatten() {
        if time::parse_hhmm(t).is_none() {
            return Err(invalid("times must use the HH:MM format"));
        }
    }
    if s.days.iter().any(|d| *d > 6) {
        return Err(invalid("days must be 0 (Sunday) to 6 (Saturday)"));
    }
    if s.enabled && s.start_time.is_none() && s.stop_time.is_none() {
        return Err(invalid("a schedule needs a start or a stop time"));
    }
    Ok(())
}

impl Scheduler {
    /// Start processing queues on the current Tokio runtime.
    pub fn start(manager: DownloadManager, cfg: SchedulerConfig) -> Self {
        let (events, _) = broadcast::channel(64);
        let s = Scheduler {
            inner: Arc::new(Inner {
                manager,
                cfg,
                events,
                state: Mutex::new(State::default()),
                wake: Notify::new(),
                stopped: AtomicBool::new(false),
                pass: tokio::sync::Mutex::new(()),
            }),
        };
        // Engine changes (adds, completions, failures) trigger a pass.
        let mut rx = s.inner.manager.subscribe();
        let weak = Arc::downgrade(&s.inner);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(EngineEvent::Progress { .. }) => {}
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => match weak.upgrade() {
                        Some(i) => i.wake.notify_one(),
                        None => break,
                    },
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        let me = s.clone();
        tokio::spawn(async move {
            while !me.inner.stopped.load(Ordering::SeqCst) {
                me.pass().await;
                tokio::select! {
                    _ = me.inner.wake.notified() => {}
                    _ = tokio::time::sleep(me.inner.cfg.tick) => {}
                }
            }
        });
        s
    }

    pub fn stop(&self) {
        self.inner.stopped.store(true, Ordering::SeqCst);
        self.inner.wake.notify_one();
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SchedulerEvent> {
        self.inner.events.subscribe()
    }

    /// Run a pass soon (e.g. after settings changed).
    pub fn wake(&self) {
        self.inner.wake.notify_one();
    }

    pub fn power_hold(&self) -> Option<PowerHold> {
        self.inner.state.lock().hold
    }

    // ----- queues -------------------------------------------------------

    pub fn queues(&self) -> EngineResult<Vec<QueueInfo>> {
        Ok(self.inner.manager.db().list_queues()?)
    }

    fn queue(&self, id: &str) -> EngineResult<QueueInfo> {
        self.inner
            .manager
            .db()
            .get_queue(id)?
            .ok_or_else(|| invalid("unknown queue"))
    }

    fn changed(&self) {
        if let Ok(queues) = self.queues() {
            let _ = self
                .inner
                .events
                .send(SchedulerEvent::QueuesChanged { queues });
        }
        self.wake();
    }

    pub fn create_queue(&self, name: &str) -> EngineResult<QueueInfo> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(invalid("a queue name needs 1 to 64 characters"));
        }
        let order = self
            .queues()?
            .iter()
            .map(|q| q.sort_order)
            .max()
            .unwrap_or(0)
            + 1;
        let q = QueueInfo {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            name: name.to_string(),
            max_concurrent: 2,
            running: false,
            schedule: Schedule::default(),
            post_action: Default::default(),
            retry_failed: false,
            sort_order: order,
            built_in: false,
        };
        self.inner.manager.db().save_queue(&q)?;
        self.changed();
        Ok(q)
    }

    pub fn update_queue(&self, id: &str, u: QueueUpdate) -> EngineResult<QueueInfo> {
        let mut q = self.queue(id)?;
        if let Some(n) = u.name {
            let n = n.trim();
            if n.is_empty() || n.chars().count() > 64 {
                return Err(invalid("a queue name needs 1 to 64 characters"));
            }
            q.name = n.to_string();
        }
        if let Some(m) = u.max_concurrent {
            q.max_concurrent = m.clamp(1, 32);
        }
        if let Some(s) = u.schedule {
            validate_schedule(&s)?;
            q.schedule = s;
        }
        if let Some(p) = u.post_action {
            q.post_action = p;
        }
        if let Some(r) = u.retry_failed {
            q.retry_failed = r;
        }
        self.inner.manager.db().save_queue(&q)?;
        self.changed();
        Ok(q)
    }

    /// Delete a user queue; its downloads move to the main queue.
    pub async fn delete_queue(&self, id: &str) -> EngineResult<()> {
        let q = self.queue(id)?;
        if q.built_in {
            return Err(invalid("the main queue cannot be deleted"));
        }
        self.stop_queue(id).await?;
        for d in self.inner.manager.list() {
            if plan::queue_of(&d) == id {
                self.inner.manager.set_queue(d.id, MAIN_QUEUE_ID)?;
            }
        }
        self.inner.manager.db().delete_queue(id)?;
        self.inner.state.lock().batches.remove(id);
        self.changed();
        Ok(())
    }

    /// Start processing a queue.
    pub async fn start_queue(&self, id: &str) -> EngineResult<()> {
        let _pass = self.inner.pass.lock().await;
        self.start_queue_locked(id)?;
        drop(_pass);
        self.changed();
        Ok(())
    }

    fn start_queue_locked(&self, id: &str) -> EngineResult<()> {
        let mut q = self.queue(id)?;
        if q.retry_failed {
            for d in self.inner.manager.list() {
                if plan::queue_of(&d) == id && d.status == DownloadStatus::Failed {
                    self.inner.manager.modify(d.id, |r| {
                        r.status = DownloadStatus::Queued;
                        r.error = None;
                        r.error_kind = None;
                        r.retry_count = 0;
                    })?;
                }
            }
        }
        self.inner
            .state
            .lock()
            .batches
            .insert(id.to_string(), Batch::default());
        if !q.running {
            q.running = true;
            self.inner.manager.db().save_queue(&q)?;
            tracing::info!(queue = %q.name, "queue started");
        }
        Ok(())
    }

    /// Stop a queue: downloads it started go back to waiting in it.
    pub async fn stop_queue(&self, id: &str) -> EngineResult<()> {
        let _pass = self.inner.pass.lock().await;
        self.stop_queue_locked(id).await?;
        drop(_pass);
        self.changed();
        Ok(())
    }

    async fn stop_queue_locked(&self, id: &str) -> EngineResult<()> {
        let mut q = self.queue(id)?;
        if q.running {
            q.running = false;
            self.inner.manager.db().save_queue(&q)?;
            tracing::info!(queue = %q.name, "queue stopped");
        }
        let ids: Vec<DownloadId> = {
            let mut st = self.inner.state.lock();
            st.batches.remove(id);
            let ids = st
                .started_by
                .iter()
                .filter(|(_, q)| *q == id)
                .map(|(d, _)| *d)
                .collect::<Vec<_>>();
            for d in &ids {
                st.started_by.remove(d);
            }
            ids
        };
        for d in ids {
            self.inner.manager.requeue(d).await?;
        }
        Ok(())
    }

    // ----- processing ---------------------------------------------------

    async fn pass(&self) {
        let _pass = self.inner.pass.lock().await;
        if let Err(e) = self.pass_locked().await {
            tracing::warn!(error = %e, "scheduler pass failed");
        }
    }

    async fn pass_locked(&self) -> EngineResult<()> {
        let mgr = &self.inner.manager;
        let clock = &self.inner.cfg.clock;
        let now = clock.now_local();
        let mut changed = false;

        // 1. Schedules.
        let prev = self.inner.state.lock().prev_tick.replace(now);
        for q in self.queues()? {
            let action = match prev {
                // At startup a queue inside its window runs (its start time
                // may have passed while the app was closed).
                None => (time::in_window(&q.schedule, now) && !q.running)
                    .then_some(time::Trigger::Start),
                Some(p) => time::triggers(&q.schedule, p, now),
            };
            if q.schedule.enabled {
                tracing::debug!(queue = %q.name, schedule = ?q.schedule, running = q.running, ?prev, %now, ?action, "schedule check");
            }
            match action {
                Some(time::Trigger::Start) if !q.running => {
                    tracing::info!(queue = %q.name, "schedule start time reached");
                    self.start_queue_locked(&q.id)?;
                    changed = true;
                }
                Some(time::Trigger::Stop) if q.running => {
                    tracing::info!(queue = %q.name, "schedule stop time reached");
                    self.stop_queue_locked(&q.id).await?;
                    changed = true;
                }
                _ => {}
            }
        }

        // 2. Power policy.
        let due = self
            .inner
            .state
            .lock()
            .power_checked
            .is_none_or(|t| t.elapsed() >= self.inner.cfg.power_interval);
        if due {
            let src = self.inner.cfg.power.clone();
            let st = tokio::task::spawn_blocking(move || src.state())
                .await
                .unwrap_or_default();
            let hold = power::hold(&mgr.settings().power, &st);
            let (before, held) = {
                let mut s = self.inner.state.lock();
                s.power_checked = Some(Instant::now());
                let before = s.hold;
                s.hold = hold;
                let held: Vec<DownloadId> = if hold.is_some() && before.is_none() {
                    s.started_by.drain().map(|(d, _)| d).collect()
                } else {
                    Vec::new()
                };
                (before, held)
            };
            if before != hold {
                tracing::info!(?hold, "queue power hold changed");
                let _ = self.inner.events.send(SchedulerEvent::PowerHold { hold });
            }
            for d in held {
                mgr.requeue(d).await?;
            }
        }

        // 3. Start what may start.
        let queues = self.queues()?;
        let downloads = mgr.list();
        let running: HashSet<DownloadId> = mgr.running_ids().into_iter().collect();
        let hold = self.inner.state.lock().hold.is_some();
        let start = plan::plan(&plan::Inputs {
            queues: &queues,
            downloads: &downloads,
            running: &running,
            global_limit: mgr.settings().downloads.max_concurrent.max(1) as usize,
            now_ms: clock.now_ms(),
            hold,
        });
        for id in start {
            let Some(d) = downloads.iter().find(|d| d.id == id) else {
                continue;
            };
            let qid = plan::queue_of(d).to_string();
            match mgr.start(id) {
                Ok(()) => {
                    let mut st = self.inner.state.lock();
                    st.started_by.insert(id, qid.clone());
                    if queues.iter().any(|q| q.id == qid && q.running) {
                        st.batches.entry(qid).or_default().started.push(id);
                    }
                }
                Err(e) => tracing::warn!(%id, error = %e, "queued download could not start"),
            }
        }

        // 4. Finished batches → post-completion actions.
        let downloads = mgr.list();
        let running: HashSet<DownloadId> = mgr.running_ids().into_iter().collect();
        let finished: Vec<(QueueInfo, Vec<DownloadId>)> = {
            let mut st = self.inner.state.lock();
            st.started_by.retain(|d, _| running.contains(d));
            let done: Vec<&QueueInfo> = queues
                .iter()
                .filter(|q| q.running)
                .filter(|q| st.batches.get(&q.id).is_some_and(|b| !b.started.is_empty()))
                .filter(|q| plan::queue_idle(q, &downloads, &running))
                .collect();
            done.into_iter()
                .map(|q| {
                    (
                        q.clone(),
                        st.batches.remove(&q.id).unwrap_or_default().started,
                    )
                })
                .collect()
        };
        for (q, ids) in finished {
            let status = |s: DownloadStatus| {
                ids.iter()
                    .filter(|id| downloads.iter().any(|d| d.id == **id && d.status == s))
                    .count() as u32
            };
            let (completed, failed) = (
                status(DownloadStatus::Completed),
                status(DownloadStatus::Failed),
            );
            tracing::info!(queue = %q.name, completed, failed, post_action = ?q.post_action, "queue finished");
            let _ = self.inner.events.send(SchedulerEvent::QueueFinished {
                queue_id: q.id.clone(),
                name: q.name.clone(),
                post_action: q.post_action,
                completed,
                failed,
            });
        }
        if changed {
            if let Ok(queues) = self.queues() {
                let _ = self
                    .inner
                    .events
                    .send(SchedulerEvent::QueuesChanged { queues });
            }
        }
        Ok(())
    }
}
