//! What your fixes taught about how your words sound (Mac and iPhone
//! `SoundAnchor` / `SoundAnchorBook`). When you change a dictated "email" to
//! "Eimailar", Orttaai asks the recording what it heard and keeps a sound
//! anchor: the heard word, your word, how your word sounds to the speech
//! model ("emailer"), and the sound margin of that fix. From then on every
//! "email" the speech model writes is checked against the sound
//! ([`super::contrast`]): it becomes Eimailar only when the sound says
//! "emailer", so plain "email" stays "email". Fixing it back the other way
//! ("Eimailar" back to "email") teaches the opposite, and the line between
//! the two moves to fit your voice.
//!
//! Anchors live in `sound-anchors.json` in the data folder. They are never
//! synced and hold no audio.

use super::judge;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Margins kept for each side: enough to place the line, not a history.
pub const EVIDENCE_KEPT: usize = 12;
/// Ways your word sounds, kept: a few voices and speeds.
pub const SOUNDS_KEPT: usize = 4;
/// With no fix back yet, the line sits at zero (the sound fits your word
/// better than the heard one), or a little below when your fixes were only
/// weakly backed by the sound: never below this after one fix...
pub const LOWEST_LINE_AFTER_ONE_FIX: f32 = -1.0;
/// ...or this after several.
pub const LOWEST_LINE: f32 = -2.0;
/// Never above this, however a fix back scored.
pub const HIGHEST_LINE: f32 = 8.0;
/// How far above your highest kept heard word the line stays. It was 0.25:
/// on the Mac's run of the owner's 50 real recordings, a plain "email"
/// scored -4.89 after the line had learned from three kept "email"s
/// (highest -5.5) and was changed. At 1.5 the line is -4.0: it keeps that
/// "email" and still takes the owner's fast Eimailar (-3.57).
pub const MARGIN_ABOVE_YOUR_WORD: f32 = 1.5;
/// How far below your weakest fix the line may sit when your kept heard
/// words are far below it.
pub const MARGIN_BELOW_YOUR_FIX: f32 = 1.5;
/// Kept heard words ("email" you left as it was) needed before the line
/// follows your voice below [`LOWEST_LINE`].
pub const OWN_WORDS_TO_TRUST: usize = 3;
pub const LOWEST_LINE_FROM_YOUR_VOICE: f32 = -8.0;
/// Anchors are few (one per word you fixed): more than this, the oldest go.
pub const LARGEST_BOOK: usize = 200;

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundAnchor {
    /// What the speech model writes, as [`super::contrast::key`] joins it
    /// ("email").
    pub heard: String,
    /// Your word ("Eimailar").
    pub term: String,
    /// How your word sounds to the CTC model, learned from your fixes
    /// ("emailer", "email or"), newest first. A heard word is scored against
    /// each and the best counts: the spelling of your word plays no part.
    pub sounds: Vec<String>,
    /// Sound margins where you said your word (fixes to it).
    pub said: Vec<f32>,
    /// Sound margins where you said the heard word (fixes back to it).
    #[serde(rename = "notSaid")]
    pub not_said: Vec<f32>,
    /// Spellings fixed back: never written as your word again.
    #[serde(default)]
    pub forgotten: Vec<String>,
    /// Listening again slowed down wrote your word here and you changed it
    /// back: it isn't tried for this pair again.
    #[serde(
        default,
        rename = "slowListenOff",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub slow_listen_off: bool,
    /// Seconds since 1970.
    pub updated: f64,
}

impl SoundAnchor {
    pub fn new(heard: &str, term: &str, sounds: &[&str]) -> Self {
        Self {
            heard: judge::core_heard(heard),
            term: term.to_string(),
            sounds: merged(sounds.iter().map(|s| s.to_string()).collect(), &[]),
            said: Vec::new(),
            not_said: Vec::new(),
            forgotten: Vec::new(),
            slow_listen_off: false,
            updated: now(),
        }
    }

    pub fn with_said(mut self, said: &[f32]) -> Self {
        self.said = said.to_vec();
        self
    }

    pub fn with_not_said(mut self, not_said: &[f32]) -> Self {
        self.not_said = not_said.to_vec();
        self
    }

    /// The sound margin above which the heard word becomes your word.
    ///
    /// Measured on synthetic speech (10 voices, 6 words, the iPhone's
    /// SoundLab): at this line about half of the times your word was said are
    /// caught, and about 1 in 100 times the ordinary word was said is
    /// changed. Each fix moves it: a fix to your word with a weak sound
    /// lowers it, a fix back raises it above that fix's margin.
    pub fn line(&self) -> f32 {
        let lowest_said = self.said.iter().copied().reduce(f32::min);
        let highest_not_said = self.not_said.iter().copied().reduce(f32::max);
        let mut line = 0.0f32;
        match (lowest_said, highest_not_said) {
            (Some(lowest_said), Some(highest_not_said)) => {
                line = if lowest_said > highest_not_said {
                    // Your voice on both sides: between them, nearer to never
                    // changing the heard word.
                    //
                    // And never far below your weakest fix (not on the
                    // iPhone or Mac). Plain "email"s here score around -11,
                    // so halfway to a fix at -3 is -7: on the 50 real
                    // recordings that changed an "email" that scored -5.7.
                    ((lowest_said + highest_not_said) / 2.0)
                        .max(highest_not_said + MARGIN_ABOVE_YOUR_WORD)
                        .max((lowest_said - MARGIN_BELOW_YOUR_FIX).min(0.0))
                } else {
                    // Overlapping: halfway between the two sides' middles.
                    (median(&self.said) + median(&self.not_said)) / 2.0
                };
                // Enough of your own "email"s to know where they end: the
                // line may go as low as your voice says (the owner's real
                // "email"s scored -9.4 to -5.5, a fast Eimailar -3.6).
                if self.not_said.len() >= OWN_WORDS_TO_TRUST {
                    return HIGHEST_LINE.min(LOWEST_LINE_FROM_YOUR_VOICE.max(line));
                }
            }
            (None, Some(highest_not_said)) => {
                // Only fixes back: the line moves above the worst of them.
                line = (highest_not_said + 0.5).max(0.0);
            }
            (Some(lowest_said), None) if lowest_said < 0.0 => {
                let lowest = if self.said.len() >= 2 {
                    LOWEST_LINE
                } else {
                    LOWEST_LINE_AFTER_ONE_FIX
                };
                line = lowest.max(lowest_said / 2.0);
            }
            _ => {}
        }
        HIGHEST_LINE.min(LOWEST_LINE.max(line))
    }

    pub fn accepts(&self, margin: f32) -> bool {
        margin > self.line()
    }

    pub fn learn_said(&mut self, margin: f32, learned: &[String]) {
        self.said.push(margin);
        keep_last(&mut self.said);
        self.sounds = merged(learned.to_vec(), &self.sounds);
        self.updated = now();
    }

    pub fn learn_not_said(&mut self, margin: f32) {
        self.not_said.push(margin);
        keep_last(&mut self.not_said);
        self.updated = now();
    }
}

fn keep_last(values: &mut Vec<f32>) {
    if values.len() > EVIDENCE_KEPT {
        values.drain(..values.len() - EVIDENCE_KEPT);
    }
}

/// New ways first, each once, lowercased, at most [`SOUNDS_KEPT`].
fn merged(new: Vec<String>, old: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for sound in new.iter().chain(old) {
        let sound = sound.to_lowercase().trim().to_string();
        if !sound.is_empty() && !out.contains(&sound) {
            out.push(sound);
        }
    }
    out.truncate(SOUNDS_KEPT);
    out
}

fn median(values: &[f32]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    if sorted.is_empty() {
        return 0.0;
    }
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    }
}

/// Every anchor, looked up by heard phrase.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SoundAnchorBook {
    pub anchors: Vec<SoundAnchor>,
}

impl SoundAnchorBook {
    pub fn new(anchors: Vec<SoundAnchor>) -> Self {
        Self { anchors }
    }

    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    pub fn anchor(&self, heard: &str, term: &str) -> Option<&SoundAnchor> {
        let key = judge::core_heard(heard);
        self.anchors
            .iter()
            .find(|a| a.heard == key && a.term == term)
    }

    /// Whether `term` is ever written for a heard phrase by sound: such a
    /// pair is decided by the sound, never by a text rule.
    pub fn decides_pair_by_sound(&self, heard: &str, term: &str) -> bool {
        self.anchor(heard, term).is_some()
    }

    /// Whether any anchor says how `term` sounds: then the sound check, not
    /// Word Boost's spelling rules, decides where it's written.
    pub fn decides_by_sound(&self, term: &str) -> bool {
        let lower = term.to_lowercase();
        self.anchors.iter().any(|a| a.term.to_lowercase() == lower)
    }

    /// A fix to your word: you said `term` where the speech model wrote `heard`.
    pub fn learn_said(&mut self, heard: &str, term: &str, sounds: &[String], margin: f32) {
        let key = judge::core_heard(heard);
        if key.is_empty() || term.is_empty() {
            return;
        }
        if let Some(anchor) = self
            .anchors
            .iter_mut()
            .find(|a| a.heard == key && a.term == term)
        {
            anchor.learn_said(margin, sounds);
        } else if !sounds.is_empty() {
            let sounds: Vec<&str> = sounds.iter().map(String::as_str).collect();
            self.anchors
                .push(SoundAnchor::new(&key, term, &sounds).with_said(&[margin]));
        }
        self.trim();
    }

    /// A spelling of your word's sound that isn't an English word ("emailer"
    /// for Eimailar, heard as "email" plus an ending), fixed to your word: it
    /// is your word wherever the speech model writes it. No margin is kept:
    /// nothing was scored. `known` are the sounds a starting guess had.
    pub fn learn_spelling(&mut self, heard: &str, term: &str, spelling: &str, known: &[String]) {
        let key = judge::core_heard(heard);
        let letters = super::contrast::letters(spelling);
        if key.is_empty() || term.is_empty() || letters.is_empty() {
            return;
        }
        let spelling = spelling.to_lowercase();
        if let Some(anchor) = self
            .anchors
            .iter_mut()
            .find(|a| a.heard == key && a.term == term)
        {
            anchor.forgotten.retain(|f| *f != letters);
            anchor.sounds = merged(vec![spelling], &anchor.sounds);
            anchor.updated = now();
        } else {
            let mut sounds = vec![spelling.as_str()];
            sounds.extend(known.iter().map(String::as_str));
            self.anchors.push(SoundAnchor::new(&key, term, &sounds));
            self.trim();
        }
    }

    /// A fix back: you said `heard` where the sound check wrote `term`. With
    /// no anchor yet (the change came from a starting guess), one is made, so
    /// the fix back is remembered.
    pub fn learn_not_said(&mut self, heard: &str, term: &str, margin: f32) {
        let key = judge::core_heard(heard);
        if key.is_empty() || term.is_empty() {
            return;
        }
        if let Some(anchor) = self
            .anchors
            .iter_mut()
            .find(|a| a.heard == key && a.term == term)
        {
            anchor.learn_not_said(margin);
        } else {
            let options = judge::sound_options(&key, term);
            let options: Vec<&str> = options.iter().map(String::as_str).collect();
            self.anchors
                .push(SoundAnchor::new(&key, term, &options).with_not_said(&[margin]));
            self.trim();
        }
    }

    /// A spelling of your word's sound fixed back to what it says ("emailer"
    /// back from Eimailar): never written as your word again.
    pub fn forget_spelling(&mut self, spelling: &str, term: &str) {
        let letters = super::contrast::letters(spelling);
        for anchor in self.anchors.iter_mut().filter(|a| a.term == term) {
            if anchor.forgotten.contains(&letters) {
                continue;
            }
            anchor.forgotten.push(letters.clone());
            anchor
                .sounds
                .retain(|s| super::contrast::letters(s) != letters);
        }
    }

    /// Listening again slowed down wrote `term` for `heard` and it was fixed
    /// back: it isn't tried for this pair again.
    pub fn stop_slow_listen(&mut self, heard: &str, term: &str) {
        let key = judge::core_heard(heard);
        for anchor in self
            .anchors
            .iter_mut()
            .filter(|a| a.heard == key && a.term == term)
        {
            anchor.slow_listen_off = true;
        }
    }

    /// Forgets every anchor for a word, as when it leaves your dictionary.
    pub fn forget(&mut self, term: &str) {
        let lower = term.to_lowercase();
        self.anchors.retain(|a| a.term.to_lowercase() != lower);
    }

    fn trim(&mut self) {
        if self.anchors.len() <= LARGEST_BOOK {
            return;
        }
        self.anchors.sort_by(|a, b| b.updated.total_cmp(&a.updated));
        self.anchors.truncate(LARGEST_BOOK);
    }

    /// These anchors, plus each of `guesses` they don't already hold (the
    /// same heard word and term). On the iPhone a taught anchor replaces
    /// every guess for its word; here a fix about one heard word ("emails")
    /// must not take away what the guess knows about another ("email").
    pub fn adding(&self, guesses: &SoundAnchorBook) -> SoundAnchorBook {
        let mut anchors = self.anchors.clone();
        anchors.extend(
            guesses
                .anchors
                .iter()
                .filter(|g| {
                    !self.anchors.iter().any(|a| {
                        a.heard == g.heard && a.term.to_lowercase() == g.term.to_lowercase()
                    })
                })
                .cloned(),
        );
        SoundAnchorBook { anchors }
    }

    /// The anchors your fixes taught, plus starting ones for your words that
    /// have none yet. The starting ones are never saved.
    pub fn adding_guesses(&self, terms: &[String]) -> SoundAnchorBook {
        self.adding(&SoundAnchorBook::new(super::guesses::anchors(terms)))
    }

    // ---- Stored ----

    /// An unreadable or missing file is an empty book.
    pub fn load(path: &Path) -> SoundAnchorBook {
        std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    /// Written whole to a temporary file and renamed into place.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        let part = path.with_extension("json.part");
        std::fs::write(&part, data)?;
        std::fs::rename(&part, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(said: &[f32]) -> SoundAnchor {
        SoundAnchor::new("email", "Eimailar", &["emailer"]).with_said(said)
    }

    #[test]
    fn line_starts_at_zero() {
        let anchor = SoundAnchor::new("Email", "Eimailar", &["emailer"]).with_said(&[4.2]);
        assert_eq!(anchor.heard, "email");
        assert_eq!(anchor.line(), 0.0);
        assert!(anchor.accepts(1.0));
        assert!(!anchor.accepts(-0.5));
    }

    #[test]
    fn a_weak_fix_allows_a_little_below_zero() {
        let weak = anchor(&[-1.2]);
        assert_eq!(weak.line(), -0.6);
        assert!(weak.accepts(-0.4));
        // One fix never takes the line below -1; several, -2.
        assert_eq!(anchor(&[-6.0]).line(), -1.0);
        assert_eq!(anchor(&[-6.0, 2.0]).line(), -2.0);
    }

    #[test]
    fn fixes_both_ways_put_the_line_between() {
        let mut anchor = anchor(&[3.0, 5.0]);
        anchor.learn_not_said(1.0);
        assert_eq!(anchor.line(), 2.5);
        assert!(!anchor.accepts(2.4));
        assert!(anchor.accepts(2.6));
    }

    #[test]
    fn only_fixes_back_raise_the_line() {
        let mut anchor = anchor(&[]);
        anchor.learn_not_said(2.0);
        assert_eq!(anchor.line(), 2.5);
        // Never above the top, however a fix back scored.
        anchor.learn_not_said(40.0);
        assert_eq!(anchor.line(), HIGHEST_LINE);
    }

    #[test]
    fn evidence_is_bounded() {
        let mut anchor = anchor(&[]);
        for value in 0..30 {
            anchor.learn_said(value as f32, &[]);
        }
        assert_eq!(anchor.said.len(), EVIDENCE_KEPT);
        assert_eq!(anchor.said.first(), Some(&18.0));
    }

    #[test]
    fn your_kept_words_let_the_line_follow_your_voice() {
        // The owner's voice: a fix at -5.2, kept "email"s at -9.4, -9.3, -5.5.
        let mut anchor = SoundAnchor::new("email", "Eimailar", &["emaila"]).with_said(&[-5.2]);
        anchor.learn_not_said(-9.4);
        anchor.learn_not_said(-9.3);
        assert_eq!(anchor.line(), -2.0); // two kept words: not trusted yet
        anchor.learn_not_said(-5.5);
        assert_eq!(anchor.line(), -4.0);
        assert!(anchor.accepts(-3.6));
        assert!(!anchor.accepts(-4.89));
        assert!(!anchor.accepts(-5.5));
        assert!(!anchor.accepts(-6.6));
    }

    #[test]
    fn the_line_stays_near_a_weak_fix_when_kept_words_are_far_below() {
        // This PC's voice: a fix at -3, kept "email"s near -11.
        let mut anchor = SoundAnchor::new("email", "Eimailar", &["emaila"]).with_said(&[-3.0]);
        for kept in [-11.9, -11.4, -11.4] {
            anchor.learn_not_said(kept);
        }
        assert_eq!(anchor.line(), -4.5);
        assert!(!anchor.accepts(-5.7), "a real email from the 50 recordings");
        assert!(anchor.accepts(-3.0));
        // A strong fix doesn't lift the line above zero.
        let mut strong = SoundAnchor::new("email", "Eimailar", &["emaila"]).with_said(&[3.9]);
        for kept in [-11.9, -11.4, -11.4] {
            strong.learn_not_said(kept);
        }
        assert_eq!(strong.line(), 0.0);
    }

    #[test]
    fn book_learns_forgets_and_round_trips() {
        let mut book = SoundAnchorBook::default();
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        book.learn_said("email", "Eimailar", &s(&["emailer"]), 3.0);
        book.learn_said("Email", "Eimailar", &s(&["Emaila", "emailer"]), 4.0);
        assert_eq!(book.anchors.len(), 1);
        let anchor = book.anchor("email", "Eimailar").unwrap();
        assert_eq!(anchor.said, vec![3.0, 4.0]);
        assert_eq!(anchor.sounds, vec!["emaila", "emailer"]);
        book.learn_not_said("email", "Eimailar", 1.0);
        assert_eq!(
            book.anchor("email", "Eimailar").unwrap().not_said,
            vec![1.0]
        );
        assert!(book.decides_pair_by_sound("Email", "Eimailar"));
        assert!(!book.decides_pair_by_sound("email", "Vercel"));
        assert!(book.decides_by_sound("eimailar"));

        let dir = std::env::temp_dir().join(format!("orttaai-anchors-{}", std::process::id()));
        let path = dir.join("nested").join("sound-anchors.json");
        book.save(&path).unwrap();
        assert_eq!(SoundAnchorBook::load(&path), book);
        // The iPhone's field names, so the format is one format.
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"notSaid\":[1.0]"), "{json}");
        assert!(!json.contains("slowListenOff"), "{json}");
        let _ = std::fs::remove_dir_all(&dir);

        book.forget("eimailar");
        assert!(book.is_empty());
        assert!(SoundAnchorBook::load(Path::new("/nonexistent/anchors.json")).is_empty());
    }

    #[test]
    fn a_file_from_the_iphone_reads() {
        let json = r#"{"anchors":[{"heard":"email","notSaid":[-9.4],"said":[-5.2],"sounds":["emaila"],"term":"Eimailar","updated":1800000000}]}"#;
        let book: SoundAnchorBook = serde_json::from_str(json).unwrap();
        assert_eq!(book.anchors[0].not_said, vec![-9.4]);
        assert!(book.anchors[0].forgotten.is_empty());
        assert!(!book.anchors[0].slow_listen_off);
    }

    #[test]
    fn a_fix_back_with_no_anchor_makes_one_and_slow_listen_can_be_stopped() {
        let mut book = SoundAnchorBook::default();
        book.learn_not_said("email", "Eimailar", 1.5);
        let anchor = book.anchor("email", "Eimailar").unwrap();
        assert!(anchor.sounds.contains(&"emailer".to_string()));
        assert_eq!(anchor.line(), 2.0);
        book.stop_slow_listen("Email", "Eimailar");
        assert!(book.anchors[0].slow_listen_off);
        // Nothing is made from a fix to a word with no sounds to go on.
        book.learn_said("verse", "Vercel", &[], 2.0);
        assert_eq!(book.anchors.len(), 1);
    }

    #[test]
    fn a_fix_about_another_heard_word_keeps_the_guess() {
        let mut book = SoundAnchorBook::default();
        book.learn_said("emails", "Eimailar", &["eimailar".to_string()], 1.0);
        let merged = book.adding_guesses(&["Eimailar".to_string()]);
        let heard: Vec<&str> = merged.anchors.iter().map(|a| a.heard.as_str()).collect();
        assert_eq!(heard, ["emails", "email"]);
    }

    #[test]
    fn a_spelling_fixed_to_your_word_is_learned_without_a_margin() {
        let mut book = SoundAnchorBook::default();
        let known = vec!["eimailar".to_string(), "emaila".to_string()];
        book.learn_spelling("email", "Eimailar", "Emailer", &known);
        let anchor = book.anchor("email", "Eimailar").unwrap();
        assert_eq!(anchor.sounds, ["emailer", "eimailar", "emaila"]);
        assert!(anchor.said.is_empty() && anchor.line() == 0.0);
        // Fixed back, then taught again: no longer forgotten.
        book.forget_spelling("emailer", "Eimailar");
        assert!(!judge::sound_spellings(&book).contains_key("emailer"));
        book.learn_spelling("email", "Eimailar", "emailer", &[]);
        assert_eq!(
            judge::sound_spellings(&book)
                .get("emailer")
                .map(String::as_str),
            Some("Eimailar")
        );
    }

    #[test]
    fn taught_anchors_are_kept_over_guesses() {
        let book = SoundAnchorBook::new(vec![anchor(&[3.0])]);
        let merged = book.adding_guesses(&["Eimailar".to_string(), "soundAr".to_string()]);
        assert_eq!(
            merged
                .anchors
                .iter()
                .filter(|a| a.term == "Eimailar")
                .count(),
            1
        );
        assert!(merged.anchors.iter().any(|a| a.term == "soundAr"));
    }
}
