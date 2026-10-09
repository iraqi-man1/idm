//! Clipboard monitoring: a copied link to a downloadable file opens the Add
//! Download dialog (Settings → General → Monitor the clipboard).

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::AppState;

const POLL: Duration = Duration::from_millis(700);

/// A single http(s)/ftp/sftp URL whose file extension is in `extensions`.
pub fn candidate(text: &str, extensions: &[String]) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.len() > 8 * 1024 || t.chars().any(char::is_whitespace) {
        return None;
    }
    let url = url::Url::parse(t).ok()?;
    if !matches!(url.scheme(), "http" | "https" | "ftp" | "sftp") || url.host_str().is_none() {
        return None;
    }
    let name = url.path_segments()?.next_back()?.to_ascii_lowercase();
    let (_, ext) = name.rsplit_once('.')?;
    extensions
        .iter()
        .any(|e| e.eq_ignore_ascii_case(ext))
        .then(|| t.to_string())
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Content already on the clipboard when monitoring starts is ignored.
        let mut last: Option<Option<String>> = None;
        loop {
            tokio::time::sleep(POLL).await;
            let settings = app.state::<AppState>().manager.settings();
            if !settings.general.clipboard_monitor {
                last = None;
                continue;
            }
            let reader = app.clone();
            let text =
                tauri::async_runtime::spawn_blocking(move || reader.clipboard().read_text().ok())
                    .await
                    .ok()
                    .flatten();
            let previous = last.replace(text.clone());
            if previous.is_none() || previous == Some(text.clone()) {
                continue;
            }
            let Some(url) = text
                .as_deref()
                .and_then(|t| candidate(t, &settings.browser.capture_extensions))
            else {
                continue;
            };
            if !app
                .state::<AppState>()
                .manager
                .find_duplicates(&url)
                .is_empty()
            {
                continue;
            }
            tracing::info!(url = %crate::security::redact_url(&url), "download link copied");
            crate::show_main_window(&app);
            let _ = app.emit("app://add-url", url);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::candidate;

    #[test]
    fn only_single_links_to_listed_extensions() {
        let exts = vec!["zip".to_string(), "mp4".to_string()];
        assert_eq!(
            candidate(" https://e.com/a/file.ZIP?x=1 ", &exts).as_deref(),
            Some("https://e.com/a/file.ZIP?x=1")
        );
        assert_eq!(
            candidate("ftp://e.com/v.mp4", &exts).as_deref(),
            Some("ftp://e.com/v.mp4")
        );
        assert!(candidate("https://e.com/page.html", &exts).is_none());
        assert!(candidate("https://e.com/", &exts).is_none());
        assert!(candidate("see https://e.com/a.zip", &exts).is_none());
        assert!(candidate("file:///etc/a.zip", &exts).is_none());
        assert!(candidate("javascript:alert(1)//a.zip", &exts).is_none());
        assert!(candidate("", &exts).is_none());
    }
}
