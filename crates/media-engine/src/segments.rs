//! Parallel, resumable download of HLS/DASH segments.
//!
//! Each segment is written to its own file and renamed into place only after
//! it is complete and fsynced, so an interrupted download resumes by
//! skipping finished segments. Segments are concatenated (init section
//! first) once all are present.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use bytes::Bytes;
use futures::TryStreamExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use url::Url;
use velox_core::ratelimit::RateLimiter;
use velox_http::{fetch_bytes, reqwest::Client, HttpError};

use crate::{MediaError, RequestInfo};

/// Largest single segment accepted (sanity bound).
const MAX_SEGMENT: usize = 512 * 1024 * 1024;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRef {
    pub uri: Url,
    pub iv: Option<[u8; 16]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentJob {
    pub url: Url,
    /// Inclusive byte range.
    pub range: Option<(u64, u64)>,
    pub key: Option<KeyRef>,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackPlan {
    /// "video", "audio", "sub-ar", ...
    pub name: String,
    /// Extension of the concatenated file ("ts", "mp4", "aac", "vtt"...).
    pub ext: String,
    pub init: Option<(Url, Option<(u64, u64)>)>,
    pub segments: Vec<SegmentJob>,
}

/// Shared progress counters for all tracks of a media download.
#[derive(Debug, Default)]
pub struct Progress {
    pub bytes: AtomicU64,
    pub done: AtomicU64,
    pub total: AtomicU64,
}

pub fn decrypt_aes128(
    mut data: Vec<u8>,
    key: &[u8; 16],
    iv: &[u8; 16],
) -> Result<Vec<u8>, MediaError> {
    let len = Aes128CbcDec::new(key.into(), iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut data)
        .map_err(|_| MediaError::Parse("segment decryption failed (wrong key?)".into()))?
        .len();
    data.truncate(len);
    Ok(data)
}

fn sequence_iv(seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}

pub struct Downloader {
    pub client: Client,
    pub info: RequestInfo,
    pub concurrency: usize,
    pub cancel: CancellationToken,
    pub progress: Arc<Progress>,
    pub limiters: Vec<Arc<RateLimiter>>,
    keys: Mutex<HashMap<Url, [u8; 16]>>,
}

impl Downloader {
    pub fn new(
        client: Client,
        info: RequestInfo,
        concurrency: usize,
        cancel: CancellationToken,
        limiters: Vec<Arc<RateLimiter>>,
    ) -> Self {
        Self {
            client,
            info,
            concurrency: concurrency.clamp(1, 16),
            cancel,
            progress: Arc::new(Progress::default()),
            limiters,
            keys: Mutex::new(HashMap::new()),
        }
    }

    async fn fetch(
        &self,
        url: &Url,
        range: Option<(u64, u64)>,
        max: usize,
    ) -> Result<Bytes, MediaError> {
        let ctx = self.info.context_for(url.as_str());
        let mut delay = Duration::from_millis(500);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let res = tokio::select! {
                _ = self.cancel.cancelled() => return Err(MediaError::Cancelled),
                r = fetch_bytes(&self.client, &ctx, range, max) => r,
            };
            match res {
                Ok(f) => {
                    let wait = self
                        .limiters
                        .iter()
                        .map(|l| l.reserve(f.body.len() as u64))
                        .max()
                        .unwrap_or_default();
                    if !wait.is_zero() {
                        tokio::select! {
                            _ = self.cancel.cancelled() => return Err(MediaError::Cancelled),
                            _ = tokio::time::sleep(wait) => {}
                        }
                    }
                    return Ok(f.body);
                }
                Err(e) if e.is_transient() && attempt < 5 => {
                    tokio::select! {
                        _ = self.cancel.cancelled() => return Err(MediaError::Cancelled),
                        _ = tokio::time::sleep(delay) => {}
                    }
                    delay *= 2;
                }
                Err(e) => return Err(MediaError::Http(e)),
            }
        }
    }

    async fn key(&self, uri: &Url) -> Result<[u8; 16], MediaError> {
        // Held across the fetch so concurrent segments request each key once.
        let mut keys = self.keys.lock().await;
        if let Some(k) = keys.get(uri) {
            return Ok(*k);
        }
        let body = self.fetch(uri, None, 1024).await?;
        let key: [u8; 16] = body.as_ref().try_into().map_err(|_| {
            MediaError::Parse(format!("AES-128 key must be 16 bytes, got {}", body.len()))
        })?;
        keys.insert(uri.clone(), key);
        Ok(key)
    }

    async fn one(&self, job: &SegmentJob, path: PathBuf) -> Result<(), MediaError> {
        let mut data = self.fetch(&job.url, job.range, MAX_SEGMENT).await?.to_vec();
        if let Some(k) = &job.key {
            let key = self.key(&k.uri).await?;
            let iv = k.iv.unwrap_or_else(|| sequence_iv(job.sequence));
            data = decrypt_aes128(data, &key, &iv)?;
        }
        let len = data.len() as u64;
        let tmp = path.with_extension("part");
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&data)?;
            f.sync_data()?;
            drop(f);
            std::fs::rename(&tmp, &path)
        })
        .await
        .map_err(|e| MediaError::Io(e.to_string()))?
        .map_err(|e| MediaError::Io(e.to_string()))?;
        self.progress.bytes.fetch_add(len, Ordering::Relaxed);
        self.progress.done.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Download every segment of `plan` into `dir/<name>/`, then concatenate
    /// them into `dir/<name>.<ext>`.
    pub async fn download_track(
        &self,
        plan: &TrackPlan,
        dir: &Path,
    ) -> Result<PathBuf, MediaError> {
        let track_dir = dir.join(&plan.name);
        std::fs::create_dir_all(&track_dir).map_err(|e| MediaError::Io(e.to_string()))?;
        if let Some((url, range)) = &plan.init {
            let p = track_dir.join("init.seg");
            if !p.exists() {
                let body = self.fetch(url, *range, MAX_SEGMENT).await?;
                std::fs::write(track_dir.join("init.part"), &body)
                    .map_err(|e| MediaError::Io(e.to_string()))?;
                std::fs::rename(track_dir.join("init.part"), &p)
                    .map_err(|e| MediaError::Io(e.to_string()))?;
            }
        }
        let paths: Vec<PathBuf> = (0..plan.segments.len())
            .map(|i| track_dir.join(format!("{i:07}.seg")))
            .collect();
        let mut pending = Vec::new();
        for (job, path) in plan.segments.iter().zip(&paths) {
            match std::fs::metadata(path) {
                Ok(m) => {
                    self.progress.bytes.fetch_add(m.len(), Ordering::Relaxed);
                    self.progress.done.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => pending.push((job, path.clone())),
            }
        }
        futures::stream::iter(pending.into_iter().map(Ok::<_, MediaError>))
            .try_for_each_concurrent(self.concurrency, |(job, path)| async move {
                self.one(job, path).await
            })
            .await?;

        let out = dir.join(format!("{}.{}", plan.name, plan.ext));
        let init = plan.init.as_ref().map(|_| track_dir.join("init.seg"));
        tokio::task::spawn_blocking({
            let out = out.clone();
            move || -> std::io::Result<()> {
                let tmp = out.with_extension("concat-part");
                let mut w =
                    std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
                for p in init.iter().chain(paths.iter()) {
                    let mut r = std::fs::File::open(p)?;
                    std::io::copy(&mut r, &mut w)?;
                }
                let f = w.into_inner().map_err(|e| e.into_error())?;
                f.sync_all()?;
                drop(f);
                std::fs::rename(&tmp, &out)
            }
        })
        .await
        .map_err(|e| MediaError::Io(e.to_string()))?
        .map_err(|e| MediaError::Io(e.to_string()))?;
        Ok(out)
    }
}

/// WebVTT segments: keep the header of the first file only.
pub fn merge_webvtt(text: &str) -> String {
    let mut out = String::new();
    let mut first = true;
    for block in text.split("WEBVTT").filter(|b| !b.trim().is_empty()) {
        if first {
            out.push_str("WEBVTT");
            first = false;
        }
        // Drop per-segment header lines (e.g. X-TIMESTAMP-MAP) up to the first blank line.
        let body = block.split_once("\n\n").map(|(_, b)| b).unwrap_or("");
        out.push_str("\n\n");
        out.push_str(body.trim());
    }
    out.push('\n');
    out
}

/// Map HTTP errors of manifest fetches to the user-facing kind.
pub fn is_expired(e: &MediaError) -> bool {
    matches!(
        e,
        MediaError::Http(HttpError::Status {
            status: 401 | 403 | 404 | 410,
            ..
        })
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncryptMut;

    #[test]
    fn aes_roundtrip_and_default_iv() {
        let key = [7u8; 16];
        let iv = sequence_iv(42);
        assert_eq!(iv[15], 42);
        let plain = b"hello segment data, more than one block long!".to_vec();
        let mut buf = plain.clone();
        buf.resize(plain.len() + 16, 0);
        let enc = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
            .unwrap()
            .to_vec();
        assert_eq!(enc.len() % 16, 0);
        assert_eq!(decrypt_aes128(enc.clone(), &key, &iv).unwrap(), plain);
        assert_ne!(decrypt_aes128(enc, &[8u8; 16], &iv).ok(), Some(plain));
    }

    #[test]
    fn webvtt_merge() {
        let a = "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000\n\n00:00.000 --> 00:01.000\nمرحبا\n";
        let b = "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000\n\n00:01.000 --> 00:02.000\nworld\n";
        let m = merge_webvtt(&format!("{a}{b}"));
        assert_eq!(m.matches("WEBVTT").count(), 1);
        assert!(m.contains("مرحبا") && m.contains("world"));
        assert!(!m.contains("X-TIMESTAMP-MAP"));
    }
}
