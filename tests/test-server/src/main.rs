//! Standalone test server for manual testing:
//! `cargo run -p velox-test-server -- --port 8787`
//! (add `--static DIR` to serve DIR at `/static/...`),
//! then add e.g. `http://127.0.0.1:8787/file/big.bin?size=500000000&rate=5000000`.

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let mut port = 8787u16;
    let mut static_dir = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--port" {
            port = args
                .next()
                .and_then(|p| p.parse().ok())
                .expect("--port <number>");
        } else if a == "--static" {
            static_dir = Some(args.next().expect("--static <dir>"));
        }
    }
    let server =
        velox_test_server::TestServer::start_on(SocketAddr::from(([127, 0, 0, 1], port))).await;
    if let Some(d) = static_dir {
        server.serve_dir(d);
    }
    println!("velox test server listening on http://{}", server.addr);
    println!(
        "example: {}",
        server.url("/file/sample.bin?size=104857600&rate=4000000")
    );
    tokio::signal::ctrl_c().await.ok();
}
