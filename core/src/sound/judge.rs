//! Applies your sound anchors to one transcript (Mac and iPhone
//! `SoundAnchorJudge`): every place the speech model wrote an anchor's heard
//! word is scored by sound against each way your word sounds, and written as
//! your word only when the sound says so. The transcriber supplies the
//! scoring ([`super::contrast`] over its CTC frames).

use super::anchors::SoundAnchorBook;
use super::contrast::{self, SoundWord};
use crate::text::lexicon;
use crate::wordboost::guard::spelling_similarity;
use std::collections::HashMap;
use std::ops::Range;

/// Ways of sounding kept from each fix (the best few of the candidates).
pub const SOUNDS_LEARNED_PER_FIX: usize = 3;

/// Pieces the speech model splits off the end of your word ("emailer" as
/// "email er"): scored together with the heard word. Only pieces that aren't
/// everyday words: in the iPhone's lab, plain "email or a text" and "email at
/// noon" sounded as much like "emailer" as the real thing, so taking "or" or
/// "at" along changed one in ten of them.
pub const ABSORBABLE: &[&str] = &["er", "ar", "uh", "ah", "erm"];
/// Taking a piece along needs the sound to be this much clearer.
pub const ABSORBED_EXTRA_LINE: f32 = 1.5;
/// Everyday words that can pass for the end of your word: "email or" and
/// "emailer" are nearly the same sound. Before one of these, the heard word
/// is changed only on much clearer sound.
pub const SOUNDS_LIKE_AN_ENDING: &[&str] = &["or", "a", "at", "are", "her", "of", "uh", "er"];
pub const BEFORE_AN_ENDING_EXTRA_LINE: f32 = 4.0;
/// Endings a fast word loses: "emailer" said, "email" written.
pub const LOST_ENDINGS: &[&str] = &["er", "ar", "a", "or", "ah", "ers", "ars"];

/// The margin recorded where listening again slowed down heard your word.
pub const SLOW_LISTEN_MARGIN: f32 = 50.0;
/// Speeds to listen again at, slowest first: at 60% of the speed, the
/// iPhone's lab heard the longer word in 35 of 133 clips where "emailer" had
/// been written "email", and in none of 478 where "email" was said.
pub const SLOW_LISTEN_RATES: [f32; 2] = [0.6, 0.7];
/// The margin recorded for a word written as one of your word's sound
/// spellings: the sound was unmistakable.
pub const SPELLED_MARGIN: f32 = 99.0;
/// How alike a written word must be to one of your word's sound spellings to
/// count as it: "emila" and "emaila" are 0.83, "emilar" and "emailar" 0.86
/// (both written for Eimailar on the owner's phone), "emily" and "emaila"
/// only 0.67.
pub const NEAR_SPELLING_SIMILARITY: f32 = 0.8;

fn is_ending(word: &str) -> bool {
    SOUNDS_LIKE_AN_ENDING.contains(&word)
}

/// One place a sound anchor judged: "email" kept or written Eimailar.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Decision {
    pub heard: String,
    pub term: String,
    /// The margin the decision was made on ([`SPELLED_MARGIN`] for a sound
    /// spelling, [`SLOW_LISTEN_MARGIN`] where the slow listen heard it).
    pub margin: f32,
    pub applied: bool,
    /// The margin with the next short word taken along, when tried.
    pub margin_with_next_word: Option<f32>,
    /// The sound check's own margin for the heard word, whatever was decided
    /// afterwards (listening again keeps it here).
    pub sound_margin: Option<f32>,
    /// The heard words, for a second listen where the sound check kept them.
    #[serde(skip)]
    pub word_range: Option<Range<usize>>,
}

/// Whether listening again slowed down heard your word where the heard one
/// was written: your word itself, one of its sound spellings, or the heard
/// word with the ending it lost ("emailer" for "email").
pub fn slow_listen_heard(
    words: &[String],
    heard: &str,
    term: &str,
    spellings: &HashMap<String, String>,
) -> bool {
    let term_letters = contrast::letters(term);
    words.iter().any(|word| {
        let spelled = contrast::letters(word);
        spelled == term_letters
            || spellings.get(&spelled).is_some_and(|t| t == term)
            || is_longer_form(word, heard)
    })
}

/// `word` is `heard` with an ending it lost ("emailer" for "email"): a fix
/// like that is about the sound, so it's learned by sound even when the word
/// isn't in your dictionary.
pub fn is_longer_form(word: &str, heard: &str) -> bool {
    let core = contrast::letters(&core_heard(heard));
    let spelled = contrast::letters(word);
    if core.is_empty() {
        return false;
    }
    spelled
        .strip_prefix(core.as_str())
        .is_some_and(|ending| LOST_ENDINGS.contains(&ending))
}

/// Spellings to try when learning how your word sounds, before the CTC
/// model's own: your word in lower case, split where its capitals are, and
/// the heard word with the endings it most often lost.
pub fn sound_options(heard: &str, term: &str) -> Vec<String> {
    let base = core_heard(heard);
    let mut out: Vec<String> = Vec::new();
    let candidates = [term.to_lowercase(), split(term)]
        .into_iter()
        .chain(["er", "ar", "a", "or"].iter().map(|e| format!("{base}{e}")));
    for candidate in candidates {
        if !candidate.is_empty() && !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// Whether the CTC model's own spelling of the fixed words can stand for how
/// your word sounds. Its spellings are often noise ("to wecome chter"); on
/// the iPhone lab's clips, keeping only ones like these took false changes
/// from 18 in 211 to 1.
pub fn is_plausible_sound(spelling: &str, heard: &str, term: &str) -> bool {
    let sound = contrast::letters(spelling);
    let word = contrast::letters(term);
    let base = contrast::letters(heard);
    if sound.is_empty() || sound == base || sound.chars().count() < base.chars().count() {
        return false;
    }
    if spelling.split(' ').filter(|w| !w.is_empty()).count() > split(term).split(' ').count() + 1 {
        return false;
    }
    spelling_similarity(&sound, &word) >= 0.5
}

/// The heard words without an everyday ending word the speech model split
/// off your word: a fix from "email at" teaches about "email". Anchors are
/// kept for that, so a genuine "email at noon" is weighed with the stricter
/// check for endings, never as a taught phrase.
pub fn core_heard(heard: &str) -> String {
    let mut words = contrast::key(heard);
    while words.len() > 1 && words.last().is_some_and(|w| is_ending(w)) {
        words.pop();
    }
    words.join(" ")
}

/// "soundAr" -> "sound ar", "BudgetBook" -> "budget book".
pub fn split(term: &str) -> String {
    let mut out = String::new();
    for character in term.chars() {
        if character.is_uppercase() && !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push(character);
    }
    out.to_lowercase()
}

/// What the judge decided for one transcript.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Judged {
    /// UTF-8 ranges of the text to write as your words.
    pub replacements: Vec<(Range<usize>, String)>,
    pub decisions: Vec<Decision>,
}

/// `margin(range, heard, sound)` is the sound margin of a word range, scored
/// as heard text against one way of sounding; `None` when it can't be scored.
pub fn judge(
    book: &SoundAnchorBook,
    text: &str,
    words: &[SoundWord],
    margin: &dyn Fn(&Range<usize>, &str, &str) -> Option<f32>,
) -> Judged {
    let mut out = Judged::default();
    if book.is_empty() || words.is_empty() {
        return out;
    }
    let ranges = utf8_ranges(words, text);
    let mut taken: Vec<Range<usize>> = Vec::new();
    let overlaps = |a: &Range<usize>, b: &Range<usize>| a.start < b.end && b.start < a.end;

    // Your word as the speech model spells its sound ("emailer",
    // "E-mailer"): the sound has already said which word it was.
    let spellings = sound_spellings(book);
    for (index, word) in words.iter().enumerate() {
        let letters = contrast::letters(&word.text);
        let Some(term) = spellings
            .get(&letters)
            .cloned()
            .or_else(|| near_spelling(&letters, &spellings))
        else {
            continue;
        };
        let Some(range) = ranges[index].clone() else {
            continue;
        };
        if word.text == term {
            continue;
        }
        out.replacements.push((range, term.clone()));
        taken.push(index..index + 1);
        out.decisions.push(Decision {
            heard: word.text.clone(),
            term,
            margin: SPELLED_MARGIN,
            applied: true,
            margin_with_next_word: None,
            sound_margin: None,
            word_range: None,
        });
    }

    let joined = |range: &Range<usize>| {
        words[range.clone()]
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    for anchor in book.anchors.iter().filter(|a| !a.sounds.is_empty()) {
        for occurrence in contrast::occurrences(&anchor.heard, words) {
            if taken.iter().any(|t| overlaps(t, &occurrence)) {
                continue;
            }
            let best = |range: &Range<usize>| -> Option<f32> {
                let heard = joined(range);
                anchor
                    .sounds
                    .iter()
                    .filter_map(|sound| margin(range, &heard, sound))
                    .reduce(f32::max)
            };
            let Some(alone) = best(&occurrence) else {
                continue;
            };
            let next_key = words
                .get(occurrence.end)
                .map(|w| contrast::key(&w.text).concat())
                .unwrap_or_default();
            // "email or" heard (a fix taught it) or "email" heard before
            // "or": either way an everyday word may be the ending.
            let heard_words: Vec<&str> = anchor.heard.split(' ').collect();
            let ends_in_an_ending =
                heard_words.len() > 1 && heard_words.last().is_some_and(|w| is_ending(w));
            // Before an everyday ending the bar never drops with your fixes:
            // "email at lunch" must stay, however often you said Eimailar.
            let anchor_line = anchor.line();
            let line = if is_ending(&next_key) || ends_in_an_ending {
                anchor_line.max(0.0) + BEFORE_AN_ENDING_EXTRA_LINE
            } else {
                anchor_line
            };
            let mut chosen: Option<(Range<usize>, f32)> =
                (alone > line).then(|| (occurrence.clone(), alone));
            let mut with_next = None;
            if ABSORBABLE.contains(&next_key.as_str()) || is_ending(&next_key) {
                let longer = occurrence.start..occurrence.end + 1;
                with_next = best(&longer);
                if let Some(together) = with_next {
                    if ABSORBABLE.contains(&next_key.as_str())
                        && together > anchor_line + ABSORBED_EXTRA_LINE
                        && together > alone
                    {
                        chosen = Some((longer, together));
                    }
                }
            }
            let heard = joined(&occurrence);
            let placed = chosen.and_then(|(range, margin)| {
                let first = ranges[range.start].clone()?;
                let last = ranges[range.end - 1].clone()?;
                Some((range, margin, first.start..last.end))
            });
            let Some((range, chosen_margin, utf8)) = placed else {
                out.decisions.push(Decision {
                    heard,
                    term: anchor.term.clone(),
                    margin: alone,
                    applied: false,
                    margin_with_next_word: with_next,
                    sound_margin: Some(alone),
                    word_range: Some(occurrence),
                });
                continue;
            };
            out.replacements.push((utf8, anchor.term.clone()));
            taken.push(range);
            out.decisions.push(Decision {
                heard,
                term: anchor.term.clone(),
                margin: chosen_margin,
                applied: true,
                margin_with_next_word: with_next,
                sound_margin: Some(alone),
                word_range: None,
            });
        }
    }
    out
}

/// Spellings of your words' sounds that stand for them wherever the speech
/// model writes them: one word, not the heard word itself, and not in the
/// English word list ("emailer" and "emaila" are kept, "sounder" isn't: that
/// one is left to the sound check).
pub fn sound_spellings(book: &SoundAnchorBook) -> HashMap<String, String> {
    let mut spellings = HashMap::new();
    if !lexicon::is_available() {
        return spellings;
    }
    for anchor in &book.anchors {
        let heard = contrast::letters(&anchor.heard);
        let term = contrast::letters(&anchor.term);
        let options = sound_options(&anchor.heard, &anchor.term);
        for sound in anchor.sounds.iter().chain(&options) {
            let spelled = contrast::letters(sound);
            if sound.contains(' ')
                || spelled.chars().count() < 4
                || spelled == heard
                || spelled == term
                || anchor.forgotten.contains(&spelled)
                || lexicon::is_listed_word(&spelled)
            {
                continue;
            }
            spellings
                .entry(spelled)
                .or_insert_with(|| anchor.term.clone());
        }
    }
    spellings
}

/// A word the speech model spelled close to one of your word's sound
/// spellings ("Emila" for "emaila"), that isn't English: your word.
fn near_spelling(letters: &str, spellings: &HashMap<String, String>) -> Option<String> {
    let count = letters.chars().count();
    if count < 4 || !lexicon::is_available() || lexicon::is_real_word(letters) {
        return None;
    }
    let mut best: Option<(&String, &String, f32)> = None;
    for (spelling, term) in spellings {
        if spelling.chars().count().abs_diff(count) > 2 {
            continue;
        }
        let similarity = spelling_similarity(letters, spelling);
        if similarity < NEAR_SPELLING_SIMILARITY {
            continue;
        }
        // The closest spelling; between equals, the same one every time.
        let better = match best {
            None => true,
            Some((s, _, top)) => similarity > top || (similarity == top && spelling < s),
        };
        if better {
            best = Some((spelling, term, similarity));
        }
    }
    best.map(|(_, term, _)| term.clone())
}

/// Each word's UTF-8 range in `text`, found in order; `None` where the text
/// doesn't spell the word the same way.
pub fn utf8_ranges(words: &[SoundWord], text: &str) -> Vec<Option<Range<usize>>> {
    let mut cursor = 0;
    words
        .iter()
        .map(|word| {
            if word.text.is_empty() {
                return None;
            }
            let found = text[cursor..].find(&word.text)? + cursor;
            cursor = found + word.text.len();
            Some(found..cursor)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::anchors::SoundAnchor;
    use super::*;
    use crate::wordboost::rescore::apply_replacements;

    fn words(texts: &[&str]) -> Vec<SoundWord> {
        texts
            .iter()
            .enumerate()
            .map(|(i, w)| SoundWord::new(*w, i as f64, i as f64 + 0.5))
            .collect()
    }

    fn book() -> SoundAnchorBook {
        SoundAnchorBook::new(vec![
            SoundAnchor::new("email", "Eimailar", &["emailer"]).with_said(&[3.0])
        ])
    }

    const TEXT: &str = "Open email and the email er list.";
    fn text_words() -> Vec<SoundWord> {
        words(&["Open", "email", "and", "the", "email", "er", "list."])
    }

    fn applied(judged: &Judged, text: &str) -> String {
        apply_replacements(&judged.replacements, text)
    }

    #[test]
    fn writes_your_word_only_where_the_sound_says_so() {
        // The first "email" sounded like "email", the second like "emailer".
        let text = "Open email and the email list.";
        let w = words(&["Open", "email", "and", "the", "email", "list."]);
        let judged = judge(&book(), text, &w, &|range, _, _| {
            Some(if range.start == 1 { -6.0 } else { 2.0 })
        });
        assert_eq!(applied(&judged, text), "Open email and the Eimailar list.");
        let flags: Vec<bool> = judged.decisions.iter().map(|d| d.applied).collect();
        assert_eq!(flags, [false, true]);
        assert_eq!(judged.decisions[0].word_range, Some(1..2));
        assert_eq!(judged.decisions[0].sound_margin, Some(-6.0));
    }

    #[test]
    fn takes_a_short_word_along_only_when_clearly_better() {
        // "email er" together sounds like "emailer" far better than "email" alone.
        let judged = judge(&book(), TEXT, &text_words(), &|range, _, _| {
            Some(if *range == (4..6) {
                4.0
            } else if range.start == 4 {
                0.5
            } else {
                -6.0
            })
        });
        assert_eq!(applied(&judged, TEXT), "Open email and the Eimailar list.");
        // Only a little better: nothing changes, since "er" after it also
        // asks the heard word alone for much clearer sound.
        let kept = judge(&book(), TEXT, &text_words(), &|range, _, _| {
            Some(if *range == (4..6) {
                1.0
            } else if range.start == 4 {
                0.5
            } else {
                -6.0
            })
        });
        assert!(kept.replacements.is_empty());
    }

    #[test]
    fn before_an_everyday_ending_the_sound_must_be_clearer() {
        let w = words(&["an", "email", "or", "a", "text"]);
        let text = "an email or a text";
        let some = judge(&book(), text, &w, &|_, _, _| Some(2.0));
        assert!(some.replacements.is_empty());
        let clear = judge(&book(), text, &w, &|_, _, _| Some(5.0));
        assert_eq!(applied(&clear, text), "an Eimailar or a text");
    }

    #[test]
    fn the_bar_before_an_ending_never_drops_with_your_fixes() {
        // A line your voice took down to -5.25 still asks for 4 before "at".
        let mut anchor = SoundAnchor::new("email", "Eimailar", &["emaila"]).with_said(&[-5.2]);
        for kept in [-9.4, -9.3, -5.5] {
            anchor.learn_not_said(kept);
        }
        assert_eq!(anchor.line(), -5.25);
        let book = SoundAnchorBook::new(vec![anchor]);
        let w = words(&["read", "the", "email", "at", "lunch"]);
        let text = "read the email at lunch";
        assert!(judge(&book, text, &w, &|_, _, _| Some(3.5))
            .replacements
            .is_empty());
        let plain = words(&["read", "the", "email", "today"]);
        let judged = judge(&book, "read the email today", &plain, &|_, _, _| Some(-3.6));
        assert_eq!(judged.replacements.len(), 1);
    }

    #[test]
    fn a_taught_phrase_ending_in_an_everyday_word_becomes_the_word() {
        // Taught from "email or": kept as "email", and "email or" in later
        // speech needs the stricter check for endings.
        let book =
            SoundAnchorBook::new(vec![
                SoundAnchor::new("email or", "Eimailar", &["emailer"]).with_said(&[3.0])
            ]);
        assert_eq!(book.anchors[0].heard, "email");
        let w = words(&["send", "the", "email", "or", "update"]);
        let judged = judge(&book, "send the email or update", &w, &|_, _, _| Some(2.0));
        assert!(judged.replacements.is_empty());
    }

    #[test]
    fn your_word_spelled_as_it_sounds_is_your_word() {
        let text = "Open E-mailer and check the emailer list, not the email.";
        let w = words(&[
            "Open", "E-mailer", "and", "check", "the", "emailer", "list,", "not", "the", "email.",
        ]);
        let judged = judge(&book(), text, &w, &|_, _, _| Some(-9.0));
        assert_eq!(
            applied(&judged, text),
            "Open Eimailar and check the Eimailar list, not the email."
        );
        assert_eq!(judged.decisions.iter().filter(|d| d.applied).count(), 2);
    }

    #[test]
    fn everyday_words_are_never_spellings() {
        let book = SoundAnchorBook::new(vec![SoundAnchor::new(
            "sound",
            "soundAr",
            &["sounder", "soundar"],
        )
        .with_said(&[2.0])]);
        let spellings = sound_spellings(&book);
        assert!(!spellings.contains_key("sounder"));
        assert!(!spellings.contains_key("sound"));
        assert_eq!(
            spellings.get("soundar"),
            None,
            "the word itself isn't a spelling of it"
        );
        assert_eq!(spellings.get("sounda").map(String::as_str), Some("soundAr"));
    }

    #[test]
    fn a_spelling_fixed_back_is_forgotten() {
        let mut book = book();
        book.forget_spelling("Emailer", "Eimailar");
        let spellings = sound_spellings(&book);
        assert!(!spellings.contains_key("emailer"));
        assert_eq!(
            spellings.get("emaila").map(String::as_str),
            Some("Eimailar")
        );
    }

    #[test]
    fn slow_listen_hears_only_the_longer_word() {
        let s = |v: &[&str]| v.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        let spellings: HashMap<String, String> = [("emailer".to_string(), "Eimailar".to_string())]
            .into_iter()
            .collect();
        let none = HashMap::new();
        assert!(slow_listen_heard(
            &s(&["open", "emailer", "and"]),
            "email",
            "Eimailar",
            &none
        ));
        assert!(slow_listen_heard(
            &s(&["open", "eimailar"]),
            "email",
            "Eimailar",
            &none
        ));
        assert!(slow_listen_heard(
            &s(&["open", "emaila"]),
            "email",
            "Eimailar",
            &none
        ));
        assert!(!slow_listen_heard(
            &s(&["e", "mailer"]),
            "email",
            "Eimailar",
            &spellings
        ));
        assert!(!slow_listen_heard(
            &s(&["open", "email", "and"]),
            "email",
            "Eimailar",
            &spellings
        ));
        assert!(!slow_listen_heard(
            &s(&["emails", "emailed"]),
            "email",
            "Eimailar",
            &spellings
        ));
    }

    #[test]
    fn a_fix_from_a_word_and_an_ending_teaches_the_word() {
        let mut book = SoundAnchorBook::default();
        book.learn_said("email at", "Eimailar", &["emailer".to_string()], 2.0);
        let heard: Vec<&str> = book.anchors.iter().map(|a| a.heard.as_str()).collect();
        assert_eq!(heard, ["email"]);
        assert!(sound_options("email or", "Eimailar").contains(&"emailer".to_string()));
        assert_eq!(core_heard("my local"), "my local");
        assert_eq!(core_heard("or"), "or");
    }

    #[test]
    fn no_anchors_change_nothing() {
        let judged = judge(
            &SoundAnchorBook::default(),
            TEXT,
            &text_words(),
            &|_, _, _| Some(9.0),
        );
        assert!(judged.replacements.is_empty() && judged.decisions.is_empty());
    }

    #[test]
    fn plausible_sounds_look_like_your_word() {
        assert!(is_plausible_sound("emailer", "email", "Eimailar"));
        assert!(is_plausible_sound("email or", "email", "Eimailar"));
        assert!(!is_plausible_sound("email", "email", "Eimailar"));
        assert!(!is_plausible_sound("to wecome chter", "sound", "soundAr"));
        assert!(!is_plausible_sound("so", "sound", "soundAr"));
    }

    #[test]
    fn sound_options_cover_common_endings() {
        assert_eq!(
            sound_options("Email", "Eimailar"),
            ["eimailar", "emailer", "emailar", "emaila", "emailor"]
        );
        assert_eq!(split("soundAr"), "sound ar");
        assert_eq!(split("BudgetBook"), "budget book");
    }

    #[test]
    fn longer_forms_are_the_heard_word_and_a_lost_ending() {
        assert!(is_longer_form("emailer", "email"));
        assert!(is_longer_form("Emailar", "email or"));
        assert!(!is_longer_form("email", "email"));
        assert!(!is_longer_form("emails", "email"));
        assert!(!is_longer_form("emailed", "email"));
        assert!(!is_longer_form("er", ""));
    }

    #[test]
    fn misspellings_of_your_words_sound_are_your_word() {
        let book = SoundAnchorBook::default().adding_guesses(&["Eimailar".to_string()]);
        let text = "We fixed the bug in emila yesterday. Emilar is ready. Emily said email works.";
        let w: Vec<SoundWord> = text
            .split(' ')
            .map(|word| SoundWord::new(word, 0.0, 0.0))
            .collect();
        let judged = judge(&book, text, &w, &|_, _, _| Some(-9.0));
        assert_eq!(
            applied(&judged, text),
            "We fixed the bug in Eimailar yesterday. Eimailar is ready. Emily said email works."
        );
    }

    #[test]
    fn words_the_text_spells_differently_are_left_alone() {
        // The transcript was tidied after the tokens were read: no range, no change.
        let w = words(&["Open", "email", "now"]);
        let judged = judge(&book(), "Open e mail now", &w, &|_, _, _| Some(6.0));
        assert!(judged.replacements.is_empty());
        assert_eq!(
            utf8_ranges(&w, "Open email now"),
            vec![Some(0..4), Some(5..10), Some(11..14)]
        );
    }
}
