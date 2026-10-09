//! FTP and SFTP sources against the in-process test servers.

use futures::StreamExt;
use velox_ftp::{FtpError, Opened, Remote, RemoteConfig};
use velox_test_server::expected_content;
use velox_test_server::ftp::{FtpOptions, FtpServer};
use velox_test_server::sftp::SftpServer;

async fn read_all(o: Opened) -> Result<Vec<u8>, FtpError> {
    let mut out = Vec::new();
    let mut body = o.body;
    while let Some(chunk) = body.next().await {
        out.extend_from_slice(&chunk?);
    }
    Ok(out)
}

fn cfg(dir: &tempfile::TempDir) -> RemoteConfig {
    RemoteConfig {
        known_hosts: Some(dir.path().join("known_hosts")),
        ..Default::default()
    }
}

#[tokio::test]
async fn ftp_full_read_offsets_and_ranges() {
    let server = FtpServer::start(FtpOptions::default()).await;
    let size = 300_000;
    let expected = expected_content(server.seed_for("a.bin"), size);
    let r = Remote::new(&server.url(size, "a.bin"), None, RemoteConfig::default()).unwrap();
    let info = r.stat().await.unwrap();
    assert_eq!(info.size, Some(size));
    assert!(info.resumable);
    assert!(info.modified.is_some());

    let o = r.open(0, None).await.unwrap();
    assert!(o.at_offset);
    assert_eq!(read_all(o).await.unwrap(), expected);

    let o = r.open(100_000, Some(50_000)).await.unwrap();
    assert!(o.at_offset);
    assert_eq!(read_all(o).await.unwrap(), &expected[100_000..150_000]);
    let offsets = server
        .state
        .rest_offsets
        .lock()
        .get("a.bin")
        .cloned()
        .unwrap();
    assert_eq!(offsets.last(), Some(&100_000));
}

#[tokio::test]
async fn ftp_without_rest() {
    let server = FtpServer::start(FtpOptions {
        no_rest: true,
        ..Default::default()
    })
    .await;
    let r = Remote::new(&server.url(10_000, "n.bin"), None, RemoteConfig::default()).unwrap();
    assert!(!r.stat().await.unwrap().resumable);
    // A probe from an offset gets the whole file and says so ...
    let o = r.open(5_000, None).await.unwrap();
    assert!(!o.at_offset);
    assert_eq!(read_all(o).await.unwrap().len(), 10_000);
    // ... but a range cannot be served.
    assert!(matches!(
        r.open(5_000, Some(100)).await.err(),
        Some(FtpError::NoResume(_))
    ));
}

#[tokio::test]
async fn ftp_errors_are_classified() {
    let server = FtpServer::start(FtpOptions {
        login: Some(("alice".into(), "secret".into())),
        max_connections: Some(1),
        fail_after: Some(20_000),
        ..Default::default()
    })
    .await;
    let good = server.url(100_000, "x.bin");
    let wrong = good.replace("secret", "nope");
    assert!(matches!(
        Remote::new(&wrong, None, RemoteConfig::default())
            .unwrap()
            .stat()
            .await,
        Err(FtpError::Auth(_))
    ));

    let missing = format!("ftp://alice:secret@{}/elsewhere/x.bin", server.addr);
    assert!(matches!(
        Remote::new(&missing, None, RemoteConfig::default())
            .unwrap()
            .stat()
            .await,
        Err(FtpError::NotFound(_))
    ));

    // A transfer that breaks after 20 kB is a (retryable) network error.
    let r = Remote::new(&good, None, RemoteConfig::default()).unwrap();
    let o = r.open(0, None).await.unwrap();
    let err = read_all(o).await;
    // Unknown-length reads end at EOF; the size check happens in the engine.
    // With a requested length the short transfer is detected here:
    assert!(err.is_ok() || matches!(err, Err(FtpError::Network(_))));
    let o = r.open(0, Some(100_000)).await.unwrap();
    let e = read_all(o).await.unwrap_err();
    assert!(e.is_transient(), "{e:?}");

    // One connection at a time: a second one is refused (connection limit).
    let held = r.open(0, Some(100_000)).await.unwrap();
    let e = r.stat().await.unwrap_err();
    assert!(e.is_connection_limit(), "{e:?}");
    drop(held);
}

#[tokio::test]
async fn sftp_reads_and_learns_the_host_key() {
    let dir = tempfile::tempdir().unwrap();
    let server = SftpServer::start("bob", "pw", None).await;
    let size = 250_000;
    let expected = expected_content(server.seed_for("s.bin"), size);
    let r = Remote::new(&server.url(size, "s.bin"), None, cfg(&dir)).unwrap();
    let info = r.stat().await.unwrap();
    assert_eq!(info.size, Some(size));
    assert!(info.resumable);
    let known = std::fs::read_to_string(dir.path().join("known_hosts")).unwrap();
    assert!(
        known.contains(&format!("[127.0.0.1]:{}", server.addr.port())),
        "{known}"
    );

    let o = r.open(0, None).await.unwrap();
    assert_eq!(read_all(o).await.unwrap(), expected);
    let o = r.open(200_000, Some(30_000)).await.unwrap();
    assert_eq!(read_all(o).await.unwrap(), &expected[200_000..230_000]);
}

#[tokio::test]
async fn sftp_refuses_a_changed_host_key_and_bad_logins() {
    let dir = tempfile::tempdir().unwrap();
    let server = SftpServer::start("bob", "pw", None).await;
    // A different key is already recorded for this host and port.
    let other =
        russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519).unwrap();
    russh::keys::known_hosts::learn_known_hosts_path(
        "127.0.0.1",
        server.addr.port(),
        other.public_key(),
        dir.path().join("known_hosts"),
    )
    .unwrap();
    let r = Remote::new(&server.url(1000, "k.bin"), None, cfg(&dir)).unwrap();
    match r.stat().await {
        Err(FtpError::HostKey(m)) => assert!(m.contains("changed"), "{m}"),
        other => panic!("expected a host key error, got {other:?}"),
    }

    let dir = tempfile::tempdir().unwrap();
    let bad = server.url(1000, "k.bin").replace(":pw@", ":wrong@");
    assert!(matches!(
        Remote::new(&bad, None, cfg(&dir)).unwrap().stat().await,
        Err(FtpError::Auth(_))
    ));
    let missing = format!("sftp://bob:pw@{}/nope/k.bin", server.addr);
    assert!(matches!(
        Remote::new(&missing, None, cfg(&dir)).unwrap().stat().await,
        Err(FtpError::NotFound(_))
    ));
}
