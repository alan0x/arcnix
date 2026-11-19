//! Finite software discovery and remote-user desktop preferences over SSH.
use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use anyhow::{Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::desktop_shortcuts::Shortcuts;
use crate::remote::{CommandStream, RemoteSession};
use crate::{CommandOutput, Theme};

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
    #[serde(default, skip_serializing_if = "ExplorerPreferences::is_default")]
    pub explorer: ExplorerPreferences,
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
    Store,
    // Retained so existing desktop and Dock preferences still deserialize.
    Services,
    Logs,
    Explorer,
    Disks,
    Docker,
    Editor,
    Code,
    Video,
    Browser,
    Settings,
}

impl DesktopApp {
    pub const COUNT: usize = 13;
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
    pub x: u32,
    pub y: u32,
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
            self.x <= 32768
                && self.y <= 32768
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
pub struct ExplorerPreferences {
    #[serde(default)]
    pub sidebar_hidden: bool,
    #[serde(default)]
    pub favorites: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, FolderColor>,
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

impl ExplorerPreferences {
    pub const MAX_FAVORITES: usize = 24;
    pub const MAX_COLORS: usize = 64;
    fn is_default(&self) -> bool {
        !self.sidebar_hidden && self.favorites.is_empty() && self.colors.is_empty()
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
            self.favorites
                .iter()
                .chain(self.colors.keys())
                .map(String::len)
                .sum::<usize>()
                <= 24 * 1024,
            "Explorer preferences exceed the path limit"
        );
        let mut seen = HashSet::new();
        for path in &self.favorites {
            Self::validate_path(path)?;
            ensure!(seen.insert(path), "Duplicate favorite folder");
        }
        for path in self.colors.keys() {
            Self::validate_path(path)?;
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
        ensure!(self.tabs.len() <= 8, "At most 8 terminal tabs can be saved");
        let mut ids = std::collections::HashSet::new();
        for tab in &self.tabs {
            crate::terminal::validate_session(&tab.id)?;
            ensure!(ids.insert(tab.id.clone()), "Duplicate terminal session");
            if let Some(name) = &tab.name {
                ensure!(
                    name.len() <= 128 && !name.chars().any(char::is_control),
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
            explorer: Default::default(),
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
        self.explorer.validate()?;
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
    Ok(CommandOutput {
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        exit_code: output.exit_code,
    })
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

pub fn preferences_read_command() -> String {
    format!(
        "# arcnix:desktop:read\n{DIRECTORY}\nif [ -f \"$arcnix_file\" ]; then head -c 65537 -- \"$arcnix_file\"; else printf 'null\\n'; fi"
    )
}
pub fn parse_preferences(text: &str) -> Result<DesktopPreferences> {
    ensure!(text.len() <= 65536, "Desktop preferences exceed 64 KiB");
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
pub fn read_preferences(session: &RemoteSession) -> Result<DesktopPreferences> {
    let output = execute(session, &preferences_read_command(), 69632)?;
    ensure!(
        output.exit_code == 0,
        "Cannot read ~/.arcnix/desktop.json: {}",
        output.stderr.trim()
    );
    parse_preferences(&output.stdout)
}
pub fn preferences_write_command(prefs: &DesktopPreferences) -> Result<String> {
    prefs.validate()?;
    let json = serde_json::to_string(prefs)?;
    ensure!(json.len() <= 65536, "Desktop preferences exceed 64 KiB");
    let encoded = STANDARD.encode(json);
    Ok(format!(
        r#"# arcnix:desktop:write
{DIRECTORY}
umask 077
if [ ! -d "$arcnix_dir" ]; then mkdir -m 700 -- "$arcnix_dir"; fi
arcnix_temp=$(mktemp "$arcnix_dir/.desktop.XXXXXXXX")
trap 'rm -f -- "$arcnix_temp"' EXIT HUP INT TERM
printf '%s' '{encoded}' | base64 -d > "$arcnix_temp"
chmod 600 -- "$arcnix_temp"
mv -fT -- "$arcnix_temp" "$arcnix_file"
printf 'ARCNIX_DESKTOP_SAVED_V1\n'
"#
    ))
}
pub fn write_preferences(session: &RemoteSession, prefs: &DesktopPreferences) -> Result<()> {
    let output = execute(session, &preferences_write_command(prefs)?, 8192)?;
    ensure!(
        output.exit_code == 0 && output.stdout.trim() == "ARCNIX_DESKTOP_SAVED_V1",
        "Cannot save ~/.arcnix/desktop.json: {}",
        output.stderr.trim()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restores_independent_windows_and_explorer_settings_from_remote_json() {
        let prefs = parse_preferences(r#"{
            "version":1,"theme":"Dark",
            "terminal":{"tabs":[],"active":"","maximized":true},
            "windows":{"explorer":{"bounds":{"x":34,"y":52,"width":820,"height":480}}},
            "explorer":{"sidebar_hidden":true,"favorites":["/srv/网站"],"colors":{"/srv/网站":"purple"}}
        }"#).unwrap();
        assert!(!prefs.windows[&DesktopApp::Explorer].maximized);
        assert_eq!(prefs.explorer.colors["/srv/网站"], FolderColor::Purple);
        assert_eq!(
            parse_preferences(&serde_json::to_string(&prefs).unwrap()).unwrap(),
            prefs
        );
        for field in [
            r#""windows":{"explorer":{"bounds":{"x":0,"y":0,"width":0,"height":900}}}"#,
            r#""explorer":{"favorites":["relative"]}"#,
            r#""explorer":{"colors":{"/srv":"chartreuse"}}"#,
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
        let write = preferences_write_command(&DesktopPreferences {
            version: 1,
            theme: Theme::Dark,
            ..Default::default()
        })
        .unwrap();
        assert!(
            write.contains("mktemp") && write.contains("mv -fT") && write.contains("chmod 600")
        );
    }
}
