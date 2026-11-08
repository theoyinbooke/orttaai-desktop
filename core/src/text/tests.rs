//! Pipeline tests ported from the Mac `RuleBasedTextProcessorTests`.

use super::*;
use crate::memory::{DictionaryRule, SnippetRule};

fn all_on() -> TextOptions {
    TextOptions {
        style_mode: StyleMode::Clean,
        spoken_formatting: true,
        auto_paragraphs: true,
        dictionary_enabled: true,
        fuzzy_dictionary: true,
        snippets_enabled: true,
        disfluency_cleanup: true,
        language: "en".into(),
    }
}

fn processor(opts: TextOptions, memory: MemoryService) -> TextProcessor {
    TextProcessor::new(opts, memory)
}

fn run(text: &str) -> ProcessedText {
    processor(all_on(), MemoryService::new()).process(text)
}

fn processed(text: &str) -> String {
    run(text).text
}

fn has(out: &ProcessedText, needle: &str) -> bool {
    out.changes.iter().any(|c| c.contains(needle))
}

#[test]
fn processor_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TextProcessor>();
}

#[test]
fn options_follow_settings_and_effective_language() {
    let mut s = Settings {
        dictation_language: "auto".into(),
        ..Settings::default()
    };
    assert_eq!(TextOptions::from_settings(&s).language, "auto");
    // Express doesn't force English: with "auto" it isn't active and Whisper
    // dictates in whatever language is spoken (Mac isExpressEngineActive).
    s.engine = crate::settings::SpeechEngine::Express;
    assert_eq!(TextOptions::from_settings(&s).language, "auto");
    s.engine = crate::settings::SpeechEngine::Whisper;
    s.dictation_language = "es".into();
    s.fuzzy_dictionary = false;
    let o = TextOptions::from_settings(&s);
    assert_eq!(o.language, "es");
    assert!(!o.fuzzy_dictionary);
}

#[test]
fn dictionary_replacement_reports_hits() {
    let mut m = MemoryService::new();
    m.add_term("whispr", "Wispr");
    let out = processor(all_on(), m).process("whispr flow and whispr");
    assert_eq!(out.text, "Wispr flow and Wispr");
    assert!(has(&out, "Dictionary"));
    assert_eq!(out.dictionary_hits, vec!["whispr", "whispr"]);
}

#[test]
fn snippet_expansion_with_command_prefix() {
    let mut m = MemoryService::new();
    m.add_snippet("my email", "theoyinbooke@gmail.com");
    let out = processor(all_on(), m).process("insert my email");
    assert_eq!(out.text, "theoyinbooke@gmail.com");
    assert!(has(&out, "Snippet expanded"));
    assert_eq!(out.snippet_hits, vec!["my email"]);
}

#[test]
fn disabled_features_bypass_rules() {
    let mut m = MemoryService::new();
    m.add_term("whispr", "Wispr");
    m.add_snippet("my email", "me@example.com");
    let opts = TextOptions {
        dictionary_enabled: false,
        snippets_enabled: false,
        ..all_on()
    };
    let out = processor(opts, m).process("insert my email and whispr");
    assert_eq!(out.text, "insert my email and whispr");
    assert!(out.changes.is_empty());
    assert!(out.dictionary_hits.is_empty() && out.snippet_hits.is_empty());
}

#[test]
fn rebuilt_processor_follows_memory_changes() {
    let mut m = MemoryService::new();
    m.add_term("whispr", "Wispr");
    assert_eq!(
        processor(all_on(), m.clone()).process("whispr flow").text,
        "Wispr flow"
    );
    m.add_term("whispr", "Whisper");
    assert_eq!(
        processor(all_on(), m).process("whispr flow").text,
        "Whisper flow"
    );
    assert_eq!(
        processor(all_on(), MemoryService::new())
            .process("insert my email")
            .text,
        "insert my email"
    );
}

#[test]
fn numbered_lists_from_spoken_markers() {
    let out = run("number one it has to be this number two it has to be that");
    assert_eq!(out.text, "1. It has to be this\n2. It has to be that");
    assert!(out
        .changes
        .iter()
        .any(|c| c == "Spoken formatting: numbered list"));
    assert_eq!(
        processed("Number one, open settings. Number two, choose audio."),
        "1. Open settings.\n2. Choose audio."
    );
    assert_eq!(
        processed("1. Open Settings 2. Choose Audio"),
        "1. Open Settings\n2. Choose Audio"
    );
    assert_eq!(
        processed("Here are the steps 1. open settings 2. choose audio"),
        "Here are the steps\n1. Open settings\n2. Choose audio"
    );
    assert_eq!(
        processed("here are the steps number 1 open settings and number 2 choose audio"),
        "here are the steps\n1. Open settings\n2. Choose audio"
    );
    assert_eq!(
        processed("number one is speed number two is stability"),
        "1. Speed\n2. Stability"
    );
    assert_eq!(
        processed("number one buy milk. number two wash car."),
        "1. Buy milk.\n2. Wash car."
    );
}

#[test]
fn numbered_lists_from_ordinal_words() {
    assert_eq!(
        processed("My concerns are these. First, the cancel button should match the others. Second, we don't need the extra keys. Third, it should be pure voice."),
        "My concerns are these.\n1. The cancel button should match the others.\n2. We don't need the extra keys.\n3. It should be pure voice."
    );
    assert_eq!(
        processed("First, fix the cancel button. Number two, remove the extra keys."),
        "1. Fix the cancel button.\n2. Remove the extra keys."
    );
    assert_eq!(
        processed("Firstly: ship the fix. Secondly: tell the testers."),
        "1. Ship the fix.\n2. Tell the testers."
    );
    assert_eq!(
        processed("My three priorities are, first, hiring two engineers, second, closing the seed round, and third, launching the iPhone app."),
        "My three priorities are\n1. Hiring two engineers\n2. Closing the seed round\n3. Launching the iPhone app."
    );
}

#[test]
fn long_tail_after_the_last_item_becomes_a_paragraph() {
    let tail = "The other thing I wanted to say is that the colors feel far too loud for a keyboard \
                that sits under every single app you use all day long on your phone and on your iPad too.";
    let out = processed(&format!(
        "First, fix the cancel button. Second, drop the extra keys. It should be pure voice. {tail}"
    ));
    assert_eq!(
        out,
        format!(
            "1. Fix the cancel button.\n2. Drop the extra keys. It should be pure voice.\n\n{tail}"
        )
    );
}

#[test]
fn ordinals_numbers_digits_and_bullets_in_prose_are_left_alone() {
    for text in [
        "First, let me check the logs before we ship.",
        "This is the first time, and the second time, it worked.",
        "Wait a second, I think it is fine.",
        "I think the number one reason is latency",
        "Please make the first bullet point stronger",
        "bullet point is a phrase I might say",
        "I bought 2. Then I bought 3. That is all for today.",
        "Send a letter a day to keep in touch.",
        "I wrote a letter to Bob and a letter to Ann.",
        "Plan a) is fine for now.",
    ] {
        let out = run(text);
        assert_eq!(out.text, text);
        assert!(!has(&out, "Spoken formatting"), "{text}");
    }
}

fn long_dictation(openers: &[&str]) -> String {
    let body =
        "The keyboard should feel like a regular keyboard with one big microphone in the middle. \
                The cancel button should sit at the bottom where people expect it. \
                Every extra control we add makes it harder to use with one hand.";
    openers
        .iter()
        .map(|o| format!("{o} {body}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn long_dictation_is_split_into_paragraphs_at_new_points() {
    let text = long_dictation(&[
        "Here is my take.",
        "Also, one more thing.",
        "Finally, to wrap up.",
    ]);
    let out = processed(&text);
    let paragraphs: Vec<&str> = out.split("\n\n").collect();
    assert_eq!(paragraphs.len(), 3, "{out}");
    assert!(paragraphs[1].starts_with("Also, one more thing."));
    assert!(paragraphs[2].starts_with("Finally, to wrap up."));
    assert_eq!(out.replace("\n\n", " "), text, "no words change");
}

#[test]
fn short_dictation_stays_one_paragraph() {
    let text = "Also, the cancel button should sit at the bottom. Finally, ship it today.";
    assert_eq!(processed(text), text);
}

#[test]
fn automatic_paragraphs_skip_terminals_and_can_be_disabled() {
    let text = long_dictation(&[
        "Here is my take.",
        "Also, one more thing.",
        "Finally, to wrap up.",
    ]);
    let p = processor(all_on(), MemoryService::new());
    assert_eq!(p.process_for_app(&text, Some("Terminal")).text, text);
    assert_eq!(
        p.process_for_app(&text, Some("WindowsTerminal.exe")).text,
        text
    );
    let off = processor(
        TextOptions {
            auto_paragraphs: false,
            ..all_on()
        },
        MemoryService::new(),
    );
    assert_eq!(off.process(&text).text, text);
}

#[test]
fn speakers_own_breaks_are_kept() {
    let text = format!(
        "{} new paragraph Finally, done.",
        long_dictation(&["Here is my take.", "Also, one more thing."])
    );
    assert_eq!(
        processed(&text).split("\n\n").count(),
        2,
        "only the spoken break"
    );
}

#[test]
fn bullet_lists() {
    let out = run("bullet point fast on device transcription bullet point clipboard is preserved");
    assert_eq!(
        out.text,
        "- Fast on device transcription\n- Clipboard is preserved"
    );
    assert!(out
        .changes
        .iter()
        .any(|c| c == "Spoken formatting: bullet list"));
}

#[test]
fn spoken_formatting_can_be_disabled() {
    let p = processor(
        TextOptions {
            spoken_formatting: false,
            ..all_on()
        },
        MemoryService::new(),
    );
    let out = p.process("number one open settings number two choose audio");
    assert_eq!(out.text, "number one open settings number two choose audio");
    assert!(!has(&out, "Spoken formatting"));
}

#[test]
fn line_break_commands() {
    let out = run("thanks for your help new paragraph best regards John");
    assert_eq!(out.text, "thanks for your help\n\nBest regards John");
    assert!(has(&out, "line break command"));
    assert_eq!(
        processed("first step done, new line, second step"),
        "first step done\nSecond step"
    );
    assert_eq!(
        processed("Done. New paragraph. Next section covers billing"),
        "Done.\n\nNext section covers billing"
    );
    assert_eq!(processed("send the file new line"), "send the file");
    assert_eq!(processed("alpha newline beta"), "alpha\nBeta");
    assert_eq!(
        processed(
            "here is the plan new line number one review the budget number two send the report"
        ),
        "here is the plan\n1. Review the budget\n2. Send the report"
    );
}

#[test]
fn break_commands_used_as_noun_phrases_are_left_alone() {
    for text in [
        "we are launching a new line of products this fall",
        "add a new paragraph about pricing to the doc",
    ] {
        let out = run(text);
        assert_eq!(out.text, text);
        assert!(!has(&out, "line break"));
    }
}

#[test]
fn breaks_end_lists_and_separate_paragraphs() {
    assert_eq!(
        processed("Number one, open the app. Number two, click save. New paragraph. Then we are done with the setup and we can go home."),
        "1. Open the app.\n2. Click save.\n\nThen we are done with the setup and we can go home."
    );
    assert_eq!(
        processed(
            "Thanks for joining the call today. New paragraph. Here is what we agreed. Number one, Sam owns the pricing page. \
             Number two, Ade fixes the login bug. Number three, we meet again on Thursday. New paragraph. \
             Let me know if I missed anything. New paragraph. Thanks, Theo."
        ),
        "Thanks for joining the call today.\n\nHere is what we agreed.\n1. Sam owns the pricing page.\n2. Ade fixes the login bug.\
         \n3. We meet again on Thursday.\n\nLet me know if I missed anything.\n\nThanks, Theo."
    );
    assert_eq!(
        processed("Number one, open the app. Number two, click save. New line. Thanks."),
        "1. Open the app.\n2. Click save.\nThanks."
    );
}

#[test]
fn two_lists_in_one_dictation() {
    let expected =
        "For the morning\n1. Coffee\n2. Email.\n\nFor the afternoon\n1. Meetings\n2. Code review.";
    assert_eq!(
        processed("For the morning, number one, coffee, number two, email. New paragraph. For the afternoon, number one, meetings, number two, code review."),
        expected
    );
    assert_eq!(
        processed("For the morning, number one, coffee, number two, email. For the afternoon, number one, meetings, number two, code review."),
        expected
    );
}

#[test]
fn lists_go_past_ten() {
    let items = [
        "call the bank",
        "email Sarah",
        "review the contract",
        "book the venue",
        "order the food",
        "send invites",
        "buy decorations",
        "hire a DJ",
        "confirm the photographer",
        "rent chairs",
        "print the menus",
        "clean up",
    ];
    let words = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
        "twelve",
    ];
    let spoken: Vec<String> = words
        .iter()
        .zip(items)
        .map(|(w, i)| format!("Number {w}, {i}."))
        .collect();
    let expected: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(n, i)| {
            let mut c = i.chars();
            let first = c.next().unwrap().to_uppercase().collect::<String>();
            format!("{}. {first}{}.", n + 1, c.as_str())
        })
        .collect();
    assert_eq!(processed(&spoken.join(" ")), expected.join("\n"));
}

#[test]
fn lettered_lists() {
    let out = run("The options are letter A, the red one, letter B, the blue one, and letter C, the green one.");
    assert_eq!(
        out.text,
        "The options are\nA. The red one\nB. The blue one\nC. The green one."
    );
    assert!(out
        .changes
        .iter()
        .any(|c| c == "Spoken formatting: lettered list"));
    assert_eq!(
        processed("Choose one of these. A) red, B) blue, C) green."),
        "Choose one of these.\nA. Red\nB. Blue\nC. Green."
    );
    assert_eq!(
        processed("Pick one: (a) the red car, (b) the blue car, or (c) the green car."),
        "Pick one:\nA. The red car\nB. The blue car\nC. The green car."
    );
    assert_eq!(
        processed("Number one, fruits. Letter A, apples. Letter B, pears. Number two, vegetables. Letter A, carrots. Letter B, spinach."),
        "1. Fruits.\n   a. Apples.\n   b. Pears.\n2. Vegetables.\n   a. Carrots.\n   b. Spinach."
    );
}

fn rule(source: &str, target: &str, usage: i64) -> DictionaryRule {
    DictionaryRule {
        id: None,
        source: source.into(),
        target: target.into(),
        case_sensitive: false,
        usage_count: usage,
    }
}

#[test]
fn vocabulary_terms_use_targets_and_triggers_most_used_first() {
    let mut m = MemoryService::new();
    m.add_dictionary_rule(rule("olan rewaju", "Olanrewaju", 1));
    m.add_dictionary_rule(rule("wisper kit", "WhisperKit", 5));
    m.add_snippet_rule(SnippetRule {
        id: None,
        trigger: "my email sig".into(),
        expansion: "Best,\nTheo".into(),
        usage_count: 0,
    });
    let p = processor(all_on(), m);
    assert_eq!(
        p.vocabulary_terms(),
        vec!["WhisperKit", "Olanrewaju", "my email sig"]
    );
    let prompt = p.vocabulary_prompt().unwrap();
    assert!(prompt.contains("Olanrewaju") && prompt.contains("WhisperKit"));
    assert!(!prompt.contains("Theo"), "expansions are typed, not spoken");
    assert!(prompt.ends_with('.'));
    assert!(processor(all_on(), MemoryService::new())
        .vocabulary_prompt()
        .is_none());
}

#[test]
fn bias_prompt_respects_the_token_budget() {
    let mut m = MemoryService::new();
    for i in 0..60 {
        m.add_dictionary_rule(rule(&format!("term {i}"), &format!("Productname{i:02}"), i));
    }
    let prompt = processor(all_on(), m).vocabulary_prompt().unwrap();
    assert!(
        estimated_tokens(&prompt) <= MAX_BIAS_PROMPT_TOKENS,
        "{prompt}"
    );
    assert!(prompt.split(", ").count() >= 3);
    assert_eq!(
        normalized_bias_terms(&["  ab ".into(), "AB".into(), "".into(), "abc".into()], 40),
        vec!["abc", "ab"]
    );
}

#[test]
fn word_boost_terms_group_sources_under_targets() {
    let mut m = MemoryService::new();
    m.add_term("temi tope", "Temitope");
    m.add_term("tematope", "Temitope");
    m.add_term("Vercel", "Vercel");
    let terms = processor(all_on(), m).word_boost_terms();
    assert_eq!(terms.len(), 2);
    assert_eq!(terms[0].text, "Temitope");
    assert_eq!(terms[0].aliases, vec!["temi tope", "tematope"]);
    assert!(terms[1].aliases.is_empty());
}

fn fuzzy_memory() -> MemoryService {
    let mut m = MemoryService::new();
    m.add_term("temi tope", "Temitope");
    m
}

#[test]
fn fuzzy_dictionary_corrects_near_misses_without_counting_usage() {
    let out = processor(all_on(), fuzzy_memory()).process("Tematope will present today");
    assert_eq!(out.text, "Temitope will present today");
    assert!(out
        .changes
        .iter()
        .any(|c| c == "Dictionary (fuzzy): 'Tematope' -> 'Temitope'"));
    assert!(
        out.dictionary_hits.is_empty(),
        "fuzzy hits must not feed usage counts"
    );
}

#[test]
fn exact_rows_run_before_fuzzy_matching() {
    let mut m = MemoryService::new();
    m.add_term("mitumor", "Meetumo");
    let out = processor(all_on(), m).process("mitumor and Mitumo");
    assert_eq!(out.text, "Meetumo and Meetumo");
    assert_eq!(
        out.changes
            .iter()
            .filter(|c| c.starts_with("Dictionary:"))
            .count(),
        1
    );
    assert_eq!(
        out.changes
            .iter()
            .filter(|c| c.starts_with("Dictionary (fuzzy):"))
            .count(),
        1
    );
    assert_eq!(
        out.dictionary_hits,
        vec!["mitumor"],
        "only the literal hit counts"
    );
}

#[test]
fn fuzzy_dictionary_leaves_real_words_and_similar_names_alone() {
    let mut m = MemoryService::new();
    m.add_term("vasel", "Vercel");
    m.add_term("mitumor", "Meetumo");
    m.add_term("web socket", "WebSocket");
    m.add_term("michael s", "Michael");
    let p = processor(all_on(), m);
    let out = p.process("The verse fits the meetup");
    assert_eq!(out.text, "The verse fits the meetup");
    assert!(out.changes.is_empty());
    assert_eq!(
        p.process("We use websockets and ask Michelle or Michaela.")
            .text,
        "We use websockets and ask Michelle or Michaela."
    );
}

#[test]
fn fuzzy_dictionary_honours_its_switches_and_language() {
    let off = TextOptions {
        fuzzy_dictionary: false,
        ..all_on()
    };
    assert_eq!(
        processor(off, fuzzy_memory())
            .process("Tematope will present")
            .text,
        "Tematope will present"
    );
    let no_dict = TextOptions {
        dictionary_enabled: false,
        ..all_on()
    };
    assert_eq!(
        processor(no_dict, fuzzy_memory())
            .process("Tematope will present")
            .text,
        "Tematope will present"
    );
    assert_eq!(
        processor(all_on(), MemoryService::new())
            .process("Tematope")
            .text,
        "Tematope"
    );
    for language in ["es", "fr", "auto"] {
        let opts = TextOptions {
            language: language.into(),
            ..all_on()
        };
        let out = processor(opts, fuzzy_memory()).process("Tematope will present");
        assert_eq!(out.text, "Tematope will present", "{language}");
        assert!(!out
            .changes
            .iter()
            .any(|c| c.starts_with("Dictionary (fuzzy)")));
    }
    let s = Settings {
        dictation_language: "en".into(),
        ..Settings::default()
    };
    let p = processor(TextOptions::from_settings(&s), fuzzy_memory());
    assert_eq!(
        p.process("Tematope will present").text,
        "Temitope will present"
    );
}

#[test]
fn disfluency_cleanup_in_the_pipeline() {
    let out = run("so um um to avoid the the memory i think");
    assert_eq!(out.text, "so to avoid the memory I think");
    assert!(out
        .changes
        .iter()
        .any(|c| c == "Disfluency: removed 2 fillers"));
    assert_eq!(
        processed("Send the report, um, new line best regards"),
        "Send the report\nBest regards"
    );
    assert_eq!(
        processed("Send the report new line um best regards"),
        "Send the report\nBest regards"
    );
    let off = TextOptions {
        disfluency_cleanup: false,
        ..all_on()
    };
    assert_eq!(
        processor(off, MemoryService::new())
            .process("so um the the plan i think")
            .text,
        "so um the the plan i think"
    );
    let es = TextOptions {
        language: "es".into(),
        ..all_on()
    };
    assert_eq!(
        processor(es, MemoryService::new())
            .process("voy a a la tienda")
            .text,
        "voy a a la tienda"
    );
}

#[test]
fn snippet_expansion_is_never_cleaned() {
    let mut m = MemoryService::new();
    m.add_snippet("my sig", "um the the plan i think");
    let out = processor(all_on(), m).process("insert my sig");
    assert_eq!(out.text, "um the the plan i think");
    assert!(!out.changes.iter().any(|c| c.starts_with("Disfluency")));
}

#[test]
fn raw_style_keeps_what_was_heard_after_memory() {
    let mut m = MemoryService::new();
    m.add_term("whispr", "Wispr");
    let raw = TextOptions {
        style_mode: StyleMode::Raw,
        ..all_on()
    };
    let out = processor(raw, m).process("um whispr new line number one a number two b");
    assert_eq!(out.text, "um Wispr new line number one a number two b");
    assert!(style_instruction(StyleMode::Raw).is_none());
    assert!(style_instruction(StyleMode::Clean).is_none());
    assert!(style_instruction(StyleMode::Formal)
        .unwrap()
        .contains("formal"));
    assert!(style_instruction(StyleMode::Casual)
        .unwrap()
        .contains("conversational"));
}

#[test]
fn empty_input_is_returned_as_is() {
    assert_eq!(processed("   "), "   ");
    assert_eq!(processed("  hello  "), "hello");
}

/// The Mac's ASR-eval regression (gauntlet/asr_eval): over real Whisper output
/// the fuzzy pass must leave every correct reference alone, never rewrite to
/// anything but a hard term, and raise hard-term recall on both halves.
#[test]
fn fuzzy_pass_raises_recall_without_false_rewrites() {
    #[derive(serde::Deserialize)]
    struct Item {
        id: String,
        reference: String,
        hard_terms: Vec<String>,
        whole: String,
        live: String,
    }
    #[derive(serde::Deserialize)]
    struct Fixture {
        bias_vocabulary: Vec<String>,
        items: Vec<Item>,
    }
    let fixture: Fixture =
        serde_json::from_str(include_str!("../../tests/fixtures/fuzzy_recall.json")).unwrap();
    let matcher = fuzzy::FuzzyMatcher::new(&fixture.bias_vocabulary);
    let squashed = |t: &str| {
        t.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let words = |t: &str| -> std::collections::HashSet<String> {
        t.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let mut false_rewrites = Vec::new();
    let (mut tune_before, mut tune_after, mut held_before, mut held_after) = (0, 0, 0, 0);
    for item in &fixture.items {
        let (same, reps) = matcher.apply(&item.reference);
        assert_eq!(same, item.reference, "{}: {reps:?}", item.id);
        let number: u32 = item
            .id
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        for (path, text) in [("whole", &item.whole), ("live", &item.live)] {
            let (fixed, reps) = matcher.apply(text);
            for term in &item.hard_terms {
                let hit = usize::from(squashed(text).contains(&term.to_lowercase()));
                let hit_after = usize::from(squashed(&fixed).contains(&term.to_lowercase()));
                if number % 2 == 0 {
                    tune_before += hit;
                    tune_after += hit_after;
                } else {
                    held_before += hit;
                    held_after += hit_after;
                }
            }
            for r in &reps {
                if !item.hard_terms.contains(&r.target) {
                    false_rewrites.push(format!(
                        "{path}/{}: '{}' -> '{}'",
                        item.id, r.original, r.target
                    ));
                }
            }
            let allowed: std::collections::HashSet<String> =
                item.hard_terms.iter().flat_map(|t| words(t)).collect();
            let introduced: Vec<String> = words(&fixed)
                .difference(&words(text))
                .filter(|w| !allowed.contains(*w))
                .cloned()
                .collect();
            if !introduced.is_empty() {
                false_rewrites.push(format!("{path}/{}: introduced {introduced:?}", item.id));
            }
        }
    }
    assert!(false_rewrites.is_empty(), "{false_rewrites:#?}");
    assert!(
        tune_after > tune_before,
        "no recall gain on the tuning half"
    );
    assert!(
        held_after > held_before,
        "no recall gain on the held-out half"
    );
}
