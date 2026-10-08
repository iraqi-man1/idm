//! Velox download engine.
//!
//! * [`DownloadManager`] – registry and command entry point.
//! * [`task`] – the segmented, resumable, crash-safe file download task.
//! * [`naming`] – file name sanitization and conflict resolution.
//!
//! The engine has no UI dependencies; the desktop app talks to it through
//! [`DownloadManager`] and receives [`velox_types::EngineEvent`]s.

pub mod checksum;
pub mod error;
pub mod fsutil;
pub mod manager;
pub mod naming;
pub mod ratelimit;
pub mod speed;
pub mod task;
pub mod transport;
mod writer;

pub use error::{EngineError, EngineResult};
pub use manager::{detect_kind, DownloadManager, ManagerConfig, MediaRunner};
pub use task::{LiveState, StopReason, TaskEnv, TaskHooks, TaskOutcome, TaskShared};
