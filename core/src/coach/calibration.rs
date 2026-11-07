//! A judge has to earn its place. Every question comes with dictations whose
//! answer is plain, two that clearly do it and two that clearly don't; a
//! model is used as judge only if it gets them right, both ways.

use super::scoring::JudgeAnswer;
use super::skill::CoachQuestion;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationExample {
    pub question: CoachQuestion,
    /// Always yes or no: the examples are chosen to be clear.
    pub expected: JudgeAnswer,
    pub text: &'static str,
}

/// How a model did on the examples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeGrade {
    pub model: String,
    /// Points over all examples: right 1, partly 0.5, wrong or no answer 0.
    pub accuracy: f64,
    /// Of the examples that clearly do it, how many it said yes to.
    pub yes_recall: f64,
    /// Of the examples that clearly don't, how many it said no to.
    pub no_recall: f64,
    pub answered: usize,
    pub total: usize,
    pub ms_per_answer: Option<i64>,
    pub version: i64,
    pub graded_at: i64,
}

pub const PASS_ACCURACY: f64 = 0.8;
pub const PASS_RECALL: f64 = 0.7;
/// Bump whenever the examples change, so every model is graded again.
pub const VERSION: i64 = 1;

impl JudgeGrade {
    /// Right often enough, both ways, and answering nearly every time. A
    /// model that says the same thing to everything can't pass.
    pub fn passes(&self) -> bool {
        self.total > 0
            && self.answered as f64 >= self.total as f64 * 0.9
            && self.accuracy >= PASS_ACCURACY
            && self.yes_recall >= PASS_RECALL
            && self.no_recall >= PASS_RECALL
    }

    /// Graded against the examples in use now.
    pub fn is_current(&self) -> bool {
        self.version == VERSION
    }

    /// Why it failed, in a line (Mac `whyFailed`).
    pub fn why_failed(&self) -> &'static str {
        if (self.answered as f64) < self.total as f64 * 0.9 {
            "It didn't answer often enough."
        } else if self.no_recall < PASS_RECALL {
            "It said yes too often, even when the answer was plainly no."
        } else if self.yes_recall < PASS_RECALL {
            "It said no too often, even when the answer was plainly yes."
        } else {
            "It got too many wrong."
        }
    }
}

pub fn grade(
    model: &str,
    answers: &[(&CalibrationExample, Option<JudgeAnswer>)],
    ms_per_answer: Option<i64>,
    at: i64,
) -> JudgeGrade {
    let mut points = 0.0;
    let (mut yes_hits, mut yes_total, mut no_hits, mut no_total, mut answered) = (0, 0, 0, 0, 0);
    for (example, answer) in answers {
        if answer.is_some() {
            answered += 1;
        }
        if example.expected == JudgeAnswer::Yes {
            yes_total += 1;
        } else {
            no_total += 1;
        }
        match answer {
            Some(a) if *a == example.expected => {
                points += 1.0;
                if example.expected == JudgeAnswer::Yes {
                    yes_hits += 1;
                } else {
                    no_hits += 1;
                }
            }
            Some(JudgeAnswer::Partly) => points += 0.5,
            _ => {}
        }
    }
    let ratio = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    JudgeGrade {
        model: model.to_string(),
        accuracy: if answers.is_empty() {
            0.0
        } else {
            points / answers.len() as f64
        },
        yes_recall: ratio(yes_hits, yes_total),
        no_recall: ratio(no_hits, no_total),
        answered,
        total: answers.len(),
        ms_per_answer,
        version: VERSION,
        graded_at: at,
    }
}

macro_rules! ex {
    ($q:ident, $a:ident, $t:expr) => {
        CalibrationExample {
            question: CoachQuestion::$q,
            expected: JudgeAnswer::$a,
            text: $t,
        }
    };
}

pub const EXAMPLES: &[CalibrationExample] = &[
    // Task stated first
    ex!(TaskFirst, Yes, "Add a retry to the paste step. Right now if the target app is busy the paste just fails silently, so try again once after 200 milliseconds."),
    ex!(TaskFirst, Yes, "Rename the settings tab from Audio to Microphone. It's the only thing on that page now."),
    ex!(TaskFirst, No, "So I was looking at the logs this morning and there's a lot going on with the paste timing, and I think it's related to the overlay, anyway can you make the paste retry once."),
    ex!(TaskFirst, No, "Okay, a bit of background first. We moved the models folder last week and some people still have the old path saved, which is why the download screen shows zero."),
    // Gives the context needed
    ex!(Context, Yes, "Update the onboarding copy on the permissions step. It's in PermissionStepView, it should mention that Orttaai never sends audio off the Mac, and keep it under two sentences."),
    ex!(Context, Yes, "Make the history list load faster. It reads every transcription at launch from the SQLite database; page it in batches of 200 and keep the search working across all of them."),
    ex!(Context, No, "Fix it."),
    ex!(Context, No, "Can you make that thing work better, the one we talked about."),
    // Names the file, screen, error or measurement
    ex!(FilesAndErrors, Yes, "The crash is in AudioCaptureService.swift on line 212, it says index out of range when the microphone changes mid recording."),
    ex!(FilesAndErrors, Yes, "On the Model settings screen the download progress sticks at 97 percent and the error says the checksum doesn't match."),
    ex!(FilesAndErrors, No, "Something's off with the app, it feels slow and sometimes things break."),
    ex!(FilesAndErrors, No, "Please make the experience nicer and more polished overall."),
    // Says what done looks like
    ex!(FinishLine, Yes, "Speed up the paste after switching apps. Done when the paste lands in under 100 milliseconds and the existing paste tests still pass."),
    ex!(FinishLine, Yes, "Clean up the export menu. It's finished when there are only three options and each one produces a file that opens in Numbers."),
    ex!(FinishLine, No, "Have a look at the export menu and tidy it up a bit."),
    ex!(FinishLine, No, "Start working on the paste speed, it's been bugging me."),
    // One request
    ex!(OneAsk, Yes, "Change the overlay's corner radius to 12 points to match the rest of the app."),
    ex!(OneAsk, Yes, "Write a test for the filler counter that covers um, uh and erm with stretched spellings."),
    ex!(OneAsk, No, "Fix the login bug, also redesign the settings page, and while you're at it update the docs and bump the version."),
    ex!(OneAsk, No, "Can you speed up the history page, add dark mode to the overlay, and look into why sync fails on the second Mac."),
    // A newcomer would follow it
    ex!(Colleague, Yes, "In the Orttaai repo, open Settings, General. Add a toggle called Play a sound when dictation starts, off by default, saved in AppSettings, and play the Tink system sound when it's on."),
    ex!(Colleague, Yes, "Go to the Releases folder, run release_dmg.sh with no options, and upload the DMG it prints to the GitHub release called v1.16.0."),
    ex!(Colleague, No, "Do the usual thing with the build and then the other step."),
    ex!(Colleague, No, "Same as last time but for the new one, you know."),
    // The ask comes first
    ex!(AskFirst, Yes, "Could you review my pull request before 3? It's the paste fix, small, about 40 lines."),
    ex!(AskFirst, Yes, "Can you send me the slides from Tuesday? I need them for the client call tomorrow."),
    ex!(AskFirst, No, "Hey, hope your week's going well. I've been heads down on the paste fix and it's finally working. When you get a chance could you review it?"),
    ex!(AskFirst, No, "So the client moved the meeting again and now it's Thursday and the deck isn't ready and I was wondering if you could help with the slides."),
    // Says when and why
    ex!(WhenWhy, Yes, "Could you review the paste fix by 3 today? It has to go into tonight's build for the client demo tomorrow."),
    ex!(WhenWhy, Yes, "Please send the invoice by Friday, our books close on Monday and it needs to be in this month."),
    ex!(WhenWhy, No, "Could you review the paste fix?"),
    ex!(WhenWhy, No, "Send me the invoice when you can."),
    // Friendly and direct
    ex!(Tone, Yes, "Thanks for jumping on this. Could you send the updated numbers by noon? That gives me time to check them before the call."),
    ex!(Tone, Yes, "Hi Sam, quick one: can you add me to the design review? Thanks."),
    ex!(Tone, No, "Send the numbers. Now. I've asked twice already."),
    ex!(Tone, No, "I'm so sorry to bother you, I really hate to ask, and please ignore this if you're busy, but I was maybe wondering if possibly you might have a moment at some point."),
    // Main point first
    ex!(PointFirst, Yes, "We should ship the paste fix on its own this week. It's ready, it's small, and waiting for the redesign would hold it back a month."),
    ex!(PointFirst, Yes, "The new model is twice as fast with the same accuracy. On our test set it takes 80 milliseconds instead of 160."),
    ex!(PointFirst, No, "So there are a few things to think about, and the timing is tricky, and people have different views, but in the end I think we should ship the paste fix on its own."),
    ex!(PointFirst, No, "Let me start with some history about how the old model was trained and the data it used back in the spring."),
    // One idea per sentence
    ex!(OneIdea, Yes, "The paste fix is ready. It cuts the delay to 80 milliseconds. I'd like to ship it on Thursday."),
    ex!(OneIdea, Yes, "Coach counts your fillers. It compares this month with last. It shows the change on your voice shape."),
    ex!(OneIdea, No, "The paste fix is ready and it cuts the delay to 80 milliseconds which matters because people noticed the lag, and I'd like to ship it Thursday unless the redesign is ready, in which case we could bundle them, although that adds risk."),
    ex!(OneIdea, No, "We moved the models folder, which broke downloads for some people because their saved path pointed at the old place, and also the progress bar was wrong, so I fixed both and added a migration and some tests."),
    // Ends with a summary or next step
    ex!(Summary, Yes, "The new model is faster and just as accurate. Downloads are smaller too. Next step: I'll switch the default on Monday."),
    ex!(Summary, Yes, "We looked at three ways to store the habits and picked the simplest. So, to sum up: one table, one row per habit."),
    ex!(Summary, No, "The new model is faster. It also uses less memory on older Macs. The download is 300 megabytes."),
    ex!(Summary, No, "There were three options for storing habits. The first used a table per habit. The second used a single JSON blob."),
    // Plain words
    ex!(PlainWords, Yes, "Coach listens to how you talk when you dictate and shows you one thing to get better at each week."),
    ex!(PlainWords, Yes, "The app keeps everything on your Mac. Nothing you say is sent anywhere."),
    ex!(PlainWords, No, "We leverage a bootstrapped CI over the delta of hit-rate distributions against a control drift baseline to adjudicate habit efficacy."),
    ex!(PlainWords, No, "The ANE-backed CoreML pipeline does int8 PTQ on the LSTM decoder with a stateful KV cache for low TTFT."),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn grade_with(
        mut answer: impl FnMut(&CalibrationExample) -> Option<JudgeAnswer>,
    ) -> JudgeGrade {
        let answers: Vec<_> = EXAMPLES.iter().map(|e| (e, answer(e))).collect();
        grade("m", &answers, Some(120), 0)
    }

    #[test]
    fn every_question_has_two_clear_yeses_and_two_clear_nos() {
        for question in CoachQuestion::ALL {
            let mine: Vec<_> = EXAMPLES.iter().filter(|e| e.question == question).collect();
            assert_eq!(
                mine.iter()
                    .filter(|e| e.expected == JudgeAnswer::Yes)
                    .count(),
                2,
                "{question:?}"
            );
            assert_eq!(
                mine.iter()
                    .filter(|e| e.expected == JudgeAnswer::No)
                    .count(),
                2,
                "{question:?}"
            );
        }
        assert_eq!(
            EXAMPLES
                .iter()
                .map(|e| e.text)
                .collect::<HashSet<_>>()
                .len(),
            EXAMPLES.len()
        );
    }

    #[test]
    fn a_judge_that_gets_them_right_passes() {
        let perfect = grade_with(|e| Some(e.expected));
        assert_eq!(perfect.accuracy, 1.0);
        assert_eq!(perfect.yes_recall, 1.0);
        assert_eq!(perfect.no_recall, 1.0);
        assert!(perfect.passes());
        assert!(perfect.is_current());
        assert_eq!(perfect.ms_per_answer, Some(120));
    }

    #[test]
    fn a_judge_that_says_the_same_thing_to_everything_fails() {
        let always_no = grade_with(|_| Some(JudgeAnswer::No));
        assert_eq!(always_no.accuracy, 0.5);
        assert_eq!(always_no.yes_recall, 0.0);
        assert!(!always_no.passes());
        assert_eq!(
            always_no.why_failed(),
            "It said no too often, even when the answer was plainly yes."
        );
        let always_yes = grade_with(|_| Some(JudgeAnswer::Yes));
        assert_eq!(always_yes.no_recall, 0.0);
        assert!(!always_yes.passes());
    }

    #[test]
    fn partly_earns_half_and_too_many_misses_fail() {
        let hedging = grade_with(|_| Some(JudgeAnswer::Partly));
        assert_eq!(hedging.accuracy, 0.5);
        assert!(!hedging.passes());
        let mut index = 0;
        let quiet = grade_with(|e| {
            index += 1;
            if index % 5 == 0 {
                None
            } else {
                Some(e.expected)
            }
        });
        assert!(quiet.answered < quiet.total);
        assert!(!quiet.passes());
        let mut index = 0;
        let mostly_right = grade_with(|e| {
            index += 1;
            if index % 10 == 0 {
                Some(if e.expected == JudgeAnswer::Yes {
                    JudgeAnswer::No
                } else {
                    JudgeAnswer::Yes
                })
            } else {
                Some(e.expected)
            }
        });
        assert!(mostly_right.accuracy >= 0.8);
        assert!(mostly_right.passes());
    }

    #[test]
    fn grades_from_older_examples_are_not_current() {
        let mut old = grade_with(|e| Some(e.expected));
        old.version = VERSION - 1;
        assert!(!old.is_current());
        let back: JudgeGrade = serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
        assert_eq!(back, old);
    }
}
