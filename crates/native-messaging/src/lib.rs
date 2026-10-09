//! Browser integration plumbing for Velox Download Manager.
//!
//! * [`framing`] – length-prefixed JSON for native messaging and IPC.
//! * [`ipc`] – authenticated local channel between the host and the app.
//! * [`manifest`] – host manifests, registration and browser detection.
//!
//! The `velox-nmh` binary (src/bin) is the native messaging host.

pub mod framing;
pub mod ipc;
pub mod manifest;
