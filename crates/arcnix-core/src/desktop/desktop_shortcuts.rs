use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    NextApp,
    PreviousApp,
    Maximize,
    Minimize,
    Fullscreen,
    ExitFullscreen,
    ShowDesktop,
    CloseWindow,
    CloseTab,
    OpenFiles,
    FocusNext,
    FocusPrevious,
    Activate,
    ActivateSpace,
    TerminalNewTab,
    TerminalCloseTab,
    TerminalNextTab,
    TerminalPreviousTab,
    TerminalCopy,
    TerminalInterrupt,
    TerminalPaste,
    TerminalSelectAll,
    TerminalClear,
    TerminalContext,
    FilesDuplicateWindow,
    FilesEditPath,
    FilesHidden,
    FilesSelectAll,
    FilesCopy,
    FilesPaste,
    FilesSelect,
    FilesUp,
    FilesDown,
    FilesLeft,
    FilesRight,
    FilesFirst,
    FilesLast,
    CodeSave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Desktop,
    Terminal,
    Files,
    Code,
}

pub struct Binding {
    pub action: Action,
    pub label: &'static str,
    pub default: &'static str,
}

const fn key(action: Action, label: &'static str, default: &'static str) -> Binding {
    Binding {
        action,
        label,
        default,
    }
}

pub const GROUPS: &[(Scope, &[Binding])] = &[
    (
        Scope::Desktop,
        &[
            key(Action::NextApp, "Switch to next app", "Ctrl+Tab"),
            key(
                Action::PreviousApp,
                "Switch to previous app",
                "Ctrl+Shift+Tab",
            ),
            key(Action::Maximize, "Toggle maximize", "Ctrl+Shift+ArrowUp"),
            key(Action::Minimize, "Minimize window", "Ctrl+Shift+ArrowDown"),
            key(
                Action::Fullscreen,
                "Toggle desktop fullscreen",
                "Ctrl+Shift+F",
            ),
            key(Action::ExitFullscreen, "Exit desktop fullscreen", "Escape"),
            key(Action::ShowDesktop, "Show desktop", "Meta+D"),
            key(Action::CloseWindow, "Close application", "Alt+F4"),
            key(Action::CloseTab, "Close active tab", "Ctrl+F4"),
            key(Action::OpenFiles, "Open a Files window", "Ctrl+E"),
            key(Action::FocusNext, "Move focus forward", "Tab"),
            key(Action::FocusPrevious, "Move focus backward", "Shift+Tab"),
            key(Action::Activate, "Activate focused control", "Enter"),
            key(
                Action::ActivateSpace,
                "Activate focused control with Space",
                "Space",
            ),
        ],
    ),
    (
        Scope::Terminal,
        &[
            key(Action::TerminalNewTab, "Open tab", "Ctrl+Shift+T"),
            key(Action::TerminalCloseTab, "Close tab", "Ctrl+Shift+W"),
            key(Action::TerminalNextTab, "Next tab", "Ctrl+Tab"),
            key(
                Action::TerminalPreviousTab,
                "Previous tab",
                "Ctrl+Shift+Tab",
            ),
            key(Action::TerminalCopy, "Copy", "Ctrl+Shift+C"),
            key(
                Action::TerminalInterrupt,
                "Copy selection / interrupt",
                "Ctrl+C",
            ),
            key(Action::TerminalPaste, "Paste", "Ctrl+Shift+V"),
            key(Action::TerminalSelectAll, "Select all", "Ctrl+Shift+A"),
            key(Action::TerminalClear, "Clear display", "Ctrl+Shift+L"),
            key(Action::TerminalContext, "Context menu", "Shift+F10"),
        ],
    ),
    (
        Scope::Files,
        &[
            key(
                Action::FilesDuplicateWindow,
                "Duplicate current Files window",
                "Ctrl+N",
            ),
            key(Action::FilesEditPath, "Edit path", "Ctrl+L"),
            key(Action::FilesHidden, "Toggle hidden files", "Ctrl+H"),
            key(Action::FilesCopy, "Copy files", "Ctrl+C"),
            key(Action::FilesPaste, "Paste files", "Ctrl+V"),
            key(Action::FilesSelectAll, "Select all", "Ctrl+A"),
            key(Action::FilesSelect, "Select focused entry", "Space"),
            key(Action::FilesUp, "Move up", "ArrowUp"),
            key(Action::FilesDown, "Move down", "ArrowDown"),
            key(
                Action::FilesLeft,
                "Move left / collapse folder",
                "ArrowLeft",
            ),
            key(
                Action::FilesRight,
                "Move right / expand folder",
                "ArrowRight",
            ),
            key(Action::FilesFirst, "Select first entry", "Home"),
            key(Action::FilesLast, "Select last entry", "End"),
        ],
    ),
    (Scope::Code, &[key(Action::CodeSave, "Save", "Ctrl+S")]),
];

impl Action {
    pub fn scope(self) -> Scope {
        self.definition().0
    }

    fn definition(self) -> (Scope, &'static Binding) {
        GROUPS
            .iter()
            .find_map(|(scope, bindings)| {
                bindings
                    .iter()
                    .find(|binding| binding.action == self)
                    .map(|binding| (*scope, binding))
            })
            .expect("every shortcut action has a definition")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    #[default]
    Auto,
    Windows,
    MacOs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    Windows,
    MacOs,
}

impl Scheme {
    pub fn preset(self) -> Preset {
        match self {
            Self::MacOs => Preset::MacOs,
            Self::Auto if cfg!(target_os = "macos") => Preset::MacOs,
            Self::Windows | Self::Auto => Preset::Windows,
        }
    }
}

impl Preset {
    pub fn default_key(self, action: Action) -> &'static str {
        if self == Self::MacOs {
            match action {
                Action::Minimize => return "Meta+M",
                Action::Maximize => return "Ctrl+Meta+ArrowUp",
                Action::Fullscreen => return "Ctrl+Meta+F",
                Action::ShowDesktop => return "Ctrl+Shift+D",
                Action::CloseTab => return "Meta+W",
                Action::CloseWindow => return "Shift+Meta+W",
                Action::OpenFiles => return "Shift+Meta+E",
                Action::TerminalNewTab => return "Meta+T",
                Action::TerminalCloseTab => return "Meta+W",
                Action::TerminalCopy | Action::FilesCopy => return "Meta+C",
                Action::TerminalPaste | Action::FilesPaste => return "Meta+V",
                Action::TerminalSelectAll | Action::FilesSelectAll => return "Meta+A",
                Action::TerminalClear => return "Meta+K",
                Action::FilesDuplicateWindow => return "Meta+N",
                Action::FilesEditPath => return "Shift+Meta+G",
                Action::FilesHidden => return "Shift+Meta+Period",
                Action::CodeSave => return "Meta+S",
                _ => {}
            }
        }
        action.definition().1.default
    }

    pub fn display(self, chord: &str) -> String {
        if self == Self::MacOs {
            chord
                .replace("Meta+", "Cmd+")
                .replace("Alt+", "Option+")
                .replace("Period", ".")
        } else {
            chord.replace("Meta+", "Win+")
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    overrides: BTreeMap<Action, Option<String>>,
    additions: BTreeMap<Action, Vec<String>>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    disabled: BTreeSet<Action>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Profiles {
    windows: Profile,
    macos: Profile,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Shortcuts(Profiles);

impl<'de> Deserialize<'de> for Shortcuts {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Profiles(Profiles),
            Legacy(BTreeMap<Action, Option<String>>),
        }
        Ok(Self(match Stored::deserialize(deserializer)? {
            Stored::Profiles(profiles) => profiles,
            Stored::Legacy(overrides) => {
                let profile = Profile {
                    overrides,
                    ..Default::default()
                };
                Profiles {
                    windows: profile.clone(),
                    macos: profile,
                }
            }
        }))
    }
}

impl Shortcuts {
    pub fn resolved(&self, preset: Preset) -> Keymap<'_> {
        let profile = match preset {
            Preset::Windows => &self.0.windows,
            Preset::MacOs => &self.0.macos,
        };
        Keymap { preset, profile }
    }

    pub fn profile_mut(&mut self, preset: Preset) -> &mut Profile {
        match preset {
            Preset::Windows => &mut self.0.windows,
            Preset::MacOs => &mut self.0.macos,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for preset in [Preset::Windows, Preset::MacOs] {
            self.resolved(preset).validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct Keymap<'a> {
    pub preset: Preset,
    profile: &'a Profile,
}

impl<'a> Keymap<'a> {
    pub fn overridden(self, action: Action) -> bool {
        self.profile.overrides.contains_key(&action)
    }

    pub fn modified(self, action: Action) -> bool {
        self.overridden(action)
            || !self.additions(action).is_empty()
            || self.profile.disabled.contains(&action)
    }

    pub fn enabled(self, action: Action) -> bool {
        self.chords(action).next().is_some()
    }

    pub fn binding(self, action: Action) -> Option<&'a str> {
        if let Some(binding) = self.profile.overrides.get(&action) {
            return binding.as_deref();
        }
        let default = self.preset.default_key(action);
        // Existing custom shortcuts take precedence over defaults added by a newer preset.
        let in_scope = |other: Action| other.scope() == action.scope();
        let shadowed = self
            .profile
            .overrides
            .iter()
            .any(|(&other, key)| in_scope(other) && key.as_deref() == Some(default))
            || self
                .profile
                .additions
                .iter()
                .any(|(&other, keys)| in_scope(other) && keys.iter().any(|key| key == default));
        (!shadowed).then_some(default)
    }

    pub fn additions(self, action: Action) -> &'a [String] {
        self.profile
            .additions
            .get(&action)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn configured_chords(self, action: Action) -> impl Iterator<Item = &'a str> {
        self.binding(action)
            .into_iter()
            .chain(self.additions(action).iter().map(String::as_str))
    }

    pub fn chords(self, action: Action) -> impl Iterator<Item = &'a str> {
        self.configured_chords(action)
            .filter(move |_| !self.profile.disabled.contains(&action))
    }

    pub fn action(self, scope: Scope, chord: &str) -> Option<Action> {
        self.action_matching(scope, |action| self.chords(action).any(|key| key == chord))
    }

    fn configured_action(self, scope: Scope, chord: &str) -> Option<Action> {
        self.action_matching(scope, |action| {
            self.configured_chords(action).any(|key| key == chord)
        })
    }

    fn action_matching(
        self,
        scope: Scope,
        mut matches: impl FnMut(Action) -> bool,
    ) -> Option<Action> {
        GROUPS
            .iter()
            .find(|(group, _)| *group == scope)?
            .1
            .iter()
            .map(|binding| binding.action)
            .find(|&action| matches(action))
    }

    fn validate(self) -> Result<()> {
        for (_, definitions) in GROUPS {
            let mut seen = HashSet::new();
            for definition in *definitions {
                for chord in self.configured_chords(definition.action) {
                    validate_chord(self.preset, definition.action, chord)?;
                    ensure!(seen.insert(chord), "This shortcut is already in use.");
                }
            }
        }
        Ok(())
    }
}

impl Profile {
    pub fn reset(&mut self, preset: Preset, action: Action) -> Result<()> {
        let keys = Keymap {
            preset,
            profile: self,
        };
        let default = preset.default_key(action);
        ensure!(
            keys.configured_action(action.scope(), default)
                .is_none_or(|owner| owner == action),
            "This shortcut is already in use."
        );
        self.overrides.remove(&action);
        self.additions.remove(&action);
        self.disabled.remove(&action);
        Ok(())
    }

    pub fn reset_scope(&mut self, scope: Scope) {
        self.overrides.retain(|action, _| action.scope() != scope);
        self.additions.retain(|action, _| action.scope() != scope);
        self.disabled.retain(|action| action.scope() != scope);
    }

    pub fn set_enabled(&mut self, preset: Preset, action: Action, enabled: bool) -> Result<()> {
        if enabled {
            let keys = Keymap {
                preset,
                profile: self,
            };
            if keys.configured_chords(action).next().is_none() {
                self.set(preset, action, Some(preset.default_key(action).into()))?;
            }
            self.disabled.remove(&action);
        } else {
            self.disabled.insert(action);
        }
        Ok(())
    }

    pub fn set(&mut self, preset: Preset, action: Action, chord: Option<String>) -> Result<()> {
        if let Some(chord) = chord.as_deref() {
            let keys = Keymap {
                preset,
                profile: self,
            };
            validate_chord(preset, action, chord)?;
            ensure!(
                keys.binding(action) == Some(chord)
                    || keys.configured_action(action.scope(), chord).is_none(),
                "This shortcut is already in use."
            );
        }
        if chord.as_deref() == Some(preset.default_key(action)) {
            self.overrides.remove(&action);
        } else {
            self.overrides.insert(action, chord);
        }
        Ok(())
    }

    pub fn add(&mut self, preset: Preset, action: Action, chord: String) -> Result<()> {
        validate_chord(preset, action, &chord)?;
        let keys = Keymap {
            preset,
            profile: self,
        };
        ensure!(
            keys.configured_action(action.scope(), &chord).is_none(),
            "This shortcut is already in use."
        );
        self.additions.entry(action).or_default().push(chord);
        Ok(())
    }

    pub fn replace_extra(
        &mut self,
        preset: Preset,
        action: Action,
        previous: &str,
        chord: String,
    ) -> Result<()> {
        let keys = Keymap {
            preset,
            profile: self,
        };
        let index = keys
            .additions(action)
            .iter()
            .position(|key| key == previous)
            .ok_or_else(|| anyhow::anyhow!("This shortcut has changed."))?;
        if previous != chord {
            validate_chord(preset, action, &chord)?;
            ensure!(
                keys.configured_action(action.scope(), &chord).is_none(),
                "This shortcut is already in use."
            );
            self.additions.get_mut(&action).unwrap()[index] = chord;
        }
        Ok(())
    }

    pub fn remove_extra(&mut self, action: Action, chord: &str) {
        if let Some(keys) = self.additions.get_mut(&action) {
            keys.retain(|key| key != chord);
            if keys.is_empty() {
                self.additions.remove(&action);
            }
        }
    }
}

fn validate_chord(preset: Preset, action: Action, chord: &str) -> Result<()> {
    if chord == preset.default_key(action) || chord == action.definition().1.default {
        return Ok(());
    }
    let (modifiers, key) = chord.rsplit_once('+').unwrap_or(("", chord));
    let mut previous = 0;
    let mut primary = false;
    for modifier in modifiers.split('+').filter(|_| !modifiers.is_empty()) {
        let order = match modifier {
            "Ctrl" => 1,
            "Alt" => 2,
            "Shift" => 3,
            "Meta" => 4,
            _ => 0,
        };
        ensure!(order > previous, "Unsupported shortcut.");
        previous = order;
        primary |= modifier != "Shift";
    }
    let function = key
        .strip_prefix('F')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=12).contains(&number) && key == format!("F{number}"));
    let navigation = matches!(
        key,
        "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "PageUp"
            | "PageDown"
            | "Home"
            | "End"
    );
    let character = matches!(key.as_bytes(), [b'A'..=b'Z' | b'0'..=b'9']);
    let modified_control = matches!(key, "Space" | "Enter" | "Escape" | "Delete");
    ensure!(
        (function && chord != "F12")
            || primary && (navigation || character || modified_control || key == "Period")
            || key == "Tab" && matches!(modifiers, "Ctrl" | "Ctrl+Shift"),
        "Use Ctrl, Alt or Meta with a supported key, or use a function key."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const WIN: Preset = Preset::Windows;

    #[test]
    fn defaults_overrides_and_disabling_round_trip() -> Result<()> {
        let mut bindings = Shortcuts::default();
        bindings.validate()?;
        assert_eq!(
            bindings.resolved(WIN).action(Scope::Desktop, "Ctrl+Tab"),
            Some(Action::NextApp)
        );
        assert_eq!(
            bindings
                .resolved(WIN)
                .action(Scope::Desktop, "Ctrl+Shift+Tab"),
            Some(Action::PreviousApp)
        );
        assert!(
            bindings
                .profile_mut(WIN)
                .set(WIN, Action::NextApp, Some("Alt+Tab".into()))
                .is_err()
        );
        assert!(
            bindings
                .profile_mut(WIN)
                .set(WIN, Action::NextApp, Some("Ctrl+Shift+F".into()))
                .is_err()
        );
        assert!(
            bindings
                .profile_mut(WIN)
                .set(WIN, Action::NextApp, Some("Shift+B".into()))
                .is_err()
        );
        bindings
            .profile_mut(WIN)
            .set(WIN, Action::Fullscreen, None)?;
        bindings
            .profile_mut(WIN)
            .set(WIN, Action::NextApp, Some("Ctrl+Shift+F".into()))?;
        let decoded: Shortcuts = serde_json::from_str(&serde_json::to_string(&bindings)?)?;
        decoded.validate()?;
        assert_eq!(decoded, bindings);
        assert_eq!(
            decoded.resolved(WIN).action(Scope::Desktop, "Ctrl+Shift+F"),
            Some(Action::NextApp)
        );
        assert_eq!(decoded.resolved(WIN).binding(Action::Fullscreen), None);
        bindings
            .profile_mut(WIN)
            .set(WIN, Action::NextApp, Some("Ctrl+Tab".into()))?;
        bindings
            .profile_mut(WIN)
            .set(WIN, Action::Fullscreen, Some("Ctrl+Shift+F".into()))?;
        assert_eq!(bindings, Shortcuts::default());
        Ok(())
    }

    #[test]
    fn conflicts_and_restores_follow_the_shortcut_scope() -> Result<()> {
        let mut bindings = Shortcuts::default();
        bindings
            .profile_mut(WIN)
            .set(WIN, Action::TerminalNewTab, Some("Ctrl+E".into()))?;
        assert_eq!(
            bindings.resolved(WIN).action(Scope::Desktop, "Ctrl+E"),
            Some(Action::OpenFiles)
        );
        assert_eq!(
            bindings.resolved(WIN).action(Scope::Terminal, "Ctrl+E"),
            Some(Action::TerminalNewTab)
        );
        bindings
            .profile_mut(WIN)
            .set(WIN, Action::TerminalNextTab, Some("Ctrl+Shift+T".into()))?;
        assert!(
            bindings
                .profile_mut(WIN)
                .set(
                    WIN,
                    Action::TerminalNewTab,
                    Some(WIN.default_key(Action::TerminalNewTab).into())
                )
                .is_err()
        );
        assert_eq!(
            bindings.resolved(WIN).binding(Action::TerminalNewTab),
            Some("Ctrl+E")
        );
        bindings.profile_mut(WIN).set(
            WIN,
            Action::TerminalNextTab,
            Some(WIN.default_key(Action::TerminalNextTab).into()),
        )?;
        bindings.profile_mut(WIN).set(
            WIN,
            Action::TerminalNewTab,
            Some(WIN.default_key(Action::TerminalNewTab).into()),
        )?;
        assert_eq!(bindings, Shortcuts::default());
        Ok(())
    }

    #[test]
    fn disabled_actions_keep_their_shortcuts_and_reset_individually() -> Result<()> {
        let action = Action::Fullscreen;
        let mut shortcuts = Shortcuts::default();
        let profile = shortcuts.profile_mut(WIN);
        profile.set(WIN, action, Some("F7".into()))?;
        profile.add(WIN, action, "F8".into())?;
        profile.set_enabled(WIN, action, false)?;
        assert!(
            profile
                .set(WIN, Action::Minimize, Some("F7".into()))
                .is_err()
        );
        profile.set(WIN, Action::Minimize, Some("F9".into()))?;

        let mut loaded: Shortcuts = serde_json::from_str(&serde_json::to_string(&shortcuts)?)?;
        loaded.validate()?;
        let keys = loaded.resolved(WIN);
        assert!(keys.modified(action));
        assert!(!keys.enabled(action));
        assert_eq!(keys.binding(action), Some("F7"));
        assert_eq!(keys.additions(action), &["F8"]);
        assert_eq!(keys.action(Scope::Desktop, "F7"), None);
        assert_eq!(keys.action(Scope::Desktop, "F8"), None);
        assert!(loaded.resolved(Preset::MacOs).enabled(action));

        loaded.profile_mut(WIN).set_enabled(WIN, action, true)?;
        assert_eq!(
            loaded.resolved(WIN).chords(action).collect::<Vec<_>>(),
            ["F7", "F8"]
        );
        loaded.profile_mut(WIN).set_enabled(WIN, action, false)?;
        loaded.profile_mut(WIN).reset(WIN, action)?;
        let keys = loaded.resolved(WIN);
        assert!(!keys.modified(action));
        assert_eq!(keys.chords(action).collect::<Vec<_>>(), ["Ctrl+Shift+F"]);
        assert_eq!(keys.binding(Action::Minimize), Some("F9"));
        Ok(())
    }

    #[test]
    fn reenabling_an_unassigned_shortcut_checks_for_default_key_conflicts() -> Result<()> {
        let mut shortcuts: Shortcuts =
            serde_json::from_str(r#"{"fullscreen":null,"minimize":"Ctrl+Shift+F"}"#)?;
        let action = Action::Fullscreen;
        assert!(!shortcuts.resolved(WIN).enabled(action));
        let profile = shortcuts.profile_mut(WIN);
        assert!(profile.set_enabled(WIN, action, true).is_err());
        assert!(profile.reset(WIN, action).is_err());
        profile.reset(WIN, Action::Minimize)?;
        profile.set_enabled(WIN, action, true)?;
        assert!(shortcuts.resolved(WIN).enabled(action));
        assert!(!shortcuts.resolved(WIN).modified(action));
        Ok(())
    }

    #[test]
    fn an_existing_desktop_override_can_shadow_the_new_open_files_default() -> Result<()> {
        let keys: Shortcuts = serde_json::from_str(r#"{"minimize":"Ctrl+E"}"#)?;
        keys.validate()?;
        assert_eq!(keys.resolved(WIN).binding(Action::Minimize), Some("Ctrl+E"));
        assert_eq!(keys.resolved(WIN).binding(Action::OpenFiles), None);
        Ok(())
    }

    #[test]
    fn macos_show_desktop_uses_a_modified_key_and_respects_existing_shortcuts() -> Result<()> {
        let mac = Preset::MacOs;
        let action = Action::ShowDesktop;
        let defaults = Shortcuts::default();
        assert_eq!(defaults.resolved(mac).binding(action), Some("Ctrl+Shift+D"));
        assert_eq!(defaults.resolved(WIN).binding(action), Some("Meta+D"));

        let mut saved: Shortcuts =
            serde_json::from_str(r#"{"macos":{"additions":{"minimize":["Ctrl+Shift+D"]}}}"#)?;
        saved.validate()?;
        let keys = saved.resolved(mac);
        assert_eq!(keys.binding(action), None);
        assert_eq!(
            keys.action(Scope::Desktop, "Ctrl+Shift+D"),
            Some(Action::Minimize)
        );
        saved.profile_mut(mac).reset(mac, Action::Minimize)?;
        assert_eq!(saved.resolved(mac).binding(action), Some("Ctrl+Shift+D"));
        Ok(())
    }

    #[test]
    fn legacy_edits_seed_both_profiles_and_subsequent_changes_are_independent() -> Result<()> {
        let mut shortcuts: Shortcuts =
            serde_json::from_str(r#"{"fullscreen":"F7","minimize":null}"#)?;
        let mac = Preset::MacOs;
        for preset in [WIN, mac] {
            let keys = shortcuts.resolved(preset);
            assert_eq!(keys.binding(Action::Fullscreen), Some("F7"));
            assert_eq!(keys.binding(Action::Minimize), None);
        }
        let profile = shortcuts.profile_mut(mac);
        profile.add(mac, Action::Fullscreen, "F8".into())?;
        assert!(profile.add(mac, Action::Minimize, "F8".into()).is_err());
        assert!(
            profile
                .replace_extra(mac, Action::Fullscreen, "F8", "F7".into())
                .is_err()
        );
        profile.replace_extra(mac, Action::Fullscreen, "F8", "F9".into())?;
        let saved = serde_json::to_string(&shortcuts)?;
        assert!(!saved.contains("Meta+S") && !saved.contains("Ctrl+S"));
        let mut loaded: Shortcuts = serde_json::from_str(&saved)?;
        loaded.validate()?;
        assert_eq!(
            loaded.resolved(mac).action(Scope::Desktop, "F9"),
            Some(Action::Fullscreen)
        );
        assert_eq!(loaded.resolved(WIN).action(Scope::Desktop, "F9"), None);
        loaded.profile_mut(mac).reset_scope(Scope::Desktop);
        assert_eq!(
            loaded.resolved(mac).binding(Action::Fullscreen),
            Some("Ctrl+Meta+F")
        );
        assert_eq!(loaded.resolved(WIN).binding(Action::Fullscreen), Some("F7"));
        Ok(())
    }

    #[test]
    fn independent_clients_can_reset_one_profile_while_adding_to_the_other() -> Result<()> {
        use crate::server_apps::DesktopPreferences;
        use crate::sync::merge_desktop;
        let mut base = DesktopPreferences::default();
        base.shortcuts
            .profile_mut(WIN)
            .add(WIN, Action::OpenFiles, "F7".into())?;
        let mut local = base.clone();
        local.shortcuts.profile_mut(WIN).reset_scope(Scope::Desktop);
        let mut remote = base.clone();
        let mac = Preset::MacOs;
        remote
            .shortcuts
            .profile_mut(mac)
            .add(mac, Action::OpenFiles, "F8".into())?;
        let (merged, conflicts) = merge_desktop(&base, &local, &remote, false)?;
        assert!(conflicts.is_empty());
        assert!(
            merged
                .shortcuts
                .resolved(WIN)
                .additions(Action::OpenFiles)
                .is_empty()
        );
        assert_eq!(
            merged.shortcuts.resolved(mac).additions(Action::OpenFiles),
            ["F8"]
        );
        Ok(())
    }
}
