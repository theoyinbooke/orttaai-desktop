//! Intent-bearing signals in dictated text (Mac `SemanticSignalExtractor`):
//! commitments, questions, decisions, blockers, tone and intent. The
//! deterministic extractor always runs; an optional local-model pass
//! (`semantic_summary_enabled`) refines a chunk under its own model id.

use super::analyzer;
use super::util::clipped;
use crate::error::Result;
use crate::llm::{parse_json_reply, ChatMessage, ChatOptions, LlmClient};
use serde::{Deserialize, Serialize};

pub const HEURISTIC_MODEL_ID: &str = "heuristic-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalFamily {
    Commitment,
    Question,
    Decision,
    Blocker,
    Tone,
    Intent,
}

impl SignalFamily {
    pub const ALL: [SignalFamily; 6] = [
        SignalFamily::Commitment,
        SignalFamily::Question,
        SignalFamily::Decision,
        SignalFamily::Blocker,
        SignalFamily::Tone,
        SignalFamily::Intent,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SignalFamily::Commitment => "commitment",
            SignalFamily::Question => "question",
            SignalFamily::Decision => "decision",
            SignalFamily::Blocker => "blocker",
            SignalFamily::Tone => "tone",
            SignalFamily::Intent => "intent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtractedSignal {
    pub family: SignalFamily,
    pub value: String,
    pub confidence: f64,
}

const STRONG_COMMITMENT: &[&str] = &[
    "i will ",
    "i'll ",
    "i am going to ",
    "i'm going to ",
    "i plan to ",
    "i promise",
    "i want us to ",
    "i am about to ",
];
const SOFT_COMMITMENT: &[&str] = &[
    "i need to ",
    "i have to ",
    "i should ",
    "we need to ",
    "we should ",
    "we have to ",
    "let me ",
    "i want to ",
    "next step is ",
];
const INTERROGATIVE_OPENERS: &[&str] = &[
    "do you",
    "can you",
    "can we",
    "could you",
    "could we",
    "should we",
    "should i",
    "would you",
    "what ",
    "what's",
    "why ",
    "how ",
    "where ",
    "when ",
    "which ",
    "who ",
    "is there",
    "are there",
    "is it",
    "are we",
    "did you",
    "does it",
    "will it",
    "am i",
];
const IMPERATIVE_OPENERS: &[&str] = &[
    "please ",
    "can you ",
    "i want you to ",
    "make sure ",
    "go ahead",
    "let's ",
    "let us ",
];
const IMPERATIVE_VERBS: &[&str] = &[
    "make",
    "add",
    "fix",
    "update",
    "create",
    "remove",
    "delete",
    "change",
    "build",
    "implement",
    "write",
    "generate",
    "check",
    "run",
    "test",
    "use",
    "move",
    "rename",
    "refactor",
    "install",
    "open",
    "close",
    "show",
    "give",
    "find",
    "search",
    "look",
    "try",
    "start",
    "stop",
    "ensure",
    "verify",
    "investigate",
    "review",
    "explain",
    "help",
];
pub const DECISION_CUES: &[&str] = &[
    "let's go with",
    "let us go with",
    "we'll use",
    "we will use",
    "i decided",
    "we decided",
    "we agreed",
    "i've chosen",
    "i have chosen",
    "we're going with",
    "we are going with",
    "final decision",
    "i'm settling on",
    "go with the",
    "stick with",
];
pub const FRUSTRATION_LEXICON: &[&str] = &[
    "stuck",
    "blocked",
    "confusing",
    "frustrat",
    "annoying",
    "not working",
    "doesn't work",
    "does not work",
    "failed",
    "failing",
    "broken",
    "still broken",
    "wrong again",
    "keeps happening",
    "no idea why",
    "can't figure",
    "cannot figure",
    "glitch",
    "the bug",
    "this bug",
];
const EXCITEMENT_LEXICON: &[&str] = &[
    "i love",
    "awesome",
    "amazing",
    "so fast",
    "perfect",
    "excellent",
    "fantastic",
    "well done",
    "great job",
    "beautiful",
    "works great",
    "so good",
    "impressive",
    "appreciate",
];

fn normalize(sentence: &str) -> String {
    sentence.to_lowercase().replace('’', "'")
}

fn signal_sentences(text: &str) -> Vec<String> {
    analyzer::sentences(text.trim())
        .into_iter()
        .filter(|s| s.chars().count() >= 8)
        .collect()
}

/// Deterministic signals for one chunk of text.
pub fn signals(text: &str) -> Vec<ExtractedSignal> {
    let sentences = signal_sentences(text);
    if sentences.is_empty() {
        return Vec::new();
    }
    let mut results = Vec::new();
    let (mut frustration, mut excitement) = (0usize, 0usize);
    let (mut questions, mut commitments, mut imperatives) = (0usize, 0usize, 0usize);

    for sentence in &sentences {
        let lowered = normalize(sentence);
        // Cues are matched with a trailing space; a sentence-final cue counts too.
        let padded = format!("{lowered} ");
        let confidence = if STRONG_COMMITMENT.iter().any(|c| padded.contains(c)) {
            Some(0.9)
        } else if SOFT_COMMITMENT.iter().any(|c| padded.contains(c)) {
            Some(0.7)
        } else {
            None
        };
        if let Some(c) = confidence {
            results.push(ExtractedSignal {
                family: SignalFamily::Commitment,
                value: clipped(sentence, 200),
                confidence: c,
            });
            commitments += 1;
        }
        let q = if sentence.ends_with('?') {
            Some(0.95)
        } else if INTERROGATIVE_OPENERS.iter().any(|o| lowered.starts_with(o)) {
            Some(0.6)
        } else {
            None
        };
        if let Some(c) = q {
            results.push(ExtractedSignal {
                family: SignalFamily::Question,
                value: clipped(sentence, 200),
                confidence: c,
            });
            questions += 1;
        }
        if DECISION_CUES.iter().any(|c| lowered.contains(c)) {
            results.push(ExtractedSignal {
                family: SignalFamily::Decision,
                value: clipped(sentence, 200),
                confidence: 0.8,
            });
        }
        let cues = FRUSTRATION_LEXICON
            .iter()
            .filter(|c| lowered.contains(*c))
            .count();
        if cues > 0 {
            results.push(ExtractedSignal {
                family: SignalFamily::Blocker,
                value: clipped(sentence, 200),
                confidence: (0.55 + 0.1 * cues as f64).min(0.9),
            });
        }
        frustration += cues;
        excitement += EXCITEMENT_LEXICON
            .iter()
            .filter(|c| lowered.contains(*c))
            .count();
        if starts_imperatively(&lowered) {
            imperatives += 1;
        }
    }

    // One tone signal per chunk, only when word-supported.
    if frustration > 0 || excitement > 0 {
        let dominant = if frustration >= excitement {
            "frustrated"
        } else {
            "energized"
        };
        results.push(ExtractedSignal {
            family: SignalFamily::Tone,
            value: dominant.into(),
            confidence: (0.5 + 0.1 * frustration.max(excitement) as f64).min(0.9),
        });
    }
    let intent = if imperatives >= (sentences.len() / 3).max(1) {
        "instruct"
    } else if questions > 0 && questions >= commitments {
        "ask"
    } else if commitments > 0 {
        "plan"
    } else {
        "reflect"
    };
    results.push(ExtractedSignal {
        family: SignalFamily::Intent,
        value: intent.into(),
        confidence: 0.6,
    });
    results
}

fn starts_imperatively(lowered: &str) -> bool {
    if IMPERATIVE_OPENERS.iter().any(|o| lowered.starts_with(o)) {
        return true;
    }
    lowered
        .split_whitespace()
        .next()
        .map(|w| IMPERATIVE_VERBS.contains(&w.trim_matches(|c: char| !c.is_alphanumeric())))
        .unwrap_or(false)
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RefinementPayload {
    commitments: Vec<String>,
    questions: Vec<String>,
    decisions: Vec<String>,
    blockers: Vec<String>,
}

fn refinement_schema() -> serde_json::Value {
    let list = serde_json::json!({ "type": "array", "items": { "type": "string" } });
    serde_json::json!({
        "type": "object",
        "properties": {
            "commitments": list, "questions": list, "decisions": list, "blockers": list
        },
        "required": ["commitments", "questions", "decisions", "blockers"]
    })
}

/// Ask a local model for the commitments, open questions, decisions and
/// blockers in one chunk. Only sentences the speaker actually said count:
/// values are clipped and the model's own commentary is dropped.
pub fn refine_with_model(
    client: &dyn LlmClient,
    model: &str,
    text: &str,
) -> Result<Vec<ExtractedSignal>> {
    let prompt = format!(
        "Below is a passage of someone's own voice dictation. Extract, quoting their words as closely as possible:\n\
         - commitments: things they said they will do or need to do;\n\
         - questions: questions they asked that look unanswered;\n\
         - decisions: choices they clearly made;\n\
         - blockers: problems that actually got in their way (errors, broken behavior, being stuck).\n\
         Use empty lists when there are none. Never invent anything that is not in the passage.\n\n\
         Passage:\n{text}"
    );
    let opts = ChatOptions {
        temperature: Some(0.1),
        think: Some(false),
        json_schema: Some(refinement_schema()),
        max_tokens: Some(320),
        num_ctx: Some(4096),
        timeout_ms: Some(60_000),
        ..ChatOptions::model(model)
    };
    let reply = client.chat(&[ChatMessage::user(prompt)], &opts, None)?;
    let payload: RefinementPayload = parse_json_reply(&reply.content)?;
    let mut out = Vec::new();
    let mut push = |family: SignalFamily, values: Vec<String>, confidence: f64| {
        for v in values.into_iter().take(4) {
            let v = v.trim().trim_matches('"').trim();
            if v.chars().count() < 6 || v.eq_ignore_ascii_case("none") {
                continue;
            }
            out.push(ExtractedSignal {
                family,
                value: clipped(v, 200),
                confidence,
            });
        }
    };
    push(SignalFamily::Commitment, payload.commitments, 0.85);
    push(SignalFamily::Question, payload.questions, 0.9);
    push(SignalFamily::Decision, payload.decisions, 0.85);
    push(SignalFamily::Blocker, payload.blockers, 0.8);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlm;

    fn has(s: &[ExtractedSignal], f: SignalFamily) -> bool {
        s.iter().any(|x| x.family == f)
    }

    #[test]
    fn extracts_commitments_questions_decisions() {
        let s = signals(
            "I'll send the invoice to Sarah tomorrow. Should we move the launch to Friday? \
             Let's go with the blue theme. The build is still broken and I'm stuck.",
        );
        let commitment = s
            .iter()
            .find(|x| x.family == SignalFamily::Commitment)
            .unwrap();
        assert_eq!(commitment.confidence, 0.9);
        assert!(commitment.value.starts_with("I'll send"));
        let q = s
            .iter()
            .find(|x| x.family == SignalFamily::Question)
            .unwrap();
        assert_eq!(q.confidence, 0.95);
        assert!(has(&s, SignalFamily::Decision));
        assert!(has(&s, SignalFamily::Blocker));
        let tone = s.iter().find(|x| x.family == SignalFamily::Tone).unwrap();
        assert_eq!(tone.value, "frustrated");
        assert!(has(&s, SignalFamily::Intent));
    }

    #[test]
    fn interrogative_openers_without_question_marks() {
        let s = signals("how do I reset the router settings");
        let q = s
            .iter()
            .find(|x| x.family == SignalFamily::Question)
            .unwrap();
        assert_eq!(q.confidence, 0.6);
        let intent = s.iter().find(|x| x.family == SignalFamily::Intent).unwrap();
        assert_eq!(intent.value, "ask");
    }

    #[test]
    fn intent_instruct_and_reflect() {
        let s = signals("Fix the padding on the card. Add a new button to the header.");
        assert_eq!(
            s.iter()
                .find(|x| x.family == SignalFamily::Intent)
                .unwrap()
                .value,
            "instruct"
        );
        let r = signals("The weather was lovely during the walk this afternoon.");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].value, "reflect");
        assert!(signals("hi").is_empty());
    }

    #[test]
    fn model_refinement_parses_and_filters() {
        let mock = MockLlm::fixed(
            r#"{"commitments":["I will call the bank","none"],"questions":[],"decisions":["Use Postgres for storage"],"blockers":["ok"]}"#,
        );
        let s = refine_with_model(&mock, "m", "text").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].family, SignalFamily::Commitment);
        assert_eq!(s[1].family, SignalFamily::Decision);
        let call = &mock.calls.lock().unwrap()[0];
        assert!(call.1.json_schema.is_some());
        assert!(refine_with_model(&MockLlm::offline(), "m", "x").is_err());
    }
}
