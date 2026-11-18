use arcnix_core::desktop_shortcuts::Action as Shortcut;
use serde::Deserialize;

use super::*;

fn label(action: Shortcut) -> &'static str {
    match action {
        Shortcut::NextApp => "Switch to next app",
        Shortcut::PreviousApp => "Switch to previous app",
        Shortcut::Maximize => "Toggle maximize",
        Shortcut::Minimize => "Minimize window",
        Shortcut::Fullscreen => "Toggle desktop fullscreen",
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
    Run { chord: String },
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
        Intent::Run { chord } => {
            let action = ui
                .state
                .with(|s| s.server_apps.preferences.shortcuts.action(&chord));
            match action {
                Some(Shortcut::NextApp) => ui.cycle_desktop_apps(false),
                Some(Shortcut::PreviousApp) => ui.cycle_desktop_apps(true),
                Some(Shortcut::Maximize) => ui.toggle_desktop_maximize(),
                Some(Shortcut::Minimize) => ui.minimize_desktop_window(),
                Some(Shortcut::Fullscreen) => ui.toggle_desktop_fullscreen(),
                None => {}
            }
        }
        Intent::Record { action, chord } => change(ui, |keys| keys.set(action, Some(chord))),
    }
}

pub(super) fn bridge(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        text()
            .class("desktop-shortcut-state hidden")
            .content(state.map(|s| {
                let keys = &s.server_apps.preferences.shortcuts;
                let bindings: Vec<_> = Shortcut::ALL
                    .into_iter()
                    .filter(|action| match action {
                        Shortcut::NextApp | Shortcut::PreviousApp => !s.workbench.apps.is_empty(),
                        Shortcut::Maximize | Shortcut::Minimize => {
                            server_desktop::app_id(s.workspace_page).is_some()
                        }
                        Shortcut::Fullscreen => true,
                    })
                    .filter_map(|action| keys.binding(action))
                    .collect();
                serde_json::json!({
                    "epoch": s.epoch,
                    "active": desktop_active(s),
                    "bindings": bindings,
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

pub(super) fn controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    let reset = ui.clone();
    col().class("desktop-settings-card desktop-shortcut-group desktop-custom-keys").fill((
        row().class("section-heading desktop-key-heading").fill((
            text().class("section-title").content(i18n::bind("Desktop shortcuts")),
            help_ui::tip("Change shortcut", "Click a shortcut and press the new keys. Esc cancels. Use Ctrl, Alt or Meta with a letter, number, function key or navigation key."),
        )),
        fragment(Shortcut::ALL.into_iter().map(move |action| {
            let clear = ui.clone();
            row().class("desktop-key-binding").fill((
                text().content(i18n::bind(label(action))),
                row().class("desktop-key-controls").fill((
                    text().class("desktop-shortcut-id hidden").content(
                        serde_json::to_string(&action).expect("valid desktop shortcut action"),
                    ),
                    sapid::button()
                        .class("button desktop-shortcut-record")
                        .name(i18n::bind(format!("Change shortcut: {}", label(action))))
                        .enabled(state.map(editing))
                        .fill((
                            text().class("desktop-key-combination").content(i18n::bind(state.map(move |s| {
                                s.server_apps.preferences.shortcuts.binding(action).unwrap_or("Unassigned").to_owned()
                            }))),
                            text().class("desktop-key-recording").content(i18n::bind("Press shortcut…")),
                        )),
                    close_action(format!("Disable shortcut: {}", label(action)))
                        .class("icon-button desktop-shortcut-disable")
                        .enabled(state.map(move |s| editing(s) && s.server_apps.preferences.shortcuts.binding(action).is_some()))
                        .on_press(move || change(&clear, |keys| keys.set(action, None))),
                )),
            ))
        }).collect::<Vec<_>>()),
        btn("Restore shortcut defaults", move || change(&reset, |keys| {
            *keys = Default::default();
            Ok(())
        }))
        .class("desktop-shortcut-restore")
        .enabled(state.map(|s| editing(s) && !s.server_apps.preferences.shortcuts.is_default())),
        text().class("error desktop-shortcut-error").content(i18n::bind(
            state.map(|s| s.server_apps.shortcut_error.clone()),
        )),
    ))
}
