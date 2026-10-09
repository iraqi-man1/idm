//! Native messaging host manifests and their registration with browsers.
//!
//! * Windows: the manifest files live in the app data directory and a
//!   registry value under `HKCU\Software\<browser>\NativeMessagingHosts`
//!   points to them (the installer additionally writes HKLM keys for
//!   per-machine installs).
//! * macOS / Linux: a copy of the manifest is placed in each installed
//!   browser's `NativeMessagingHosts` directory.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;
use velox_types::NATIVE_HOST_NAME;

/// Extension ID of the unpacked development build (derived from the public
/// key in `extensions/shared/manifest.base.json`).
pub const CHROMIUM_DEV_EXTENSION_ID: &str = "encnclpojnlecaheiiibdkkgiapnhocl";
/// IDs assigned by the Chrome Web Store / Edge Add-ons. Empty until the
/// extension is published; further IDs can be allowed in the app settings
/// (`browser.extra_allowed_extension_ids`).
pub const CHROMIUM_STORE_EXTENSION_IDS: &[&str] = &[];
/// Gecko ID of the Firefox extension.
pub const FIREFOX_EXTENSION_ID: &str = "velox@veloxdm.app";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Chromium,
    Firefox,
}

#[derive(Debug, Clone, Copy)]
pub struct BrowserTarget {
    pub id: &'static str,
    pub name: &'static str,
    pub family: Family,
    /// Registry key (relative to HKCU/HKLM) on Windows.
    pub win_key: &'static str,
    /// Executable checked in "App Paths" on Windows.
    pub win_exe: &'static str,
    /// Config dir relative to `~/Library/Application Support` (macOS).
    pub mac_dir: &'static str,
    /// Config dir relative to `~` (Linux).
    pub linux_dir: &'static str,
    /// Name of the manifest directory inside the config dir.
    pub manifest_subdir: &'static str,
}

pub const BROWSERS: &[BrowserTarget] = &[
    BrowserTarget {
        id: "chrome",
        name: "Google Chrome",
        family: Family::Chromium,
        win_key: r"Software\Google\Chrome\NativeMessagingHosts",
        win_exe: "chrome.exe",
        mac_dir: "Google/Chrome",
        linux_dir: ".config/google-chrome",
        manifest_subdir: "NativeMessagingHosts",
    },
    BrowserTarget {
        id: "edge",
        name: "Microsoft Edge",
        family: Family::Chromium,
        win_key: r"Software\Microsoft\Edge\NativeMessagingHosts",
        win_exe: "msedge.exe",
        mac_dir: "Microsoft Edge",
        linux_dir: ".config/microsoft-edge",
        manifest_subdir: "NativeMessagingHosts",
    },
    BrowserTarget {
        id: "brave",
        name: "Brave",
        family: Family::Chromium,
        win_key: r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
        win_exe: "brave.exe",
        mac_dir: "BraveSoftware/Brave-Browser",
        linux_dir: ".config/BraveSoftware/Brave-Browser",
        manifest_subdir: "NativeMessagingHosts",
    },
    BrowserTarget {
        id: "chromium",
        name: "Chromium",
        family: Family::Chromium,
        win_key: r"Software\Chromium\NativeMessagingHosts",
        win_exe: "chromium.exe",
        mac_dir: "Chromium",
        linux_dir: ".config/chromium",
        manifest_subdir: "NativeMessagingHosts",
    },
    BrowserTarget {
        id: "vivaldi",
        name: "Vivaldi",
        family: Family::Chromium,
        // Vivaldi and Opera read Chrome's registry key on Windows.
        win_key: r"Software\Google\Chrome\NativeMessagingHosts",
        win_exe: "vivaldi.exe",
        mac_dir: "Vivaldi",
        linux_dir: ".config/vivaldi",
        manifest_subdir: "NativeMessagingHosts",
    },
    BrowserTarget {
        id: "opera",
        name: "Opera",
        family: Family::Chromium,
        win_key: r"Software\Google\Chrome\NativeMessagingHosts",
        win_exe: "opera.exe",
        mac_dir: "com.operasoftware.Opera",
        linux_dir: ".config/opera",
        manifest_subdir: "NativeMessagingHosts",
    },
    BrowserTarget {
        id: "firefox",
        name: "Mozilla Firefox",
        family: Family::Firefox,
        win_key: r"Software\Mozilla\NativeMessagingHosts",
        win_exe: "firefox.exe",
        mac_dir: "Mozilla",
        linux_dir: ".mozilla",
        manifest_subdir: if cfg!(target_os = "macos") {
            "NativeMessagingHosts"
        } else {
            "native-messaging-hosts"
        },
    },
];

/// Chromium `allowed_origins` for the given extension IDs.
pub fn chromium_origins(extra_ids: &[String]) -> Vec<String> {
    let mut ids: Vec<String> = std::iter::once(CHROMIUM_DEV_EXTENSION_ID.to_string())
        .chain(CHROMIUM_STORE_EXTENSION_IDS.iter().map(|s| s.to_string()))
        .chain(extra_ids.iter().filter(|s| is_chromium_id(s)).cloned())
        .collect();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| format!("chrome-extension://{id}/"))
        .collect()
}

/// A Chromium extension id: 32 characters a-p.
pub fn is_chromium_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

pub fn manifest_json(family: Family, host_path: &Path, extra_ids: &[String]) -> serde_json::Value {
    let path = host_path.to_string_lossy().to_string();
    match family {
        Family::Chromium => json!({
            "name": NATIVE_HOST_NAME,
            "description": "Velox Download Manager browser integration",
            "path": path,
            "type": "stdio",
            "allowed_origins": chromium_origins(extra_ids),
        }),
        Family::Firefox => json!({
            "name": NATIVE_HOST_NAME,
            "description": "Velox Download Manager browser integration",
            "path": path,
            "type": "stdio",
            "allowed_extensions": [FIREFOX_EXTENSION_ID],
        }),
    }
}

/// Is `origin` (as passed on the host's command line) one of ours?
pub fn origin_allowed(origin: &str, extra_ids: &[String]) -> bool {
    if origin == FIREFOX_EXTENSION_ID {
        return true;
    }
    chromium_origins(extra_ids).iter().any(|o| o == origin)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserStatus {
    pub id: String,
    pub name: String,
    pub family: Family,
    pub installed: bool,
    pub registered: bool,
    pub manifest_path: Option<String>,
    pub error: Option<String>,
}

fn write_manifest(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value).expect("json"))
}

/// The manifest at `path` points to `host_path`.
fn manifest_points_to(path: &Path, host_path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(PathBuf::from))
        .is_some_and(|p| p == host_path)
}

#[cfg(not(windows))]
fn config_root(b: &BrowserTarget) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support").join(b.mac_dir))
    } else {
        Some(home.join(b.linux_dir))
    }
}

#[cfg(not(windows))]
fn manifest_location(b: &BrowserTarget) -> Option<PathBuf> {
    Some(
        config_root(b)?
            .join(b.manifest_subdir)
            .join(format!("{NATIVE_HOST_NAME}.json")),
    )
}

/// Register the host for every installed browser (current user only).
/// `manifest_dir` is where Windows manifests are stored.
pub fn register_user(
    host_path: &Path,
    manifest_dir: &Path,
    extra_ids: &[String],
) -> Vec<BrowserStatus> {
    let mut out = Vec::new();
    #[cfg(not(windows))]
    {
        let _ = manifest_dir;
        for b in BROWSERS {
            let root = config_root(b);
            let installed = root.as_ref().is_some_and(|r| r.exists());
            let mut st = BrowserStatus {
                id: b.id.into(),
                name: b.name.into(),
                family: b.family,
                installed,
                registered: false,
                manifest_path: None,
                error: None,
            };
            if installed {
                if let Some(path) = manifest_location(b) {
                    match write_manifest(&path, &manifest_json(b.family, host_path, extra_ids)) {
                        Ok(()) => st.registered = true,
                        Err(e) => st.error = Some(e.to_string()),
                    }
                    st.manifest_path = Some(path.to_string_lossy().to_string());
                }
            }
            out.push(st);
        }
    }
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let chromium_path = manifest_dir.join(format!("{NATIVE_HOST_NAME}.chromium.json"));
        let firefox_path = manifest_dir.join(format!("{NATIVE_HOST_NAME}.firefox.json"));
        let w1 = write_manifest(
            &chromium_path,
            &manifest_json(Family::Chromium, host_path, extra_ids),
        );
        let w2 = write_manifest(
            &firefox_path,
            &manifest_json(Family::Firefox, host_path, extra_ids),
        );
        for b in BROWSERS {
            let path = if b.family == Family::Chromium {
                &chromium_path
            } else {
                &firefox_path
            };
            let write_err = if b.family == Family::Chromium {
                w1.as_ref().err()
            } else {
                w2.as_ref().err()
            };
            let mut st = BrowserStatus {
                id: b.id.into(),
                name: b.name.into(),
                family: b.family,
                installed: windows_installed(b),
                registered: false,
                manifest_path: Some(path.to_string_lossy().to_string()),
                error: write_err.map(|e| e.to_string()),
            };
            if st.error.is_none() {
                let key = format!(r"{}\{}", b.win_key, NATIVE_HOST_NAME);
                match hkcu
                    .create_subkey(&key)
                    .and_then(|(k, _)| k.set_value("", &path.to_string_lossy().to_string()))
                {
                    Ok(()) => st.registered = true,
                    Err(e) => st.error = Some(e.to_string()),
                }
            }
            out.push(st);
        }
    }
    out
}

/// Remove the current user's registrations.
pub fn unregister_user(manifest_dir: &Path) {
    #[cfg(not(windows))]
    {
        let _ = manifest_dir;
        for b in BROWSERS {
            if let Some(p) = manifest_location(b) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for b in BROWSERS {
            let _ = hkcu.delete_subkey_all(format!(r"{}\{}", b.win_key, NATIVE_HOST_NAME));
        }
        let _ =
            std::fs::remove_file(manifest_dir.join(format!("{NATIVE_HOST_NAME}.chromium.json")));
        let _ = std::fs::remove_file(manifest_dir.join(format!("{NATIVE_HOST_NAME}.firefox.json")));
    }
}

#[cfg(windows)]
fn windows_installed(b: &BrowserTarget) -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let sub = format!(
        r"Software\Microsoft\Windows\CurrentVersion\App Paths\{}",
        b.win_exe
    );
    [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
        .iter()
        .any(|h| RegKey::predef(*h).open_subkey(&sub).is_ok())
}

/// Report installation / registration state without changing anything.
pub fn status(host_path: &Path) -> Vec<BrowserStatus> {
    let mut out = Vec::new();
    for b in BROWSERS {
        #[cfg(not(windows))]
        {
            let installed = config_root(b).is_some_and(|r| r.exists());
            let path = manifest_location(b);
            let registered = path
                .as_ref()
                .is_some_and(|p| manifest_points_to(p, host_path));
            out.push(BrowserStatus {
                id: b.id.into(),
                name: b.name.into(),
                family: b.family,
                installed,
                registered,
                manifest_path: path.map(|p| p.to_string_lossy().to_string()),
                error: None,
            });
        }
        #[cfg(windows)]
        {
            use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
            use winreg::RegKey;
            let key = format!(r"{}\{}", b.win_key, NATIVE_HOST_NAME);
            let manifest: Option<String> =
                [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
                    .iter()
                    .find_map(|h| {
                        RegKey::predef(*h)
                            .open_subkey(&key)
                            .ok()?
                            .get_value::<String, _>("")
                            .ok()
                    });
            let registered = manifest
                .as_ref()
                .is_some_and(|m| manifest_points_to(Path::new(m), host_path));
            out.push(BrowserStatus {
                id: b.id.into(),
                name: b.name.into(),
                family: b.family,
                installed: windows_installed(b),
                registered,
                manifest_path: manifest,
                error: None,
            });
        }
    }
    out
}

/// Location of the host binary next to the running executable.
pub fn host_binary_next_to_current_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "velox-nmh.exe"
    } else {
        "velox-nmh"
    };
    let p = exe.parent()?.join(name);
    p.exists().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_and_ids() {
        assert!(is_chromium_id(CHROMIUM_DEV_EXTENSION_ID));
        assert!(!is_chromium_id("not-an-id"));
        let o = chromium_origins(&["abcdefghijklmnopabcdefghijklmnop".into(), "bogus".into()]);
        assert_eq!(o.len(), 2);
        assert!(origin_allowed(
            &format!("chrome-extension://{CHROMIUM_DEV_EXTENSION_ID}/"),
            &[]
        ));
        assert!(origin_allowed(FIREFOX_EXTENSION_ID, &[]));
        assert!(!origin_allowed(
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
            &[]
        ));
    }

    #[test]
    fn manifests_have_required_fields() {
        let m = manifest_json(Family::Chromium, Path::new("/opt/velox/velox-nmh"), &[]);
        assert_eq!(m["name"], NATIVE_HOST_NAME);
        assert_eq!(m["type"], "stdio");
        assert_eq!(
            m["allowed_origins"][0],
            format!("chrome-extension://{CHROMIUM_DEV_EXTENSION_ID}/")
        );
        let f = manifest_json(Family::Firefox, Path::new("/opt/velox/velox-nmh"), &[]);
        assert_eq!(f["allowed_extensions"][0], FIREFOX_EXTENSION_ID);
    }

    #[cfg(unix)]
    #[test]
    fn register_and_status_in_fake_home() {
        let home = tempfile::tempdir().unwrap();
        // SAFETY: tests in this module do not run in parallel with other env readers.
        unsafe { std::env::set_var("HOME", home.path()) };
        let chrome_root = if cfg!(target_os = "macos") {
            home.path()
                .join("Library/Application Support/Google/Chrome")
        } else {
            home.path().join(".config/google-chrome")
        };
        std::fs::create_dir_all(&chrome_root).unwrap();
        let host = home.path().join("velox-nmh");
        let st = register_user(&host, home.path(), &[]);
        let chrome = st.iter().find(|s| s.id == "chrome").unwrap();
        assert!(chrome.installed && chrome.registered, "{chrome:?}");
        let edge = st.iter().find(|s| s.id == "edge").unwrap();
        assert!(!edge.installed && !edge.registered);
        let st2 = status(&host);
        assert!(st2.iter().find(|s| s.id == "chrome").unwrap().registered);
        // A different host path is reported as not registered.
        assert!(
            !status(Path::new("/elsewhere"))
                .iter()
                .find(|s| s.id == "chrome")
                .unwrap()
                .registered
        );
        unregister_user(home.path());
        assert!(
            !status(&host)
                .iter()
                .find(|s| s.id == "chrome")
                .unwrap()
                .registered
        );
    }
}
