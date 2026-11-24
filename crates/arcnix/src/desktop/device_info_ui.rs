use arcnix_core::device_info::DeviceInfo;

use crate::desktop_customization_ui::{SettingsPage, settings_page, settings_row};
use crate::i18n::desktop as i18n;
use crate::server_apps_worker::{Action as AppAction, Reply};
use crate::*;

#[derive(Clone, Default)]
pub(crate) struct Runtime {
    pub pending: Option<u64>,
    snapshot: Option<DeviceInfo>,
    error: String,
}

pub(crate) fn enter(ui: &Ui) {
    if ui.state.with(|s| {
        server_apps_ui::settings_visible(s)
            && settings_page(s) == SettingsPage::General
            && s.server_apps.device.pending.is_none()
            && s.server_apps.device.snapshot.is_none()
    }) {
        let serial = ui.send(Action::ServerApps(AppAction::DeviceInfo));
        ui.state.revise(|s| {
            s.server_apps.device.pending = Some(serial);
            s.server_apps.device.error.clear();
        });
    }
}

pub(crate) fn received(ui: &Ui, result: std::result::Result<Reply, String>) {
    ui.state.revise(|s| {
        let device = &mut s.server_apps.device;
        device.pending = None;
        match result {
            Ok(Reply::DeviceInfo(snapshot)) => device.snapshot = Some(snapshot),
            Err(error) => device.error = error,
            _ => device.error = "Invalid device information response".into(),
        }
    });
}

pub(crate) fn details(ui: Ui) -> impl Widget {
    let state = ui.state;
    let field = move |label, read: fn(&DeviceInfo, &State) -> String| {
        settings_row(
            label,
            text()
                .class("settings-copyable device-info-value")
                .content(state.map(move |s| {
                    let value = s
                        .server_apps
                        .device
                        .snapshot
                        .as_ref()
                        .map(|info| read(info, s))
                        .unwrap_or_default();
                    if value.is_empty() {
                        crate::i18n::translate_for(
                            "Unavailable",
                            desktop_system_settings::language(s),
                        )
                    } else {
                        value
                    }
                })),
        )
    };
    let power = ui.clone();
    col().class("desktop-settings-pane").fill((
        col()
            .class("desktop-settings-card desktop-settings-rows")
            .fill((
                settings::section_heading("Device", true, desktop_power::menu(power)),
                when(
                    state.map(|s| s.server_apps.device.pending.is_some()),
                    || text().class("subtle").content(i18n::bind("Loading…")),
                ),
                when(
                    state.map(|s| !s.server_apps.device.error.is_empty()),
                    move || {
                        let retry = ui.clone();
                        fragment((
                            text().class("error").content(i18n::bind(
                                state.map(|s| s.server_apps.device.error.clone()),
                            )),
                            row().fill(
                                i18n::text_action("Refresh", Icon::Refresh)
                                    .on_press(move || enter(&retry)),
                            ),
                        ))
                    },
                ),
                when(
                    state.map(|s| s.server_apps.device.snapshot.is_some()),
                    move || {
                        fragment((
                            field("CPU model", |info, _| info.cpu.join("\n")),
                            field("Physical cores", |info, _| {
                                info.cores.map(|n| n.to_string()).unwrap_or_default()
                            }),
                            field("Logical processors", |info, _| {
                                info.logical.map(|n| n.to_string()).unwrap_or_default()
                            }),
                            field("Memory", |info, s| {
                                info.memory
                                    .map(|n| formatting::bytes_with(n, s.config.settings.display))
                                    .unwrap_or_default()
                            }),
                            field("GPU model", |info, _| info.graphics.join("\n")),
                        ))
                    },
                ),
            )),
        when(
            state.map(|s| s.server_apps.device.snapshot.is_some()),
            move || {
                col()
                    .class("desktop-settings-card desktop-settings-rows")
                    .fill((
                        text()
                            .class("section-title")
                            .content(i18n::bind("System information")),
                        field("Operating system", |info, _| info.os.clone()),
                        field("OS version", |info, _| info.version.clone()),
                        field("Architecture", |info, _| info.architecture.clone()),
                        when(
                            state.map(|s| {
                                s.server_apps
                                    .device
                                    .snapshot
                                    .as_ref()
                                    .is_some_and(|info| !info.kernel.is_empty())
                            }),
                            move || field("Kernel version", |info, _| info.kernel.clone()),
                        ),
                    ))
            },
        ),
    ))
}
