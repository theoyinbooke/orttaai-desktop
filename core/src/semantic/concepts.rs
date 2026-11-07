//! Per-dictation concept extraction (Mac `ConceptExtractor`) and dictionary
//! term matching (Mac `DictionaryTermMatcher`). Results are cached per
//! dictation in `concept_extraction`, keyed by text hash + extractor version.

use super::analyzer;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// What a concept is. Drives color, ranking weight, and how aliases merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ConceptKind {
    Person,
    Organization,
    Place,
    /// Products, projects, tools and other proper names.
    Product,
    Topic,
}

impl ConceptKind {
    pub const ALL: [ConceptKind; 5] = [
        ConceptKind::Person,
        ConceptKind::Organization,
        ConceptKind::Place,
        ConceptKind::Product,
        ConceptKind::Topic,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            ConceptKind::Person => "Person",
            ConceptKind::Organization => "Organization",
            ConceptKind::Place => "Place",
            ConceptKind::Product => "Project or product",
            ConceptKind::Topic => "Topic",
        }
    }

    /// Named things outrank generic topics when they tie on mentions.
    pub fn rank_boost(self) -> f64 {
        match self {
            ConceptKind::Person => 1.45,
            ConceptKind::Product => 1.35,
            ConceptKind::Organization => 1.3,
            ConceptKind::Place => 1.1,
            ConceptKind::Topic => 1.0,
        }
    }

    /// Precedence when the same key is tagged with different kinds.
    pub fn precedence(self) -> i32 {
        match self {
            ConceptKind::Person => 4,
            ConceptKind::Product => 3,
            ConceptKind::Organization => 2,
            ConceptKind::Place => 1,
            ConceptKind::Topic => 0,
        }
    }
}

/// One concept found in one dictation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExtractedConcept {
    pub key: String,
    pub title: String,
    pub kind: ConceptKind,
}

/// Bump when extraction output changes so cached results are recomputed.
pub const EXTRACTOR_VERSION: i64 = 3;
pub const MAX_TOPICS_PER_DICTATION: usize = 6;
pub const MAX_NAMES_PER_DICTATION: usize = 8;

fn set(words: &'static str) -> HashSet<&'static str> {
    words.split_whitespace().collect()
}

fn name_edge_words() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set("it its can could you your we they he she i is are was be make made do does did what how \
        why when where which who the a an and or but so to of in on at for with from by as that this \
        these those there then now just also please okay ok yes no nope yeah hey let lets let's will \
        would should have has had get got go going")
    })
}

fn common_capitalized() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set(
            "i'm i've i'll i'd okay ok the and but you your yours we they he she it this that \
        monday tuesday wednesday thursday friday saturday sunday january february march april may \
        june july august september october november december english mac macos ios api url pdf ui \
        ux ai usa god mr mrs ms dr svg png jpg json css html xml sdk cli vps llm ide os id faq csv \
        http https www com app what",
        )
    })
}

/// Nouns that are everyday vocabulary rather than a subject.
pub fn low_signal_nouns() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set(
            "app application experience button page card user change code mode model agent line \
        area rest detail look space goal text date job error file image menu issue feature option \
        description action row screen component system plan base message chart color colour \
        setting icon content template person people form banner test data scenario hand side \
        default clip check ticket platform drop tab generation device update list view section \
        window field item name word sentence question answer problem solution result step process \
        work task context information info level value status state flow part term note link \
        title label box input output response request prompt version build feedback support help \
        need use number reason sense idea point fact case example end top bottom middle front back \
        left right stuff way thing lot bit kind sort type bunch couple guy man woman folk \
        everybody anybody somebody nobody month year minute second hour week day time moment \
        today tomorrow yesterday morning night while place size shape style tool function method \
        class variable string object property logic structure approach choice quality attempt \
        instance mistake fix bug",
        )
    })
}

fn stop_keys() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set(
            "you your we they he she it this that what how svg png jpg json css html xml sdk cli \
        api url pdf ui ux ai vps llm ide id csv i im ive okay ok yeah hey hello hi thanks thank \
        please sorry sure right well so um uh number sense reason end side top bottom level type \
        stuff anything everything something nothing someone",
        )
    })
}

/// Identity for a concept: case, possessives and a trailing " app" never
/// split one thing into two ("Meetumo app", "meetumo's" → "meetumo").
pub fn normalized_key(title: &str, kind: ConceptKind) -> String {
    let lowered = title.to_lowercase().replace('’', "'");
    // Drop "'s" at a word boundary.
    let chars: Vec<char> = lowered.chars().collect();
    let mut no_poss = String::with_capacity(lowered.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\''
            && chars.get(i + 1) == Some(&'s')
            && chars
                .get(i + 2)
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true)
        {
            i += 2;
            continue;
        }
        no_poss.push(chars[i]);
        i += 1;
    }
    let mut key = String::with_capacity(no_poss.len());
    let mut last_space = true;
    for c in no_poss.chars() {
        if c.is_alphanumeric() {
            key.push(c);
            last_space = false;
        } else if !last_space {
            key.push(' ');
            last_space = true;
        }
    }
    let mut key = key.trim().to_string();
    if kind != ConceptKind::Topic && key.ends_with(" app") && key.chars().count() > 5 {
        key.truncate(key.len() - 4);
    }
    key
}

/// Trimmed, whitespace-collapsed, first letter uppercased.
pub fn display_title(raw: &str) -> String {
    let trimmed = raw
        .trim_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(f) if f.is_lowercase() => f.to_uppercase().collect::<String>() + chars.as_str(),
        _ => trimmed,
    }
}

/// Name taggers glue neighbors onto names ("Twitter it", "MCP can"). Trim
/// function words and stray letters from both ends.
pub fn trimmed_name(raw: &str) -> Option<String> {
    let mut tokens: Vec<String> = raw.split_whitespace().map(String::from).collect();
    let is_junk = |t: &str| {
        let lower = t
            .to_lowercase()
            .trim_matches(|c: char| c.is_ascii_punctuation())
            .to_string();
        lower.chars().count() <= 1 || name_edge_words().contains(lower.as_str())
    };
    while tokens.last().map(|t| is_junk(t)).unwrap_or(false) {
        tokens.pop();
    }
    while tokens.first().map(|t| is_junk(t)).unwrap_or(false) {
        tokens.remove(0);
    }
    tokens.retain(|t| t.chars().count() > 1);
    if tokens.is_empty() {
        return None;
    }
    let joined = tokens.join(" ");
    if common_capitalized().contains(joined.to_lowercase().as_str()) {
        None
    } else {
        Some(joined)
    }
}

/// Capitalized words not at the start of a sentence — in dictated text almost
/// always names ("we shipped Orttaai to Vercel").
pub fn proper_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for sentence in analyzer::sentences(text) {
        let words: Vec<&str> = sentence
            .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’' || c == '-'))
            .filter(|w| !w.is_empty())
            .collect();
        for (index, raw) in words.iter().enumerate().skip(1) {
            let word = raw.trim_matches(|c| c == '\'' || c == '’' || c == '-');
            let count = word.chars().count();
            let Some(first) = word.chars().next() else {
                continue;
            };
            if count < 3
                || !first.is_uppercase()
                || !(word.chars().skip(1).any(char::is_lowercase) || count <= 5)
                || common_capitalized().contains(word.to_lowercase().as_str())
            {
                continue;
            }
            if !seen.insert(word.to_lowercase()) {
                continue;
            }
            if let Some(next) = words.get(index + 1) {
                if next.chars().next().map(char::is_uppercase).unwrap_or(false) {
                    continue;
                }
            }
            let previous = words[index - 1];
            if previous
                .chars()
                .next()
                .map(char::is_uppercase)
                .unwrap_or(false)
                && index > 1
            {
                continue;
            }
            names.push(word.to_string());
        }
    }
    names
}

/// The concepts in one dictation, named things first.
pub fn concepts(text: &str) -> Vec<ExtractedConcept> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut by_key: HashMap<String, ExtractedConcept> = HashMap::new();
    let mut add = |title: &str, kind: ConceptKind, lemma_key: Option<&str>| {
        let name = if kind == ConceptKind::Topic {
            Some(title.to_string())
        } else {
            trimmed_name(title)
        };
        let Some(name) = name else {
            return;
        };
        let cleaned = display_title(&name);
        let key = match lemma_key {
            Some(k) => normalized_key(k, kind),
            None => normalized_key(&cleaned, kind),
        };
        if key.chars().count() < 2 || stop_keys().contains(key.as_str()) {
            return;
        }
        if let Some(existing) = by_key.get(&key) {
            if existing.kind.precedence() >= kind.precedence() {
                return;
            }
        }
        by_key.insert(
            key.clone(),
            ExtractedConcept {
                key,
                title: cleaned,
                kind,
            },
        );
    };

    for entity in analyzer::named_entities(trimmed, MAX_NAMES_PER_DICTATION) {
        let kind = match entity.category.as_str() {
            "Person" => ConceptKind::Person,
            "Organization" => ConceptKind::Organization,
            "Place" => ConceptKind::Place,
            _ => ConceptKind::Product,
        };
        add(&entity.title, kind, None);
    }
    for name in proper_names(trimmed)
        .into_iter()
        .take(MAX_NAMES_PER_DICTATION)
    {
        add(&name, ConceptKind::Product, None);
    }
    for topic in analyzer::topic_concepts(trimmed, MAX_TOPICS_PER_DICTATION * 2) {
        let informative = topic
            .key
            .split(' ')
            .any(|t| !low_signal_nouns().contains(t));
        if !informative {
            continue;
        }
        add(&topic.title, ConceptKind::Topic, Some(&topic.key));
    }

    // A phrase covers its words: "design system" makes "design" redundant.
    let phrase_tokens: HashSet<String> = by_key
        .keys()
        .filter(|k| k.contains(' '))
        .flat_map(|k| k.split(' ').map(String::from).collect::<Vec<_>>())
        .collect();
    by_key.retain(|key, c| {
        !(c.kind == ConceptKind::Topic && !key.contains(' ') && phrase_tokens.contains(key))
    });
    let mut topics: Vec<String> = by_key
        .values()
        .filter(|c| c.kind == ConceptKind::Topic)
        .map(|c| c.key.clone())
        .collect();
    topics.sort_by(|a, b| (!a.contains(' '), a.as_str()).cmp(&(!b.contains(' '), b.as_str())));
    for extra in topics.into_iter().skip(MAX_TOPICS_PER_DICTATION) {
        by_key.remove(&extra);
    }

    let mut out: Vec<ExtractedConcept> = by_key.into_values().collect();
    out.sort_by(|a, b| {
        b.kind
            .precedence()
            .cmp(&a.kind.precedence())
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

/// Finds the user's dictionary terms in dictation text: names they cared
/// enough to teach Orttaai count as named concepts with canonical spelling.
#[derive(Debug, Clone, Default)]
pub struct DictionaryTermMatcher {
    patterns: Vec<(ExtractedConcept, String)>,
}

impl DictionaryTermMatcher {
    pub fn new(terms: &[String]) -> Self {
        let mut seen = HashSet::new();
        let patterns = terms
            .iter()
            .filter_map(|raw| {
                let title = display_title(raw);
                let key = normalized_key(&title, ConceptKind::Product);
                if key.chars().count() < 3 || !seen.insert(key.clone()) {
                    return None;
                }
                let needle = raw.trim().to_lowercase();
                if needle.is_empty() {
                    return None;
                }
                Some((
                    ExtractedConcept {
                        key,
                        title,
                        kind: ConceptKind::Product,
                    },
                    needle,
                ))
            })
            .collect();
        Self { patterns }
    }

    /// Normalized keys of the terms (preferred spellings when merging variants).
    pub fn preferred_keys(&self) -> HashSet<String> {
        self.patterns.iter().map(|(c, _)| c.key.clone()).collect()
    }

    pub fn merging(&self, concepts: &[ExtractedConcept], text: &str) -> Vec<ExtractedConcept> {
        if self.patterns.is_empty() {
            return concepts.to_vec();
        }
        let mut result = concepts.to_vec();
        let haystack = text.to_lowercase();
        for (concept, needle) in &self.patterns {
            if !contains_bounded(&haystack, needle) {
                continue;
            }
            if let Some(existing) = result.iter_mut().find(|c| c.key == concept.key) {
                if existing.kind.precedence() < concept.kind.precedence()
                    || existing.kind == ConceptKind::Topic
                {
                    *existing = concept.clone();
                }
            } else {
                result.push(concept.clone());
            }
        }
        result
    }
}

/// `needle` occurs in `haystack` with no letter/digit on either side.
pub fn contains_bounded(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(pos) = haystack[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true);
        let after_ok = haystack[end..]
            .chars()
            .next()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        from = start + needle.chars().next().map(char::len_utf8).unwrap_or(1);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_keys() {
        assert_eq!(
            normalized_key("Meetumo app", ConceptKind::Product),
            "meetumo"
        );
        assert_eq!(normalized_key("meetumo’s", ConceptKind::Product), "meetumo");
        assert_eq!(
            normalized_key("Design-System!", ConceptKind::Topic),
            "design system"
        );
        assert_eq!(normalized_key("Todo app", ConceptKind::Topic), "todo app");
    }

    #[test]
    fn trims_glued_words() {
        assert_eq!(trimmed_name("Twitter it").as_deref(), Some("Twitter"));
        assert_eq!(trimmed_name("Groq g r o q").as_deref(), Some("Groq"));
        assert_eq!(trimmed_name("the it"), None);
        assert_eq!(trimmed_name("Monday"), None);
    }

    #[test]
    fn finds_single_word_proper_names() {
        let names = proper_names("We shipped Orttaai to Vercel today. Monday was busy.");
        assert_eq!(names, ["Orttaai", "Vercel"]);
    }

    #[test]
    fn extracts_kinds_with_phrases_covering_words() {
        let c = concepts(
            "I talked with Sarah about the design system for Orttaai. \
             The design system needs a new color palette before launch.",
        );
        let get = |k: &str| c.iter().find(|x| x.key == k).map(|x| x.kind);
        assert_eq!(get("sarah"), Some(ConceptKind::Person));
        assert_eq!(get("orttaai"), Some(ConceptKind::Product));
        assert_eq!(get("design system"), Some(ConceptKind::Topic));
        assert_eq!(get("design"), None, "phrase covers its words: {c:?}");
        // Named things sort before topics.
        assert!(c.first().unwrap().kind != ConceptKind::Topic);
        assert!(c.iter().filter(|x| x.kind == ConceptKind::Topic).count() <= 6);
    }

    #[test]
    fn dictionary_terms_win() {
        let m = DictionaryTermMatcher::new(&["Meetumo".into(), "x".into()]);
        let merged = m.merging(
            &[ExtractedConcept {
                key: "meetumo".into(),
                title: "Meetumo".into(),
                kind: ConceptKind::Topic,
            }],
            "the meetumo launch",
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].kind, ConceptKind::Product);
        assert!(m.merging(&[], "meetumoz").is_empty());
        assert!(contains_bounded("a meetumo, b", "meetumo"));
        assert_eq!(m.preferred_keys().len(), 1);
    }
}
