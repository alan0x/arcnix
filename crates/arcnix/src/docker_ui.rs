use arcnix_core::docker::{Container, ContainerDetails, Operation, Snapshot, VolumeDetails};

use super::*;
use crate::server_apps_worker::{Action as AppAction, Reply};

#[derive(Clone, Default)]
pub(super) struct DockerState {
    snapshot: Option<Snapshot>,
    selected: String,
    selected_volume: String,
    details: Option<ContainerDetails>,
    volume: Option<VolumeDetails>,
    pending: Option<u64>,
    pub(super) view: u8,
    filter: String,
    running_only: bool,
    detail_open: bool,
    logs: Option<LogView>,
    /// One of [`docker::LOG_LINES`]; zero until the user picks another count.
    log_lines: u16,
    log_filter: String,
    error: String,
    status: String,
    target: Option<(Container, Operation)>,
    restart: String,
    cpu: String,
    memory: String,
    original_cpu: String,
    original_memory: String,
}
use crate::log_text::{LogSpan, Spans};
/// The logs from one reply. The text is shared so that cloning the state stays
/// cheap; it is turned into spans only when the reply, filter or language
/// changes, not on every state change.
#[derive(Clone, Default)]
pub(super) struct LogView {
    reply: u64,
    text: std::rc::Rc<str>,
}
fn log_lines(s: &State) -> u16 {
    if docker::LOG_LINES.contains(&s.docker.log_lines) {
        s.docker.log_lines
    } else {
        docker::LOG_LINES[0]
    }
}
/// `docker logs --timestamps` prefixes each line with an RFC 3339 time and a space.
fn split_timestamp(line: &str) -> Option<(&str, &str)> {
    let (time, rest) = line.split_once(' ')?;
    let b = time.as_bytes();
    (b.len() >= 20
        && b[4] == b'-'
        && b[10] == b'T'
        && b.iter()
            .all(|c| c.is_ascii_digit() || b"-T:.Z+".contains(c)))
    .then_some((time, rest))
}
/// Container output with its terminal colors, one muted timestamp per line.
/// A carriage return in a line restarts only the output after the timestamp.
///
/// Only lines whose text as shown contains `filter` are kept. Colors set in a
/// hidden line still apply to the lines after it, as they would in a terminal.
fn log_spans(logs: &str, filter: &str) -> Vec<LogSpan> {
    let needle = filter.to_lowercase();
    let mut spans = Spans::default();
    for line in logs.strip_suffix('\n').unwrap_or(logs).split('\n') {
        let start = spans.mark();
        let output = match split_timestamp(line) {
            Some((time, output)) => {
                spans.plain(&format!("{time} "), "docker-log-time");
                output
            }
            None => line,
        };
        spans.output(output);
        if !needle.is_empty() && !spans.text_since(start).to_lowercase().contains(&needle) {
            spans.truncate(start);
            continue;
        }
        spans.newline();
    }
    // Output with nothing to show, such as an empty reply, reads as "No logs".
    if spans.is_blank() {
        return Vec::new();
    }
    spans.finish()
}
fn ready(s: &State) -> bool {
    s.connected
        && s.server_apps.applications.docker
        && s.docker.pending.is_none()
        && !s.system_task_running
}
fn selected(s: &State) -> Option<Container> {
    s.docker
        .snapshot
        .as_ref()?
        .containers
        .iter()
        .find(|c| c.id == s.docker.selected)
        .cloned()
}
/// Advice for the CLI's error when it may not use the system Docker socket,
/// which usually means the SSH account is not in the docker group.
fn socket_hint(error: &str, user: &str) -> Option<String> {
    let error = error.to_lowercase();
    let system_socket = ["unix:///var/run/docker.sock", "unix:///run/docker.sock"]
        .iter()
        .any(|endpoint| error.contains(endpoint));
    (system_socket && error.contains("permission denied") && !user.is_empty()).then(|| {
        let user = arcnix_core::remote_tasks::quote(user);
        i18n::translate(&format!(
            "This SSH account cannot use the Docker socket. On the server, run `sudo usermod -aG docker -- {user}`, then reconnect so the new group applies. Members of the docker group have root-equivalent access to the server."
        ))
    })
}
fn hint(s: &State) -> Option<String> {
    let user = s
        .config
        .servers
        .iter()
        .find(|p| Some(&p.id) == s.selected.as_ref())
        .map(|p| p.username.as_str())?;
    socket_hint(&s.docker.error, user)
}
fn actionable(s: &State) -> bool {
    ready(s) && s.docker.error.is_empty() && selected(s).is_some()
}
impl Ui {
    fn docker_send(&self, action: AppAction) {
        if !self.state.with(ready) {
            return;
        }
        let serial = self.send(Action::ServerApps(action));
        self.state.revise(|s| {
            s.docker.pending = Some(serial);
            s.docker.error.clear();
            s.docker.status.clear();
        });
    }
    pub(super) fn docker_enter(&self) {
        if self.state.with(|s| {
            navigation::workspace_visible(s)
                && s.workspace_page == WorkspacePage::Docker
                && s.docker.snapshot.is_none()
                && s.docker.error.is_empty()
                && ready(s)
        }) {
            self.docker_refresh();
        }
    }
    fn docker_refresh(&self) {
        if self.state.with(|s| s.docker.view == 3) {
            self.compose_refresh();
            return;
        }
        self.docker_send(AppAction::DockerSnapshot);
    }
    fn docker_select(&self, id: String) {
        if !self.state.with(ready) {
            return;
        }
        self.state.revise(|s| {
            s.docker.selected = id.clone();
            s.docker.detail_open = true;
            s.docker.details = None;
            s.docker.logs = None;
            s.docker.log_filter.clear();
            s.docker.view = 0;
        });
        self.docker_send(AppAction::DockerInspect(id));
    }
    fn docker_logs(&self) {
        if !self.state.with(actionable) {
            return;
        }
        let id = self.state.with(|s| s.docker.selected.clone());
        let lines = self.state.with(log_lines);
        self.state
            .revise(|s| s.docker.logs = Some(LogView::default()));
        self.docker_send(AppAction::DockerLogs(id, lines));
    }
    fn docker_prepare(&self, operation: Operation) {
        if !self.state.with(actionable) {
            return;
        }
        if let Some(target) = self.state.with(selected) {
            self.state
                .revise(|s| s.docker.target = Some((target, operation)));
            self.open_dialog(Dialog::DockerAction);
        }
    }
    fn docker_operate(&self) {
        if !self.state.with(actionable) {
            return;
        }
        let target = self.state.with(|s| s.docker.target.clone());
        if let Some((target, operation)) = target {
            self.docker_send(AppAction::DockerOperate(target.id, operation));
        }
    }
    fn docker_configure(&self) {
        if !self.state.with(actionable) {
            return;
        }
        let details = self.state.with(|s| s.docker.details.clone());
        if let Some(details) = details {
            self.state.revise(|s| {
                s.docker.restart = if details.restart == "on-failure" && details.retries > 0 {
                    format!("on-failure:{}", details.retries)
                } else {
                    details.restart.clone()
                };
                let cpu = details.nano_cpus / 1_000_000;
                s.docker.cpu = format!("{}.{:03}", cpu / 1000, cpu % 1000);
                s.docker.memory = details.memory.div_ceil(1024 * 1024).to_string();
                s.docker.original_cpu = s.docker.cpu.clone();
                s.docker.original_memory = s.docker.memory.clone();
                s.docker.target = selected(s).map(|c| {
                    (
                        c,
                        Operation::Configure {
                            restart: s.docker.restart.clone(),
                            cpu_millis: None,
                            memory_mib: None,
                        },
                    )
                });
            });
            self.open_dialog(Dialog::DockerConfigure);
        }
    }
    fn docker_save_config(&self) {
        if !self.state.with(actionable) {
            return;
        }
        let result = self.state.with(|s| -> anyhow::Result<Operation> {
            let cpu_millis = if s.docker.cpu == s.docker.original_cpu {
                None
            } else {
                Some(parse_cpu(&s.docker.cpu)?)
            };
            let memory_mib = if s.docker.memory == s.docker.original_memory {
                None
            } else {
                Some(s.docker.memory.trim().parse::<u64>()?)
            };
            let operation = Operation::Configure {
                restart: s.docker.restart.trim().to_owned(),
                cpu_millis,
                memory_mib,
            };
            let target = s
                .docker
                .target
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Select a container"))?;
            arcnix_core::docker::operation_command(&target.0.id, &operation)?;
            Ok(operation)
        });
        match result {
            Ok(operation) => {
                self.state.revise(|s| {
                    if let Some(target) = &mut s.docker.target {
                        target.1 = operation;
                    }
                });
                self.docker_operate();
            }
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
        }
    }
}
fn parse_cpu(text: &str) -> anyhow::Result<u32> {
    let (whole, fraction) = text.trim().split_once('.').unwrap_or((text.trim(), ""));
    anyhow::ensure!(
        !whole.is_empty()
            && whole.bytes().all(|b| b.is_ascii_digit())
            && fraction.len() <= 3
            && fraction.bytes().all(|b| b.is_ascii_digit()),
        "Use a CPU count with up to three decimal places"
    );
    let whole = whole.parse::<u32>()?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u32>()? * 10u32.pow(3 - fraction.len() as u32)
    };
    whole
        .checked_mul(1000)
        .and_then(|n| n.checked_add(fraction))
        .ok_or_else(|| anyhow::anyhow!("CPU limit is too large"))
}

pub(super) fn event(ui: &Ui, serial: u64, result: std::result::Result<Reply, String>) {
    if !ui.state.with(|s| s.docker.pending == Some(serial)) {
        return;
    }
    let mut select_after = None;
    let mut refresh = false;
    let mut close = false;
    ui.state.revise(|s| {
        s.docker.pending = None;
        match result {
            Ok(Reply::Snapshot(snapshot)) => {
                let still_present = snapshot
                    .containers
                    .iter()
                    .any(|c| c.id == s.docker.selected);
                if !still_present {
                    s.docker.selected.clear();
                    s.docker.detail_open = false;
                }
                s.docker.details = None;
                s.docker.volume = None;
                s.docker.logs = None;
                if s.docker.view == 0 && s.docker.detail_open {
                    select_after = Some(s.docker.selected.clone());
                }
                s.docker.snapshot = Some(snapshot);
            }
            Ok(Reply::Container(details)) if details.id == s.docker.selected => {
                s.docker.details = Some(details)
            }
            Ok(Reply::Volume(volume)) if volume.name == s.docker.selected_volume => {
                s.docker.volume = Some(volume)
            }
            Ok(Reply::Logs(logs)) => {
                s.docker.logs = Some(LogView {
                    reply: serial,
                    text: logs.into(),
                })
            }
            Ok(Reply::Operated) => {
                refresh = true;
                close = true;
                s.docker.status = "Docker operation completed".into();
            }
            Err(error) => s.docker.error = error,
            _ => s.docker.error = "Invalid Docker response".into(),
        }
    });
    if close {
        if ui.state.with(|s| {
            matches!(
                s.dialog,
                Some(Dialog::DockerAction | Dialog::DockerConfigure)
            )
        }) {
            ui.close_dialog();
        }
        ui.state.revise(|s| s.docker.target = None);
        ui.log("Docker operation completed");
    }
    if refresh {
        ui.docker_refresh();
    } else if let Some(id) = select_after {
        ui.docker_select(id);
    }
}

fn container_button(ui: &Ui, label: &'static str, operation: Operation) -> impl Widget + use<> {
    let state = ui.state;
    let ui = ui.clone();
    let predicate = operation.clone();
    sapid::button()
        .class("button compact")
        .label(i18n::bind(label))
        .enabled(state.map(move |s| {
            actionable(s)
                && selected(s).is_some_and(|c| match predicate {
                    Operation::Start => matches!(c.state.as_str(), "exited" | "created"),
                    Operation::Stop => {
                        matches!(c.state.as_str(), "running" | "restarting" | "paused")
                    }
                    _ => true,
                })
        }))
        .on_press(move || ui.docker_prepare(operation.clone()))
}
fn container_list(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("docker-container-list").fill(
        each(state.map(|s| {
            let filter = s.docker.filter.to_lowercase();
            s.docker
                .snapshot
                .as_ref()
                .map(|v| {
                    v.containers
                        .iter()
                        .filter(|c| {
                            (!s.docker.running_only || c.state == "running")
                                && format!("{} {} {} {}", c.name, c.image, c.state, c.ports)
                                    .to_lowercase()
                                    .contains(&filter)
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }))
        .key(|c| c.id.clone())
        .children(move |item| {
            let container = item.value;
            let select = ui.clone();
            sapid::button()
                .class(state.map(move |s| {
                    if s.docker.selected == container.get().id {
                        "docker-container-row selected"
                    } else {
                        "docker-container-row"
                    }
                    .to_owned()
                }))
                .name(container.map(|c| c.name.clone()))
                .enabled(state.map(ready))
                .pressed(state.map(move |s| s.docker.selected == container.get().id))
                .fill((
                    text()
                        .class("docker-resource-name")
                        .content(container.map(|c| c.name.clone())),
                    text()
                        .class(container.map(|c| {
                            if c.state == "running" {
                                "docker-resource-state running"
                            } else {
                                "docker-resource-state"
                            }
                            .to_owned()
                        }))
                        .content(container.map(|c| c.status.clone())),
                    text()
                        .class("docker-resource-image")
                        .content(container.map(|c| c.image.clone())),
                    text()
                        .class("docker-resource-ports")
                        .content(container.map(|c| c.ports.clone())),
                ))
                .on_press(move || select.docker_select(container.get().id))
        }),
    )
}
fn detail(ui: Ui) -> impl Widget {
    let state = ui.state;
    let configure = ui.clone();
    let logs = ui.clone();
    col().class("docker-detail").fill((
        sapid::button().class("button compact docker-back").label(i18n::bind("Back to containers")).on_press(move || state.revise(|s| s.docker.detail_open = false)),
        text().class("docker-detail-name").content(state.map(|s| selected(s).map(|c| c.name).unwrap_or_default())),
        row().class("docker-container-actions").fill((
            container_button(&ui, "Start", Operation::Start),
            container_button(&ui, "Stop", Operation::Stop),
            container_button(&ui, "Restart", Operation::Restart),
            text_action("Logs", Icon::List).class("button with-icon compact").enabled(state.map(actionable)).on_press(move || logs.docker_logs()),
            text_action("Configure", Icon::Settings).class("button with-icon compact").enabled(state.map(|s| actionable(s) && s.docker.details.as_ref().is_some_and(|d| d.id == s.docker.selected))).on_press(move || configure.docker_configure()),
        )),
        text().class("docker-container-id subtle").selectable(true).content(state.map(|s| s.docker.selected.clone())),
        when(state.map(|s| s.docker.logs.is_some()), {
            let ui = ui.clone();
            move || {
                let overview = ui.clone();
                col().class("docker-log-panel").fill((
                    row().class("section-heading").fill((
                        text().content(i18n::bind("Recent logs")),
                        help_ui::tip("Recent logs", "The last 200, 500 or 1000 lines, fetched on demand. Container stdout and stderr are included, with their terminal colors. No background log process."),
                        sapid::button().class("button compact").label(i18n::bind("Details")).on_press(move || overview.state.revise(|s| s.docker.logs = None)),
                    )),
                    row().class("docker-log-tools").fill((
                        text().class("field-label").content(i18n::bind("Lines")),
                        row().class("logs-options").fill(docker::LOG_LINES.iter().map(|&lines| {
                            let ui = ui.clone();
                            sapid::button()
                                .class(state.map(move |s| if log_lines(s) == lines { "choice active" } else { "choice" }.to_owned()))
                                .label(lines.to_string())
                                .enabled(state.map(actionable))
                                .on_press(move || {
                                    ui.state.revise(|s| s.docker.log_lines = lines);
                                    ui.docker_logs();
                                })
                        }).collect::<Vec<_>>()),
                        input()
                            .name(i18n::bind("筛选当前日志"))
                            .semantics(Some(Box::new(Semantics { placeholder: Some(i18n::translate("筛选当前结果…")), ..Default::default() })))
                            .class("logs-filter")
                            .value(state.map(|s| s.docker.log_filter.clone()))
                            .on(value_changed, move |e| {
                                if let glory::ui::Event::ValueChanged { value, .. } = e {
                                    state.revise(|s| s.docker.log_filter = value);
                                }
                            }),
                    )),
                    // The scrolling block stays, so filtering keeps the scroll position. Its
                    // spans are rebuilt for a new reply, filter or language; other state
                    // changes leave them alone.
                    view().class("docker-log-output").fill(
                        each(state.map(|s| {
                            s.docker.logs.as_ref().map(|v| {
                                let loading = v.reply == 0 && s.docker.pending.is_some();
                                (v.reply, loading, s.docker.log_filter.clone(), s.config.settings.language as u8)
                            }).into_iter().collect::<Vec<_>>()
                        }))
                        .key(Clone::clone)
                        .children(move |item| {
                            let (_, loading, filter, _) = item.value.get_untracked();
                            let text = state.with_untracked(|s| s.docker.logs.as_ref().map(|v| v.text.clone()).unwrap_or_default());
                            let mut spans = log_spans(&text, &filter);
                            if spans.is_empty() {
                                let message = if loading {
                                    "Loading…"
                                } else if filter.is_empty() || log_spans(&text, "").is_empty() {
                                    "No logs"
                                } else {
                                    "No entries match this filter."
                                };
                                let mut empty = Spans::default();
                                empty.plain(&i18n::translate(message), "");
                                spans = empty.finish();
                            }
                            crate::log_text::view_spans("log-text", &spans)
                        }),
                    ),
                ))
            }
        }).otherwise(move || col().class("docker-inspection").fill((
            text().class("docker-configuration-summary").content(i18n::bind(state.map(|s| s.docker.details.as_ref().map(|d| format!("Restart policy: {} · CPU: {} · Memory: {} MiB", if d.restart == "on-failure" && d.retries > 0 { format!("on-failure:{}", d.retries) } else { d.restart.clone() }, d.nano_cpus as f64 / 1_000_000_000.0, d.memory / (1024 * 1024))).unwrap_or_default()))),
            row().class("section-heading").fill((text().content(i18n::bind("Port bindings")), help_ui::tip("Port bindings", "Published mappings show host addresses and ports. Exposed ports without a host binding are not published. Host networking uses the server network directly. Changing mappings or mounts requires recreating the container."))),
            text().class("docker-port-details").selectable(true).content(state.map(|s| s.docker.details.as_ref().map(format_ports).unwrap_or_default())),
            text().class("field-label").content(i18n::bind("Mounts")),
            text().class("docker-mount-details").selectable(true).content(state.map(|s| s.docker.details.as_ref().map(format_mounts).unwrap_or_default())),
        ))),
    ))
}
fn format_ports(details: &ContainerDetails) -> String {
    let mut rows = vec![format!(
        "{}: {}",
        i18n::translate("Network"),
        details.network
    )];
    for (source, value) in [
        ("Published", &details.ports),
        ("Configured", &details.bindings),
    ] {
        if let Some(ports) = value.as_object() {
            let mut keys = ports.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(bindings) = ports[key].as_array() {
                    for binding in bindings {
                        let host = binding["HostIp"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("*");
                        let host = if host.contains(':') {
                            format!("[{host}]")
                        } else {
                            host.to_owned()
                        };
                        rows.push(format!(
                            "{}: {}:{} → {key}",
                            i18n::translate(source),
                            host,
                            binding["HostPort"].as_str().unwrap_or("?")
                        ));
                    }
                } else if source == "Published" {
                    rows.push(format!("{key} · {}", i18n::translate("Exposed only")));
                }
            }
        }
    }
    rows.join("\n")
}
fn format_mounts(details: &ContainerDetails) -> String {
    details
        .mounts
        .as_array()
        .map(|mounts| {
            mounts
                .iter()
                .map(|m| {
                    format!(
                        "{} · {} → {} · {}",
                        m["Type"].as_str().unwrap_or("?"),
                        m["Source"].as_str().unwrap_or("?"),
                        m["Destination"].as_str().unwrap_or("?"),
                        if m["RW"].as_bool() == Some(true) {
                            "rw"
                        } else {
                            "ro"
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
fn volumes(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("docker-volumes").fill((
        column_titles(
            &["Name", "Driver / scope", "Used by"],
            "docker-volume-head docker-columns",
        ),
        col().class("docker-volume-list").fill(
            each(state.map(|s| {
                let filter = s.docker.filter.to_lowercase();
                s.docker
                    .snapshot
                    .as_ref()
                    .map(|v| {
                        v.volumes
                            .iter()
                            .filter(|v| {
                                format!("{} {}", v.name, v.driver)
                                    .to_lowercase()
                                    .contains(&filter)
                            })
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            }))
            .key(|v| v.name.clone())
            .children(move |item| {
                let volume = item.value;
                let select = ui.clone();
                sapid::button()
                    .class("docker-volume-row")
                    .name(volume.map(|v| v.name.clone()))
                    .enabled(state.map(ready))
                    .fill((
                        text()
                            .class("docker-resource-name")
                            .content(volume.map(|v| v.name.clone())),
                        text().content(volume.map(|v| format!("{} · {}", v.driver, v.scope))),
                        text().class("subtle").content(state.map(move |s| {
                            s.docker
                                .snapshot
                                .as_ref()
                                .map(|v| {
                                    v.containers
                                        .iter()
                                        .filter(|c| {
                                            c.mounts
                                                .split(',')
                                                .any(|m| m.trim() == volume.get().name)
                                        })
                                        .map(|c| c.name.clone())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                                .unwrap_or_default()
                        })),
                    ))
                    .on_press(move || {
                        let name = volume.get().name;
                        state.revise(|s| {
                            s.docker.selected_volume = name.clone();
                            s.docker.volume = None;
                        });
                        select.docker_send(AppAction::DockerVolume(name));
                    })
            }),
        ),
        text()
            .class("docker-volume-details")
            .selectable(true)
            .content(state.map(|s| {
                s.docker
                    .volume
                    .as_ref()
                    .map(|v| format!("{} · {} · {}\n{}", v.name, v.driver, v.scope, v.mountpoint))
                    .unwrap_or_default()
            })),
    ))
}
fn ports(ui: Ui) -> impl Widget {
    let state = ui.state;
    let listeners = ui.clone();
    col().class("docker-ports").fill((
        row().class("section-heading").fill((
            text_action("Server listeners", Icon::Server).class("button with-icon compact")
                .on_press(move || {
                    listeners.state.revise(|s| {
                        monitor_ui::set_section(s, monitor_ui::Section::Processes);
                        processes_ui::show_port_details(s, true);
                    });
                    listeners.open_desktop_app(WorkspacePage::Monitor);
                }),
            help_ui::tip("Docker ports", "Docker reports published and exposed ports. Host-network containers use server listeners directly; inspect them in Monitor → Processes → Port details."),
        )),
        each(state.map(|s| {
            s.docker
                .snapshot
                .as_ref()
                .map(|v| {
                    v.containers
                        .iter()
                        .filter(|c| {
                            (!c.ports.is_empty() || c.networks.split(',').any(|n| n.trim() == "host"))
                                && format!("{} {}", c.name, c.ports)
                                    .to_lowercase()
                                    .contains(&s.docker.filter.to_lowercase())
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }))
        .key(|c| c.id.clone())
        .children(move |item| {
            let container = item.value;
            let select = ui.clone();
            sapid::button()
                .class("docker-port-row")
                .name(container.map(|c| c.name.clone()))
                .enabled(state.map(ready))
                .fill((
                    text()
                        .class("docker-resource-name")
                        .content(container.map(|c| c.name.clone())),
                    text()
                        .class("docker-resource-ports")
                        .content(i18n::bind(container.map(|c| if c.ports.is_empty() { "Host network".into() } else { c.ports.clone() }))),
                    text().class("docker-resource-state").content(container.map(|c| c.status.clone())),
                ))
                .on_press(move || select.docker_select(container.get().id))
        }),
    ))
}
fn column_titles(labels: &[&'static str], class: &'static str) -> impl Widget + use<> {
    row().class(class).fill(
        labels
            .iter()
            .map(|&label| text().content(i18n::bind(label)))
            .collect::<Vec<_>>(),
    )
}
const SECTIONS: [&str; 4] = ["Containers", "Volumes", "Ports", "Projects"];

fn navigation(ui: Ui) -> impl Widget {
    let state = ui.state;
    let icons = [Icon::Grid, Icon::Disks, Icon::Server, Icon::Folder];
    let items = SECTIONS
        .into_iter()
        .zip(icons)
        .enumerate()
        .map(move |(index, (label, icon))| {
            let view = index as u8;
            let ui = ui.clone();
            text_action(label, icon)
                .class("button with-icon docker-nav-item")
                .pressed(state.map(move |s| s.docker.view == view))
                .enabled(state.map(ready))
                .on_press(move || {
                    state.revise(|s| {
                        s.docker.view = view;
                        s.docker.detail_open = false;
                        s.docker.filter.clear();
                    });
                    if view == 3 {
                        ui.compose_enter();
                    }
                })
        })
        .collect::<Vec<_>>();
    col().class("docker-tabs").fill(items)
}

fn toolbar(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    row().class("docker-toolbar").fill((
        when(state.map(|s| s.docker.view != 3 && !s.docker.detail_open), move || {
            row().class("docker-search-field").fill((
                icons::glyph(Icon::Search),
                text().content(i18n::bind("Search")),
                input()
                    .class("docker-filter")
                    .name(i18n::bind("Filter Docker resources"))
                    .value(state.map(|s| s.docker.filter.clone()))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event {
                            state.revise(|s| s.docker.filter = value);
                        }
                    }),
            ))
        }),
        when(state.map(|s| s.docker.view == 0 && !s.docker.detail_open), move || {
            sapid::checkbox().class("preference-check")
                .label(i18n::bind("Only running containers"))
                .checked(state.map(|s| s.docker.running_only))
                .on_checked_change(move |value| state.revise(|s| s.docker.running_only = value))
        }),
        when(state.map(|s| s.docker.view != 3), move || {
            let ui = ui.clone();
            let refresh = refresh.clone();
            fragment((
                text_action("Install software", Icon::Plus)
                    .class("button with-icon compact")
                    .enabled(state.map(|s| s.connected && !scripts_ui::busy(s)))
                    .on_press(move || ui.open_software_install(2, None)),
                icon_action("Refresh Docker", Icon::Refresh)
                    .class("icon-button")
                    .enabled(state.map(ready))
                    .on_press(move || refresh.docker_refresh()),
            ))
        }),
        help_ui::tip("Docker connection", "Uses this SSH user's local Docker socket, including rootless Docker. No automatic sudo. Start Docker or check this user's socket access if a connection error appears."),
    ))
}

pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let sidebar = navigation(ui.clone());
    let content = col().class("docker-main").fill((
        text().class("docker-section-title").content(i18n::bind(
            state.map(|s| SECTIONS[s.docker.view as usize].to_owned()),
        )),
        toolbar(ui.clone()),
        text()
            .class("docker-summary subtle")
            .content(i18n::bind(state.map(|s| {
                s.docker
                    .snapshot
                    .as_ref()
                    .map(|v| {
                        format!(
                            "Docker {} · {} containers · {} volumes",
                            v.version,
                            v.containers.len(),
                            v.volumes.len()
                        )
                    })
                    .unwrap_or_default()
            }))),
        text()
            .class("docker-status subtle")
            .content(i18n::bind(state.map(|s| {
                if s.docker.pending.is_some() {
                    "Loading…".into()
                } else {
                    s.docker.status.clone()
                }
            }))),
        text()
            .class("docker-error error")
            .content(state.map(|s| s.docker.error.clone())),
        text()
            .class(state.map(|s| {
                if hint(s).is_some() {
                    "docker-error-hint hint"
                } else {
                    "hidden"
                }
                .to_owned()
            }))
            .selectable(true)
            .content(state.map(|s| hint(s).unwrap_or_default())),
        when(
            state.map(|s| {
                s.docker.view > 0
                    && s.docker.view < 3
                    && s.docker.snapshot.as_ref().is_some_and(|v| {
                        if s.docker.view == 1 {
                            v.volumes.is_empty()
                        } else {
                            v.containers.is_empty()
                        }
                    })
            }),
            || {
                text()
                    .class("empty-copy")
                    .content(i18n::bind("No Docker resources"))
            },
        ),
        when(
            state.map(|s| s.docker.view == 0 && !s.docker.detail_open),
            {
                let ui = ui.clone();
                move || {
                    col().class("docker-inventory").fill((
                        column_titles(
                            &["Name", "Status", "Container image", "Ports"],
                            "docker-container-head docker-columns",
                        ),
                        container_list(ui.clone()),
                        text()
                            .class("docker-no-matches subtle")
                            .content(i18n::bind("No matching containers")),
                    ))
                }
            },
        ),
        when(state.map(|s| s.docker.view == 0 && s.docker.detail_open), {
            let ui = ui.clone();
            move || detail(ui.clone())
        }),
        when(state.map(|s| s.docker.view == 1), {
            let ui = ui.clone();
            move || volumes(ui.clone())
        }),
        when(state.map(|s| s.docker.view == 2), {
            let ui = ui.clone();
            move || ports(ui.clone())
        }),
        when(state.map(|s| s.docker.view == 3), move || {
            compose_ui::projects(ui.clone())
        }),
    ));
    row().class("docker-page").fill((sidebar, content))
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        when(state.map(|s| s.dialog == Some(Dialog::DockerAction)), {
            let ui = ui.clone();
            move || {
                let confirm = ui.clone();
                modal(
                    ui.clone(),
                    state.with(|s| {
                        s.docker
                            .target
                            .as_ref()
                            .map(|t| t.1.label())
                            .unwrap_or("Docker")
                    }),
                    col().class("docker-action-review").fill((
                        text().class("field-label").content(state.map(|s| {
                            s.docker
                                .target
                                .as_ref()
                                .map(|t| t.0.name.clone())
                                .unwrap_or_default()
                        })),
                        text()
                            .class("subtle")
                            .selectable(true)
                            .content(state.map(|s| {
                                s.docker
                                    .target
                                    .as_ref()
                                    .map(|t| t.0.id.clone())
                                    .unwrap_or_default()
                            })),
                        text().class("subtle").content(state.map(|s| {
                            s.config
                                .servers
                                .iter()
                                .find(|p| Some(&p.id) == s.selected.as_ref())
                                .map(|p| format!("{} @ {}", p.username, p.host))
                                .unwrap_or_default()
                        })),
                        text()
                            .class("error")
                            .content(state.map(|s| s.docker.error.clone())),
                    )),
                    sapid::button()
                        .class("button primary docker-confirm")
                        .label(i18n::bind("Confirm"))
                        .enabled(state.map(actionable))
                        .on_press(move || confirm.docker_operate()),
                )
            }
        }),
        when(
            state.map(|s| s.dialog == Some(Dialog::DockerConfigure)),
            move || {
                let save = ui.clone();
                modal(ui.clone(), "Configure container", col().class("docker-config-form").fill((
                text().class("field-label").content(state.map(|s| s.docker.target.as_ref().map(|t| t.0.name.clone()).unwrap_or_default())),
                help_ui::tip("Container configuration", "Restart policy: no, always, unless-stopped or on-failure[:retries]. CPU and memory limits use 0 for unlimited. Changes apply to this container; Compose may replace them when it recreates the container."),
                field("Restart policy", &ui, |s| s.docker.restart.clone(), |s,v| s.docker.restart = v),
                field("CPU limit", &ui, |s| s.docker.cpu.clone(), |s,v| s.docker.cpu = v),
                field("Memory limit (MiB)", &ui, |s| s.docker.memory.clone(), |s,v| s.docker.memory = v),
                text().class("error").content(state.map(|s| s.docker.error.clone())),
            )), sapid::button().class("button primary docker-config-save").label(i18n::bind("Save to server")).enabled(state.map(actionable)).on_press(move || save.docker_save_config()))
            },
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_input_is_decimal_and_bounded_before_shell_generation() {
        assert_eq!(parse_cpu("1.5").unwrap(), 1500);
        assert_eq!(parse_cpu("0.01").unwrap(), 10);
        for invalid in ["NaN", "inf", "1; id", "-1", "1.0001", "999999999999"] {
            assert!(parse_cpu(invalid).is_err());
        }
    }

    #[test]
    fn socket_permission_errors_suggest_the_docker_group() {
        for error in [
            "Docker command failed (1): permission denied while trying to connect to the docker API at unix:///var/run/docker.sock",
            "Docker command failed (1): Got permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock: Get \"http://%2Fvar%2Frun%2Fdocker.sock/v1.24/version\": dial unix /var/run/docker.sock: connect: permission denied",
        ] {
            let hint = socket_hint(error, "deploy").unwrap();
            assert!(
                hint.contains("sudo usermod -aG docker -- 'deploy'"),
                "{hint}"
            );
        }
        for error in [
            "",
            "Docker command failed (1): No such container",
            "Docker command failed (1): open /srv/data: permission denied",
            "Docker command failed (1): permission denied while trying to connect to the docker API at unix:///run/user/1000/docker.sock",
            "Docker command failed (1): permission denied while trying to connect to the docker API at unix:///home/deploy/custom-runtime/docker.sock",
        ] {
            assert_eq!(socket_hint(error, "deploy"), None, "{error}");
        }
        let error =
            "permission denied connecting to the Docker socket at unix:///var/run/docker.sock";
        let hint = socket_hint(error, "o'neil").unwrap();
        assert!(
            hint.contains("sudo usermod -aG docker -- 'o'\\''neil'"),
            "{hint}"
        );
    }
    #[test]
    fn logs_keep_colors_and_mute_docker_timestamps() {
        let logs = "2026-11-18T09:15:02.123456789Z \x1b[32mINFO\x1b[0m ready\r\n2026-11-18T09:15:03.000000000Z plain\n";
        let spans = log_spans(logs, "");
        let parts: Vec<_> = spans
            .iter()
            .map(|s| (s.text.as_str(), s.class.as_str()))
            .collect();
        assert_eq!(
            parts,
            [
                ("2026-11-18T09:15:02.123456789Z ", "docker-log-time"),
                ("INFO", "ansi ansi-fg-2"),
                (" ready\n", ""),
                ("2026-11-18T09:15:03.000000000Z ", "docker-log-time"),
                ("plain\n", ""),
            ]
        );
    }
    #[test]
    fn log_redraws_and_empty_lines_keep_the_line_structure() {
        let logs = "2026-11-18T09:15:02.1Z 10%\r100%\n\nno timestamp: here\n";
        let text: String = log_spans(logs, "").into_iter().map(|s| s.text).collect();
        assert_eq!(text, "2026-11-18T09:15:02.1Z 100%\n\nno timestamp: here\n");
    }
    #[test]
    fn the_filter_keeps_matching_lines_and_the_colors_before_them() {
        let logs = "2026-11-18T09:15:01Z \x1b[31mERR\x1b[0mOR one\n2026-11-18T09:15:02Z \x1b[32mhidden\n2026-11-18T09:15:03Z green on\n";
        let parts = |filter| {
            log_spans(logs, filter)
                .into_iter()
                .map(|s| (s.text, s.class))
                .collect::<Vec<_>>()
        };
        // The filter matches the text as shown, timestamps included.
        assert_eq!(
            parts("error"),
            [
                ("2026-11-18T09:15:01Z ".into(), "docker-log-time".into()),
                ("ERR".into(), "ansi ansi-fg-1".into()),
                ("OR one\n".into(), String::new()),
            ]
        );
        assert_eq!(parts("09:15:03")[0].0, "2026-11-18T09:15:03Z ");
        // Green set in the hidden line still applies, as in a terminal.
        assert_eq!(
            parts("GREEN")[1],
            ("green on\n".into(), "ansi ansi-fg-2".into())
        );
        assert!(parts("[31m").is_empty());
        assert!(parts("nothing").is_empty());
    }
    #[test]
    fn logs_without_visible_output_read_as_no_logs() {
        for logs in ["", "\n", "\x1b[0m\n", "\r\n\n"] {
            assert!(log_spans(logs, "").is_empty(), "{logs:?}");
        }
        assert!(!log_spans("2026-11-18T09:15:02.1Z \n", "").is_empty());
    }
}
