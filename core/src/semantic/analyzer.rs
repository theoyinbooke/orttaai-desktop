//! Portable replacement for Apple's NaturalLanguage framework as the Mac's
//! `SemanticTextAnalyzer` uses it: tokenization, sentence splitting, a
//! lexicon-and-suffix part-of-speech guess (nouns only), plural lemmas,
//! noun-phrase topic extraction, and person/organization/place heuristics with
//! the Mac's capitalized-run fallback. Deterministic — no model, no network.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// A concept extracted from dictated text (Mac `SemanticConcept`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemanticConcept {
    /// Canonical key (lemmatized, lowercased) used for identity.
    pub key: String,
    /// Display form ("Memory Graph").
    pub title: String,
}

/// A named entity (Mac `SemanticNamedEntity`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamedEntity {
    pub key: String,
    pub title: String,
    /// "Person", "Place", "Organization", or "Name" for the capitalized-run fallback.
    pub category: String,
}

/// One word of a sentence, with the clause boundary that precedes it.
#[derive(Debug, Clone)]
pub struct Token {
    /// As written, edge apostrophes/hyphens trimmed.
    pub surface: String,
    pub lower: String,
    /// A comma, colon, bracket… sits between this token and the previous one.
    pub after_break: bool,
    /// First word of its sentence.
    pub sentence_start: bool,
    /// The raw word ended with `.`, `!` or `?`.
    pub ends_sentence: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '’' || c == '-'
}

/// Split text into word tokens, remembering clause breaks and sentence starts.
pub fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut pending_break = false;
    let mut sentence_start = true;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let flush = |current: &mut String,
                 out: &mut Vec<Token>,
                 pending_break: &mut bool,
                 sentence_start: &mut bool,
                 ends: bool| {
        let trimmed = current.trim_matches(|c| c == '\'' || c == '’' || c == '-');
        if !trimmed.is_empty() {
            out.push(Token {
                surface: trimmed.to_string(),
                lower: trimmed.to_lowercase().replace('’', "'"),
                after_break: *pending_break,
                sentence_start: *sentence_start,
                ends_sentence: ends,
            });
            *pending_break = false;
            *sentence_start = ends;
        }
        current.clear();
    };
    while i < chars.len() {
        let c = chars[i];
        if is_word_char(c) {
            current.push(c);
        } else {
            let ends = matches!(c, '.' | '!' | '?' | '\n')
                && !(c == '.'
                    && i + 1 < chars.len()
                    && chars[i + 1].is_alphanumeric()
                    && !current.is_empty());
            if c == '.' && !ends {
                // "v1.2", "e.g", "node.js" stay one word.
                current.push(c);
                i += 1;
                continue;
            }
            let had_word = !current.is_empty();
            flush(
                &mut current,
                &mut out,
                &mut pending_break,
                &mut sentence_start,
                ends,
            );
            if ends {
                sentence_start = true;
                pending_break = true;
                if !had_word {
                    if let Some(last) = out.last_mut() {
                        last.ends_sentence = true;
                    }
                }
            } else if !c.is_whitespace() {
                pending_break = true;
            }
        }
        i += 1;
    }
    flush(
        &mut current,
        &mut out,
        &mut pending_break,
        &mut sentence_start,
        false,
    );
    out
}

/// Sentences (terminal punctuation kept), trimmed, empty ones dropped.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            let s = current.trim();
            if !s.is_empty() {
                out.push(s.to_string());
            }
            current.clear();
            continue;
        }
        current.push(c);
        if matches!(c, '.' | '!' | '?') {
            let next = chars.get(i + 1).copied();
            let boundary = match next {
                None => true,
                Some(n) => n.is_whitespace() || n == '"' || n == '”' || n == ')',
            };
            // Keep runs like "?!" or "..." together.
            let next_is_punct = matches!(next, Some('.' | '!' | '?'));
            if boundary && !next_is_punct && !is_abbreviation(&current) {
                let s = current.trim();
                if !s.is_empty() {
                    out.push(s.to_string());
                }
                current.clear();
            }
        }
    }
    let s = current.trim();
    if !s.is_empty() {
        out.push(s.to_string());
    }
    out
}

fn is_abbreviation(sentence_so_far: &str) -> bool {
    let last = sentence_so_far
        .trim_end_matches('.')
        .rsplit(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("")
        .to_lowercase();
    matches!(
        last.as_str(),
        "mr" | "mrs" | "ms" | "dr" | "st" | "vs" | "etc" | "e.g" | "i.e" | "approx" | "no"
    ) && !sentence_so_far.ends_with("..")
}

// ---- Part of speech ---------------------------------------------------------

fn set(words: &'static str) -> HashSet<&'static str> {
    words.split_whitespace().collect()
}

/// Words that are never nouns in dictated speech: function words, pronouns,
/// auxiliaries, common verbs, adjectives, adverbs, interjections.
fn non_nouns() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set("a an the this that these those there here then than and but or nor yet so if \
        because although though while whereas unless until since as of in on at to for with from by \
        about into onto upon over under above below between among through during before after around \
        against without within along across behind beyond toward towards via per off out up down away \
        back i me my mine myself we us our ours ourselves you your yours yourself yourselves he him his \
        himself she her hers herself it its itself they them their theirs themselves who whom whose which \
        what whatever whoever whichever when where why how whether one ones someone anyone everyone \
        nobody somebody anybody everybody something anything everything nothing somewhere anywhere \
        everywhere nowhere each every either neither both all any some many much more most few fewer \
        less least several such own other others another same is am are was were be been being have has \
        had having do does did doing done will would shall should can could may might must ought \
        i'm i've i'll i'd you're you've you'll you'd we're we've we'll we'd they're they've they'll \
        they'd he's she's it's that's there's here's what's who's let's don't doesn't didn't won't \
        wouldn't can't cannot couldn't shouldn't isn't aren't wasn't weren't haven't hasn't hadn't \
        mustn't gonna wanna gotta kinda sorta lemme ain't y'all \
        make makes made making get gets got gotten getting go goes went gone come comes came take takes \
        took taken give gives gave given know knows knew known think thinks thought say says said \
        tell tells told want wants see sees saw seen seem seems seemed try tries tried let lets put \
        puts keep keeps kept begin begins began begun feel feels felt leave leaves bring brings brought \
        hear hears heard become becomes became mean means meant find finds found ask asks asked \
        like likes liked love loves loved hate hates hated wish wishes hope hopes hoped guess guessed \
        believe believes believed remember remembers remembered forget forgets forgot forgotten \
        understand understands understood realize realized realise realised notice noticed \
        decide decides decided agree agrees agreed suppose supposed expect expects expected \
        happen happens happened include includes included allow allows allowed provide provides \
        provided create creates created add adds added remove removes removed delete deletes deleted \
         fixes fixed implement implements implemented generate generates generated ensure ensures \
        ensured verify verifies verified investigate  reviewed explain explains explained \
        rename renamed refactor refactored install installs installed  opens opened  closes \
        closed  writes wrote written read reads  sends sent sit sat stand stood speak spoke \
        spoken talk talks talked walk walked  watched wait waited  stops stopped  starts \
        started finish finishes finished continue continues continued  moves moved  turns \
        turned  sets setting's apply applies applied enable enables enabled disable disabled \
         clicks clicked  typed press pressed scroll scrolled select selected choose chose \
        chosen pick picked drag dragged  shared  saved  loaded  pasted  copied \
         held lose lost win won pay paid buy bought sell sold spend spent  meet met \
        seemingly really very quite rather too also just only even still already almost always never \
        ever often sometimes usually maybe perhaps probably actually basically literally honestly \
        definitely certainly obviously simply exactly especially generally finally recently currently \
        again once twice now today tomorrow yesterday tonight soon later earlier yet ago anyway anyhow \
        otherwise instead however therefore thus hence meanwhile besides furthermore moreover indeed \
        well yes yeah yep yup no nope nah okay ok alright hey hi hello oh ah uh um hmm wow please thanks \
        thank sorry sure right fine cool great good bad better best worse worst big bigger biggest small \
        smaller smallest large larger largest little long longer short shorter high higher low lower \
        new newer newest old older oldest young first second third last next previous early late easy \
        easier hard harder simple difficult possible impossible important different similar same real \
        true false full empty whole entire certain clear able unable available ready happy sad nice \
        beautiful awesome amazing perfect wrong correct proper main major minor general specific \
        particular current recent likely unlikely free  closed public private local global quick \
        quickly slow slowly fast faster fastest strong weak hot cold warm cool dark light heavy huge tiny \
        major key top bottom left-hand right-hand overall final initial basic advanced extra sure \
        capable comfortable reliable scalable usable readable valuable suitable reasonable probable \
        notable stable unstable acceptable adjustable configurable searchable editable sortable \
        clickable doable noticeable predictable affordable sustainable responsive interactive \
        intuitive effective expensive cheap exclusive inclusive extensive impressive aggressive \
        positive negative native relative creative active inactive massive attractive \
        two three four five six seven eight nine ten eleven twelve twenty thirty forty fifty hundred \
        thousand million billion dozen half \
        compare describe summarize summarise translate handle prepare organize organise clean cook \
        remind invite ask asks asking tell telling telled discuss discussed mention mentioned \
        suggest suggested recommend recommended consider considered improve improved reduce \
        increase avoid avoided include contain contains depend depends happen allow enjoy \
        wonder wondering worry worried broken stuck done gone wrong lost blocked busy \
        sorry glad weird strange tired")
    })
}

/// Words that are nouns or verbs depending on context ("the " vs " it").
fn ambiguous() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set(
            "work plan test change use need look help call email build update run show feel \
        play check list view report design process record order answer question return search \
        sign support release launch deploy push pull merge commit message text post note link \
        file note track trade price cost charge book hand mind point name place face end start \
        stop turn move step watch wait talk walk break drive ship fix sort lead act count cut \
        set share save load paste copy edit draft review fund host brand market script schedule \
        upgrade import export sync input output request respond reply sketch map chart picture \
        control access block lock store stream train guide experience need close open hold \
        click write send play",
        )
    })
}

/// -ing words that are nouns on their own.
fn gerund_nouns() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set("meeting meetings building buildings onboarding pricing wedding morning evening setting \
        settings painting training booking funding hosting branding marketing engineering ceiling \
        recording recordings embedding embeddings ring king thing things string strings spring \
        wing wings clothing housing heading headings feeling feelings beginning ending offering \
        offerings listing listings drawing drawings filling clipping mapping rendering routing \
        caching logging billing shipping writing writings reading readings warning warnings \
        coding planning testing debugging styling spacing padding tracking ranking rating \
        landing dashboard sibling darling pudding")
    })
}

/// Words before a noun that mark the next ambiguous word as a noun.
fn noun_markers() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set(
            "the a an my our your his her their its this that these those some any each every \
        no new first last next whole entire main of for with about on in our's whose which another \
        other same final initial big small current previous",
        )
    })
}

/// Words before "to" that make the next word an infinitive verb.
const INFINITIVE_LEADS: &[&str] = &[
    "need", "needs", "needed", "want", "wants", "wanted", "have", "has", "had", "going", "able",
    "trying", "try", "tried", "how", "like", "love", "plan", "planning", "planned", "decided",
    "used", "supposed", "got", "ought", "hope", "hoping", "wish", "started", "start", "begin",
    "began", "forgot", "remember", "learn", "learning", "easy", "hard", "ready", "time", "way",
    "help", "helps", "let's",
];

const LY_NOUNS: &[&str] = &[
    "family",
    "reply",
    "supply",
    "assembly",
    "july",
    "italy",
    "ally",
    "rally",
    "butterfly",
    "anomaly",
    "monopoly",
    "belly",
    "jelly",
    "bully",
    "lily",
    "emily",
    "molly",
    "holly",
    "kelly",
    "sally",
    "polly",
];

const ED_NOUNS: &[&str] = &[
    "need", "speed", "seed", "feed", "breed", "greed", "embed", "bed", "shed", "red", "ted",
    "fred", "reed", "steed", "creed", "weed", "bled", "sled", "led", "med", "ed", "bread",
    "thread", "head", "spread", "ahead", "instead", "dread", "tread", "wed", "hundred", "kindred",
];

/// True when `index` in `tokens` reads as a noun (the Mac's `.noun` lexical class).
pub fn is_noun(tokens: &[Token], index: usize) -> bool {
    noun_at(tokens, index, 3)
}

fn noun_at(tokens: &[Token], index: usize, depth: u8) -> bool {
    let token = &tokens[index];
    let lower = token.lower.as_str();
    if lower
        .chars()
        .all(|c| c.is_numeric() || c == '.' || c == '-')
    {
        return false;
    }
    if !lower.chars().any(|c| c.is_alphabetic()) {
        return false;
    }
    // Possessives keep their noun: "Sam's", "the team's".
    let base = lower.strip_suffix("'s").unwrap_or(lower);
    if base != lower {
        return !non_nouns().contains(base);
    }
    if lower.contains('\'') {
        return false;
    }
    if non_nouns().contains(lower) {
        return false;
    }
    let prev = index
        .checked_sub(1)
        .map(|i| &tokens[i])
        .filter(|_| !token.after_break);
    let prev_marks_noun = prev
        .map(|p| noun_markers().contains(p.lower.as_str()))
        .unwrap_or(false);
    // A capitalized word mid-sentence is a proper noun.
    let capitalized = token
        .surface
        .chars()
        .next()
        .map(char::is_uppercase)
        .unwrap_or(false);
    if capitalized && !token.sentence_start {
        return true;
    }
    // "need to migrate", "going to deploy" — an infinitive is a verb.
    if prev.map(|p| p.lower == "to").unwrap_or(false) && index >= 2 {
        let before = tokens[index - 2].lower.as_str();
        if INFINITIVE_LEADS.contains(&before) {
            return false;
        }
    }
    // Whether the previous word is itself a noun ("release plan", "graph shows").
    let prev_is_noun = prev.is_some() && depth > 0 && noun_at(tokens, index - 1, depth - 1);
    let prev_is_plural = prev
        .map(|p| p.lower.ends_with('s') && !p.lower.ends_with("ss") && lemma(&p.lower) != p.lower)
        .unwrap_or(false);
    if ambiguous().contains(lower) {
        // "the plan", "release plan" are nouns; "tokens look", "we plan" are verbs.
        return prev_marks_noun || (prev_is_noun && !prev_is_plural);
    }
    // "the graph shows", "the step needs" — a third-person verb after its subject.
    if lower.ends_with('s') && prev_is_noun && !prev_marks_noun {
        let stem = lemma(lower);
        if ambiguous().contains(stem.as_str()) || non_nouns().contains(stem.as_str()) {
            return false;
        }
    }
    let len = lower.chars().count();
    if lower.ends_with("ing") && len > 4 {
        return gerund_nouns().contains(lower) || prev_marks_noun;
    }
    if lower.ends_with("ings") && len > 5 {
        return true;
    }
    if lower.ends_with("ed") && len > 4 && !ED_NOUNS.contains(&lower) {
        return false;
    }
    if lower.ends_with("ly") && len > 4 && !LY_NOUNS.contains(&lower) {
        return false;
    }
    for suffix in ["ous", "less", "ical", "ish"] {
        if lower.ends_with(suffix) && len > suffix.len() + 2 {
            return false;
        }
    }
    if lower.ends_with("ful") && len > 5 && lower != "handful" && lower != "spoonful" {
        return false;
    }
    if lower.ends_with("ible") && len > 6 && lower != "bible" {
        return false;
    }
    if lower.ends_with("ize") || lower.ends_with("ise") && len > 6 && lower != "enterprise" {
        // "optimize", "realise" — verbs; "promise"/"premise" stay nouns below 7 chars.
        return lower == "size" || lower == "prize" || lower == "exercise" || lower == "expertise";
    }
    true
}

/// Plural → singular lemma (Mac's `.lemma` for nouns).
pub fn lemma(lower: &str) -> String {
    let w = lower.strip_suffix("'s").unwrap_or(lower);
    let irregular = match w {
        "people" => Some("person"),
        "children" => Some("child"),
        "men" => Some("man"),
        "women" => Some("woman"),
        "mice" => Some("mouse"),
        "feet" => Some("foot"),
        "teeth" => Some("tooth"),
        "geese" => Some("goose"),
        "analyses" => Some("analysis"),
        "crises" => Some("crisis"),
        "theses" => Some("thesis"),
        "indices" => Some("index"),
        "criteria" => Some("criterion"),
        "phenomena" => Some("phenomenon"),
        "lives" => Some("life"),
        "wives" => Some("wife"),
        "knives" => Some("knife"),
        "leaves" => Some("leaf"),
        "halves" => Some("half"),
        "shelves" => Some("shelf"),
        _ => None,
    };
    if let Some(i) = irregular {
        return i.to_string();
    }
    let len = w.chars().count();
    const KEEP: &[&str] = &[
        "news",
        "series",
        "species",
        "lens",
        "bus",
        "gas",
        "chaos",
        "canvas",
        "status",
        "census",
        "corpus",
        "campus",
        "bonus",
        "virus",
        "focus",
        "genius",
        "plus",
        "minus",
        "analysis",
        "basis",
        "crisis",
        "thesis",
        "axis",
        "atlas",
        "alias",
        "ios",
        "macos",
        "kubernetes",
        "jeans",
        "pants",
        "glasses",
        "physics",
        "mathematics",
        "economics",
        "politics",
        "ethics",
        "analytics",
        "metrics",
        "graphics",
        "logistics",
        "statistics",
        "electronics",
        "robotics",
        "thanks",
        "always",
        "less",
        "process",
        "access",
        "address",
        "business",
        "success",
        "progress",
        "express",
        "class",
        "glass",
        "grass",
        "pass",
        "boss",
        "loss",
        "cross",
        "dress",
        "stress",
        "chess",
        "mess",
        "guess",
        "kiss",
        "miss",
    ];
    if KEEP.contains(&w) || len <= 3 {
        return w.to_string();
    }
    if let Some(stem) = w.strip_suffix("ies") {
        if len > 4 {
            return format!("{stem}y");
        }
    }
    for suffix in ["sses", "shes", "ches", "xes", "zzes"] {
        if let Some(stem) = w.strip_suffix(suffix) {
            return format!("{stem}{}", &suffix[..suffix.len() - 2]);
        }
    }
    if w.ends_with("ss") || w.ends_with("us") || w.ends_with("is") || w.ends_with("'") {
        return w.to_string();
    }
    if let Some(stem) = w.strip_suffix('s') {
        return stem.to_string();
    }
    w.to_string()
}

/// Nouns too generic to be a life concept in dictated speech.
fn generic_nouns() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set("thing things stuff way ways lot lots bit bits kind kinds sort sorts one ones something \
        anything everything nothing someone anyone everyone guy guys part parts place case point fact \
        idea example second seconds minute minutes moment today tomorrow yesterday tonight morning \
        evening afternoon night day days week weeks hour hours time times okay yeah yes hmm \
        monday tuesday wednesday thursday friday saturday sunday january february march april \
        june july august september october november december weekend weekends")
    })
}

/// Capitalized function words that can't begin a name run at a sentence start.
fn run_function_words() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set("the this that these those there then than and but for nor yet not now also just you \
        your they them their she his her its our was were are will would could should can may might \
        must shall did does doing have has had let lets please make makes take give come look what \
        when where which who whom whose why how yes yeah okay well maybe because after before today \
        tomorrow yesterday here some any all one two very with from into about over under again once \
        email tell ask call message ping text send thank thanks invite remind meet reply help show \
        check fix update add use open try see find get compare explain describe summarize write \
        draft schedule book plan review finish ship deploy run build create delete remove move")
    })
}

fn entity_sentence_starters() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        [
            "the",
            "this",
            "that",
            "these",
            "those",
            "when",
            "where",
            "what",
            "please",
            "today",
            "tomorrow",
            "yesterday",
            "after",
            "before",
            "because",
            "do you",
            "can you",
            "i want",
            "i need",
            "let us",
            "make sure",
        ]
        .into_iter()
        .collect()
    })
}

// ---- Topics ----------------------------------------------------------------

type Register<'a> = dyn FnMut(&[String], &[String], f64) + 'a;

/// Salient noun concepts, most salient first. Nouns and consecutive-noun
/// phrases are counted by lemma; phrases (scored 1.8) outrank single nouns (1.0).
pub fn topic_concepts(text: &str, limit: usize) -> Vec<SemanticConcept> {
    if limit == 0 || text.trim().is_empty() {
        return Vec::new();
    }
    let toks = tokens(text);
    let mut candidates: HashMap<String, (f64, String)> = HashMap::new();
    let mut phrase_lemmas: Vec<String> = Vec::new();
    let mut phrase_surfaces: Vec<String> = Vec::new();

    let mut register = |lemmas: &[String], surfaces: &[String], score: f64| {
        let key = lemmas.join(" ");
        if key.is_empty() {
            return;
        }
        let title = surfaces.join(" ");
        candidates
            .entry(key)
            .and_modify(|c| c.0 += score)
            .or_insert((score, title));
    };
    let flush =
        |lemmas: &mut Vec<String>, surfaces: &mut Vec<String>, register: &mut Register<'_>| {
            if !lemmas.is_empty() {
                for (l, s) in lemmas.iter().zip(surfaces.iter()) {
                    register(std::slice::from_ref(l), std::slice::from_ref(s), 1.0);
                }
                if lemmas.len() >= 2 {
                    let n = lemmas.len();
                    register(&lemmas[n - 2..], &surfaces[n - 2..], 1.8);
                }
            }
            lemmas.clear();
            surfaces.clear();
        };

    for i in 0..toks.len() {
        let t = &toks[i];
        if t.after_break || t.sentence_start {
            flush(&mut phrase_lemmas, &mut phrase_surfaces, &mut register);
        }
        let surface_len = t.surface.chars().count();
        if surface_len < 3 || !is_noun(&toks, i) {
            flush(&mut phrase_lemmas, &mut phrase_surfaces, &mut register);
            continue;
        }
        // Proper nouns keep their spelling ("Postgres" is not a plural).
        let proper = !t.sentence_start
            && t.surface
                .chars()
                .next()
                .map(char::is_uppercase)
                .unwrap_or(false);
        let lem = if proper {
            t.lower.strip_suffix("'s").unwrap_or(&t.lower).to_string()
        } else {
            lemma(&t.lower)
        };
        if lem.chars().count() < 3 || generic_nouns().contains(lem.as_str()) {
            flush(&mut phrase_lemmas, &mut phrase_surfaces, &mut register);
            continue;
        }
        phrase_lemmas.push(lem);
        phrase_surfaces.push(t.lower.strip_suffix("'s").unwrap_or(&t.lower).to_string());
    }
    flush(&mut phrase_lemmas, &mut phrase_surfaces, &mut register);

    let mut ranked: Vec<(String, (f64, String))> = candidates.into_iter().collect();
    ranked.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(key, (_, title))| SemanticConcept {
            key,
            title: capitalize_words(&title),
        })
        .collect()
}

/// "memory graph" → "Memory Graph" (Swift `.capitalized`).
pub fn capitalize_words(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- Entities --------------------------------------------------------------

fn first_names() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set("james john robert michael william david richard joseph thomas charles christopher daniel \
        matthew anthony mark donald steven paul andrew joshua kenneth kevin brian george timothy ronald \
        edward jason jeffrey ryan jacob gary nicholas eric jonathan stephen larry justin scott brandon \
        benjamin samuel gregory alexander frank patrick raymond jack dennis jerry tyler aaron jose adam \
        nathan henry douglas zachary peter kyle noah ethan jeremy walter christian keith roger terry \
        austin sean gerald carl harold dylan arthur lawrence jordan jesse bryan billy bruce gabriel joe \
        logan albert willie alan eugene russell vincent philip bobby johnny bradley roy ralph louis \
        mary patricia jennifer linda elizabeth barbara susan jessica sarah karen lisa nancy betty \
        sandra margaret ashley kimberly emily donna michelle carol amanda melissa deborah stephanie \
        dorothy rebecca sharon laura cynthia amy kathleen angela shirley brenda emma anna pamela \
        nicole samantha katherine christine helen debra rachel carolyn janet maria catherine heather \
        diane olivia julie joyce victoria ruth virginia lauren kelly christina joan evelyn judith \
        andrea hannah megan cheryl jacqueline martha madison teresa gloria sara janice ann kathryn \
        abigail sophia frances jean alice judy isabella julia grace amber denise danielle marilyn \
        beverly charlotte natalie theresa diana brittany doris kayla alexis lori marie mia chloe \
        sam alex chris max ben tom tim jim mike dan dave steve nick matt josh jake luke liam lucas \
        mason oliver elijah aiden harper ava amelia ella zoe lily nora leah aria riley layla \
        mohammed muhammad ahmed ali omar hassan fatima aisha yusuf ibrahim khalid layla \
        wei li ming chen yan hui jun hiro yuki kenji sakura haruto raj priya arjun anil sunita \
        rahul amit deepak ravi vikram ananya kofi kwame ama chidi ngozi tunde ade ayo segun \
        emeka femi bola dayo yemi tolu funmi lanre olu juan carlos luis miguel pedro diego \
        sofia lucia elena carmen pablo javier mateo santiago valentina camila hans klaus jurgen \
        lukas leon felix anja katrin ingrid lars erik sven nils astrid pierre jean-luc louis \
        claire camille chloe manon lea ivan dmitri sergei olga natasha anastasia marcus \
        aaron adrian andre angel antonio ashton blake bryce caleb cameron carter chase cole colin \
        connor damian darius derek dominic dylan eli elliot evan ezra gavin grant hayden hunter \
        ian isaac isaiah jared jasper jayden jonah julian kai kaleb kevin landon leo levi lincoln \
        malik marco martin mateus micah miles milo nathaniel nolan omar oscar owen parker preston \
        quinn reid rhys riley roman rowan ryder sebastian silas simon theo tobias travis tristan \
        victor vincent wesley wyatt xavier zane alicia allison alyssa ariana aubrey audrey bella \
        brooke caroline claudia daisy delilah eleanor eliza elise eva everly faith fiona gabriela \
        gemma georgia gianna hailey hazel ivy jade jasmine jenna jocelyn josephine kaitlyn kate \
        katie kylie leila lillian lucy luna lydia mackenzie maya melanie mila molly naomi nina \
        nova paige penelope piper quinn rose ruby sadie savannah scarlett serena sienna skylar \
        sophie stella summer sydney tessa valerie vanessa violet vivian willow zara zoey")
    })
}

fn organizations() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set("google apple microsoft amazon meta facebook openai anthropic netflix tesla nvidia ibm \
        intel amd oracle salesforce adobe spotify uber lyft airbnb twitter linkedin youtube \
        instagram tiktok bytedance samsung sony huawei xiaomi alibaba tencent baidu github gitlab \
        stripe paypal shopify slack zoom dropbox atlassian notion figma canva vercel netlify \
        cloudflare docker mozilla wikipedia reddit discord telegram whatsapp snapchat pinterest \
        deepmind mistral cohere huggingface nasa nato unesco unicef fifa nba nfl bbc cnn reuters \
        harvard stanford mit oxford cambridge yale princeton berkeley walmart target costco ikea \
        nike adidas boeing airbus toyota honda ford bmw mercedes volkswagen audi porsche ferrari \
        visa mastercard goldman jpmorgan blackrock deloitte mckinsey accenture pwc kpmg")
    })
}

fn org_suffixes() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set(
            "inc corp corporation llc ltd limited labs lab technologies technology university \
        college bank foundation institute group company co agency studio studios systems partners \
        capital ventures association council ministry department church hospital school academy \
        holdings industries solutions consulting media networks",
        )
    })
}

fn places() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        [
            "america",
            "usa",
            "canada",
            "mexico",
            "brazil",
            "argentina",
            "chile",
            "peru",
            "colombia",
            "venezuela",
            "uk",
            "england",
            "scotland",
            "wales",
            "ireland",
            "france",
            "germany",
            "spain",
            "portugal",
            "italy",
            "netherlands",
            "belgium",
            "switzerland",
            "austria",
            "sweden",
            "norway",
            "denmark",
            "finland",
            "iceland",
            "poland",
            "ukraine",
            "russia",
            "greece",
            "turkey",
            "egypt",
            "morocco",
            "nigeria",
            "ghana",
            "kenya",
            "ethiopia",
            "south africa",
            "tanzania",
            "uganda",
            "rwanda",
            "senegal",
            "israel",
            "jordan",
            "lebanon",
            "saudi arabia",
            "uae",
            "dubai",
            "qatar",
            "iran",
            "iraq",
            "india",
            "pakistan",
            "bangladesh",
            "china",
            "japan",
            "korea",
            "south korea",
            "taiwan",
            "vietnam",
            "thailand",
            "singapore",
            "malaysia",
            "indonesia",
            "philippines",
            "australia",
            "new zealand",
            "europe",
            "asia",
            "africa",
            "london",
            "paris",
            "berlin",
            "madrid",
            "rome",
            "amsterdam",
            "brussels",
            "vienna",
            "zurich",
            "geneva",
            "stockholm",
            "oslo",
            "copenhagen",
            "helsinki",
            "dublin",
            "edinburgh",
            "manchester",
            "lisbon",
            "barcelona",
            "milan",
            "munich",
            "prague",
            "warsaw",
            "budapest",
            "athens",
            "istanbul",
            "moscow",
            "kyiv",
            "cairo",
            "lagos",
            "abuja",
            "accra",
            "nairobi",
            "johannesburg",
            "cape town",
            "tokyo",
            "osaka",
            "kyoto",
            "seoul",
            "beijing",
            "shanghai",
            "hong kong",
            "shenzhen",
            "mumbai",
            "delhi",
            "new delhi",
            "bangalore",
            "bengaluru",
            "chennai",
            "hyderabad",
            "karachi",
            "lahore",
            "sydney",
            "melbourne",
            "brisbane",
            "perth",
            "auckland",
            "toronto",
            "vancouver",
            "montreal",
            "ottawa",
            "new york",
            "los angeles",
            "san francisco",
            "chicago",
            "boston",
            "seattle",
            "austin",
            "houston",
            "dallas",
            "miami",
            "atlanta",
            "denver",
            "phoenix",
            "portland",
            "philadelphia",
            "washington",
            "las vegas",
            "san diego",
            "san jose",
            "detroit",
            "nashville",
            "brooklyn",
            "manhattan",
            "silicon valley",
            "california",
            "texas",
            "florida",
            "oregon",
            "nevada",
            "arizona",
            "colorado",
            "georgia",
            "ohio",
            "michigan",
            "illinois",
            "virginia",
            "massachusetts",
            "new jersey",
            "pennsylvania",
            "carolina",
            "utah",
            "alaska",
            "hawaii",
        ]
        .into_iter()
        .collect()
    })
}

fn place_suffixes() -> &'static HashSet<&'static str> {
    static S: OnceLock<HashSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        set(
            "city county street avenue road park lake river mountains mountain island islands \
        beach valley bay state province district village town airport square bridge",
        )
    })
}

const HONORIFICS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sir", "madam", "uncle", "aunt",
];

/// Named entities via lexicon NER (person/organization/place), with the Mac's
/// capitalized-run fallback for product/tool names. Most frequent first.
pub fn named_entities(text: &str, limit: usize) -> Vec<NamedEntity> {
    if limit == 0 || text.trim().is_empty() {
        return Vec::new();
    }
    let mut candidates: HashMap<String, (usize, String, String)> = HashMap::new();
    let mut register = |title: &str, category: &str| {
        let trimmed = title.trim_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation());
        let n = trimmed.chars().count();
        if !(3..=64).contains(&n) {
            return;
        }
        let key = trimmed.to_lowercase();
        if entity_sentence_starters().contains(key.as_str()) {
            return;
        }
        candidates
            .entry(key)
            .and_modify(|c| {
                c.0 += 1;
                if c.2 == "Name" && category != "Name" {
                    c.2 = category.to_string();
                }
            })
            .or_insert((1, trimmed.to_string(), category.to_string()));
    };

    for (title, category) in lexicon_entities(text) {
        register(&title, category);
    }
    for run in capitalized_runs(text) {
        register(&run, "Name");
    }

    let mut ranked: Vec<(String, (usize, String, String))> = candidates.into_iter().collect();
    ranked.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(key, (_, title, category))| NamedEntity {
            key,
            title,
            category,
        })
        .collect()
}

/// NER stand-in: capitalized names (joined across runs) classified by
/// lexicons, honorifics, and organization/place suffixes.
fn lexicon_entities(text: &str) -> Vec<(String, &'static str)> {
    let toks = tokens(text);
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let capitalized = t
            .surface
            .chars()
            .next()
            .map(char::is_uppercase)
            .unwrap_or(false);
        if !capitalized || t.lower.contains('\'') && !t.lower.ends_with("'s") {
            i += 1;
            continue;
        }
        // "Tell Marcus…", "Email Sarah Chen…": the sentence-initial verb is not part of the name.
        if t.sentence_start && run_function_words().contains(t.lower.as_str()) {
            i += 1;
            continue;
        }
        // Join a run of capitalized words ("Sam Rivera", "Acme Labs").
        let mut j = i + 1;
        while j < toks.len()
            && !toks[j].after_break
            && !toks[j].sentence_start
            && toks[j]
                .surface
                .chars()
                .next()
                .map(char::is_uppercase)
                .unwrap_or(false)
            && !toks[j - 1].lower.ends_with("'s")
        {
            j += 1;
        }
        let words: Vec<String> = toks[i..j]
            .iter()
            .map(|t| t.surface.trim_end_matches("'s").to_string())
            .collect();
        let lowers: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        let joined_lower = lowers.join(" ");
        let prev = i
            .checked_sub(1)
            .map(|p| toks[p].lower.trim_end_matches('.'));
        let category = if lowers
            .last()
            .map(|l| org_suffixes().contains(l.as_str()))
            .unwrap_or(false)
            && lowers.len() >= 2
            || lowers.iter().any(|l| organizations().contains(l.as_str())) && lowers.len() <= 2
        {
            Some("Organization")
        } else if places().contains(joined_lower.as_str())
            || lowers.len() >= 2
                && lowers
                    .last()
                    .map(|l| place_suffixes().contains(l.as_str()))
                    .unwrap_or(false)
        {
            Some("Place")
        } else if first_names().contains(lowers[0].as_str())
            || prev.map(|p| HONORIFICS.contains(&p)).unwrap_or(false)
        {
            Some("Person")
        } else {
            None
        };
        // A sentence-initial function word never starts a name.
        let blocked = t.sentence_start && run_function_words().contains(lowers[0].as_str());
        if let (Some(cat), false) = (category, blocked) {
            let title = if cat == "Person" && lowers.len() > 3 {
                words[..2].join(" ")
            } else {
                words.join(" ")
            };
            out.push((title, cat));
        }
        i = j;
    }
    out
}

/// Runs of 2+ capitalized words that are not sentence-start grammar
/// ("Whisper Large", "Memory Graph"). Catches tool/product names NER misses.
pub fn capitalized_runs(text: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut previous_ended_sentence = true;
    for raw in text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-' || c == '.')) {
        if raw.is_empty() {
            continue;
        }
        let ends_sentence = raw.ends_with('.');
        // A name never spans a sentence boundary.
        if previous_ended_sentence && current.len() >= 2 {
            runs.push(current.join(" "));
        }
        if previous_ended_sentence {
            current.clear();
        }
        let word = raw.trim_matches(|c: char| c.is_ascii_punctuation());
        let Some(first) = word.chars().next() else {
            if current.len() >= 2 {
                runs.push(current.join(" "));
            }
            current.clear();
            previous_ended_sentence = ends_sentence;
            continue;
        };
        let is_capitalized = first.is_uppercase() && word.chars().count() > 2;
        let blocked_starter =
            previous_ended_sentence && run_function_words().contains(word.to_lowercase().as_str());
        let can_start_run = is_capitalized && !blocked_starter;
        if is_capitalized && (can_start_run || !current.is_empty()) {
            current.push(word.to_string());
        } else {
            if current.len() >= 2 {
                runs.push(current.join(" "));
            }
            current.clear();
        }
        previous_ended_sentence = ends_sentence;
    }
    if current.len() >= 2 {
        runs.push(current.join(" "));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_with_breaks_and_sentences() {
        let t = tokens("We shipped v1.2 today. Then, Sam's design system broke!");
        let words: Vec<&str> = t.iter().map(|t| t.surface.as_str()).collect();
        assert_eq!(
            words,
            ["We", "shipped", "v1.2", "today", "Then", "Sam's", "design", "system", "broke"]
        );
        assert!(t[0].sentence_start);
        assert!(t[3].ends_sentence);
        assert!(t[4].sentence_start);
        assert!(t[5].after_break);
    }

    #[test]
    fn splits_sentences() {
        let s = sentences("Is it done? Yes. Mr. Smith said so.\nNew line here");
        assert_eq!(
            s,
            ["Is it done?", "Yes.", "Mr. Smith said so.", "New line here"]
        );
    }

    #[test]
    fn lemmatizes_plurals() {
        assert_eq!(lemma("insights"), "insight");
        assert_eq!(lemma("libraries"), "library");
        assert_eq!(lemma("boxes"), "box");
        assert_eq!(lemma("people"), "person");
        assert_eq!(lemma("status"), "status");
        assert_eq!(lemma("analytics"), "analytics");
        assert_eq!(lemma("settings"), "setting");
    }

    #[test]
    fn topics_are_nouns_and_phrases_not_stopwords() {
        let text = "I am going to fix the memory graph again. The memory graph shows insights. \
                    Honestly I believe it is going well here.";
        let topics = topic_concepts(text, 6);
        let keys: Vec<&str> = topics.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys[0], "memory graph");
        assert!(keys.contains(&"insight"));
        for bad in ["going", "here", "believe", "again", "honestly"] {
            assert!(!keys.contains(&bad), "{bad} leaked into {keys:?}");
        }
        assert_eq!(topics[0].title, "Memory Graph");
    }

    #[test]
    fn entities_classify_people_orgs_places() {
        let text = "I met Sarah Chen in London to talk about Google. \
                    Then the Whisper Large model ran. Sarah liked it.";
        let ents = named_entities(text, 8);
        let find = |k: &str| {
            ents.iter()
                .find(|e| e.key == k)
                .map(|e| e.category.as_str())
        };
        assert_eq!(find("sarah chen"), Some("Person"));
        assert_eq!(find("london"), Some("Place"));
        assert_eq!(find("google"), Some("Organization"));
        assert_eq!(find("whisper large"), Some("Name"));
        assert!(find("then the").is_none());
    }

    #[test]
    fn capitalized_runs_skip_sentence_grammar() {
        let runs = capitalized_runs("Do You Know. Project Atlas is live with Memory Graph.");
        assert!(runs.contains(&"Project Atlas".to_string()));
        assert!(runs.contains(&"Memory Graph".to_string()));
        assert!(!runs.iter().any(|r| r.starts_with("Do")));
    }
}
