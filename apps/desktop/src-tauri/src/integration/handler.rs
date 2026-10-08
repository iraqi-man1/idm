//! Answers requests coming from the browser extension.

use tauri::{AppHandle, Emitter, Manager};
use velox_nm::ipc::{BridgeHandler, HandlerFuture, PeerInfo};
use velox_types::protocol::{BrowserDownload, CaptureOrigin, ExtMessage, ExtReply, ExtRequest};
use velox_types::{
    AddDownloadRequest, AddSource, DuplicatePolicy, ExtensionSeen, HeaderPair, StartMode,
    MIN_SUPPORTED_PROTOCOL_VERSION, PROTOCOL_VERSION,
};

use super::{browser_config, capture, Bridge};
use crate::security::redact_url;
use crate::AppState;

pub struct AppBridge {
    app: AppHandle,
}

impl AppBridge {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }
}

/// Host part of a URL, lower-case.
fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(|h| h.to_ascii_lowercase())
}

/// `example.com` excludes `example.com` and `*.example.com`.
pub fn site_excluded(excluded: &[String], urls: &[Option<&str>]) -> bool {
    urls.iter()
        .flatten()
        .filter_map(|u| host_of(u))
        .any(|host| {
            excluded.iter().any(|pat| {
                let pat = pat
                    .trim()
                    .trim_start_matches("*.")
                    .trim_start_matches('.')
                    .to_ascii_lowercase();
                !pat.is_empty() && (host == pat || host.ends_with(&format!(".{pat}")))
            })
        })
}

fn to_request(d: &BrowserDownload) -> AddDownloadRequest {
    AddDownloadRequest {
        url: d.url.clone(),
        file_name: d.file_name.clone().filter(|n| !n.trim().is_empty()),
        referer: d.referrer.clone().filter(|r| !r.is_empty()),
        user_agent: d.user_agent.clone(),
        headers: d
            .headers
            .iter()
            .map(|h| HeaderPair {
                name: h.name.clone(),
                value: h.value.clone(),
            })
            .collect(),
        cookies: d.cookies.clone().filter(|c| !c.is_empty()),
        start: StartMode::Now,
        source: match d.origin {
            CaptureOrigin::Capture => AddSource::Browser,
            CaptureOrigin::ContextMenu => AddSource::ContextMenu,
            CaptureOrigin::Batch => AddSource::Batch,
            CaptureOrigin::Video => AddSource::VideoButton,
        },
        expected_size: d.file_size,
        mime: d.mime.clone(),
        page_url: d.page_url.clone(),
        ..Default::default()
    }
}

impl AppBridge {
    async fn add_from_browser(&self, d: BrowserDownload) -> ExtReply {
        let state = self.state();
        let s = state.manager.settings();
        if d.origin == CaptureOrigin::Capture {
            if !s.browser.capture_downloads {
                return ExtReply::Added {
                    accepted: false,
                    download_id: None,
                    pending: false,
                    reason: Some("capture disabled".into()),
                };
            }
            if site_excluded(
                &s.browser.excluded_sites,
                &[Some(&d.url), d.page_url.as_deref(), d.referrer.as_deref()],
            ) {
                return ExtReply::Added {
                    accepted: false,
                    download_id: None,
                    pending: false,
                    reason: Some("site excluded".into()),
                };
            }
        }
        let duplicate = state.manager.find_duplicates(&d.url).first().copied();
        if duplicate.is_some() && s.general.duplicate_policy == DuplicatePolicy::Skip {
            crate::show_main_window(&self.app);
            return ExtReply::Added {
                accepted: true,
                download_id: duplicate,
                pending: false,
                reason: Some("duplicate".into()),
            };
        }
        let req = to_request(&d);
        tracing::info!(url = %redact_url(&d.url), origin = ?d.origin, "download received from browser");
        let ask = d.interactive
            && (s.general.show_add_dialog_for_browser
                || (duplicate.is_some() && s.general.duplicate_policy == DuplicatePolicy::Ask));
        if ask {
            match capture::open(&self.app, req, duplicate) {
                Ok(()) => ExtReply::Added {
                    accepted: true,
                    download_id: None,
                    pending: true,
                    reason: None,
                },
                Err(e) => ExtReply::Error {
                    code: "app".into(),
                    message: e.to_string(),
                },
            }
        } else {
            match state.manager.add(req).await {
                Ok(info) => ExtReply::Added {
                    accepted: true,
                    download_id: Some(info.id),
                    pending: false,
                    reason: None,
                },
                Err(e) => ExtReply::Error {
                    code: e.kind().as_str().into(),
                    message: e.to_string(),
                },
            }
        }
    }
}

impl BridgeHandler for AppBridge {
    fn handle<'a>(&'a self, peer: &'a PeerInfo, req: ExtRequest) -> HandlerFuture<'a> {
        Box::pin(async move {
            match req.message {
                ExtMessage::Hello {
                    extension_version,
                    browser,
                    protocol_version,
                } => {
                    let compatible = (MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION)
                        .contains(&protocol_version);
                    let message = if compatible {
                        None
                    } else if protocol_version < MIN_SUPPORTED_PROTOCOL_VERSION {
                        Some("Please update the Velox browser extension.".to_string())
                    } else {
                        Some("Please update Velox Download Manager.".to_string())
                    };
                    let seen = ExtensionSeen {
                        browser,
                        version: extension_version,
                        origin: peer.origin.clone(),
                        last_seen: velox_types::now_ms(),
                        compatible,
                    };
                    let _ = self
                        .state()
                        .manager
                        .db()
                        .put_setting("browser_extension", &seen);
                    *self.app.state::<Bridge>().last_extension.lock() = Some(seen);
                    let _ = self.app.emit("app://browser-status-changed", ());
                    ExtReply::Hello {
                        app_version: self.app.package_info().version.to_string(),
                        protocol_version: PROTOCOL_VERSION,
                        compatible,
                        message,
                        config: browser_config(&self.state().manager.settings()),
                    }
                }
                ExtMessage::Ping => ExtReply::Pong,
                ExtMessage::GetConfig => ExtReply::Config {
                    config: browser_config(&self.state().manager.settings()),
                },
                ExtMessage::AddDownload { download } => self.add_from_browser(download).await,
                ExtMessage::AddBatch { items, page_url } => {
                    let urls: Vec<String> = items.iter().map(|i| i.url.clone()).collect();
                    let count = urls.len() as u32;
                    crate::show_main_window(&self.app);
                    let _ = self.app.emit(
                        "app://batch-urls",
                        serde_json::json!({ "urls": urls, "referer": page_url }),
                    );
                    ExtReply::BatchAdded { count }
                }
                ExtMessage::ProbeMedia { .. } | ExtMessage::DownloadMedia { .. } => {
                    crate::media_bridge::handle(&self.app, req.message).await
                }
                ExtMessage::ShowApp => {
                    crate::show_main_window(&self.app);
                    ExtReply::Ok
                }
            }
        })
    }

    fn connected(&self, peer: &PeerInfo) {
        tracing::info!(browser = %peer.browser, origin = %peer.origin, "browser extension connected");
        let _ = self.app.emit("app://browser-status-changed", ());
    }

    fn disconnected(&self, peer: &PeerInfo) {
        tracing::info!(browser = %peer.browser, "browser extension disconnected");
        let _ = self.app.emit("app://browser-status-changed", ());
    }
}

#[cfg(test)]
mod tests {
    use super::site_excluded;

    #[test]
    fn exclusions_match_hosts_and_subdomains() {
        let ex = vec!["example.com".to_string(), "*.bank.test".to_string()];
        assert!(site_excluded(&ex, &[Some("https://example.com/a.zip")]));
        assert!(site_excluded(&ex, &[Some("https://cdn.example.com/a.zip")]));
        assert!(site_excluded(&ex, &[None, Some("https://my.bank.test/")]));
        assert!(!site_excluded(&ex, &[Some("https://notexample.com/a.zip")]));
        assert!(!site_excluded(&[], &[Some("https://example.com/")]));
    }
}
