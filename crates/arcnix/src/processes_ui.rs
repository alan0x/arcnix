use arcnix_core::processes::Process;

use super::*;
use crate::server_apps_worker::{Action as AppAction, Reply};

#[derive(Clone, Default)]
pub(super) struct Runtime {
    pending: Option<u64>,
    snapshot: Option<Arc<[Process]>>,
    filter: String,
    filtered: Option<Arc<[usize]>>,
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
                update_filter(s);
            }
            Ok(_) => view.error = "Invalid process inventory response".into(),
            Err(error) => view.error = error,
        }
    });
    None
}

impl Runtime {
    fn count(&self) -> usize {
        self.filtered.as_deref().map_or_else(
            || self.snapshot.as_deref().unwrap_or_default().len(),
            |indices| indices.len(),
        )
    }

    fn process(&self, row: usize) -> Option<&Process> {
        let index = match &self.filtered {
            Some(indices) => *indices.get(row)?,
            None => row,
        };
        self.snapshot.as_ref()?.get(index)
    }
}

// Store indices into the sorted snapshot so scrolling never repeats a filter or port lookup
// across the inventory. An unfiltered list indexes the snapshot directly.
pub(super) fn update_filter(s: &mut State) {
    let view = &s.server_apps.processes;
    let filter = view.filter.trim().to_lowercase();
    let indices = (!filter.is_empty()).then(|| {
        view.snapshot
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .filter_map(|(index, p)| {
                (p.program.to_lowercase().contains(&filter)
                    || p.pid.to_string().contains(&filter)
                    || p.uid.to_string().contains(&filter)
                    || resource_ports_ui::process_ports(s, p)
                        .to_ascii_lowercase()
                        .contains(&filter))
                .then_some(index)
            })
            .collect()
    });
    s.server_apps.processes.filtered = indices;
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
                            update_filter(s);
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
            let view = &s.server_apps.processes;
            (view.count() > 0)
                .then(|| view.filter.clone())
                .into_iter()
                .collect::<Vec<_>>()
        }))
        .key(Clone::clone)
        .children(move |_| {
            col().class("monitor-process-table").fill((
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
                process_list(state),
            ))
        }),
        when(
            state.map(|s| {
                s.server_apps.processes.snapshot.is_some() && s.server_apps.processes.count() == 0
            }),
            || {
                text()
                    .class("subtle")
                    .content(i18n::bind("No matching processes"))
            },
        ),
    ))
}

fn process_list(state: Cage<State>) -> impl Widget {
    use glory::style::{Length, Style};

    glory::ui::widget(move |scope| {
        let offset = scope.cage(0.0_f64);
        let height = scope.cage(600.0_f64);
        let row_height = scope.cage(35.0_f64);
        let window = scope
            .bond(move || {
                const MARGIN: usize = 8;
                let count = state.with(|s| s.server_apps.processes.count());
                let step = row_height.get();
                let visible = (height.get() / step).ceil() as usize + 1;
                let first = ((offset.get() / step) as usize).min(count.saturating_sub(visible));
                (
                    first.saturating_sub(MARGIN)..(first + visible + MARGIN).min(count),
                    count,
                    step,
                )
            })
            .with_partial_eq();
        let spacer = |below: bool| {
            view().class("monitor-process-spacer").style(
                scope
                    .bond(move || {
                        window.with(|(rows, count, step)| {
                            let n = if below { count - rows.end } else { rows.start };
                            Style::new().height(Length::Px(n as f64 * step))
                        })
                    })
                    .with_partial_eq(),
            )
        };
        col()
            .class("monitor-process-viewport")
            .role(glory::render::Role::List)
            .name(i18n::bind("Processes"))
            .focus(FocusMode::TabStop)
            .on(scroll, move |event| {
                if let Event::Scroll { y, .. } = event {
                    offset.revise(|value| *value = y.max(0.0));
                }
            })
            .on(resize, move |event| {
                if let Event::Resize { viewport, .. } = event {
                    height.revise(|value| *value = viewport.height);
                    row_height
                        .revise(|value| *value = if viewport.width <= 510.0 { 56.0 } else { 35.0 });
                }
            })
            .fill((
                spacer(false),
                each(
                    scope.bond(move || window.with(|(rows, ..)| rows.clone().collect::<Vec<_>>())),
                )
                .key(|index| *index)
                .children(move |item| {
                    process_row(state, item.value.get())
                        .style(row_height.map(|step| Style::new().height(Length::Px(*step))))
                }),
                spacer(true),
            ))
            .build(scope)
    })
}

fn process_row(state: Cage<State>, index: usize) -> glory::ui::View {
    let cell = move |format: fn(&State, &Process) -> String| {
        state
            .map(move |s| {
                s.server_apps
                    .processes
                    .process(index)
                    .map(|p| format(s, p))
                    .unwrap_or_default()
            })
            .with_partial_eq()
    };
    row()
        .class("monitor-process-row")
        .role(glory::render::Role::ListItem)
        .fill((
            text()
                .selectable(true)
                .content(cell(|_, p| p.pid.to_string())),
            text().selectable(true).content(cell(|_, p| {
                if p.program.is_empty() {
                    "—".into()
                } else {
                    p.program.clone()
                }
            })),
            text().content(cell(|_, p| p.uid.to_string())),
            text().content(cell(|_, p| format!("{:.1}%", p.cpu_percent))),
            text().content(cell(|_, p| size(p.resident_bytes))),
            text()
                .class("monitor-process-ports")
                .selectable(true)
                .content(cell(|s, p| {
                    resource_ports_ui::process_ports(s, p).to_owned()
                })),
        ))
}
