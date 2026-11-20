use std::collections::{BTreeMap, HashSet};

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

/// Missing entries use defaults. A null entry disables the action.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Shortcuts(BTreeMap<Action, Option<String>>);

impl Shortcuts {
    pub fn is_default(&self) -> bool {
        self.0.is_empty()
    }

    pub fn overridden(&self, action: Action) -> bool {
        self.0.contains_key(&action)
    }

    pub fn binding(&self, action: Action) -> Option<&str> {
        if let Some(binding) = self.0.get(&action) {
            return binding.as_deref();
        }
        // Ctrl+E could be reassigned before Open Files was configurable.
        if action == Action::OpenFiles
            && [
                Action::NextApp,
                Action::PreviousApp,
                Action::Maximize,
                Action::Minimize,
                Action::Fullscreen,
            ]
            .iter()
            .any(|action| {
                self.0
                    .get(action)
                    .is_some_and(|value| value.as_deref() == Some("Ctrl+E"))
            })
        {
            return None;
        }
        Some(action.definition().1.default)
    }

    pub fn action(&self, scope: Scope, chord: &str) -> Option<Action> {
        let (_, definitions) = GROUPS.iter().find(|(group, _)| *group == scope)?;
        definitions
            .iter()
            .map(|binding| binding.action)
            .find(|&action| self.binding(action) == Some(chord))
    }

    pub fn restore(&mut self, action: Action) -> Result<()> {
        self.set(action, Some(action.definition().1.default.to_owned()))
    }

    pub fn set(&mut self, action: Action, chord: Option<String>) -> Result<()> {
        let (scope, definition) = action.definition();
        if let Some(chord) = chord.as_deref() {
            validate_chord(action, chord)?;
            ensure!(
                self.action(scope, chord)
                    .is_none_or(|existing| existing == action),
                "This shortcut is already in use."
            );
        }
        if chord.as_deref() == Some(definition.default) {
            self.0.remove(&action);
        } else {
            self.0.insert(action, chord);
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for (_, definitions) in GROUPS {
            let mut seen = HashSet::new();
            for definition in *definitions {
                if let Some(chord) = self.binding(definition.action) {
                    validate_chord(definition.action, chord)?;
                    ensure!(seen.insert(chord), "This shortcut is already in use.");
                }
            }
        }
        Ok(())
    }
}

fn validate_chord(action: Action, chord: &str) -> Result<()> {
    let (_, definition) = action.definition();
    if chord == definition.default {
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
            || primary && (navigation || character || modified_control)
            || key == "Tab" && matches!(modifiers, "Ctrl" | "Ctrl+Shift"),
        "Use Ctrl, Alt or Meta with a supported key, or use a function key."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_overrides_and_disabling_round_trip() -> Result<()> {
        let mut bindings = Shortcuts::default();
        bindings.validate()?;
        assert_eq!(
            bindings.action(Scope::Desktop, "Ctrl+Tab"),
            Some(Action::NextApp)
        );
        assert_eq!(
            bindings.action(Scope::Desktop, "Ctrl+Shift+Tab"),
            Some(Action::PreviousApp)
        );
        assert!(
            bindings
                .set(Action::NextApp, Some("Alt+Tab".into()))
                .is_err()
        );
        assert!(
            bindings
                .set(Action::NextApp, Some("Ctrl+Shift+F".into()))
                .is_err()
        );
        assert!(
            bindings
                .set(Action::NextApp, Some("Shift+B".into()))
                .is_err()
        );
        bindings.set(Action::Fullscreen, None)?;
        bindings.set(Action::NextApp, Some("Ctrl+Shift+F".into()))?;
        let decoded: Shortcuts = serde_json::from_str(&serde_json::to_string(&bindings)?)?;
        decoded.validate()?;
        assert_eq!(decoded, bindings);
        assert_eq!(
            decoded.action(Scope::Desktop, "Ctrl+Shift+F"),
            Some(Action::NextApp)
        );
        assert_eq!(decoded.binding(Action::Fullscreen), None);
        bindings.set(Action::NextApp, Some("Ctrl+Tab".into()))?;
        bindings.set(Action::Fullscreen, Some("Ctrl+Shift+F".into()))?;
        assert!(bindings.is_default());
        Ok(())
    }

    #[test]
    fn conflicts_and_restores_follow_the_shortcut_scope() -> Result<()> {
        let mut bindings = Shortcuts::default();
        bindings.set(Action::TerminalNewTab, Some("Ctrl+E".into()))?;
        assert_eq!(
            bindings.action(Scope::Desktop, "Ctrl+E"),
            Some(Action::OpenFiles)
        );
        assert_eq!(
            bindings.action(Scope::Terminal, "Ctrl+E"),
            Some(Action::TerminalNewTab)
        );
        bindings.set(Action::TerminalNextTab, Some("Ctrl+Shift+T".into()))?;
        assert!(bindings.restore(Action::TerminalNewTab).is_err());
        assert_eq!(bindings.binding(Action::TerminalNewTab), Some("Ctrl+E"));
        bindings.restore(Action::TerminalNextTab)?;
        bindings.restore(Action::TerminalNewTab)?;
        assert!(bindings.is_default());
        Ok(())
    }

    #[test]
    fn an_existing_desktop_override_can_shadow_the_new_open_files_default() -> Result<()> {
        let keys: Shortcuts = serde_json::from_str(r#"{"minimize":"Ctrl+E"}"#)?;
        keys.validate()?;
        assert_eq!(keys.binding(Action::Minimize), Some("Ctrl+E"));
        assert_eq!(keys.binding(Action::OpenFiles), None);
        Ok(())
    }
}
