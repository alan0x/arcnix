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
}

impl Action {
    pub const ALL: [Self; 5] = [
        Self::NextApp,
        Self::PreviousApp,
        Self::Maximize,
        Self::Minimize,
        Self::Fullscreen,
    ];

    fn default_binding(self) -> &'static str {
        match self {
            Self::NextApp => "Ctrl+Tab",
            Self::PreviousApp => "Ctrl+Shift+Tab",
            Self::Maximize => "Ctrl+Shift+ArrowUp",
            Self::Minimize => "Ctrl+Shift+ArrowDown",
            Self::Fullscreen => "Ctrl+Shift+F",
        }
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

    pub fn binding(&self, action: Action) -> Option<&str> {
        self.0
            .get(&action)
            .map_or_else(|| Some(action.default_binding()), Option::as_deref)
    }

    pub fn action(&self, chord: &str) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|&action| self.binding(action) == Some(chord))
    }

    pub fn set(&mut self, action: Action, chord: Option<String>) -> Result<()> {
        if let Some(chord) = chord.as_deref() {
            validate_chord(chord)?;
            ensure!(
                self.action(chord).is_none_or(|existing| existing == action),
                "This shortcut is already in use."
            );
        }
        if chord.as_deref() == Some(action.default_binding()) {
            self.0.remove(&action);
        } else {
            self.0.insert(action, chord);
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let mut seen = HashSet::new();
        for chord in Action::ALL
            .into_iter()
            .filter_map(|action| self.binding(action))
        {
            validate_chord(chord)?;
            ensure!(seen.insert(chord), "This shortcut is already in use.");
        }
        Ok(())
    }
}

fn validate_chord(chord: &str) -> Result<()> {
    let Some((modifiers, key)) = chord.rsplit_once('+') else {
        anyhow::bail!(
            "Use Ctrl, Alt or Meta with a letter, number, function key or navigation key."
        );
    };
    let mut previous = 0;
    let mut primary = false;
    for modifier in modifiers.split('+') {
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
    let supported_key = matches!(
        key,
        "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "PageUp"
            | "PageDown"
            | "Home"
            | "End"
            | "F1"
            | "F2"
            | "F3"
            | "F4"
            | "F5"
            | "F6"
            | "F7"
            | "F8"
            | "F9"
            | "F10"
            | "F11"
            | "F12"
    ) || matches!(key.as_bytes(), [b'A'..=b'Z' | b'0'..=b'9'])
        || key == "Tab" && matches!(modifiers, "Ctrl" | "Ctrl+Shift");
    ensure!(
        primary && supported_key,
        "Use Ctrl, Alt or Meta with a letter, number, function key or navigation key."
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
        assert_eq!(bindings.action("Ctrl+Tab"), Some(Action::NextApp));
        assert_eq!(bindings.action("Ctrl+Shift+Tab"), Some(Action::PreviousApp));
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
                .set(Action::NextApp, Some("Shift+F12".into()))
                .is_err()
        );
        bindings.set(Action::Fullscreen, None)?;
        bindings.set(Action::NextApp, Some("Ctrl+Shift+F".into()))?;
        let decoded: Shortcuts = serde_json::from_str(&serde_json::to_string(&bindings)?)?;
        decoded.validate()?;
        assert_eq!(decoded, bindings);
        assert_eq!(decoded.action("Ctrl+Shift+F"), Some(Action::NextApp));
        assert_eq!(decoded.binding(Action::Fullscreen), None);
        bindings.set(Action::NextApp, Some("Ctrl+Tab".into()))?;
        bindings.set(Action::Fullscreen, Some("Ctrl+Shift+F".into()))?;
        assert!(bindings.is_default());
        Ok(())
    }
}
