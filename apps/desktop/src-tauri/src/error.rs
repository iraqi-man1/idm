use serde::Serialize;
use velox_core::EngineError;

/// Error returned to the UI by every command.
#[derive(Debug, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<EngineError> for CommandError {
    fn from(e: EngineError) -> Self {
        Self {
            code: e.kind().as_str().into(),
            message: e.to_string(),
        }
    }
}

impl From<velox_persistence::DbError> for CommandError {
    fn from(e: velox_persistence::DbError) -> Self {
        Self::new("database", e.to_string())
    }
}

impl From<tauri::Error> for CommandError {
    fn from(e: tauri::Error) -> Self {
        Self::new("app", e.to_string())
    }
}

impl From<uuid::Error> for CommandError {
    fn from(e: uuid::Error) -> Self {
        Self::new("invalid_id", e.to_string())
    }
}

pub type CmdResult<T> = Result<T, CommandError>;
