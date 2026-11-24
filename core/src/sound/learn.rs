//! Learning how your words sound from what you do after a dictation (the
//! iPhone keyboard's `learnSounds` and `finishWatching`).
//!
//! | You do this | It learns |
//! |---|---|
//! | Change a dictated word to one of your dictionary words ("email" to Eimailar), or to the heard word plus the ending it lost ("email" to "emailer") | You said your word there. Its sound is learned from the recording, never as a text pair. |
//! | Change your word back ("Eimailar" to "email") where the sound wrote it | You didn't say it there. The line moves above that margin, and no pair is made the other way. |
//! | Leave a heard word the sound check kept ("email") as it was | One of your own "email"s: it shows where your voice's line sits. |
//!
//! The sound comes from the recording itself, which [`SoundMemory`] keeps for
//! the last few dictations, for ten minutes, in memory only.

use super::anchors::SoundAnchorBook;
use super::contrast;
use super::judge::{self, Decision};
use crate::text::learning::{DictationEdits, Replacement};
use crate::text::lexicon;
use crate::transcription::Transcriber;
use crate::types::TARGET_SAMPLE_RATE;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Recordings kept for learning from a fix.
pub const RECORDINGS_KEPT: usize = 3;
/// How long each is kept.
pub const KEEP_FOR: Duration = Duration::from_secs(600);
/// Longer recordings aren't kept: two minutes is 7.7 MB.
pub const LONGEST_KEPT_SAMPLES: usize = 120 * TARGET_SAMPLE_RATE as usize;
/// The stored anchors, in the data folder.
pub const FILE_NAME: &str = "sound-anchors.json";

/// One dictation's sound, with what the sound check decided in it.
#[derive(Clone)]
pub struct KeptRecording {
    pub id: u64,
    pub samples: Arc<Vec<f32>>,
    pub decisions: Vec<Decision>,
    pub transcriber: Arc<dyn Transcriber>,
    kept_at: Instant,
}

/// The last few dictations' sound, in memory only and never written to disk.
#[derive(Default)]
pub struct SoundMemory {
    inner: Mutex<(u64, VecDeque<KeptRecording>)>,
}

impl SoundMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep a dictation's sound; returns its id, or `None` when it is empty
    /// or too long to keep.
    pub fn keep(
        &self,
        samples: Vec<f32>,
        decisions: Vec<Decision>,
        transcriber: Arc<dyn Transcriber>,
    ) -> Option<u64> {
        if samples.is_empty() || samples.len() > LONGEST_KEPT_SAMPLES {
            return None;
        }
        let mut inner = self.inner.lock().unwrap();
        inner.0 += 1;
        let id = inner.0;
        inner.1.push_back(KeptRecording {
            id,
            samples: Arc::new(samples),
            decisions,
            transcriber,
            kept_at: Instant::now(),
        });
        while inner.1.len() > RECORDINGS_KEPT {
            inner.1.pop_front();
        }
        Some(id)
    }

    pub fn get(&self, id: u64) -> Option<KeptRecording> {
        self.purge();
        self.inner
            .lock()
            .unwrap()
            .1
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    /// Drop recordings older than [`KEEP_FOR`].
    pub fn purge(&self) {
        self.purge_at(Instant::now());
    }

    pub fn purge_at(&self, now: Instant) {
        self.inner
            .lock()
            .unwrap()
            .1
            .retain(|r| now.saturating_duration_since(r.kept_at) < KEEP_FOR);
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().1.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().1.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What one watched dictation taught.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Learned {
    /// Fixes to your word, learned by sound.
    pub said: usize,
    /// Your word changed back where the sound wrote it.
    pub fixed_back: usize,
    /// Heard words left as they were.
    pub kept: usize,
    /// The replacements handled here, as `(heard, corrected)`: the text
    /// learner must not turn them into dictionary pairs.
    pub handled: Vec<(String, String)>,
}

impl Learned {
    pub fn anything(&self) -> bool {
        self.said + self.fixed_back + self.kept > 0
    }
}

/// Learn from one watched dictation. `dictionary` are the active dictionary
/// words, `guesses` their starting anchors, and `edits` what the user did to
/// the dictation (`None` when that isn't known: then only nothing is learned
/// from the words left alone).
pub fn learn(
    book: &mut SoundAnchorBook,
    guesses: &SoundAnchorBook,
    dictionary: &[String],
    recording: Option<&KeptRecording>,
    edits: Option<&DictationEdits>,
) -> Learned {
    let mut learned = Learned::default();
    let Some(edits) = edits else {
        return learned;
    };
    let mut decisions: Vec<Decision> = recording.map(|r| r.decisions.clone()).unwrap_or_default();
    for fix in &edits.replacements {
        let handled = fixed_back(book, guesses, &mut decisions, fix, &mut learned)
            || said(
                book,
                guesses,
                dictionary,
                recording,
                &decisions,
                fix,
                &mut learned,
            );
        if handled {
            learned
                .handled
                .push((fix.heard.clone(), fix.corrected.clone()));
        }
    }
    // Each heard word the sound check kept and you left as it was is one of
    // your own. Missing a wrong one only makes the line stricter.
    for decision in &decisions {
        if decision.applied || decision.margin >= judge::SLOW_LISTEN_MARGIN {
            continue;
        }
        let words = contrast::key(&decision.heard);
        if words
            .iter()
            .any(|w| edits.touched.contains(&contrast::letters(w)))
        {
            continue;
        }
        book.learn_not_said(&decision.heard, &decision.term, decision.margin);
        learned.kept += 1;
    }
    learned
}

/// Your word changed back to what was heard, where the sound check wrote it.
fn fixed_back(
    book: &mut SoundAnchorBook,
    guesses: &SoundAnchorBook,
    decisions: &mut Vec<Decision>,
    fix: &Replacement,
    learned: &mut Learned,
) -> bool {
    let Some(index) = decisions.iter().position(|d| {
        // By letters, so "E-mailer" put back as "emailer" still counts.
        d.applied
            && d.term.to_lowercase() == fix.heard.to_lowercase()
            && contrast::letters(&d.heard) == contrast::letters(&fix.corrected)
    }) else {
        return false;
    };
    let decision = decisions.remove(index);
    if decision.margin >= judge::SPELLED_MARGIN {
        // A spelling of its sound ("emailer") that you put back: never
        // again. A starting guess that knew the spelling is kept from now
        // on, so that it stays forgotten.
        if !book.anchors.iter().any(|a| a.term == decision.term) {
            let from_guess: Vec<_> = guesses
                .anchors
                .iter()
                .filter(|g| g.term == decision.term)
                .cloned()
                .collect();
            book.anchors.extend(from_guess);
        }
        book.forget_spelling(&decision.heard, &decision.term);
    } else if decision.margin >= judge::SLOW_LISTEN_MARGIN {
        // Listening again was wrong here: what the sound check itself said
        // is the evidence, and the slow listen isn't tried for it again.
        book.learn_not_said(
            &decision.heard,
            &decision.term,
            decision.sound_margin.unwrap_or(0.0),
        );
        book.stop_slow_listen(&decision.heard, &decision.term);
    } else {
        book.learn_not_said(&decision.heard, &decision.term, decision.margin);
    }
    learned.fixed_back += 1;
    true
}

/// A heard word changed to one of your words, or to itself plus the ending
/// it lost: you said your word there.
fn said(
    book: &mut SoundAnchorBook,
    guesses: &SoundAnchorBook,
    dictionary: &[String],
    recording: Option<&KeptRecording>,
    decisions: &[Decision],
    fix: &Replacement,
    learned: &mut Learned,
) -> bool {
    let heard_words = contrast::key(&fix.heard);
    if heard_words.is_empty() || contrast::key(&fix.corrected).len() != 1 {
        return false;
    }
    let corrected = contrast::letters(&fix.corrected);
    // Your word as the dictionary spells it.
    let term = dictionary
        .iter()
        .find(|w| contrast::letters(w) == corrected && !w.contains(' '))
        .cloned()
        .or_else(|| {
            judge::is_longer_form(&fix.corrected, &fix.heard).then(|| fix.corrected.clone())
        });
    let Some(term) = term else {
        return false;
    };
    if contrast::letters(&term) == heard_words.concat() {
        // A fix of capitals or hyphens only: nothing about the sound.
        return false;
    }
    // Only ordinary words are decided by sound: a misspelling ("Emila") is
    // safe as a dictionary pair, which the text learner suggests.
    if !lexicon::is_available() || !heard_words.iter().all(|w| lexicon::is_real_word(w)) {
        return false;
    }
    let known = |heard: &str| -> Vec<String> {
        guesses
            .anchor(heard, &term)
            .map(|g| g.sounds.clone())
            .unwrap_or_default()
    };
    // "emailer" isn't an English word: it is how your word sounds, spelled
    // out. From now on it is your word, and "email" is judged against it.
    if let [only] = heard_words.as_slice() {
        if !lexicon::is_listed_word(only) {
            let stem = lexicon::stems(only)
                .into_iter()
                .find(|s| lexicon::is_listed_word(s) && judge::is_longer_form(only, s));
            if let Some(stem) = stem {
                book.learn_spelling(&stem, &term, only, &known(&stem));
                learned.said += 1;
                return true;
            }
        }
    }
    let new_anchor = book.anchor(&fix.heard, &term).is_none();
    // The sound's own verdict where it kept the heard word, if it judged it.
    let judged = decisions.iter().find(|d| {
        !d.applied && d.term == term && contrast::letters(&d.heard) == contrast::letters(&fix.heard)
    });
    let from_sound = recording.and_then(|r| {
        r.transcriber
            .learn_sound(&r.samples, &fix.heard, &term, fix.occurrence)
            .unwrap_or_else(|e| {
                tracing::warn!("sound anchors: couldn't learn from a fix: {e}");
                None
            })
    });
    let (mut sounds, margin) = match (from_sound, judged) {
        (Some(sound), _) => (sound.sounds, sound.margin),
        (None, Some(judged)) => (Vec::new(), judged.sound_margin.unwrap_or(judged.margin)),
        (None, None) => return false,
    };
    if new_anchor {
        // What the starting guess knew stays, after what the fix taught.
        sounds.extend(known(&fix.heard));
    }
    let before = book.anchors.len();
    let had = !new_anchor;
    book.learn_said(&fix.heard, &term, &sounds, margin);
    if !had && book.anchors.len() == before {
        // Nothing to go on (no sounds): not learned.
        return false;
    }
    tracing::info!("sound anchors: a fix taught how one of your words sounds (margin {margin:.1})");
    learned.said += 1;
    true
}

/// The anchors that apply to this dictionary: the stored ones whose word is
/// still an active dictionary word (or is a heard word plus its lost ending,
/// which needs no dictionary entry), plus starting guesses for the rest.
pub fn active_book(stored: &SoundAnchorBook, dictionary: &[String]) -> SoundAnchorBook {
    let words: Vec<String> = dictionary.iter().map(|w| w.to_lowercase()).collect();
    let kept: Vec<_> = stored
        .anchors
        .iter()
        .filter(|a| {
            words.contains(&a.term.to_lowercase()) || judge::is_longer_form(&a.term, &a.heard)
        })
        .cloned()
        .collect();
    SoundAnchorBook::new(kept).adding_guesses(dictionary)
}

/// Whether the text pair `source` -> `target` is decided by sound: the source
/// is ordinary English and an anchor says how the target sounds against it.
/// Such a pair would otherwise rewrite every "email" you say.
pub fn pair_decided_by_sound(book: &SoundAnchorBook, source: &str, target: &str) -> bool {
    let words = contrast::key(source);
    !words.is_empty()
        && lexicon::is_available()
        && words.iter().all(|w| lexicon::is_real_word(w))
        && book.decides_pair_by_sound(source, target.trim())
}

/// One writer at a time: fixes are learned on a watcher thread while the
/// dictionary is edited on another.
static STORE: Mutex<()> = Mutex::new(());

/// Read the stored anchors, change them, and write them back when they
/// changed. Returns `f`'s result.
pub fn update_stored<T>(path: &Path, f: impl FnOnce(&mut SoundAnchorBook) -> T) -> T {
    let _guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let mut book = SoundAnchorBook::load(path);
    let before = book.clone();
    let out = f(&mut book);
    if book != before {
        if let Err(e) = book.save(path) {
            tracing::warn!("sound anchors couldn't be saved: {e}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::anchors::SoundAnchor;
    use super::super::LearnedSound;
    use super::*;
    use crate::types::DecodeOptions;

    /// A transcriber that "learns" a fixed sound, and counts the asks.
    struct Sounding(Mutex<Vec<(String, String, usize)>>, Option<LearnedSound>);

    impl Transcriber for Sounding {
        fn load_model(&mut self, _: &str) -> crate::Result<()> {
            Ok(())
        }
        fn loaded_model(&self) -> Option<&str> {
            None
        }
        fn transcribe(&self, _: &[f32], _: &DecodeOptions) -> crate::Result<String> {
            Ok(String::new())
        }
        fn learn_sound(
            &self,
            _samples: &[f32],
            heard: &str,
            term: &str,
            occurrence: usize,
        ) -> crate::Result<Option<LearnedSound>> {
            self.0
                .lock()
                .unwrap()
                .push((heard.to_string(), term.to_string(), occurrence));
            Ok(self.1.clone())
        }
    }

    fn sounding(sound: Option<(&[&str], f32)>) -> Arc<Sounding> {
        Arc::new(Sounding(
            Mutex::new(Vec::new()),
            sound.map(|(sounds, margin)| LearnedSound {
                sounds: sounds.iter().map(|s| s.to_string()).collect(),
                margin,
            }),
        ))
    }

    fn recording(t: &Arc<Sounding>, decisions: Vec<Decision>) -> KeptRecording {
        let memory = SoundMemory::new();
        let id = memory
            .keep(
                vec![0.1; 16_000],
                decisions,
                t.clone() as Arc<dyn Transcriber>,
            )
            .unwrap();
        memory.get(id).unwrap()
    }

    fn decision(heard: &str, margin: f32, applied: bool) -> Decision {
        Decision {
            heard: heard.into(),
            term: "Eimailar".into(),
            margin,
            applied,
            margin_with_next_word: None,
            sound_margin: Some(if margin >= judge::SLOW_LISTEN_MARGIN {
                -3.0
            } else {
                margin
            }),
            word_range: None,
        }
    }

    fn edits(pairs: &[(&str, &str, usize)], touched: &[&str]) -> DictationEdits {
        DictationEdits {
            replacements: pairs
                .iter()
                .map(|(h, c, o)| Replacement {
                    heard: h.to_string(),
                    corrected: c.to_string(),
                    occurrence: *o,
                })
                .collect(),
            touched: touched.iter().map(|t| t.to_string()).collect(),
        }
    }

    fn dictionary() -> Vec<String> {
        vec!["Eimailar".to_string(), "Vercel".to_string()]
    }

    fn guesses() -> SoundAnchorBook {
        SoundAnchorBook::new(super::super::guesses::anchors(&dictionary()))
    }

    #[test]
    fn a_fix_to_your_word_is_learned_from_the_recording() {
        let t = sounding(Some((&["emaila", "emailer"], -1.2)));
        let rec = recording(&t, vec![decision("email", -4.0, false)]);
        let mut book = SoundAnchorBook::default();
        let e = edits(&[("email", "eimailar", 1)], &["email"]);
        let learned = learn(&mut book, &guesses(), &dictionary(), Some(&rec), Some(&e));
        assert_eq!((learned.said, learned.fixed_back, learned.kept), (1, 0, 0));
        assert_eq!(
            learned.handled,
            vec![("email".to_string(), "eimailar".to_string())]
        );
        // Asked about the right word and place, and kept under the
        // dictionary's own spelling with the guess's sounds after the fix's.
        assert_eq!(
            *t.0.lock().unwrap(),
            vec![("email".to_string(), "Eimailar".to_string(), 1)]
        );
        let anchor = book.anchor("email", "Eimailar").unwrap();
        assert_eq!(anchor.said, vec![-1.2]);
        assert_eq!(anchor.sounds, ["emaila", "emailer", "eimailar", "emailar"]);
        assert_eq!(anchor.line(), -0.6);
    }

    #[test]
    fn a_fix_that_only_adds_the_lost_ending_is_learned_too() {
        let t = sounding(Some((&["emailer"], 0.5)));
        let rec = recording(&t, Vec::new());
        let mut book = SoundAnchorBook::default();
        let e = edits(&[("email", "emailer", 0)], &["email"]);
        let learned = learn(&mut book, &guesses(), &dictionary(), Some(&rec), Some(&e));
        assert_eq!(learned.said, 1);
        assert!(book.anchor("email", "emailer").is_some());
    }

    #[test]
    fn fixes_that_arent_about_your_words_sound_are_left_to_the_text_learner() {
        let t = sounding(Some((&["x"], 1.0)));
        let rec = recording(&t, Vec::new());
        let mut book = SoundAnchorBook::default();
        for (heard, corrected) in [
            // Not a dictionary word, not a longer form.
            ("email", "message"),
            ("tanya", "Tania"),
            // A misspelling, not an ordinary word: a text pair is safe.
            ("Emila", "Eimailar"),
            // Capitals only.
            ("eimailar", "Eimailar"),
            // More than one word typed.
            ("email", "the Eimailar"),
        ] {
            let e = edits(&[(heard, corrected, 0)], &[]);
            let learned = learn(&mut book, &guesses(), &dictionary(), Some(&rec), Some(&e));
            assert!(learned.handled.is_empty(), "{heard} -> {corrected}");
        }
        assert!(book.is_empty());
        assert!(t.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_spelling_of_the_sound_fixed_to_your_word_becomes_your_word_everywhere() {
        let t = sounding(None);
        let mut book = SoundAnchorBook::default();
        let e = edits(&[("emailer", "Eimailar", 0)], &["emailer"]);
        let learned = learn(&mut book, &guesses(), &dictionary(), None, Some(&e));
        assert_eq!(learned.said, 1);
        let anchor = book
            .anchor("email", "Eimailar")
            .expect("kept under the stem");
        assert_eq!(anchor.sounds[0], "emailer");
        assert!(anchor.said.is_empty());
        assert!(t.0.lock().unwrap().is_empty());
    }

    #[test]
    fn your_word_changed_back_teaches_the_other_way_and_makes_no_pair() {
        let t = sounding(None);
        // The margin wrote it.
        let rec = recording(&t, vec![decision("email", 1.4, true)]);
        let mut book =
            SoundAnchorBook::new(vec![
                SoundAnchor::new("email", "Eimailar", &["emailer"]).with_said(&[3.0])
            ]);
        let e = edits(&[("Eimailar", "email", 0)], &["eimailar"]);
        let learned = learn(&mut book, &guesses(), &dictionary(), Some(&rec), Some(&e));
        assert_eq!((learned.said, learned.fixed_back), (0, 1));
        assert_eq!(learned.handled.len(), 1);
        let anchor = book.anchor("email", "Eimailar").unwrap();
        assert_eq!(anchor.not_said, vec![1.4]);
        assert!(!anchor.accepts(1.4) && anchor.accepts(3.0));
    }

    #[test]
    fn a_spelling_changed_back_is_forgotten_even_from_a_guess() {
        let t = sounding(None);
        let rec = recording(&t, vec![decision("emailer", judge::SPELLED_MARGIN, true)]);
        let mut book = SoundAnchorBook::default();
        let e = edits(&[("Eimailar", "emailer", 0)], &["eimailar"]);
        let learned = learn(&mut book, &guesses(), &dictionary(), Some(&rec), Some(&e));
        assert_eq!(learned.fixed_back, 1);
        let active = active_book(&book, &dictionary());
        assert!(!judge::sound_spellings(&active).contains_key("emailer"));
        assert!(judge::sound_spellings(&active).contains_key("emaila"));
    }

    #[test]
    fn a_hyphenated_spelling_put_back_plainly_still_counts() {
        let t = sounding(None);
        let rec = recording(&t, vec![decision("E-mailer", judge::SPELLED_MARGIN, true)]);
        let mut book = SoundAnchorBook::default();
        let e = edits(&[("Eimailar", "emailer", 0)], &["eimailar"]);
        let learned = learn(&mut book, &guesses(), &dictionary(), Some(&rec), Some(&e));
        assert_eq!(learned.fixed_back, 1);
        assert!(book.anchors[0].forgotten.contains(&"emailer".to_string()));
    }

    #[test]
    fn a_slow_listen_changed_back_is_not_tried_again() {
        let t = sounding(None);
        let rec = recording(&t, vec![decision("email", judge::SLOW_LISTEN_MARGIN, true)]);
        let mut book = SoundAnchorBook::default();
        let e = edits(&[("Eimailar", "email", 0)], &["eimailar"]);
        learn(&mut book, &guesses(), &dictionary(), Some(&rec), Some(&e));
        let anchor = book.anchor("email", "Eimailar").unwrap();
        assert!(anchor.slow_listen_off);
        // The sound check's own margin is the evidence, not the marker.
        assert_eq!(anchor.not_said, vec![-3.0]);
    }

    #[test]
    fn heard_words_left_alone_teach_your_voices_line() {
        let t = sounding(None);
        let decisions = vec![
            decision("email", -9.4, false),
            decision("email", -5.5, false),
            // Written by the sound: not a kept word.
            decision("email", 4.0, true),
        ];
        let rec = recording(&t, decisions);
        let mut book = SoundAnchorBook::default();
        let untouched = DictationEdits::default();
        let learned = learn(
            &mut book,
            &guesses(),
            &dictionary(),
            Some(&rec),
            Some(&untouched),
        );
        assert_eq!(learned.kept, 2);
        assert_eq!(
            book.anchor("email", "Eimailar").unwrap().not_said,
            vec![-9.4, -5.5]
        );
        // A heard word that was changed or removed is not one you kept, and
        // an unreadable field teaches nothing.
        let mut other = SoundAnchorBook::default();
        let changed = edits(&[], &["email"]);
        assert_eq!(
            learn(
                &mut other,
                &guesses(),
                &dictionary(),
                Some(&rec),
                Some(&changed)
            )
            .kept,
            0
        );
        assert!(!learn(&mut other, &guesses(), &dictionary(), Some(&rec), None).anything());
        assert!(other.is_empty());
    }

    #[test]
    fn without_the_recording_the_sound_checks_own_verdict_is_used() {
        let t = sounding(None);
        let mut rec = recording(&t, vec![decision("email", -2.0, false)]);
        // Sound can't be learned (the model is gone): the judged margin stands.
        let mut book = SoundAnchorBook::default();
        let e = edits(&[("email", "Eimailar", 0)], &["email"]);
        let learned = learn(&mut book, &guesses(), &dictionary(), Some(&rec), Some(&e));
        assert_eq!(learned.said, 1);
        let anchor = book.anchor("email", "Eimailar").unwrap();
        assert_eq!(anchor.said, vec![-2.0]);
        assert!(!anchor.sounds.is_empty(), "the guess's sounds carry over");
        // No verdict either: nothing to learn from, and nothing is handled.
        rec.decisions.clear();
        let mut empty = SoundAnchorBook::default();
        let learned = learn(&mut empty, &guesses(), &dictionary(), Some(&rec), Some(&e));
        assert!(learned.handled.is_empty() && empty.is_empty());
    }

    #[test]
    fn recordings_are_few_short_lived_and_never_huge() {
        let t = sounding(None);
        let memory = SoundMemory::new();
        let keep =
            |n: usize| memory.keep(vec![0.0; n], Vec::new(), t.clone() as Arc<dyn Transcriber>);
        assert_eq!(keep(0), None);
        assert_eq!(keep(LONGEST_KEPT_SAMPLES + 1), None);
        let ids: Vec<u64> = (0..5).map(|_| keep(100).unwrap()).collect();
        assert_eq!(memory.len(), RECORDINGS_KEPT);
        assert!(memory.get(ids[0]).is_none() && memory.get(ids[4]).is_some());
        memory.purge_at(Instant::now() + KEEP_FOR - Duration::from_secs(5));
        assert_eq!(memory.len(), RECORDINGS_KEPT);
        memory.purge_at(Instant::now() + KEEP_FOR + Duration::from_secs(1));
        assert!(memory.is_empty());
    }

    #[test]
    fn anchors_follow_the_dictionary() {
        let stored = SoundAnchorBook::new(vec![
            SoundAnchor::new("email", "Eimailar", &["emaila"]).with_said(&[1.0]),
            SoundAnchor::new("verse", "Vercel", &["vercel"]).with_said(&[1.0]),
            SoundAnchor::new("email", "emailer", &["emailer"]).with_said(&[1.0]),
        ]);
        // Vercel left the dictionary: its anchor no longer applies. The
        // longer form needs no dictionary entry.
        let active = active_book(&stored, &["Eimailar".to_string(), "soundAr".to_string()]);
        let terms: Vec<&str> = active.anchors.iter().map(|a| a.term.as_str()).collect();
        assert_eq!(terms, ["Eimailar", "emailer", "soundAr"]);
        assert_eq!(
            active.anchors[0].said,
            vec![1.0],
            "the taught one, not the guess"
        );
    }

    #[test]
    fn an_ordinary_words_pair_with_an_anchor_is_left_to_the_sound() {
        let book = active_book(&SoundAnchorBook::default(), &dictionary());
        assert!(pair_decided_by_sound(&book, "email", "Eimailar"));
        assert!(pair_decided_by_sound(&book, "Email", " Eimailar "));
        // No anchor for the pair, or not an ordinary word: the text decides.
        assert!(!pair_decided_by_sound(&book, "verse", "Vercel"));
        assert!(!pair_decided_by_sound(&book, "Emila", "Eimailar"));
        assert!(!pair_decided_by_sound(&book, "", "Eimailar"));

        let mut memory = crate::memory::MemoryService::new();
        memory.add_term("email", "Eimailar");
        memory.add_term("Emila", "Eimailar");
        memory.leave_to_sound(|s, t| pair_decided_by_sound(&book, s, t));
        let pass = memory.apply_dictionary("Emila and the email");
        assert_eq!(pass.text, "Eimailar and the email");
        // The pair is still in the dictionary: its word stays known.
        assert_eq!(memory.dictionary().len(), 2);
    }

    #[test]
    fn stored_anchors_are_updated_in_place() {
        let dir = std::env::temp_dir().join(format!("orttaai-sound-learn-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let _ = std::fs::remove_dir_all(&dir);
        // Nothing changed: nothing written.
        update_stored(&path, |_| ());
        assert!(!path.exists());
        update_stored(&path, |book| book.learn_not_said("email", "Eimailar", 1.0));
        update_stored(&path, |book| book.learn_not_said("email", "Eimailar", 2.0));
        assert_eq!(
            SoundAnchorBook::load(&path).anchors[0].not_said,
            vec![1.0, 2.0]
        );
        update_stored(&path, |book| book.forget("Eimailar"));
        assert!(SoundAnchorBook::load(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
