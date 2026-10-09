//! Standalone test server for manual testing:
//! `cargo run -p velox-test-server -- --port 8787`
//! (add `--static DIR` to serve DIR at `/static/...`, `--ftp-port P` /
//! `--sftp-port P` for the FTP and SFTP test servers),
//! then add e.g. `http://127.0.0.1:8787/file/big.bin?size=500000000&rate=5000000`.

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let mut port = 8787u16;
    let mut static_dir = None;
    let mut ftp_port: Option<u16> = None;
    let mut sftp_port: Option<u16> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--port" {
            port = args
                .next()
                .and_then(|p| p.parse().ok())
                .expect("--port <number>");
        } else if a == "--static" {
            static_dir = Some(args.next().expect("--static <dir>"));
        } else if a == "--ftp-port" {
            ftp_port = args.next().and_then(|p| p.parse().ok());
        } else if a == "--sftp-port" {
            sftp_port = args.next().and_then(|p| p.parse().ok());
        }
    }
    let server =
        velox_test_server::TestServer::start_on(SocketAddr::from(([127, 0, 0, 1], port))).await;
    if let Some(d) = static_dir {
        server.serve_dir(d);
    }
    let local = |p: u16| SocketAddr::from(([127, 0, 0, 1], p));
    let _ftp = match ftp_port {
        Some(p) => {
            let s = velox_test_server::ftp::FtpServer::start_on(local(p), Default::default()).await;
            println!("ftp: {}", s.url(1_048_576, "example.bin"));
            Some(s)
        }
        None => None,
    };
    let _sftp = match sftp_port {
        Some(p) => {
            let s =
                velox_test_server::sftp::SftpServer::start_on(local(p), "tester", "pw", None).await;
            println!("sftp: {}", s.url(1_048_576, "example.bin"));
            Some(s)
        }
        None => None,
    };
    println!("velox test server listening on http://{}", server.addr);
    println!(
        "example: {}",
        server.url("/file/sample.bin?size=104857600&rate=4000000")
    );
    tokio::signal::ctrl_c().await.ok();
}
