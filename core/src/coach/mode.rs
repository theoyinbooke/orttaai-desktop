//! The way someone was talking, from the app they dictated into: to an AI
//! agent, to people, or thinking on the page.
//!
//! The Mac keys apps by bundle id. On Linux and Windows the engine records an
//! app id (an X11 `WM_CLASS`, a `.desktop` id or an executable name/path) and
//! a display name, so the built-in table also knows those spellings.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoachMode {
    Ai,
    People,
    Writing,
    /// Browsers, unknown apps and anything the app couldn't be told for.
    Elsewhere,
}

impl CoachMode {
    pub const ALL: [CoachMode; 4] = [Self::Ai, Self::People, Self::Writing, Self::Elsewhere];
    /// The modes that get a voice shape.
    pub const SHAPED: [CoachMode; 3] = [Self::Ai, Self::People, Self::Writing];

    pub fn id(self) -> &'static str {
        match self {
            Self::Ai => "ai",
            Self::People => "people",
            Self::Writing => "writing",
            Self::Elsewhere => "elsewhere",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Ai => "Talking to AI",
            Self::People => "Talking to people",
            Self::Writing => "Thinking and writing",
            Self::Elsewhere => "Elsewhere",
        }
    }

    pub fn short_title(self) -> &'static str {
        match self {
            Self::Ai => "AI",
            Self::People => "People",
            Self::Writing => "Writing",
            Self::Elsewhere => "Elsewhere",
        }
    }
}

/// What the person chose for an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoachAppRule {
    Ai,
    People,
    Writing,
    Elsewhere,
    /// Coach never reads it.
    Off,
}

impl CoachAppRule {
    pub const ALL: [CoachAppRule; 5] = [
        Self::Ai,
        Self::People,
        Self::Writing,
        Self::Elsewhere,
        Self::Off,
    ];

    pub fn mode(self) -> Option<CoachMode> {
        match self {
            Self::Ai => Some(CoachMode::Ai),
            Self::People => Some(CoachMode::People),
            Self::Writing => Some(CoachMode::Writing),
            Self::Elsewhere => Some(CoachMode::Elsewhere),
            Self::Off => None,
        }
    }

    pub fn from_mode(mode: CoachMode) -> Self {
        match mode {
            CoachMode::Ai => Self::Ai,
            CoachMode::People => Self::People,
            CoachMode::Writing => Self::Writing,
            CoachMode::Elsewhere => Self::Elsewhere,
        }
    }

    pub fn id(self) -> &'static str {
        self.mode().map(CoachMode::id).unwrap_or("off")
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.id() == id)
    }
}

/// App key → rule, as kept in `Settings::coach_app_overrides`.
pub type Overrides = BTreeMap<String, CoachAppRule>;

/// Parse the stored overrides, dropping anything unknown.
pub fn parse_overrides(raw: &BTreeMap<String, String>) -> Overrides {
    raw.iter()
        .filter_map(|(k, v)| CoachAppRule::from_id(v).map(|r| (k.clone(), r)))
        .collect()
}

/// Mac bundle identifiers (lowercased), kept so rows synced from a Mac file the same way.
const BY_BUNDLE_ID: &[(&str, CoachMode)] = &[
    ("com.anthropic.claudefordesktop", CoachMode::Ai),
    ("com.openai.codex", CoachMode::Ai),
    ("com.openai.chat", CoachMode::Ai),
    ("com.todesktop.230313mzl4w4u92", CoachMode::Ai),
    ("com.exafunction.windsurf", CoachMode::Ai),
    ("com.microsoft.vscode", CoachMode::Ai),
    ("com.microsoft.vscodeinsiders", CoachMode::Ai),
    ("com.vscodium", CoachMode::Ai),
    ("dev.zed.zed", CoachMode::Ai),
    ("com.apple.terminal", CoachMode::Ai),
    ("com.googlecode.iterm2", CoachMode::Ai),
    ("com.mitchellh.ghostty", CoachMode::Ai),
    ("dev.warp.warp-stable", CoachMode::Ai),
    ("com.github.wez.wezterm", CoachMode::Ai),
    ("net.kovidgoyal.kitty", CoachMode::Ai),
    ("com.cmuxterm.app", CoachMode::Ai),
    ("com.t3tools.t3code", CoachMode::Ai),
    ("com.google.antigravity", CoachMode::Ai),
    ("ai.opencode.desktop", CoachMode::Ai),
    ("com.orttaai.orttaai", CoachMode::Ai),
    ("com.tinyspeck.slackmacgap", CoachMode::People),
    ("com.apple.mail", CoachMode::People),
    ("com.apple.mobilesms", CoachMode::People),
    ("net.whatsapp.whatsapp", CoachMode::People),
    ("desktop.whatsapp", CoachMode::People),
    ("ru.keepcoder.telegram", CoachMode::People),
    ("com.tdesktop.telegram", CoachMode::People),
    ("com.hnc.discord", CoachMode::People),
    ("com.microsoft.teams2", CoachMode::People),
    ("com.microsoft.teams", CoachMode::People),
    ("com.microsoft.outlook", CoachMode::People),
    ("com.readdle.smartemail-mac", CoachMode::People),
    ("com.superhuman.electron", CoachMode::People),
    ("com.facebook.archon", CoachMode::People),
    ("com.apple.notes", CoachMode::Writing),
    ("com.apple.iwork.pages", CoachMode::Writing),
    ("md.obsidian", CoachMode::Writing),
    ("com.microsoft.word", CoachMode::Writing),
    ("notion.id", CoachMode::Writing),
    ("net.shinyfrog.bear", CoachMode::Writing),
    ("com.apple.textedit", CoachMode::Writing),
    ("com.ulyssesapp.mac", CoachMode::Writing),
    ("com.lukilabs.lukiapp", CoachMode::Writing),
];

/// Linux and Windows app ids: X11 `WM_CLASS`, `.desktop` ids (reverse-DNS
/// Flatpak ids included) and executable names, lowercased with any path and
/// `.exe`/`.desktop` suffix removed.
const BY_DESKTOP_ID: &[(&str, CoachMode)] = &[
    // Agents, and the editors and terminals they run in.
    ("claude", CoachMode::Ai),
    ("chatgpt", CoachMode::Ai),
    ("codex", CoachMode::Ai),
    ("code", CoachMode::Ai),
    ("code-oss", CoachMode::Ai),
    ("code-insiders", CoachMode::Ai),
    ("codium", CoachMode::Ai),
    ("vscodium", CoachMode::Ai),
    ("com.visualstudio.code", CoachMode::Ai),
    ("com.vscodium.codium", CoachMode::Ai),
    ("cursor", CoachMode::Ai),
    ("windsurf", CoachMode::Ai),
    ("zed", CoachMode::Ai),
    ("zeditor", CoachMode::Ai),
    ("dev.zed.zed", CoachMode::Ai),
    ("antigravity", CoachMode::Ai),
    ("opencode", CoachMode::Ai),
    ("orttaai", CoachMode::Ai),
    ("gnome-terminal", CoachMode::Ai),
    ("gnome-terminal-server", CoachMode::Ai),
    ("org.gnome.terminal", CoachMode::Ai),
    ("org.gnome.console", CoachMode::Ai),
    ("kgx", CoachMode::Ai),
    ("ptyxis", CoachMode::Ai),
    ("org.gnome.ptyxis", CoachMode::Ai),
    ("konsole", CoachMode::Ai),
    ("org.kde.konsole", CoachMode::Ai),
    ("xterm", CoachMode::Ai),
    ("uxterm", CoachMode::Ai),
    ("alacritty", CoachMode::Ai),
    ("kitty", CoachMode::Ai),
    ("wezterm", CoachMode::Ai),
    ("wezterm-gui", CoachMode::Ai),
    ("org.wezfurlong.wezterm", CoachMode::Ai),
    ("ghostty", CoachMode::Ai),
    ("com.mitchellh.ghostty", CoachMode::Ai),
    ("tilix", CoachMode::Ai),
    ("terminator", CoachMode::Ai),
    ("xfce4-terminal", CoachMode::Ai),
    ("foot", CoachMode::Ai),
    ("footclient", CoachMode::Ai),
    ("warp", CoachMode::Ai),
    ("warp-terminal", CoachMode::Ai),
    ("windowsterminal", CoachMode::Ai),
    ("wt", CoachMode::Ai),
    ("cmd", CoachMode::Ai),
    ("powershell", CoachMode::Ai),
    ("pwsh", CoachMode::Ai),
    ("conhost", CoachMode::Ai),
    ("openconsole", CoachMode::Ai),
    // People.
    ("slack", CoachMode::People),
    ("com.slack.slack", CoachMode::People),
    ("discord", CoachMode::People),
    ("com.discordapp.discord", CoachMode::People),
    ("teams", CoachMode::People),
    ("ms-teams", CoachMode::People),
    ("msteams", CoachMode::People),
    ("teams-for-linux", CoachMode::People),
    ("thunderbird", CoachMode::People),
    ("org.mozilla.thunderbird", CoachMode::People),
    ("betterbird", CoachMode::People),
    ("outlook", CoachMode::People),
    ("olk", CoachMode::People),
    ("hxoutlook", CoachMode::People),
    ("telegram", CoachMode::People),
    ("telegram-desktop", CoachMode::People),
    ("telegramdesktop", CoachMode::People),
    ("org.telegram.desktop", CoachMode::People),
    ("whatsapp", CoachMode::People),
    ("whatsapp-for-linux", CoachMode::People),
    ("signal", CoachMode::People),
    ("signal-desktop", CoachMode::People),
    ("org.signal.signal", CoachMode::People),
    ("element", CoachMode::People),
    ("im.riot.riot", CoachMode::People),
    ("evolution", CoachMode::People),
    ("org.gnome.evolution", CoachMode::People),
    ("geary", CoachMode::People),
    ("org.gnome.geary", CoachMode::People),
    ("mailspring", CoachMode::People),
    ("kmail", CoachMode::People),
    ("org.kde.kmail2", CoachMode::People),
    ("messenger", CoachMode::People),
    ("skype", CoachMode::People),
    ("zulip", CoachMode::People),
    ("mattermost", CoachMode::People),
    // Thinking on the page.
    ("obsidian", CoachMode::Writing),
    ("md.obsidian.obsidian", CoachMode::Writing),
    ("notion", CoachMode::Writing),
    ("notion-app", CoachMode::Writing),
    ("winword", CoachMode::Writing),
    ("libreoffice-writer", CoachMode::Writing),
    ("lowriter", CoachMode::Writing),
    ("onlyoffice", CoachMode::Writing),
    ("gedit", CoachMode::Writing),
    ("org.gnome.gedit", CoachMode::Writing),
    ("gnome-text-editor", CoachMode::Writing),
    ("org.gnome.texteditor", CoachMode::Writing),
    ("kate", CoachMode::Writing),
    ("org.kde.kate", CoachMode::Writing),
    ("kwrite", CoachMode::Writing),
    ("mousepad", CoachMode::Writing),
    ("notepad", CoachMode::Writing),
    ("notepad++", CoachMode::Writing),
    ("typora", CoachMode::Writing),
    ("marktext", CoachMode::Writing),
    ("joplin", CoachMode::Writing),
    ("logseq", CoachMode::Writing),
    ("onenote", CoachMode::Writing),
    ("ghostwriter", CoachMode::Writing),
    ("apostrophe", CoachMode::Writing),
    ("org.gnome.gitlab.somas.apostrophe", CoachMode::Writing),
    ("anytype", CoachMode::Writing),
    ("craft", CoachMode::Writing),
    // Browsers file as elsewhere, as on the Mac.
    ("firefox", CoachMode::Elsewhere),
    ("navigator", CoachMode::Elsewhere),
    ("org.mozilla.firefox", CoachMode::Elsewhere),
    ("chrome", CoachMode::Elsewhere),
    ("google-chrome", CoachMode::Elsewhere),
    ("chromium", CoachMode::Elsewhere),
    ("chromium-browser", CoachMode::Elsewhere),
    ("msedge", CoachMode::Elsewhere),
    ("microsoft-edge", CoachMode::Elsewhere),
    ("brave", CoachMode::Elsewhere),
    ("brave-browser", CoachMode::Elsewhere),
    ("opera", CoachMode::Elsewhere),
    ("vivaldi", CoachMode::Elsewhere),
    ("vivaldi-stable", CoachMode::Elsewhere),
];

/// Lowercased display names, for rows that only kept the name.
const BY_NAME: &[(&str, CoachMode)] = &[
    ("claude", CoachMode::Ai),
    ("codex", CoachMode::Ai),
    ("chatgpt", CoachMode::Ai),
    ("cursor", CoachMode::Ai),
    ("windsurf", CoachMode::Ai),
    ("visual studio code", CoachMode::Ai),
    ("code", CoachMode::Ai),
    ("vscodium", CoachMode::Ai),
    ("zed", CoachMode::Ai),
    ("terminal", CoachMode::Ai),
    ("iterm2", CoachMode::Ai),
    ("iterm", CoachMode::Ai),
    ("ghostty", CoachMode::Ai),
    ("warp", CoachMode::Ai),
    ("wezterm", CoachMode::Ai),
    ("kitty", CoachMode::Ai),
    ("cmux", CoachMode::Ai),
    ("t3 code", CoachMode::Ai),
    ("t3 code (beta)", CoachMode::Ai),
    ("antigravity", CoachMode::Ai),
    ("opencode", CoachMode::Ai),
    ("devin", CoachMode::Ai),
    ("orttaai", CoachMode::Ai),
    ("windows terminal", CoachMode::Ai),
    ("windows powershell", CoachMode::Ai),
    ("powershell", CoachMode::Ai),
    ("command prompt", CoachMode::Ai),
    ("gnome terminal", CoachMode::Ai),
    ("console", CoachMode::Ai),
    ("konsole", CoachMode::Ai),
    ("alacritty", CoachMode::Ai),
    ("tilix", CoachMode::Ai),
    ("terminator", CoachMode::Ai),
    ("xterm", CoachMode::Ai),
    ("slack", CoachMode::People),
    ("mail", CoachMode::People),
    ("messages", CoachMode::People),
    ("whatsapp", CoachMode::People),
    ("telegram", CoachMode::People),
    ("telegram desktop", CoachMode::People),
    ("discord", CoachMode::People),
    ("microsoft teams", CoachMode::People),
    ("teams", CoachMode::People),
    ("microsoft outlook", CoachMode::People),
    ("outlook", CoachMode::People),
    ("spark", CoachMode::People),
    ("superhuman", CoachMode::People),
    ("messenger", CoachMode::People),
    ("thunderbird", CoachMode::People),
    ("mozilla thunderbird", CoachMode::People),
    ("signal", CoachMode::People),
    ("element", CoachMode::People),
    ("evolution", CoachMode::People),
    ("geary", CoachMode::People),
    ("notes", CoachMode::Writing),
    ("pages", CoachMode::Writing),
    ("obsidian", CoachMode::Writing),
    ("microsoft word", CoachMode::Writing),
    ("word", CoachMode::Writing),
    ("notion", CoachMode::Writing),
    ("bear", CoachMode::Writing),
    ("textedit", CoachMode::Writing),
    ("ulysses", CoachMode::Writing),
    ("craft", CoachMode::Writing),
    ("libreoffice writer", CoachMode::Writing),
    ("text editor", CoachMode::Writing),
    ("gedit", CoachMode::Writing),
    ("kate", CoachMode::Writing),
    ("notepad", CoachMode::Writing),
    ("notepad++", CoachMode::Writing),
    ("onenote", CoachMode::Writing),
    ("typora", CoachMode::Writing),
    ("joplin", CoachMode::Writing),
    ("logseq", CoachMode::Writing),
];

fn lookup(table: &[(&str, CoachMode)], key: &str) -> Option<CoachMode> {
    table.iter().find(|(k, _)| *k == key).map(|(_, m)| *m)
}

/// An app id as the desktop table keys it: lowercased, without a path or
/// a `.exe` / `.desktop` suffix.
pub fn normalize_app_id(id: &str) -> String {
    let id = id.trim().to_lowercase();
    let base = id.rsplit(['/', '\\']).next().unwrap_or(&id);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    let base = base.strip_suffix(".desktop").unwrap_or(base);
    base.to_string()
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty())
}

/// The key a person's choice for an app is kept under: the app id, else
/// `name:` and the name, lowercased.
pub fn app_key(app_id: Option<&str>, app_name: Option<&str>) -> Option<String> {
    if let Some(id) = non_empty(app_id) {
        return Some(id);
    }
    non_empty(app_name).map(|n| format!("name:{n}"))
}

pub fn builtin_mode(app_id: Option<&str>, app_name: Option<&str>) -> CoachMode {
    if let Some(id) = non_empty(app_id) {
        if let Some(mode) = lookup(BY_BUNDLE_ID, &id) {
            return mode;
        }
        if let Some(mode) = lookup(BY_DESKTOP_ID, &normalize_app_id(&id)) {
            return mode;
        }
    }
    if let Some(name) = non_empty(app_name) {
        if let Some(mode) = lookup(BY_NAME, &name) {
            return mode;
        }
        if let Some(mode) = lookup(BY_DESKTOP_ID, &normalize_app_id(&name)) {
            return mode;
        }
    }
    CoachMode::Elsewhere
}

/// The mode Coach files a dictation under, or `None` when its app is off.
pub fn mode_for(
    app_id: Option<&str>,
    app_name: Option<&str>,
    overrides: &Overrides,
) -> Option<CoachMode> {
    if let Some(key) = app_key(app_id, app_name) {
        if let Some(rule) = overrides.get(&key) {
            return rule.mode();
        }
    }
    Some(builtin_mode(app_id, app_name))
}

/// The rule in force for an app: the person's choice, else the table.
pub fn rule_for(
    app_id: Option<&str>,
    app_name: Option<&str>,
    overrides: &Overrides,
) -> CoachAppRule {
    if let Some(rule) = app_key(app_id, app_name).and_then(|k| overrides.get(&k).copied()) {
        return rule;
    }
    CoachAppRule::from_mode(builtin_mode(app_id, app_name))
}

/// Keys of apps switched off.
pub fn off_keys(overrides: &Overrides) -> Vec<String> {
    overrides
        .iter()
        .filter(|(_, r)| **r == CoachAppRule::Off)
        .map(|(k, _)| k.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_table() {
        assert_eq!(
            builtin_mode(Some("com.anthropic.claudefordesktop"), Some("Claude")),
            CoachMode::Ai
        );
        assert_eq!(
            builtin_mode(Some("com.apple.Terminal"), None),
            CoachMode::Ai
        );
        assert_eq!(
            builtin_mode(Some("com.tinyspeck.slackmacgap"), Some("Slack")),
            CoachMode::People
        );
        assert_eq!(builtin_mode(Some("md.obsidian"), None), CoachMode::Writing);
        assert_eq!(
            builtin_mode(Some("com.apple.Safari"), Some("Safari")),
            CoachMode::Elsewhere
        );
        assert_eq!(
            builtin_mode(None, Some("Orttakey on iPhone")),
            CoachMode::Elsewhere
        );
        assert_eq!(builtin_mode(None, Some(" Slack ")), CoachMode::People);
        assert_eq!(
            builtin_mode(Some("com.cmuxterm.app"), Some("cmux")),
            CoachMode::Ai
        );
        assert_eq!(
            builtin_mode(Some("com.orttaai.Orttaai"), Some("Orttaai")),
            CoachMode::Ai
        );
    }

    #[test]
    fn linux_and_windows_app_names() {
        assert_eq!(
            builtin_mode(Some("Code"), Some("Visual Studio Code")),
            CoachMode::Ai
        );
        assert_eq!(
            builtin_mode(Some("C:\\Program Files\\Microsoft VS Code\\Code.exe"), None),
            CoachMode::Ai
        );
        assert_eq!(
            builtin_mode(Some("gnome-terminal-server"), None),
            CoachMode::Ai
        );
        assert_eq!(
            builtin_mode(Some("WindowsTerminal.exe"), None),
            CoachMode::Ai
        );
        assert_eq!(builtin_mode(Some("slack.exe"), None), CoachMode::People);
        assert_eq!(
            builtin_mode(Some("org.mozilla.Thunderbird.desktop"), None),
            CoachMode::People
        );
        assert_eq!(builtin_mode(None, Some("discord")), CoachMode::People);
        assert_eq!(builtin_mode(Some("ms-teams.exe"), None), CoachMode::People);
        assert_eq!(builtin_mode(Some("OUTLOOK.EXE"), None), CoachMode::People);
        assert_eq!(builtin_mode(Some("WINWORD.EXE"), None), CoachMode::Writing);
        assert_eq!(builtin_mode(Some("obsidian"), None), CoachMode::Writing);
        assert_eq!(
            builtin_mode(Some("firefox"), Some("Mozilla Firefox")),
            CoachMode::Elsewhere
        );
        assert_eq!(builtin_mode(Some("chrome.exe"), None), CoachMode::Elsewhere);
        assert_eq!(builtin_mode(Some("msedge.exe"), None), CoachMode::Elsewhere);
        assert_eq!(
            builtin_mode(Some("unknown-thing"), Some("Something")),
            CoachMode::Elsewhere
        );
    }

    #[test]
    fn overrides_win() {
        let mut overrides = Overrides::new();
        overrides.insert("com.apple.safari".into(), CoachAppRule::Writing);
        overrides.insert("com.tinyspeck.slackmacgap".into(), CoachAppRule::Off);
        overrides.insert("name:codex".into(), CoachAppRule::People);
        assert_eq!(
            mode_for(Some("com.apple.Safari"), Some("Safari"), &overrides),
            Some(CoachMode::Writing)
        );
        assert_eq!(
            mode_for(Some("com.tinyspeck.slackmacgap"), Some("Slack"), &overrides),
            None
        );
        assert_eq!(
            mode_for(None, Some("Codex"), &overrides),
            Some(CoachMode::People)
        );
        assert_eq!(
            mode_for(Some("com.openai.codex"), Some("Codex"), &overrides),
            Some(CoachMode::Ai)
        );
        assert_eq!(app_key(None, None), None);
        assert_eq!(
            off_keys(&overrides),
            vec!["com.tinyspeck.slackmacgap".to_string()]
        );
        assert_eq!(
            rule_for(None, Some("slack"), &overrides),
            CoachAppRule::People
        );
    }

    #[test]
    fn stored_overrides_parse_strictly() {
        let mut raw = BTreeMap::new();
        raw.insert("firefox".to_string(), "writing".to_string());
        raw.insert("x".to_string(), "nonsense".to_string());
        let parsed = parse_overrides(&raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed["firefox"], CoachAppRule::Writing);
    }
}
