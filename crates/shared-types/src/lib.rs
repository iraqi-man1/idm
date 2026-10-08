//! Shared data types for Velox Download Manager.
//!
//! These types form the contract between the Rust download engine, the Tauri
//! desktop shell (and, through generated TypeScript bindings, the React UI),
//! and the browser extension (through the native messaging protocol).
//!
//! Integer fields that may exceed `u32` are exported to TypeScript as
//! `number`; JavaScript numbers are exact up to 2^53 bytes (8 PiB), which is
//! far beyond any realistic download size.

pub mod download;
pub mod events;
pub mod media;
pub mod protocol;
pub mod queue;
pub mod settings;
pub mod stats;

pub use download::*;
pub use events::*;
pub use media::*;
pub use queue::*;
pub use settings::*;
pub use stats::*;

/// Human readable product name.
pub const APP_NAME: &str = "Velox Download Manager";
/// Short product name used in menus ("Download with Velox").
pub const APP_SHORT_NAME: &str = "Velox";
/// Native messaging host name registered with browsers.
pub const NATIVE_HOST_NAME: &str = "com.veloxdm.host";
/// Version of the extension <-> desktop protocol. Bump on breaking changes.
pub const PROTOCOL_VERSION: u32 = 1;
/// Oldest extension protocol version the desktop app still accepts.
pub const MIN_SUPPORTED_PROTOCOL_VERSION: u32 = 1;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
