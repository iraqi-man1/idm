//! File name sanitization and conflict resolution.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Maximum file name length in bytes (most file systems allow 255).
const MAX_NAME_BYTES: usize = 200;

const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9", "conin$",
    "conout$",
];

/// Turn an untrusted name (from a server, URL or browser) into a safe file
/// name valid on Windows, macOS and Linux.
///
/// * strips directories – a name can never escape the target folder,
/// * removes control and bidirectional-override characters (which can
///   disguise `exe` files as documents),
/// * replaces characters reserved on Windows,
/// * avoids reserved device names (`CON`, `NUL`, ...),
/// * trims trailing dots/spaces and limits the length, keeping the extension.
pub fn sanitize_file_name(raw: &str) -> String {
    // Only the last path component.
    let last = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let mut s: String = last
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_control(*c) && *c != '\u{FEFF}')
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c => c,
        })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).trim().to_string();
    while s.starts_with("..") {
        s.remove(0);
    }
    if s.is_empty() || s == "." {
        return "download".to_string();
    }
    let (stem, _) = split_ext(&s);
    if WINDOWS_RESERVED.contains(&stem.to_ascii_lowercase().as_str()) {
        s = format!("_{s}");
    }
    truncate_name(&s, MAX_NAME_BYTES)
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{061C}')
}

/// Split `name.ext` into (`name`, `Some("ext")`). Dot files have no extension.
pub fn split_ext(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], Some(&name[i + 1..])),
        _ => (name, None),
    }
}

fn truncate_name(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name);
    let ext_part = ext.map(|e| format!(".{e}")).unwrap_or_default();
    let ext_part = if ext_part.len() > 32 {
        String::new()
    } else {
        ext_part
    };
    let budget = max.saturating_sub(ext_part.len());
    let mut cut = budget.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{}", stem[..cut].trim_end(), ext_part)
}

/// Ensure the name has an extension, adding one guessed from the MIME type.
pub fn ensure_extension(name: &str, mime: Option<&str>) -> String {
    if split_ext(name).1.is_some() {
        return name.to_string();
    }
    match mime.and_then(velox_http::headers::extension_for_mime) {
        Some(ext) => format!("{name}.{ext}"),
        None => name.to_string(),
    }
}

/// Pick a free path in `dir` for `name`: `name.ext`, `name (1).ext`, ...
/// Paths in `reserved` (targets of other downloads) are treated as taken.
pub fn unique_path(dir: &Path, name: &str, reserved: &HashSet<PathBuf>) -> PathBuf {
    let taken = |p: &Path| p.exists() || reserved.contains(p) || partial_path(p).exists();
    let first = dir.join(name);
    if !taken(&first) {
        return first;
    }
    let (stem, ext) = split_ext(name);
    for i in 1..10_000 {
        let candidate = match ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let p = dir.join(candidate);
        if !taken(&p) {
            return p;
        }
    }
    dir.join(format!("{stem}-{}", uuid::Uuid::new_v4()))
}

/// Suffix of partial files.
pub const PARTIAL_SUFFIX: &str = ".vdpart";

/// Path of the partial file for a final path.
pub fn partial_path(final_path: &Path) -> PathBuf {
    let mut s = final_path.as_os_str().to_os_string();
    s.push(PARTIAL_SUFFIX);
    PathBuf::from(s)
}

/// The OS default download directory.
pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_hostile_names() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("..\\..\\boot.ini"), "boot.ini");
        assert_eq!(
            sanitize_file_name("a<b>c:d\"e|f?g*h.txt"),
            "a_b_c_d_e_f_g_h.txt"
        );
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_file_name("name. . "), "name");
        assert_eq!(sanitize_file_name(""), "download");
        assert_eq!(sanitize_file_name(".."), "download");
        assert_eq!(sanitize_file_name("..hidden"), ".hidden");
        assert_eq!(
            sanitize_file_name("invoice\u{202E}fdp.exe"),
            "invoicefdp.exe"
        );
        assert_eq!(sanitize_file_name("tab\tname\n.zip"), "tabname.zip");
        assert_eq!(sanitize_file_name("تقرير سنوي.pdf"), "تقرير سنوي.pdf");
        assert_eq!(sanitize_file_name(".bashrc"), ".bashrc");
    }

    #[test]
    fn truncates_keeping_extension_and_utf8() {
        let long = format!("{}.mkv", "ü".repeat(300));
        let s = sanitize_file_name(&long);
        assert!(s.len() <= MAX_NAME_BYTES);
        assert!(s.ends_with(".mkv"));
    }

    #[test]
    fn extensions() {
        assert_eq!(ensure_extension("video", Some("video/mp4")), "video.mp4");
        assert_eq!(ensure_extension("file.bin", Some("video/mp4")), "file.bin");
        assert_eq!(
            ensure_extension("blob", Some("application/octet-stream")),
            "blob"
        );
        assert_eq!(split_ext(".bashrc"), (".bashrc", None));
        assert_eq!(split_ext("a.tar.gz"), ("a.tar", Some("gz")));
    }

    #[test]
    fn unique_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.zip"), b"x").unwrap();
        std::fs::write(dir.path().join("f (1).zip.vdpart"), b"x").unwrap();
        let mut reserved = HashSet::new();
        reserved.insert(dir.path().join("f (2).zip"));
        assert_eq!(
            unique_path(dir.path(), "f.zip", &reserved),
            dir.path().join("f (3).zip")
        );
        assert_eq!(
            unique_path(dir.path(), "g.zip", &reserved),
            dir.path().join("g.zip")
        );
        std::fs::write(dir.path().join("noext"), b"x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "noext", &reserved),
            dir.path().join("noext (1)")
        );
    }
}
