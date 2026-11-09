//! One name per app across every device. The Mac reports apps by their
//! display name ("Google Chrome"), Windows and Linux by their program file
//! ("chrome", "gnome-terminal"); in one synced archive the same app would
//! otherwise show twice in Top Apps. The Mac and Orttakey have the same rules
//! (`AppNames.swift` in OrttaaiSync); `tests/fixtures/app-names.json` keeps
//! them in step.

use std::collections::HashMap;
use std::sync::OnceLock;

pub const UNKNOWN: &str = "Unknown App";

/// Display name, then every program or display name it goes by.
const GROUPS: &[(&str, &[&str])] = &[
    ("Google Chrome", &["chrome", "googlechrome"]),
    ("Microsoft Edge", &["msedge", "microsoftedge"]),
    ("Firefox", &["firefox", "mozillafirefox"]),
    ("Brave Browser", &["brave", "bravebrowser"]),
    ("Arc", &["arc"]),
    ("Safari", &["safari"]),
    ("Opera", &["opera"]),
    ("Vivaldi", &["vivaldi"]),
    (
        "Visual Studio Code",
        &["code", "visualstudiocode", "vscode"],
    ),
    (
        "Visual Studio Code - Insiders",
        &["codeinsiders", "visualstudiocodeinsiders"],
    ),
    ("Visual Studio", &["devenv", "visualstudio"]),
    ("Cursor", &["cursor"]),
    ("Windsurf", &["windsurf"]),
    ("Zed", &["zed"]),
    ("IntelliJ IDEA", &["idea64", "idea", "intellijidea"]),
    ("PyCharm", &["pycharm64", "pycharm"]),
    ("Sublime Text", &["sublimetext"]),
    ("Slack", &["slack"]),
    ("Discord", &["discord"]),
    ("Microsoft Teams", &["teams", "msteams", "microsoftteams"]),
    ("Zoom", &["zoom", "zoomus"]),
    ("WhatsApp", &["whatsapp"]),
    ("Telegram", &["telegram", "telegramdesktop"]),
    ("Signal", &["signal"]),
    ("Microsoft Word", &["winword", "microsoftword", "word"]),
    ("Microsoft Excel", &["excel", "microsoftexcel"]),
    (
        "Microsoft PowerPoint",
        &["powerpnt", "microsoftpowerpoint", "powerpoint"],
    ),
    ("Microsoft Outlook", &["outlook", "olk", "microsoftoutlook"]),
    ("Microsoft OneNote", &["onenote", "microsoftonenote"]),
    ("Notion", &["notion"]),
    ("Obsidian", &["obsidian"]),
    ("Claude", &["claude"]),
    ("ChatGPT", &["chatgpt"]),
    ("Windows Terminal", &["windowsterminal", "wt"]),
    ("Command Prompt", &["cmd", "commandprompt"]),
    ("PowerShell", &["powershell", "pwsh", "windowspowershell"]),
    ("File Explorer", &["explorer", "fileexplorer"]),
    ("Notepad", &["notepad"]),
    (
        "Sticky Notes",
        &["stickynotes", "microsoftstickynotes", "microsoftnotes"],
    ),
    ("GNOME Terminal", &["gnometerminal", "gnometerminalserver"]),
    ("Thunderbird", &["thunderbird"]),
    ("iTerm", &["iterm", "iterm2"]),
];

/// Lowercase letters and digits only, without ".exe".
fn match_key(name: &str) -> String {
    let lower = name.to_lowercase();
    let lower = lower.strip_suffix(".exe").unwrap_or(&lower);
    lower.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn aliases() -> &'static HashMap<String, &'static str> {
    static MAP: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::new();
        for (name, keys) in GROUPS {
            for key in *keys {
                map.insert(key.to_string(), *name);
            }
            map.insert(match_key(name), *name);
        }
        map
    })
}

/// The name to show and group by.
pub fn display_name(raw: Option<&str>) -> String {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return UNKNOWN.to_string();
    };
    if let Some(known) = aliases().get(&match_key(trimmed)) {
        return known.to_string();
    }
    // A program file's name ("obsidian", "gnome-terminal"): capitalised.
    if trimmed == trimmed.to_lowercase() && !trimmed.contains(' ') {
        let stem = trimmed.strip_suffix(".exe").unwrap_or(trimmed);
        let words: Vec<String> = stem
            .split(['-', '_', '.'])
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect();
        if !words.is_empty() {
            return words.join(" ");
        }
    }
    trimmed.to_string()
}
