//! HTTP Live Streaming playlist parsing (RFC 8216).
//!
//! Supports master playlists (variants, alternative audio/subtitle
//! renditions) and media playlists (segments, byte ranges, init sections,
//! AES-128 keys). DRM key systems (SAMPLE-AES, FairPlay, Widevine,
//! PlayReady) are detected and reported, never handled.

use std::collections::HashMap;

use url::Url;

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub uri: Url,
    pub bandwidth: Option<u64>,
    pub average_bandwidth: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f32>,
    pub codecs: Option<String>,
    pub audio_group: Option<String>,
    pub subtitles_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rendition {
    /// AUDIO, SUBTITLES, VIDEO or CLOSED-CAPTIONS.
    pub kind: String,
    pub group_id: String,
    pub name: Option<String>,
    pub language: Option<String>,
    pub uri: Option<Url>,
    pub default: bool,
    pub autoselect: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MasterPlaylist {
    pub variants: Vec<Variant>,
    pub renditions: Vec<Rendition>,
    /// Session keys announcing DRM for the whole presentation.
    pub drm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyMethod {
    None,
    Aes128,
    /// SAMPLE-AES / SAMPLE-AES-CTR or a non-identity key format: DRM.
    Protected(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub method: KeyMethod,
    pub uri: Option<Url>,
    /// Explicit IV (16 bytes); otherwise the media sequence number is used.
    pub iv: Option<[u8; 16]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub uri: Url,
    pub duration: f64,
    pub byte_range: Option<ByteRange>,
    pub key: Option<Key>,
    /// Initialization section (fMP4) that applies to this segment.
    pub map: Option<(Url, Option<ByteRange>)>,
    pub sequence: u64,
    pub discontinuity: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlaylist {
    pub target_duration: f64,
    pub media_sequence: u64,
    pub segments: Vec<Segment>,
    /// `#EXT-X-ENDLIST` present (VOD); otherwise the stream is live.
    pub ended: bool,
    pub drm: bool,
}

impl MediaPlaylist {
    pub fn duration(&self) -> f64 {
        self.segments.iter().map(|s| s.duration).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Playlist {
    Master(MasterPlaylist),
    Media(MediaPlaylist),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HlsError {
    #[error("not an HLS playlist (missing #EXTM3U)")]
    NotHls,
    #[error("invalid playlist: {0}")]
    Invalid(String),
}

/// Parse `KEY=VALUE,KEY="quoted, value"` attribute lists.
pub fn parse_attributes(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ',' || c.is_whitespace()) {
            chars.next();
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' {
                break;
            }
            key.push(c);
            chars.next();
        }
        if chars.next().is_none() || key.is_empty() {
            break;
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                value.push(c);
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ',' {
                    break;
                }
                value.push(c);
                chars.next();
            }
        }
        out.insert(key.trim().to_ascii_uppercase(), value);
    }
    out
}

fn parse_byte_range(s: &str, prev_end: Option<u64>) -> Option<ByteRange> {
    let (len, off) = match s.split_once('@') {
        Some((l, o)) => (l.trim().parse().ok()?, Some(o.trim().parse().ok()?)),
        None => (s.trim().parse().ok()?, None),
    };
    Some(ByteRange {
        length: len,
        offset: off.or(prev_end).unwrap_or(0),
    })
}

fn parse_iv(s: &str) -> Option<[u8; 16]> {
    let hexs = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    let bytes = hex::decode(format!("{hexs:0>32}")).ok()?;
    bytes.try_into().ok()
}

fn key_from(attrs: &HashMap<String, String>, base: &Url) -> Key {
    let method = attrs
        .get("METHOD")
        .map(|m| m.to_ascii_uppercase())
        .unwrap_or_default();
    let format = attrs.get("KEYFORMAT").map(|f| f.to_ascii_lowercase());
    let uri = attrs.get("URI").and_then(|u| base.join(u).ok());
    let method = match method.as_str() {
        "NONE" | "" => KeyMethod::None,
        "AES-128" if format.as_deref().is_none_or(|f| f == "identity") => KeyMethod::Aes128,
        other => KeyMethod::Protected(
            format!("{other} {}", format.unwrap_or_default())
                .trim()
                .to_string(),
        ),
    };
    Key {
        method,
        uri,
        iv: attrs.get("IV").and_then(|v| parse_iv(v)),
    }
}

fn resolution(s: &str) -> (Option<u32>, Option<u32>) {
    match s.split_once(['x', 'X']) {
        Some((w, h)) => (w.trim().parse().ok(), h.trim().parse().ok()),
        None => (None, None),
    }
}

/// Parse a playlist fetched from `base`.
pub fn parse(text: &str, base: &Url) -> Result<Playlist, HlsError> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next() != Some("#EXTM3U") {
        return Err(HlsError::NotHls);
    }
    let is_master = text.contains("#EXT-X-STREAM-INF");
    if is_master {
        let mut variants = Vec::new();
        let mut renditions = Vec::new();
        let mut drm = false;
        let mut pending: Option<HashMap<String, String>> = None;
        for line in lines {
            if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
                pending = Some(parse_attributes(rest));
            } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA:") {
                let a = parse_attributes(rest);
                renditions.push(Rendition {
                    kind: a
                        .get("TYPE")
                        .cloned()
                        .unwrap_or_default()
                        .to_ascii_uppercase(),
                    group_id: a.get("GROUP-ID").cloned().unwrap_or_default(),
                    name: a.get("NAME").cloned(),
                    language: a.get("LANGUAGE").cloned(),
                    uri: a.get("URI").and_then(|u| base.join(u).ok()),
                    default: a
                        .get("DEFAULT")
                        .is_some_and(|v| v.eq_ignore_ascii_case("YES")),
                    autoselect: a
                        .get("AUTOSELECT")
                        .is_some_and(|v| v.eq_ignore_ascii_case("YES")),
                });
            } else if let Some(rest) = line.strip_prefix("#EXT-X-SESSION-KEY:") {
                if matches!(
                    key_from(&parse_attributes(rest), base).method,
                    KeyMethod::Protected(_)
                ) {
                    drm = true;
                }
            } else if !line.starts_with('#') {
                if let Some(a) = pending.take() {
                    let uri = base
                        .join(line)
                        .map_err(|e| HlsError::Invalid(e.to_string()))?;
                    let (width, height) = a
                        .get("RESOLUTION")
                        .map(|r| resolution(r))
                        .unwrap_or((None, None));
                    variants.push(Variant {
                        uri,
                        bandwidth: a.get("BANDWIDTH").and_then(|v| v.parse().ok()),
                        average_bandwidth: a.get("AVERAGE-BANDWIDTH").and_then(|v| v.parse().ok()),
                        width,
                        height,
                        frame_rate: a.get("FRAME-RATE").and_then(|v| v.parse().ok()),
                        codecs: a.get("CODECS").cloned(),
                        audio_group: a.get("AUDIO").cloned(),
                        subtitles_group: a.get("SUBTITLES").cloned(),
                    });
                }
            }
        }
        if variants.is_empty() {
            return Err(HlsError::Invalid("master playlist without variants".into()));
        }
        return Ok(Playlist::Master(MasterPlaylist {
            variants,
            renditions,
            drm,
        }));
    }

    let mut target_duration = 0.0;
    let mut media_sequence = 0u64;
    let mut segments = Vec::new();
    let mut ended = false;
    let mut drm = false;
    let mut key: Option<Key> = None;
    let mut map: Option<(Url, Option<ByteRange>)> = None;
    let mut duration: Option<f64> = None;
    let mut range: Option<ByteRange> = None;
    let mut discontinuity = false;
    let mut last_range_end: Option<u64> = None;
    for line in lines {
        if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            target_duration = v.trim().parse().unwrap_or(0.0);
        } else if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            media_sequence = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("#EXTINF:") {
            duration = v.split(',').next().and_then(|d| d.trim().parse().ok());
        } else if let Some(v) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            range = parse_byte_range(v, last_range_end);
        } else if let Some(v) = line.strip_prefix("#EXT-X-KEY:") {
            let k = key_from(&parse_attributes(v), base);
            if matches!(k.method, KeyMethod::Protected(_)) {
                drm = true;
            }
            key = (k.method != KeyMethod::None).then_some(k);
        } else if let Some(v) = line.strip_prefix("#EXT-X-MAP:") {
            let a = parse_attributes(v);
            if let Some(u) = a.get("URI").and_then(|u| base.join(u).ok()) {
                map = Some((
                    u,
                    a.get("BYTERANGE")
                        .and_then(|r| parse_byte_range(r, Some(0))),
                ));
            }
        } else if line == "#EXT-X-DISCONTINUITY" {
            discontinuity = true;
        } else if line == "#EXT-X-ENDLIST" {
            ended = true;
        } else if line.starts_with("#EXT-X-PLAYLIST-TYPE:VOD") {
            // VOD playlists always end; ENDLIST is still expected.
        } else if !line.starts_with('#') {
            let uri = base
                .join(line)
                .map_err(|e| HlsError::Invalid(e.to_string()))?;
            if let Some(r) = &range {
                last_range_end = Some(r.offset + r.length);
            }
            segments.push(Segment {
                uri,
                duration: duration.take().unwrap_or(target_duration),
                byte_range: range.take(),
                key: key.clone(),
                map: map.clone(),
                sequence: media_sequence + segments.len() as u64,
                discontinuity: std::mem::take(&mut discontinuity),
            });
        }
    }
    Ok(Playlist::Media(MediaPlaylist {
        target_duration,
        media_sequence,
        segments,
        ended,
        drm,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://cdn.example.com/video/master.m3u8?token=abc").unwrap()
    }

    #[test]
    fn attributes_with_quotes_and_commas() {
        let a = parse_attributes(
            r#"BANDWIDTH=1280000,CODECS="avc1.4d401f,mp4a.40.2",RESOLUTION=640x360,AUDIO="aud""#,
        );
        assert_eq!(a["BANDWIDTH"], "1280000");
        assert_eq!(a["CODECS"], "avc1.4d401f,mp4a.40.2");
        assert_eq!(a["RESOLUTION"], "640x360");
        assert_eq!(a["AUDIO"], "aud");
    }

    #[test]
    fn master_playlist() {
        let text = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"audio/en.m3u8\"\n#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"العربية\",LANGUAGE=\"ar\",URI=\"subs/ar.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,FRAME-RATE=25.000,CODECS=\"avc1.4d401e,mp4a.40.2\",AUDIO=\"aud\",SUBTITLES=\"subs\"\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=4000000,RESOLUTION=1920x1080\nhttps://other.example.com/hi.m3u8\n";
        let Playlist::Master(m) = parse(text, &base()).unwrap() else {
            panic!()
        };
        assert_eq!(m.variants.len(), 2);
        assert_eq!(
            m.variants[0].uri.as_str(),
            "https://cdn.example.com/video/low/index.m3u8"
        );
        assert_eq!(m.variants[0].height, Some(360));
        assert_eq!(m.variants[0].frame_rate, Some(25.0));
        assert_eq!(m.variants[0].audio_group.as_deref(), Some("aud"));
        assert_eq!(m.variants[1].uri.host_str(), Some("other.example.com"));
        assert_eq!(m.renditions.len(), 2);
        assert_eq!(m.renditions[1].language.as_deref(), Some("ar"));
        assert!(!m.drm);
    }

    #[test]
    fn media_playlist_with_key_map_and_ranges() {
        let text = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:10\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"720@0\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x0000000000000000000000000000000A\n#EXTINF:4.0,\n#EXT-X-BYTERANGE:1000@720\nmain.mp4\n#EXTINF:3.5,\n#EXT-X-BYTERANGE:500\nmain.mp4\n#EXT-X-DISCONTINUITY\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:2,\nlast.ts\n#EXT-X-ENDLIST\n";
        let Playlist::Media(m) = parse(text, &base()).unwrap() else {
            panic!()
        };
        assert!(m.ended);
        assert!(!m.drm);
        assert_eq!(m.segments.len(), 3);
        assert_eq!(m.segments[0].sequence, 10);
        assert_eq!(
            m.segments[0].byte_range,
            Some(ByteRange {
                offset: 720,
                length: 1000
            })
        );
        assert_eq!(
            m.segments[1].byte_range,
            Some(ByteRange {
                offset: 1720,
                length: 500
            })
        );
        let k = m.segments[0].key.as_ref().unwrap();
        assert_eq!(k.method, KeyMethod::Aes128);
        assert_eq!(k.iv.unwrap()[15], 10);
        assert_eq!(
            m.segments[0].map.as_ref().unwrap().1,
            Some(ByteRange {
                offset: 0,
                length: 720
            })
        );
        assert!(m.segments[2].key.is_none());
        assert!(m.segments[2].discontinuity);
        assert!((m.duration() - 9.5).abs() < 1e-9);
    }

    #[test]
    fn drm_is_detected() {
        let fairplay = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXTINF:6,\na.ts\n";
        let Playlist::Media(m) = parse(fairplay, &base()).unwrap() else {
            panic!()
        };
        assert!(m.drm);
        let widevine = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-KEY:METHOD=AES-128,URI=\"data:...\",KEYFORMAT=\"urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed\"\n#EXTINF:6,\na.ts\n";
        let Playlist::Media(m) = parse(widevine, &base()).unwrap() else {
            panic!()
        };
        assert!(
            m.drm,
            "AES-128 with a non-identity key format is a DRM system"
        );
        let session = "#EXTM3U\n#EXT-X-SESSION-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n";
        let Playlist::Master(m) = parse(session, &base()).unwrap() else {
            panic!()
        };
        assert!(m.drm);
    }

    #[test]
    fn rejects_non_hls() {
        assert_eq!(parse("<html>", &base()), Err(HlsError::NotHls));
    }
}
