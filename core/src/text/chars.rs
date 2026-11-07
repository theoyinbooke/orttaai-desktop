//! Character classes shared by the text passes (the Swift originals use ICU
//! classes such as `[\p{L}\p{N}_]`).

/// A space or a tab (never a line break).
pub fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Swift's `Character.isNewline`.
pub fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// `[\p{L}\p{N}_]`: part of a word.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Lowercase, diacritic-free form of one character (Swift
/// `folding(options: [.diacriticInsensitive, .caseInsensitive])`). Combining
/// marks fold to nothing.
pub fn fold(c: char, out: &mut String) {
    for lower in c.to_lowercase() {
        if ('\u{0300}'..='\u{036F}').contains(&lower) {
            continue;
        }
        out.push(strip_diacritic(lower));
    }
}

fn strip_diacritic(c: char) -> char {
    match c {
        'à'..='å'
        | 'ā'
        | 'ă'
        | 'ą'
        | 'ǎ'
        | 'ạ'
        | 'ả'
        | 'ấ'
        | 'ầ'
        | 'ẩ'
        | 'ẫ'
        | 'ậ'
        | 'ắ'
        | 'ằ'
        | 'ẳ'
        | 'ẵ'
        | 'ặ' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' | 'ḍ' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'ẹ' | 'ẻ' | 'ẽ' | 'ế' | 'ề' | 'ể' | 'ễ' | 'ệ' => {
            'e'
        }
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' | 'ǐ' | 'ỉ' | 'ị' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ǹ' | 'ṅ' => 'n',
        'ò'..='ö'
        | 'ø'
        | 'ō'
        | 'ŏ'
        | 'ő'
        | 'ǒ'
        | 'ọ'
        | 'ỏ'
        | 'ố'
        | 'ồ'
        | 'ổ'
        | 'ỗ'
        | 'ộ'
        | 'ớ'
        | 'ờ'
        | 'ở'
        | 'ỡ'
        | 'ợ'
        | 'ơ' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' | 'ṣ' => 's',
        'ţ' | 'ť' | 'ŧ' | 'ț' | 'ṭ' => 't',
        'ù'..='ü'
        | 'ũ'
        | 'ū'
        | 'ŭ'
        | 'ů'
        | 'ű'
        | 'ų'
        | 'ǔ'
        | 'ư'
        | 'ụ'
        | 'ủ'
        | 'ứ'
        | 'ừ'
        | 'ử'
        | 'ữ'
        | 'ự' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' | 'ỳ' | 'ỵ' | 'ỷ' | 'ỹ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

/// Lowercase, diacritic-free letters and digits only.
pub fn skeleton(text: &str) -> Vec<char> {
    let mut folded = String::with_capacity(text.len());
    for c in text.chars() {
        fold(c, &mut folded);
    }
    folded.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Plain Levenshtein distance over characters (Mac `WordBoostGuard.editDistance`).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// Trimmed, lowercased, inner-whitespace-collapsed key (Mac
/// `PersonalMemoryNormalizer.normalizedKey`).
pub fn normalized_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_folds_case_and_diacritics() {
        assert_eq!(
            skeleton("Olúwasèyí"),
            "oluwaseyi".chars().collect::<Vec<_>>()
        );
        assert_eq!(
            skeleton("Ṣadé-Ọlá 2"),
            "sadeola2".chars().collect::<Vec<_>>()
        );
        assert_eq!(skeleton("e\u{0301}"), vec!['e']);
    }

    #[test]
    fn edit_distance_is_levenshtein() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
    }

    #[test]
    fn normalized_key_collapses_whitespace() {
        assert_eq!(normalized_key("  My   Email\n"), "my email");
    }
}
