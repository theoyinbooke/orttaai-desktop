//! Word Boost for Express (Mac `ExpressTranscriber`'s Word Boost half): holds
//! the CTC model, its tokenizer and this dictation's dictionary, scores audio
//! in the Mac's 15 s chunks (reusing chunks scored while the recording was
//! still going), and applies the replacements the rescorer proposes and the
//! guard keeps.

use super::chunks;
use super::ctc::{CtcModel, FRAME_SECONDS};
use super::rescore::{boost, BoostOutcome, PreparedVocabulary, TokenTiming};
use super::tokenizer::CtcTokenizer;
use crate::error::{CoreError, Result};
use crate::text::WordBoostTerm;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, Mutex};

struct Scored {
    range: Range<usize>,
    fingerprint: u64,
    log_probs: Arc<Vec<Vec<f32>>>,
}

pub struct WordBoost {
    model: CtcModel,
    tokenizer: CtcTokenizer,
    vocabulary: Mutex<Option<(Vec<WordBoostTerm>, Arc<PreparedVocabulary>)>>,
    scored: Mutex<HashMap<usize, Scored>>,
}

impl WordBoost {
    /// Load from the Word Boost folder (model.int8.onnx, tokenizer.json).
    pub fn load(dir: &Path, threads: i32) -> Result<Self> {
        let json = std::fs::read_to_string(dir.join("tokenizer.json"))?;
        let tokenizer = CtcTokenizer::from_json(&json)?;
        let model = CtcModel::load(&dir.join("model.int8.onnx"), threads)?;
        Ok(Self {
            model,
            tokenizer,
            vocabulary: Mutex::new(None),
            scored: Mutex::new(HashMap::new()),
        })
    }

    /// The dictionary words to listen for; returns how many can be boosted.
    pub fn set_terms(&self, terms: &[WordBoostTerm]) -> usize {
        let mut slot = self.vocabulary.lock().unwrap();
        if let Some((current, vocab)) = slot.as_ref() {
            if current == terms {
                return vocab.len();
            }
        }
        let vocab = PreparedVocabulary::new(terms, &self.tokenizer);
        let n = vocab.len();
        *slot = (n > 0).then(|| (terms.to_vec(), Arc::new(vocab)));
        n
    }

    pub fn active(&self) -> bool {
        self.vocabulary.lock().unwrap().is_some()
    }

    /// Score every complete 15 s chunk of a recording still in progress.
    pub fn prepare(&self, prefix: &[f32]) {
        for range in chunks::complete_chunks(prefix.len()) {
            if let Err(e) = self.scored(range, prefix) {
                tracing::warn!("Word Boost couldn't score ahead: {e}");
                return;
            }
        }
    }

    /// Forget scored chunks (they belong to one recording).
    pub fn clear(&self) {
        self.scored.lock().unwrap().clear();
    }

    fn scored(&self, range: Range<usize>, samples: &[f32]) -> Result<Arc<Vec<Vec<f32>>>> {
        let slice = samples
            .get(range.clone())
            .ok_or_else(|| CoreError::Transcription("Word Boost chunk out of range".into()))?;
        let fingerprint = chunks::fingerprint(slice);
        if let Some(hit) = self.scored.lock().unwrap().get(&range.start) {
            if hit.range == range && hit.fingerprint == fingerprint {
                return Ok(Arc::clone(&hit.log_probs));
            }
        }
        let log_probs = Arc::new(self.model.log_probs(slice)?);
        self.scored.lock().unwrap().insert(
            range.start,
            Scored {
                range,
                fingerprint,
                log_probs: Arc::clone(&log_probs),
            },
        );
        Ok(log_probs)
    }

    /// CTC log-probabilities for the whole recording, stitched from chunks.
    pub fn log_probs(&self, samples: &[f32]) -> Result<Vec<Vec<f32>>> {
        let mut parts = Vec::new();
        for range in chunks::chunks(samples.len()) {
            parts.push(self.scored(range, samples)?.as_ref().clone());
        }
        Ok(chunks::stitch(&parts, FRAME_SECONDS))
    }

    /// Apply Word Boost to the recognizer's raw text and token timings.
    pub fn boost(
        &self,
        text: &str,
        timings: &[TokenTiming],
        log_probs: &[Vec<f32>],
    ) -> Option<BoostOutcome> {
        let vocab = self
            .vocabulary
            .lock()
            .unwrap()
            .as_ref()
            .map(|(_, v)| Arc::clone(v))?;
        Some(boost(text, timings, log_probs, FRAME_SECONDS, &vocab))
    }
}
