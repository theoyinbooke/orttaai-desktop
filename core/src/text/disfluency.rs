//! Deterministic cleanup of what Whisper transcribes verbatim (Mac
//! `DisfluencyCleaner`): filler words, stuttered function words, restarted
//! phrases and a lowercase "i". English only, and deliberately narrow: every
//! rule has a closed list or a guard so real repetition ("no no no", "had
//! had") and code ("for i in range") survive.
//!
//! The Mac version is written with ICU regexes (look-behind and back
//! references); this is a hand-written scanner with the same rules.

use super::chars::{is_blank, is_newline, is_word_char};

const FILLERS: &[&str] = &["um", "uhm", "umm", "erm", "uh"];
/// Words that are never legitimately doubled in speech. "that", "had", "is",
/// "very", "no" and "you" are excluded on purpose.
const DOUBLED_WORDS: &[&str] = &["the", "to", "and", "a", "of", "in", "for"];
/// Phrases that start or end with one of these are left alone, so "over and
/// over and over" and "step by step by step" keep their meaning.
const CONNECTIVES: &[&str] = &["and", "or", "by", "after", "to", "upon", "than"];
/// "section i", "chapter i" and friends: a roman numeral, not a pronoun.
const NUMERAL_INTRODUCERS: &[&str] = &[
    "section", "chapter", "part", "item", "appendix", "article", "figure", "table", "volume",
    "page", "clause", "book", "act", "scene", "level", "tier",
];
/// A bare "i" right after one of these names the letter (the i icon, a
/// variable i), never the pronoun.
const LETTER_INTRODUCERS: &[&str] = &[
    "the", "a", "an", "variable", "letter", "index", "counter", "symbol", "int", "var",
];

/// Result of a cleanup pass: the text and human-readable change notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    pub text: String,
    pub changes: Vec<String>,
}

pub fn clean(text: &str) -> Cleaned {
    let (without_fillers, filler_count) = remove_fillers(text);
    // A lone "Um." would otherwise paste as nothing at all.
    if without_fillers.trim().is_empty() {
        return Cleaned {
            text: text.to_string(),
            changes: Vec::new(),
        };
    }
    let mut changes = Vec::new();
    if filler_count > 0 {
        changes.push(format!(
            "Disfluency: removed {filler_count} filler{}",
            plural(filler_count)
        ));
    }
    let (value, repeated) = collapse_until_stable(&without_fillers, repeated_word_at);
    if repeated > 0 {
        changes.push(format!(
            "Disfluency: collapsed {repeated} repeated word{}",
            plural(repeated)
        ));
    }
    let (value, phrases) = collapse_until_stable(&value, repeated_phrase_at);
    if phrases > 0 {
        changes.push(format!(
            "Disfluency: collapsed {phrases} repeated phrase{}",
            plural(phrases)
        ));
    }
    let (value, capitalized) = capitalize_pronoun_i(&value);
    if capitalized > 0 {
        changes.push(format!("Disfluency: capitalized {capitalized} 'i'"));
    }
    Cleaned {
        text: value,
        changes,
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn lower_eq(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Case-insensitive match of `word` at `at`; returns the end index.
fn match_word(text: &[char], at: usize, word: &str) -> Option<usize> {
    let mut i = at;
    for w in word.chars() {
        if i >= text.len() || !lower_eq(text[i], w) {
            return None;
        }
        i += 1;
    }
    Some(i)
}

// ---- Fillers -----------------------------------------------------------------

/// Filler occurrences: a standalone spoken word, delimited on the left by
/// whitespace or the start, and on the right by whitespace, the end, or
/// sentence punctuation itself followed by whitespace or the end. That keeps
/// "um@x.com", "um.txt", "file.um", "um: hello", "\"um\"" and "(um)" intact.
fn filler_matches(text: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if i > 0 && !text[i - 1].is_whitespace() {
            i += 1;
            continue;
        }
        let mut found = None;
        for filler in FILLERS {
            let Some(end) = match_word(text, i, filler) else {
                continue;
            };
            let mut j = end;
            while j < text.len() && ",.?!;…".contains(text[j]) {
                j += 1;
            }
            if j < text.len() && !text[j].is_whitespace() {
                continue;
            }
            // "uh huh" and "uh oh" are words, not hesitations.
            if *filler == "uh" {
                let mut k = end;
                while k < text.len() && is_blank(text[k]) {
                    k += 1;
                }
                if k > end
                    && ["huh", "oh"].iter().any(|w| {
                        match_word(text, k, w)
                            .is_some_and(|e| e >= text.len() || !is_word_char(text[e]))
                    })
                {
                    continue;
                }
            }
            found = Some(end);
            break;
        }
        match found {
            Some(end) => {
                out.push((i, end));
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

/// One pass over the fillers, left to right. Each removal drops only the
/// filler, the commas Whisper wraps it in and the spaces adjoining it;
/// newlines, indentation and everything else in between is copied as is.
fn remove_fillers(source: &str) -> (String, usize) {
    let text: Vec<char> = source.chars().collect();
    let mut output: Vec<char> = Vec::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut count = 0usize;
    // Set when a filler opened a sentence: the next word kept is recapitalized.
    let mut capitalize_next = false;

    for (start, end) in filler_matches(&text) {
        if start < cursor {
            continue;
        }
        let filler = &text[start..end];
        // "UM" is an initialism, not a hesitation.
        if filler.iter().all(|c| !c.is_lowercase()) {
            continue;
        }
        let kept = &text[cursor..start];
        if capitalize_next && !kept.is_empty() {
            output.extend(capitalized_first_letter(kept));
            capitalize_next = false;
        } else {
            output.extend_from_slice(kept);
        }

        let left_gap = remove_trailing_blanks(&mut output);
        if output.last() == Some(&',') {
            output.pop();
            remove_trailing_blanks(&mut output);
        }
        let has_text_before = !output.is_empty();
        let at_line_start = output.last().is_none_or(|c| is_newline(*c));
        let ends_sentence = at_line_start || output.last().is_some_and(|c| ".!?…".contains(*c));
        let ends_with_ellipsis = output.last() == Some(&'…') || output.ends_with(&['.', '.', '.']);

        cursor = skip_blanks(&text, end);
        let right_gap_end = cursor;
        if cursor < text.len() && text[cursor] == ',' {
            cursor = skip_blanks(&text, cursor + 1);
        }
        // "Um." on its own is a whole sentence of hesitation; so is the "..."
        // trailing a filler that follows an ellipsis.
        if ends_sentence {
            while cursor < text.len() && ".!?…".contains(text[cursor]) {
                cursor += 1;
            }
            cursor = skip_blanks(&text, cursor);
        }

        if at_line_start {
            output.extend(left_gap);
        } else if cursor < text.len()
            && !is_newline(text[cursor])
            && !",.;:!?)".contains(text[cursor])
        {
            if left_gap.is_empty() {
                output.extend_from_slice(&text[end..right_gap_end]);
            } else {
                output.extend(left_gap);
            }
        }

        let filler_is_capitalized = filler[0].is_uppercase();
        // Only . ? ! and a line start open a sentence, never an ellipsis.
        if ends_sentence && !ends_with_ellipsis && (filler_is_capitalized || has_text_before) {
            capitalize_next = true;
        }
        count += 1;
    }

    let tail = &text[cursor..];
    if capitalize_next {
        output.extend(capitalized_first_letter(tail));
    } else {
        output.extend_from_slice(tail);
    }
    (output.into_iter().collect(), count)
}

/// Removes and returns the spaces and tabs at the end of `text`.
fn remove_trailing_blanks(text: &mut Vec<char>) -> Vec<char> {
    let mut removed = Vec::new();
    while let Some(&last) = text.last() {
        if !is_blank(last) {
            break;
        }
        removed.insert(0, last);
        text.pop();
    }
    removed
}

fn skip_blanks(text: &[char], mut index: usize) -> usize {
    while index < text.len() && is_blank(text[index]) {
        index += 1;
    }
    index
}

/// Leaves mixed-case tokens such as "iPhone" alone.
fn capitalized_first_letter(text: &[char]) -> Vec<char> {
    let Some(&first) = text.first() else {
        return Vec::new();
    };
    if !first.is_lowercase() || text.get(1).is_some_and(|c| c.is_uppercase()) {
        return text.to_vec();
    }
    let mut out: Vec<char> = first.to_uppercase().collect();
    out.extend_from_slice(&text[1..]);
    out
}

// ---- Repeated words and phrases ----------------------------------------------

/// Not preceded by anything that makes it part of another word ("um-brella",
/// "don't").
fn boundary_before(text: &[char], at: usize) -> bool {
    at == 0 || {
        let c = text[at - 1];
        !(is_word_char(c) || c == '\'' || c == '\u{2019}' || c == '-')
    }
}

fn boundary_after(text: &[char], at: usize) -> bool {
    let Some(&c) = text.get(at) else {
        return true;
    };
    if is_word_char(c) || c == '\'' || c == '\u{2019}' {
        return false;
    }
    !(c == '-' && text.get(at + 1).is_some_and(|n| n.is_alphanumeric()))
}

/// After a repeated unit ending at `at`: an optional comma (with blanks before
/// it), then at least one blank. Returns where the repeat must start.
fn repeat_starts(text: &[char], at: usize) -> Vec<usize> {
    let mut starts = Vec::with_capacity(2);
    let mut k = at;
    while k < text.len() && is_blank(text[k]) {
        k += 1;
    }
    if k < text.len() && text[k] == ',' {
        let after_comma = k + 1;
        let s = skip_blanks(text, after_comma);
        if s > after_comma {
            starts.push(s);
        }
    }
    let s = skip_blanks(text, at);
    if s > at {
        starts.push(s);
    }
    starts
}

/// Case-insensitive equality of `text[at..]` with `unit`.
fn matches_at(text: &[char], at: usize, unit: &[char]) -> Option<usize> {
    if at + unit.len() > text.len() {
        return None;
    }
    unit.iter()
        .zip(&text[at..at + unit.len()])
        .all(|(a, b)| lower_eq(*a, *b))
        .then_some(at + unit.len())
}

/// A stuttered function word at `at`: returns (match end, kept unit end).
fn repeated_word_at(text: &[char], at: usize) -> Option<(usize, usize)> {
    if !boundary_before(text, at) {
        return None;
    }
    for word in DOUBLED_WORDS {
        let Some(end) = match_word(text, at, word) else {
            continue;
        };
        let unit = &text[at..end];
        for start in repeat_starts(text, end) {
            if let Some(repeat_end) = matches_at(text, start, unit) {
                if boundary_after(text, repeat_end) {
                    return Some((repeat_end, end));
                }
            }
        }
    }
    None
}

fn is_phrase_letter(c: char) -> bool {
    c.is_alphabetic() || c == '\'' || c == '\u{2019}'
}

fn starts_with_connective(text: &[char], at: usize) -> bool {
    CONNECTIVES
        .iter()
        .any(|c| match_word(text, at, c).is_some_and(|e| e >= text.len() || !is_word_char(text[e])))
}

/// A two-to-four word phrase said twice in a row when the speaker restarts
/// ("we don't we don't need them").
fn repeated_phrase_at(text: &[char], at: usize) -> Option<(usize, usize)> {
    if !boundary_before(text, at) || at >= text.len() || !is_phrase_letter(text[at]) {
        return None;
    }
    // Word spans separated only by blanks, at most four.
    let mut words: Vec<(usize, usize)> = Vec::with_capacity(4);
    let mut i = at;
    while words.len() < 4 {
        let start = i;
        while i < text.len() && is_phrase_letter(text[i]) {
            i += 1;
        }
        if i == start {
            break;
        }
        words.push((start, i));
        let after = skip_blanks(text, i);
        if after == i || after >= text.len() || !is_phrase_letter(text[after]) {
            break;
        }
        i = after;
    }
    if starts_with_connective(text, at) {
        return None;
    }
    for n in (2..=words.len()).rev() {
        let (last_start, end) = words[n - 1];
        if starts_with_connective(text, last_start) {
            continue;
        }
        let unit = &text[at..end];
        for start in repeat_starts(text, end) {
            if let Some(repeat_end) = matches_at(text, start, unit) {
                if boundary_after(text, repeat_end) {
                    return Some((repeat_end, end));
                }
            }
        }
    }
    None
}

/// Finds a repeat at a position: (match end, kept unit end).
type RepeatFinder = fn(&[char], usize) -> Option<(usize, usize)>;

/// Replaces every non-overlapping match (left to right) with its first unit,
/// repeating until nothing matches — a triple ("the the the") needs a second
/// pass.
fn collapse_until_stable(source: &str, find: RepeatFinder) -> (String, usize) {
    let mut text: Vec<char> = source.chars().collect();
    let mut total = 0usize;
    loop {
        let mut out: Vec<char> = Vec::with_capacity(text.len());
        let mut count = 0usize;
        let mut i = 0usize;
        while i < text.len() {
            if let Some((match_end, keep_end)) = find(&text, i) {
                out.extend_from_slice(&text[i..keep_end]);
                i = match_end;
                count += 1;
            } else {
                out.push(text[i]);
                i += 1;
            }
        }
        if count == 0 {
            break;
        }
        total += count;
        text = out;
    }
    (text.into_iter().collect(), total)
}

// ---- Pronoun I ---------------------------------------------------------------

fn i_boundary_before(text: &[char], at: usize) -> bool {
    at == 0 || {
        let c = text[at - 1];
        !(is_word_char(c) || "'\u{2019}.-/@#$".contains(c))
    }
}

fn capitalize_pronoun_i(source: &str) -> (String, usize) {
    let mut text: Vec<char> = source.chars().collect();
    let mut positions = Vec::new();
    for at in 0..text.len() {
        if text[at] != 'i' || !i_boundary_before(&text, at) {
            continue;
        }
        let next = text.get(at + 1).copied();
        let is_contraction = next.is_some_and(|c| c == '\'' || c == '\u{2019}')
            && ["m", "ll", "ve", "d"].iter().any(|suffix| {
                match_exact(&text, at + 2, suffix)
                    .is_some_and(|e| e >= text.len() || !is_word_char(text[e]))
            });
        if is_contraction {
            positions.push(at);
            continue;
        }
        let bare = next.is_none_or(|c| !(is_word_char(c) || c == '\'' || c == '\u{2019}'));
        if bare && is_pronoun(&text, at) {
            positions.push(at);
        }
    }
    for &p in &positions {
        text[p] = 'I';
    }
    (text.into_iter().collect(), positions.len())
}

fn match_exact(text: &[char], at: usize, word: &str) -> Option<usize> {
    let mut i = at;
    for w in word.chars() {
        if text.get(i) != Some(&w) {
            return None;
        }
        i += 1;
    }
    Some(i)
}

/// Rejects a bare "i" that is a list marker, roman numeral, loop counter or a
/// member of a run of one-letter names ("i, j, k").
fn is_pronoun(text: &[char], at: usize) -> bool {
    let before = &text[..at];
    let after = &text[at + 1..];
    if after.first() == Some(&'.') {
        return false;
    }
    let spaces = after.iter().take_while(|c| is_blank(**c)).count();
    let after_spaces = &after[spaces..];
    if let Some(&next) = after_spaces.first() {
        if "=<>+*/%&|^~[](){}:-".contains(next) {
            return false;
        }
        if next == '!' && after_spaces.get(1) == Some(&'=') {
            return false;
        }
    }
    let previous_word = word_ending_at(before).to_lowercase();
    let skip = after_spaces
        .iter()
        .take_while(|c| **c == ',' || is_blank(**c))
        .count();
    let next_word = word_starting_at(&after_spaces[skip..]).to_lowercase();
    if NUMERAL_INTRODUCERS.contains(&previous_word.as_str())
        || LETTER_INTRODUCERS.contains(&previous_word.as_str())
    {
        return false;
    }
    if previous_word == "for" && next_word == "in" {
        return false;
    }
    ![previous_word, next_word]
        .iter()
        .any(|w| is_single_letter_name_or_numeral(w))
}

fn is_single_letter_name_or_numeral(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() == 1 && word != "a" && word != "i" && chars[0].is_alphabetic() {
        return true;
    }
    chars.len() > 1 && chars.iter().all(|c| "ivx".contains(*c))
}

fn word_ending_at(text: &[char]) -> String {
    let mut end = text.len();
    while end > 0 && (is_blank(text[end - 1]) || text[end - 1] == ',') {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && text[start - 1].is_alphanumeric() {
        start -= 1;
    }
    text[start..end].iter().collect()
}

fn word_starting_at(text: &[char]) -> String {
    text.iter().take_while(|c| c.is_alphanumeric()).collect()
}

#[cfg(test)]
mod tests {
    use super::clean;

    fn cleaned(s: &str) -> String {
        clean(s).text
    }

    #[test]
    fn collapses_restarted_phrases() {
        for (input, expected) in [
            ("We don't we don't need them.", "We don't need them."),
            ("we don't, we don't need them", "we don't need them"),
            ("I think I think it works", "I think it works"),
            (
                "it should be it should be pure voice",
                "it should be pure voice",
            ),
            ("we don't we don't we don't", "we don't"),
        ] {
            assert_eq!(cleaned(input), expected, "{input}");
        }
    }

    #[test]
    fn keeps_deliberate_repetition() {
        for input in [
            "I tried over and over and over again.",
            "Do it step by step by step.",
            "It got better day after day after day.",
            "We had had enough.",
            "no no no, not that one",
        ] {
            assert_eq!(cleaned(input), input);
        }
    }

    #[test]
    fn cleans_fillers_stutters_and_pronoun_i() {
        for (input, expected) in [
            ("And, um, remember", "And remember"),
            ("so um um to avoid", "so to avoid"),
            ("and And that", "and that"),
            ("the the memory", "the memory"),
            ("i'm sure", "I'm sure"),
            ("Um, I think so", "I think so"),
            ("Um, i think so", "I think so"),
            ("Well um, yes", "Well yes"),
            ("I went, uh the store", "I went the store"),
            ("It was fine, uhm.", "It was fine."),
            ("Okay. Um. Then we ship.", "Okay. Then we ship."),
            ("Okay. Uh, then we ship.", "Okay. Then we ship."),
            ("okay. umm then we ship.", "okay. Then we ship."),
            ("we should erm go", "we should go"),
            ("Um... the plan", "The plan"),
            ("so uh iPhone sales", "so iPhone sales"),
            ("Um, iPhone sales", "iPhone sales"),
            ("to to avoid it", "to avoid it"),
            ("the, the memory", "the memory"),
            (
                "of of the plan and and the rest",
                "of the plan and the rest",
            ),
            ("in in the box and a a test", "in the box and a test"),
            ("for for you", "for you"),
            ("the the the end", "the end"),
            ("The the plan", "The plan"),
            (
                "i think i'll go and i've seen i'd say",
                "I think I'll go and I've seen I'd say",
            ),
            ("so do i, really", "so do I, really"),
            ("i\u{2019}m sure", "I\u{2019}m sure"),
            ("then i said no", "then I said no"),
        ] {
            assert_eq!(cleaned(input), expected, "input: {input}");
        }
    }

    #[test]
    fn leaves_legitimate_text_unchanged() {
        for input in [
            "no no no",
            "very very very",
            "that that's",
            "UM",
            "The UM protocol",
            "We should UH go",
            "i.e. this",
            "for i in range(10)",
            "had had",
            "the um-brella",
            "the uh-oh moment",
            "uh huh, sounds good",
            "to-do list",
            "a to-to plan",
            "section i.",
            "section i and section ii",
            "i++",
            "while i < 10",
            "let i = 0",
            "i, j, k",
            "i j",
            "x[i] is set",
            "the term is fine",
            "you you are great",
            "it it works",
            "Vitamin A and a plan",
            "the theory of the thing",
            "Um.",
            "um",
        ] {
            let r = clean(input);
            assert_eq!(r.text, input, "input: {input}");
            assert!(r.changes.is_empty(), "input: {input}");
        }
    }

    #[test]
    fn reports_what_it_changed() {
        let r = clean("Well, um, i think the the plan, uh, works");
        assert_eq!(r.text, "Well I think the plan works");
        assert_eq!(
            r.changes,
            vec![
                "Disfluency: removed 2 fillers",
                "Disfluency: collapsed 1 repeated word",
                "Disfluency: capitalized 1 'i'"
            ]
        );
    }

    #[test]
    fn multiline_text_keeps_its_line_breaks() {
        assert_eq!(
            cleaned("First line.\nUm, second line."),
            "First line.\nSecond line."
        );
    }

    #[test]
    fn fillers_inside_emails_filenames_urls_and_quotes_are_kept() {
        for input in [
            "email me at um@x.com",
            "um.txt",
            "open file.um now",
            "um: hello",
            "\"um\"",
            "(um)",
            "[uh] marker",
            "see http://x.com/um/ok",
            "the path is /usr/um",
            "use um_value here",
            "call me@uh.org",
        ] {
            let r = clean(input);
            assert_eq!(r.text, input, "input: {input}");
            assert!(r.changes.is_empty(), "input: {input}");
        }
    }

    #[test]
    fn fillers_delimited_by_sentence_punctuation_are_removed() {
        for (input, expected) in [
            ("Well, um; yes", "Well; yes"),
            ("Is it um? Yes", "Is it? Yes"),
            ("Wow um! Great", "Wow! Great"),
            ("So, uh. We go", "So. We go"),
            ("email um then send", "email then send"),
        ] {
            assert_eq!(cleaned(input), expected, "input: {input}");
        }
    }

    #[test]
    fn filler_removal_keeps_newlines_indentation_tabs_and_list_markers() {
        for (input, expected) in [
            ("Hello um\nWorld", "Hello\nWorld"),
            ("Hello um  \nWorld", "Hello\nWorld"),
            ("Hello, um,\nWorld", "Hello\nWorld"),
            ("Hello\num", "Hello\n"),
            ("Hello\r\nx um\r\ny", "Hello\r\nx\r\ny"),
            ("um\nWorld", "\nWorld"),
            ("First.\n\nUm, second.", "First.\n\nSecond."),
            ("a\tum\tb", "a\tb"),
            ("\tum foo", "\tfoo"),
            ("  Um, i think", "  I think"),
            ("- um item\n- uh other", "- item\n- other"),
            ("1. um, the plan\n2. the rest", "1. The plan\n2. the rest"),
            ("keep  double  spaces um here", "keep  double  spaces here"),
            ("a\n\n\nb um c", "a\n\n\nb c"),
        ] {
            assert_eq!(cleaned(input), expected, "input: {input:?}");
        }
    }

    #[test]
    fn ellipsis_never_opens_a_sentence() {
        for (input, expected) in [
            (
                "we can have... um email address",
                "we can have... email address",
            ),
            ("wait... um... okay", "wait... okay"),
            ("wait\u{2026} um okay", "wait\u{2026} okay"),
            ("hold on... uh, then go", "hold on... then go"),
            ("So um... yes", "So... yes"),
            ("we can. um email address", "we can. Email address"),
            ("really? uh so yes", "really? So yes"),
        ] {
            assert_eq!(cleaned(input), expected, "input: {input}");
        }
    }

    #[test]
    fn long_dictation_with_many_fillers_is_fast() {
        let sentence =
            "we need to ship the release um and then, uh, check the dashboard erm before lunch. ";
        let text = sentence.repeat(1_500);
        let start = std::time::Instant::now();
        let r = clean(&text);
        assert_eq!(
            r.changes.first().map(String::as_str),
            Some("Disfluency: removed 4500 fillers")
        );
        assert!(!r.text.contains(" um "));
        assert!(start.elapsed().as_secs_f64() < 2.0);
    }

    #[test]
    fn letter_i_after_a_determiner_is_not_capitalized() {
        for input in [
            "click on the i icon",
            "use a i variable",
            "the loop variable i is unused",
            "type the letter i here",
        ] {
            assert_eq!(cleaned(input), input);
        }
        assert_eq!(cleaned("then i went home"), "then I went home");
        assert_eq!(cleaned("so the i think"), "so the i think");
    }
}
