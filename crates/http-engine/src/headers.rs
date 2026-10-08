//! Parsing helpers for HTTP headers relevant to downloads.

use percent_encoding::percent_decode;

/// Parsed `Content-Range` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    /// Inclusive byte range, `None` for `bytes */total`.
    pub range: Option<(u64, u64)>,
    /// Complete length, `None` when the server sent `*`.
    pub total: Option<u64>,
}

/// Parse `bytes 0-499/1234`, `bytes 0-499/*` or `bytes */1234`.
pub fn parse_content_range(v: &str) -> Option<ContentRange> {
    let v = v.trim();
    let rest = v.strip_prefix("bytes")?.trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
    let (range_part, total_part) = rest.split_once('/')?;
    let total = match total_part.trim() {
        "*" => None,
        t => Some(t.parse::<u64>().ok()?),
    };
    let range = match range_part.trim() {
        "*" => None,
        r => {
            let (a, b) = r.split_once('-')?;
            let a: u64 = a.trim().parse().ok()?;
            let b: u64 = b.trim().parse().ok()?;
            if a > b {
                return None;
            }
            if let Some(t) = total {
                if b >= t {
                    return None;
                }
            }
            Some((a, b))
        }
    };
    if range.is_none() && total.is_none() {
        return None;
    }
    Some(ContentRange { range, total })
}

/// Extract the file name from a `Content-Disposition` header value.
///
/// Supports RFC 6266 / RFC 5987 (`filename*=UTF-8''...`), quoted and bare
/// `filename=` parameters, and the common non-standard practice of sending
/// raw UTF-8 bytes. The result is *not* sanitized for the file system.
pub fn filename_from_content_disposition(raw: &[u8]) -> Option<String> {
    let value = String::from_utf8_lossy(raw);
    let mut plain: Option<String> = None;
    let mut extended: Option<String> = None;
    for param in split_params(&value).into_iter().skip(1) {
        let Some((k, v)) = param.split_once('=') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let v = v.trim();
        match key.as_str() {
            "filename*" => {
                if let Some(decoded) = decode_ext_value(v) {
                    extended = Some(decoded);
                }
            }
            "filename" => plain = Some(unquote(v)),
            _ => {}
        }
    }
    extended
        .or(plain)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Split on `;` outside quoted strings.
fn split_params(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for c in v.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => {
                cur.push(c);
                escaped = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            ';' if !in_quotes => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let inner = &v[1..v.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else {
                out.push(c);
            }
        }
        out
    } else {
        v.to_string()
    }
}

/// Decode an RFC 5987 ext-value: `charset'lang'pct-encoded`.
fn decode_ext_value(v: &str) -> Option<String> {
    let v = unquote(v);
    let mut parts = v.splitn(3, '\'');
    let charset = parts.next()?.trim().to_ascii_lowercase();
    let _lang = parts.next()?;
    let encoded = parts.next()?;
    let bytes: Vec<u8> = percent_decode(encoded.as_bytes()).collect();
    match charset.as_str() {
        "utf-8" | "utf8" | "" => String::from_utf8(bytes).ok(),
        "iso-8859-1" | "latin1" | "us-ascii" => Some(bytes.iter().map(|&b| b as char).collect()),
        _ => None,
    }
}

/// File name derived from the last path segment of a URL.
pub fn filename_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let seg = parsed
        .path_segments()?
        .rev()
        .find(|s| !s.is_empty())?
        .to_string();
    let decoded = percent_decode(seg.as_bytes())
        .decode_utf8_lossy()
        .to_string();
    let decoded = decoded.trim().to_string();
    if decoded.is_empty() || decoded == "." || decoded == ".." {
        None
    } else {
        Some(decoded)
    }
}

/// Preferred file extension for a MIME type (without the dot).
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    let essence = mime.split(';').next()?.trim().to_ascii_lowercase();
    // A few common types whose first guess from mime_guess is unhelpful.
    let fixed = match essence.as_str() {
        "application/octet-stream" | "binary/octet-stream" => return None,
        "text/plain" => Some("txt"),
        "text/html" => Some("html"),
        "image/jpeg" => Some("jpg"),
        "video/mp4" => Some("mp4"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" => Some("m4a"),
        "video/webm" => Some("webm"),
        "audio/webm" => Some("weba"),
        "application/zip" | "application/x-zip-compressed" => Some("zip"),
        "application/x-msdownload" | "application/x-msdos-program" => Some("exe"),
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" => Some("m3u8"),
        "application/dash+xml" => Some("mpd"),
        _ => None,
    };
    if fixed.is_some() {
        return fixed;
    }
    mime_guess::get_mime_extensions_str(&essence).and_then(|exts| exts.first().copied())
}

/// The MIME type indicates an HTML page rather than a file.
pub fn is_html(mime: Option<&str>) -> bool {
    mime.is_some_and(|m| {
        let m = m.to_ascii_lowercase();
        m.starts_with("text/html") || m.starts_with("application/xhtml")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range() {
        assert_eq!(
            parse_content_range("bytes 0-499/1234"),
            Some(ContentRange {
                range: Some((0, 499)),
                total: Some(1234)
            })
        );
        assert_eq!(
            parse_content_range("bytes 10-19/*"),
            Some(ContentRange {
                range: Some((10, 19)),
                total: None
            })
        );
        assert_eq!(
            parse_content_range("bytes */1234"),
            Some(ContentRange {
                range: None,
                total: Some(1234)
            })
        );
        assert_eq!(parse_content_range("bytes 5-4/10"), None);
        assert_eq!(parse_content_range("bytes 0-10/10"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes */*"), None);
    }

    #[test]
    fn content_disposition() {
        let f = |s: &str| filename_from_content_disposition(s.as_bytes());
        assert_eq!(
            f("attachment; filename=\"a b.zip\""),
            Some("a b.zip".into())
        );
        assert_eq!(
            f("attachment; filename=plain.txt"),
            Some("plain.txt".into())
        );
        assert_eq!(
            f("attachment; filename=\"fallback.bin\"; filename*=UTF-8''%D9%85%D9%84%D9%81.pdf"),
            Some("ملف.pdf".into())
        );
        assert_eq!(
            f("attachment; filename*=iso-8859-1'en'%A3%20rates.txt"),
            Some("£ rates.txt".into())
        );
        assert_eq!(
            f("attachment; filename=\"semi;colon.txt\""),
            Some("semi;colon.txt".into())
        );
        assert_eq!(
            f("attachment; filename=\"esc\\\"aped.txt\""),
            Some("esc\"aped.txt".into())
        );
        assert_eq!(f("inline"), None);
        // Raw UTF-8 in filename= (non-standard but common).
        assert_eq!(
            f("attachment; filename=\"файл.zip\""),
            Some("файл.zip".into())
        );
    }

    #[test]
    fn url_names() {
        assert_eq!(
            filename_from_url("https://e.com/dir/file%20name.zip?x=1"),
            Some("file name.zip".into())
        );
        assert_eq!(filename_from_url("https://e.com/dir/"), Some("dir".into()));
        assert_eq!(filename_from_url("https://e.com/"), None);
        assert_eq!(
            filename_from_url("https://e.com/%E6%96%87%E4%BB%B6.7z"),
            Some("文件.7z".into())
        );
    }

    #[test]
    fn mime_extensions() {
        assert_eq!(extension_for_mime("video/mp4"), Some("mp4"));
        assert_eq!(extension_for_mime("application/pdf"), Some("pdf"));
        assert_eq!(extension_for_mime("application/octet-stream"), None);
        assert!(is_html(Some("text/html; charset=utf-8")));
    }
}
