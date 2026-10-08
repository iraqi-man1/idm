//! End-to-end tests of the download engine against the local test server.
//! Every test performs real HTTP transfers over localhost sockets and
//! verifies the resulting file byte-for-byte.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use velox_core::{DownloadManager, ManagerConfig};
use velox_persistence::{Database, SecretBox};
use velox_test_server::{expected_content, TestServer};
use velox_types::{
    AddDownloadRequest, AppSettings, ChecksumAlgorithm, ChecksumSpec, DownloadInfo, DownloadStatus,
    ErrorKind, ProxyMode, StartMode,
};

const MIB: u64 = 1024 * 1024;

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    db_path: PathBuf,
    key: [u8; 32],
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        Self {
            db_path: root.join("velox.sqlite"),
            root,
            _dir: dir,
            key: SecretBox::generate_key(),
        }
    }

    fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }

    async fn manager_with(
        &self,
        f: impl FnOnce(&mut AppSettings),
    ) -> (DownloadManager, Vec<uuid::Uuid>) {
        let db = Database::open(&self.db_path).unwrap();
        let mut s = db.load_settings().unwrap();
        s.general.download_dir = self.downloads().to_string_lossy().to_string();
        s.network.proxy.mode = ProxyMode::None;
        s.network.read_timeout_secs = 10;
        s.downloads.retry_delay_secs = 1;
        s.downloads.max_retries = 5;
        s.downloads.min_segment_size = 256 * 1024;
        f(&mut s);
        db.save_settings(&s.normalized()).unwrap();
        DownloadManager::open(ManagerConfig {
            db,
            secret_box: Some(SecretBox::new(&self.key)),
            proxy_password: None,
        })
        .await
        .unwrap()
    }

    async fn manager(&self) -> DownloadManager {
        self.manager_with(|_| {}).await.0
    }
}

fn req(url: String) -> AddDownloadRequest {
    AddDownloadRequest {
        url,
        ..Default::default()
    }
}

async fn wait_until(
    mgr: &DownloadManager,
    id: uuid::Uuid,
    timeout: Duration,
    pred: impl Fn(&DownloadInfo) -> bool,
) -> DownloadInfo {
    let start = Instant::now();
    loop {
        let info = mgr.get(id).expect("download exists");
        if pred(&info) {
            return info;
        }
        if start.elapsed() > timeout {
            panic!(
                "timeout waiting; last state: {:?} error={:?} downloaded={}",
                info.status, info.error, info.downloaded
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_finished(mgr: &DownloadManager, id: uuid::Uuid, timeout: Duration) -> DownloadInfo {
    wait_until(mgr, id, timeout, |i| {
        i.status.is_terminal() && !mgr.is_running(id)
    })
    .await
}

fn assert_content(path: &Path, seed: u64, size: u64) {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_eq!(data.len() as u64, size, "size of {}", path.display());
    let expected = expected_content(seed, size);
    if data != expected {
        let first = data
            .iter()
            .zip(&expected)
            .position(|(a, b)| a != b)
            .unwrap();
        panic!("content mismatch at byte {first} in {}", path.display());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn segmented_download_uses_multiple_connections() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = 24 * MIB;
    let mut r = req(srv.url(&format!("/file/seg.bin?size={size}&rate=6000000")));
    r.connections = Some(8);
    let info = mgr.add(r).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(60)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_eq!(done.file_name, "seg.bin");
    assert_eq!(done.total_size, Some(size));
    assert_eq!(done.resumable, Some(true));
    assert_content(&done.file_path(), srv.seed_for("seg.bin"), size);
    let st = srv.stats("seg.bin");
    assert!(
        st.max_active.load(Ordering::SeqCst) >= 3,
        "expected parallel connections, got {}",
        st.max_active.load(Ordering::SeqCst)
    );
    // No byte transferred twice beyond what connection hand-over costs.
    assert!(st.bytes_sent.load(Ordering::SeqCst) < size + size / 4);
    assert!(!velox_core::naming::partial_path(&done.file_path()).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_without_range_support_downloads_with_one_connection() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = 3 * MIB + 17;
    let info = mgr
        .add(req(
            srv.url(&format!("/file/norange.bin?size={size}&norange=1"))
        ))
        .await
        .unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_eq!(done.resumable, Some(false));
    assert_eq!(
        srv.stats("norange.bin").max_active.load(Ordering::SeqCst),
        1
    );
    assert_content(&done.file_path(), srv.seed_for("norange.bin"), size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_length_chunked_download() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = 2 * MIB + 5;
    let info = mgr
        .add(req(
            srv.url(&format!("/file/chunked.bin?size={size}&nolength=1"))
        ))
        .await
        .unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_eq!(done.total_size, Some(size));
    assert_content(&done.file_path(), srv.seed_for("chunked.bin"), size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn empty_file() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let info = mgr
        .add(req(srv.url("/file/empty.txt?size=0")))
        .await
        .unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(10)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_eq!(std::fs::metadata(done.file_path()).unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_and_resume_continue_where_they_stopped() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = 8 * MIB;
    let mut r = req(srv.url(&format!("/file/pause.bin?size={size}&rate=1500000")));
    r.connections = Some(4);
    let info = mgr.add(r).await.unwrap();
    wait_until(&mgr, info.id, Duration::from_secs(20), |i| {
        i.downloaded > 2 * MIB
    })
    .await;
    mgr.pause(info.id).await.unwrap();
    let paused = mgr.get(info.id).unwrap();
    assert_eq!(paused.status, DownloadStatus::Paused);
    assert!(
        paused.downloaded > MIB && paused.downloaded < size,
        "{}",
        paused.downloaded
    );
    let sent_before = srv.stats("pause.bin").bytes_sent.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Nothing is transferred while paused.
    assert_eq!(
        srv.stats("pause.bin").bytes_sent.load(Ordering::SeqCst),
        sent_before
    );

    mgr.start(info.id).unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(60)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_content(&done.file_path(), srv.seed_for("pause.bin"), size);
    // Resume must not re-download what was already on disk.
    let total_sent = srv.stats("pause.bin").bytes_sent.load(Ordering::SeqCst);
    assert!(total_sent < size + size / 3, "sent {total_sent} for {size}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovers_after_crash_from_durable_checkpoint() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let size = 10 * MIB;
    let url = srv.url(&format!("/file/crash.bin?size={size}&rate=400000"));

    // Run the first manager on its own runtime and kill the runtime
    // abruptly, as a crash or power loss would (no graceful shutdown).
    let id = {
        let env_ref = &env;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let (id, downloaded) = std::thread::scope(|s| {
            s.spawn(|| {
                rt.block_on(async {
                    let mgr = env_ref.manager().await;
                    let mut r = req(url.clone());
                    r.connections = Some(4);
                    let info = mgr.add(r).await.unwrap();
                    // Wait for at least one checkpoint with real progress.
                    let i = wait_until(&mgr, info.id, Duration::from_secs(30), |i| {
                        i.downloaded > 3 * MIB && mgr.record(info.id).unwrap().downloaded > 2 * MIB
                    })
                    .await;
                    (info.id, i.downloaded)
                })
            })
            .join()
            .unwrap()
        });
        rt.shutdown_background();
        assert!(downloaded < size);
        id
    };

    let (mgr, interrupted) = env.manager_with(|_| {}).await;
    assert_eq!(
        interrupted,
        vec![id],
        "interrupted download is reported for resumption"
    );
    let rec = mgr.record(id).unwrap();
    assert_eq!(rec.status, DownloadStatus::Paused);
    assert!(rec.downloaded > 0);
    mgr.start(id).unwrap();
    let done = wait_finished(&mgr, id, Duration::from_secs(60)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_content(&done.file_path(), srv.seed_for("crash.bin"), size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_connections_are_retried() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = 6 * MIB;
    // The first 6 responses are cut after 700 KB.
    let mut r = req(srv.url(&format!(
        "/file/flaky.bin?size={size}&fail_after=700000&fail_times=6"
    )));
    r.connections = Some(2);
    let info = mgr.add(r).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(60)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_content(&done.file_path(), srv.seed_for("flaky.bin"), size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn respects_server_connection_limit() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env
        .manager_with(|s| s.downloads.adaptive_connections = false)
        .await
        .0;
    let size = 12 * MIB;
    let mut r = req(srv.url(&format!(
        "/file/limited.bin?size={size}&maxconn=2&rate=3000000"
    )));
    r.connections = Some(8);
    let info = mgr.add(r).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(60)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_content(&done.file_path(), srv.seed_for("limited.bin"), size);
    let st = srv.stats("limited.bin");
    assert!(st.max_active.load(Ordering::SeqCst) <= 2);
    assert!(
        st.rejected.load(Ordering::SeqCst) >= 1,
        "the limit was hit and handled"
    );
    let log = mgr.log(info.id).unwrap();
    assert!(
        log.iter()
            .any(|l| l.message.contains("refused an extra connection")),
        "{log:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_change_is_detected_on_resume() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = 6 * MIB;
    let info = mgr
        .add(req(srv.url(&format!(
            "/file/changing.bin?size={size}&rate=1500000"
        ))))
        .await
        .unwrap();
    wait_until(&mgr, info.id, Duration::from_secs(20), |i| {
        i.downloaded > MIB
    })
    .await;
    mgr.pause(info.id).await.unwrap();
    let partial = mgr.record(info.id).unwrap().temp_path.unwrap();
    let partial_before = std::fs::read(&partial).unwrap();

    srv.bump("changing.bin");
    mgr.start(info.id).unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(20)).await;
    assert_eq!(done.status, DownloadStatus::Failed);
    assert_eq!(done.error_kind, Some(ErrorKind::RemoteChanged));
    // The partial file was not touched with data from the new version.
    assert_eq!(std::fs::read(&partial).unwrap(), partial_before);

    // Restart downloads the new version completely.
    mgr.restart(info.id).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(60)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_content(&done.file_path(), srv.seed_for("changing.bin"), size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_link_can_be_refreshed() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = 6 * MIB;
    // Same content (explicit seed) under two names: the first link expires.
    let first = srv.url(&format!(
        "/file/exp1.bin?size={size}&seed=99&rate=1500000&expire_after=1"
    ));
    let mut r = req(first);
    r.connections = Some(1);
    let info = mgr.add(r).await.unwrap();
    wait_until(&mgr, info.id, Duration::from_secs(20), |i| {
        i.downloaded > MIB
    })
    .await;
    mgr.pause(info.id).await.unwrap();
    mgr.start(info.id).unwrap();
    let failed = wait_finished(&mgr, info.id, Duration::from_secs(20)).await;
    assert_eq!(failed.status, DownloadStatus::Failed, "{:?}", failed.error);
    assert_eq!(
        failed.error_kind,
        Some(ErrorKind::LinkExpired),
        "{:?}",
        failed.error
    );

    mgr.update_url(
        info.id,
        &srv.url(&format!("/file/exp2.bin?size={size}&seed=99")),
    )
    .unwrap();
    mgr.start(info.id).unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_content(&done.file_path(), 99, size);
    // Only the remainder was fetched from the new address.
    assert!(srv.stats("exp2.bin").bytes_sent.load(Ordering::SeqCst) < size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_content_range_falls_back_safely() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = 2 * MIB;
    let info = mgr
        .add(req(
            srv.url(&format!("/file/bad.bin?size={size}&malformed=1"))
        ))
        .await
        .unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_eq!(done.resumable, Some(false));
    assert_content(&done.file_path(), srv.seed_for("bad.bin"), size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_errors_fail_or_retry_appropriately() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager_with(|s| s.downloads.max_retries = 2).await.0;

    let info = mgr
        .add(req(srv.url("/file/missing.bin?size=10&status=404")))
        .await
        .unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(10)).await;
    assert_eq!(done.status, DownloadStatus::Failed);
    assert_eq!(done.error_kind, Some(ErrorKind::Http));
    assert_eq!(
        srv.stats("missing.bin").requests.load(Ordering::SeqCst),
        1,
        "404 is not retried"
    );

    let info = mgr
        .add(req(srv.url("/file/busy.bin?size=10&status=503")))
        .await
        .unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Failed);
    assert_eq!(
        srv.stats("busy.bin").requests.load(Ordering::SeqCst),
        3,
        "503 retried max_retries times"
    );
    assert_eq!(done.retry_count, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_connection_times_out_and_recovers() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env
        .manager_with(|s| s.network.read_timeout_secs = 5)
        .await
        .0;
    let size = MIB;
    let mut r = req(srv.url(&format!(
        "/file/stall.bin?size={size}&stall_after=300000&stall_times=1"
    )));
    r.connections = Some(1);
    let info = mgr.add(r).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(40)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_content(&done.file_path(), srv.seed_for("stall.bin"), size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_removes_partial_data() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let info = mgr
        .add(req(srv.url("/file/cancel.bin?size=8000000&rate=1000000")))
        .await
        .unwrap();
    wait_until(&mgr, info.id, Duration::from_secs(20), |i| {
        i.downloaded > 500_000
    })
    .await;
    let partial = mgr.record(info.id).unwrap().temp_path.unwrap();
    assert!(Path::new(&partial).exists());
    mgr.cancel(info.id).await.unwrap();
    let c = mgr.get(info.id).unwrap();
    assert_eq!(c.status, DownloadStatus::Cancelled);
    assert!(!Path::new(&partial).exists());
    assert!(mgr.db().load_segments(info.id).unwrap().is_empty());

    mgr.remove(info.id, false).await.unwrap();
    assert!(mgr.get(info.id).is_none());
    assert!(mgr.db().get_download(info.id).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checksum_verification() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = MIB + 3;
    let content = expected_content(7, size);
    use sha2::Digest;
    let good = hex::encode(sha2::Sha256::digest(&content));

    let mut r = req(srv.url(&format!("/file/sum.bin?size={size}&seed=7")));
    r.checksum = Some(ChecksumSpec {
        algorithm: ChecksumAlgorithm::Sha256,
        expected: good.to_uppercase(),
    });
    let info = mgr.add(r).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(20)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_eq!(done.checksum_ok, Some(true));

    let mut r = req(srv.url(&format!("/file/sum2.bin?size={size}&seed=8")));
    r.checksum = Some(ChecksumSpec {
        algorithm: ChecksumAlgorithm::Sha256,
        expected: good.clone(),
    });
    let info = mgr.add(r).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(20)).await;
    assert_eq!(done.status, DownloadStatus::Failed);
    assert_eq!(done.error_kind, Some(ErrorKind::ChecksumMismatch));
    assert_eq!(done.checksum_ok, Some(false));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn names_from_headers_are_sanitized_and_conflicts_renamed() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    // Content-Disposition tries to escape the download folder.
    let info = mgr
        .add(req(srv.url("/file/x?size=100&cd=..%2F..%2Fevil.exe")))
        .await
        .unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(10)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_eq!(done.file_name, "evil.exe");
    assert_eq!(done.file_path().parent().unwrap(), env.downloads());

    // Unicode (Arabic) names survive; a second download gets "(1)".
    for expect in ["تقرير.pdf", "تقرير (1).pdf"] {
        let info = mgr
            .add(req(srv.url(
                "/file/y?size=100&cd=%D8%AA%D9%82%D8%B1%D9%8A%D8%B1.pdf",
            )))
            .await
            .unwrap();
        let done = wait_finished(&mgr, info.id, Duration::from_secs(10)).await;
        assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
        assert_eq!(done.file_name, expect);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redirects_auth_and_cookies() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let info = mgr
        .add(req(srv.url("/redirect?n=3&to=/file/r.bin%3Fsize%3D50000")))
        .await
        .unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(10)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert!(done.final_url.unwrap().contains("/file/r.bin"));
    assert_eq!(done.file_name, "r.bin");

    let mut r = req(srv.url("/file/a.bin?size=1000000&auth=1&cookie=session%3Dok"));
    r.credentials = Some(velox_types::Credentials {
        username: "user".into(),
        password: "pass".into(),
    });
    r.cookies = Some("session=ok; other=1".into());
    let info = mgr.add(r).await.unwrap();
    assert!(info.has_secrets);
    // Secrets are encrypted at rest.
    let rec = mgr.record(info.id).unwrap();
    assert!(rec.secret_id.is_some());
    let done = wait_finished(&mgr, info.id, Duration::from_secs(10)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);

    let mut r = req(srv.url("/file/b.bin?size=1000&auth=1"));
    r.credentials = Some(velox_types::Credentials {
        username: "user".into(),
        password: "wrong".into(),
    });
    let info = mgr.add(r).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(10)).await;
    assert_eq!(done.status, DownloadStatus::Failed);
    assert_eq!(done.error_kind, Some(ErrorKind::Auth));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_downloads() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let mut ids = Vec::new();
    for i in 0..6 {
        let size = MIB * (i + 1) + i;
        let info = mgr
            .add(req(srv.url(&format!("/file/c{i}.bin?size={size}"))))
            .await
            .unwrap();
        ids.push((info.id, format!("c{i}.bin"), size));
    }
    for (id, name, size) in ids {
        let done = wait_finished(&mgr, id, Duration::from_secs(60)).await;
        assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
        assert_content(&done.file_path(), srv.seed_for(&name), size);
    }
    assert_eq!(mgr.running_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_speed_limit_is_enforced() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env
        .manager_with(|s| s.downloads.speed_limit = 1_000_000)
        .await
        .0;
    let size = 3_000_000;
    let start = Instant::now();
    let mut r = req(srv.url(&format!("/file/limit.bin?size={size}")));
    r.connections = Some(4);
    let info = mgr.add(r).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    let secs = start.elapsed().as_secs_f64();
    assert!(secs > 2.3, "3 MB at 1 MB/s took only {secs:.2}s");
    assert_content(&done.file_path(), srv.seed_for("limit.bin"), size);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_and_paused_adds_do_not_start() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let mut r = req(srv.url("/file/q.bin?size=1000"));
    r.start = StartMode::Queue;
    let q = mgr.add(r).await.unwrap();
    let mut r = req(srv.url("/file/p.bin?size=1000"));
    r.start = StartMode::Paused;
    let p = mgr.add(r).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(mgr.get(q.id).unwrap().status, DownloadStatus::Queued);
    assert_eq!(mgr.get(p.id).unwrap().status, DownloadStatus::Paused);
    assert_eq!(srv.stats("q.bin").requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        mgr.find_duplicates(&srv.url("/file/q.bin?size=1000")),
        vec![q.id]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_url_reports_metadata() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let info = mgr
        .probe_url(&req(srv.url("/file/movie?size=4096&mime=video%2Fmp4")))
        .await
        .unwrap();
    assert_eq!(info.file_name, "movie.mp4");
    assert_eq!(info.total_size, Some(4096));
    assert!(info.resumable);
    assert_eq!(info.category, velox_types::Category::Video);
}

/// Multi-gigabyte download (sparse pre-allocation, offsets > 4 GiB).
/// Run with `cargo test -p velox-core --release -- --ignored large_file`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn large_file_over_4gib() {
    let srv = TestServer::start().await;
    let env = Env::new();
    let mgr = env.manager().await;
    let size = 4 * 1024 * MIB + 12345;
    let mut r = req(srv.url(&format!("/file/large.bin?size={size}")));
    r.connections = Some(16);
    let info = mgr.add(r).await.unwrap();
    let done = wait_finished(&mgr, info.id, Duration::from_secs(1800)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    // Spot-check content at several offsets, including beyond 4 GiB.
    let f = std::fs::File::open(done.file_path()).unwrap();
    assert_eq!(f.metadata().unwrap().len(), size);
    let seed = srv.seed_for("large.bin");
    for off in [0u64, 1 << 20, (1 << 32) - 3, 1 << 32, size - 4096] {
        let mut buf = vec![0u8; 4096.min((size - off) as usize)];
        velox_core::fsutil::read_exact_at(&f, &mut buf, off).unwrap();
        let mut exp = vec![0u8; buf.len()];
        velox_test_server::fill(seed, off, &mut exp);
        assert_eq!(buf, exp, "mismatch at offset {off}");
    }
}
