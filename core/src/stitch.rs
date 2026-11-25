//! Joining overlapping decodes of one recording (Mac `stitchedExpressText`,
//! round 4 of its live-segments evaluation, kept here with token timings).
//!
//! While the speaker talks, Express decodes the recording a segment at a
//! time, each with 3 s of audio before it and 2 s after. Neighbouring
//! decodes therefore hear the same words where they overlap. The seam is the
//! longest run of words both decoded there (at least two, ignoring case and
//! punctuation), split in its middle: the earlier decode supplies the words
//! before it and the later one the words after, so the words each decode
//! heard nearest its own edge, with the least context, are dropped. On the
//! Mac this matched one whole-recording decode (49 of 50 real-voice
//! dictations identical).
//!
//! Unlike the Mac, which joined text, the pieces here keep each token's time
//! (seconds from the recording's start), so Word Boost and the sound anchors
//! can run once, over the whole recording, after the last piece is joined.
//! (On the Mac, Word Boost ran inside each piece and lost two dictionary
//! names at the seams, which is why it kept segments off with Word Boost.)

use crate::transcription::RawDecode;

/// Words searched for a seam at each end of an overlap: 5 s of audio holds
/// about 15 spoken words (Mac `expressStitchSearchWordCount`).
pub const SEARCH_WORDS: usize = 30;

/// Whether a recognizer token starts a word (SentencePiece `▁`, or a space
/// as sherpa-onnx writes it), as Word Boost reads them.
fn starts_word(token: &str) -> bool {
    token.starts_with('\u{2581}') || token.starts_with(' ')
}

/// One word: the range of its tokens and its comparison key.
struct Word {
    tokens: std::ops::Range<usize>,
    text: String,
    key: String,
}

fn words(raw: &RawDecode) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    for (i, token) in raw.tokens.iter().enumerate() {
        if token.is_empty() || token == "<blank>" || token == "<pad>" {
            continue;
        }
        let piece = token
            .strip_prefix('\u{2581}')
            .or_else(|| token.strip_prefix(' '))
            .unwrap_or(token);
        match out.last_mut() {
            Some(w) if !starts_word(token) => {
                w.tokens.end = i + 1;
                w.text.push_str(piece);
            }
            _ => out.push(Word {
                tokens: i..i + 1,
                text: piece.to_string(),
                key: String::new(),
            }),
        }
    }
    out.retain(|w| !w.text.trim().is_empty());
    for w in &mut out {
        w.text = w.text.trim().to_string();
        w.key = w
            .text
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
    }
    out
}

/// A decode rebuilt from its own tokens, so its text holds exactly the words
/// Word Boost rebuilds from them.
fn from_words(parts: &[(&RawDecode, &[Word])]) -> RawDecode {
    let mut out = RawDecode::default();
    let mut texts = Vec::new();
    for (raw, words) in parts {
        for w in words.iter() {
            texts.push(w.text.clone());
            for i in w.tokens.clone() {
                out.tokens.push(raw.tokens[i].clone());
                out.starts.push(raw.starts.get(i).copied().unwrap_or(0.0));
                out.durations
                    .push(raw.durations.get(i).copied().unwrap_or(0.0));
            }
        }
    }
    out.text = texts.join(" ");
    out
}

/// Whether a decode can be stitched: one start time per token, and tokens
/// that account for its words.
pub fn stitchable(raw: &RawDecode) -> bool {
    raw.starts.len() == raw.tokens.len() && (raw.text.trim().is_empty() || !words(raw).is_empty())
}

/// The decode rebuilt from its tokens (the form [`stitch`] returns), for a
/// recording that has only one piece.
pub fn normalized(raw: &RawDecode) -> RawDecode {
    let w = words(raw);
    from_words(&[(raw, &w)])
}

/// `earlier` (the recording so far) joined to `later`, the decode of the
/// next, overlapping window, both timed from the recording's start. `None`
/// when they don't agree anywhere in the overlap.
pub fn stitch(earlier: &RawDecode, later: &RawDecode) -> Option<RawDecode> {
    let left = words(earlier);
    let right = words(later);
    if left.is_empty() {
        return Some(from_words(&[(later, &right)]));
    }
    let left_offset = left.len().saturating_sub(SEARCH_WORDS);
    let left_keys: Vec<&str> = left[left_offset..].iter().map(|w| w.key.as_str()).collect();
    let right_keys: Vec<&str> = right
        .iter()
        .take(SEARCH_WORDS)
        .map(|w| w.key.as_str())
        .collect();
    // Longest common run of words, by the usual one-row dynamic program; on
    // a tie, the run ending later in the earlier text.
    let mut best: Option<(usize, usize, usize)> = None; // (left end, right end, length)
    let mut runs = vec![0usize; right_keys.len() + 1];
    for (i, lk) in left_keys.iter().enumerate() {
        let mut diagonal = 0;
        for (j, rk) in right_keys.iter().enumerate() {
            let above = runs[j + 1];
            runs[j + 1] = if !lk.is_empty() && lk == rk {
                diagonal + 1
            } else {
                0
            };
            diagonal = above;
            if runs[j + 1] >= best.map_or(0, |b| b.2).max(2) {
                best = Some((left_offset + i + 1, j + 1, runs[j + 1]));
            }
        }
    }
    let (left_end, right_end, length) = best?;
    let half = length / 2;
    let left_cut = left_end - length + half;
    let right_cut = right_end - length + half;
    Some(from_words(&[
        (earlier, &left[..left_cut]),
        (later, &right[right_cut..]),
    ]))
}

/// `raw`, decoded from audio starting `offset` seconds into the recording,
/// timed from the recording's start.
pub fn shifted(mut raw: RawDecode, offset: f32) -> RawDecode {
    for s in &mut raw.starts {
        *s += offset;
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A decode of `text`, one token per word (plus one for a trailing
    /// punctuation mark), a word every 0.3 s from `start`.
    fn decode(text: &str, start: f32) -> RawDecode {
        let mut raw = RawDecode {
            text: text.to_string(),
            ..Default::default()
        };
        let mut t = start;
        for word in text.split_whitespace() {
            let (body, mark) = match word.strip_suffix(['.', ',']) {
                Some(b) => (b, Some(&word[b.len()..])),
                None => (word, None),
            };
            raw.tokens.push(format!("\u{2581}{body}"));
            raw.starts.push(t);
            raw.durations.push(0.2);
            if let Some(m) = mark {
                raw.tokens.push(m.to_string());
                raw.starts.push(t + 0.2);
                raw.durations.push(0.05);
            }
            t += 0.3;
        }
        raw
    }

    #[test]
    fn joins_in_the_middle_of_the_agreed_words() {
        let a = decode("so the plan for today is to ship the build", 0.0);
        let b = decode("ship the build tonight and test it", 2.4);
        let joined = stitch(&a, &b).unwrap();
        assert_eq!(
            joined.text,
            "so the plan for today is to ship the build tonight and test it"
        );
        // "ship the build": 3 agreed words, cut after "ship" (half = 1). The
        // earlier decode's "ship" and the later one's "the build ...".
        let the = joined
            .tokens
            .iter()
            .position(|t| t == "\u{2581}the")
            .unwrap();
        assert_eq!(
            joined.starts[the], 0.3,
            "the first 'the' is the earlier decode's"
        );
        let build = joined
            .tokens
            .iter()
            .position(|t| t == "\u{2581}build")
            .unwrap();
        assert!(
            (joined.starts[build] - 3.0).abs() < 1e-6,
            "'build' is the later decode's"
        );
        assert_eq!(joined.tokens.len(), joined.starts.len());
    }

    #[test]
    fn each_side_drops_its_words_nearest_the_edge() {
        // The earlier decode misheard its last word, the later its first:
        // both are outside the seam and dropped.
        let a = decode("we met the team at noon to talk shop", 0.0);
        let b = decode("hmm at noon to talk shop and lunch", 1.5);
        let joined = stitch(&a, &b).unwrap();
        assert_eq!(
            joined.text,
            "we met the team at noon to talk shop and lunch"
        );
    }

    #[test]
    fn case_and_punctuation_dont_break_agreement() {
        let a = decode("Call me tomorrow. Bye", 0.0);
        let b = decode("call me tomorrow, bye now", 0.0);
        let joined = stitch(&a, &b).unwrap();
        assert_eq!(joined.text, "Call me tomorrow, bye now");
    }

    #[test]
    fn no_agreement_is_no_stitch() {
        let a = decode("the first part of it", 0.0);
        let b = decode("something else entirely", 1.0);
        assert!(stitch(&a, &b).is_none());
        // One shared word isn't a seam.
        let c = decode("it ends here", 1.0);
        assert!(stitch(&a, &c).is_none());
    }

    #[test]
    fn the_first_piece_is_taken_whole() {
        let b = decode("hello there.", 0.0);
        let joined = stitch(&RawDecode::default(), &b).unwrap();
        assert_eq!(joined.text, "hello there.");
        assert_eq!(normalized(&b).text, "hello there.");
    }

    #[test]
    fn a_repeated_phrase_seams_at_the_latest_match() {
        let a = decode("go on go on and on", 0.0);
        let b = decode("go on and on forever", 0.9);
        let joined = stitch(&a, &b).unwrap();
        assert_eq!(joined.text, "go on go on and on forever");
    }

    #[test]
    fn subword_tokens_make_one_word() {
        let raw = RawDecode {
            text: "Orttaai works".into(),
            tokens: vec![
                "\u{2581}Ort".into(),
                "ta".into(),
                "ai".into(),
                " works".into(),
            ],
            starts: vec![0.0, 0.1, 0.2, 0.5],
            durations: vec![0.1; 4],
        };
        assert!(stitchable(&raw));
        assert_eq!(normalized(&raw).text, "Orttaai works");
        let later = RawDecode {
            text: "Orttaai works well".into(),
            tokens: vec![
                "\u{2581}Ort".into(),
                "ta".into(),
                "ai".into(),
                "\u{2581}works".into(),
                "\u{2581}well".into(),
            ],
            starts: vec![0.0, 0.1, 0.2, 0.5, 0.8],
            durations: vec![0.1; 5],
        };
        assert_eq!(stitch(&raw, &later).unwrap().text, "Orttaai works well");
    }

    #[test]
    fn shifting_moves_every_start() {
        let raw = shifted(decode("a b", 0.0), 2.0);
        assert_eq!(raw.starts, vec![2.0, 2.3]);
    }

    #[test]
    fn missing_times_cant_be_stitched() {
        let mut raw = decode("a b c", 0.0);
        raw.starts.pop();
        assert!(!stitchable(&raw));
        let text_only = RawDecode {
            text: "words".into(),
            ..Default::default()
        };
        assert!(!stitchable(&text_only));
        assert!(stitchable(&RawDecode::default()));
    }
}
