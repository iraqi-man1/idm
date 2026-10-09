//! FFmpeg remuxing and audio extraction.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio_util::sync::CancellationToken;
use velox_types::OutputContainer;

use crate::tools::{command, tail};
use crate::MediaError;

#[derive(Debug, Clone, Default)]
pub struct MuxInput {
    pub video: Option<PathBuf>,
    pub audio: Option<PathBuf>,
    /// Subtitle files with their language code.
    pub subtitles: Vec<(PathBuf, String)>,
}

/// Extension of the output file for a container (`Original` keeps `fallback`).
pub fn output_ext(container: OutputContainer, fallback: &str) -> String {
    container
        .extension()
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

/// Demuxer for a downloaded track. Forcing it keeps FFmpeg from probing
/// remote-supplied data as something else (e.g. an HLS playlist that
/// references local files).
fn demuxer(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "ts" => "mpegts",
        "mp4" | "m4s" | "m4a" | "m4v" | "mov" => "mov",
        "aac" => "aac",
        "mp3" => "mp3",
        "vtt" => "webvtt",
        "webm" | "mkv" => "matroska",
        _ => return None,
    })
}

/// `-i path`, restricted to local files and the expected container.
fn add_input(a: &mut Vec<String>, path: &Path) {
    a.extend(["-protocol_whitelist".into(), "file".into()]);
    if let Some(f) = demuxer(path) {
        a.extend(["-f".into(), f.into()]);
    }
    a.extend(["-i".into(), path.to_string_lossy().to_string()]);
}

/// FFmpeg arguments for a mux job (exposed for tests).
pub fn args(
    input: &MuxInput,
    container: OutputContainer,
    out: &Path,
    transcode_audio: bool,
) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-loglevel".into(),
        "error".into(),
    ];
    let mut inputs = 0;
    let mut maps = Vec::new();
    let audio_only = container.is_audio_only();
    if let Some(v) = &input.video {
        add_input(&mut a, v);
        if !audio_only {
            maps.push(format!("{inputs}:v?"));
        }
        if input.audio.is_none() {
            maps.push(format!("{inputs}:a?"));
        }
        inputs += 1;
    }
    if let Some(au) = &input.audio {
        add_input(&mut a, au);
        maps.push(format!("{inputs}:a"));
        inputs += 1;
    }
    let embed_subs =
        !audio_only && matches!(container, OutputContainer::Mp4 | OutputContainer::Mkv);
    let mut sub_index = 0;
    if embed_subs {
        for (path, _) in &input.subtitles {
            add_input(&mut a, path);
            maps.push(format!("{inputs}:s?"));
            inputs += 1;
        }
    }
    for m in maps {
        a.extend(["-map".into(), m]);
    }
    match container {
        OutputContainer::Mp3 => a.extend([
            "-vn".into(),
            "-c:a".into(),
            "libmp3lame".into(),
            "-q:a".into(),
            "2".into(),
        ]),
        OutputContainer::M4a => {
            a.push("-vn".into());
            if transcode_audio {
                a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
            } else {
                a.extend(["-c:a".into(), "copy".into()]);
            }
            a.extend(["-movflags".into(), "+faststart".into()]);
        }
        OutputContainer::Mp4 => {
            a.extend(["-c:v".into(), "copy".into()]);
            if transcode_audio {
                a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
            } else {
                a.extend(["-c:a".into(), "copy".into()]);
            }
            if embed_subs {
                a.extend(["-c:s".into(), "mov_text".into()]);
            }
            a.extend(["-movflags".into(), "+faststart".into()]);
        }
        OutputContainer::Mkv | OutputContainer::Original => a.extend(["-c".into(), "copy".into()]),
    }
    if embed_subs {
        for (i, (_, lang)) in input.subtitles.iter().enumerate() {
            a.extend([format!("-metadata:s:s:{i}"), format!("language={lang}")]);
            sub_index += 1;
        }
    }
    let _ = sub_index;
    a.push(out.to_string_lossy().to_string());
    a
}

async fn run(ffmpeg: &Path, args: &[String], cancel: &CancellationToken) -> Result<(), String> {
    let mut c = command(ffmpeg);
    c.args(args).stdout(Stdio::null()).stderr(Stdio::piped());
    let child = c.spawn().map_err(|e| format!("cannot run FFmpeg: {e}"))?;
    tokio::select! {
        _ = cancel.cancelled() => Err("cancelled".into()),
        out = child.wait_with_output() => {
            let out = out.map_err(|e| e.to_string())?;
            if out.status.success() { Ok(()) } else { Err(tail(&String::from_utf8_lossy(&out.stderr), 1500)) }
        }
    }
}

/// Mux / remux `input` into `out`. Copies streams when possible and
/// re-encodes audio only when the container cannot hold the source codec.
pub async fn mux(
    ffmpeg: &Path,
    input: &MuxInput,
    container: OutputContainer,
    out: &Path,
    cancel: &CancellationToken,
) -> Result<(), MediaError> {
    let first = run(ffmpeg, &args(input, container, out, false), cancel).await;
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    match first {
        Ok(()) => Ok(()),
        Err(e) if matches!(container, OutputContainer::Mp4 | OutputContainer::M4a) => {
            // e.g. Opus/Vorbis audio cannot be copied into MP4: re-encode audio.
            tracing::info!(error = %e, "stream copy failed, re-encoding audio");
            run(ffmpeg, &args(input, container, out, true), cancel)
                .await
                .map_err(|e2| {
                    if cancel.is_cancelled() {
                        MediaError::Cancelled
                    } else {
                        MediaError::Tool(format!("FFmpeg failed: {e2}"))
                    }
                })
        }
        Err(e) => Err(MediaError::Tool(format!("FFmpeg failed: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp4_with_separate_audio_and_subs() {
        let input = MuxInput {
            video: Some("/w/video.ts".into()),
            audio: Some("/w/audio.ts".into()),
            subtitles: vec![("/w/sub-ar.vtt".into(), "ar".into())],
        };
        let a = args(&input, OutputContainer::Mp4, Path::new("/w/out.mp4"), false).join(" ");
        assert!(a.contains("-map 0:v? -map 1:a -map 2:s?"), "{a}");
        assert!(a.contains("-c:v copy -c:a copy -c:s mov_text"), "{a}");
        assert!(a.contains("-metadata:s:s:0 language=ar"), "{a}");
        assert!(a.ends_with("/w/out.mp4"));
        assert!(
            a.contains("-protocol_whitelist file -f mpegts -i /w/video.ts"),
            "{a}"
        );
        assert!(
            a.contains("-protocol_whitelist file -f webvtt -i /w/sub-ar.vtt"),
            "{a}"
        );
    }

    #[test]
    fn audio_extraction() {
        let input = MuxInput {
            video: Some("/w/v.ts".into()),
            ..Default::default()
        };
        let a = args(&input, OutputContainer::Mp3, Path::new("/w/o.mp3"), false).join(" ");
        assert!(a.contains("-map 0:a?") && !a.contains("0:v"), "{a}");
        assert!(a.contains("-vn -c:a libmp3lame"), "{a}");
        let a = args(&input, OutputContainer::M4a, Path::new("/w/o.m4a"), true).join(" ");
        assert!(a.contains("-c:a aac"), "{a}");
    }
}
