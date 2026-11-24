//! Opt-in checks of the sound anchors against real recordings (the iPhone's
//! `SoundLab`). Nothing here runs in the normal test suite, and no recording
//! is ever copied into the repo: each test reads a folder named by an
//! environment variable.
//!
//! ```text
//! ORTTAAI_EXPRESS_MODEL_DIR=…/fluid-speech-mini-onnx \
//! ORTTAAI_WORD_BOOST_DIR=…/sherpa-onnx-nemo-parakeet_tdt_ctc_110m-en-36000-int8 \
//! ORTTAAI_SOUND_LAB_OWN=…/OwnVoice/orttakey-sound \
//! ORTTAAI_SOUND_LAB_REAL=…/gauntlet/asr_eval/corpus_real/manifest.json \
//! cargo test --release -p orttaai-core --features express --lib -- --ignored sound_lab --nocapture
//! ```

use super::express::ExpressTranscriber;
use super::Transcriber;
use crate::memory::MemoryService;
use crate::sound::{contrast, SoundAnchorBook};
use crate::text::{format_numbers, TextOptions, TextProcessor, WordBoostTerm};
use crate::types::{DecodeOptions, TARGET_SAMPLE_RATE};
use std::path::Path;
use std::sync::Arc;

const TERM: &str = "Eimailar";

fn samples(path: &Path) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    assert_eq!(reader.spec().sample_rate, TARGET_SAMPLE_RATE);
    assert_eq!(reader.spec().channels, 1);
    match reader.spec().sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect(),
    }
}

fn transcriber() -> ExpressTranscriber {
    let dir = std::env::var("ORTTAAI_EXPRESS_MODEL_DIR").expect("ORTTAAI_EXPRESS_MODEL_DIR");
    std::env::var("ORTTAAI_WORD_BOOST_DIR").expect("ORTTAAI_WORD_BOOST_DIR");
    ExpressTranscriber::from_dir(Path::new(&dir), 4).unwrap()
}

fn terms() -> Vec<WordBoostTerm> {
    vec![WordBoostTerm {
        text: TERM.into(),
        aliases: Vec::new(),
    }]
}

/// What dictation would type: Express's text through number formatting and
/// the text pipeline with the word in the dictionary.
fn written(raw: &str) -> String {
    let mut memory = MemoryService::new();
    memory.add_term(TERM, TERM);
    let processor = TextProcessor::new(TextOptions::default(), memory);
    processor.process(&format_numbers(raw)).text
}

fn scores(decisions: &[crate::sound::Decision]) -> String {
    decisions
        .iter()
        .map(|d| format!("{}{:.1}", if d.applied { "+" } else { "kept " }, d.margin))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The owner's own voice: 10 sentences with Eimailar, 6 with plain "email".
/// Counts Eimailar shown and plain "email" changed before any fix, after one
/// fix, and after one fix plus three "email"s left as they were.
#[test]
#[ignore]
fn sound_lab_own_voice() {
    // As shipped: the word starts with its guessed anchor.
    own_voice(true);
    // The fix alone: no starting guess, so the first fix teaches everything.
    own_voice(false);
}

fn own_voice(guesses: bool) {
    let tag = if guesses {
        "OWN"
    } else {
        "OWN (no starting guess)"
    };
    let folder = std::env::var("ORTTAAI_SOUND_LAB_OWN").expect("ORTTAAI_SOUND_LAB_OWN");
    let folder = Path::new(&folder);
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("manifest.json")).unwrap())
            .unwrap();
    let items: Vec<(String, bool)> = manifest["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["id"].as_str().unwrap().to_string(),
                i["said"].as_str().unwrap() == "term",
            )
        })
        .collect();
    let clip = |id: &str| samples(&folder.join(format!("{id}.wav")));
    let t = transcriber();
    t.set_word_boost_terms(&terms());
    assert!(t.sound_ready(), "Word Boost's model didn't load");
    let opts = DecodeOptions::default();
    let mut book = if guesses {
        SoundAnchorBook::default().adding_guesses(&[TERM.to_string()])
    } else {
        SoundAnchorBook::default()
    };
    // What Express writes with the word in the dictionary and no anchors,
    // and how long each decode takes without the sound check.
    let mut plain_ms = 0u128;
    for (id, _) in &items {
        let started = std::time::Instant::now();
        let raw = t.transcribe(&clip(id), &opts).unwrap();
        plain_ms += started.elapsed().as_millis();
        if guesses {
            eprintln!("{tag} without anchors {id} «{raw}»");
        }
    }
    let mut fixed: Option<String> = None;
    let mut results = Vec::new();
    let taught_ids = ["word-01", "word-02", "word-03"];
    for stage in [
        "before any fix",
        "after one fix",
        "after one fix and 3 kept emails",
    ] {
        let last = stage == "after one fix and 3 kept emails";
        if last {
            // The first three plain "email"s, left as they were, teach where
            // this voice's "email"s end; they're then left out of the count.
            for id in taught_ids {
                t.set_sound_anchors(Arc::new(book.clone()));
                let text = t.transcribe(&clip(id), &opts).unwrap();
                for d in t.sound_decisions(&text) {
                    if !d.applied {
                        book.learn_not_said(&d.heard, &d.term, d.margin);
                    }
                }
            }
            let line = book
                .anchors
                .iter()
                .find(|a| a.term == TERM)
                .map(|a| a.line());
            eprintln!("{tag} line now {line:?}");
        }
        t.set_sound_anchors(Arc::new(book.clone()));
        let (mut right, mut said, mut changed, mut plain) = (0, 0, 0, 0);
        let mut first_wrong: Option<String> = None;
        let (mut slowest, mut total_ms) = (0u128, 0u128);
        for (id, is_term) in &items {
            if fixed.as_deref() == Some(id) || (last && taught_ids.contains(&id.as_str())) {
                continue;
            }
            let started = std::time::Instant::now();
            let raw = t.transcribe(&clip(id), &opts).unwrap();
            slowest = slowest.max(started.elapsed().as_millis());
            total_ms += started.elapsed().as_millis();
            let text = written(&raw);
            let has = text.contains(TERM);
            eprintln!(
                "{tag} {stage} {id} [{}] «{text}» {}",
                if *is_term { "term" } else { "word" },
                scores(&t.sound_decisions(&raw))
            );
            if *is_term {
                said += 1;
                if has {
                    right += 1;
                } else if first_wrong.is_none() {
                    first_wrong = Some(id.clone());
                }
            } else {
                plain += 1;
                if has {
                    changed += 1;
                }
            }
        }
        eprintln!(
            "{tag} {stage}: Eimailar shown {right}/{said}; plain email changed {changed}/{plain}; slowest decode {slowest} ms, mean {} ms (without anchors {} ms)",
            total_ms / (said + plain).max(1) as u128,
            plain_ms / items.len() as u128
        );
        results.push((stage, right, said, changed, plain));
        if stage == "before any fix" {
            if let Some(id) = first_wrong {
                let audio = clip(&id);
                // The fix a person would make: the word that came out "email".
                t.set_sound_anchors(Arc::new(SoundAnchorBook::default()));
                let raw = t.transcribe(&audio, &opts).unwrap();
                t.set_sound_anchors(Arc::new(book.clone()));
                let heard = raw
                    .split(' ')
                    .find(|w| contrast::key(w).concat().starts_with("email"))
                    .map(str::to_string);
                if let Some(heard) = heard {
                    if let Some(learned) = t.learn_sound(&audio, &heard, TERM, 0).unwrap() {
                        eprintln!(
                            "{tag} fixed {id}: learned {:?}, margin {:.2}",
                            learned.sounds, learned.margin
                        );
                        book.learn_said(&heard, TERM, &learned.sounds, learned.margin);
                        fixed = Some(id);
                    }
                }
            }
        }
    }
    // Plain "email" must never change, at any stage.
    for (stage, _, _, changed, _) in &results {
        assert_eq!(*changed, 0, "plain email changed {stage}");
    }
}

/// The 50 real-voice sentences recorded for Orttaai's tests: with the
/// starting guess and a learned fix on, no transcript may change.
#[test]
#[ignore]
fn sound_lab_real_corpus() {
    let manifest_path = std::env::var("ORTTAAI_SOUND_LAB_REAL").expect("ORTTAAI_SOUND_LAB_REAL");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let base = Path::new(&manifest_path).parent().unwrap();
    let t = transcriber();
    let opts = DecodeOptions::default();
    let mut book = SoundAnchorBook::default().adding_guesses(&[TERM.to_string()]);
    // A fix as the own-voice lab learns it, so the taught path is on too.
    book.learn_said(
        "email",
        TERM,
        &["emaila".to_string(), "emailer".to_string()],
        -1.5,
    );
    let (mut total, mut changed_plain, mut changed_boost, mut emails) = (0, 0, 0, 0);
    for item in manifest["items"].as_array().unwrap() {
        let audio = samples(&base.join(item["wav"].as_str().unwrap()));
        // No dictionary at all.
        t.set_word_boost_terms(&[]);
        t.set_sound_anchors(Arc::new(SoundAnchorBook::default()));
        let plain = t.transcribe(&audio, &opts).unwrap();
        // The word in the dictionary, no anchors (as before this change).
        t.set_word_boost_terms(&terms());
        let boosted = t.transcribe(&audio, &opts).unwrap();
        // The word in the dictionary with its anchor.
        t.set_sound_anchors(Arc::new(book.clone()));
        let anchored = t.transcribe(&audio, &opts).unwrap();
        let decisions = t.sound_decisions(&anchored);
        total += 1;
        emails += decisions.len();
        if written(&anchored) != written(&plain) {
            changed_plain += 1;
            eprintln!("REAL changed {}: «{plain}» -> «{anchored}»", item["id"]);
        }
        if anchored != boosted {
            changed_boost += 1;
            eprintln!(
                "REAL differs from Word Boost alone {}: «{boosted}» -> «{anchored}»",
                item["id"]
            );
        }
        if !decisions.is_empty() {
            eprintln!("REAL {} «{anchored}» {}", item["id"], scores(&decisions));
        }
    }
    eprintln!(
        "REAL: {changed_plain}/{total} transcripts changed with the anchor on ({emails} heard words judged; {changed_boost} differ from Word Boost alone)"
    );
    assert_eq!(changed_plain, 0);
}
