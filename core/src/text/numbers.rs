//! Writes the numbers Parakeet ("Express") spells out the way Whisper writes
//! them (Mac `ExpressNumberFormatter`): "five five five zero one four three" →
//! "555-0143", "two thousand four hundred and fifty dollars" → "$2,450",
//! "6.45 a.m." → "6:45 a.m.". Meant for Express output only: Whisper already
//! writes digits, so these rules could only add risk there. Small counts stay
//! words ("twelve of the fifteen pairs") because that is how people write
//! them too.

use regex::Regex;
use std::sync::LazyLock;

const UNITS: &[(&str, u64)] = &[
    ("zero", 0),
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
];
const TENS: &[(&str, u64)] = &[
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("seventy", 70),
    ("eighty", 80),
    ("ninety", 90),
];
const SCALES: &[(&str, u64)] = &[
    ("hundred", 100),
    ("thousand", 1_000),
    ("million", 1_000_000),
    ("billion", 1_000_000_000),
];
const ORDINALS: &[(&str, u64)] = &[
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
    ("eleventh", 11),
    ("twelfth", 12),
    ("thirteenth", 13),
    ("fourteenth", 14),
    ("fifteenth", 15),
    ("sixteenth", 16),
    ("seventeenth", 17),
    ("eighteenth", 18),
    ("nineteenth", 19),
    ("twentieth", 20),
    ("thirtieth", 30),
];
const MONTHS: &[&str] = &[
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
const TIME_PREPOSITIONS: &[&str] = &["at", "around", "until", "till", "before", "after"];

fn lookup(table: &[(&str, u64)], word: &str) -> Option<u64> {
    table.iter().find(|(w, _)| *w == word).map(|(_, v)| *v)
}

static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\p{L}+(?:['’]\p{L}+)?").expect("static pattern"));
static MERIDIEM_AFTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[ \t]*[ap]\.?[ \t]?m\b").expect("static pattern"));
static SPOKEN_MERIDIEM_AFTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[ \t]+[ap]\.?[ \t]?m\b").expect("static pattern"));
static TIME_WORD_AFTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^[ \t]+(?:local|sharp|tonight|tomorrow|today|this|on|o['’]clock|in[ \t]+the[ \t]+(?:morning|afternoon|evening))\b",
    )
    .expect("static pattern")
});

/// Rewrites spelled-out numbers, phone digits, times, money, percentages and
/// date ordinals in `text`.
pub fn format_numbers(text: &str) -> String {
    let value = rewrite_dotted_times(text);
    format_number_words(&value)
}

/// "6.45 a.m." and "10.30pm": a dotted time is only rewritten when a meridiem
/// follows, so decimals ("4.25%", "26.2 miles") are left alone. Without a
/// meridiem it needs a time preposition before it and a time word after it
/// ("at 8.05 local time", "until 6.30 tonight").
fn rewrite_dotted_times(text: &str) -> String {
    let first = rewrite_times(text, |text, start, minutes_end| {
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_numeric() || c == '.'));
        before_ok && MERIDIEM_AFTER.is_match(&text[minutes_end..])
    });
    rewrite_times(&first, |text, start, minutes_end| {
        preceded_by_time_preposition(text, start) && TIME_WORD_AFTER.is_match(&text[minutes_end..])
    })
}

/// `(1[0-2]|0?[1-9])\.([0-5][0-9])` at each position, replaced by `h:mm` when
/// `accept(text, start, end)` holds.
fn rewrite_times(text: &str, accept: impl Fn(&str, usize, usize) -> bool) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let digit = |k: usize| b.get(k).copied().filter(u8::is_ascii_digit);
        let mut hour_lengths = Vec::with_capacity(2);
        if b[i] == b'1' && matches!(digit(i + 1), Some(b'0'..=b'2')) {
            hour_lengths.push(2);
        }
        if b[i] == b'0' && matches!(digit(i + 1), Some(b'1'..=b'9')) {
            hour_lengths.push(2);
        } else if b[i] != b'0' {
            hour_lengths.push(1);
        }
        let mut matched = None;
        for hour_len in hour_lengths {
            let dot = i + hour_len;
            let minutes_ok = b.get(dot) == Some(&b'.')
                && matches!(digit(dot + 1), Some(b'0'..=b'5'))
                && digit(dot + 2).is_some();
            if minutes_ok && accept(text, i, dot + 3) {
                matched = Some(dot);
                break;
            }
        }
        if let Some(dot) = matched {
            out.push_str(&text[cursor..dot]);
            out.push(':');
            cursor = dot + 1;
            i = dot + 3;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[cursor..]);
    out
}

fn preceded_by_time_preposition(text: &str, at: usize) -> bool {
    let Some(before) = text[..at].strip_suffix([' ', '\t']) else {
        return false;
    };
    TIME_PREPOSITIONS.iter().any(|p| {
        before.len() >= p.len()
            && before.is_char_boundary(before.len() - p.len())
            && before[before.len() - p.len()..].eq_ignore_ascii_case(p)
            && before[..before.len() - p.len()]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

// ---- Number words ------------------------------------------------------------

struct Word {
    text: String,
    lower: String,
    start: usize,
    end: usize,
}

struct Match {
    start: usize,
    end: usize,
    replacement: String,
    next_index: usize,
}

fn format_number_words(text: &str) -> String {
    let words: Vec<Word> = WORD
        .find_iter(text)
        .map(|m| Word {
            text: m.as_str().to_string(),
            lower: m.as_str().to_lowercase(),
            start: m.start(),
            end: m.end(),
        })
        .collect();
    if !words.iter().any(|w| is_number_word(&w.lower)) {
        return text.to_string();
    }
    let mut replacements = Vec::new();
    let mut index = 0;
    while index < words.len() {
        if let Some(m) = match_at(index, &words, text) {
            index = m.next_index;
            replacements.push(m);
        } else {
            index += 1;
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for m in replacements {
        out.push_str(&text[cursor..m.start]);
        out.push_str(&m.replacement);
        cursor = m.end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn match_at(index: usize, words: &[Word], text: &str) -> Option<Match> {
    if !is_number_word(&words[index].lower) {
        return None;
    }
    digit_sequence(index, words, text)
        .or_else(|| spoken_time(index, words, text))
        .or_else(|| date_ordinal(index, words, text))
        .or_else(|| {
            let c = cardinal(index, words, text)?;
            contextual_cardinal(&c, words, text)
        })
}

/// Four or more single digits read one by one: a phone number, PIN or code.
/// Seven digits read as a local phone number (555-0143), ten as a full one
/// (415-555-0143).
fn digit_sequence(index: usize, words: &[Word], text: &str) -> Option<Match> {
    let mut digits = String::new();
    let mut end = index;
    while end < words.len() {
        let Some(d) = single_digit(&words[end].lower, end == index) else {
            break;
        };
        if end > index && !is_adjacent(&words[end - 1], &words[end], text, " -") {
            break;
        }
        digits.push(char::from(b'0' + d as u8));
        end += 1;
    }
    if digits.len() < 4 {
        return None;
    }
    let formatted = match digits.len() {
        7 => format!("{}-{}", &digits[..3], &digits[3..]),
        10 => format!("{}-{}-{}", &digits[..3], &digits[3..6], &digits[6..]),
        _ => digits,
    };
    Some(Match {
        start: words[index].start,
        end: words[end - 1].end,
        replacement: formatted,
        next_index: end,
    })
}

/// "three pm" → "3 pm", "ten thirty a.m." → "10:30 a.m.", "six oh five pm" →
/// "6:05 pm". Needs a meridiem, so "at ten thirty" stays words.
fn spoken_time(index: usize, words: &[Word], text: &str) -> Option<Match> {
    let hour = lookup(UNITS, &words[index].lower).filter(|h| (1..=12).contains(h))?;
    let mut next = index + 1;
    let mut minutes = None;
    if next < words.len() && is_adjacent(&words[next - 1], &words[next], text, " ") {
        let word = words[next].lower.as_str();
        let oh_unit = (word == "oh" || word == "o")
            .then(|| words.get(next + 1))
            .flatten()
            .and_then(|w| lookup(UNITS, &w.lower))
            .filter(|u| (1..=9).contains(u));
        if let (Some(unit), true) = (
            oh_unit,
            next + 1 < words.len() && is_adjacent(&words[next], &words[next + 1], text, " "),
        ) {
            minutes = Some(unit);
            next += 2;
        } else if let Some((value, after)) = minute_value(next, words, text) {
            minutes = Some(value);
            next = after;
        }
    }
    if !SPOKEN_MERIDIEM_AFTER.is_match(&text[words[next - 1].end..]) {
        return None;
    }
    let replacement = match minutes {
        Some(m) => format!("{hour}:{m:02}"),
        None => hour.to_string(),
    };
    Some(Match {
        start: words[index].start,
        end: words[next - 1].end,
        replacement,
        next_index: next,
    })
}

fn minute_value(index: usize, words: &[Word], text: &str) -> Option<(u64, usize)> {
    let word = words[index].lower.as_str();
    if let Some(unit) = lookup(UNITS, word).filter(|u| (10..=19).contains(u)) {
        return Some((unit, index + 1));
    }
    let ten = lookup(TENS, word).filter(|t| *t <= 50)?;
    if let Some(next) = words.get(index + 1) {
        if let Some(unit) = lookup(UNITS, &next.lower).filter(|u| (1..=9).contains(u)) {
            if is_adjacent(&words[index], next, text, " -") {
                return Some((ten + unit, index + 2));
            }
        }
    }
    Some((ten, index + 1))
}

/// "March fifteenth" → "March 15th", "the seventeenth of April" → "the 17th
/// of April". Ordinals elsewhere ("the first few sessions") stay.
fn date_ordinal(index: usize, words: &[Word], text: &str) -> Option<Match> {
    let (value, next) = ordinal_value(index, words, text)?;
    if !(1..=31).contains(&value) {
        return None;
    }
    let after_month = index > 0
        && is_month(&words[index - 1])
        && is_adjacent(&words[index - 1], &words[index], text, " ");
    let before_month =
        next + 1 < words.len() && words[next].lower == "of" && is_month(&words[next + 1]);
    if !(after_month || before_month) {
        return None;
    }
    Some(Match {
        start: words[index].start,
        end: words[next - 1].end,
        replacement: format!("{value}{}", ordinal_suffix(value)),
        next_index: next,
    })
}

/// Capitalized only: "you may first check" is not a date.
fn is_month(word: &Word) -> bool {
    MONTHS.contains(&word.lower.as_str())
        && word.text.chars().next().is_some_and(char::is_uppercase)
}

fn ordinal_value(index: usize, words: &[Word], text: &str) -> Option<(u64, usize)> {
    let word = words[index].lower.as_str();
    if let Some(v) = lookup(ORDINALS, word) {
        return Some((v, index + 1));
    }
    let ten = lookup(TENS, word)?;
    let next = words.get(index + 1)?;
    let unit = lookup(ORDINALS, &next.lower).filter(|u| (1..=9).contains(u))?;
    is_adjacent(&words[index], next, text, " -").then_some((ten + unit, index + 2))
}

fn ordinal_suffix(value: u64) -> &'static str {
    if (11..=13).contains(&(value % 100)) {
        return "th";
    }
    match value % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

struct Cardinal {
    value: u64,
    has_scale: bool,
    start_index: usize,
    next_index: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    None,
    Unit,
    Ten,
    Scale,
}

/// Parses "two thousand four hundred and fifty" or "forty-five". Stops where a
/// second number would begin ("one two" is two numbers).
fn cardinal(index: usize, words: &[Word], text: &str) -> Option<Cardinal> {
    let mut total = 0u64;
    let mut current = 0u64;
    let mut has_scale = false;
    let mut last = Kind::None;
    let mut end = index;
    while end < words.len() {
        if end > index && !is_adjacent(&words[end - 1], &words[end], text, " -") {
            break;
        }
        let word = words[end].lower.as_str();
        if let Some(unit) = lookup(UNITS, word) {
            if last == Kind::Unit || (last == Kind::Ten && unit >= 10) || (unit == 0 && end > index)
            {
                break;
            }
            current += unit;
            last = Kind::Unit;
        } else if let Some(ten) = lookup(TENS, word) {
            if last == Kind::Unit || last == Kind::Ten {
                break;
            }
            current += ten;
            last = Kind::Ten;
        } else if let Some(scale) =
            lookup(SCALES, word).filter(|s| end > index && (last != Kind::Scale || *s > 100))
        {
            current = current.max(1) * scale;
            if scale >= 1_000 {
                total += current;
                current = 0;
            }
            has_scale = true;
            last = Kind::Scale;
        } else if word == "and"
            && last == Kind::Scale
            && words.get(end + 1).is_some_and(|n| {
                lookup(UNITS, &n.lower).is_some() || lookup(TENS, &n.lower).is_some()
            })
        {
            end += 1;
            continue;
        } else {
            break;
        }
        end += 1;
    }
    (end > index).then_some(Cardinal {
        value: total + current,
        has_scale,
        start_index: index,
        next_index: end,
    })
}

/// Rewrites a parsed number only where Whisper would write digits: money,
/// percentages and anything with a scale word ("two hundred and fifty").
fn contextual_cardinal(c: &Cardinal, words: &[Word], text: &str) -> Option<Match> {
    let start = words[c.start_index].start;
    let last = &words[c.next_index - 1];
    let next = words.get(c.next_index);
    let next_adjacent = next.is_some_and(|n| is_adjacent(last, n, text, " "));
    if let (Some(n), true) = (next, next_adjacent) {
        if n.lower == "dollars" || n.lower == "dollar" {
            return Some(Match {
                start,
                end: n.end,
                replacement: format!("${}", grouped(c.value)),
                next_index: c.next_index + 1,
            });
        }
        if n.lower == "percent" {
            return Some(Match {
                start,
                end: n.end,
                replacement: format!("{}%", c.value),
                next_index: c.next_index + 1,
            });
        }
    }
    if !c.has_scale {
        return None;
    }
    Some(Match {
        start,
        end: last.end,
        replacement: grouped(c.value),
        next_index: c.next_index,
    })
}

fn is_number_word(word: &str) -> bool {
    lookup(UNITS, word).is_some()
        || lookup(TENS, word).is_some()
        || lookup(ORDINALS, word).is_some()
}

fn single_digit(word: &str, is_first: bool) -> Option<u64> {
    if word == "oh" && !is_first {
        return Some(0);
    }
    lookup(UNITS, word).filter(|u| *u <= 9)
}

/// True when only characters from `separators` sit between two words.
fn is_adjacent(first: &Word, second: &Word, text: &str, separators: &str) -> bool {
    let gap = &text[first.end..second.start];
    !gap.is_empty() && gap.chars().all(|c| separators.contains(c))
}

fn grouped(value: u64) -> String {
    let digits = value.to_string();
    if digits.len() <= 3 {
        return digits;
    }
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (offset, c) in digits.chars().enumerate() {
        if offset > 0 && (digits.len() - offset) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::format_numbers as format;

    #[test]
    fn spoken_digits_become_phone_numbers_and_codes() {
        assert_eq!(
            format("Call me back at five five five zero one four three after 3 pm."),
            "Call me back at 555-0143 after 3 pm."
        );
        assert_eq!(
            format("It's four one five five five five zero one four three."),
            "It's 415-555-0143."
        );
        assert_eq!(format("The code is four oh four two."), "The code is 4042.");
        assert_eq!(
            format("Pick one two or three of them."),
            "Pick one two or three of them."
        );
    }

    #[test]
    fn dotted_times() {
        assert_eq!(
            format("Our flight leaves at 6.45 a.m. on Tuesday."),
            "Our flight leaves at 6:45 a.m. on Tuesday."
        );
        assert_eq!(format("See you at 10.30pm."), "See you at 10:30pm.");
        assert_eq!(
            format("Flight 214 departs at 6.45 a.m. and lands in Denver at 8.05 local time."),
            "Flight 214 departs at 6:45 a.m. and lands in Denver at 8:05 local time."
        );
        assert_eq!(
            format("Back around 2.15 this afternoon."),
            "Back around 2:15 this afternoon."
        );
        assert_eq!(
            format("Interest rates sat at 4.25 percent."),
            "Interest rates sat at 4.25 percent."
        );
        assert_eq!(
            format("It closed at 3.15 yesterday."),
            "It closed at 3.15 yesterday."
        );
        assert_eq!(
            format("Rates fell to 4.25 percent over 26.2 miles."),
            "Rates fell to 4.25 percent over 26.2 miles."
        );
        assert_eq!(format("Version 1.11.1 shipped."), "Version 1.11.1 shipped.");
    }

    #[test]
    fn spoken_times() {
        assert_eq!(format("Call me after three pm."), "Call me after 3 pm.");
        assert_eq!(
            format("It starts at seven P.M. sharp."),
            "It starts at 7 P.M. sharp."
        );
        assert_eq!(format("Moved to ten thirty am."), "Moved to 10:30 am.");
        assert_eq!(
            format("Leaves at six forty-five a.m."),
            "Leaves at 6:45 a.m."
        );
        assert_eq!(format("Lands at eight oh five pm."), "Lands at 8:05 pm.");
        assert_eq!(
            format("Meet me at ten thirty tomorrow."),
            "Meet me at ten thirty tomorrow."
        );
    }

    #[test]
    fn money_percent_and_scaled_counts() {
        assert_eq!(
            format("The invoice is two thousand four hundred and fifty dollars."),
            "The invoice is $2,450."
        );
        assert_eq!(format("It costs forty dollars."), "It costs $40.");
        assert_eq!(
            format("Startup time dropped by thirty percent."),
            "Startup time dropped by 30%."
        );
        assert_eq!(format("I'm one hundred percent sure."), "I'm 100% sure.");
        assert_eq!(
            format("We need two hundred and fifty chairs."),
            "We need 250 chairs."
        );
        assert_eq!(
            format("We onboarded one thousand eight hundred forty-seven users."),
            "We onboarded 1,847 users."
        );
        assert_eq!(
            format("Twelve of the fifteen pairs met twice, with forty tables."),
            "Twelve of the fifteen pairs met twice, with forty tables."
        );
    }

    #[test]
    fn date_ordinals() {
        assert_eq!(
            format("It is due on March fifteenth."),
            "It is due on March 15th."
        );
        assert_eq!(
            format("Tuesday, October twenty-eighth."),
            "Tuesday, October 28th."
        );
        assert_eq!(
            format("From April third to April seventeenth."),
            "From April 3rd to April 17th."
        );
        assert_eq!(
            format("Moved from the 3rd of March to the 17th of April."),
            "Moved from the 3rd of March to the 17th of April."
        );
        assert_eq!(
            format("She turns seven on the twenty eighth of September."),
            "She turns seven on the 28th of September."
        );
        assert_eq!(
            format("After the first few sessions, the second lesson runs long."),
            "After the first few sessions, the second lesson runs long."
        );
        assert_eq!(format("May the first one win."), "May the first one win.");
        assert_eq!(
            format("You may first check the logs."),
            "You may first check the logs."
        );
    }

    #[test]
    fn unchanged_text() {
        let text = "Send the contract draft to Folasade and copy the legal team.";
        assert_eq!(format(text), text);
        let text = "The invoice total comes to $1,250.50, due on the 15th at 3:30 pm.";
        assert_eq!(format(text), text);
    }

    #[test]
    fn real_voice_outputs_from_the_eval() {
        assert_eq!(
            format("The invoice is two thousand four hundred and fifty dollars and it is due on March fifteenth."),
            "The invoice is $2,450 and it is due on March 15th."
        );
        assert_eq!(
            format("The meeting moved from April third to April seventeenth at ten thirty a.m."),
            "The meeting moved from April 3rd to April 17th at 10:30 a.m."
        );
        assert_eq!(
            format("Book a table for six people at seven pm on Friday."),
            "Book a table for six people at 7 pm on Friday."
        );
        assert_eq!(
            format("Our meeting moved to the 17th of April at 3.30 p.m."),
            "Our meeting moved to the 17th of April at 3:30 p.m."
        );
    }
}
