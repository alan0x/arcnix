use arcnix_core::host::HostOs;
use arcnix_core::network::Snapshot;

use crate::desktop_customization_ui::{SettingsPage, settings_page};
use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{btn, text_action};
use crate::server_apps_worker::{Action as AppAction, Reply};
use crate::*;

#[derive(Clone, Default)]
pub(crate) struct Runtime {
    snapshot: Option<Snapshot>,
    platform: Option<HostOs>,
    pending: Option<u64>,
    error: String,
}

fn active(s: &State) -> bool {
    server_apps_ui::settings_visible(s) && settings_page(s) == SettingsPage::Network
}

fn ready(s: &State) -> bool {
    active(s) && s.server_apps.network.pending.is_none()
}

fn can_configure(s: &State) -> bool {
    active(s) && s.server_apps.network.platform.or(s.metrics.remote_platform) == Some(HostOs::Linux)
}

pub(crate) fn enter(ui: &Ui) {
    if ui.state.with(|s| {
        ready(s)
            && s.server_apps.network.snapshot.is_none()
            && s.server_apps.network.error.is_empty()
    }) {
        refresh(ui);
    }
}

fn refresh(ui: &Ui) {
    if !ui.state.with(ready) {
        return;
    }
    let serial = ui.send(Action::ServerApps(AppAction::NetworkSettings));
    ui.state.revise(|s| {
        s.server_apps.network.pending = Some(serial);
        s.server_apps.network.error.clear();
    });
}

pub(crate) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    if !ui
        .state
        .with(|s| s.server_apps.network.pending == Some(serial))
    {
        return Some(result);
    }
    ui.state.revise(|s| {
        let network = &mut s.server_apps.network;
        network.pending = None;
        network.snapshot = match result {
            Ok(Reply::NetworkSettings(platform, snapshot)) => {
                network.platform = Some(platform);
                Some(snapshot)
            }
            Err(error) => {
                network.error = error;
                None
            }
            _ => {
                network.error = "Invalid network settings response".into();
                None
            }
        };
    });
    None
}

fn values(rows: &[String], empty: &'static str) -> String {
    if rows.is_empty() {
        empty.to_owned()
    } else {
        rows.join("\n")
    }
}

fn snapshot(s: &State) -> Option<&Snapshot> {
    s.server_apps.network.snapshot.as_ref()
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let read = ui.clone();
    col().class("desktop-settings-pane network-settings").fill((
        col().class("desktop-settings-card").fill((
            text()
                .class("section-title")
                .content(i18n::bind("Network interfaces")),
            row().class("choice-list").fill((
                text_action("Refresh", Icon::Refresh)
                    .class("button with-icon")
                    .enabled(state.map(ready))
                    .on_press(move || refresh(&read)),
                btn("Configure in Terminal…", move || {
                    if ui.state.with(can_configure) {
                        ui.open_dialog(Dialog::ConfigureNetwork);
                    }
                })
                .class("button")
                .enabled(state.map(can_configure)),
            )),
            when(
                state.map(|s| s.server_apps.network.pending.is_some()),
                || text().class("subtle").content(i18n::bind("Loading…")),
            ),
            text().class("error").content(i18n::bind(
                state.map(|s| s.server_apps.network.error.clone()),
            )),
            when(state.map(|s| snapshot(s).is_some()), move || {
                col().class("network-settings-list").fill(
                    each(
                        state.map(|s| snapshot(s).map_or_else(Vec::new, |s| s.interfaces.clone())),
                    )
                    .key(|interface| interface.ifindex)
                    .children(|item| {
                        let interface = item.value;
                        row().class("network-interface").fill((
                            col().fill((
                                text()
                                    .class("network-interface-name")
                                    .content(interface.map(|i| i.name.clone())),
                                text()
                                    .class("subtle")
                                    .content(interface.map(|i| i.state.clone())),
                            )),
                            text().class("network-settings-values").content(i18n::bind(
                                interface.map(|i| {
                                    if i.addresses.is_empty() {
                                        return "No IP addresses".to_owned();
                                    }
                                    i.addresses
                                        .iter()
                                        .map(|a| format!("{}/{}", a.ip, a.prefix))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                }),
                            )),
                        ))
                    }),
                )
            }),
        )),
        when(state.map(|s| snapshot(s).is_some()), move || {
            fragment((
                col().class("desktop-settings-card").fill((
                    text()
                        .class("section-title")
                        .content(i18n::bind("Default routes")),
                    row().class("network-interface").fill((
                        text().content("IPv4"),
                        text()
                            .class("network-settings-values")
                            .content(i18n::bind(state.map(|s| {
                                snapshot(s).map_or_else(String::new, |s| {
                                    values(&s.ipv4_routes, "No default route")
                                })
                            }))),
                    )),
                    row().class("network-interface").fill((
                        text().content("IPv6"),
                        text()
                            .class("network-settings-values")
                            .content(i18n::bind(state.map(|s| {
                                snapshot(s).map_or_else(String::new, |s| {
                                    values(&s.ipv6_routes, "No default route")
                                })
                            }))),
                    )),
                )),
                col().class("desktop-settings-card").fill((
                    text()
                        .class("section-title")
                        .content(i18n::bind("DNS servers (/etc/resolv.conf)")),
                    text()
                        .class("network-settings-values")
                        .content(i18n::bind(state.map(|s| {
                            snapshot(s)
                                .and_then(|s| s.dns_servers.as_ref())
                                .map_or_else(
                                    || "Resolver configuration unavailable".to_owned(),
                                    |servers| values(servers, "No DNS servers listed"),
                                )
                        }))),
                )),
            ))
        }),
    ))
}

pub(crate) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::ConfigureNetwork)),
        move || {
            let cancel = ui.clone();
            let confirm = ui.clone();
            let warning = text().class("network-warning").content(i18n::bind(concat!(
                "Changing IP addresses, routes, or firewall rules may interrupt SSH and prevent reconnection. ",
                "Make sure you have access to the server console before continuing."
            )));
            modal(
                ui.clone(),
                "Configure network",
                warning,
                (
                    btn("取消", move || cancel.close_dialog()).class("button"),
                    btn("Continue to Terminal", move || {
                        if confirm
                            .state
                            .with(|s| active(s) && s.dialog == Some(Dialog::ConfigureNetwork))
                        {
                            confirm.close_dialog();
                            confirm.open_desktop_app(WorkspacePage::Terminal);
                        }
                    })
                    .class("button danger")
                    .enabled(state.map(active)),
                ),
            )
        },
    )
}
