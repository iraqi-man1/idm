use futures::StreamExt;
use velox_http::{
    build_client, open_range, probe, ClientOptions, HttpError, RangeSupport, RequestContext,
    Validators,
};
use velox_test_server::{expected_content, TestServer};
use velox_types::{ProxyMode, ProxySettings};

fn opts() -> ClientOptions {
    ClientOptions {
        proxy: ProxySettings {
            mode: ProxyMode::None,
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn collect(mut s: velox_http::ByteStream) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    while let Some(c) = s.next().await {
        out.extend_from_slice(&c?);
    }
    Ok(out)
}

#[tokio::test]
async fn probe_reports_size_ranges_and_validators() {
    let srv = TestServer::start().await;
    let client = build_client(&opts()).unwrap();
    let ctx = RequestContext::new(srv.url("/file/a.bin?size=100000&cd=r%C3%A9sum%C3%A9.bin"));
    let p = probe(&client, &ctx, 0, &Validators::default())
        .await
        .unwrap();
    assert_eq!(p.status, 206);
    assert_eq!(p.ranges, RangeSupport::Yes);
    assert_eq!(p.total_size, Some(100_000));
    assert!(p.validators.etag.is_some());
    assert_eq!(p.disposition_name.as_deref(), Some("résumé.bin"));
    let body = collect(p.body.unwrap()).await.unwrap();
    assert_eq!(body, expected_content(srv.seed_for("a.bin"), 100_000));
}

#[tokio::test]
async fn probe_without_range_support() {
    let srv = TestServer::start().await;
    let client = build_client(&opts()).unwrap();
    let ctx = RequestContext::new(srv.url("/file/b.bin?size=5000&norange=1"));
    let p = probe(&client, &ctx, 0, &Validators::default())
        .await
        .unwrap();
    assert_eq!(p.status, 200);
    assert_eq!(p.ranges, RangeSupport::No);
    assert_eq!(p.total_size, Some(5000));
}

#[tokio::test]
async fn resume_probe_detects_change_via_if_range() {
    let srv = TestServer::start().await;
    let client = build_client(&opts()).unwrap();
    let ctx = RequestContext::new(srv.url("/file/c.bin?size=50000"));
    let first = probe(&client, &ctx, 0, &Validators::default())
        .await
        .unwrap();
    let v = first.validators.clone();
    drop(first);

    let again = probe(&client, &ctx, 1000, &v).await.unwrap();
    assert_eq!(again.status, 206);
    assert!(again.validator_matched);
    assert_eq!(again.body_start, 1000);
    drop(again);

    srv.bump("c.bin");
    let changed = probe(&client, &ctx, 1000, &v).await.unwrap();
    assert_eq!(changed.status, 200);
    assert!(changed.entity_changed);
}

#[tokio::test]
async fn open_range_validates_responses() {
    let srv = TestServer::start().await;
    let client = build_client(&opts()).unwrap();
    let ctx = RequestContext::new(srv.url("/file/d.bin?size=100000"));
    let p = probe(&client, &ctx, 0, &Validators::default())
        .await
        .unwrap();
    let v = p.validators.clone();
    drop(p);
    let body = collect(
        open_range(&client, &ctx, 40_000, Some(50_000), Some(100_000), &v)
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        body,
        expected_content(srv.seed_for("d.bin"), 100_000)[40_000..50_000].to_vec()
    );

    // Size mismatch -> remote changed.
    let err = open_range(&client, &ctx, 0, Some(10), Some(99_999), &v)
        .await
        .err()
        .unwrap();
    assert_eq!(err, HttpError::RemoteChanged);

    // Server that ignores ranges.
    let ctx2 = RequestContext::new(srv.url("/file/e.bin?size=1000&norange=1"));
    let err = open_range(
        &client,
        &ctx2,
        10,
        Some(20),
        Some(1000),
        &Validators::default(),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(err, HttpError::RangeIgnored);

    // Malformed Content-Range.
    let ctx3 = RequestContext::new(srv.url("/file/f.bin?size=1000&malformed=1"));
    let err = open_range(
        &client,
        &ctx3,
        10,
        Some(20),
        Some(1000),
        &Validators::default(),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(err, HttpError::BadContentRange(_)));
}

#[tokio::test]
async fn redirects_auth_and_status_errors() {
    let srv = TestServer::start().await;
    let client = build_client(&opts()).unwrap();
    let ctx = RequestContext::new(srv.url("/redirect?n=3&to=/file/g.bin%3Fsize%3D10"));
    let p = probe(&client, &ctx, 0, &Validators::default())
        .await
        .unwrap();
    assert!(p.final_url.contains("/file/g.bin"));
    assert_eq!(p.total_size, Some(10));

    let ctx = RequestContext::new(srv.url("/file/h.bin?size=10&auth=1"));
    let err = probe(&client, &ctx, 0, &Validators::default())
        .await
        .err()
        .unwrap();
    assert_eq!(err.status(), Some(401));
    let mut ctx = ctx;
    ctx.credentials = Some(velox_types::Credentials {
        username: "user".into(),
        password: "pass".into(),
    });
    assert!(probe(&client, &ctx, 0, &Validators::default())
        .await
        .is_ok());

    let ctx = RequestContext::new(srv.url("/file/i.bin?size=10&status=503"));
    let err = probe(&client, &ctx, 0, &Validators::default())
        .await
        .err()
        .unwrap();
    assert!(err.is_transient());
    assert!(err.is_connection_limit());
}

#[tokio::test]
async fn connection_refused_is_transient_network_error() {
    let client = build_client(&opts()).unwrap();
    let ctx = RequestContext::new("http://127.0.0.1:9/file.bin");
    let err = probe(&client, &ctx, 0, &Validators::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(err, HttpError::Network(_)), "{err:?}");
    assert!(err.is_transient());
}
