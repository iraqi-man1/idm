//! End-to-end media downloads against the local test server.
//!
//! The fixtures are real media produced by FFmpeg at test time (HLS with
//! TS and fMP4 segments, AES-128 encrypted HLS, DASH with templates and with
//! byte ranges), served over HTTP and downloaded through the
//! `DownloadManager` with the `MediaEngine` runner. Results are verified
//! with ffprobe and a full decode.
//!
//! The tests need `ffmpeg` and `ffprobe` (any build with an H.264 encoder:
//! libx264 or OpenH264, e.g. the bundled LGPL build) on `PATH` or in
//! `VELOX_FFMPEG` / `VELOX_FFPROBE`. When they are missing the tests are
//! skipped, unless `VELOX_REQUIRE_MEDIA_TESTS=1` (set in CI) makes that an
//! error.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use velox_core::{DownloadManager, ManagerConfig};
use velox_media::{MediaEngine, RequestInfo, Tools};
use velox_persistence::{Database, SecretBox};
use velox_test_server::TestServer;
use velox_types::{
    AddDownloadRequest, DownloadId, DownloadInfo, DownloadStatus, ErrorKind, MediaRequest,
    MediaSourceKind, OutputContainer, ProxyMode,
};

const SEGMENTS: usize = 6;

fn tools() -> Option<Tools> {
    let t = Tools::discover(&[]);
    if t.ffmpeg.is_some() && t.ffprobe.is_some() {
        return Some(t);
    }
    if std::env::var("VELOX_REQUIRE_MEDIA_TESTS").as_deref() == Ok("1") {
        panic!("ffmpeg/ffprobe not found but VELOX_REQUIRE_MEDIA_TESTS=1");
    }
    eprintln!("SKIPPED: ffmpeg/ffprobe not found");
    None
}

fn run(cmd: &mut Command) {
    let out = cmd.output().expect("run tool");
    assert!(
        out.status.success(),
        "{:?} failed: {}",
        cmd,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn ffmpeg(t: &Tools, args: &[&str]) {
    let mut c = Command::new(t.ffmpeg.as_ref().unwrap());
    c.args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args);
    run(&mut c);
}

/// H.264 encoder arguments: libx264 when available (GPL builds), else
/// OpenH264 (the bundled LGPL builds), else MPEG-4 part 2.
fn h264(t: &Tools) -> Vec<&'static str> {
    let out = Command::new(t.ffmpeg.as_ref().unwrap())
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    if out.contains(" libx264 ") {
        vec!["-c:v", "libx264", "-preset", "ultrafast"]
    } else if out.contains(" libopenh264 ") {
        vec!["-c:v", "libopenh264"]
    } else {
        vec!["-c:v", "mpeg4"]
    }
}

const SUB_VTT: &str = "WEBVTT\n\n00:00:00.500 --> 00:00:02.000\nHello Velox\n\n00:00:02.500 --> 00:00:04.000\nمرحبا\n";

/// Generate all fixtures once per test binary.
fn fixtures(t: &Tools) -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("media-fixtures");
        let _ = std::fs::remove_dir_all(&dir);
        let p = |s: &str| dir.join(s).to_string_lossy().to_string();
        for d in ["hls/v0", "hls/v1", "hls/a", "hls/a2", "hls/sub", "enc", "fmp4", "dash", "dash1", "drm"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        // 6 s of video (1 s GOP) and a sine tone.
        let enc = h264(t);
        let src = p("src.mp4");
        let mut a = vec![
            "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25",
            "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "6",
        ];
        a.extend(&enc);
        a.extend([
            "-pix_fmt", "yuv420p", "-g", "25", "-keyint_min", "25", "-sc_threshold", "0",
            "-c:a", "aac", "-b:a", "96k", &src,
        ]);
        ffmpeg(t, &a);
        let hls = ["-f", "hls", "-hls_time", "1", "-hls_playlist_type", "vod"];
        for (name, size) in [("v0", "320x180"), ("v1", "640x360")] {
            let mut a = vec!["-i", &*Box::leak(p("src.mp4").into_boxed_str()), "-map", "0:v"];
            a.extend(&enc);
            a.extend(["-s", size, "-g", "25", "-keyint_min", "25", "-sc_threshold", "0", "-an"]);
            a.extend(hls);
            let seg = Box::leak(p(&format!("hls/{name}/seg%03d.ts")).into_boxed_str());
            let idx = Box::leak(p(&format!("hls/{name}/index.m3u8")).into_boxed_str());
            a.extend(["-hls_segment_filename", seg, idx]);
            ffmpeg(t, &a);
        }
        let mut a = vec!["-i", &*Box::leak(p("src.mp4").into_boxed_str()), "-map", "0:a", "-c:a", "copy"];
        a.extend(hls);
        let seg = Box::leak(p("hls/a/seg%03d.ts").into_boxed_str());
        let idx = Box::leak(p("hls/a/index.m3u8").into_boxed_str());
        a.extend(["-hls_segment_filename", seg, idx]);
        ffmpeg(t, &a);
        // A second (Arabic) audio rendition with a different tone.
        ffmpeg(
            t,
            &[
                "-f", "lavfi", "-i", "sine=frequency=880:sample_rate=48000", "-t", "6", "-c:a", "aac", "-b:a", "64k",
                "-f", "hls", "-hls_time", "1", "-hls_playlist_type", "vod",
                "-hls_segment_filename", &p("hls/a2/seg%03d.ts"), &p("hls/a2/index.m3u8"),
            ],
        );
        std::fs::write(dir.join("hls/sub/en.vtt"), SUB_VTT).unwrap();
        std::fs::write(
            dir.join("hls/sub/en.m3u8"),
            "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:6.0,\nen.vtt\n#EXT-X-ENDLIST\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("hls/master.m3u8"),
            concat!(
                "#EXTM3U\n#EXT-X-VERSION:4\n",
                "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"a/index.m3u8\"\n",
                "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Arabic\",LANGUAGE=\"ar\",DEFAULT=NO,AUTOSELECT=YES,URI=\"a2/index.m3u8\"\n",
                "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=NO,AUTOSELECT=YES,URI=\"sub/en.m3u8\"\n",
                "#EXT-X-STREAM-INF:BANDWIDTH=400000,RESOLUTION=320x180,CODECS=\"avc1.42c01e,mp4a.40.2\",AUDIO=\"aud\",SUBTITLES=\"subs\"\n",
                "v0/index.m3u8\n",
                "#EXT-X-STREAM-INF:BANDWIDTH=1200000,RESOLUTION=640x360,CODECS=\"avc1.42c01f,mp4a.40.2\",AUDIO=\"aud\",SUBTITLES=\"subs\"\n",
                "v1/index.m3u8\n",
            ),
        )
        .unwrap();

        // AES-128 (IV derived from the media sequence number).
        std::fs::write(dir.join("enc/key.bin"), [0x5a_u8; 16]).unwrap();
        std::fs::write(dir.join("keyinfo"), format!("key.bin\n{}\n", p("enc/key.bin"))).unwrap();
        let mut a = vec!["-i", &*Box::leak(p("src.mp4").into_boxed_str()), "-c", "copy"];
        a.extend(hls);
        let ki = Box::leak(p("keyinfo").into_boxed_str());
        let seg = Box::leak(p("enc/seg%03d.ts").into_boxed_str());
        let idx = Box::leak(p("enc/index.m3u8").into_boxed_str());
        a.extend(["-hls_key_info_file", ki, "-hls_segment_filename", seg, idx]);
        ffmpeg(t, &a);

        // HLS with fragmented MP4 segments.
        let mut a = vec!["-i", &*Box::leak(p("src.mp4").into_boxed_str()), "-c", "copy"];
        a.extend(hls);
        let seg = Box::leak(p("fmp4/seg%03d.m4s").into_boxed_str());
        let idx = Box::leak(p("fmp4/index.m3u8").into_boxed_str());
        a.extend(["-hls_segment_type", "fmp4", "-hls_fmp4_init_filename", "init.mp4", "-hls_segment_filename", seg, idx]);
        ffmpeg(t, &a);

        // DASH: SegmentTemplate + SegmentTimeline, separate video and audio.
        ffmpeg(
            t,
            &[
                "-i", &p("src.mp4"), "-map", "0:v", "-map", "0:a", "-c", "copy", "-f", "dash", "-seg_duration", "1",
                "-use_template", "1", "-use_timeline", "1", "-adaptation_sets", "id=0,streams=v id=1,streams=a",
                &p("dash/manifest.mpd"),
            ],
        );
        // DASH: one file per representation, SegmentList with byte ranges.
        ffmpeg(
            t,
            &[
                "-i", &p("src.mp4"), "-map", "0:v", "-map", "0:a", "-c", "copy", "-f", "dash", "-seg_duration", "1",
                "-single_file", "1", "-use_template", "0", "-use_timeline", "0",
                "-adaptation_sets", "id=0,streams=v id=1,streams=a", &p("dash1/manifest.mpd"),
            ],
        );

        // DRM-protected playlist (must never be downloaded).
        std::fs::write(
            dir.join("drm/index.m3u8"),
            concat!(
                "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-PLAYLIST-TYPE:VOD\n",
                "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key-id\",KEYFORMAT=\"com.apple.streamingkeydelivery\",KEYFORMATVERSIONS=\"1\"\n",
                "#EXTINF:1.0,\n../hls/v0/seg000.ts\n#EXT-X-ENDLIST\n",
            ),
        )
        .unwrap();
        dir
    })
}

struct Env {
    _dir: tempfile::TempDir,
    db_path: PathBuf,
    downloads: PathBuf,
    key: [u8; 32],
    tools: Tools,
    server: TestServer,
}

impl Env {
    async fn new() -> Option<Self> {
        let tools = tools()?;
        let fx = fixtures(&tools);
        let server = TestServer::start().await;
        server.serve_dir(fx);
        let dir = tempfile::tempdir().unwrap();
        Some(Self {
            db_path: dir.path().join("velox.sqlite"),
            downloads: dir.path().join("downloads"),
            _dir: dir,
            key: SecretBox::generate_key(),
            tools,
            server,
        })
    }

    async fn manager(&self) -> DownloadManager {
        let db = Database::open(&self.db_path).unwrap();
        let mut s = db.load_settings().unwrap();
        s.general.download_dir = self.downloads.to_string_lossy().to_string();
        s.network.proxy.mode = ProxyMode::None;
        s.downloads.retry_delay_secs = 1;
        s.downloads.max_retries = 2;
        s.media.segment_concurrency = 3;
        db.save_settings(&s.normalized()).unwrap();
        let (mgr, _) = DownloadManager::open(ManagerConfig {
            db,
            secret_box: Some(SecretBox::new(&self.key)),
            proxy_password: None,
            known_hosts: None,
        })
        .await
        .unwrap();
        mgr.set_media_runner(Arc::new(MediaEngine::new(self.tools.clone())));
        mgr
    }

    fn url(&self, path: &str) -> String {
        self.server.url(&format!("/static/{path}"))
    }

    fn media(&self, kind: MediaSourceKind, path: &str, container: OutputContainer) -> MediaRequest {
        MediaRequest {
            kind,
            url: self.url(path),
            container,
            title: Some("Test clip".into()),
            ..Default::default()
        }
    }

    async fn add(
        &self,
        mgr: &DownloadManager,
        media: MediaRequest,
        speed_limit: Option<u64>,
    ) -> DownloadId {
        mgr.add(AddDownloadRequest {
            url: media.url.clone(),
            media: Some(media),
            speed_limit,
            ..Default::default()
        })
        .await
        .unwrap()
        .id
    }

    /// Sum of requests made for the static files under `prefix`.
    fn requests(&self, files: &[String]) -> Vec<u32> {
        files
            .iter()
            .map(|f| {
                self.server
                    .static_stats(f)
                    .requests
                    .load(std::sync::atomic::Ordering::SeqCst)
            })
            .collect()
    }
}

async fn wait_until(
    mgr: &DownloadManager,
    id: DownloadId,
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
                "timeout; last state: {:?} error={:?} downloaded={}",
                info.status, info.error, info.downloaded
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn finished(mgr: &DownloadManager, id: DownloadId) -> DownloadInfo {
    wait_until(mgr, id, Duration::from_secs(90), |i| {
        i.status.is_terminal() && !mgr.is_running(id)
    })
    .await
}

async fn completed(mgr: &DownloadManager, id: DownloadId) -> PathBuf {
    let info = finished(mgr, id).await;
    assert_eq!(
        info.status,
        DownloadStatus::Completed,
        "error: {:?}",
        info.error
    );
    let path = Path::new(&info.save_dir).join(&info.file_name);
    assert!(path.is_file(), "{} missing", path.display());
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        info.total_size.unwrap()
    );
    // The work directory is removed.
    let leftovers: Vec<_> = std::fs::read_dir(&info.save_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".velox-"))
        .collect();
    assert!(leftovers.is_empty(), "work dir left behind");
    path
}

#[derive(Debug)]
struct Probed {
    format: String,
    duration: f64,
    streams: Vec<(String, String, Option<u64>)>,
}

impl Probed {
    fn kinds(&self) -> Vec<&str> {
        self.streams.iter().map(|s| s.0.as_str()).collect()
    }
    fn video_height(&self) -> Option<u64> {
        self.streams
            .iter()
            .find(|s| s.0 == "video")
            .and_then(|s| s.2)
    }
}

/// ffprobe the file and decode it completely; any decode error fails.
fn verify(t: &Tools, path: &Path) -> Probed {
    let out = Command::new(t.ffprobe.as_ref().unwrap())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=format_name,duration:stream=codec_type,codec_name,height",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "ffprobe: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let streams = v["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["codec_type"].as_str().unwrap_or("").to_string(),
                s["codec_name"].as_str().unwrap_or("").to_string(),
                s["height"].as_u64(),
            )
        })
        .collect();
    let decode = Command::new(t.ffmpeg.as_ref().unwrap())
        .args(["-v", "error", "-xerror", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(
        decode.status.success() && decode.stderr.is_empty(),
        "decode errors in {}: {}",
        path.display(),
        String::from_utf8_lossy(&decode.stderr)
    );
    Probed {
        format: v["format"]["format_name"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        duration: v["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse().ok())
            .unwrap_or(0.0),
        streams,
    }
}

fn assert_duration(p: &Probed) {
    assert!(
        (p.duration - 6.0).abs() < 0.6,
        "duration {} in {p:?}",
        p.duration
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn probe_hls_master_lists_variants_audio_and_subtitles() {
    let Some(env) = Env::new().await else { return };
    let engine = MediaEngine::new(env.tools.clone());
    let info = RequestInfo {
        url: env.url("hls/master.m3u8"),
        ..Default::default()
    };
    let r = engine
        .probe(
            MediaSourceKind::Hls,
            &info,
            &Default::default(),
            &env.downloads,
        )
        .await
        .unwrap();
    let ids: Vec<&str> = r.formats.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, ["v0", "v1", "a0", "a1"]);
    assert_eq!(r.formats[3].language.as_deref(), Some("ar"));
    assert_eq!(r.formats[1].height, Some(360));
    assert!(
        r.formats[1].has_video && !r.formats[1].has_audio,
        "audio comes from the rendition"
    );
    assert_eq!(r.subtitles.len(), 1);
    assert_eq!(r.subtitles[0].language, "en");
    assert!(!r.is_live && !r.drm_protected);
    assert!((r.duration_secs.unwrap() - 6.0).abs() < 0.5);

    let info = RequestInfo {
        url: env.url("dash/manifest.mpd"),
        ..Default::default()
    };
    let r = engine
        .probe(
            MediaSourceKind::Dash,
            &info,
            &Default::default(),
            &env.downloads,
        )
        .await
        .unwrap();
    assert_eq!(r.formats.iter().filter(|f| f.has_video).count(), 1);
    assert_eq!(
        r.formats
            .iter()
            .filter(|f| f.has_audio && !f.has_video)
            .count(),
        1
    );

    let info = RequestInfo {
        url: env.url("drm/index.m3u8"),
        ..Default::default()
    };
    let r = engine
        .probe(
            MediaSourceKind::Hls,
            &info,
            &Default::default(),
            &env.downloads,
        )
        .await
        .unwrap();
    assert!(r.drm_protected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_best_variant_with_separate_audio_and_embedded_subtitles() {
    let Some(env) = Env::new().await else { return };
    let mgr = env.manager().await;
    let mut m = env.media(
        MediaSourceKind::Hls,
        "hls/master.m3u8",
        OutputContainer::Mp4,
    );
    m.subtitle_languages = vec!["en".into()];
    m.embed_subtitles = true;
    let id = env.add(&mgr, m, None).await;
    let path = completed(&mgr, id).await;
    assert_eq!(path.file_name().unwrap(), "Test clip.mp4");
    let p = verify(&env.tools, &path);
    assert!(p.format.contains("mp4"), "{p:?}");
    assert_eq!(p.kinds(), ["video", "audio", "subtitle"]);
    assert_eq!(p.video_height(), Some(360), "best variant");
    assert_duration(&p);
    // Only the chosen variant was fetched.
    assert_eq!(
        env.server
            .static_stats("hls/v0/seg000.ts")
            .requests
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_selected_variant_mkv_with_subtitle_file() {
    let Some(env) = Env::new().await else { return };
    let mgr = env.manager().await;
    let mut m = env.media(
        MediaSourceKind::Hls,
        "hls/master.m3u8",
        OutputContainer::Mkv,
    );
    m.format_id = Some("v0".into());
    m.subtitle_languages = vec!["en".into()];
    m.embed_subtitles = false;
    let id = env.add(&mgr, m, None).await;
    let path = completed(&mgr, id).await;
    assert_eq!(path.extension().unwrap(), "mkv");
    let p = verify(&env.tools, &path);
    assert!(p.format.contains("matroska"), "{p:?}");
    assert_eq!(p.kinds(), ["video", "audio"]);
    assert_eq!(p.video_height(), Some(180));
    let sub = path.with_file_name("Test clip.en.vtt");
    let text = std::fs::read_to_string(&sub).unwrap();
    assert!(
        text.starts_with("WEBVTT") && text.contains("مرحبا"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_aes128_segments_are_decrypted() {
    let Some(env) = Env::new().await else { return };
    let playlist = std::fs::read_to_string(fixtures(&env.tools).join("enc/index.m3u8")).unwrap();
    assert!(
        playlist.contains("METHOD=AES-128"),
        "fixture must be encrypted"
    );
    // Encrypted segments are not playable on their own.
    let raw = fixtures(&env.tools).join("enc/seg000.ts");
    assert_ne!(
        std::fs::read(&raw).unwrap()[0],
        0x47,
        "segment must not be a plain TS"
    );

    let mgr = env.manager().await;
    let id = env
        .add(
            &mgr,
            env.media(MediaSourceKind::Hls, "enc/index.m3u8", OutputContainer::Mp4),
            None,
        )
        .await;
    let path = completed(&mgr, id).await;
    let p = verify(&env.tools, &path);
    assert_eq!(p.kinds(), ["video", "audio"]);
    assert_duration(&p);
    // The key is fetched once and cached.
    assert_eq!(
        env.server
            .static_stats("enc/key.bin")
            .requests
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_fmp4_original_container() {
    let Some(env) = Env::new().await else { return };
    let mgr = env.manager().await;
    let id = env
        .add(
            &mgr,
            env.media(
                MediaSourceKind::Hls,
                "fmp4/index.m3u8",
                OutputContainer::Original,
            ),
            None,
        )
        .await;
    let path = completed(&mgr, id).await;
    assert_eq!(path.extension().unwrap(), "mp4");
    let p = verify(&env.tools, &path);
    assert_eq!(p.kinds(), ["video", "audio"]);
    assert_duration(&p);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_audio_extraction_to_mp3() {
    let Some(env) = Env::new().await else { return };
    let mgr = env.manager().await;
    let id = env
        .add(
            &mgr,
            env.media(
                MediaSourceKind::Hls,
                "hls/master.m3u8",
                OutputContainer::Mp3,
            ),
            None,
        )
        .await;
    let path = completed(&mgr, id).await;
    assert_eq!(path.extension().unwrap(), "mp3");
    let p = verify(&env.tools, &path);
    assert_eq!(p.streams.len(), 1);
    assert_eq!(p.streams[0].1, "mp3");
    assert_eq!(mgr.get(id).unwrap().category, velox_types::Category::Music);
    // No video segment was downloaded.
    assert_eq!(
        env.server
            .static_stats("hls/v1/seg000.ts")
            .requests
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dash_template_timeline_video_and_audio() {
    let Some(env) = Env::new().await else { return };
    let mgr = env.manager().await;
    let id = env
        .add(
            &mgr,
            env.media(
                MediaSourceKind::Dash,
                "dash/manifest.mpd",
                OutputContainer::Mp4,
            ),
            None,
        )
        .await;
    let path = completed(&mgr, id).await;
    let p = verify(&env.tools, &path);
    assert_eq!(p.kinds(), ["video", "audio"]);
    assert_eq!(p.video_height(), Some(360));
    assert_duration(&p);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dash_segment_list_with_byte_ranges() {
    let Some(env) = Env::new().await else { return };
    let mpd = std::fs::read_to_string(fixtures(&env.tools).join("dash1/manifest.mpd")).unwrap();
    assert!(
        mpd.contains("mediaRange"),
        "fixture must use byte ranges:\n{mpd}"
    );
    let mgr = env.manager().await;
    let id = env
        .add(
            &mgr,
            env.media(
                MediaSourceKind::Dash,
                "dash1/manifest.mpd",
                OutputContainer::Mkv,
            ),
            None,
        )
        .await;
    let path = completed(&mgr, id).await;
    let p = verify(&env.tools, &path);
    assert_eq!(p.kinds(), ["video", "audio"]);
    assert_duration(&p);
    let files: Vec<String> = std::fs::read_dir(fixtures(&env.tools).join("dash1"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.ends_with(".mpd"))
        .collect();
    let ranged: u32 = files
        .iter()
        .map(|f| {
            env.server
                .static_stats(&format!("dash1/{f}"))
                .range_requests
                .load(std::sync::atomic::Ordering::SeqCst)
        })
        .sum();
    assert!(
        ranged as usize >= 2 * SEGMENTS,
        "segments must be fetched with Range requests ({ranged})"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drm_protected_media_is_refused() {
    let Some(env) = Env::new().await else { return };
    let mgr = env.manager().await;
    let id = env
        .add(
            &mgr,
            env.media(MediaSourceKind::Hls, "drm/index.m3u8", OutputContainer::Mp4),
            None,
        )
        .await;
    let info = finished(&mgr, id).await;
    assert_eq!(info.status, DownloadStatus::Failed);
    assert_eq!(info.error_kind, Some(ErrorKind::Unsupported));
    assert!(info.error.unwrap().contains("DRM"));
    assert_eq!(
        env.server
            .static_stats("hls/v0/seg000.ts")
            .requests
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_segment_url_is_reported() {
    let Some(env) = Env::new().await else { return };
    env.server.force_status("hls/v1/seg003.ts", Some(403));
    let mgr = env.manager().await;
    let mut m = env.media(
        MediaSourceKind::Hls,
        "hls/master.m3u8",
        OutputContainer::Mp4,
    );
    m.format_id = Some("v1".into());
    let id = env.add(&mgr, m, None).await;
    let info = finished(&mgr, id).await;
    assert_eq!(info.status, DownloadStatus::Failed);
    assert_eq!(
        info.error_kind,
        Some(ErrorKind::LinkExpired),
        "{:?}",
        info.error
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_restart_app_and_resume_keeps_finished_segments() {
    let Some(env) = Env::new().await else { return };
    let seg_files: Vec<String> = (0..SEGMENTS)
        .map(|i| format!("hls/v1/seg{i:03}.ts"))
        .collect();
    let total: u64 = seg_files
        .iter()
        .map(|f| {
            std::fs::metadata(fixtures(&env.tools).join(f))
                .unwrap()
                .len()
        })
        .sum();
    let mgr = env.manager().await;
    let mut m = env.media(
        MediaSourceKind::Hls,
        "hls/master.m3u8",
        OutputContainer::Mp4,
    );
    m.format_id = Some("v1".into());
    // Slow enough to pause in the middle.
    let id = env.add(&mgr, m, Some((total / 4).max(64 * 1024))).await;
    let work = wait_until(&mgr, id, Duration::from_secs(60), |i| {
        i.downloaded > total / 3
    })
    .await;
    mgr.pause(id).await.unwrap();
    let paused = mgr.get(id).unwrap();
    assert_eq!(paused.status, DownloadStatus::Paused);
    let work_dir = Path::new(&work.save_dir).join(".velox-".to_string() + &id.simple().to_string());
    let done_before: HashSet<usize> = std::fs::read_dir(work_dir.join("video"))
        .expect("work dir kept while paused")
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_suffix(".seg")
                .and_then(|n| n.parse().ok())
        })
        .collect();
    assert!(
        !done_before.is_empty() && done_before.len() < SEGMENTS,
        "{done_before:?}"
    );
    let before = env.requests(&seg_files);

    // Simulate an application restart.
    mgr.shutdown().await;
    drop(mgr);
    let mgr = env.manager().await;
    assert_eq!(mgr.get(id).unwrap().status, DownloadStatus::Paused);
    mgr.set_speed_limit(id, 0).unwrap();
    mgr.start(id).unwrap();
    let path = completed(&mgr, id).await;
    let p = verify(&env.tools, &path);
    assert_eq!(p.kinds(), ["video", "audio"]);
    assert_duration(&p);

    let after = env.requests(&seg_files);
    for i in &done_before {
        assert_eq!(after[*i], before[*i], "segment {i} was downloaded again");
    }
    assert!(after.iter().all(|&n| n >= 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_removes_the_work_directory() {
    let Some(env) = Env::new().await else { return };
    let mgr = env.manager().await;
    let mut m = env.media(
        MediaSourceKind::Hls,
        "hls/master.m3u8",
        OutputContainer::Mp4,
    );
    m.format_id = Some("v1".into());
    let id = env.add(&mgr, m, Some(64 * 1024)).await;
    let info = wait_until(&mgr, id, Duration::from_secs(60), |i| i.downloaded > 0).await;
    let work_dir = Path::new(&info.save_dir).join(format!(".velox-{}", id.simple()));
    assert!(work_dir.is_dir());
    mgr.cancel(id).await.unwrap();
    assert_eq!(mgr.get(id).unwrap().status, DownloadStatus::Cancelled);
    assert!(!work_dir.exists(), "work dir must be deleted on cancel");
}

/// yt-dlp tests also need yt-dlp (`VELOX_YTDLP` or `PATH`); required when
/// `VELOX_REQUIRE_YTDLP_TESTS=1`.
fn has_ytdlp(env: &Env) -> bool {
    if env.tools.ytdlp.is_some() {
        return true;
    }
    if std::env::var("VELOX_REQUIRE_YTDLP_TESTS").as_deref() == Ok("1") {
        panic!("yt-dlp not found but VELOX_REQUIRE_YTDLP_TESTS=1");
    }
    eprintln!("SKIPPED: yt-dlp not found");
    false
}

fn page_url(env: &Env) -> String {
    let video = env.url("src.mp4");
    env.server.url(&format!(
        "/page?title=Velox%20Clip&video={}",
        video.replace(':', "%3A").replace('/', "%2F")
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ytdlp_probe_of_a_page_with_a_video() {
    let Some(env) = Env::new().await else { return };
    if !has_ytdlp(&env) {
        return;
    }
    let engine = MediaEngine::new(env.tools.clone());
    let work = tempfile::tempdir().unwrap();
    let info = RequestInfo {
        url: page_url(&env),
        ..Default::default()
    };
    let r = engine
        .probe(
            MediaSourceKind::Page,
            &info,
            &Default::default(),
            work.path(),
        )
        .await
        .unwrap();
    assert_eq!(r.kind, MediaSourceKind::Page);
    assert!(!r.formats.is_empty(), "{r:?}");
    assert!(r.extractor.is_some(), "{r:?}");
    assert!(
        r.formats.iter().any(|f| f.has_video && f.has_audio),
        "{r:?}"
    );
    assert_eq!(
        r.title.as_deref().map(|t| t.starts_with("Velox Clip")),
        Some(true),
        "{r:?}"
    );
    assert!(!r.drm_protected && !r.is_live);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ytdlp_download_of_a_page_with_a_video() {
    let Some(env) = Env::new().await else { return };
    if !has_ytdlp(&env) {
        return;
    }
    let mgr = env.manager().await;
    let mut m = env.media(MediaSourceKind::Page, "", OutputContainer::Mkv);
    m.url = page_url(&env);
    m.title = Some("Page clip".into());
    let id = env.add(&mgr, m, None).await;
    let path = completed(&mgr, id).await;
    assert_eq!(path.extension().unwrap(), "mkv", "{}", path.display());
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("Page clip"));
    let p = verify(&env.tools, &path);
    assert_eq!(p.kinds(), ["video", "audio"]);
    assert_duration(&p);
    assert!(
        env.server
            .static_stats("src.mp4")
            .requests
            .load(std::sync::atomic::Ordering::SeqCst)
            >= 1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn untitled_manifest_is_named_after_the_playlist_without_its_extension() {
    let Some(env) = Env::new().await else { return };
    let mgr = env.manager().await;
    let mut m = env.media(
        MediaSourceKind::Dash,
        "dash/manifest.mpd",
        OutputContainer::Mkv,
    );
    m.title = None;
    let id = env.add(&mgr, m, None).await;
    let path = completed(&mgr, id).await;
    assert_eq!(path.file_name().unwrap(), "manifest.mkv");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn login_in_the_manifest_address_is_sent_and_stored_encrypted() {
    let Some(env) = Env::new().await else { return };
    env.server.require_static_login("viewer", "p4ss");
    let mgr = env.manager().await;
    let mut m = env.media(
        MediaSourceKind::Hls,
        "hls/master.m3u8",
        OutputContainer::Mp4,
    );
    m.url = m.url.replacen("http://", "http://viewer:p4ss@", 1);
    m.format_id = Some("v0".into());
    let id = env.add(&mgr, m, None).await;
    let info = mgr.get(id).unwrap();
    assert!(!info.url.contains("p4ss"), "{}", info.url);
    // Every playlist, key and segment request carried the login.
    let path = completed(&mgr, id).await;
    verify(&env.tools, &path);
    assert_eq!(
        env.server
            .static_stats("hls/master.m3u8")
            .rejected
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    // Everything SQLite wrote, including the WAL.
    let mut raw = Vec::new();
    for suffix in ["", "-wal", "-journal"] {
        if let Ok(b) = std::fs::read(format!("{}{suffix}", env.db_path.display())) {
            raw.extend(b);
        }
    }
    let contains = |needle: &[u8]| raw.windows(needle.len()).any(|w| w == needle);
    assert!(contains(b"master.m3u8"), "the scan sees the record");
    assert!(!contains(b"p4ss"), "password stored in plain text");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hls_alternate_audio_track_is_used() {
    let Some(env) = Env::new().await else { return };
    let mgr = env.manager().await;
    let mut m = env.media(
        MediaSourceKind::Hls,
        "hls/master.m3u8",
        OutputContainer::Mkv,
    );
    m.format_id = Some("v0".into());
    m.audio_format_id = Some("a1".into());
    let id = env.add(&mgr, m, None).await;
    let path = completed(&mgr, id).await;
    let p = verify(&env.tools, &path);
    assert_eq!(p.kinds(), ["video", "audio"]);
    let count = |f: &str| {
        env.server
            .static_stats(f)
            .requests
            .load(std::sync::atomic::Ordering::SeqCst)
    };
    assert!(count("hls/a2/seg000.ts") >= 1, "Arabic track downloaded");
    assert_eq!(count("hls/a/seg000.ts"), 0, "default track not downloaded");
}
