//! MPEG-DASH manifest (MPD) parsing for static (on-demand) presentations.
//!
//! Supports BaseURL resolution, SegmentTemplate (with `$Number$`/`$Time$`
//! and SegmentTimeline), SegmentList and single-file representations
//! (SegmentBase / BaseURL). `ContentProtection` marks the presentation as
//! DRM-protected; such content is reported and never downloaded.

use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Text,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegRef {
    pub url: Url,
    /// Inclusive byte range.
    pub range: Option<(u64, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Representation {
    pub id: String,
    pub kind: TrackKind,
    pub mime: Option<String>,
    pub codecs: Option<String>,
    pub bandwidth: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f32>,
    pub lang: Option<String>,
    pub init: Option<SegRef>,
    pub segments: Vec<SegRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub is_live: bool,
    pub drm: bool,
    pub duration: Option<f64>,
    pub representations: Vec<Representation>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DashError {
    #[error("not a DASH manifest: {0}")]
    NotDash(String),
    #[error("invalid manifest: {0}")]
    Invalid(String),
}

/// Parse an ISO 8601 duration such as `PT1H2M3.5S` or `P1DT2S` into seconds.
pub fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim();
    let rest = s.strip_prefix('P')?;
    let (date, time) = match rest.split_once('T') {
        Some((d, t)) => (d, t),
        None => (rest, ""),
    };
    let mut total = 0.0;
    let mut num = String::new();
    for c in date.chars() {
        match c {
            '0'..='9' | '.' => num.push(c),
            'Y' => total += num.parse::<f64>().ok()? * 365.0 * 86400.0,
            'M' => total += num.parse::<f64>().ok()? * 30.0 * 86400.0,
            'W' => total += num.parse::<f64>().ok()? * 7.0 * 86400.0,
            'D' => total += num.parse::<f64>().ok()? * 86400.0,
            _ => return None,
        }
        if c.is_ascii_alphabetic() {
            num.clear();
        }
    }
    for c in time.chars() {
        match c {
            '0'..='9' | '.' => num.push(c),
            'H' => total += num.parse::<f64>().ok()? * 3600.0,
            'M' => total += num.parse::<f64>().ok()? * 60.0,
            'S' => total += num.parse::<f64>().ok()?,
            _ => return None,
        }
        if c.is_ascii_alphabetic() {
            num.clear();
        }
    }
    Some(total)
}

/// Expand `$RepresentationID$`, `$Bandwidth$`, `$Number%05d$`, `$Time$`, `$$`.
pub fn expand_template(
    t: &str,
    rep_id: &str,
    bandwidth: Option<u64>,
    number: Option<u64>,
    time: Option<u64>,
) -> String {
    let mut out = String::with_capacity(t.len() + 16);
    let mut rest = t;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let token = &after[..end];
        rest = &after[end + 1..];
        if token.is_empty() {
            out.push('$');
            continue;
        }
        let (name, fmt) = match token.split_once('%') {
            Some((n, f)) => (n, Some(f)),
            None => (token, None),
        };
        let value: Option<String> = match name {
            "RepresentationID" => Some(rep_id.to_string()),
            "Bandwidth" => bandwidth.map(|b| b.to_string()),
            "Number" => number.map(|n| n.to_string()),
            "Time" => time.map(|n| n.to_string()),
            _ => None,
        };
        match value {
            Some(v) => {
                let width = fmt
                    .and_then(|f| {
                        f.trim_end_matches('d')
                            .trim_start_matches('0')
                            .parse::<usize>()
                            .ok()
                    })
                    .unwrap_or(0);
                out.push_str(&format!("{v:0>width$}"));
            }
            None => {
                out.push('$');
                out.push_str(token);
                out.push('$');
            }
        }
    }
    out.push_str(rest);
    out
}

fn child<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn children<'a>(
    n: roxmltree::Node<'a, 'a>,
    name: &'a str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'a>> + 'a {
    n.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn resolve_base(base: &Url, node: roxmltree::Node) -> Url {
    match child(node, "BaseURL").and_then(|b| b.text()).map(str::trim) {
        Some(t) if !t.is_empty() => base.join(t).unwrap_or_else(|_| base.clone()),
        _ => base.clone(),
    }
}

/// SegmentTemplate attributes merged from AdaptationSet and Representation.
#[derive(Default, Clone)]
struct Template {
    media: Option<String>,
    init: Option<String>,
    start_number: Option<u64>,
    timescale: Option<u64>,
    duration: Option<u64>,
    timeline: Option<Vec<(Option<u64>, u64, i64)>>,
}

fn read_template(n: roxmltree::Node, parent: Option<&Template>) -> Option<Template> {
    let t = child(n, "SegmentTemplate");
    if t.is_none() {
        return parent.cloned();
    }
    let t = t.unwrap();
    let mut out = parent.cloned().unwrap_or_default();
    if let Some(v) = t.attribute("media") {
        out.media = Some(v.to_string());
    }
    if let Some(v) = t.attribute("initialization") {
        out.init = Some(v.to_string());
    }
    if let Some(v) = t.attribute("startNumber").and_then(|v| v.parse().ok()) {
        out.start_number = Some(v);
    }
    if let Some(v) = t.attribute("timescale").and_then(|v| v.parse().ok()) {
        out.timescale = Some(v);
    }
    if let Some(v) = t.attribute("duration").and_then(|v| v.parse().ok()) {
        out.duration = Some(v);
    }
    if let Some(tl) = child(t, "SegmentTimeline") {
        out.timeline = Some(
            children(tl, "S")
                .map(|s| {
                    (
                        s.attribute("t").and_then(|v| v.parse().ok()),
                        s.attribute("d").and_then(|v| v.parse().ok()).unwrap_or(0),
                        s.attribute("r").and_then(|v| v.parse().ok()).unwrap_or(0),
                    )
                })
                .collect(),
        );
    }
    Some(out)
}

fn parse_range(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// Maximum number of segments accepted per representation (sanity bound).
const MAX_SEGMENTS: usize = 200_000;

pub fn parse(text: &str, manifest_url: &Url) -> Result<Manifest, DashError> {
    let doc = roxmltree::Document::parse(text).map_err(|e| DashError::NotDash(e.to_string()))?;
    let mpd = doc.root_element();
    if mpd.tag_name().name() != "MPD" {
        return Err(DashError::NotDash(format!(
            "root element is {}",
            mpd.tag_name().name()
        )));
    }
    let is_live = mpd.attribute("type") == Some("dynamic");
    let duration = mpd
        .attribute("mediaPresentationDuration")
        .and_then(parse_duration);
    let mpd_base = resolve_base(manifest_url, mpd);
    let period = child(mpd, "Period").ok_or_else(|| DashError::Invalid("no Period".into()))?;
    let period_duration = period
        .attribute("duration")
        .and_then(parse_duration)
        .or(duration);
    let period_base = resolve_base(&mpd_base, period);
    let mut drm = doc
        .descendants()
        .any(|n| n.is_element() && n.tag_name().name() == "ContentProtection");
    let mut reps = Vec::new();

    for set in children(period, "AdaptationSet") {
        let set_base = resolve_base(&period_base, set);
        let set_mime = set.attribute("mimeType");
        let set_type = set.attribute("contentType");
        let set_template = read_template(set, None);
        for rep in children(set, "Representation") {
            let id = rep.attribute("id").unwrap_or("").to_string();
            let mime = rep.attribute("mimeType").or(set_mime).map(str::to_string);
            let codecs = rep
                .attribute("codecs")
                .or(set.attribute("codecs"))
                .map(str::to_string);
            let kind = match (set_type, mime.as_deref()) {
                (Some("video"), _) => TrackKind::Video,
                (Some("audio"), _) => TrackKind::Audio,
                (Some("text"), _) => TrackKind::Text,
                (_, Some(m)) if m.starts_with("video/") => TrackKind::Video,
                (_, Some(m)) if m.starts_with("audio/") => TrackKind::Audio,
                (_, Some(m))
                    if m.starts_with("text/") || m.contains("ttml") || m.contains("vtt") =>
                {
                    TrackKind::Text
                }
                _ => TrackKind::Other,
            };
            let bandwidth = rep.attribute("bandwidth").and_then(|v| v.parse().ok());
            let width = rep
                .attribute("width")
                .or(set.attribute("width"))
                .and_then(|v| v.parse().ok());
            let height = rep
                .attribute("height")
                .or(set.attribute("height"))
                .and_then(|v| v.parse().ok());
            let frame_rate = rep
                .attribute("frameRate")
                .or(set.attribute("frameRate"))
                .and_then(|v| match v.split_once('/') {
                    Some((a, b)) => Some(a.parse::<f32>().ok()? / b.parse::<f32>().ok()?),
                    None => v.parse().ok(),
                });
            let lang = set
                .attribute("lang")
                .or(rep.attribute("lang"))
                .map(str::to_string);
            let base = resolve_base(&set_base, rep);
            if child(rep, "ContentProtection").is_some() {
                drm = true;
            }

            let mut init = None;
            let mut segments = Vec::new();
            if let Some(t) = read_template(rep, set_template.as_ref()) {
                let media = t
                    .media
                    .clone()
                    .ok_or_else(|| DashError::Invalid("SegmentTemplate without media".into()))?;
                if let Some(i) = &t.init {
                    let u = expand_template(i, &id, bandwidth, None, None);
                    init = Some(SegRef {
                        url: base
                            .join(&u)
                            .map_err(|e| DashError::Invalid(e.to_string()))?,
                        range: None,
                    });
                }
                let start = t.start_number.unwrap_or(1);
                let timescale = t.timescale.unwrap_or(1).max(1);
                if let Some(tl) = &t.timeline {
                    let mut time = 0u64;
                    let mut number = start;
                    for (i, (t0, d, r)) in tl.iter().enumerate() {
                        if let Some(t0) = t0 {
                            time = *t0;
                        }
                        let repeats = if *r >= 0 {
                            *r as u64
                        } else {
                            // r = -1: repeat until the next S@t or the period end.
                            let end = tl
                                .get(i + 1)
                                .and_then(|n| n.0)
                                .map(|x| x as f64 / timescale as f64)
                                .or(period_duration)
                                .unwrap_or(0.0);
                            let remaining =
                                (end * timescale as f64 - time as f64) / (*d).max(1) as f64;
                            remaining.ceil().max(1.0) as u64 - 1
                        };
                        for _ in 0..=repeats {
                            let u =
                                expand_template(&media, &id, bandwidth, Some(number), Some(time));
                            segments.push(SegRef {
                                url: base
                                    .join(&u)
                                    .map_err(|e| DashError::Invalid(e.to_string()))?,
                                range: None,
                            });
                            time += d;
                            number += 1;
                            if segments.len() > MAX_SEGMENTS {
                                return Err(DashError::Invalid("too many segments".into()));
                            }
                        }
                    }
                } else if let Some(d) = t.duration {
                    let total = period_duration.ok_or_else(|| {
                        DashError::Invalid("unknown presentation duration".into())
                    })?;
                    let seg_secs = d as f64 / timescale as f64;
                    let count = (total / seg_secs).ceil() as u64;
                    if count as usize > MAX_SEGMENTS {
                        return Err(DashError::Invalid("too many segments".into()));
                    }
                    for k in 0..count {
                        let n = start + k;
                        let u = expand_template(&media, &id, bandwidth, Some(n), Some(k * d));
                        segments.push(SegRef {
                            url: base
                                .join(&u)
                                .map_err(|e| DashError::Invalid(e.to_string()))?,
                            range: None,
                        });
                    }
                } else {
                    return Err(DashError::Invalid(
                        "SegmentTemplate without duration or timeline".into(),
                    ));
                }
            } else if let Some(list) =
                child(rep, "SegmentList").or_else(|| child(set, "SegmentList"))
            {
                if let Some(i) = child(list, "Initialization") {
                    let u = i
                        .attribute("sourceURL")
                        .map(|s| base.join(s))
                        .transpose()
                        .map_err(|e| DashError::Invalid(e.to_string()))?;
                    init = Some(SegRef {
                        url: u.unwrap_or_else(|| base.clone()),
                        range: i.attribute("range").and_then(parse_range),
                    });
                }
                for s in children(list, "SegmentURL") {
                    let url = match s.attribute("media") {
                        Some(m) => base
                            .join(m)
                            .map_err(|e| DashError::Invalid(e.to_string()))?,
                        None => base.clone(),
                    };
                    segments.push(SegRef {
                        url,
                        range: s.attribute("mediaRange").and_then(parse_range),
                    });
                }
            } else {
                // Single file (SegmentBase or plain BaseURL): download it whole.
                segments.push(SegRef {
                    url: base.clone(),
                    range: None,
                });
            }
            reps.push(Representation {
                id,
                kind,
                mime,
                codecs,
                bandwidth,
                width,
                height,
                frame_rate,
                lang,
                init,
                segments,
            });
        }
    }
    if reps.is_empty() {
        return Err(DashError::Invalid("no representations".into()));
    }
    Ok(Manifest {
        is_live,
        drm,
        duration,
        representations: reps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H2M3.5S"), Some(3723.5));
        assert_eq!(parse_duration("PT0.96S"), Some(0.96));
        assert_eq!(parse_duration("P1DT2S"), Some(86402.0));
        assert_eq!(parse_duration("1H"), None);
    }

    #[test]
    fn templates() {
        assert_eq!(
            expand_template(
                "seg-$RepresentationID$-$Number%05d$.m4s",
                "v1",
                None,
                Some(7),
                None
            ),
            "seg-v1-00007.m4s"
        );
        assert_eq!(
            expand_template(
                "$Bandwidth$/$Time$.m4s",
                "a",
                Some(128000),
                None,
                Some(9000)
            ),
            "128000/9000.m4s"
        );
        assert_eq!(
            expand_template("price$$5", "a", None, None, None),
            "price$5"
        );
        assert_eq!(
            expand_template("x$Unknown$y", "a", None, None, None),
            "x$Unknown$y"
        );
    }

    const MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S">
  <BaseURL>media/</BaseURL>
  <Period>
    <AdaptationSet contentType="video" mimeType="video/mp4">
      <SegmentTemplate timescale="1000" initialization="init-$RepresentationID$.mp4" media="chunk-$RepresentationID$-$Number%03d$.m4s" startNumber="1" duration="4000"/>
      <Representation id="v720" bandwidth="2000000" width="1280" height="720" codecs="avc1.64001f" frameRate="30000/1001"/>
      <Representation id="v360" bandwidth="600000" width="640" height="360" codecs="avc1.4d401e"/>
    </AdaptationSet>
    <AdaptationSet contentType="audio" mimeType="audio/mp4" lang="ar">
      <Representation id="a1" bandwidth="128000" codecs="mp4a.40.2">
        <SegmentTemplate timescale="48000" initialization="a-init.mp4" media="a-$Time$.m4s">
          <SegmentTimeline><S t="0" d="96000" r="2"/><S d="48000"/></SegmentTimeline>
        </SegmentTemplate>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;

    #[test]
    fn manifest_with_templates_and_timeline() {
        let base = Url::parse("https://e.com/v/manifest.mpd").unwrap();
        let m = parse(MPD, &base).unwrap();
        assert!(!m.is_live && !m.drm);
        assert_eq!(m.duration, Some(10.0));
        assert_eq!(m.representations.len(), 3);
        let v = &m.representations[0];
        assert_eq!(v.kind, TrackKind::Video);
        assert_eq!(v.height, Some(720));
        assert!((v.frame_rate.unwrap() - 29.97).abs() < 0.01);
        assert_eq!(
            v.init.as_ref().unwrap().url.as_str(),
            "https://e.com/v/media/init-v720.mp4"
        );
        assert_eq!(v.segments.len(), 3, "10 s / 4 s = 3 segments");
        assert_eq!(
            v.segments[2].url.as_str(),
            "https://e.com/v/media/chunk-v720-003.m4s"
        );
        let a = &m.representations[2];
        assert_eq!(a.kind, TrackKind::Audio);
        assert_eq!(a.lang.as_deref(), Some("ar"));
        let urls: Vec<&str> = a.segments.iter().map(|s| s.url.path()).collect();
        assert_eq!(
            urls,
            vec![
                "/v/media/a-0.m4s",
                "/v/media/a-96000.m4s",
                "/v/media/a-192000.m4s",
                "/v/media/a-288000.m4s"
            ]
        );
    }

    #[test]
    fn content_protection_marks_drm() {
        let mpd = r#"<MPD type="static" mediaPresentationDuration="PT4S"><Period><AdaptationSet mimeType="video/mp4"><ContentProtection schemeIdUri="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"/><Representation id="v" bandwidth="1"><BaseURL>v.mp4</BaseURL></Representation></AdaptationSet></Period></MPD>"#;
        let m = parse(mpd, &Url::parse("https://e.com/m.mpd").unwrap()).unwrap();
        assert!(m.drm);
        assert_eq!(
            m.representations[0].segments[0].url.as_str(),
            "https://e.com/v.mp4"
        );
    }

    #[test]
    fn rejects_non_mpd() {
        assert!(parse("<html/>", &Url::parse("https://e.com").unwrap()).is_err());
        assert!(parse("not xml", &Url::parse("https://e.com").unwrap()).is_err());
    }
}
