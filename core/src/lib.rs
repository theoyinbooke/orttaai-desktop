//! `orttaai-core` — the write-once cross-platform engine for Orttaai on Linux & Windows.
//!
//! The crate has **no GUI dependencies**. Every OS-specific behavior (audio capture,
//! text injection, global hotkeys) lives behind a trait, so a change on one platform
//! never forces a rewrite elsewhere. The UI (`app/`, Tauri) and the headless `cli/`
//! are thin consumers of the types and traits defined here.
//!
//! See `docs/architecture.md` for the full picture.

pub mod analytics;
pub mod apps;
pub mod audio;
pub mod chat;
pub mod clipboard;
pub mod coach;
pub mod coordinator;
pub mod edit;
pub mod error;
pub mod focus;
pub mod gesture;
pub mod hardware;
pub mod hotkey;
pub mod injection;
pub mod live;
pub mod llm;
pub mod memory;
pub mod milestones;
#[cfg(feature = "models")]
pub mod models;
pub mod polish;
pub mod semantic;
pub mod settings;
pub mod store;
#[cfg(feature = "link")]
pub mod sync;
pub mod text;
pub mod transcription;
pub mod types;
pub mod vad;
pub mod whatsnew;
pub mod wordboost;
pub mod writing;

pub use coordinator::{DictationCoordinator, DictationOutcome};
pub use error::{CoreError, Result};
pub use types::*;
