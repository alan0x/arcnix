use arcnix_core::host::PowerAction;
use glory::render::{AnchorAlign, AnchorPlacement, AnchorSide};

use crate::desktop_customization_ui::{SettingsPage, settings_page};
use crate::i18n::desktop as i18n;
use crate::*;

pub(super) fn supported(s: &State) -> bool {
    matches!(s.local_desktop, None | Some(local_desktop::Target::Native))
}

pub(super) fn available(s: &State) -> bool {
    server_desktop::visible(s)
        && supported(s)
        && s.server_apps.power_pending.is_none()
        && (s.local_desktop.is_some() || !s.server_apps.connected_address.is_empty())
}

#[derive(Clone, Copy)]
pub(super) enum MenuLocation {
    General,
    Launcher,
}

pub(super) fn menu(ui: Ui, location: MenuLocation) -> impl Widget {
    let (id, side) = match location {
        MenuLocation::General => ("device-power", AnchorSide::Bottom),
        MenuLocation::Launcher => ("launcher-power", AnchorSide::Top),
    };
    let state = ui.state;
    let item = |action: PowerAction, label: &'static str| {
        let ui = ui.clone();
        sapid::menu_item(action.key())
            .label(i18n::bind(label))
            .on_activate(move || {
                if let Some(epoch) = state.with(|s| {
                    let visible = match location {
                        MenuLocation::General => {
                            s.dialog.is_none()
                                && server_apps_ui::settings_visible(s)
                                && settings_page(s) == SettingsPage::General
                        }
                        MenuLocation::Launcher => {
                            s.dialog == Some(Dialog::Launcher)
                                && desktop_customization_ui::appearance(s).dock.launcher_power
                        }
                    };
                    (visible && available(s)).then_some(s.epoch)
                }) {
                    ui.open_dialog(Dialog::SystemPower(epoch, action));
                }
            })
    };
    row().class("device-power-menu").fill(
        sapid::dropdown_menu(id)
            .label(i18n::bind("Power options"))
            .trigger_content(icons::glyph(Icon::Power))
            .position(AnchorPlacement::new(side, AnchorAlign::End))
            .enabled(state.map(available))
            .item(item(PowerAction::Restart, "Restart system…"))
            .item(item(PowerAction::ShutDown, "Shut down…")),
    )
}

pub(crate) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    each(state.map(|s| match s.dialog {
        Some(Dialog::SystemPower(epoch, action)) => vec![(epoch, action)],
        _ => Vec::new(),
    }))
    .key(|(epoch, action)| (*epoch, action.key()))
    .children(move |item| {
        let (epoch, action) = item.value.get();
        let ui = ui.clone();
        widget(move |scope| {
            let typed = scope.cage(String::new());
            let remote = state.with(|s| s.local_desktop.is_none());
            let strict = remote && action == PowerAction::ShutDown;
            let address = state.with(|s| s.server_apps.connected_address.clone());
            let cancel = ui.clone();
            let confirm = ui.clone();
            let expected = address.clone();
            let ready = move || {
                state.with(|s| available(s) && s.epoch == epoch
                    && s.dialog == Some(Dialog::SystemPower(epoch, action))
                    && (!remote || s.server_apps.connected_address == expected))
                    && (!strict || (!expected.is_empty() && typed.with(|value| value == &expected)))
            };
            let enabled = scope.bond(ready.clone());
            let (title, button) = match action {
                PowerAction::Restart => ("Restart system?", "Restart"),
                PowerAction::ShutDown => ("Shut down system?", "Shut down"),
            };
            let warning = match (action, remote) {
                (PowerAction::Restart, true) => "Restarting this server will interrupt its services and disconnect SSH. Save your work first.",
                (PowerAction::ShutDown, true) => "Shutting down this server will stop its services and disconnect SSH. You will need console or physical access to power it on again.",
                (PowerAction::Restart, false) => "Open sessions will be interrupted. Save your work before restarting.",
                (PowerAction::ShutDown, false) => "Shutting down will close applications and end this session. Save your work first.",
            };
            let target = if remote {
                format!("SSH · {}", state.with(server_desktop::selected_name))
            } else {
                i18n::translate("This computer")
            };
            modal(ui.clone(), title, col().class("dialog-stack").fill((
                text().class("section-title settings-copyable").content(target),
                when(remote, move || text().class("settings-copyable power-target-address").content(address.clone())),
                text().class("dialog-description").content(i18n::bind(warning)),
                when(strict, move || {
                    let prompt = i18n::bind("Enter the server address shown above to confirm shutdown");
                    col().class("field").fill((
                        text().class("field-label").content(prompt.clone()),
                        sapid::text_field().name(prompt).value(typed)
                            .on_value_change(move |value| typed.revise(|input| *input = value)),
                    ))
                }),
            )), (
                i18n::btn("Cancel", move || cancel.close_dialog()),
                i18n::btn(button, move || {
                    if ready() {
                        let serial = confirm.send(Action::ServerApps(server_apps_worker::Action::SystemPower(action)));
                        confirm.state.revise(|s| s.server_apps.power_pending = Some((serial, action)));
                        confirm.close_dialog();
                    }
                }).class("button danger").enabled(enabled),
            )).show_in(scope)
        })
    })
}
