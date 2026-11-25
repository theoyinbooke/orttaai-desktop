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
//! ORTTAAI_SOUND_LAB_SYNTH=…/sound-lab-e2e \
//! cargo test --release -p orttaai-core --features express --lib -- --ignored sound_lab --nocapture
//! ```
//!
//! The output holds clip ids, counts and scores. What was said is printed
//! only with `ORTTAAI_SOUND_LAB_SHOW_TEXT=1`.

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

/// What was written, for the log: nothing but a word count unless
/// `ORTTAAI_SOUND_LAB_SHOW_TEXT=1`. The recordings are the owner's own
/// words, and logs hold counts and scores only.
fn shown(text: &str) -> String {
    if std::env::var("ORTTAAI_SOUND_LAB_SHOW_TEXT").is_ok_and(|v| v == "1") {
        format!("«{text}»")
    } else {
        format!("({} words)", text.split_whitespace().count())
    }
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
    let t: Arc<dyn Transcriber> = Arc::new(transcriber());
    t.set_word_boost_terms(&terms());
    assert!(t.sound_ready(), "Word Boost's model didn't load");
    let opts = DecodeOptions::default();
    let dictionary = vec![TERM.to_string()];
    let guessed = if guesses {
        SoundAnchorBook::new(crate::sound::guesses::anchors(&dictionary))
    } else {
        SoundAnchorBook::default()
    };
    // What your fixes taught (stored), and what dictation listens with.
    let mut stored = SoundAnchorBook::default();
    let listening = |stored: &SoundAnchorBook| Arc::new(stored.adding(&guessed));
    // What Express writes with the word in the dictionary and no anchors,
    // and how long each decode takes without the sound check.
    let mut plain_ms = 0u128;
    for (id, _) in &items {
        let started = std::time::Instant::now();
        let raw = t.transcribe(&clip(id), &opts).unwrap();
        plain_ms += started.elapsed().as_millis();
        if guesses {
            eprintln!("{tag} without anchors {id} {}", shown(&raw));
        }
    }
    let memory = crate::sound::SoundMemory::new();
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
                t.set_sound_anchors(listening(&stored));
                let audio = clip(id);
                let raw = t.transcribe(&audio, &opts).unwrap();
                let kept = memory
                    .keep(audio, t.sound_decisions(&raw), Arc::clone(&t))
                    .and_then(|id| memory.get(id));
                let untouched = crate::text::learning::DictationEdits::default();
                crate::sound::learn::learn(
                    &mut stored,
                    &guessed,
                    &dictionary,
                    kept.as_ref(),
                    Some(&untouched),
                );
            }
            let line = listening(&stored)
                .anchors
                .iter()
                .find(|a| a.term == TERM)
                .map(|a| a.line());
            eprintln!("{tag} line now {line:?}");
        }
        t.set_sound_anchors(listening(&stored));
        let (mut right, mut said, mut changed, mut plain) = (0, 0, 0, 0);
        let mut first_wrong: Option<(String, String)> = None;
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
                "{tag} {stage} {id} [{}] {} {}",
                if *is_term { "term" } else { "word" },
                shown(&text),
                scores(&t.sound_decisions(&raw))
            );
            if *is_term {
                said += 1;
                if has {
                    right += 1;
                } else if first_wrong.is_none() {
                    first_wrong = Some((id.clone(), text.clone()));
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
            if let Some((id, text)) = first_wrong {
                // The fix a person would make: the word that should have been
                // Eimailar, retyped, and learned the way the app learns it.
                let audio = clip(&id);
                let raw = t.transcribe(&audio, &opts).unwrap();
                let wrong = text
                    .split(' ')
                    .find(|w| {
                        let l = contrast::letters(w);
                        l.starts_with("emai") || l.starts_with("emil")
                    })
                    .map(str::to_string);
                if let Some(wrong) = wrong {
                    let after = text.replacen(&wrong, TERM, 1);
                    let edits = crate::text::learning::dictation_edits(&text, &text, None, &after);
                    let kept = memory
                        .keep(audio, t.sound_decisions(&raw), Arc::clone(&t))
                        .and_then(|id| memory.get(id));
                    let learned = crate::sound::learn::learn(
                        &mut stored,
                        &guessed,
                        &dictionary,
                        kept.as_ref(),
                        edits.as_ref(),
                    );
                    eprintln!(
                        "{tag} fixed {id}: {} said, {} fixed back, {} kept; {} anchor(s) stored",
                        learned.said,
                        learned.fixed_back,
                        learned.kept,
                        stored.anchors.len()
                    );
                    if learned.said > 0 {
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

/// The book after "one fix and 3 kept emails" in the owner's voice: a clip
/// where Eimailar was said and the speech model wrote "email" is fixed by
/// hand and learned by sound, then three plain "email"s are left alone. This
/// is the book whose line follows the voice below zero, so it is the one
/// most likely to change a real "email".
fn taught_book(t: &Arc<dyn Transcriber>) -> SoundAnchorBook {
    let folder = std::env::var("ORTTAAI_SOUND_LAB_OWN").expect("ORTTAAI_SOUND_LAB_OWN");
    let folder = Path::new(&folder);
    let clip = |id: &str| samples(&folder.join(format!("{id}.wav")));
    let opts = DecodeOptions::default();
    let dictionary = vec![TERM.to_string()];
    let guessed = SoundAnchorBook::new(crate::sound::guesses::anchors(&dictionary));
    let memory = crate::sound::SoundMemory::new();
    let mut stored = SoundAnchorBook::default();
    // Written "email" with no anchors: the fix a person would have to make.
    t.set_sound_anchors(Arc::new(SoundAnchorBook::default()));
    for id in ["term-03", "fast-04"] {
        let audio = clip(id);
        let raw = t.transcribe(&audio, &opts).unwrap();
        let text = written(&raw);
        if !contrast::key(&text).contains(&"email".to_string()) {
            continue;
        }
        let after = text.replacen("email", TERM, 1);
        let edits = crate::text::learning::dictation_edits(&text, &text, None, &after);
        let kept = memory
            .keep(audio, Vec::new(), Arc::clone(t))
            .and_then(|id| memory.get(id));
        let learned = crate::sound::learn::learn(
            &mut stored,
            &guessed,
            &dictionary,
            kept.as_ref(),
            edits.as_ref(),
        );
        eprintln!("REAL taught from {id}: {} said", learned.said);
        if learned.said > 0 {
            break;
        }
    }
    for id in ["word-01", "word-02", "word-03"] {
        t.set_sound_anchors(Arc::new(stored.adding(&guessed)));
        let audio = clip(id);
        let raw = t.transcribe(&audio, &opts).unwrap();
        let kept = memory
            .keep(audio, t.sound_decisions(&raw), Arc::clone(t))
            .and_then(|id| memory.get(id));
        crate::sound::learn::learn(
            &mut stored,
            &guessed,
            &dictionary,
            kept.as_ref(),
            Some(&crate::text::learning::DictationEdits::default()),
        );
    }
    let book = stored.adding(&guessed);
    for a in &book.anchors {
        eprintln!(
            "REAL taught anchor: {} sound(s), said {:?}, kept {:?}, line {:.2}",
            a.sounds.len(),
            a.said,
            a.not_said,
            a.line()
        );
    }
    book
}

/// The 50 real-voice sentences recorded for Orttaai's tests: no transcript
/// may change, with the starting guess alone and with the book the owner's
/// voice teaches (one fix and 3 kept "email"s), whose line sits below zero.
#[test]
#[ignore]
fn sound_lab_real_corpus() {
    let manifest_path = std::env::var("ORTTAAI_SOUND_LAB_REAL").expect("ORTTAAI_SOUND_LAB_REAL");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let base = Path::new(&manifest_path).parent().unwrap();
    let t: Arc<dyn Transcriber> = Arc::new(transcriber());
    t.set_word_boost_terms(&terms());
    assert!(t.sound_ready(), "Word Boost's model didn't load");
    let opts = DecodeOptions::default();
    let taught = taught_book(&t);
    let mut weak = taught.clone();
    for anchor in weak.anchors.iter_mut().filter(|a| a.term == TERM) {
        anchor.said = vec![-3.0];
        eprintln!("REAL weak-fix line {:.2}", anchor.line());
    }
    let books = [
        (
            "starting guess",
            SoundAnchorBook::default().adding_guesses(&[TERM.to_string()]),
        ),
        ("one fix and 3 kept emails", taught.clone()),
        // The same, had the fix been weakly backed by the sound (as fixes
        // are: a word is fixed where the sound check kept it). The line
        // must not fall to the middle of a wide gap.
        ("a weak fix (-3.0) and 3 kept emails", weak),
    ];
    let mut failed = 0;
    for (name, book) in &books {
        let (mut total, mut changed_plain, mut changed_boost, mut judged) = (0, 0, 0, 0);
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
            judged += decisions.len();
            if written(&anchored) != written(&plain) {
                changed_plain += 1;
                eprintln!(
                    "REAL [{name}] changed {}: {} -> {}",
                    item["id"],
                    shown(&plain),
                    shown(&anchored)
                );
            }
            if anchored != boosted {
                changed_boost += 1;
            }
            if !decisions.is_empty() {
                eprintln!(
                    "REAL [{name}] {} {} {}",
                    item["id"],
                    shown(&anchored),
                    scores(&decisions)
                );
            }
        }
        eprintln!(
            "REAL [{name}]: {changed_plain}/{total} transcripts changed with the anchor on ({judged} heard words judged; {changed_boost} differ from Word Boost alone)"
        );
        // The check ran for real: the corpus has two plain "email"s.
        assert!(judged >= 2, "the sound check never ran");
        failed += changed_plain;
    }
    assert_eq!(failed, 0, "transcripts changed");
}

/// Synthetic voices (the iPhone lab's clips, made with macOS `say`): files
/// named `<voice>-<rate>-<n>-term.wav` say "emailer", `…-word.wav` say plain
/// "email", at 170 to 320 words a minute. Counts what was written with the
/// starting guess only, by speed, and which step wrote it: the slow listen's
/// stretch is checked here by what the speech model then hears.
#[test]
#[ignore]
fn sound_lab_synthetic() {
    let folder = std::env::var("ORTTAAI_SOUND_LAB_SYNTH").expect("ORTTAAI_SOUND_LAB_SYNTH");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&folder)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "wav"))
        .collect();
    files.sort();
    let t = transcriber();
    t.set_word_boost_terms(&terms());
    let book = SoundAnchorBook::default().adding_guesses(&[TERM.to_string()]);
    t.set_sound_anchors(Arc::new(book));
    let opts = DecodeOptions::default();
    // rate -> (said, shown, by spelling, by margin, by slow listen, plain, changed, changed by slow listen)
    let mut by_rate: std::collections::BTreeMap<String, [usize; 8]> = Default::default();
    let mut kept_plain: Vec<f32> = Vec::new();
    let mut slow_caught: Vec<f32> = Vec::new();
    let mut missed: Vec<f32> = Vec::new();
    for file in &files {
        let name = file.file_stem().unwrap().to_string_lossy().to_string();
        let parts: Vec<&str> = name.split('-').collect();
        let (Some(rate), Some(kind)) = (parts.get(1), parts.last()) else {
            continue;
        };
        let is_term = *kind == "term";
        let raw = t.transcribe(&samples(file), &opts).unwrap();
        let text = written(&raw);
        let has = text.contains(TERM);
        let decisions = t.sound_decisions(&raw);
        let row = by_rate.entry(rate.to_string()).or_default();
        let slow = decisions
            .iter()
            .any(|d| d.applied && d.margin == crate::sound::judge::SLOW_LISTEN_MARGIN);
        if is_term {
            row[0] += 1;
            if has {
                row[1] += 1;
                if decisions.iter().any(|d| d.applied && d.margin >= 99.0) {
                    row[2] += 1;
                } else if slow {
                    row[4] += 1;
                    slow_caught.extend(decisions.iter().filter_map(|d| d.sound_margin));
                } else if decisions.iter().any(|d| d.applied) {
                    row[3] += 1;
                }
            } else {
                missed.extend(
                    decisions
                        .iter()
                        .filter(|d| !d.applied)
                        .filter_map(|d| d.sound_margin),
                );
                eprintln!(
                    "SYNTH missed {name} {} {}",
                    shown(&text),
                    scores(&decisions)
                );
            }
        } else {
            row[5] += 1;
            if has {
                row[6] += 1;
                if slow {
                    row[7] += 1;
                }
                eprintln!(
                    "SYNTH changed {name} {} {}",
                    shown(&text),
                    scores(&decisions)
                );
            } else {
                kept_plain.extend(
                    decisions
                        .iter()
                        .filter(|d| !d.applied)
                        .filter_map(|d| d.sound_margin),
                );
            }
        }
    }
    let mut changed = 0;
    for (rate, r) in &by_rate {
        eprintln!(
            "SYNTH rate {rate}: Eimailar shown {}/{} (spelling {}, sound {}, slow listen {}); plain email changed {}/{} ({} by the slow listen)",
            r[1], r[0], r[2], r[3], r[4], r[6], r[5], r[7]
        );
        changed += r[6];
    }
    let span = |v: &mut Vec<f32>| -> String {
        if v.is_empty() {
            return "none".into();
        }
        v.sort_by(f32::total_cmp);
        format!(
            "{} from {:.1} to {:.1}, median {:.1}",
            v.len(),
            v[0],
            v[v.len() - 1],
            v[v.len() / 2]
        )
    };
    eprintln!(
        "SYNTH margins of plain email kept: {}",
        span(&mut kept_plain)
    );
    eprintln!(
        "SYNTH margins where the slow listen heard the word: {}",
        span(&mut slow_caught)
    );
    eprintln!("SYNTH margins of the word missed: {}", span(&mut missed));
    assert_eq!(changed, 0, "plain email changed");
}
