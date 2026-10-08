//! Application settings. Every struct uses `#[serde(default)]` so settings
//! saved by older versions load cleanly after an upgrade.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::download::ConflictPolicy;
use crate::media::OutputContainer;

pub const DEFAULT_USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ",
    "(KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DuplicatePolicy {
    /// Ask the user what to do.
    #[default]
    Ask,
    /// Always add a new download.
    Allow,
    /// Ignore the request (focus the existing download).
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProxyMode {
    /// Use the operating system / environment proxy configuration.
    #[default]
    System,
    /// Connect directly.
    None,
    Http,
    Socks5,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct ProxySettings {
    pub mode: ProxyMode,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Indicates a password is stored in the secret store. The password
    /// itself is never part of the settings document.
    pub has_password: bool,
    /// Hosts that bypass the proxy (comma separated patterns).
    pub bypass: String,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            mode: ProxyMode::System,
            host: String::new(),
            port: 8080,
            username: String::new(),
            has_password: false,
            bypass: "localhost,127.0.0.1".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct GeneralSettings {
    /// Default download directory (empty = OS "Downloads" folder).
    pub download_dir: String,
    /// Save files into per-category sub folders (Video, Music, ...).
    pub category_folders: bool,
    pub launch_at_startup: bool,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub notify_on_complete: bool,
    pub notify_on_error: bool,
    /// Watch the clipboard for downloadable links (opt-in).
    pub clipboard_monitor: bool,
    pub conflict_policy: ConflictPolicy,
    pub duplicate_policy: DuplicatePolicy,
    /// Show the "download file info" dialog for downloads sent by the browser.
    pub show_add_dialog_for_browser: bool,
    /// Open the progress window when a download starts.
    pub show_progress_window: bool,
    /// Show the completion dialog when a download finishes.
    pub show_complete_dialog: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            download_dir: String::new(),
            category_folders: false,
            launch_at_startup: false,
            start_minimized: false,
            minimize_to_tray: true,
            close_to_tray: true,
            notify_on_complete: true,
            notify_on_error: true,
            clipboard_monitor: false,
            conflict_policy: ConflictPolicy::Rename,
            duplicate_policy: DuplicatePolicy::Ask,
            show_add_dialog_for_browser: true,
            show_progress_window: false,
            show_complete_dialog: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct DownloadSettings {
    /// Maximum downloads running simultaneously across all queues.
    pub max_concurrent: u32,
    /// Default maximum connections per download (1..=32).
    pub connections_per_download: u8,
    /// Adapt the connection count to measured throughput and server limits.
    pub adaptive_connections: bool,
    /// Do not split segments smaller than this (bytes).
    #[ts(type = "number")]
    pub min_segment_size: u64,
    /// Automatic retries before a download is marked failed.
    pub max_retries: u32,
    /// Base delay between retries, seconds (grows exponentially).
    pub retry_delay_secs: u32,
    /// Global speed limit, bytes/second (0 = unlimited).
    #[ts(type = "number")]
    pub speed_limit: u64,
    /// Directory for partial files (empty = next to the final file).
    pub temp_dir: String,
    /// Resume downloads that were running when the app last exited.
    pub resume_on_startup: bool,
    /// Pre-allocate the full file size when the size is known.
    pub preallocate: bool,
}

impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            connections_per_download: 8,
            adaptive_connections: true,
            min_segment_size: 1024 * 1024,
            max_retries: 20,
            retry_delay_secs: 3,
            speed_limit: 0,
            temp_dir: String::new(),
            resume_on_startup: true,
            preallocate: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct NetworkSettings {
    pub proxy: ProxySettings,
    pub connect_timeout_secs: u32,
    /// Abort a connection that receives no data for this long.
    pub read_timeout_secs: u32,
    pub user_agent: String,
    pub max_redirects: u32,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            proxy: ProxySettings::default(),
            connect_timeout_secs: 30,
            read_timeout_secs: 60,
            user_agent: DEFAULT_USER_AGENT.to_string(),
            max_redirects: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct BrowserSettings {
    /// Take over browser downloads matching `capture_extensions`.
    pub capture_downloads: bool,
    /// File extensions captured from the browser.
    pub capture_extensions: Vec<String>,
    /// Do not capture files smaller than this (bytes, 0 = no limit).
    #[ts(type = "number")]
    pub min_capture_size: u64,
    /// Sites (host patterns) never captured.
    pub excluded_sites: Vec<String>,
    /// Detect video/audio resources on web pages.
    pub video_detection: bool,
    /// Show the floating "Download This Video" button.
    pub floating_button: bool,
    /// Extra extension IDs allowed to talk to the native host (store builds).
    pub extra_allowed_extension_ids: Vec<String>,
}

pub const DEFAULT_CAPTURE_EXTENSIONS: &[&str] = &[
    "3gp", "7z", "aac", "ace", "aif", "apk", "arj", "asf", "avi", "bin", "bz2", "dmg", "deb", "exe",
    "flac", "flv", "gz", "gzip", "img", "iso", "lzh", "m4a", "m4v", "mkv", "mov", "mp3", "mp4",
    "mpeg", "mpg", "msi", "msix", "ogg", "ogv", "opus", "pdf", "pkg", "rar", "rpm", "tar", "tgz",
    "wav", "webm", "wma", "wmv", "xz", "zip", "zst", "appimage",
];

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            capture_downloads: true,
            capture_extensions: DEFAULT_CAPTURE_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
            min_capture_size: 0,
            excluded_sites: Vec::new(),
            video_detection: true,
            floating_button: true,
            extra_allowed_extension_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum QualityPreference {
    #[default]
    Best,
    P2160,
    P1440,
    P1080,
    P720,
    P480,
    P360,
    AudioOnly,
}

impl QualityPreference {
    pub fn max_height(self) -> Option<u32> {
        match self {
            Self::Best | Self::AudioOnly => None,
            Self::P2160 => Some(2160),
            Self::P1440 => Some(1440),
            Self::P1080 => Some(1080),
            Self::P720 => Some(720),
            Self::P480 => Some(480),
            Self::P360 => Some(360),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct MediaSettings {
    pub preferred_quality: QualityPreference,
    pub output_container: OutputContainer,
    pub download_subtitles: bool,
    pub subtitle_languages: Vec<String>,
    pub embed_subtitles: bool,
    /// Parallel segment downloads for HLS/DASH.
    pub segment_concurrency: u8,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            preferred_quality: QualityPreference::Best,
            output_container: OutputContainer::Mp4,
            download_subtitles: false,
            subtitle_languages: vec!["en".into()],
            embed_subtitles: true,
            segment_concurrency: 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct PowerSettings {
    /// Pause the queue while running on battery below `battery_threshold`.
    pub pause_on_low_battery: bool,
    pub battery_threshold: u8,
    /// Do not start queued downloads on metered connections (Windows only).
    pub pause_on_metered: bool,
}

impl Default for PowerSettings {
    fn default() -> Self {
        Self { pause_on_low_battery: false, battery_threshold: 20, pause_on_metered: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct AppearanceSettings {
    pub theme: ThemeMode,
    /// "system", "en" or "ar".
    pub language: String,
    /// Columns visible in the download table.
    pub visible_columns: Vec<String>,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            language: "system".into(),
            visible_columns: [
                "name", "size", "progress", "speed", "eta", "status", "date_added",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct UpdateSettings {
    pub auto_check: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { auto_check: true }
    }
}

/// The complete settings document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct AppSettings {
    pub general: GeneralSettings,
    pub downloads: DownloadSettings,
    pub network: NetworkSettings,
    pub browser: BrowserSettings,
    pub media: MediaSettings,
    pub power: PowerSettings,
    pub appearance: AppearanceSettings,
    pub updates: UpdateSettings,
    /// The first-run onboarding was completed.
    pub onboarding_done: bool,
}

impl AppSettings {
    /// Clamp values into supported ranges.
    pub fn normalized(mut self) -> Self {
        let d = &mut self.downloads;
        d.max_concurrent = d.max_concurrent.clamp(1, 32);
        d.connections_per_download = d.connections_per_download.clamp(1, 32);
        d.min_segment_size = d.min_segment_size.clamp(64 * 1024, 64 * 1024 * 1024);
        d.max_retries = d.max_retries.min(1000);
        d.retry_delay_secs = d.retry_delay_secs.clamp(1, 600);
        let n = &mut self.network;
        n.connect_timeout_secs = n.connect_timeout_secs.clamp(3, 300);
        n.read_timeout_secs = n.read_timeout_secs.clamp(5, 600);
        n.max_redirects = n.max_redirects.clamp(0, 30);
        if n.user_agent.trim().is_empty() {
            n.user_agent = DEFAULT_USER_AGENT.to_string();
        }
        self.media.segment_concurrency = self.media.segment_concurrency.clamp(1, 16);
        self.power.battery_threshold = self.power.battery_threshold.clamp(5, 95);
        for ext in &mut self.browser.capture_extensions {
            *ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        }
        self.browser.capture_extensions.retain(|e| !e.is_empty());
        self.browser.capture_extensions.sort();
        self.browser.capture_extensions.dedup();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_documents_load_with_defaults() {
        let s: AppSettings = serde_json::from_str(r#"{"downloads":{"max_concurrent":2}}"#).unwrap();
        assert_eq!(s.downloads.max_concurrent, 2);
        assert_eq!(s.downloads.connections_per_download, 8);
        assert!(s.browser.capture_downloads);
    }

    #[test]
    fn normalize_clamps() {
        let mut s = AppSettings::default();
        s.downloads.connections_per_download = 200;
        s.browser.capture_extensions = vec![".ZIP".into(), "zip".into(), " ".into()];
        let s = s.normalized();
        assert_eq!(s.downloads.connections_per_download, 32);
        assert_eq!(s.browser.capture_extensions, vec!["zip".to_string()]);
    }
}
