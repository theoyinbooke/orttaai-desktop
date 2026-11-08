//! Spoken formatting (Mac `SpokenFormattingFormatter`) and automatic
//! paragraphs (Mac `AutoParagraphFormatter`).
//!
//! Spoken formatting turns "new line" / "new paragraph" into real breaks and
//! "number one … number two …", "first, … second, …", "letter a …" and
//! "bullet point …" into lists. Automatic paragraphs split a long dictation
//! that came out as one block at sentence ends, preferring a sentence that
//! opens a new point. The Swift originals use ICU look-arounds; the guards are
//! checked by hand here around `regex` matches.

use super::chars::{is_blank, is_newline, is_word_char};
use regex::Regex;
use std::sync::LazyLock;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern compiles")
}

fn prev_char(text: &str, at: usize) -> Option<char> {
    text[..at].chars().next_back()
}

fn next_char(text: &str, at: usize) -> Option<char> {
    text[at..].chars().next()
}

fn not_word_before(text: &str, at: usize) -> bool {
    prev_char(text, at).is_none_or(|c| !is_word_char(c))
}

fn not_word_after(text: &str, at: usize) -> bool {
    next_char(text, at).is_none_or(|c| !is_word_char(c))
}

/// Every match of `re` in `text` (left to right, non-overlapping) for which
/// `accept(start, end)` holds; a rejected match lets the search resume one
/// character later, like a regex look-around failing.
fn guarded_matches(
    re: &Regex,
    text: &str,
    mut accept: impl FnMut(&regex::Captures<'_>) -> bool,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= text.len() {
        let Some(caps) = re.captures_at(text, pos) else {
            break;
        };
        let m = caps.get(0).expect("group 0");
        if accept(&caps) {
            out.push((m.start(), m.end()));
            pos = if m.end() > m.start() {
                m.end()
            } else {
                m.end() + next_char(text, m.end()).map_or(1, char::len_utf8)
            };
        } else {
            pos = m.start() + next_char(text, m.start()).map_or(1, char::len_utf8);
        }
    }
    out
}

/// Trim spaces/tabs and other non-newline whitespace (Swift `.whitespaces`).
fn trim_spaces(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() && !is_newline(c))
}

// ---- Line break commands ---------------------------------------------------------

static PARAGRAPH_KEYWORD: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:new|next)[ \t]+paragraph"));
static LINE_KEYWORD: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:new[ \t]+line|newline|next[ \t]+line)"));

/// "add [a] new paragraph [about pricing]" is prose, "new paragraph" alone is
/// a command.
const ARTICLES: &[&str] = &[
    "a", "the", "another", "every", "each", "this", "that", "per", "one",
];
const FOLLOWERS: &[&str] = &[
    "of", "for", "to", "in", "on", "at", "with", "from", "about", "between", "into",
];

fn preceded_by_article(text: &str, at: usize) -> bool {
    let Some(before) = text[..at].strip_suffix(' ') else {
        return false;
    };
    ARTICLES.iter().any(|a| {
        before.len() >= a.len()
            && before.is_char_boundary(before.len() - a.len())
            && before[before.len() - a.len()..].eq_ignore_ascii_case(a)
            && not_word_before(before, before.len() - a.len())
    })
}

fn followed_by_preposition(text: &str, at: usize) -> bool {
    let rest = &text[at..];
    let trimmed = rest.trim_start_matches([' ', '\t']);
    if trimmed.len() == rest.len() {
        return false;
    }
    FOLLOWERS.iter().any(|w| {
        trimmed.len() >= w.len()
            && trimmed.is_char_boundary(w.len())
            && trimmed[..w.len()].eq_ignore_ascii_case(w)
            && not_word_after(trimmed, w.len())
    })
}

/// Replaces each guarded keyword match (with the filler punctuation around
/// it) by `replacement`; returns the new text and the match count.
fn replace_break_commands(text: &str, keyword: &Regex, replacement: &str) -> (String, usize) {
    let matches = guarded_matches(keyword, text, |caps| {
        let m = caps.get(0).expect("group 0");
        not_word_before(text, m.start())
            && !preceded_by_article(text, m.start())
            && not_word_after(text, m.end())
            && !followed_by_preposition(text, m.end())
    });
    if matches.is_empty() {
        return (text.to_string(), 0);
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in &matches {
        // `[ \t]*[,;:]?[ \t]*` before the keyword, never reaching back past
        // the previous replacement.
        let mut s = *start;
        while s > cursor && matches!(bytes[s - 1], b' ' | b'\t') {
            s -= 1;
        }
        if s > cursor && matches!(bytes[s - 1], b',' | b';' | b':') {
            s -= 1;
            while s > cursor && matches!(bytes[s - 1], b' ' | b'\t') {
                s -= 1;
            }
        }
        // `[.,;:]?` after it.
        let mut e = *end;
        if e < bytes.len() && matches!(bytes[e], b'.' | b',' | b';' | b':') {
            e += 1;
        }
        out.push_str(&text[cursor..s]);
        out.push_str(replacement);
        cursor = e;
    }
    out.push_str(&text[cursor..]);
    (out, matches.len())
}

static BLANKS_BEFORE_NEWLINE: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t]+\n"));
static BLANKS_AFTER_NEWLINE: LazyLock<Regex> = LazyLock::new(|| re(r"\n[ \t]+"));
static EXTRA_NEWLINES: LazyLock<Regex> = LazyLock::new(|| re(r"\n{3,}"));
static TRAILING_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| re(r"\s+$"));

/// Applies spoken "new line" / "new paragraph" commands as real breaks,
/// consuming the filler punctuation Whisper wraps them in and re-casing the
/// sentence that follows. Trailing breaks are dropped (a pasted trailing
/// newline submits in terminals and some chat apps); leading breaks are kept
/// so dictation can push below the current line.
pub fn apply_line_break_commands(text: &str) -> (String, Vec<String>) {
    let (value, paragraphs) = replace_break_commands(text, &PARAGRAPH_KEYWORD, "\n\n");
    let (value, lines) = replace_break_commands(&value, &LINE_KEYWORD, "\n");
    let count = paragraphs + lines;
    if count == 0 {
        return (text.to_string(), Vec::new());
    }
    let value = BLANKS_BEFORE_NEWLINE.replace_all(&value, "\n");
    let value = BLANKS_AFTER_NEWLINE.replace_all(&value, "\n");
    let value = EXTRA_NEWLINES.replace_all(&value, "\n\n");
    let value = TRAILING_WHITESPACE.replace_all(&value, "");
    let value = capitalized_after_line_breaks(&value);
    let label = if count == 1 {
        "1 line break command".to_string()
    } else {
        format!("{count} line break commands")
    };
    (value, vec![format!("Spoken formatting: {label}")])
}

fn capitalized_after_line_breaks(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\n' {
            i += 1;
            continue;
        }
        let mut content = i + 1;
        while content < chars.len() && chars[content] == '\n' {
            content += 1;
        }
        if content >= chars.len() {
            break;
        }
        // Leave mixed-case tokens like "iPhone" alone.
        let followed_by_upper = chars.get(content + 1).is_some_and(|c| c.is_uppercase());
        if chars[content].is_lowercase() && !followed_by_upper {
            let upper: Vec<char> = chars[content].to_uppercase().collect();
            chars.splice(content..=content, upper);
        }
        i = content;
    }
    chars.into_iter().collect()
}

// ---- Lists -----------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct OrderedMarker {
    start: usize,
    end: usize,
    number: u32,
    /// "First," is ordinary prose on its own; only "number one" and "item
    /// one" may start a one-item list.
    is_ordinal: bool,
    /// Written as a digit ("2."), which prose has too: a list of these must
    /// start at 1.
    is_inline: bool,
}

#[derive(Debug, Clone, Copy)]
struct LetterMarker {
    start: usize,
    end: usize,
    /// 1 for a, 2 for b…
    index: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MarkerKind {
    Ordered,
    Bullet,
}

/// One item's lines, and for the last item the prose that follows the list.
type ItemLayout = (Vec<String>, Option<String>);

const NUMBER_WORDS: &[(&str, u32)] = &[
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
    ("twenty", 20),
];

const ORDINAL_WORDS: &[(&str, u32)] = &[
    ("first", 1),
    ("second", 2),
    ("third", 3),
    ("fourth", 4),
    ("fifth", 5),
    ("sixth", 6),
    ("seventh", 7),
    ("eighth", 8),
    ("ninth", 9),
    ("tenth", 10),
];

/// `20|1[0-9]|[1-9]` or a number word up to twenty.
fn list_number(token: &str) -> Option<u32> {
    let lower = token.to_lowercase();
    if let Some((_, n)) = NUMBER_WORDS.iter().find(|(w, _)| *w == lower) {
        return Some(*n);
    }
    if lower.bytes().all(|b| b.is_ascii_digit()) && !lower.starts_with('0') && lower.len() <= 2 {
        let n: u32 = lower.parse().ok()?;
        return (1..=20).contains(&n).then_some(n);
    }
    None
}

static SPOKEN_ORDERED: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:number|item)\s+([\p{L}\p{N}_]+)"));
static BULLET_MARKER: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)bullet\s+point"));
static SPOKEN_LETTER: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)letter\s+([a-j])"));
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| re("[.!?][\"”’)]?\\s+"));

fn ordered_markers(text: &str) -> Vec<OrderedMarker> {
    let mut markers = Vec::new();

    // "number one", "item 3".
    let mut numbers = Vec::new();
    guarded_matches(&SPOKEN_ORDERED, text, |caps| {
        let m = caps.get(0).expect("group 0");
        let token = caps.get(1).expect("group 1").as_str();
        match list_number(token) {
            Some(n) if not_word_before(text, m.start()) => {
                numbers.push((m.start(), m.end(), n));
                true
            }
            _ => false,
        }
    });
    markers.extend(
        numbers
            .into_iter()
            .map(|(start, end, number)| OrderedMarker {
                start,
                end,
                number,
                is_ordinal: false,
                is_inline: false,
            }),
    );

    // "First, … Second, …" and "Firstly: …": an ordinal opening a clause (or
    // following ", and") and followed by a comma or colon.
    for (i, _) in text.char_indices() {
        let Some((word, number)) = ORDINAL_WORDS.iter().find(|(w, _)| {
            text.get(i..i + w.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(w))
        }) else {
            continue;
        };
        if !opens_clause(text, i) {
            continue;
        }
        let after_word = i + word.len();
        let with_ly = text
            .get(after_word..after_word + 2)
            .filter(|s| s.eq_ignore_ascii_case("ly"))
            .map(|_| after_word + 2);
        let end = with_ly
            .into_iter()
            .chain(std::iter::once(after_word))
            .find(|&e| followed_by_comma_or_colon(text, e));
        if let Some(end) = end {
            markers.push(OrderedMarker {
                start: i,
                end,
                number: *number,
                is_ordinal: true,
                is_inline: false,
            });
        }
    }

    // "1. Open", "2) Choose": a digit written by the recognizer.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() || !not_word_before(text, i) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        let number = list_number(&text[i..j]);
        let punct = bytes.get(j).is_some_and(|b| *b == b'.' || *b == b')');
        let spaced = next_char(text, (j + 1).min(text.len())).is_some_and(char::is_whitespace);
        if let (Some(number), true, true) = (number, punct, spaced) {
            markers.push(OrderedMarker {
                start: i,
                end: j + 1,
                number,
                is_ordinal: false,
                is_inline: true,
            });
        }
        i = j;
    }

    // "number 2." matches as a spoken and as a written marker: keep one.
    markers.sort_by_key(|m| m.start);
    let mut kept: Vec<OrderedMarker> = Vec::with_capacity(markers.len());
    for m in markers {
        if kept.last().is_some_and(|p| m.start < p.end) {
            continue;
        }
        kept.push(m);
    }
    kept
}

/// `^`, `(?<=[.!?:;,][ \t])` or `(?<=,[ \t]and[ \t])` before an ordinal.
fn opens_clause(text: &str, at: usize) -> bool {
    let before = &text[..at];
    let mut rev = before.chars().rev();
    let Some(c1) = rev.next() else {
        return true;
    };
    if c1 == '\n' || c1 == '\r' {
        return true;
    }
    if !is_blank(c1) {
        return false;
    }
    if rev.next().is_some_and(|c| ".!?:;,".contains(c)) {
        return true;
    }
    let trimmed = &before[..before.len() - 1];
    if trimmed.len() < 5 || !trimmed.is_char_boundary(trimmed.len() - 5) {
        return false;
    }
    let tail = &trimmed[trimmed.len() - 5..];
    let tail_bytes = tail.as_bytes();
    tail_bytes[0] == b','
        && matches!(tail_bytes[1], b' ' | b'\t')
        && tail[2..].eq_ignore_ascii_case("and")
}

fn followed_by_comma_or_colon(text: &str, at: usize) -> bool {
    text[at..]
        .trim_start_matches([' ', '\t'])
        .starts_with([',', ':'])
}

fn bullet_markers(text: &str) -> Vec<(usize, usize)> {
    guarded_matches(&BULLET_MARKER, text, |caps| {
        let m = caps.get(0).expect("group 0");
        not_word_before(text, m.start()) && not_word_after(text, m.end())
    })
}

/// "letter a, … letter b, …" as said, "a) … b) …" / "(a) … (b) …" as
/// transcribed, and "a. …" at the start of a line. "a." mid-sentence is left
/// alone: it's an initial or the article more often than a list.
fn letter_markers(text: &str) -> Vec<LetterMarker> {
    let mut markers = Vec::new();
    let mut spoken = Vec::new();
    guarded_matches(&SPOKEN_LETTER, text, |caps| {
        let m = caps.get(0).expect("group 0");
        let ok = not_word_before(text, m.start()) && not_word_after(text, m.end());
        if ok {
            let letter = caps.get(1).expect("group 1").as_str();
            spoken.push((m.start(), m.end(), letter_index(letter)));
        }
        ok
    });
    markers.extend(spoken.into_iter().map(|(start, end, index)| LetterMarker {
        start,
        end,
        index,
    }));

    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        // `(?<![\p{L}\p{N}_])\(?([a-j])\)` followed by a blank.
        if not_word_before(text, i) {
            let open = usize::from(bytes[i] == b'(');
            let l = i + open;
            if l + 1 < bytes.len()
                && is_list_letter(bytes[l])
                && bytes[l + 1] == b')'
                && bytes.get(l + 2).is_some_and(|b| *b == b' ' || *b == b'\t')
            {
                markers.push(LetterMarker {
                    start: i,
                    end: l + 2,
                    index: letter_index(&text[l..l + 1]),
                });
                i = l + 2;
                continue;
            }
        }
        // `^[ \t]*([a-j])[.)]` followed by a blank, at a line start.
        if i == 0 || bytes[i - 1] == b'\n' || bytes[i - 1] == b'\r' {
            let mut l = i;
            while l < bytes.len() && (bytes[l] == b' ' || bytes[l] == b'\t') {
                l += 1;
            }
            if l + 1 < bytes.len()
                && is_list_letter(bytes[l])
                && (bytes[l + 1] == b'.' || bytes[l + 1] == b')')
                && bytes.get(l + 2).is_some_and(|b| *b == b' ' || *b == b'\t')
            {
                markers.push(LetterMarker {
                    start: i,
                    end: l + 2,
                    index: letter_index(&text[l..l + 1]),
                });
                i = l + 2;
                continue;
            }
        }
        i += 1;
    }
    markers.sort_by_key(|m| m.start);
    markers
}

fn is_list_letter(b: u8) -> bool {
    matches!(b.to_ascii_lowercase(), b'a'..=b'j')
}

fn letter_index(letter: &str) -> u32 {
    letter
        .bytes()
        .next()
        .map_or(0, |b| u32::from(b.to_ascii_lowercase() - b'a' + 1))
}

fn letter(index: u32, uppercase: bool) -> char {
    let c = char::from(b'a' + (index.saturating_sub(1).min(25)) as u8);
    if uppercase {
        c.to_ascii_uppercase()
    } else {
        c
    }
}

/// `^[ \t]*(?:\(?[a-j][.)](?=[ \t])|letter\s+[a-j](?![\p{L}\p{N}_]))`.
fn line_starts_with_letter(text: &str) -> bool {
    let rest = text.trim_start_matches([' ', '\t']);
    let b = rest.as_bytes();
    let open = usize::from(b.first() == Some(&b'('));
    if b.len() > open + 2
        && is_list_letter(b[open])
        && (b[open + 1] == b'.' || b[open + 1] == b')')
        && (b[open + 2] == b' ' || b[open + 2] == b'\t')
    {
        return true;
    }
    if rest.len() >= 6 && rest.is_char_boundary(6) && rest[..6].eq_ignore_ascii_case("letter") {
        let after = rest[6..].trim_start_matches(char::is_whitespace);
        if after.len() < rest.len() - 6 {
            let ab = after.as_bytes();
            return ab.first().is_some_and(|b| is_list_letter(*b)) && not_word_after(after, 1);
        }
    }
    false
}

/// The first run of consecutive letters starting at "a", at least two long.
fn letter_run(text: &str) -> Option<Vec<LetterMarker>> {
    let mut run: Vec<LetterMarker> = Vec::new();
    for marker in letter_markers(text) {
        if let Some(last) = run.last() {
            if marker.start < last.end {
                continue;
            }
            if marker.index == last.index + 1 {
                run.push(marker);
                continue;
            }
        }
        if run.len() >= 2 {
            break;
        }
        run = if marker.index == 1 {
            vec![marker]
        } else {
            Vec::new()
        };
    }
    (run.len() >= 2).then_some(run)
}

/// Runs of consecutive numbers (1, 2, 3…), each of at least two items, in
/// order. Digit-only runs must start at 1, so "I bought 2. Then I bought 3."
/// stays prose. A lone "number one" opening the paragraph is a one-item list.
fn ordered_runs(markers: &[OrderedMarker], text: &str) -> Vec<Vec<OrderedMarker>> {
    let mut sequences: Vec<Vec<OrderedMarker>> = Vec::new();
    for &m in markers {
        match sequences.last_mut() {
            Some(seq) if seq.last().is_some_and(|l| m.number == l.number + 1) => seq.push(m),
            _ => sequences.push(vec![m]),
        }
    }
    let runs: Vec<Vec<OrderedMarker>> = sequences
        .into_iter()
        .filter(|s| s.len() >= 2 && !(s[0].is_inline && s[0].number != 1))
        .collect();
    if !runs.is_empty() {
        return runs;
    }
    if let Some(&first) = markers.first() {
        if first.number == 1
            && !first.is_ordinal
            && is_likely_single_marker_command(first.start, first.end, text, MarkerKind::Ordered)
        {
            return vec![vec![first]];
        }
    }
    Vec::new()
}

fn bullet_run(markers: &[(usize, usize)], text: &str) -> Option<Vec<(usize, usize)>> {
    let &(start, end) = markers.first()?;
    if markers.len() >= 2 {
        return Some(markers.to_vec());
    }
    is_likely_single_marker_command(start, end, text, MarkerKind::Bullet)
        .then(|| vec![(start, end)])
}

fn is_likely_single_marker_command(start: usize, end: usize, text: &str, kind: MarkerKind) -> bool {
    if !text[..start].trim().is_empty() {
        return false;
    }
    let item = cleaned_item_content(&text[end..], false);
    if word_count(&item) < 2 {
        return false;
    }
    let Some(first) = first_word(&item) else {
        return false;
    };
    let mut blocked = vec![
        "are", "can", "could", "is", "means", "refers", "should", "was", "were", "will", "would",
    ];
    if kind == MarkerKind::Ordered {
        blocked.extend([
            "candidate",
            "choice",
            "position",
            "rank",
            "ranking",
            "reason",
            "reasons",
            "spot",
        ]);
    }
    !blocked.contains(&first.as_str())
}

/// Every numbered list in a paragraph. When a second list follows in the same
/// paragraph, the first one's last item ends at the last sentence before it,
/// so the sentence that introduces the second list stays with it.
fn format_ordered_lists(text: &str) -> Option<String> {
    let runs = ordered_runs(&ordered_markers(text), text);
    if runs.is_empty() {
        return None;
    }
    let mut lists = Vec::with_capacity(runs.len());
    let mut start = 0;
    for (index, run) in runs.iter().enumerate() {
        let end = match runs.get(index + 1) {
            Some(next) => last_sentence_end(text, run[run.len() - 1].end, next[0].start),
            None => text.len(),
        };
        let ranges: Vec<(usize, usize)> = run.iter().map(|m| (m.start, m.end)).collect();
        let list = laid_out_list(&ranges, text, start, end, |item, raw, is_last| {
            ordered_item_layout(run[item].number, raw, is_last)
        })?;
        lists.push(list);
        start = end;
    }
    Some(lists.join("\n\n"))
}

/// Lettered points under a numbered item sit under its text.
const NESTED_INDENT: &str = "   ";

/// "1. Item", and under it "a. …" lines when the item holds its own lettered
/// points ("letter a, … letter b, …").
fn ordered_item_layout(number: u32, raw: &str, is_last: bool) -> Option<ItemLayout> {
    let Some(letters) = letter_run(raw) else {
        return item_layout(&format!("{number}."), raw, is_last, "");
    };
    let head = cleaned_item_content(&raw[..letters[0].start], true);
    let mut lines = vec![if head.is_empty() {
        format!("{number}.")
    } else {
        format!("{number}. {}", capitalized_first_content_word(&head))
    }];
    let mut trailing = None;
    for (index, marker) in letters.iter().enumerate() {
        let is_last_letter = index == letters.len() - 1;
        let end = if is_last_letter {
            raw.len()
        } else {
            letters[index + 1].start
        };
        let layout = item_layout(
            &format!("{}.", letter(marker.index, false)),
            &raw[marker.end..end],
            is_last && is_last_letter,
            NESTED_INDENT,
        )?;
        lines.extend(layout.0);
        trailing = layout.1;
    }
    Some((lines, trailing))
}

/// A list of letters on its own: "A. …", "B. …".
fn format_lettered_list(text: &str) -> Option<String> {
    let run = letter_run(text)?;
    let ranges: Vec<(usize, usize)> = run.iter().map(|m| (m.start, m.end)).collect();
    laid_out_list(&ranges, text, 0, text.len(), |item, raw, is_last| {
        item_layout(
            &format!("{}.", letter(run[item].index, true)),
            raw,
            is_last,
            "",
        )
    })
}

fn format_bullet_list(text: &str) -> Option<String> {
    let run = bullet_run(&bullet_markers(text), text)?;
    laid_out_list(&run, text, 0, text.len(), |_, raw, is_last| {
        item_layout("-", raw, is_last, "")
    })
}

/// Lays out one run of markers found in `text[start..end]`: what comes before
/// the first marker is the list's introduction, each item runs to the next
/// marker, and the last one to `end`, or to a line break the speaker (or
/// Polish) put after it. `None` if an item is empty.
fn laid_out_list(
    markers: &[(usize, usize)],
    text: &str,
    start: usize,
    end: usize,
    mut item: impl FnMut(usize, &str, bool) -> Option<ItemLayout>,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut trailing_prose = None;
    let mut trailing_lines = None;
    for index in 0..markers.len() {
        let is_last = index == markers.len() - 1;
        let item_end = if is_last { end } else { markers[index + 1].0 };
        let mut raw = &text[markers[index].1..item_end.max(markers[index].1)];
        if is_last {
            let (kept, lines_after) = end_of_last_item(raw);
            raw = kept;
            trailing_lines = lines_after;
        }
        let layout = item(index, raw, is_last)?;
        lines.extend(layout.0);
        if is_last {
            trailing_prose = layout.1;
        }
    }
    let mut list = lines.join("\n");
    if let Some(p) = trailing_prose {
        list.push_str("\n\n");
        list.push_str(&p);
    }
    if let Some(l) = trailing_lines {
        list.push('\n');
        list.push_str(&l);
    }
    let prefix = text[start..markers[0].0.max(start)]
        .trim()
        .trim_matches([' ', ',', ';']);
    Some(if prefix.is_empty() {
        list
    } else {
        format!("{prefix}\n{list}")
    })
}

fn item_layout(marker: &str, raw: &str, is_last: bool, indent: &str) -> Option<ItemLayout> {
    let mut item = cleaned_item_content(raw, true);
    if item.is_empty() {
        return None;
    }
    let mut trailing = None;
    if is_last {
        let (kept, rest) = split_trailing_prose(&item);
        item = kept;
        trailing = rest;
    }
    Some((
        vec![format!(
            "{indent}{marker} {}",
            capitalized_first_content_word(&item)
        )],
        trailing,
    ))
}

/// The last item ends at a line break, unless the next line carries on with
/// the item's own lettered points. What follows the break is kept as it was.
fn end_of_last_item(raw: &str) -> (&str, Option<String>) {
    let mut search = 0;
    while let Some(offset) = raw[search..].find('\n') {
        let newline = search + offset;
        let rest = &raw[newline + 1..];
        search = newline + 1;
        // Item text starting on the next line ("number one, new line, …").
        if cleaned_item_content(&raw[..newline], true).is_empty() {
            continue;
        }
        if line_starts_with_letter(rest) {
            continue;
        }
        let trailing = rest.trim();
        return (
            &raw[..newline],
            (!trailing.is_empty()).then(|| trailing.to_string()),
        );
    }
    (raw, None)
}

/// Just after the last sentence end in `text[start..end]`, or `end`.
fn last_sentence_end(text: &str, start: usize, end: usize) -> usize {
    if start >= end {
        return end;
    }
    SENTENCE_END
        .find_iter(&text[start..end])
        .last()
        .map_or(end, |m| start + m.end())
}

static WHITESPACE_RUN: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));
static LEADING_DELIMITERS: LazyLock<Regex> = LazyLock::new(|| re(r"^[\s,:;.\-]+"));
static LEADING_LINE_COMMAND: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(new line|new paragraph)\b\s*"));
static TRAILING_LINE_COMMAND: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\s*\b(new line|new paragraph)$"));
static LEADING_LINKING_VERB: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(is|are|was|were)\s+"));
/// A dangling "and"/"or"/"then"/"next" ending an item. It must be its own
/// word, so "brand" and "color" keep their letters.
static TRAILING_CONNECTOR: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:^|,?\s+)(?:and|or|then|next)$"));

fn cleaned_item_content(raw: &str, strip_leading_linking_verb: bool) -> String {
    let value = raw.replace("\r\n", "\n").replace('\n', " ");
    let value = WHITESPACE_RUN.replace_all(&value, " ");
    let value = value.trim();
    let value = LEADING_DELIMITERS.replace(value, "");
    let value = LEADING_LINE_COMMAND.replace(&value, "");
    let value = TRAILING_LINE_COMMAND.replace(&value, "");
    let mut value = value.trim().to_string();
    if strip_leading_linking_verb {
        value = LEADING_LINKING_VERB.replace(&value, "").into_owned();
    }
    let value = TRAILING_CONNECTOR.replace(&value, "");
    value.trim().trim_matches([' ', ',', ':', ';']).to_string()
}

/// Nothing marks where the last item ends, so a long one keeps its opening
/// sentences (up to `LAST_ITEM_MAXIMUM_WORDS`) and the rest of the dictation
/// follows the list as its own paragraph.
const LAST_ITEM_MAXIMUM_WORDS: usize = 40;
const LAST_ITEM_MAXIMUM_SENTENCES: usize = 3;

fn split_trailing_prose(item: &str) -> (String, Option<String>) {
    let sentences = sentences(item);
    if sentences.len() <= 1 || word_count(item) <= LAST_ITEM_MAXIMUM_WORDS {
        return (item.to_string(), None);
    }
    let mut kept = vec![sentences[0].clone()];
    let mut kept_words = word_count(&sentences[0]);
    for sentence in sentences.iter().skip(1) {
        let words = word_count(sentence);
        if kept.len() >= LAST_ITEM_MAXIMUM_SENTENCES || kept_words + words > LAST_ITEM_MAXIMUM_WORDS
        {
            break;
        }
        kept.push(sentence.clone());
        kept_words += words;
    }
    if kept.len() >= sentences.len() {
        return (item.to_string(), None);
    }
    let trailing = sentences[kept.len()..].join(" ");
    (
        kept.join(" "),
        Some(capitalized_first_content_word(&trailing)),
    )
}

fn capitalized_first_content_word(text: &str) -> String {
    let Some((index, first)) = text.char_indices().find(|(_, c)| !c.is_whitespace()) else {
        return text.to_string();
    };
    if !first.is_lowercase() {
        return text.to_string();
    }
    let next = index + first.len_utf8();
    if next_char(text, next).is_some_and(char::is_uppercase) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push_str(&text[..index]);
    out.extend(first.to_uppercase());
    out.push_str(&text[next..]);
    out
}

/// Words split on anything that is not a letter or digit.
fn word_count(text: &str) -> usize {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .count()
}

fn first_word(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .find(|w| !w.is_empty())
        .map(str::to_lowercase)
}

type ListFormatter = fn(&str) -> Option<String>;

const CHANGE_NUMBERED: &str = "Spoken formatting: numbered list";
const CHANGE_LETTERED: &str = "Spoken formatting: lettered list";
const CHANGE_BULLET: &str = "Spoken formatting: bullet list";

/// Line-break commands, then lists, one paragraph at a time: a list ends where
/// its paragraph does, and a list already laid out (by Polish, or an earlier
/// pass) comes through as it is.
pub fn format_spoken(text: &str) -> (String, Vec<String>) {
    let (value, mut changes) = apply_line_break_commands(text);
    let formatters: [(ListFormatter, &str); 3] = [
        (format_ordered_lists, CHANGE_NUMBERED),
        (format_lettered_list, CHANGE_LETTERED),
        (format_bullet_list, CHANGE_BULLET),
    ];
    let blocks: Vec<String> = value
        .split("\n\n")
        .map(|block| {
            for (format, change) in &formatters {
                let Some(formatted) = format(block) else {
                    continue;
                };
                if formatted != block && !changes.iter().any(|c| c == change) {
                    changes.push((*change).to_string());
                }
                return formatted;
            }
            block.to_string()
        })
        .collect();
    (blocks.join("\n\n"), changes)
}

// ---- Automatic paragraphs --------------------------------------------------------

/// Shorter dictations (about 30 seconds of speech) stay one paragraph.
pub const MINIMUM_PARAGRAPH_WORDS: usize = 80;
/// A paragraph may end at a new point once it has this much…
const MINIMUM_SENTENCES_BEFORE_SHIFT: usize = 2;
const MINIMUM_WORDS_BEFORE_SHIFT: usize = 25;
/// …and must end by this size even without one.
const MAXIMUM_PARAGRAPH_WORDS: usize = 90;
const MINIMUM_SENTENCES_PER_PARAGRAPH: usize = 3;
/// A shorter last sentence joins the paragraph before it.
const MINIMUM_TRAILING_WORDS: usize = 12;

/// Openers that move to a new point, matched against the start of a sentence,
/// case-insensitively, as whole words.
const TOPIC_SHIFT_OPENERS: &[&str] = &[
    "also",
    "another",
    "additionally",
    "in addition",
    "next",
    "then",
    "finally",
    "lastly",
    "now",
    "anyway",
    "so",
    "however",
    "on the other hand",
    "the other thing",
    "the other one",
    "one more thing",
    "moving on",
    "secondly",
    "thirdly",
    "besides",
    "meanwhile",
    "overall",
    "in summary",
];

/// Apps where a pasted newline runs a command (macOS names from the Mac app
/// plus the Linux `WM_CLASS` and Windows image names of common terminals).
const TERMINAL_APPS: &[&str] = &[
    "terminal",
    "iterm",
    "iterm2",
    "ghostty",
    "warp",
    "alacritty",
    "kitty",
    "wezterm",
    "hyper",
    "tabby",
    "cmux",
    "windowsterminal",
    "cmd",
    "powershell",
    "pwsh",
    "conhost",
    "mintty",
    "gnome-terminal",
    "konsole",
    "xterm",
    "uxterm",
    "urxvt",
    "rxvt",
    "tilix",
    "terminator",
    "foot",
    "xfce4-terminal",
    "mate-terminal",
    "lxterminal",
    "qterminal",
    "terminology",
    "st",
    "guake",
    "yakuake",
    "tilda",
    "blackbox",
    "ptyxis",
    "kgx",
    "contour",
    "rio",
    "wave",
];

/// False for terminals and other command-line apps, where automatic
/// paragraphs never apply.
pub fn auto_paragraphs_apply_to(target_app: Option<&str>) -> bool {
    let Some(app) = target_app else {
        return true;
    };
    let mut name = app.trim().to_lowercase();
    for suffix in [".exe", ".desktop", "-server", "-gui"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            name = stripped.to_string();
        }
    }
    let name = name
        .rsplit(['/', '\\', '.'])
        .next()
        .unwrap_or(&name)
        .to_string();
    !(TERMINAL_APPS.contains(&name.as_str()) || name.contains("terminal"))
}

/// Splits each block of prose (text between blank lines) on its own, so the
/// paragraph after a list is split too; blocks with line breaks inside, like
/// the list itself, stay as they are.
pub fn format_paragraphs(text: &str) -> (String, Vec<String>) {
    let mut added = 0usize;
    let blocks: Vec<String> = text
        .split("\n\n")
        .map(|block| {
            let split = paragraphs(block);
            added += split.len() - 1;
            split.join("\n\n")
        })
        .collect();
    if added == 0 {
        return (text.to_string(), Vec::new());
    }
    let label = if added == 1 {
        "1 paragraph break".to_string()
    } else {
        format!("{added} paragraph breaks")
    };
    (
        blocks.join("\n\n"),
        vec![format!("Formatting: added {label}")],
    )
}

fn whitespace_word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn paragraphs(text: &str) -> Vec<String> {
    if text.contains('\n') {
        return vec![text.to_string()];
    }
    let all = sentences(text);
    if all.len() < MINIMUM_SENTENCES_PER_PARAGRAPH * 2
        || whitespace_word_count(text) < MINIMUM_PARAGRAPH_WORDS
    {
        return vec![text.to_string()];
    }
    let mut paragraphs: Vec<Vec<String>> = vec![Vec::new()];
    let mut current_words = 0usize;
    for sentence in all {
        let words = whitespace_word_count(&sentence);
        let current = paragraphs.last().expect("non-empty");
        let shifts_topic = opens_new_point(&sentence)
            && current.len() >= MINIMUM_SENTENCES_BEFORE_SHIFT
            && current_words >= MINIMUM_WORDS_BEFORE_SHIFT;
        let is_full = current.len() >= MINIMUM_SENTENCES_PER_PARAGRAPH
            && current_words + words > MAXIMUM_PARAGRAPH_WORDS;
        if !current.is_empty() && (shifts_topic || is_full) {
            paragraphs.push(Vec::new());
            current_words = 0;
        }
        paragraphs.last_mut().expect("non-empty").push(sentence);
        current_words += words;
    }
    if paragraphs.len() > 1 {
        let last = paragraphs.last().expect("non-empty");
        if last.len() == 1 && whitespace_word_count(&last[0]) < MINIMUM_TRAILING_WORDS {
            let sentence = paragraphs.pop().expect("non-empty").remove(0);
            paragraphs.last_mut().expect("non-empty").push(sentence);
        }
    }
    if paragraphs.len() <= 1 {
        return vec![text.to_string()];
    }
    paragraphs.into_iter().map(|p| p.join(" ")).collect()
}

/// Sentences end at . ! ? (with any closing quote) before a capital letter or
/// digit, so "e.g. this" and "3.5 GB" stay whole.
pub fn sentences(text: &str) -> Vec<String> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        if !chars[i].1.is_whitespace() || i == 0 {
            i += 1;
            continue;
        }
        let ends_sentence = {
            let c1 = chars[i - 1].1;
            ".!?".contains(c1) || ("\"”’)".contains(c1) && i >= 2 && ".!?".contains(chars[i - 2].1))
        };
        let mut j = i;
        while j < chars.len() && chars[j].1.is_whitespace() {
            j += 1;
        }
        let opens = {
            let mut k = j;
            if k < chars.len() && "\"“(".contains(chars[k].1) {
                k += 1;
            }
            k < chars.len() && (chars[k].1.is_uppercase() || chars[k].1.is_numeric())
        };
        if ends_sentence && opens {
            let sentence = trim_spaces(&text[start..chars[i].0]);
            if !sentence.is_empty() {
                out.push(sentence.to_string());
            }
            start = if j < chars.len() {
                chars[j].0
            } else {
                text.len()
            };
        }
        i = j;
    }
    let tail = trim_spaces(&text[start..]);
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

fn opens_new_point(sentence: &str) -> bool {
    let lowered = sentence.to_lowercase();
    TOPIC_SHIFT_OPENERS.iter().any(|opener| {
        lowered
            .strip_prefix(opener)
            .is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphabetic()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(text: &str) -> String {
        format_spoken(text).0
    }

    #[test]
    fn polished_answers_come_through_unchanged() {
        for polished in [
            "For the morning:\n1. Coffee\n2. Email\n\nFor the afternoon:\n1. Meetings\n2. Code review",
            "The meeting went well.\n\n1. We agreed on the pricing.\n2. We agreed on the launch date.\n\nAfter the meeting, I sent the notes to everyone.",
            "Here is what we agreed:\n\n1. Sam owns the pricing page.\n2. Ade fixes the login bug.\n\nThanks, Theo.",
            "1. Rent chairs.\n2. Print the menus.\n\nNumber 11, print the menus.",
            "1. Fruits.\n   a. Apples.\n   b. Pears.\n2. Vegetables.\n   a. Carrots.\n   b. Spinach.",
            "The options are:\nA. The red one\nB. The blue one\nC. The green one",
            "We have three things to do:\n- Update the pricing page.\n- Fix the login bug.\n\nAfter that, the iPhone app.",
            "1. Open the app.\n2. Click save.\nThanks.",
        ] {
            let (text, changes) = format_spoken(polished);
            assert_eq!(text, polished);
            assert!(changes.is_empty(), "{changes:?}");
        }
    }

    #[test]
    fn lettered_points_without_indent_go_back_under_their_item() {
        assert_eq!(
            spoken("1. Fruits.\na. Apples.\nb. Pears.\n2. Vegetables.\na. Carrots.\nb. Spinach."),
            "1. Fruits.\n   a. Apples.\n   b. Pears.\n2. Vegetables.\n   a. Carrots.\n   b. Spinach."
        );
    }

    #[test]
    fn trailing_connector_must_be_a_whole_word() {
        assert_eq!(
            spoken("bullet point pick a brand bullet point pick a color"),
            "- Pick a brand\n- Pick a color"
        );
    }

    #[test]
    fn sentences_split_before_capitals_and_digits_only() {
        assert_eq!(
            sentences("Use e.g. this one. Then 3.5 GB is fine! \"Quoted\" end"),
            vec![
                "Use e.g. this one.",
                "Then 3.5 GB is fine!",
                "\"Quoted\" end"
            ]
        );
    }

    #[test]
    fn terminals_skip_auto_paragraphs() {
        for app in [
            "Terminal",
            "WindowsTerminal.exe",
            "gnome-terminal-server",
            "Alacritty",
            "powershell.exe",
            "org.gnome.Ptyxis",
        ] {
            assert!(!auto_paragraphs_apply_to(Some(app)), "{app}");
        }
        for app in ["Code", "slack", "firefox.exe"] {
            assert!(auto_paragraphs_apply_to(Some(app)), "{app}");
        }
        assert!(auto_paragraphs_apply_to(None));
    }
}
