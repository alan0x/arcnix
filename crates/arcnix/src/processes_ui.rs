use arcnix_core::processes::Process;

use super::*;
use crate::server_apps_worker::{Action as AppAction, Reply};

const PAGE_SIZE: usize = 50;

#[derive(Clone, Default)]
pub(super) struct Runtime {
    pending: Option<u64>,
    snapshot: Option<Arc<[Process]>>,
    filter: String,
    page: usize,
    error: String,
    port_details: bool,
}

pub(super) fn port_details_visible(s: &State) -> bool {
    s.server_apps.processes.port_details
}

pub(super) fn show_port_details(s: &mut State, show: bool) {
    s.server_apps.processes.port_details = show;
    if !show {
        resource_ports_ui::dismiss(s);
    }
}

fn ready(s: &State) -> bool {
    s.connected
        && !s.system_task_running
        && s.server_apps.processes.pending.is_none()
        && monitor_ui::active(s) == Some(monitor_ui::Section::Processes)
}

pub(super) fn enter(ui: &Ui) {
    if ui.state.with(|s| {
        !port_details_visible(s)
            && s.server_apps.processes.snapshot.is_none()
            && s.server_apps.processes.error.is_empty()
    }) {
        refresh_processes(ui);
    }
    ui.ports_enter();
}

fn refresh(ui: &Ui) {
    refresh_processes(ui);
    ui.ports_refresh_as_user();
}

fn refresh_processes(ui: &Ui) {
    if ui.state.with(ready) {
        let serial = ui.send(Action::ServerApps(AppAction::Processes));
        ui.state.revise(|s| {
            s.server_apps.processes.pending = Some(serial);
            s.server_apps.processes.error.clear();
        });
    }
}

pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    if !ui
        .state
        .with(|s| s.server_apps.processes.pending == Some(serial))
    {
        return Some(result);
    }
    ui.state.revise(|s| {
        let view = &mut s.server_apps.processes;
        view.pending = None;
        match result {
            Ok(Reply::Processes(rows)) => {
                view.snapshot = Some(rows.into());
                view.page = 0;
            }
            Ok(_) => view.error = "Invalid process inventory response".into(),
            Err(error) => view.error = error,
        }
    });
    None
}

fn matching(s: &State) -> impl Iterator<Item = &Process> {
    let view = &s.server_apps.processes;
    let filter = view.filter.trim().to_lowercase();
    view.snapshot
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(move |process| {
            filter.is_empty()
                || process.program.to_lowercase().contains(&filter)
                || process.pid.to_string().contains(&filter)
                || process.uid.to_string().contains(&filter)
                || resource_ports_ui::process_ports(s, process)
                    .to_ascii_lowercase()
                    .contains(&filter)
        })
}

fn pages(s: &State) -> usize {
    matching(s).count().div_ceil(PAGE_SIZE).max(1)
}

fn page_index(s: &State) -> usize {
    let page = s.server_apps.processes.page;
    if page == 0 { 0 } else { page.min(pages(s) - 1) }
}

fn status(s: &State) -> &'static str {
    let view = &s.server_apps.processes;
    if view.pending.is_some() {
        "Reading processes…"
    } else if view.snapshot.is_some() && !view.error.is_empty() {
        "Showing the previous process snapshot"
    } else {
        ""
    }
}

pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let details = ui.clone();
    fragment((
        when(state.map(port_details_visible), move || {
            let back = details.clone();
            col().class("monitor-port-details").fill((
                row().fill(btn("Back to processes", move || {
                    state.revise(|s| show_port_details(s, false));
                    enter(&back);
                })),
                resource_ports_ui::panel(details.clone()),
            ))
        }),
        when(state.map(|s| !port_details_visible(s)), move || {
            inventory(ui.clone())
        }),
    ))
}

fn inventory(ui: Ui) -> impl Widget {
    let state = ui.state;
    let details = ui.clone();
    col().class("monitor-process-inventory").fill((
        row().class("monitor-process-toolbar").fill((
            input()
                .class("monitor-process-filter")
                .name(i18n::bind("Filter by program, PID, UID or port"))
                .semantics(state.map(|s| {
                    Some(Box::new(glory::render::Semantics {
                        placeholder: Some(i18n::translate_for(
                            "Filter processes",
                            s.config.settings.language,
                        )),
                        ..Default::default()
                    }))
                }))
                .value(state.map(|s| s.server_apps.processes.filter.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| {
                            s.server_apps.processes.filter = value;
                            s.server_apps.processes.page = 0;
                        });
                    }
                }),
            row().class("monitor-process-actions").fill((
                btn("Port details", move || {
                    state.revise(|s| show_port_details(s, true));
                    enter(&details);
                }),
                text_action("Refresh processes", Icon::Refresh)
                    .enabled(state.map(ready))
                    .on_press(move || refresh(&ui)),
            )),
            when(state.map(|s| pages(s) > 1), move || {
                row().class("monitor-process-pager").fill((
                    icon_action("Previous page", Icon::ChevronRight)
                        .class("icon-button monitor-process-previous")
                        .enabled(state.map(|s| page_index(s) > 0))
                        .on_press(move || {
                            state.revise(|s| {
                                s.server_apps.processes.page = page_index(s).saturating_sub(1);
                            })
                        }),
                    text()
                        .class("subtle")
                        .content(state.map(|s| format!("{} / {}", page_index(s) + 1, pages(s)))),
                    icon_action("Next page", Icon::ChevronRight)
                        .enabled(state.map(|s| page_index(s) + 1 < pages(s)))
                        .on_press(move || {
                            state.revise(|s| {
                                let next = page_index(s) + 1;
                                if next < pages(s) {
                                    s.server_apps.processes.page = next;
                                }
                            })
                        }),
                ))
            }),
        )),
        when(
            state.map(|s| {
                !status(s).is_empty()
                    || !resource_ports_ui::process_port_status(s).is_empty()
                    || !s.server_apps.processes.error.is_empty()
            }),
            move || {
                col().class("monitor-process-feedback").fill((
                    text()
                        .class("subtle monitor-process-status")
                        .content(i18n::bind(state.map(|s| status(s).to_owned()))),
                    text()
                        .class("subtle monitor-process-status")
                        .content(i18n::bind(
                            state.map(|s| resource_ports_ui::process_port_status(s).to_owned()),
                        )),
                    text()
                        .class("error monitor-process-error")
                        .content(i18n::bind(
                            state.map(|s| s.server_apps.processes.error.clone()),
                        )),
                ))
            },
        ),
        each(state.map(|s| {
            matching(s)
                .next()
                .map(|_| (page_index(s), s.server_apps.processes.filter.clone()))
                .into_iter()
                .collect::<Vec<_>>()
        }))
        .key(Clone::clone)
        .children(move |_| {
            col()
                .class("monitor-process-table")
                .name(i18n::bind("Processes"))
                .focus(FocusMode::TabStop)
                .fill((
                    row()
                        .class("monitor-process-row monitor-process-header")
                        .fill((
                            text().content("PID"),
                            text().content(i18n::bind("Program")),
                            text().content("UID"),
                            text().content("CPU"),
                            text().content(i18n::bind("Memory")),
                            text()
                                .class("monitor-process-port-heading")
                                .content(i18n::bind("Listening ports")),
                        )),
                    each(state.map(|s| {
                        matching(s)
                            .skip(page_index(s) * PAGE_SIZE)
                            .take(PAGE_SIZE)
                            .cloned()
                            .collect::<Vec<_>>()
                    }))
                    .key(|process| process.pid)
                    .children(move |item| {
                        let process = item.value;
                        row().class("monitor-process-row").fill((
                            text()
                                .selectable(true)
                                .content(process.map(|p| p.pid.to_string())),
                            text().selectable(true).content(process.map(|p| {
                                if p.program.is_empty() {
                                    "—".into()
                                } else {
                                    p.program.clone()
                                }
                            })),
                            text().content(process.map(|p| p.uid.to_string())),
                            text().content(process.map(|p| format!("{:.1}%", p.cpu_percent))),
                            text().content(process.map(|p| size(p.resident_bytes))),
                            text()
                                .class("monitor-process-ports")
                                .selectable(true)
                                .content(state.map(move |s| {
                                    process
                                        .with(|p| resource_ports_ui::process_ports(s, p).to_owned())
                                })),
                        ))
                    }),
                ))
        }),
        when(
            state.map(|s| {
                s.server_apps.processes.snapshot.is_some() && matching(s).next().is_none()
            }),
            || {
                text()
                    .class("subtle")
                    .content(i18n::bind("No matching processes"))
            },
        ),
    ))
}
