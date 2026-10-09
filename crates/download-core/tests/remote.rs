//! FTP and SFTP downloads through the manager against the in-process
//! test servers, verified byte for byte.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use velox_core::{DownloadManager, ManagerConfig};
use velox_persistence::{Database, SecretBox};
use velox_test_server::expected_content;
use velox_test_server::ftp::{FtpOptions, FtpServer};
use velox_test_server::sftp::SftpServer;
use velox_types::{
    AddDownloadRequest, Credentials, DownloadId, DownloadInfo, DownloadStatus, ErrorKind,
    ProxyMode, StartMode,
};

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn downloads(&self) -> PathBuf {
        self.dir.path().join("downloads")
    }

    async fn manager(&self) -> DownloadManager {
        let db = Database::open(&self.dir.path().join("velox.sqlite")).unwrap();
        let mut s = db.load_settings().unwrap();
        s.general.download_dir = self.downloads().to_string_lossy().to_string();
        s.network.proxy.mode = ProxyMode::None;
        s.network.read_timeout_secs = 10;
        s.downloads.retry_delay_secs = 1;
        s.downloads.max_retries = 5;
        s.downloads.min_segment_size = 256 * 1024;
        db.save_settings(&s.normalized()).unwrap();
        DownloadManager::open(ManagerConfig {
            db,
            secret_box: Some(SecretBox::new(&[9u8; 32])),
            proxy_password: None,
            known_hosts: Some(self.dir.path().join("known_hosts")),
        })
        .await
        .unwrap()
        .0
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
    id: DownloadId,
    secs: u64,
    pred: impl Fn(&DownloadInfo) -> bool,
) -> DownloadInfo {
    let start = Instant::now();
    loop {
        let info = mgr.get(id).unwrap();
        if pred(&info) {
            return info;
        }
        assert!(
            start.elapsed() < Duration::from_secs(secs),
            "timeout: {:?} {:?} {}",
            info.status,
            info.error,
            info.downloaded
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn finished(mgr: &DownloadManager, id: DownloadId) -> DownloadInfo {
    wait_until(mgr, id, 60, |i| {
        i.status.is_terminal() && !mgr.is_running(id)
    })
    .await
}

/// Everything SQLite has written for the database, including the WAL.
fn database_bytes(env: &Env) -> Vec<u8> {
    let base = env.dir.path().join("velox.sqlite");
    let mut raw = Vec::new();
    for suffix in ["", "-wal", "-journal"] {
        if let Ok(b) = std::fs::read(format!("{}{suffix}", base.display())) {
            raw.extend(b);
        }
    }
    raw
}

fn contains(raw: &[u8], needle: &[u8]) -> bool {
    raw.windows(needle.len()).any(|w| w == needle)
}

fn assert_file(env: &Env, name: &str, seed: u64, size: u64) {
    let data = std::fs::read(env.downloads().join(name)).unwrap();
    assert_eq!(data.len() as u64, size);
    assert!(data == expected_content(seed, size), "content of {name}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ftp_segmented_download_with_several_connections() {
    let env = Env::new();
    let server = FtpServer::start(FtpOptions {
        rate: Some(1_500_000),
        ..Default::default()
    })
    .await;
    let mgr = env.manager().await;
    let size = 8 * 1024 * 1024;
    let info = mgr
        .probe_url(&req(server.url(size, "big.iso")))
        .await
        .unwrap();
    assert_eq!(info.total_size, Some(size));
    assert!(info.resumable);
    let mut r = req(server.url(size, "big.iso"));
    r.connections = Some(4);
    let id = mgr.add(r).await.unwrap().id;
    let done = finished(&mgr, id).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_file(&env, "big.iso", server.seed_for("big.iso"), size);
    let offsets = server
        .state
        .rest_offsets
        .lock()
        .get("big.iso")
        .cloned()
        .unwrap();
    assert!(
        offsets.iter().filter(|o| **o > 0).count() >= 1,
        "ranges at offsets: {offsets:?}"
    );
    assert!(
        server.state.max_active.load(Ordering::SeqCst) >= 2,
        "several connections"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ftp_pause_and_resume_continue_where_they_stopped() {
    let env = Env::new();
    let server = FtpServer::start(FtpOptions {
        rate: Some(1_000_000),
        ..Default::default()
    })
    .await;
    let mgr = env.manager().await;
    let size = 4 * 1024 * 1024;
    let mut r = req(server.url(size, "p.bin"));
    r.connections = Some(1);
    let id = mgr.add(r).await.unwrap().id;
    wait_until(&mgr, id, 20, |i| i.downloaded > 1_000_000).await;
    mgr.pause(id).await.unwrap();
    let at = mgr.get(id).unwrap().downloaded;
    mgr.start(id).unwrap();
    let done = finished(&mgr, id).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_file(&env, "p.bin", server.seed_for("p.bin"), size);
    let offsets = server
        .state
        .rest_offsets
        .lock()
        .get("p.bin")
        .cloned()
        .unwrap();
    assert!(
        offsets.iter().any(|o| *o > 0 && *o <= at),
        "resumed with REST: {offsets:?} (paused at {at})"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ftp_changed_file_is_not_appended_to() {
    let env = Env::new();
    let server = FtpServer::start(FtpOptions {
        rate: Some(800_000),
        ..Default::default()
    })
    .await;
    let mgr = env.manager().await;
    let mut r = req(server.url(3_000_000, "c.bin"));
    r.connections = Some(1);
    let id = mgr.add(r).await.unwrap().id;
    wait_until(&mgr, id, 20, |i| i.downloaded > 500_000).await;
    mgr.pause(id).await.unwrap();
    server.bump("c.bin");
    mgr.start(id).unwrap();
    let done = finished(&mgr, id).await;
    assert_eq!(done.status, DownloadStatus::Failed);
    assert_eq!(
        done.error_kind,
        Some(ErrorKind::RemoteChanged),
        "{:?}",
        done.error
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ftp_connection_limit_and_broken_transfers() {
    let env = Env::new();
    let server = FtpServer::start(FtpOptions {
        max_connections: Some(2),
        fail_after: Some(700_000),
        fail_times: 2,
        rate: Some(2_000_000),
        ..Default::default()
    })
    .await;
    let mgr = env.manager().await;
    let size = 5 * 1024 * 1024;
    let mut r = req(server.url(size, "l.bin"));
    r.connections = Some(8);
    let id = mgr.add(r).await.unwrap().id;
    let done = finished(&mgr, id).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_file(&env, "l.bin", server.seed_for("l.bin"), size);
    assert!(
        server.state.max_active.load(Ordering::SeqCst) <= 3,
        "connection limit respected"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ftp_without_rest_downloads_on_one_connection() {
    let env = Env::new();
    let server = FtpServer::start(FtpOptions {
        no_rest: true,
        ..Default::default()
    })
    .await;
    let mgr = env.manager().await;
    let size = 2 * 1024 * 1024;
    let id = mgr.add(req(server.url(size, "n.bin"))).await.unwrap().id;
    let done = finished(&mgr, id).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_eq!(done.resumable, Some(false));
    assert_file(&env, "n.bin", server.seed_for("n.bin"), size);
    assert_eq!(server.state.retr.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn credentials_in_the_address_are_stored_encrypted_not_in_the_url() {
    let env = Env::new();
    let server = FtpServer::start(FtpOptions {
        login: Some(("carol".into(), "s3cret".into())),
        ..Default::default()
    })
    .await;
    let mgr = env.manager().await;
    let url = server.url(100_000, "auth.bin");
    assert!(url.contains("carol:s3cret@"));
    let info = mgr.add(req(url)).await.unwrap();
    assert!(
        !info.url.contains("s3cret") && !info.url.contains("carol"),
        "{}",
        info.url
    );
    assert!(info.has_secrets);
    let done = finished(&mgr, info.id).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_file(&env, "auth.bin", server.seed_for("auth.bin"), 100_000);
    let raw = database_bytes(&env);
    assert!(contains(&raw, b"auth.bin"), "the scan sees the record");
    assert!(!contains(&raw, b"s3cret"), "password stored in plain text");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changing_the_address_moves_its_login_to_encrypted_storage() {
    let env = Env::new();
    let server = FtpServer::start(FtpOptions {
        login: Some(("carol".into(), "s3cret".into())),
        ..Default::default()
    })
    .await;
    let mgr = env.manager().await;
    let with_login = server.url(100_000, "moved.bin");
    let without_login = with_login.replace("carol:s3cret@", "");
    assert_ne!(with_login, without_login);
    let info = mgr
        .add(AddDownloadRequest {
            start: StartMode::Paused,
            ..req(without_login)
        })
        .await
        .unwrap();
    assert!(!info.has_secrets);

    mgr.update_url(info.id, &with_login).unwrap();
    let updated = mgr.get(info.id).unwrap();
    assert!(
        !updated.url.contains("s3cret") && !updated.url.contains("carol"),
        "{}",
        updated.url
    );
    assert!(updated.has_secrets);
    mgr.start(info.id).unwrap();
    let done = finished(&mgr, info.id).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_file(&env, "moved.bin", server.seed_for("moved.bin"), 100_000);
    let raw = database_bytes(&env);
    assert!(contains(&raw, b"moved.bin"), "the scan sees the record");
    assert!(!contains(&raw, b"s3cret"), "password stored in plain text");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_address_naming_only_the_user_keeps_the_stored_password() {
    let env = Env::new();
    let server = FtpServer::start(FtpOptions {
        login: Some(("carol".into(), "s3cret".into())),
        ..Default::default()
    })
    .await;
    let mgr = env.manager().await;
    let with_login = server.url(100_000, "named.bin");
    let info = mgr
        .add(AddDownloadRequest {
            start: StartMode::Paused,
            credentials: Some(Credentials {
                username: "carol".into(),
                password: "s3cret".into(),
            }),
            ..req(with_login.replace("carol:s3cret@", ""))
        })
        .await
        .unwrap();
    // The usual way to write an address for an account: user name only.
    mgr.update_url(info.id, &with_login.replace("carol:s3cret@", "carol@"))
        .unwrap();
    mgr.start(info.id).unwrap();
    let done = finished(&mgr, info.id).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_file(&env, "named.bin", server.seed_for("named.bin"), 100_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sftp_segmented_download_and_resume() {
    let env = Env::new();
    let server = SftpServer::start("dave", "pw", None).await;
    let mgr = env.manager().await;
    let size = 6 * 1024 * 1024;
    let mut r = req(server.url(size, "s.bin"));
    r.connections = Some(4);
    r.speed_limit = Some(2_000_000);
    let id = mgr.add(r).await.unwrap().id;
    wait_until(&mgr, id, 30, |i| i.downloaded > 1_500_000).await;
    mgr.pause(id).await.unwrap();
    mgr.set_speed_limit(id, 0).unwrap();
    mgr.start(id).unwrap();
    let done = finished(&mgr, id).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_file(&env, "s.bin", server.seed_for("s.bin"), size);
    assert!(
        server.state.max_active.load(Ordering::SeqCst) >= 2,
        "several SFTP sessions"
    );
    let known = std::fs::read_to_string(env.dir.path().join("known_hosts")).unwrap();
    assert!(known.contains(&format!("[127.0.0.1]:{}", server.addr.port())));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sftp_changed_host_key_fails_the_download() {
    let env = Env::new();
    let server = SftpServer::start("dave", "pw", None).await;
    let other =
        russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519).unwrap();
    russh::keys::known_hosts::learn_known_hosts_path(
        "127.0.0.1",
        server.addr.port(),
        other.public_key(),
        env.dir.path().join("known_hosts"),
    )
    .unwrap();
    let mgr = env.manager().await;
    let id = mgr.add(req(server.url(1000, "k.bin"))).await.unwrap().id;
    let done = finished(&mgr, id).await;
    assert_eq!(done.status, DownloadStatus::Failed);
    assert!(
        done.error.unwrap().contains("host key"),
        "message explains the refusal"
    );
    assert_eq!(
        server.state.opens.load(Ordering::SeqCst),
        0,
        "nothing read from an untrusted host"
    );
}
