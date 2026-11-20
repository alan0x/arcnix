use arcnix_core::desktop_shortcuts::{Action as Shortcut, GROUPS, Scope as ShortcutScope};
use serde::Deserialize;

use crate::*;
use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{close_action, icon_action};

fn can_run(s: &State, action: Shortcut) -> bool {
    match action {
        Shortcut::NextApp | Shortcut::PreviousApp => !s.workbench.apps.is_empty(),
        Shortcut::Maximize | Shortcut::Minimize => {
            server_desktop::app_id(s.workspace_page).is_some()
        }
        Shortcut::Fullscreen => true,
        Shortcut::ExitFullscreen => server_desktop::fullscreen(s),
        Shortcut::OpenFiles => files_tabs::available(s),
        Shortcut::FilesDuplicateWindow => s.workspace_page.is_files() && files_tabs::available(s),
        _ => false,
    }
}

fn desktop_active(s: &State) -> bool {
    server_desktop::visible(s) && s.dialog.is_none() && s.fingerprint.is_none()
}

fn editing(s: &State) -> bool {
    desktop_active(s)
        && s.workspace_page == WorkspacePage::Settings
        && desktop_customization_ui::settings_page(s)
            == desktop_customization_ui::SettingsPage::Shortcuts
        && server_apps_ui::preferences_ready(s)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Packet {
    epoch: u64,
    intent: Intent,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Intent {
    Run { action: Shortcut },
    Record { action: Shortcut, chord: String },
}

fn receive(ui: &Ui, value: &str) {
    if value.len() > 512 {
        return;
    }
    let Ok(packet) = serde_json::from_str::<Packet>(value) else {
        return;
    };
    if !ui
        .state
        .with(|s| desktop_active(s) && s.epoch == packet.epoch)
    {
        return;
    }
    match packet.intent {
        Intent::Run { action } => {
            if !ui.state.with(|s| {
                can_run(s, action)
                    && server_apps_ui::current(s)
                        .shortcuts
                        .binding(action)
                        .is_some()
            }) {
                return;
            }
            match action {
                Shortcut::NextApp => ui.cycle_desktop_apps(false),
                Shortcut::PreviousApp => ui.cycle_desktop_apps(true),
                Shortcut::Maximize => ui.toggle_desktop_maximize(),
                Shortcut::Minimize => ui.minimize_desktop_window(),
                Shortcut::Fullscreen | Shortcut::ExitFullscreen => ui.toggle_desktop_fullscreen(),
                Shortcut::OpenFiles => files_tabs::new_window(ui, files_tabs::WindowStart::Default),
                Shortcut::FilesDuplicateWindow => {
                    files_tabs::new_window(ui, files_tabs::WindowStart::Current)
                }
                _ => {}
            }
        }
        Intent::Record { action, chord } => change(ui, |keys| {
            if cfg!(windows)
                && (chord.split('+').any(|part| part == "Meta")
                    || matches!(
                        chord.as_str(),
                        "Alt+Tab"
                            | "Alt+Shift+Tab"
                            | "Alt+F4"
                            | "Ctrl+Alt+Delete"
                            | "Ctrl+Escape"
                            | "Ctrl+Shift+Escape"
                            | "Alt+Space"
                            | "Ctrl+Alt+Tab"
                    ))
            {
                anyhow::bail!("This shortcut is reserved by Windows.");
            }
            let occupied = match action.scope() {
                ShortcutScope::Files => matches!(
                    chord.as_str(),
                    "Ctrl+T"
                        | "Ctrl+W"
                        | "Ctrl+F"
                        | "F2"
                        | "F5"
                        | "Delete"
                        | "Shift+Delete"
                        | "Alt+Enter"
                        | "Ctrl+Shift+C"
                        | "Ctrl+C"
                        | "Ctrl+V"
                        | "Shift+F10"
                ),
                ShortcutScope::Code => matches!(
                    chord.as_str(),
                    "Ctrl+P"
                        | "Ctrl+F"
                        | "Ctrl+H"
                        | "Ctrl+Z"
                        | "Ctrl+Y"
                        | "Ctrl+A"
                        | "Ctrl+X"
                        | "Ctrl+C"
                        | "Ctrl+V"
                        | "Ctrl+Shift+S"
                        | "Ctrl+Shift+P"
                        | "Ctrl+K"
                        | "F2"
                ),
                _ => false,
            };
            anyhow::ensure!(!occupied, "This shortcut is already in use.");
            keys.set(action, Some(chord))
        }),
    }
}

pub(crate) fn bridge(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        text()
            .class("desktop-shortcut-state hidden")
            .content(state.map(|s| {
                let keys = &server_apps_ui::current(s).shortcuts;
                let actions = GROUPS
                    .iter()
                    .flat_map(|(_, definitions)| definitions.iter());
                let bindings = actions
                    .clone()
                    .map(|definition| (definition.action, keys.binding(definition.action)))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let commands = actions
                    .filter(|definition| can_run(s, definition.action))
                    .map(|definition| definition.action)
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "epoch": s.epoch,
                    "active": desktop_active(s),
                    "bindings": bindings,
                    "commands": commands,
                })
                .to_string()
            })),
        input()
            .class("desktop-shortcut-input hidden")
            .value("")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    receive(&ui, &value);
                }
            }),
    ))
}

fn change(
    ui: &Ui,
    apply: impl FnOnce(&mut arcnix_core::desktop_shortcuts::Shortcuts) -> anyhow::Result<()>,
) {
    let Some(mut prefs) = ui
        .state
        .with(|s| editing(s).then(|| server_apps_ui::desired(s)))
    else {
        return;
    };
    let result = apply(&mut prefs.shortcuts);
    ui.state.revise(|s| {
        s.server_apps.shortcut_error = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
    });
    if result.is_ok() && ui.state.with(|s| server_apps_ui::current(s) != &prefs) {
        ui.queue_desktop_preferences(prefs);
    }
}

pub(crate) fn controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    let reset = ui.clone();
    let error = text()
        .class("error desktop-shortcut-error")
        .content(i18n::bind(
            state.map(|s| s.server_apps.shortcut_error.clone()),
        ));
    col().class("desktop-settings-pane desktop-custom-keys").fill((
        row().class("section-heading").fill((
            text().class("section-title").content(i18n::bind("Keyboard shortcuts")),
            help_ui::tip("Change shortcut", if cfg!(windows) {
                "Click a shortcut and press the new keys to save. Esc cancels. Windows shortcuts such as Alt+Tab and Win+L are handled by the system."
            } else {
                "Click a shortcut and press the new keys to save. Esc cancels."
            }),
        )),
        error,
        fragment(GROUPS.iter().map(move |(scope, definitions)| {
            let title = match scope {
                ShortcutScope::Desktop => "Desktop",
                ShortcutScope::Terminal => "Terminal",
                ShortcutScope::Files => "资源管理器",
                ShortcutScope::Code => "Code",
            };
            col().class("desktop-settings-card desktop-shortcut-group").fill((
                text().class("section-title").content(i18n::bind(title)),
                fragment(definitions.iter().map(|definition| {
                    let action = definition.action;
                    let label = definition.label;
                    let clear = ui.clone();
                    let restore = ui.clone();
                    row().class("desktop-key-binding").fill((
                        text().content(i18n::bind(label)),
                        row().class("desktop-key-controls").fill((
                            text().class("desktop-shortcut-id hidden").content(
                                serde_json::to_string(&action).expect("valid desktop shortcut action"),
                            ),
                            sapid::button()
                                .class("button desktop-shortcut-record")
                                .name(i18n::bind(format!("Change shortcut: {label}")))
                                .enabled(state.map(editing))
                                .fill((
                                    text().class("desktop-key-combination").content(i18n::bind(state.map(move |s| {
                                        server_apps_ui::current(s).shortcuts.binding(action).unwrap_or("Unassigned").to_owned()
                                    }))),
                                    text().class("desktop-key-recording").content(i18n::bind("Press shortcut…")),
                                )),
                            when(state.map(move |s| server_apps_ui::current(s).shortcuts.overridden(action)), move || {
                                let restore = restore.clone();
                                icon_action(format!("Restore default: {label}"), Icon::Refresh)
                                    .class("icon-button")
                                    .enabled(state.map(editing))
                                    .on_press(move || change(&restore, |keys| keys.restore(action)))
                            }),
                            close_action(format!("Disable shortcut: {label}"))
                                .class("icon-button desktop-shortcut-disable")
                                .enabled(state.map(move |s| editing(s) && server_apps_ui::current(s).shortcuts.binding(action).is_some()))
                                .on_press(move || change(&clear, |keys| keys.set(action, None))),
                        )),
                    ))
                }).collect::<Vec<_>>()),
            ))
        }).collect::<Vec<_>>()),
        sapid::button()
            .class("button desktop-shortcut-restore")
            .label(i18n::bind("Restore defaults"))
            .enabled(state.map(|s| editing(s) && !server_apps_ui::current(s).shortcuts.is_default()))
            .on_press(move || change(&reset, |keys| {
                *keys = Default::default();
                Ok(())
            })),
    ))
}
