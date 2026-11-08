//! Roleplay scenes: a partner played by the scene model, a goal to reach,
//! and what a scene needs to feel like talking rather than filling in a
//! form: the partner's reply as a chat it can stream, its mood as a separate
//! quick question, sentences handed on as soon as each is complete, and a
//! turn that ends when you pause.

use super::scoring::judge_prompt;
use super::skill::CoachTrack;
use crate::llm::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartnerMood {
    Pushing,
    Weighing,
    Agreed,
}

impl PartnerMood {
    pub const ALL: [PartnerMood; 3] = [Self::Pushing, Self::Weighing, Self::Agreed];

    pub fn title(self) -> &'static str {
        match self {
            Self::Pushing => "Pushing",
            Self::Weighing => "Weighing it",
            Self::Agreed => "Agreed",
        }
    }

    pub fn step(self) -> u8 {
        match self {
            Self::Pushing => 0,
            Self::Weighing => 1,
            Self::Agreed => 2,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "pushing" => Some(Self::Pushing),
            "weighing" => Some(Self::Weighing),
            "agreed" => Some(Self::Agreed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoachScene {
    pub id: &'static str,
    pub track: CoachTrack,
    pub title: &'static str,
    pub partner_name: &'static str,
    pub partner_role: &'static str,
    /// How the partner behaves, for the model.
    pub partner_brief: &'static str,
    /// What the partner says first.
    pub opening: &'static str,
    /// The goal, as shown: short enough for one line on a scene card.
    pub goal: &'static str,
    /// The goal, as the judge is asked about it at the end.
    pub goal_question: &'static str,
}

pub const MAX_TURNS: usize = 8;

pub const SCENES: &[CoachScene] = &[
    CoachScene {
        id: "brief-agent",
        track: CoachTrack::Prompting,
        title: "Brief an agent",
        partner_name: "Atlas",
        partner_role: "An AI coding agent",
        partner_brief: "You are Atlas, an AI coding agent about to start work. Ask one short question about whatever the speaker left out: the task, where it lives, a constraint, or how you'll know it's done. Once nothing important is missing, say you'll start, briefly.",
        opening: "Ready when you are. What should I work on?",
        goal: "Task, context and finish line",
        goal_question: "Did the speaker give the agent a clear task, the context it needs, and how to know it's done?",
    },
    CoachScene {
        id: "scope-push",
        track: CoachTrack::Messages,
        title: "Push back on scope, kindly",
        partner_name: "Ola",
        partner_role: "A client in Lagos, friendly and in a hurry",
        partner_brief: "You are Ola, a friendly client in Lagos who is in a hurry and wants two extra features, a team calendar and a PDF export, by Friday. Push back once or twice. Agree only when you're offered a realistic choice warmly.",
        opening: "Can we also get the team calendar and the PDF export in by Friday? The board wants both.",
        goal: "Keep Friday, offer a choice",
        goal_question: "Did the speaker keep the deadline realistic while offering the client a real choice, kindly?",
    },
    CoachScene {
        id: "review-ask",
        track: CoachTrack::Messages,
        title: "Get a review today",
        partner_name: "Dami",
        partner_role: "A teammate with a full day",
        partner_brief: "You are Dami, a helpful teammate with a packed day. You'll help if the ask is clear, you know when it's needed and why. If it's vague, say you're slammed and ask what exactly is needed.",
        opening: "Hey, what's up? I'm slammed today.",
        goal: "Get a review by 3 pm",
        goal_question: "Did the speaker clearly ask for the review, say by when, and say why it matters?",
    },
    CoachScene {
        id: "status-update",
        track: CoachTrack::Delivery,
        title: "A quick status update",
        partner_name: "Priya",
        partner_role: "Your manager, between meetings",
        partner_brief: "You are Priya, a manager with two minutes between meetings. Ask for the one thing you most need to know next, briefly. Agree once you have a clear picture.",
        opening: "Give me a quick update on where things are.",
        goal: "A clear update without fillers",
        goal_question: "Did the speaker give a clear, concise status update?",
    },
    CoachScene {
        id: "newcomer",
        track: CoachTrack::Explaining,
        title: "Explain it to a newcomer",
        partner_name: "Sam",
        partner_role: "Joined the team today",
        partner_brief: "You are Sam, who joined the team today and is curious. Ask about whatever you didn't follow, one question at a time. Agree once you could explain it to someone else.",
        opening: "I just joined. What does your team actually do?",
        goal: "Clear enough for Sam to retell",
        goal_question: "Would a newcomer understand the main point from what the speaker said?",
    },
];

impl CoachScene {
    pub fn by_id(id: &str) -> Option<&'static CoachScene> {
        SCENES.iter().find(|s| s.id == id)
    }

    pub fn for_track(track: CoachTrack) -> Vec<&'static CoachScene> {
        SCENES.iter().filter(|s| s.track == track).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speaker {
    You,
    Partner,
}

/// One line of a scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneTurn {
    pub speaker: Speaker,
    pub text: String,
    /// Only on spoken turns: how long it took to say.
    #[serde(default)]
    pub duration_ms: Option<i64>,
    /// Only on spoken turns: what was heard before Polish, for scoring.
    #[serde(default)]
    pub heard: Option<String>,
}

impl SceneTurn {
    pub fn new(speaker: Speaker, text: impl Into<String>) -> Self {
        Self {
            speaker,
            text: text.into(),
            duration_ms: None,
            heard: None,
        }
    }
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Prompts for the partner and how its replies are read (Mac `CoachPartnerPrompt`).
pub mod partner_prompt {
    use super::*;

    pub const JSON_SCHEMA: &str = r#"{"type":"object","properties":{"reply":{"type":"string"},"mood":{"type":"string","enum":["pushing","weighing","agreed"]}},"required":["reply","mood"]}"#;

    pub const MOOD_SCHEMA: &str = r#"{"type":"object","properties":{"mood":{"type":"string","enum":["pushing","weighing","agreed"]}},"required":["mood"]}"#;

    /// One prompt carrying the conversation, for a reply and mood in one JSON.
    pub fn prompt(scene: &CoachScene, turns: &[SceneTurn]) -> String {
        let mut lines = vec![
            scene.partner_brief.to_string(),
            "Stay in character. Speak in one to three short sentences, as in conversation. Never mention being an AI model or a roleplay.".to_string(),
            String::new(),
            "The conversation so far:".to_string(),
        ];
        for turn in turns {
            let who = if turn.speaker == Speaker::You {
                "Them"
            } else {
                scene.partner_name
            };
            lines.push(format!("{who}: {}", turn.text));
        }
        lines.push(String::new());
        lines.push(
            "How you feel now: pushing (not convinced), weighing (coming round) or agreed."
                .to_string(),
        );
        lines.push(format!(
            r#"Reply with JSON only: {{"reply": "what {} says next", "mood": "pushing" | "weighing" | "agreed"}}"#,
            scene.partner_name
        ));
        lines.join("\n")
    }

    /// The partner's line and mood, or `None` when the reply can't be read.
    pub fn parse(reply: &str) -> Option<(String, PartnerMood)> {
        let object = judge_prompt::object(reply)?;
        let text = object.get("reply")?.as_str()?.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let mood = PartnerMood::from_id(&object.get("mood")?.as_str()?.to_lowercase())?;
        Some((text, mood))
    }

    /// The scene as a chat: the partner's brief, then its lines as the
    /// assistant's and yours as the user's. The reply is plain speech, so it
    /// can be spoken while it's still being written.
    pub fn chat(scene: &CoachScene, turns: &[SceneTurn]) -> Vec<ChatMessage> {
        let mut lines = vec![
            ChatMessage::system(
                [
                    scene.partner_brief.to_string(),
                    format!(
                        "Stay in character as {}. Speak in one to three short sentences, as in conversation. Never mention being an AI model or a roleplay.",
                        scene.partner_name
                    ),
                    format!(
                        "Reply with only what {} says: no name, no quotation marks, no stage directions.",
                        scene.partner_name
                    ),
                ]
                .join("\n"),
            ),
            // Chat templates expect the person to speak first.
            ChatMessage::user("(The conversation starts.)"),
        ];
        for turn in turns {
            lines.push(match turn.speaker {
                Speaker::Partner => ChatMessage::assistant(turn.text.clone()),
                Speaker::You => ChatMessage::user(turn.text.clone()),
            });
        }
        lines
    }

    /// How the partner feels after its last line, asked separately so it
    /// never holds up the reply.
    pub fn mood_prompt(scene: &CoachScene, turns: &[SceneTurn]) -> String {
        let mut lines = vec![
            format!(
                "{} is {} in this conversation:",
                scene.partner_name,
                lower_first(scene.partner_role)
            ),
            String::new(),
        ];
        for turn in turns {
            let who = if turn.speaker == Speaker::You {
                "Them"
            } else {
                scene.partner_name
            };
            lines.push(format!("{who}: {}", turn.text));
        }
        lines.push(String::new());
        lines.push(format!(
            "After {0}'s last line, how does {0} feel: pushing (not convinced), weighing (coming round) or agreed?",
            scene.partner_name
        ));
        lines.push(
            r#"Reply with JSON only: {"mood": "pushing" | "weighing" | "agreed"}"#.to_string(),
        );
        lines.join("\n")
    }

    pub fn parse_mood(reply: &str) -> Option<PartnerMood> {
        let object = judge_prompt::object(reply)?;
        PartnerMood::from_id(&object.get("mood")?.as_str()?.to_lowercase())
    }

    fn is_quote(c: char) -> bool {
        matches!(c, '"' | '“' | '”')
    }

    /// A reply as it streams in, without a speaker name the model added
    /// anyway. Only ever removes from the start, so text that grows stays in
    /// step with what has already been handed on.
    pub fn without_name(reply: &str, partner_name: &str) -> String {
        let text = reply.trim();
        let prefix = format!("{}:", partner_name.to_lowercase());
        if !text.to_lowercase().starts_with(&prefix) {
            return text.to_string();
        }
        let skip = partner_name.chars().count() + 1;
        let rest: String = text.chars().skip(skip).collect();
        rest.trim_matches(|c: char| c == ' ' || c == '\t')
            .to_string()
    }

    /// The finished reply: no speaker name, and no quotation marks wrapping
    /// the whole of it. Quotes around a phrase inside it stay.
    pub fn cleaned(reply: &str, partner_name: &str) -> String {
        let text = without_name(reply, partner_name);
        let mut chars = text.chars();
        match chars.next() {
            Some(first) if is_quote(first) => {}
            _ => return text,
        }
        let inner: Vec<char> = chars.collect();
        // Wrapped only when the first closing quote is the last character.
        match inner.iter().position(|&c| is_quote(c)) {
            Some(close) if close + 1 == inner.len() => {
                inner[..close].iter().collect::<String>().trim().to_string()
            }
            _ => text,
        }
    }

    /// A sentence ready to be spoken: stray quotation marks at its ends add nothing.
    pub fn spoken(sentence: &str) -> String {
        sentence.trim_matches(is_quote).trim().to_string()
    }
}

/// Hands on each sentence of a reply as soon as it's complete, from text that
/// grows as it streams in.
#[derive(Debug, Default, Clone)]
pub struct SentenceStream {
    handed_on: usize,
}

impl SentenceStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// New complete sentences in `text`, the whole reply so far.
    pub fn feed(&mut self, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut sentences = Vec::new();
        let mut start = self.handed_on;
        let mut index = self.handed_on;
        while index < chars.len() {
            let c = chars[index];
            let next = chars.get(index + 1).copied();
            // A sentence ends at . ? ! once a space follows, so "3.5" waits.
            if ".?!".contains(c) && matches!(next, Some(' ') | Some('\n')) {
                let sentence: String = chars[start..=index]
                    .iter()
                    .collect::<String>()
                    .trim()
                    .to_string();
                if sentence.chars().count() > 1 {
                    sentences.push(sentence);
                }
                start = index + 1;
            }
            index += 1;
        }
        self.handed_on = start;
        sentences
    }

    /// Whatever is left once the reply is complete.
    pub fn finish(&mut self, text: &str) -> Option<String> {
        let chars: Vec<char> = text.chars().collect();
        if self.handed_on >= chars.len() {
            return None;
        }
        let rest: String = chars[self.handed_on..]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        self.handed_on = chars.len();
        (!rest.is_empty()).then_some(rest)
    }
}

/// What the turn detector saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnEvent {
    None,
    Started,
    Ended,
    GaveUp,
}

/// Decides when you've finished speaking, from the microphone's level: it
/// learns the room's quiet first, waits for you to start, and ends the turn
/// after a pause. (The UI runs the same rules on the engine's `audio-level`
/// events; this is the reference, pinned by tests.)
#[derive(Debug, Clone)]
pub struct TurnDetector {
    pub pause: f64,
    pub patience: f64,
    calibration: f64,
    started_at: Option<f64>,
    patience_from: Option<f64>,
    floor_samples: Vec<f32>,
    threshold: f32,
    last_loud: Option<f64>,
    loud_since: Option<f64>,
    done: bool,
    pub heard_speech: bool,
    /// When the speech that counted began.
    pub speech_started_at: Option<f64>,
}

impl Default for TurnDetector {
    fn default() -> Self {
        Self::new(0.8, 12.0)
    }
}

impl TurnDetector {
    /// `pause`: quiet that ends a turn once you've spoken; `patience`: how
    /// long to wait for you to start before giving up.
    pub fn new(pause: f64, patience: f64) -> Self {
        Self {
            pause,
            patience,
            calibration: 0.3,
            started_at: None,
            patience_from: None,
            floor_samples: Vec::new(),
            threshold: 0.08,
            last_loud: None,
            loud_since: None,
            done: false,
            heard_speech: false,
            speech_started_at: None,
        }
    }

    /// Starts the wait for you to speak afresh from `time`.
    pub fn restart_patience(&mut self, time: f64) {
        self.patience_from = Some(time);
    }

    /// One level reading, 0 to 1, at `time` seconds. Reports `Ended` or
    /// `GaveUp` once, then nothing more.
    pub fn feed(&mut self, level: f32, time: f64) -> TurnEvent {
        if self.done {
            return TurnEvent::None;
        }
        let started = *self.started_at.get_or_insert(time);
        let elapsed = time - started;
        if elapsed < self.calibration {
            self.floor_samples.push(level);
            let mut sorted = self.floor_samples.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let floor = sorted[sorted.len() / 2];
            self.threshold = 0.08_f32.max(floor * 3.0 + 0.03);
            return TurnEvent::None;
        }
        if level >= self.threshold {
            let since = *self.loud_since.get_or_insert(time);
            self.last_loud = Some(time);
            // A short click isn't speech; a fifth of a second of sound is.
            if !self.heard_speech && time - since >= 0.2 {
                self.heard_speech = true;
                self.speech_started_at = Some(since);
                return TurnEvent::Started;
            }
        } else {
            self.loud_since = None;
        }
        if self.heard_speech {
            if let Some(last) = self.last_loud {
                if time - last >= self.pause {
                    self.done = true;
                    return TurnEvent::Ended;
                }
            }
        }
        if !self.heard_speech && time - self.patience_from.unwrap_or(started) >= self.patience {
            self.done = true;
            return TurnEvent::GaveUp;
        }
        TurnEvent::None
    }
}

#[cfg(test)]
mod tests {
    use super::partner_prompt as pp;
    use super::*;
    use crate::llm::Role;
    use std::collections::HashSet;

    #[test]
    fn every_track_has_a_scene() {
        for track in CoachTrack::ALL {
            assert!(!CoachScene::for_track(track).is_empty());
        }
        assert_eq!(
            SCENES.iter().map(|s| s.id).collect::<HashSet<_>>().len(),
            SCENES.len()
        );
    }

    #[test]
    fn partner_prompt_carries_the_conversation() {
        let scene = CoachScene::by_id("scope-push").unwrap();
        let prompt = pp::prompt(
            scene,
            &[
                SceneTurn::new(Speaker::Partner, scene.opening),
                SceneTurn::new(Speaker::You, "By Friday I can do one of them properly."),
            ],
        );
        assert_eq!(scene.partner_name, "Ola");
        assert!(prompt.contains(&format!("{}: {}", scene.partner_name, scene.opening)));
        assert!(prompt.contains("Them: By Friday I can do one of them properly."));
        assert!(prompt.starts_with(scene.partner_brief));
    }

    #[test]
    fn partner_replies_are_read_strictly() {
        let parsed = pp::parse(r#"{"reply":"Hmm, which one then?","mood":"Weighing"}"#).unwrap();
        assert_eq!(parsed.0, "Hmm, which one then?");
        assert_eq!(parsed.1, PartnerMood::Weighing);
        assert_eq!(pp::parse(r#"{"reply":"","mood":"agreed"}"#), None);
        assert_eq!(pp::parse(r#"{"reply":"Fine.","mood":"happy"}"#), None);
        assert_eq!(pp::parse("Sure, let's do it."), None);
    }

    #[test]
    fn the_scene_becomes_a_chat_with_the_partner_as_the_assistant() {
        let scene = CoachScene::by_id("scope-push").unwrap();
        let chat = pp::chat(
            scene,
            &[
                SceneTurn::new(Speaker::Partner, scene.opening),
                SceneTurn::new(Speaker::You, "By Friday I can do one of them properly."),
            ],
        );
        assert_eq!(chat[0].role, Role::System);
        assert!(chat[0].content.starts_with(scene.partner_brief));
        assert_eq!(chat[1].role, Role::User, "The person speaks first");
        assert_eq!(chat[2], ChatMessage::assistant(scene.opening));
        assert_eq!(
            chat[3],
            ChatMessage::user("By Friday I can do one of them properly.")
        );
    }

    #[test]
    fn mood_is_read_strictly() {
        assert_eq!(
            pp::parse_mood(r#"{"mood":"Weighing"}"#),
            Some(PartnerMood::Weighing)
        );
        assert_eq!(pp::parse_mood(r#"{"mood":"cross"}"#), None);
        assert_eq!(pp::parse_mood("weighing"), None);
        let prompt = pp::mood_prompt(&SCENES[1], &[SceneTurn::new(Speaker::You, "Hi")]);
        assert!(prompt.contains("Them: Hi"));
        assert!(prompt.starts_with("Ola is a client in Lagos"));
    }

    #[test]
    fn replies_lose_a_name_or_quotes_the_model_added_anyway() {
        assert_eq!(
            pp::cleaned("Ola: \"Fine, the calendar first.\"", "Ola"),
            "Fine, the calendar first."
        );
        assert_eq!(pp::cleaned("  Sure.  ", "Ola"), "Sure.");
        assert_eq!(pp::cleaned("“Okay.”", "Ola"), "Okay.");
    }

    #[test]
    fn a_quoted_phrase_keeps_both_its_quotes() {
        let reply = "\"Try your best\" is not enough. I need both features by Friday.";
        assert_eq!(pp::cleaned(reply, "Ola"), reply);
        assert_eq!(
            pp::spoken("\"Try your best\" is not enough."),
            "Try your best\" is not enough."
        );
    }

    #[test]
    fn a_streaming_reply_only_ever_loses_its_start() {
        let partial = pp::without_name("Ola: \"Fine, the", "Ola");
        let whole = pp::without_name("Ola: \"Fine, the calendar first.\"", "Ola");
        assert!(whole.starts_with(&partial));
    }

    #[test]
    fn sentences_are_handed_on_as_soon_as_theyre_complete() {
        let mut stream = SentenceStream::new();
        assert!(stream.feed("Okay, I hear").is_empty());
        assert!(!stream.feed("Okay, I hear you. If it's").is_empty());
        assert_eq!(
            stream
                .feed("Okay, I hear you. If it's one, the calendar. But version 3.5")
                .len(),
            1
        );
        let whole = "Okay, I hear you. If it's one, the calendar. But version 3.5 works?";
        assert!(
            stream.feed(whole).is_empty(),
            "Waits for what follows the ?"
        );
        assert_eq!(
            stream.finish(whole).as_deref(),
            Some("But version 3.5 works?")
        );
        assert_eq!(stream.finish(whole), None);

        let mut stream = SentenceStream::new();
        let text = "First one. Second one! Third";
        assert_eq!(stream.feed(text), vec!["First one.", "Second one!"]);
        assert_eq!(stream.finish(text).as_deref(), Some("Third"));
    }

    fn run(
        detector: &mut TurnDetector,
        time: &mut f64,
        events: &mut Vec<TurnEvent>,
        level: f32,
        seconds: f64,
    ) {
        let end = *time + seconds;
        while *time < end - 1e-9 {
            let event = detector.feed(level, *time);
            if event != TurnEvent::None {
                events.push(event);
            }
            *time += 0.1;
        }
    }

    #[test]
    fn a_turn_ends_after_a_pause_once_youve_spoken() {
        let mut detector = TurnDetector::new(0.8, 12.0);
        let mut events = Vec::new();
        let mut time = 0.0;
        run(&mut detector, &mut time, &mut events, 0.02, 0.5);
        run(&mut detector, &mut time, &mut events, 0.4, 1.5);
        run(&mut detector, &mut time, &mut events, 0.02, 0.5);
        run(&mut detector, &mut time, &mut events, 0.4, 1.0);
        assert_eq!(events, vec![TurnEvent::Started]);
        run(&mut detector, &mut time, &mut events, 0.02, 1.0);
        assert_eq!(events, vec![TurnEvent::Started, TurnEvent::Ended]);
        assert!((detector.speech_started_at.unwrap_or(0.0) - 0.5).abs() < 0.05);
    }

    #[test]
    fn a_click_isnt_speech_and_silence_eventually_gives_up() {
        let mut detector = TurnDetector::new(0.8, 3.0);
        let mut events = Vec::new();
        let mut time = 0.0;
        let levels: Vec<f32> = std::iter::repeat_n(0.02, 5)
            .chain([0.5])
            .chain(std::iter::repeat_n(0.02, 30))
            .collect();
        for level in levels {
            let event = detector.feed(level, time);
            if event != TurnEvent::None {
                events.push(event);
            }
            time += 0.1;
        }
        assert!(!detector.heard_speech);
        assert_eq!(events.first(), Some(&TurnEvent::GaveUp));
    }

    #[test]
    fn the_wait_for_you_counts_from_when_the_partner_stops() {
        let mut detector = TurnDetector::new(0.8, 3.0);
        let mut gave_up_at = None;
        let mut time = 0.0;
        while time < 10.0 {
            if time < 5.0 {
                detector.restart_patience(time);
            }
            if detector.feed(0.02, time) == TurnEvent::GaveUp && gave_up_at.is_none() {
                gave_up_at = Some(time);
            }
            time += 0.1;
        }
        let at = gave_up_at.unwrap();
        assert!((7.8..=8.2).contains(&at));
    }

    #[test]
    fn a_noisy_room_raises_the_bar() {
        let mut detector = TurnDetector::default();
        let mut time = 0.0;
        for _ in 0..4 {
            detector.feed(0.1, time);
            time += 0.1;
        }
        let mut started = false;
        for _ in 0..20 {
            started |= detector.feed(0.12, time) == TurnEvent::Started;
            time += 0.1;
        }
        assert!(!started);
        for _ in 0..5 {
            started |= detector.feed(0.6, time) == TurnEvent::Started;
            time += 0.1;
        }
        assert!(started);
    }
}
