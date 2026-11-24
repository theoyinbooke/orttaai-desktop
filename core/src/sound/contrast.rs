//! Whether a word sounded like what the speech model wrote or like one of
//! your words, judged from the sound itself rather than the spelling (Mac and
//! iPhone `SoundContrast`).
//!
//! The speech model writes "email" when you said "emailer". A text rule
//! ("email" -> "emailer") would then rewrite every "email" you ever say, and
//! Word Boost won't touch an ordinary word. So each candidate is scored
//! against the recording: Word Boost's CTC model gives, for every 80 ms frame,
//! how likely each sound piece is. The phrase with its neighbours,
//! "open email and", and the same phrase with your word, "open emailer and",
//! are each scored over the same stretch of sound with the CTC forward
//! algorithm. The difference is how much better one explains what was said.
//! Either explanation has to account for every frame, so an "-er" that isn't
//! in the audio costs "emailer", and an "-er" that is there costs "email".
//!
//! Pure Rust: the transcriber supplies the frames, timings and token ids.

use std::ops::Range;

/// A word the speech model wrote, with when it was said (seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

impl SoundWord {
    pub fn new(text: impl Into<String>, start: f64, end: f64) -> Self {
        Self {
            text: text.into(),
            start,
            end,
        }
    }

    /// Words from the speech model's token timings: a token that starts with
    /// SentencePiece's "▁" (or a space) starts a new word.
    pub fn from_tokens<'a>(tokens: impl IntoIterator<Item = (&'a str, f64, f64)>) -> Vec<Self> {
        let mut words: Vec<SoundWord> = Vec::new();
        for (token, start, end) in tokens {
            if token.is_empty() || token == "<blank>" || token == "<pad>" {
                continue;
            }
            let starts_word =
                token.starts_with('\u{2581}') || token.starts_with(' ') || words.is_empty();
            let piece = token.replace('\u{2581}', "");
            let piece = piece.trim_matches(|c: char| c == ' ' || c == '\t');
            if starts_word {
                if piece.is_empty() {
                    continue;
                }
                words.push(SoundWord::new(piece, start, end));
            } else if !piece.is_empty() {
                if let Some(last) = words.last_mut() {
                    last.text.push_str(piece);
                    last.end = end;
                }
            }
        }
        words
    }
}

/// The speech model's word timings and the CTC model's frames can be a few
/// frames apart (0.3 s was seen), so each side is scored with three words of
/// context either way and this much sound around them. Both sides explain the
/// same frames, so the context cancels out and only the phrase itself makes
/// the difference.
pub const CONTEXT_WORDS: usize = 3;
pub const PADDING_SECONDS: f64 = 0.4;

/// Lowercased letters, digits and apostrophes: how words are compared.
pub fn key(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !(c.is_alphabetic() || c.is_numeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// A written word by its letters alone, so "E-mail," is "email".
pub fn letters(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic() || c.is_numeric())
        .collect()
}

/// Where `phrase` was written in `words`, as word ranges. Each written word
/// counts by its letters alone, so "E-mail," is "email".
pub fn occurrences(phrase: &str, words: &[SoundWord]) -> Vec<Range<usize>> {
    let wanted: Vec<String> = key(phrase).iter().map(|w| letters(w)).collect();
    if wanted.is_empty() || words.len() < wanted.len() {
        return Vec::new();
    }
    let written: Vec<String> = words.iter().map(|w| letters(&w.text)).collect();
    let mut found = Vec::new();
    let mut index = 0;
    while index + wanted.len() <= written.len() {
        if written[index..index + wanted.len()] == wanted[..] {
            found.push(index..index + wanted.len());
            index += wanted.len();
        } else {
            index += 1;
        }
    }
    found
}

/// The stretch of sound to score a word range in, with its neighbouring
/// words on each side, and the text of those neighbours.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub frames: Range<usize>,
    pub before: String,
    pub after: String,
}

pub fn window(
    range: &Range<usize>,
    words: &[SoundWord],
    frame_duration: f64,
    frame_count: usize,
) -> Option<Window> {
    if !frame_duration.is_finite()
        || frame_duration <= 0.0
        || frame_count == 0
        || range.end > words.len()
        || range.is_empty()
    {
        return None;
    }
    let first = range.start.saturating_sub(CONTEXT_WORDS);
    let last = (range.end - 1 + CONTEXT_WORDS).min(words.len() - 1);
    // Whole recording when the context reaches both ends.
    let start = if first == 0 {
        0.0
    } else {
        (words[first].start - PADDING_SECONDS).max(0.0)
    };
    let lower = (start / frame_duration).floor().max(0.0) as usize;
    let upper = if last == words.len() - 1 {
        frame_count
    } else {
        let end = ((words[last].end + PADDING_SECONDS) / frame_duration).ceil();
        frame_count.min(end.max(0.0) as usize + 1)
    };
    if lower >= upper {
        return None;
    }
    let join = |slice: &[SoundWord]| {
        slice
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    Some(Window {
        frames: lower..upper,
        before: join(&words[first..range.start]),
        after: join(&words[range.end..=last]),
    })
}

/// The text each side is scored as: the phrase with its neighbours.
pub fn phrase(middle: &str, window: &Window) -> String {
    [window.before.as_str(), middle, window.after.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The words of `spelled` (the CTC model's own transcript) that line up with
/// `range` of `words` (the speech model's): whatever lies between the
/// matching words either side. How that stretch sounded, spelled the CTC
/// model's way.
pub fn spelling(range: &Range<usize>, words: &[SoundWord], spelled: &[String]) -> Option<String> {
    let ours: Vec<String> = words.iter().map(|w| key(&w.text).concat()).collect();
    let theirs: Vec<String> = spelled.iter().map(|w| key(w).concat()).collect();
    if ours.is_empty() || theirs.is_empty() {
        return None;
    }
    // Longest common subsequence, as index pairs.
    let mut table = vec![vec![0usize; theirs.len() + 1]; ours.len() + 1];
    for i in (0..ours.len()).rev() {
        for j in (0..theirs.len()).rev() {
            table[i][j] = if ours[i] == theirs[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < ours.len() && j < theirs.len() {
        if ours[i] == theirs[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let before = pairs
        .iter()
        .rev()
        .find(|p| p.0 < range.start)
        .map(|p| p.1 + 1)
        .unwrap_or(0);
    let after = pairs
        .iter()
        .find(|p| p.0 >= range.end)
        .map(|p| p.1)
        .unwrap_or(theirs.len());
    // Words of the range that matched exactly are part of the answer too.
    if before >= after {
        return None;
    }
    let middle = spelled[before..after].join(" ");
    (!middle.is_empty()).then_some(middle)
}

/// log P(tokens | frames) under CTC, summed over every alignment, with
/// `blank` the blank token's index. -infinity when the frames are too few.
pub fn log_likelihood(
    log_probs: &[Vec<f32>],
    frames: &Range<usize>,
    tokens: &[usize],
    blank: usize,
) -> f32 {
    let neg = f32::NEG_INFINITY;
    if frames.is_empty() || frames.end > log_probs.len() {
        return neg;
    }
    let at = |row: &[f32], label: usize| row.get(label).copied().unwrap_or(neg);
    if tokens.is_empty() {
        return frames
            .clone()
            .map(|frame| at(&log_probs[frame], blank))
            .sum();
    }
    // The extended label sequence: blank, t1, blank, t2, ..., blank.
    let mut labels = Vec::with_capacity(tokens.len() * 2 + 1);
    labels.push(blank);
    for &token in tokens {
        labels.push(token);
        labels.push(blank);
    }
    let width = labels.len();
    let mut alpha = vec![neg; width];
    let first = &log_probs[frames.start];
    alpha[0] = at(first, blank);
    alpha[1] = at(first, labels[1]);
    let mut next = alpha.clone();
    for frame in frames.clone().skip(1) {
        let row = &log_probs[frame];
        for s in 0..width {
            let mut total = alpha[s];
            if s >= 1 {
                total = log_add(total, alpha[s - 1]);
            }
            if s >= 2 && labels[s] != blank && labels[s] != labels[s - 2] {
                total = log_add(total, alpha[s - 2]);
            }
            next[s] = if total == neg {
                neg
            } else {
                total + at(row, labels[s])
            };
        }
        std::mem::swap(&mut alpha, &mut next);
    }
    log_add(alpha[width - 1], alpha[width - 2])
}

fn log_add(a: f32, b: f32) -> f32 {
    if a == f32::NEG_INFINITY {
        return b;
    }
    if b == f32::NEG_INFINITY {
        return a;
    }
    let m = a.max(b);
    m + ((a - m).exp() + (b - m).exp()).ln()
}

/// How much better `term` explains the sound than `heard`, in nats, over the
/// same window: above zero it sounded more like your word.
pub fn margin(
    log_probs: &[Vec<f32>],
    window: &Window,
    heard: &str,
    term: &str,
    blank: usize,
    encode: &dyn Fn(&str) -> Vec<usize>,
) -> Option<f32> {
    let heard_score = log_likelihood(
        log_probs,
        &window.frames,
        &encode(&phrase(heard, window)),
        blank,
    );
    let term_score = log_likelihood(
        log_probs,
        &window.frames,
        &encode(&phrase(term, window)),
        blank,
    );
    (heard_score.is_finite() && term_score.is_finite()).then(|| term_score - heard_score)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Frames from a made-up 4-piece vocabulary (blank = 3): each frame puts
    /// almost all its weight on one piece.
    pub(crate) fn frames(pieces: &[usize]) -> Vec<Vec<f32>> {
        pieces
            .iter()
            .map(|&piece| {
                (0..4)
                    .map(|i| {
                        if i == piece {
                            0.97f32.ln()
                        } else {
                            0.01f32.ln()
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn plain(words: &[&str]) -> Vec<SoundWord> {
        words.iter().map(|w| SoundWord::new(*w, 0.0, 0.0)).collect()
    }

    #[test]
    fn ctc_prefers_the_sequence_that_was_said() {
        // Said: 0 0 blank 1 1, i.e. "a b".
        let lp = frames(&[0, 0, 3, 1, 1]);
        let said = log_likelihood(&lp, &(0..5), &[0, 1], 3);
        let shorter = log_likelihood(&lp, &(0..5), &[0], 3);
        let longer = log_likelihood(&lp, &(0..5), &[0, 1, 2], 3);
        assert!(said > shorter + 3.0);
        assert!(said > longer + 3.0);
    }

    #[test]
    fn repeated_pieces_need_a_blank_between() {
        // "a a" can't be said in two frames with no blank between them.
        let lp = frames(&[0, 0]);
        assert_eq!(log_likelihood(&lp, &(0..2), &[0, 0], 3), f32::NEG_INFINITY);
        assert!(log_likelihood(&frames(&[0, 3, 0]), &(0..3), &[0, 0], 3).is_finite());
    }

    #[test]
    fn out_of_range_frames_and_tokens_are_impossible_not_a_panic() {
        let lp = frames(&[0, 0]);
        assert_eq!(log_likelihood(&lp, &(0..9), &[0], 3), f32::NEG_INFINITY);
        assert_eq!(log_likelihood(&lp, &(0..0), &[0], 3), f32::NEG_INFINITY);
        assert_eq!(log_likelihood(&lp, &(0..2), &[77], 3), f32::NEG_INFINITY);
        assert!(log_likelihood(&lp, &(0..2), &[], 3).is_finite());
    }

    #[test]
    fn margin_is_positive_when_the_term_fits_the_sound() {
        // Pieces: 0 = "email", 1 = "er", 2 = "and"; the sound is "email er and".
        let lp = frames(&[0, 0, 1, 3, 2]);
        let words = vec![
            SoundWord::new("email", 0.0, 0.2),
            SoundWord::new("and", 0.3, 0.4),
        ];
        let encode = |text: &str| -> Vec<usize> {
            text.split(' ')
                .flat_map(|word| match word {
                    "email" => vec![0],
                    "emailer" => vec![0, 1],
                    "and" => vec![2],
                    _ => vec![],
                })
                .collect()
        };
        let window = window(&(0..1), &words, 0.08, lp.len()).expect("window");
        let m = margin(&lp, &window, "email", "emailer", 3, &encode).unwrap();
        assert!(m > 3.0, "{m}");
        // And the other way: the sound is plain "email and".
        let plain = frames(&[0, 0, 3, 3, 2]);
        let other = margin(&plain, &window, "email", "emailer", 3, &encode).unwrap();
        assert!(other < -3.0, "{other}");
    }

    #[test]
    fn words_come_from_token_timings() {
        let words = SoundWord::from_tokens([
            ("▁Can", 0.0, 0.1),
            ("▁open", 0.2, 0.3),
            ("▁e", 0.4, 0.5),
            ("mail", 0.5, 0.7),
            ("er", 0.7, 0.8),
            ("▁and", 0.9, 1.0),
        ]);
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Can", "open", "emailer", "and"]);
        assert!(words[2].start == 0.4 && words[2].end == 0.8);
    }

    #[test]
    fn occurrences_match_whole_words_any_case() {
        let words = plain(&["I", "sent", "the", "Email,", "then", "an", "email"]);
        assert_eq!(occurrences("email", &words), vec![3..4, 6..7]);
        assert_eq!(occurrences("an email", &words), vec![5..7]);
        assert!(occurrences("emails", &words).is_empty());
        let hyphen = plain(&["E-mail", "is", "ready"]);
        assert_eq!(occurrences("email", &hyphen), vec![0..1]);
    }

    #[test]
    fn spelling_lines_up_the_ctc_transcript() {
        let words = plain(&["Can", "you", "open", "email", "and", "check"]);
        let spelled: Vec<String> = ["can", "you", "open", "emmaler", "and", "check"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            spelling(&(3..4), &words, &spelled).as_deref(),
            Some("emmaler")
        );
    }

    #[test]
    fn the_window_is_the_whole_recording_when_context_reaches_both_ends() {
        let words: Vec<SoundWord> = (0..10)
            .map(|i| SoundWord::new(format!("w{i}"), i as f64, i as f64 + 0.5))
            .collect();
        // A short clip: all of it.
        let all = window(&(1..2), &words[..4], 0.08, 60).unwrap();
        assert_eq!(all.frames, 0..60);
        assert_eq!(all.before, "w0");
        assert_eq!(all.after, "w2 w3");
        // In the middle of a long one: three words and 0.4 s either side.
        let mid = window(&(5..6), &words, 0.08, 200).unwrap();
        assert_eq!(mid.before, "w2 w3 w4");
        assert_eq!(mid.after, "w6 w7 w8");
        assert_eq!(mid.frames.start, (1.6f64 / 0.08).floor() as usize);
        assert_eq!(mid.frames.end, (8.9f64 / 0.08).ceil() as usize + 1);
        assert!(window(&(5..5), &words, 0.08, 200).is_none());
        assert!(window(&(5..11), &words, 0.08, 200).is_none());
        assert!(window(&(5..6), &words, 0.0, 200).is_none());
    }
}
