//! Sound anchors: fixes that learn how your words sound (Mac and iPhone
//! `SoundContrast`, `SoundAnchors`, `SoundAnchorJudge`, `SoundAnchorGuesses`
//! and `SlowListen`; see `docs/sound-anchors.md`).
//!
//! You say "emailer" (Eimailar) and the speech model writes "email". A text
//! pair ("email" -> Eimailar) would rewrite every "email" you ever say, and
//! Word Boost never touches a single ordinary word. Here the recording
//! decides: said "email", it stays "email"; said your word, it becomes your
//! word.

pub mod anchors;
pub mod contrast;
pub mod guesses;
pub mod judge;
pub mod slow_listen;

pub use anchors::{SoundAnchor, SoundAnchorBook};
pub use contrast::SoundWord;
pub use judge::Decision;
