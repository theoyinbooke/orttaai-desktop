//! Core error type shared across every subsystem.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    /// A platform backend is not yet implemented on the current target.
    #[error("not implemented on this platform: {0}")]
    NotImplemented(&'static str),

    #[error("no transcription model is loaded")]
    ModelNotLoaded,

    #[error("transcription failed: {0}")]
    Transcription(String),

    #[error("audio capture failed: {0}")]
    Audio(String),

    #[error("text injection failed: {0}")]
    Injection(String),

    #[error("hotkey registration failed: {0}")]
    Hotkey(String),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
