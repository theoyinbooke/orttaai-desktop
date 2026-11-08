//! Coach: see how you talk, practise one skill at a time, and prove a habit
//! stuck. A faithful port of the Mac's `CoachCore` package and its app-side
//! `Core/Coach` (center, models, practice, scenes, month report), with the
//! same counting rules, scoring math and thresholds.
//!
//! - Counting ([`counting`], [`measure`]) reads the transcript before Polish.
//! - Ways of talking ([`mode`]) come from the app dictated into, with the
//!   person's per-app overrides (`Settings::coach_app_overrides`).
//! - The voice shape ([`shape`], [`axis`]), habits and proof ([`habit`]),
//!   practice ([`skill`], [`scoring`], [`practice`]), coaches ([`persona`]),
//!   scenes ([`scene`]), the judge check ([`calibration`]) and the month
//!   report ([`month`]) are pure and unit tested.
//! - [`center`] ties them to the store and the local model server, through
//!   [`crate::llm::LlmClient`] only.
//!
//! Integration points for the desktop shell and the engine:
//! - `persist.rs` calls [`after_save`] after every saved dictation.
//! - The engine calls [`take_practice`] after transcribing and before typing
//!   (see [`capture`]); when it returns `true` the dictation was a practice rep.
//! - The shell registers [`set_llm_factory`] (the AI area's client),
//!   [`set_change_listener`] and [`set_capture_listener`].

pub mod axis;
pub mod calendar;
pub mod calibration;
pub mod capture;
pub mod center;
pub mod counting;
pub mod habit;
pub mod measure;
pub mod mode;
pub mod models;
pub mod month;
pub mod persona;
pub mod practice;
pub mod records;
pub mod scene;
pub mod scoring;
pub mod shape;
pub mod skill;
pub mod snapshot;

pub use capture::{
    arm, arm_discard, disarm, practice_armed, practice_wants_polish, set_capture_listener,
    take_practice, PracticeCapture,
};
pub use center::{
    after_save, llm_client, set_change_listener, set_judge_check_listener, set_llm_factory,
    set_session_active,
};
