//! Media requests from the browser extension (probe formats, download a
//! selected rendition). Implemented with the media engine.

use tauri::AppHandle;
use velox_types::protocol::{ExtMessage, ExtReply};

pub async fn handle(_app: &AppHandle, _msg: ExtMessage) -> ExtReply {
    ExtReply::Error {
        code: "unsupported".into(),
        message: "media downloads are not available in this build".into(),
    }
}
