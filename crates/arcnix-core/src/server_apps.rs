//! Finite software discovery and remote-user desktop preferences over SSH.
use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::desktop_shortcuts::Shortcuts;
use crate::remote::{CommandStream, RemoteSession};
use crate::{CommandOutput, Theme};

const TERMINAL_TAB_LIMIT: usize = 8;
const TERMINAL_TAB_NAME_LIMIT: usize = 128;

// Bound untrusted transport input using the existing field limits. JSON can
// encode an ASCII byte as six bytes (\u00XX); the remaining space covers keys,
// numeric fields, and formatting. Saving uses the typed schema's limits.
pub(crate) const DESKTOP_JSON_READ_LIMIT: usize = 6
    * (FilesPreferences::PATHS_LIMIT
        + FilesPreferences::MAX_FOLDER_ICONS * FilesPreferences::ICON_ID_LIMIT
        + TERMINAL_TAB_LIMIT
            * (crate::terminal::DIRECTORY_LIMIT
                + crate::terminal::SESSION_ID_LIMIT
                + TERMINAL_TAB_NAME_LIMIT)
        + crate::terminal::SESSION_ID_LIMIT
        + crate::browser::BOOKMARK_LIMIT
            * (crate::browser::BOOKMARK_TITLE_LIMIT + crate::browser::URL_LIMIT)
        + crate::desktop_customization::MAX_ICONS
            * crate::desktop_customization::DESKTOP_NAME_LIMIT
        + crate::desktop_customization::WALLPAPER_PATH_LIMIT)
    + 64 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applications {
    pub docker: bool,
    pub compose: bool,
    pub snap: bool,
    pub tmux: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopPreferences {
    pub version: u8,
    pub theme: Theme,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_interval_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Shortcuts::is_default")]
    pub shortcuts: Shortcuts,
    #[serde(default, skip_serializing_if = "TerminalLayout::is_empty")]
    pub terminal: TerminalLayout,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub windows: BTreeMap<DesktopApp, DesktopWindow>,
    #[serde(
        default,
        alias = "explorer",
        skip_serializing_if = "FilesPreferences::is_default"
    )]
    pub files: FilesPreferences,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub browser_bookmarks: Vec<crate::browser::Bookmark>,
    #[serde(
        default,
        skip_serializing_if = "crate::desktop_customization::Appearance::is_default"
    )]
    pub appearance: crate::desktop_customization::Appearance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopApp {
    Terminal,
    Monitor,
    #[serde(alias = "store")]
    AppCenter,
    // Retained so existing desktop and Dock preferences still deserialize.
    Services,
    Logs,
    #[serde(alias = "explorer")]
    Files,
    Disks,
    Docker,
    Editor,
    Code,
    #[serde(alias = "video")]
    Player,
    Trash,
    Browser,
    Settings,
}

impl DesktopApp {
    pub const COUNT: usize = 14;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<WindowBounds>,
    #[serde(default)]
    pub maximized: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<WindowWorkArea>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowWorkArea {
    pub width: u32,
    pub height: u32,
}

impl WindowBounds {
    pub fn validate(self) -> Result<()> {
        ensure!(
            (-32768..=32768).contains(&self.x)
                && (-32768..=32768).contains(&self.y)
                && (1..=32768).contains(&self.width)
                && (1..=32768).contains(&self.height)
                && self.reference.is_none_or(|area| {
                    (1..=32768).contains(&area.width) && (1..=32768).contains(&area.height)
                }),
            "Invalid desktop window geometry"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesPreferences {
    #[serde(default)]
    pub sidebar_hidden: bool,
    #[serde(default)]
    pub favorites: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, FolderColor>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub folder_icons: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

impl FilesPreferences {
    pub const MAX_FAVORITES: usize = 24;
    pub const MAX_COLORS: usize = 64;
    pub const MAX_FOLDER_ICONS: usize = 64;
    const PATHS_LIMIT: usize = 24 * 1024;
    const ICON_ID_LIMIT: usize = 48;
    pub fn marked_paths(&self) -> std::collections::BTreeSet<String> {
        self.favorites
            .iter()
            .chain(self.colors.keys())
            .chain(self.folder_icons.keys())
            .cloned()
            .collect()
    }

    pub fn remove_path_metadata(&mut self, paths: &std::collections::BTreeSet<String>) {
        self.favorites.retain(|path| !paths.contains(path));
        self.colors.retain(|path, _| !paths.contains(path));
        self.folder_icons.retain(|path, _| !paths.contains(path));
    }

    fn is_default(&self) -> bool {
        !self.sidebar_hidden
            && self.favorites.is_empty()
            && self.colors.is_empty()
            && self.folder_icons.is_empty()
    }
    pub fn validate_path(path: &str) -> Result<()> {
        ensure!(
            path.starts_with('/') && path.len() <= 1536 && !path.chars().any(char::is_control),
            "Invalid favorite folder path"
        );
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.favorites.len() <= Self::MAX_FAVORITES,
            "Too many favorite folders"
        );
        ensure!(
            self.colors.len() <= Self::MAX_COLORS,
            "Too many folder colors"
        );
        ensure!(
            self.folder_icons.len() <= Self::MAX_FOLDER_ICONS,
            "Too many custom folder icons"
        );
        ensure!(
            self.favorites
                .iter()
                .chain(self.colors.keys())
                .chain(self.folder_icons.keys())
                .map(String::len)
                .sum::<usize>()
                <= Self::PATHS_LIMIT,
            "Files preferences exceed the path limit"
        );
        let mut seen = HashSet::new();
        for path in &self.favorites {
            Self::validate_path(path)?;
            ensure!(seen.insert(path), "Duplicate favorite folder");
        }
        for path in self.colors.keys().chain(self.folder_icons.keys()) {
            Self::validate_path(path)?;
        }
        for icon in self.folder_icons.values() {
            // An optional style prefix and a semantic name identify a client-bundled icon.
            let valid_part = |part: &str| {
                !part.is_empty()
                    && part.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            };
            ensure!(
                icon.len() <= Self::ICON_ID_LIMIT
                    && icon.split_once(':').map_or_else(
                        || valid_part(icon),
                        |(style, name)| valid_part(style) && valid_part(name),
                    ),
                "Invalid folder icon identifier"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalLayout {
    pub tabs: Vec<TerminalTab>,
    pub active: String,
    pub maximized: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalTab {
    pub id: String,
    pub name: Option<String>,
    pub directory: Option<String>,
}
impl TerminalLayout {
    fn is_empty(&self) -> bool {
        self.tabs.is_empty() && !self.maximized
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.tabs.len() <= TERMINAL_TAB_LIMIT,
            "At most 8 terminal tabs can be saved"
        );
        let mut ids = std::collections::HashSet::new();
        for tab in &self.tabs {
            crate::terminal::validate_session(&tab.id)?;
            ensure!(ids.insert(tab.id.clone()), "Duplicate terminal session");
            if let Some(name) = &tab.name {
                ensure!(
                    name.len() <= TERMINAL_TAB_NAME_LIMIT && !name.chars().any(char::is_control),
                    "Invalid terminal tab name"
                );
            }
            if let Some(directory) = &tab.directory {
                crate::terminal::validate_directory(directory)?;
            }
        }
        ensure!(
            self.active.is_empty() || ids.contains(&self.active),
            "Invalid active terminal tab"
        );
        Ok(())
    }
}
impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            version: 1,
            theme: Theme::System,
            resource_interval_seconds: None,
            shortcuts: Shortcuts::default(),
            terminal: Default::default(),
            windows: Default::default(),
            files: Default::default(),
            browser_bookmarks: Vec::new(),
            appearance: Default::default(),
        }
    }
}

impl DesktopPreferences {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported desktop preferences version");
        ensure!(
            self.resource_interval_seconds
                .is_none_or(|seconds| crate::AppSettings::RESOURCE_INTERVALS.contains(&seconds)),
            "Invalid desktop resource refresh interval"
        );
        self.terminal.validate()?;
        self.shortcuts.validate()?;
        self.files.validate()?;
        crate::browser::validate_bookmarks(&self.browser_bookmarks)?;
        self.appearance.validate()?;
        for bounds in self.windows.values().filter_map(|window| window.bounds) {
            bounds.validate()?;
        }
        Ok(())
    }
}

pub(crate) fn execute(
    session: &RemoteSession,
    command: &str,
    limit: usize,
) -> Result<CommandOutput> {
    let (stdout, stderr, exit_code) = execute_bytes(session, command, limit)?;
    Ok(CommandOutput {
        stdout: String::from_utf8(stdout)?,
        stderr: lossy(stderr),
        exit_code,
    })
}

/// Like [`execute`], for output that is displayed rather than parsed: invalid
/// UTF-8 in either stream becomes U+FFFD instead of failing the request.
pub(crate) fn execute_lossy(
    session: &RemoteSession,
    command: &str,
    limit: usize,
) -> Result<CommandOutput> {
    let (stdout, stderr, exit_code) = execute_bytes(session, command, limit)?;
    Ok(CommandOutput {
        stdout: lossy(stdout),
        stderr: lossy(stderr),
        exit_code,
    })
}

/// Reuses the buffer when it is already valid UTF-8.
fn lossy(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

fn execute_bytes(
    session: &RemoteSession,
    command: &str,
    limit: usize,
) -> Result<(Vec<u8>, Vec<u8>, i32)> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let output = session.exec_stream_checked(
        command,
        None,
        Duration::from_secs(90),
        None,
        false,
        |stream, bytes| {
            ensure!(
                stdout.len() + stderr.len() + bytes.len() <= limit,
                "Server response exceeded the size limit"
            );
            match stream {
                CommandStream::Stdout => stdout.extend_from_slice(bytes),
                CommandStream::Stderr => stderr.extend_from_slice(bytes),
            }
            Ok(())
        },
    )?;
    Ok((stdout, stderr, output.exit_code))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockSample {
    pub unix_seconds: i64,
    pub offset_minutes: i16,
}

pub fn clock(session: &RemoteSession) -> Result<ClockSample> {
    let output = execute(
        session,
        "# arcnix:desktop:clock\nLC_ALL=C date '+%s %z'",
        256,
    )?;
    ensure!(output.exit_code == 0, "Cannot read the server clock");
    let mut fields = output.stdout.split_ascii_whitespace();
    let seconds = fields
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing server time"))?;
    let offset = fields
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing server time zone"))?;
    ensure!(fields.next().is_none(), "Invalid server clock response");
    let [
        sign @ (b'+' | b'-'),
        hours_tens @ b'0'..=b'9',
        hours_units @ b'0'..=b'9',
        minutes_tens @ b'0'..=b'9',
        minutes_units @ b'0'..=b'9',
    ] = *offset.as_bytes()
    else {
        anyhow::bail!("Invalid server UTC offset");
    };
    let hours = i16::from(hours_tens - b'0') * 10 + i16::from(hours_units - b'0');
    let minutes = i16::from(minutes_tens - b'0') * 10 + i16::from(minutes_units - b'0');
    ensure!(hours < 24 && minutes < 60, "Invalid server UTC offset");
    let multiplier = if sign == b'-' { -1 } else { 1 };
    Ok(ClockSample {
        unix_seconds: seconds.parse()?,
        offset_minutes: multiplier * (hours * 60 + minutes),
    })
}

pub fn discover_command() -> &'static str {
    "# arcnix:apps:discover\nprintf 'ARCNIX_APPS_V1\\n'; if command -v docker >/dev/null 2>&1; then printf 'docker\\n'; if command -v timeout >/dev/null 2>&1 && timeout -k 1s 5s docker compose version --short >/dev/null 2>&1; then printf 'compose\\n'; fi; fi; if command -v snap >/dev/null 2>&1; then printf 'snap\\n'; fi; if command -v tmux >/dev/null 2>&1; then printf 'tmux\\n'; fi"
}
pub fn discover(session: &RemoteSession) -> Result<Applications> {
    let output = execute(session, discover_command(), 4096)?;
    ensure!(
        output.exit_code == 0,
        "Application detection failed: {}",
        output.stderr.trim()
    );
    let mut lines = output.stdout.lines();
    ensure!(
        lines.next() == Some("ARCNIX_APPS_V1"),
        "Invalid application detection response"
    );
    let mut apps = Applications::default();
    for line in lines {
        let target = match line {
            "docker" => &mut apps.docker,
            "compose" => &mut apps.compose,
            "snap" => &mut apps.snap,
            "tmux" => &mut apps.tmux,
            _ => anyhow::bail!("Invalid application detection response"),
        };
        ensure!(!*target, "Duplicate application detection response");
        *target = true;
    }
    Ok(apps)
}

pub(crate) const DIRECTORY: &str = r#"set -eu
case "${HOME:-}" in /*) ;; *) printf 'Remote login home is unavailable\n' >&2; exit 1;; esac
arcnix_dir="$HOME/.arcnix"
if [ -L "$arcnix_dir" ]; then printf 'Refusing linked .arcnix directory\n' >&2; exit 1; fi
if [ -e "$arcnix_dir" ] && { [ ! -d "$arcnix_dir" ] || [ ! -O "$arcnix_dir" ]; }; then printf 'Invalid .arcnix directory owner or type\n' >&2; exit 1; fi
arcnix_file="$arcnix_dir/desktop.json"
if [ -L "$arcnix_file" ]; then printf 'Refusing linked desktop preferences\n' >&2; exit 1; fi
if [ -e "$arcnix_file" ] && { [ ! -f "$arcnix_file" ] || [ ! -O "$arcnix_file" ]; }; then printf 'Invalid desktop preferences owner or type\n' >&2; exit 1; fi
"#;

pub fn parse_preferences(text: &str) -> Result<DesktopPreferences> {
    let prefs: Option<DesktopPreferences> = serde_json::from_str(text)?;
    let mut prefs = prefs.unwrap_or_default();
    prefs.validate()?;
    use DesktopApp::{Code, Editor};

    use crate::desktop_customization::Shortcut;
    if let Some(window) = prefs.windows.remove(&Editor) {
        prefs.windows.entry(Code).or_insert(window);
    }
    fn migrate<T: PartialEq>(items: &mut Vec<T>, old: T, new: T) {
        if items.contains(&new) {
            items.retain(|item| *item != old);
        } else if let Some(item) = items.iter_mut().find(|item| **item == old) {
            *item = new;
        }
    }
    let appearance = &mut prefs.appearance;
    migrate(
        &mut appearance.order,
        Shortcut::App(Editor),
        Shortcut::App(Code),
    );
    for apps in [
        &mut appearance.hidden,
        &mut appearance.dock.order,
        &mut appearance.dock.unpinned,
    ] {
        migrate(apps, Editor, Code);
    }
    Ok(prefs)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loads_legacy_desktop_app_names_and_saves_canonical_names() {
        let prefs = parse_preferences(r#"{
            "version":1,"theme":"Dark",
            "terminal":{"tabs":[],"active":"","maximized":true},
            "windows":{
                "explorer":{"bounds":{"x":34,"y":52,"width":820,"height":480}},
                "store":{"maximized":true},
                "video":{"maximized":false}
            },
            "explorer":{
                "sidebar_hidden":true,"favorites":["/srv/网站"],"colors":{"/srv/网站":"purple"},
                "folder_icons":{"/srv/照片":"photos","/srv/cloud":"filled:cloud","/srv/calendar":"duotone:calendar"}
            },
            "appearance":{
                "order":[{"kind":"app","id":"store"}],
                "hidden":["video"],
                "dock":{"order":["explorer","store","video"],"unpinned":["video"]}
            }
        }"#).unwrap();
        assert!(!prefs.windows[&DesktopApp::Files].maximized);
        assert!(prefs.windows[&DesktopApp::AppCenter].maximized);
        assert!(prefs.windows.contains_key(&DesktopApp::Player));
        assert_eq!(prefs.files.colors["/srv/网站"], FolderColor::Purple);
        assert_eq!(prefs.files.folder_icons["/srv/照片"], "photos");
        assert_eq!(prefs.files.folder_icons["/srv/cloud"], "filled:cloud");
        assert_eq!(
            prefs.files.folder_icons["/srv/calendar"],
            "duotone:calendar"
        );
        assert!(prefs.files.sidebar_hidden);
        assert_eq!(prefs.files.favorites, ["/srv/网站"]);
        assert_eq!(
            prefs.appearance.dock.order,
            [DesktopApp::Files, DesktopApp::AppCenter, DesktopApp::Player]
        );
        let saved = serde_json::to_value(&prefs).unwrap();
        assert_eq!(saved["windows"]["files"]["bounds"]["width"], 820);
        assert_eq!(saved["windows"]["app_center"]["maximized"], true);
        assert!(saved["windows"].get("player").is_some());
        assert_eq!(saved["files"]["sidebar_hidden"], true);
        assert!(saved.get("explorer").is_none());
        assert_eq!(saved["appearance"]["order"][0]["id"], "app_center");
        assert_eq!(saved["appearance"]["hidden"][0], "player");
        assert_eq!(saved["appearance"]["dock"]["unpinned"][0], "player");
        assert_eq!(parse_preferences(&saved.to_string()).unwrap(), prefs);
        for field in [
            r#""windows":{"explorer":{"bounds":{"x":0,"y":0,"width":0,"height":900}}}"#,
            r#""explorer":{"favorites":["relative"]}"#,
            r#""explorer":{"colors":{"/srv":"chartreuse"}}"#,
            r#""files":{"folder_icons":{"relative":"photos"}}"#,
            r#""files":{"folder_icons":{"/srv":"filled:"}}"#,
            r#""files":{"folder_icons":{"/srv":"<svg>"}}"#,
        ] {
            assert!(
                parse_preferences(&format!(r#"{{"version":1,"theme":"Dark",{field}}}"#)).is_err()
            );
        }
    }
    #[test]
    fn legacy_editor_preferences_move_to_code_once() {
        use DesktopApp::{Code, Editor, Terminal};
        let mut old = DesktopPreferences::default();
        old.windows.insert(
            Editor,
            DesktopWindow {
                maximized: true,
                ..Default::default()
            },
        );
        old.appearance.dock.order = vec![Editor, Terminal];
        let migrated = parse_preferences(&serde_json::to_string(&old).unwrap()).unwrap();
        assert!(migrated.windows[&Code].maximized);
        assert!(!migrated.windows.contains_key(&Editor));
        assert_eq!(migrated.appearance.dock.order, [Code, Terminal]);
        old.appearance.dock.order.push(Code);
        old.windows.insert(Code, DesktopWindow::default());
        let mixed = parse_preferences(&serde_json::to_string(&old).unwrap()).unwrap();
        assert_eq!(mixed.appearance.dock.order, [Terminal, Code]);
        assert!(!mixed.windows[&Code].maximized);
    }

    #[test]
    fn preferences_require_a_known_schema_and_closed_theme() {
        assert_eq!(
            parse_preferences("null").unwrap(),
            DesktopPreferences::default()
        );
        assert_eq!(
            parse_preferences(r#"{"version":1,"theme":"Light"}"#)
                .unwrap()
                .theme,
            Theme::Light
        );
        for invalid in [
            r#"{"version":2,"theme":"Dark"}"#,
            r#"{"version":1,"theme":"$(echo bad)"}"#,
            "{}",
        ] {
            assert!(parse_preferences(invalid).is_err());
        }
        assert!(parse_preferences(&" ".repeat(4097)).is_err());
    }
}
