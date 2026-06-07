//! `orttaai-core` — the write-once cross-platform engine for Orttaai on Linux & Windows.
//!
//! The crate has **no GUI dependencies**. Every OS-specific behavior (audio capture,
//! text injection, global hotkeys) lives behind a trait, so a change on one platform
//! never forces a rewrite elsewhere. The UI (`app/`, Tauri) and the headless `cli/`
//! are thin consumers of the types and traits defined here.
//!
//! See `docs/architecture.md` for the full picture.

pub mod audio;
pub mod clipboard;
pub mod coordinator;
pub mod error;
pub mod hotkey;
pub mod injection;
pub mod memory;
pub mod settings;
pub mod store;
pub mod transcription;
pub mod types;

pub use coordinator::{DictationCoordinator, DictationOutcome};
pub use error::{CoreError, Result};
pub use types::*;
