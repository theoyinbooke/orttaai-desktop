//! The coaches. Each changes how feedback is worded, never the score: the
//! checks and numbers are the same whoever coaches you.

use super::scoring::CheckResult;
use super::skill::CoachSkill;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoachPersona {
    Mentor,
    Editor,
    Socratic,
    Sergeant,
    Peer,
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl CoachPersona {
    pub const ALL: [CoachPersona; 5] = [
        Self::Mentor,
        Self::Editor,
        Self::Socratic,
        Self::Sergeant,
        Self::Peer,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Mentor => "mentor",
            Self::Editor => "editor",
            Self::Socratic => "socratic",
            Self::Sergeant => "sergeant",
            Self::Peer => "peer",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mentor => "Mentor",
            Self::Editor => "Editor",
            Self::Socratic => "Socratic",
            Self::Sergeant => "Drill sergeant",
            Self::Peer => "Peer",
        }
    }

    pub fn trait_line(self) -> &'static str {
        match self {
            Self::Mentor => "Warm, encouraging",
            Self::Editor => "Direct, short",
            Self::Socratic => "Asks, never tells",
            Self::Sergeant => "Strict, holds the line",
            Self::Peer => "Casual, a friend",
        }
    }

    /// How the coach model should write.
    pub fn style(self) -> &'static str {
        match self {
            Self::Mentor => "warm and encouraging: name one real strength first, then one specific next step",
            Self::Editor => "direct and brief, like a good editor: what worked in a few words, then the one fix",
            Self::Socratic => "Socratic: ask one question that leads the speaker to the fix, without stating it",
            Self::Sergeant => "strict and brisk, like a drill sergeant who respects effort: no praise unless earned, one firm order",
            Self::Peer => "casual and friendly, like a colleague who's on your side",
        }
    }

    /// What the coach says to a rep, used when there's no coach model or its
    /// note didn't pass the guard.
    pub fn line(self, skill: &str, met: Option<bool>, missing: Option<&str>) -> String {
        let fix = missing.map(lower_first);
        let lower = skill.to_lowercase();
        let met = met == Some(true);
        match (self, met) {
            (Self::Mentor, true) => match fix {
                Some(f) => format!("That's the one for {lower}. Next time, add this too: {f}."),
                None => format!("That's the one. {skill} was clear all the way through."),
            },
            (Self::Mentor, false) => {
                format!("Good start. Keep at {lower}; it's the one thing to bring in next time.")
            }
            (Self::Editor, true) => match fix {
                Some(f) => format!("{skill}: done. Still missing: {f}."),
                None => format!("{skill}: done. Nothing to cut."),
            },
            (Self::Editor, false) => format!("{skill} didn't land. Fix that first."),
            (Self::Socratic, true) => match fix {
                Some(f) => format!("You nailed {lower}. What would it take to also say {f}?"),
                None => "You nailed it. What made this one different from your usual?".to_string(),
            },
            (Self::Socratic, false) => {
                "If someone heard only your first sentence, would they know what you need?"
                    .to_string()
            }
            (Self::Sergeant, true) => match fix {
                Some(f) => format!("{skill}: acceptable. Now add: {f}. Again."),
                None => format!("{skill}: done. Do it again, the same way."),
            },
            (Self::Sergeant, false) => format!("{skill} missing. Again, and lead with it."),
            (Self::Peer, true) => match fix {
                Some(f) => format!("Nice, {lower} landed. I'd just add {f}."),
                None => "Nice one. That's exactly it.".to_string(),
            },
            (Self::Peer, false) => format!("Close! Just work {lower} in and you're there."),
        }
    }

    /// How the coach reacts to the same rep, for choosing one.
    pub fn sample(self) -> String {
        self.line(
            "Say what done looks like",
            Some(true),
            Some("Task stated first"),
        )
    }

    /// Speech rate and pitch for the WebView voice, so each coach sounds apart.
    pub fn voice(self) -> (f32, f32) {
        match self {
            Self::Mentor => (0.95, 1.0),
            Self::Editor => (1.05, 0.95),
            Self::Socratic => (0.92, 1.05),
            Self::Sergeant => (1.1, 0.85),
            Self::Peer => (1.02, 1.1),
        }
    }

    /// The prompt for a coach model's note on a rep.
    pub fn note_prompt(
        self,
        skill: &CoachSkill,
        results: &[CheckResult],
        facts: &[String],
        score: Option<f64>,
    ) -> String {
        let mut lines = vec![
            format!("You are a speaking coach. Your style is {}.", self.style()),
            "Write one or two short sentences of feedback on a practice attempt, speaking to the person as \"you\".".to_string(),
            format!("The skill they practised: {}.", skill.title),
            "How the attempt did on each check:".to_string(),
        ];
        for result in results {
            let answer = result.answer.map(|a| a.id()).unwrap_or("not known");
            let this = if result.check == skill.check {
                " (this skill)"
            } else {
                ""
            };
            lines.push(format!("- {}: {answer}{this}", result.check.label()));
        }
        if let Some(score) = score {
            lines.push(format!("Score: {score:.1} out of 5."));
        }
        if !facts.is_empty() {
            lines.push("Counted facts you may use:".to_string());
            lines.extend(facts.iter().map(|f| format!("- {f}")));
        }
        lines.push("Only use numbers listed above. Never quote the attempt. No emoji. At most 240 characters.".to_string());
        lines.push("Reply with the feedback only.".to_string());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coach::scoring::note_guard;
    use crate::coach::skill::all_skills;

    #[test]
    fn every_coach_has_a_line_for_every_outcome() {
        for persona in CoachPersona::ALL {
            for met in [Some(true), Some(false), None] {
                let line = persona.line("Ask first", met, Some("Says when, and why it matters"));
                assert!(!line.is_empty() && line.chars().count() <= note_guard::MAX_CHARACTERS);
            }
            let prompt =
                persona.note_prompt(&all_skills()[0], &[], &["pace 150".to_string()], Some(4.0));
            assert!(prompt.contains("4.0 out of 5"));
        }
        assert_eq!(
            CoachPersona::Mentor.sample(),
            "That's the one for say what done looks like. Next time, add this too: task stated first."
        );
    }
}
