//! Word Boost (Mac Express): a second model, Parakeet CTC 110M, checks the
//! user's dictionary words against the audio, and a guard decides which of the
//! rescorer's proposed replacements to keep. The rescoring logic is a port of
//! FluidAudio's (Apache-2.0); the model runs on the ONNX Runtime sherpa-onnx
//! links.

pub mod chunks;
pub mod guard;
pub mod rescore;
pub mod tokenizer;

#[cfg(feature = "express")]
pub mod ctc;
#[cfg(feature = "express")]
mod engine;
#[cfg(feature = "express")]
pub mod features;

#[cfg(feature = "express")]
pub use engine::WordBoost;
