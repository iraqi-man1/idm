//! Queue processing against the real download manager and test server.
//! Time and power state are injected so schedules and power holds can be
//! exercised without waiting for wall-clock times.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use parking_lot::Mutex;
use velox_core::{DownloadManager, ManagerConfig};
use velox_persistence::{Database, SecretBox};
use velox_scheduler::{Clock, PowerSource, PowerState, Scheduler, SchedulerConfig};
use velox_test_server::{expected_content, TestServer};
use velox_types::{
    AddDownloadRequest, DownloadId, DownloadStatus, PostAction, PowerHold, ProxyMode, QueueUpdate,
    Schedule, SchedulerEvent, StartMode, MAIN_QUEUE_ID,
};

struct ManualClock {
    local: Mutex<NaiveDateTime>,
    ms: Mutex<i64>,
}

impl ManualClock {
    fn new(local: &str) -> Arc<Self> {
        Arc::new(Self {
            local: Mutex::new(NaiveDateTime::parse_from_str(local, "%Y-%m-%d %H:%M:%S").unwrap()),
            ms: Mutex::new(1_700_000_000_000),
        })
    }
    fn set(&self, local: &str) {
        *self.local.lock() = NaiveDateTime::parse_from_str(local, "%Y-%m-%d %H:%M:%S").unwrap();
    }
    fn advance_ms(&self, ms: i64) {
        *self.ms.lock() += ms;
    }
}

impl Clock for ManualClock {
    fn now_local(&self) -> NaiveDateTime {
        *self.local.lock()
    }
    fn now_ms(&self) -> i64 {
        *self.ms.lock()
    }
}

#[derive(Default)]
struct FakePower(Mutex<PowerState>);

impl PowerSource for FakePower {
    fn state(&self) -> PowerState {
        *self.0.lock()
    }
}

struct Env {
    _dir: tempfile::TempDir,
    downloads: PathBuf,
    server: TestServer,
    mgr: DownloadManager,
    sched: Scheduler,
    clock: Arc<ManualClock>,
    power: Arc<FakePower>,
}

impl Env {
    async fn new(global_limit: u32, f: impl FnOnce(&mut velox_types::AppSettings)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let downloads = dir.path().join("downloads");
        let db = Database::open(&dir.path().join("velox.sqlite")).unwrap();
        let mut s = db.load_settings().unwrap();
        s.general.download_dir = downloads.to_string_lossy().to_string();
        s.network.proxy.mode = ProxyMode::None;
        s.downloads.max_concurrent = global_limit;
        s.downloads.retry_delay_secs = 1;
        s.downloads.max_retries = 0;
        f(&mut s);
        db.save_settings(&s.normalized()).unwrap();
        let (mgr, _) = DownloadManager::open(ManagerConfig {
            db,
            secret_box: Some(SecretBox::new(&SecretBox::generate_key())),
            proxy_password: None,
            known_hosts: None,
        })
        .await
        .unwrap();
        let clock = ManualClock::new("2026-10-05 12:00:00");
        let power = Arc::new(FakePower::default());
        let sched = Scheduler::start(
            mgr.clone(),
            SchedulerConfig {
                tick: Duration::from_millis(100),
                power_interval: Duration::from_millis(100),
                clock: clock.clone(),
                power: power.clone(),
            },
        );
        Self {
            _dir: dir,
            downloads,
            server: TestServer::start().await,
            mgr,
            sched,
            clock,
            power,
        }
    }

    async fn add(
        &self,
        name: &str,
        size: u64,
        rate: u64,
        queue: &str,
        start: StartMode,
    ) -> DownloadId {
        self.mgr
            .add(AddDownloadRequest {
                url: self
                    .server
                    .url(&format!("/file/{name}?size={size}&rate={rate}")),
                queue_id: Some(queue.into()),
                connections: Some(1),
                start,
                ..Default::default()
            })
            .await
            .unwrap()
            .id
    }

    fn status(&self, id: DownloadId) -> DownloadStatus {
        self.mgr.get(id).unwrap().status
    }

    fn assert_file(&self, name: &str, size: u64) {
        let data = std::fs::read(self.downloads.join(name)).unwrap();
        assert!(
            data == expected_content(self.server.seed_for(name), size),
            "content of {name}"
        );
    }
}

async fn until(timeout: Duration, what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Polls the number of running downloads until all are complete and
/// returns the maximum seen.
async fn max_running_until_done(env: &Env, ids: &[DownloadId]) -> usize {
    let max = AtomicUsize::new(0);
    until(Duration::from_secs(60), "all downloads complete", || {
        max.fetch_max(env.mgr.running_count(), Ordering::SeqCst);
        ids.iter()
            .all(|id| env.status(*id) == DownloadStatus::Completed)
    })
    .await;
    max.load(Ordering::SeqCst)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn main_queue_runs_waiting_downloads_within_its_limit() {
    let env = Env::new(10, |_| {}).await;
    env.sched
        .update_queue(
            MAIN_QUEUE_ID,
            QueueUpdate {
                max_concurrent: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(
            env.add(
                &format!("q{i}.bin"),
                300_000,
                300_000,
                MAIN_QUEUE_ID,
                StartMode::Queue,
            )
            .await,
        );
    }
    until(Duration::from_secs(10), "two running", || {
        env.mgr.running_count() == 2
    })
    .await;
    assert_eq!(max_running_until_done(&env, &ids).await, 2);
    for i in 0..5 {
        env.assert_file(&format!("q{i}.bin"), 300_000);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_limit_applies_across_queues() {
    let env = Env::new(1, |_| {}).await;
    let night = env.sched.create_queue("Night").unwrap();
    env.sched.start_queue(&night.id).await.unwrap();
    let ids = vec![
        env.add("a.bin", 200_000, 400_000, MAIN_QUEUE_ID, StartMode::Queue)
            .await,
        env.add("b.bin", 200_000, 400_000, &night.id, StartMode::Queue)
            .await,
        env.add("c.bin", 200_000, 400_000, &night.id, StartMode::Queue)
            .await,
    ];
    assert_eq!(max_running_until_done(&env, &ids).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_queue_puts_its_downloads_back_and_starting_resumes_them() {
    let env = Env::new(4, |_| {}).await;
    let q = env.sched.create_queue("Big files").unwrap();
    let id = env
        .add("big.bin", 3_000_000, 600_000, &q.id, StartMode::Queue)
        .await;
    // Created queues are stopped: nothing starts.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(env.status(id), DownloadStatus::Queued);
    env.sched.start_queue(&q.id).await.unwrap();
    until(Duration::from_secs(10), "download progressing", || {
        env.mgr.get(id).unwrap().downloaded > 300_000
    })
    .await;
    env.sched.stop_queue(&q.id).await.unwrap();
    assert_eq!(env.status(id), DownloadStatus::Queued);
    assert!(!env.mgr.is_running(id));
    let kept = env.mgr.get(id).unwrap().downloaded;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!env.mgr.is_running(id), "stopped queue must not restart it");
    env.sched.start_queue(&q.id).await.unwrap();
    until(Duration::from_secs(30), "completed", || {
        env.status(id) == DownloadStatus::Completed
    })
    .await;
    env.assert_file("big.bin", 3_000_000);
    assert!(kept > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn schedule_starts_and_stops_a_queue() {
    let env = Env::new(4, |_| {}).await;
    let q = env.sched.create_queue("Night").unwrap();
    env.sched
        .update_queue(
            &q.id,
            QueueUpdate {
                schedule: Some(Schedule {
                    enabled: true,
                    start_time: Some("02:00".into()),
                    stop_time: Some("03:00".into()),
                    days: vec![],
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let mut events = env.sched.subscribe();
    let id = env
        .add("night.bin", 4_000_000, 400_000, &q.id, StartMode::Queue)
        .await;
    env.clock.set("2026-10-06 01:59:58");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(env.status(id), DownloadStatus::Queued);

    env.clock.set("2026-10-06 02:00:01");
    until(Duration::from_secs(5), "started at 02:00", || {
        env.mgr.is_running(id)
    })
    .await;
    assert!(env
        .sched
        .queues()
        .unwrap()
        .iter()
        .any(|x| x.id == q.id && x.running));

    env.clock.set("2026-10-06 03:00:01");
    until(Duration::from_secs(5), "stopped at 03:00", || {
        !env.mgr.is_running(id)
    })
    .await;
    assert_eq!(env.status(id), DownloadStatus::Queued);
    assert!(env
        .sched
        .queues()
        .unwrap()
        .iter()
        .any(|x| x.id == q.id && !x.running));
    let mut saw_change = false;
    while let Ok(e) = events.try_recv() {
        saw_change |= matches!(e, SchedulerEvent::QueuesChanged { .. });
    }
    assert!(saw_change);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduled_download_starts_at_its_time() {
    let env = Env::new(4, |_| {}).await;
    let q = env.sched.create_queue("Stopped queue").unwrap();
    let at = env.clock.now_ms() + 60_000;
    let id = env
        .mgr
        .add(AddDownloadRequest {
            url: env.server.url("/file/later.bin?size=100000"),
            queue_id: Some(q.id.clone()),
            start: StartMode::Queue,
            scheduled_at: Some(at),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(env.status(id), DownloadStatus::Queued);
    env.clock.advance_ms(61_000);
    until(Duration::from_secs(10), "scheduled download done", || {
        env.status(id) == DownloadStatus::Completed
    })
    .await;
    assert_eq!(env.mgr.get(id).unwrap().scheduled_at, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finished_queue_reports_its_post_action_once() {
    let env = Env::new(4, |_| {}).await;
    let q = env.sched.create_queue("Then shut down").unwrap();
    env.sched
        .update_queue(
            &q.id,
            QueueUpdate {
                post_action: Some(PostAction::Shutdown),
                ..Default::default()
            },
        )
        .unwrap();
    let mut events = env.sched.subscribe();
    let ok = env
        .add("one.bin", 100_000, 0, &q.id, StartMode::Queue)
        .await;
    let bad = env
        .mgr
        .add(AddDownloadRequest {
            url: env.server.url("/file/missing.bin?size=10&status=404"),
            queue_id: Some(q.id.clone()),
            start: StartMode::Queue,
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    env.sched.start_queue(&q.id).await.unwrap();
    let finished = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(SchedulerEvent::QueueFinished {
                queue_id,
                post_action,
                completed,
                failed,
                ..
            }) = events.recv().await
            {
                return (queue_id, post_action, completed, failed);
            }
        }
    })
    .await
    .expect("queue finished event");
    assert_eq!(finished, (q.id.clone(), PostAction::Shutdown, 1, 1));
    assert_eq!(env.status(ok), DownloadStatus::Completed);
    assert_eq!(env.status(bad), DownloadStatus::Failed);
    // Only once per batch.
    tokio::time::sleep(Duration::from_millis(600)).await;
    while let Ok(e) = events.try_recv() {
        assert!(
            !matches!(e, SchedulerEvent::QueueFinished { .. }),
            "duplicate finished event"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn low_battery_holds_queue_processing() {
    let env = Env::new(4, |s| {
        s.power.pause_on_low_battery = true;
        s.power.battery_threshold = 20;
    })
    .await;
    let mut events = env.sched.subscribe();
    let id = env
        .add(
            "power.bin",
            3_000_000,
            600_000,
            MAIN_QUEUE_ID,
            StartMode::Queue,
        )
        .await;
    until(Duration::from_secs(10), "running on AC", || {
        env.mgr.get(id).unwrap().downloaded > 200_000
    })
    .await;

    *env.power.0.lock() = PowerState {
        on_battery: true,
        battery_percent: Some(10),
        metered: false,
    };
    until(Duration::from_secs(5), "held on low battery", || {
        !env.mgr.is_running(id)
    })
    .await;
    assert_eq!(env.status(id), DownloadStatus::Queued);
    assert_eq!(env.sched.power_hold(), Some(PowerHold::LowBattery));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!env.mgr.is_running(id), "must stay held");

    *env.power.0.lock() = PowerState {
        on_battery: false,
        battery_percent: Some(11),
        metered: false,
    };
    until(
        Duration::from_secs(30),
        "completed after plugging in",
        || env.status(id) == DownloadStatus::Completed,
    )
    .await;
    env.assert_file("power.bin", 3_000_000);
    let mut holds = Vec::new();
    while let Ok(e) = events.try_recv() {
        if let SchedulerEvent::PowerHold { hold } = e {
            holds.push(hold);
        }
    }
    assert_eq!(holds, vec![Some(PowerHold::LowBattery), None]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn starting_a_queue_retries_its_failed_downloads_when_enabled() {
    let env = Env::new(4, |_| {}).await;
    let q = env.sched.create_queue("Retry").unwrap();
    env.sched
        .update_queue(
            &q.id,
            QueueUpdate {
                retry_failed: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    let id = env
        .mgr
        .add(AddDownloadRequest {
            url: env.server.url("/file/flaky.bin?size=50000&status=500"),
            queue_id: Some(q.id.clone()),
            start: StartMode::Queue,
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    env.sched.start_queue(&q.id).await.unwrap();
    until(Duration::from_secs(10), "failed", || {
        env.status(id) == DownloadStatus::Failed
    })
    .await;
    let before = env
        .server
        .stats("flaky.bin")
        .requests
        .load(Ordering::SeqCst);
    env.sched.start_queue(&q.id).await.unwrap();
    until(Duration::from_secs(10), "tried again", || {
        env.server
            .stats("flaky.bin")
            .requests
            .load(Ordering::SeqCst)
            > before
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_a_queue_moves_its_downloads_to_main() {
    let env = Env::new(4, |_| {}).await;
    let q = env.sched.create_queue("Temporary").unwrap();
    let id = env
        .add("moved.bin", 1000, 0, &q.id, StartMode::Paused)
        .await;
    assert!(env.sched.delete_queue(MAIN_QUEUE_ID).await.is_err());
    env.sched.delete_queue(&q.id).await.unwrap();
    assert_eq!(
        env.mgr.get(id).unwrap().queue_id.as_deref(),
        Some(MAIN_QUEUE_ID)
    );
    assert!(env.sched.queues().unwrap().iter().all(|x| x.id != q.id));
    assert!(env
        .sched
        .update_queue(&q.id, QueueUpdate::default())
        .is_err());
    let bad = QueueUpdate {
        schedule: Some(Schedule {
            enabled: true,
            start_time: Some("25:00".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(env.sched.update_queue(MAIN_QUEUE_ID, bad).is_err());
}
