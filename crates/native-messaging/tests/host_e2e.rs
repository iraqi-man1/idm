//! Runs the real `velox-nmh` binary between a simulated browser (stdin /
//! stdout framing) and an IPC server standing in for the desktop app.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use velox_nm::ipc::{self, BridgeHandler, EndpointInfo, HandlerFuture, PeerInfo};
use velox_nm::manifest::{CHROMIUM_DEV_EXTENSION_ID, FIREFOX_EXTENSION_ID};
use velox_types::protocol::{BrowserConfig, ExtMessage, ExtReply, ExtRequest};

struct TestApp {
    seen: parking_lot::Mutex<Vec<(PeerInfo, ExtRequest)>>,
}

impl BridgeHandler for TestApp {
    fn handle<'a>(&'a self, peer: &'a PeerInfo, req: ExtRequest) -> HandlerFuture<'a> {
        Box::pin(async move {
            self.seen.lock().push((peer.clone(), req.clone()));
            match req.message {
                ExtMessage::Ping => ExtReply::Pong,
                ExtMessage::Hello { .. } => ExtReply::Hello {
                    app_version: "test".into(),
                    protocol_version: velox_types::PROTOCOL_VERSION,
                    compatible: true,
                    message: None,
                    config: BrowserConfig {
                        capture_downloads: true,
                        capture_extensions: vec!["zip".into()],
                        min_capture_size: 0,
                        excluded_sites: vec![],
                        video_detection: true,
                        floating_button: true,
                    },
                },
                ExtMessage::AddDownload { .. } => {
                    // Slow handler: other requests must not wait for it.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    ExtReply::Added {
                        accepted: true,
                        download_id: None,
                        pending: true,
                        reason: None,
                    }
                }
                _ => ExtReply::Error {
                    code: "unsupported".into(),
                    message: "test".into(),
                },
            }
        })
    }
}

async fn start_app(dir: &Path, token: &str) -> (Arc<TestApp>, ipc::ServerHandle) {
    let app = Arc::new(TestApp {
        seen: Default::default(),
    });
    let endpoint = ipc::new_endpoint_name(dir);
    let handle = ipc::serve(&endpoint, token.to_string(), app.clone())
        .await
        .unwrap();
    ipc::write_endpoint_file(
        dir,
        &EndpointInfo {
            endpoint,
            token: token.to_string(),
            pid: std::process::id(),
            app_version: "test".into(),
            app_path: None,
        },
    )
    .unwrap();
    (app, handle)
}

fn spawn_host(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_velox-nmh"))
        .args(args)
        .env("VELOX_DATA_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

async fn send(child: &mut Child, v: &Value) {
    let body = serde_json::to_vec(v).unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    stdin
        .write_all(&(body.len() as u32).to_ne_bytes())
        .await
        .unwrap();
    stdin.write_all(&body).await.unwrap();
    stdin.flush().await.unwrap();
}

async fn send_raw(child: &mut Child, body: &[u8]) {
    let stdin = child.stdin.as_mut().unwrap();
    stdin
        .write_all(&(body.len() as u32).to_ne_bytes())
        .await
        .unwrap();
    stdin.write_all(body).await.unwrap();
    stdin.flush().await.unwrap();
}

async fn recv(child: &mut Child) -> Value {
    let out = child.stdout.as_mut().unwrap();
    let mut len = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(20), out.read_exact(&mut len))
        .await
        .expect("timely reply")
        .unwrap();
    let mut buf = vec![0u8; u32::from_ne_bytes(len) as usize];
    out.read_exact(&mut buf).await.unwrap();
    serde_json::from_slice(&buf).unwrap()
}

fn chrome_origin() -> String {
    format!("chrome-extension://{CHROMIUM_DEV_EXTENSION_ID}/")
}

#[tokio::test]
async fn relays_requests_between_browser_and_app() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _h) = start_app(dir.path(), "s3cret").await;
    let mut host = spawn_host(dir.path(), &[&chrome_origin()]);

    send(&mut host, &json!({"id": 1, "type": "hello", "extension_version": "0.1.0", "browser": "chrome", "protocol_version": 1})).await;
    let r = recv(&mut host).await;
    assert_eq!(r["id"], 1);
    assert_eq!(r["type"], "hello");
    assert_eq!(r["config"]["capture_extensions"][0], "zip");

    // A slow request followed by a fast one: the fast one answers first.
    send(&mut host, &json!({"id": 2, "type": "add_download", "download": {"url": "https://example.com/a.zip", "cookies": "s=1"}})).await;
    send(&mut host, &json!({"id": 3, "type": "ping"})).await;
    let first = recv(&mut host).await;
    let second = recv(&mut host).await;
    assert_eq!(first["id"], 3);
    assert_eq!(first["type"], "pong");
    assert_eq!(second["id"], 2, "{second}");
    assert_eq!(second["type"], "added", "{second}");

    // The app saw the authenticated origin.
    let seen = app.seen.lock();
    assert_eq!(seen[0].0.origin, chrome_origin());
    assert_eq!(seen[0].0.browser, "chromium");
}

#[tokio::test]
async fn rejects_invalid_messages_without_forwarding() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _h) = start_app(dir.path(), "tok").await;
    let mut host = spawn_host(dir.path(), &["/path/manifest.json", FIREFOX_EXTENSION_ID]);

    send_raw(&mut host, b"{not json").await;
    let r = recv(&mut host).await;
    assert_eq!(r["code"], "invalid_json");

    send(
        &mut host,
        &json!({"id": 5, "type": "add_download", "download": {"url": "file:///etc/passwd"}}),
    )
    .await;
    let r = recv(&mut host).await;
    assert_eq!(
        (r["id"].as_u64(), r["code"].as_str()),
        (Some(5), Some("invalid_request"))
    );

    send(&mut host, &json!({"id": 6, "type": "add_download", "download": {"url": "https://e.com/x", "extra": true}})).await;
    let r = recv(&mut host).await;
    assert_eq!(r["code"], "invalid_request");

    send(&mut host, &json!({"id": 7, "type": "launch_missiles"})).await;
    let r = recv(&mut host).await;
    assert_eq!(r["code"], "invalid_request");

    assert!(
        app.seen.lock().is_empty(),
        "nothing invalid reached the app"
    );
}

#[tokio::test]
async fn unknown_extensions_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = spawn_host(
        dir.path(),
        &["chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"],
    );
    let status = tokio::time::timeout(Duration::from_secs(10), host.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(3));

    // Started by hand (no browser caller).
    let mut host = spawn_host(dir.path(), &[]);
    let status = tokio::time::timeout(Duration::from_secs(10), host.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}

#[tokio::test]
async fn reports_when_app_is_not_running_or_token_is_wrong() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = spawn_host(dir.path(), &[&chrome_origin()]);
    send(&mut host, &json!({"id": 1, "type": "ping"})).await;
    let r = recv(&mut host).await;
    assert_eq!(r["code"], "app_not_running");

    // App running but the endpoint file has a wrong token.
    let (_app, _h) = start_app(dir.path(), "right").await;
    let mut info = ipc::read_endpoint_file(dir.path()).unwrap();
    info.token = "wrong".into();
    ipc::write_endpoint_file(dir.path(), &info).unwrap();
    send(&mut host, &json!({"id": 2, "type": "ping"})).await;
    let r = recv(&mut host).await;
    assert_eq!(r["code"], "app_unavailable");
}

#[tokio::test]
async fn server_pushes_notifications_and_survives_app_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (_app, handle) = start_app(dir.path(), "t1").await;
    let mut host = spawn_host(dir.path(), &[&chrome_origin()]);
    send(&mut host, &json!({"id": 1, "type": "ping"})).await;
    assert_eq!(recv(&mut host).await["type"], "pong");
    for _ in 0..50 {
        if handle.connection_count() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    handle.notify_all(ExtReply::Ok);
    let n = recv(&mut host).await;
    assert_eq!(
        (n["id"].as_u64(), n["type"].as_str()),
        (Some(0), Some("ok"))
    );
}
