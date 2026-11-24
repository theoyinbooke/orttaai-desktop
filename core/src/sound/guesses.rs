//! Starting anchors for your words before you've fixed anything (Mac and
//! iPhone `SoundAnchorGuesses`). A name like Eimailar is spelled like an
//! everyday word with an ending ("email" + "ar"), and that everyday word is
//! what the speech model writes when you say it fast. So each of your words
//! that looks like that gets an anchor from the start, with nothing learned
//! yet: the line at zero, the sound's spellings ("emailer", "emailar") and
//! listening again slowed down. Your fixes then teach the real sound and move
//! the line, as for any anchor.

use super::anchors::SoundAnchor;
use super::judge::{sound_options, LOST_ENDINGS};
use crate::text::lexicon;
use crate::wordboost::guard::spelling_similarity;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// How alike your word and "everyday word + ending" must be spelled.
/// Near-exact only: "emailar" for Eimailar is 0.88, "soundar" for soundAr 1,
/// "equipar" for iquipAr 0.86; "meetupa" for Meetumo (0.71) and "hetmaner"
/// for Hetzner (0.75) are too far to guess.
pub const MINIMUM_SIMILARITY: f32 = 0.85;

/// Each term's guess, worked out once: the word list is long. Call this when
/// the dictionary changes, never while a dictation waits.
fn cache() -> &'static Mutex<HashMap<String, Option<SoundAnchor>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<SoundAnchor>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// For each of `terms`, the everyday word it starts like, if one fits.
pub fn anchors(terms: &[String]) -> Vec<SoundAnchor> {
    terms
        .iter()
        .filter_map(|term| {
            if let Some(known) = cache().lock().unwrap().get(term) {
                return known.clone();
            }
            let guess = guessed(term);
            cache().lock().unwrap().insert(term.clone(), guess.clone());
            guess
        })
        .collect()
}

fn guessed(term: &str) -> Option<SoundAnchor> {
    if !lexicon::is_available() || term.contains(' ') {
        return None;
    }
    let letters: String = term
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect();
    let count = letters.chars().count();
    // Long enough to be a name, and not itself an everyday word.
    if count < 6 || lexicon::is_real_word(&letters) {
        return None;
    }
    let first = letters.chars().next()?;
    let mut best: Option<(&str, f32)> = None;
    for word in lexicon::listed_words() {
        let length = word.chars().count();
        if !word.starts_with(first) || length < 4 || length >= count || length + 4 < count {
            continue;
        }
        for ending in LOST_ENDINGS {
            let similarity = spelling_similarity(&format!("{word}{ending}"), &letters);
            if similarity < MINIMUM_SIMILARITY {
                continue;
            }
            // The closest; between equals the longer word, then the same
            // one every time.
            let better = match best {
                None => true,
                Some((top, top_similarity)) => {
                    similarity > top_similarity
                        || (similarity == top_similarity
                            && (length > top.chars().count()
                                || (length == top.chars().count() && word < top)))
                }
            };
            if better {
                best = Some((word, similarity));
            }
        }
    }
    let (word, _) = best?;
    let options = sound_options(word, term);
    let options: Vec<&str> = options.iter().map(String::as_str).collect();
    Some(SoundAnchor::new(word, term, &options))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn your_words_start_with_an_anchor_when_they_look_like_a_word_and_an_ending() {
        let terms: Vec<String> = [
            "Eimailar", "soundAr", "Temitope", "Mylokai", "keyboard", "Meetumo", "Hetzner",
        ]
        .iter()
        .map(|t| t.to_string())
        .collect();
        let found = anchors(&terms);
        let heard = |term: &str| {
            found
                .iter()
                .find(|a| a.term == term)
                .map(|a| a.heard.as_str())
        };
        assert_eq!(heard("Eimailar"), Some("email"));
        assert_eq!(heard("soundAr"), Some("sound"));
        assert_eq!(heard("keyboard"), None);
        assert_eq!(heard("Meetumo"), None);
        assert_eq!(heard("Hetzner"), None);
        assert!(found.iter().all(|a| a.said.is_empty() && a.line() == 0.0));
        // Asked again, the answer comes from the cache and is the same.
        assert_eq!(anchors(&terms), found);
    }

    #[test]
    fn a_guess_knows_the_spellings_of_its_sound() {
        let found = anchors(&["Eimailar".to_string()]);
        assert_eq!(
            found[0].sounds,
            ["eimailar", "emailer", "emailar", "emaila"]
        );
    }
}
