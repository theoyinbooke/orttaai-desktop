//! What's New catalog shown once after an update.
//!
//! A port of the macOS `WhatsNewCatalog` + `WhatsNewCenter`: after an update,
//! the first time the window opens it shows the release's highlights (at most
//! three), one at a time. A fresh install skips it, since setup comes first.
//! The version counts as seen only when the card is closed
//! (`Settings::last_whats_new_version`), so quitting before it shows keeps it
//! for next time.

use serde::Serialize;
use std::cmp::Ordering;

/// The picture at the top of a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "symbol", rename_all = "snake_case")]
pub enum Art {
    /// A milestone badge with rays.
    Milestones,
    /// A misheard word fixed to the right one.
    Learning,
    /// The sidebar's download ring.
    Updates,
    /// One of the UI's icons (`Icon` names) in the accent rings.
    Symbol(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: &'static str,
    /// Short: about four words.
    pub title: &'static str,
    /// One or two sentences.
    pub detail: &'static str,
    pub art: Art,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Release {
    pub version: &'static str,
    pub items: &'static [Item],
}

pub const MAX_ITEMS: usize = 3;
pub const MAX_TITLE_LENGTH: usize = 32;
pub const MAX_DETAIL_LENGTH: usize = 140;

/// What's New for each release, newest first. Every release adds an entry:
/// at most three highlights, the ones people will notice, in plain words.
pub const RELEASES: &[Release] = &[Release {
    version: "0.3.0",
    items: &[
        Item {
            id: "0.3-dictation",
            title: "Dictation, rebuilt",
            detail: "Hands-free with silence stop, the Express engine, text cleanup, Polish and voice edits. Say it once and it lands right.",
            art: Art::Symbol("mic"),
        },
        Item {
            id: "0.3-memory",
            title: "Ask your own words",
            detail: "Chat AI answers from your dictations, and the Memory Graph maps the people, projects and ideas you talk about.",
            art: Art::Symbol("graph"),
        },
        Item {
            id: "0.3-progress",
            title: "Watch yourself grow",
            detail: "Analytics with Tone of Voice, badges to unlock and share, and Coach for one habit at a time, in the Mac app's design.",
            art: Art::Milestones,
        },
    ],
}];

/// Compare dotted versions numerically ("0.10.0" > "0.9.2"); a missing part
/// counts as 0, and any non-numeric suffix ("-beta") is ignored.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> Vec<u64> {
        v.trim()
            .trim_start_matches(['v', 'V'])
            .split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    }
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        match pa.get(i).unwrap_or(&0).cmp(pb.get(i).unwrap_or(&0)) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

pub fn release_for(version: &str) -> Option<&'static Release> {
    RELEASES
        .iter()
        .find(|r| compare_versions(r.version, version) == Ordering::Equal)
}

/// What to do at launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Already seen this version: nothing to do.
    Nothing,
    /// A fresh install, a downgrade, or a release without notes: record the
    /// version as seen without showing anything.
    MarkSeen,
    /// Show these notes; mark seen when the card closes.
    Show(&'static Release),
}

/// `is_existing_user` (setup already done) tells an update from a first
/// install when no version was recorded (every version before What's New).
pub fn evaluate(current: &str, last_seen: Option<&str>, is_existing_user: bool) -> Decision {
    evaluate_in(RELEASES, current, last_seen, is_existing_user)
}

pub fn evaluate_in(
    catalog: &'static [Release],
    current: &str,
    last_seen: Option<&str>,
    is_existing_user: bool,
) -> Decision {
    if last_seen == Some(current) {
        return Decision::Nothing;
    }
    let updated = match last_seen {
        Some(seen) => compare_versions(current, seen) == Ordering::Greater,
        None => is_existing_user,
    };
    let notes = catalog
        .iter()
        .find(|r| compare_versions(r.version, current) == Ordering::Equal)
        .filter(|r| !r.items.is_empty());
    match (updated, notes) {
        (true, Some(release)) => Decision::Show(release),
        _ => Decision::MarkSeen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_follows_the_rules() {
        let mut ids = std::collections::HashSet::new();
        for r in RELEASES {
            assert!(
                !r.items.is_empty() && r.items.len() <= MAX_ITEMS,
                "{}",
                r.version
            );
            for i in r.items {
                assert!(i.title.chars().count() <= MAX_TITLE_LENGTH, "{}", i.id);
                assert!(i.detail.chars().count() <= MAX_DETAIL_LENGTH, "{}", i.id);
                assert!(ids.insert(i.id), "duplicate id {}", i.id);
            }
        }
        for pair in RELEASES.windows(2) {
            assert_eq!(
                compare_versions(pair[0].version, pair[1].version),
                Ordering::Greater
            );
        }
        assert!(release_for("0.3.0").is_some());
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("0.3", "0.3.0"), Ordering::Equal);
        assert_eq!(compare_versions("v1.2.3", "1.2.4"), Ordering::Less);
        assert_eq!(compare_versions("0.3.0-beta", "0.3.0"), Ordering::Equal);
    }

    static TEST_CATALOG: &[Release] = &[Release {
        version: "2.0.0",
        items: &[Item {
            id: "2.0-a",
            title: "A",
            detail: "B",
            art: Art::Updates,
        }],
    }];

    #[test]
    fn shows_once_after_an_update_only() {
        let c = TEST_CATALOG;
        // Updated from an older version → show.
        assert_eq!(
            evaluate_in(c, "2.0.0", Some("1.9.0"), true),
            Decision::Show(&c[0])
        );
        // Already seen → nothing.
        assert_eq!(
            evaluate_in(c, "2.0.0", Some("2.0.0"), true),
            Decision::Nothing
        );
        // Fresh install (no record, setup not done) → mark seen silently.
        assert_eq!(evaluate_in(c, "2.0.0", None, false), Decision::MarkSeen);
        // Existing user from before What's New (no record) → show.
        assert_eq!(evaluate_in(c, "2.0.0", None, true), Decision::Show(&c[0]));
        // Downgrade → mark seen, don't show.
        assert_eq!(
            evaluate_in(c, "2.0.0", Some("2.1.0"), true),
            Decision::MarkSeen
        );
        // A release without notes → mark seen.
        assert_eq!(
            evaluate_in(c, "2.0.1", Some("2.0.0"), true),
            Decision::MarkSeen
        );
    }
}
