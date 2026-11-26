use arcnix_core::docker::{Container, ContainerDetails, Operation, Snapshot, VolumeDetails};

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{icon_action, text_action};
use crate::server_apps_worker::{Action as AppAction, Reply};
use crate::*;

mod cleanup;
mod login;
mod mirrors;
mod networks;
mod pull;
mod run;
mod updates;

#[derive(Clone, Default)]
pub(crate) struct DockerState {
    pub(crate) mirrors: mirrors::MirrorState,
    pub(crate) pull: pull::PullState,
    pub(crate) run: run::RunState,
    pub(crate) updates: updates::UpdatesState,
    pub(crate) cleanup: cleanup::CleanupState,
    pub(crate) login: login::LoginState,
    pub(crate) networks: networks::NetworkState,
    snapshot: Option<Snapshot>,
    images: Vec<docker::Image>,
    stats: Option<docker::Stats>,
    selected: String,
    selected_volume: String,
    details: Option<ContainerDetails>,
    volume: Option<VolumeDetails>,
    pending: Option<u64>,
    pub(crate) view: u8,
    filter: String,
    running_only: bool,
    collapsed_projects: std::collections::HashSet<String>,
    detail_open: bool,
    detail_tab: DetailTab,
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
impl DockerState {
    pub(crate) fn updates_checking(&self) -> bool {
        self.updates.checking()
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum DetailTab {
    #[default]
    Inspect,
    Logs,
    Mounts,
    Stats,
}
use crate::log_text::{LogSpan, Spans};
/// The logs from one reply. The text is shared so that cloning the state stays
/// cheap; it is turned into spans only when the reply, filter or language
/// changes, not on every state change.
#[derive(Clone, Default)]
pub(crate) struct LogView {
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
pub(crate) fn ready(s: &State) -> bool {
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
fn select_container(s: &mut State, id: String) {
    s.docker.selected = id;
    s.docker.detail_open = true;
    s.docker.detail_tab = DetailTab::Inspect;
    s.docker.details = None;
    s.docker.stats = None;
    s.docker.logs = None;
    s.docker.log_filter.clear();
    s.docker.view = 0;
}
/// Advice for the CLI's error when it may not use the system Docker socket,
/// which usually means the SSH account is not in the docker group.
fn socket_hint(error: &str, user: &str) -> Option<String> {
    let error = error.to_lowercase();
    let system_socket = error.split_ascii_whitespace().any(|word| {
        matches!(
            word.trim_matches(['\'', '"', ':', ',', '.']),
            "unix:///var/run/docker.sock" | "unix:///run/docker.sock"
        )
    });
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
    pub(crate) fn docker_enter(&self) {
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
        match self.state.with(|s| s.docker.view) {
            3 => self.compose_refresh(),
            4 => self.docker_send(AppAction::DockerImages),
            5 => self.docker_networks_refresh(),
            _ => self.docker_send(AppAction::DockerSnapshot),
        }
    }
    fn docker_select(&self, id: String) {
        if !self.state.with(ready) {
            return;
        }
        self.state.revise(|s| select_container(s, id.clone()));
        self.docker_send(AppAction::DockerInspect(id));
    }
    pub(crate) fn open_docker_container(&self, id: String) {
        if self.state.with(ready) {
            self.state.revise(|s| select_container(s, id));
            self.docker_send(AppAction::DockerSnapshot);
            self.open_desktop_app(WorkspacePage::Docker);
        }
    }
    fn docker_logs(&self) {
        if !self.state.with(actionable) {
            return;
        }
        let id = self.state.with(|s| s.docker.selected.clone());
        let lines = self.state.with(log_lines);
        self.state.revise(|s| {
            s.docker.detail_tab = DetailTab::Logs;
            s.docker.logs = Some(LogView::default());
        });
        self.docker_send(AppAction::DockerLogs(id, lines));
    }
    fn docker_stats(&self) {
        if self.state.with(actionable) {
            let id = self.state.revise(|s| {
                s.docker.detail_tab = DetailTab::Stats;
                s.docker.selected.clone()
            });
            self.docker_send(AppAction::DockerStats(id));
        }
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

pub(crate) fn event(ui: &Ui, serial: u64, result: std::result::Result<Reply, String>) {
    let Some(result) = mirrors::event(ui, serial, result) else {
        return;
    };
    let Some(result) = pull::event(ui, serial, result) else {
        return;
    };
    let Some(result) = run::event(ui, serial, result) else {
        return;
    };
    let Some(result) = cleanup::event(ui, serial, result) else {
        return;
    };
    let Some(result) = login::event(ui, serial, result) else {
        return;
    };
    let Some(result) = networks::event(ui, serial, result) else {
        return;
    };
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
            Ok(Reply::Images(images)) => s.docker.images = images,
            Ok(Reply::ContainerStats(stats)) => s.docker.stats = Some(stats),
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
        if ui.state.with(|s| s.software.app_center.inventory.is_some()) {
            ui.refresh_software_inventory();
        }
    } else if let Some(id) = select_after {
        ui.docker_select(id);
    }
}

/// The colour a container's state takes, as a class suffix.
fn tone(container: &Container) -> &'static str {
    match container.state.as_str() {
        "running" if container.status.contains("(unhealthy)") => "warning",
        "running" => "running",
        "paused" => "paused",
        "restarting" | "removing" => "busy",
        "dead" => "failed",
        "exited" if exit_code(&container.status).is_some_and(|code| code != 0) => "failed",
        _ => "stopped",
    }
}
/// The code in Docker's `Exited (1) 12 minutes ago` status.
fn exit_code(status: &str) -> Option<i32> {
    status
        .strip_prefix("Exited (")?
        .split_once(')')?
        .0
        .parse()
        .ok()
}
/// A container's status in the colour of its state.
fn pill<T: 'static>(value: Cage<T>, container: fn(&T) -> &Container) -> impl Widget + use<T> {
    text()
        .class(
            value
                .map(move |v| format!("docker-state docker-resource-state {}", tone(container(v)))),
        )
        .content(value.map(move |v| container(v).status.clone()))
}
/// `docker ps` ports as short entries. A wildcard host address is left out,
/// so `0.0.0.0:80->80/tcp, [::]:80->80/tcp` reads once as `80 → 80/tcp`.
/// Entries without a host binding are exposed only and marked as such.
fn port_entries(ports: &str) -> Vec<(String, bool)> {
    let mut entries: Vec<(String, bool)> = Vec::new();
    for entry in ports.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let item = match entry.split_once("->") {
            Some((host, target)) => {
                let (address, port) = host.rsplit_once(':').unwrap_or(("", host));
                if matches!(address, "" | "0.0.0.0" | "::" | "[::]") {
                    (format!("{port} → {target}"), true)
                } else {
                    (format!("{address}:{port} → {target}"), true)
                }
            }
            None => (entry.to_owned(), false),
        };
        if !entries.contains(&item) {
            entries.push(item);
        }
    }
    entries
}
fn port_chips(ports: Bond<Vec<(String, bool)>>) -> impl Widget {
    row()
        .class("docker-port-chips")
        .fill(
            each(ports)
                .key(|(label, _)| label.clone())
                .children(|item| {
                    let (label, published) = item.value.get();
                    text()
                        .class(if published {
                            "docker-chip"
                        } else {
                            "docker-chip exposed"
                        })
                        .content(label)
                }),
        )
}
fn container_button(
    ui: &Ui,
    label: &'static str,
    icon: Icon,
    operation: Operation,
) -> impl Widget + use<> {
    let state = ui.state;
    let ui = ui.clone();
    let shown = operation.clone();
    let remove = operation == Operation::Remove;
    // Only the operations that fit the container's state are offered. They
    // stay visible but disabled while Docker is busy or unavailable.
    text_action(label, icon)
        .class(state.map(move |s| {
            let fits = selected(s).is_some_and(|c| match shown {
                Operation::Start => matches!(c.state.as_str(), "exited" | "created"),
                Operation::Stop => matches!(c.state.as_str(), "running" | "restarting"),
                Operation::Restart => c.state != "paused",
                Operation::Remove => matches!(c.state.as_str(), "exited" | "created" | "dead"),
                Operation::Pause => c.state == "running",
                Operation::Resume => c.state == "paused",
                _ => true,
            });
            format!(
                "button with-icon compact{}{}",
                if remove { " docker-remove" } else { "" },
                if fits { "" } else { " hidden" }
            )
        }))
        .enabled(state.map(actionable))
        .on_press(move || ui.docker_prepare(operation.clone()))
}
#[derive(Clone)]
enum ContainerRow {
    Project(String, usize, usize),
    Container(Box<Container>),
}
impl ContainerRow {
    fn container(&self) -> &Container {
        match self {
            Self::Container(container) => container,
            Self::Project(..) => unreachable!("Container row key changed"),
        }
    }
}

fn container_rows(s: &State) -> Vec<ContainerRow> {
    let Some(snapshot) = &s.docker.snapshot else {
        return Vec::new();
    };
    let filter = s.docker.filter.to_lowercase();
    let mut rows = Vec::new();
    let mut projects = std::collections::BTreeMap::<&str, Vec<&Container>>::new();
    for container in snapshot.containers.iter().filter(|c| {
        (!s.docker.running_only || c.state == "running")
            && [
                &c.name,
                &c.image,
                &c.state,
                &c.ports,
                &c.compose_project,
                &c.compose_service,
            ]
            .into_iter()
            .any(|value| value.to_lowercase().contains(&filter))
    }) {
        if container.compose_project.is_empty() {
            rows.push(ContainerRow::Container(Box::new(container.clone())));
        } else {
            projects
                .entry(&container.compose_project)
                .or_default()
                .push(container);
        }
    }
    for (name, containers) in projects {
        let running = containers.iter().filter(|c| c.state == "running").count();
        rows.push(ContainerRow::Project(
            name.to_owned(),
            running,
            containers.len(),
        ));
        if !s.docker.collapsed_projects.contains(name) {
            rows.extend(
                containers
                    .into_iter()
                    .cloned()
                    .map(Box::new)
                    .map(ContainerRow::Container),
            );
        }
    }
    rows
}

fn container_list(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("docker-container-list").fill(
        each(state.map(container_rows))
            .key(|row| match row {
                ContainerRow::Container(c) => (false, c.id.clone()),
                ContainerRow::Project(name, ..) => (true, name.clone()),
            })
            .children(move |item| {
                let initial = item.value.get();
                if let ContainerRow::Project(name, ..) = initial {
                    let pressed_name = name.clone();
                    let counts = item.value;
                    return fragment(
                        sapid::button()
                            .class("button docker-project-group")
                            .name(name.clone())
                            .fill((
                                icons::glyph(Icon::Collapse),
                                text().class("docker-project-name").content(name.clone()),
                                text().class("docker-project-kind").content("Compose"),
                                text().class("docker-project-count").content(i18n::bind(
                                    counts.map(|row| match row {
                                        ContainerRow::Project(_, running, total) => {
                                            format!("{running} of {total} running")
                                        }
                                        ContainerRow::Container(_) => String::new(),
                                    }),
                                )),
                            ))
                            .pressed(
                                state.map(move |s| {
                                    !s.docker.collapsed_projects.contains(&pressed_name)
                                }),
                            )
                            .on_press(move || {
                                state.revise(|s| {
                                    if !s.docker.collapsed_projects.remove(&name) {
                                        s.docker.collapsed_projects.insert(name.clone());
                                    }
                                })
                            }),
                    );
                }
                let container = item.value;
                let select = ui.clone();
                fragment(
                    row()
                        .class(state.map(move |s| {
                            let row = container.get();
                            let c = row.container();
                            format!(
                                "docker-container-row{}{}",
                                if c.compose_project.is_empty() {
                                    ""
                                } else {
                                    " compose-member"
                                },
                                if s.docker.selected == c.id {
                                    " selected"
                                } else {
                                    ""
                                }
                            )
                        }))
                        .fill((
                            // The button covers the row, so any part of it opens the container.
                            sapid::button()
                                .class("docker-row-open")
                                .name(container.map(|row| row.container().name.clone()))
                                .enabled(state.map(ready))
                                .pressed(state.map(move |s| {
                                    s.docker.selected == container.get().container().id
                                }))
                                .fill((
                                    text()
                                        .class("docker-resource-name")
                                        .content(container.map(|row| row.container().name.clone())),
                                    text().class("docker-resource-image").content(
                                        container.map(|row| row.container().image.clone()),
                                    ),
                                    text()
                                        .class(state.map(move |s| {
                                            if updates::newer(s, &container.get().container().image)
                                            {
                                                "docker-badge update"
                                            } else {
                                                "hidden"
                                            }
                                            .to_owned()
                                        }))
                                        .content(i18n::bind("Update available")),
                                ))
                                .on_press(move || {
                                    select.docker_select(container.get().container().id.clone())
                                }),
                            pill(container, ContainerRow::container),
                            port_chips(container.map(|row| port_entries(&row.container().ports))),
                        )),
                )
            }),
    )
}
/// A labelled value in a detail list.
fn fact(label: &'static str, value: impl Widget) -> impl Widget {
    row().class("docker-fact").fill((
        text().class("docker-fact-label").content(i18n::bind(label)),
        value,
    ))
}
fn fact_text(
    state: Cage<State>,
    label: &'static str,
    value: impl Fn(&State) -> String + 'static,
) -> impl Widget {
    fact(
        label,
        text()
            .class("docker-fact-value")
            .selectable(true)
            .content(state.map(value)),
    )
}
fn limit(value: u64, unit: &str) -> String {
    if value == 0 {
        i18n::translate("Unlimited")
    } else {
        format!("{value}{unit}")
    }
}
fn restart_policy(details: &ContainerDetails) -> String {
    if details.restart == "on-failure" && details.retries > 0 {
        format!("on-failure:{}", details.retries)
    } else {
        details.restart.clone()
    }
}
/// One row per port: the address in use when Docker published it, the
/// configured binding while the container is stopped, or the exposed port.
fn port_bindings(details: &ContainerDetails) -> Vec<(String, &'static str)> {
    let mut keys: Vec<String> = Vec::new();
    for value in [&details.ports, &details.bindings] {
        for key in value.as_object().into_iter().flat_map(|ports| ports.keys()) {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    keys.sort();
    let bindings = |value: &serde_json::Value, key: &str| {
        value[key]
            .as_array()
            .map(|bindings| {
                bindings
                    .iter()
                    .map(|binding| {
                        let host = binding["HostIp"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("*");
                        let host = if host.contains(':') {
                            format!("[{host}]")
                        } else {
                            host.to_owned()
                        };
                        format!(
                            "{host}:{} → {key}",
                            binding["HostPort"].as_str().unwrap_or("?")
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let mut rows = Vec::new();
    for key in keys {
        let published = bindings(&details.ports, &key);
        if !published.is_empty() {
            rows.extend(published.into_iter().map(|b| (b, "Published")));
            continue;
        }
        let configured = bindings(&details.bindings, &key);
        if !configured.is_empty() {
            rows.extend(configured.into_iter().map(|b| (b, "Configured")));
        } else if details.ports.get(&key).is_some() {
            rows.push((key, "Exposed only"));
        }
    }
    rows
}
fn mounts(details: &ContainerDetails) -> Vec<[String; 4]> {
    details
        .mounts
        .as_array()
        .map(|mounts| {
            mounts
                .iter()
                .map(|m| {
                    [
                        m["Type"].as_str().unwrap_or("?").to_owned(),
                        m["Source"].as_str().unwrap_or("?").to_owned(),
                        m["Destination"].as_str().unwrap_or("?").to_owned(),
                        if m["RW"].as_bool() == Some(true) {
                            "Read-write".to_owned()
                        } else {
                            "Read-only".to_owned()
                        },
                    ]
                })
                .collect()
        })
        .unwrap_or_default()
}
fn inspection(state: Cage<State>) -> impl Widget {
    let details = |s: &State| {
        s.docker
            .details
            .as_ref()
            .filter(|d| d.id == s.docker.selected)
            .cloned()
    };
    col().class("docker-inspection").fill((
        col().class("docker-facts").fill((
            fact(
                "Container ID",
                text()
                    .class("docker-fact-value docker-container-id")
                    .selectable(true)
                    .content(state.map(|s| s.docker.selected.clone())),
            ),
            fact_text(state, "Network", move |s| {
                details(s).map(|d| d.network).unwrap_or_default()
            }),
        )),
        row().class("section-heading").fill((
            text().content(i18n::bind("Container configuration")),
            help_ui::tip("Container configuration", "Restart policy: no, always, unless-stopped or on-failure[:retries]. CPU and memory limits use 0 for unlimited. Changes apply to this container; Compose may replace them when it recreates the container."),
        )),
        col().class("docker-facts docker-configuration-summary").fill((
            fact_text(state, "Restart policy", move |s| {
                details(s).map(|d| restart_policy(&d)).unwrap_or_default()
            }),
            fact_text(state, "CPU limit", move |s| {
                details(s)
                    .map(|d| {
                        if d.nano_cpus == 0 {
                            i18n::translate("Unlimited")
                        } else {
                            format!("{}", d.nano_cpus as f64 / 1_000_000_000.0)
                        }
                    })
                    .unwrap_or_default()
            }),
            fact_text(state, "Memory limit", move |s| {
                details(s)
                    .map(|d| limit(d.memory / (1024 * 1024), " MiB"))
                    .unwrap_or_default()
            }),
        )),
        row().class("section-heading").fill((
            text().content(i18n::bind("Port bindings")),
            help_ui::tip("Port bindings", "Published mappings show host addresses and ports. Exposed ports without a host binding are not published. Host networking uses the server network directly. Changing mappings or mounts requires recreating the container."),
        )),
        col().class("docker-port-details docker-table").fill(
            each(state.map(move |s| details(s).map(|d| port_bindings(&d)).unwrap_or_default()))
                .key(Clone::clone)
                .children(|item| {
                    let (binding, kind) = item.value.get();
                    row().class("docker-binding").fill((
                        text().class("docker-binding-address").selectable(true).content(binding),
                        text()
                            .class(if kind == "Published" {
                                "docker-badge published"
                            } else {
                                "docker-badge"
                            })
                            .content(i18n::bind(kind)),
                    ))
                }),
        ),
        when(
            state.map(move |s| details(s).is_some_and(|d| port_bindings(&d).is_empty())),
            || text().class("docker-empty-note").content(i18n::bind("No ports are exposed")),
        ),
    ))
}
fn mount_list(state: Cage<State>) -> impl Widget {
    let mounts_of = |s: &State| {
        s.docker
            .details
            .as_ref()
            .filter(|d| d.id == s.docker.selected)
            .map(mounts)
            .unwrap_or_default()
    };
    col().class("docker-inspection").fill((
        col().class("docker-mount-details docker-table").fill(
            each(state.map(mounts_of))
                .key(Clone::clone)
                .children(|item| {
                    let [kind, source, destination, mode] = item.value.get();
                    row().class("docker-mount").fill((
                        text().class("docker-badge").content(kind),
                        col().class("docker-mount-paths").fill((
                            text()
                                .class("docker-mount-destination")
                                .selectable(true)
                                .content(destination),
                            text()
                                .class("docker-mount-source")
                                .selectable(true)
                                .content(source),
                        )),
                        text()
                            .class(if mode == "Read-write" {
                                "docker-badge"
                            } else {
                                "docker-badge read-only"
                            })
                            .content(i18n::bind(mode)),
                    ))
                }),
        ),
        when(
            state.map(move |s| {
                s.docker
                    .details
                    .as_ref()
                    .is_some_and(|d| d.id == s.docker.selected)
                    && mounts_of(s).is_empty()
            }),
            || {
                text()
                    .class("docker-empty-note")
                    .content(i18n::bind("No mounts"))
            },
        ),
    ))
}
fn detail(ui: Ui) -> impl Widget {
    let state = ui.state;
    let configure = ui.clone();
    let logs = ui.clone();
    let stats = ui.clone();
    let refresh_stats = ui.clone();
    col().class("docker-detail").fill((
        col().class("docker-detail-title").fill((
            row().class("docker-detail-heading").fill((
                text().class("docker-detail-name").content(state.map(|s| selected(s).map(|c| c.name).unwrap_or_default())),
                text()
                    .class(state.map(|s| format!("docker-state {}", selected(s).as_ref().map_or("stopped", tone))))
                    .content(state.map(|s| selected(s).map(|c| c.status).unwrap_or_default())),
            )),
            text().class("docker-detail-image").selectable(true).content(state.map(|s| selected(s).map(|c| c.image).unwrap_or_default())),
        )),
        row().class("docker-container-actions").fill((
            container_button(&ui, "Start", Icon::Play, Operation::Start),
            container_button(&ui, "Stop", Icon::Stop, Operation::Stop),
            container_button(&ui, "Restart", Icon::Refresh, Operation::Restart),
            container_button(&ui, "Pause", Icon::Pause, Operation::Pause),
            container_button(&ui, "Resume", Icon::Play, Operation::Resume),
            text_action("Configure", Icon::Settings).class("button with-icon compact").enabled(state.map(|s| actionable(s) && s.docker.details.as_ref().is_some_and(|d| d.id == s.docker.selected))).on_press(move || configure.docker_configure()),
            container_button(&ui, "Remove…", Icon::Clear, Operation::Remove),
            {
                let shell = ui.clone();
                text_action("Shell", Icon::Terminal)
                    .class(state.map(|s| {
                        if selected(s).is_some_and(|c| c.state == "running") {
                            "button with-icon compact"
                        } else {
                            "hidden"
                        }
                        .to_owned()
                    }))
                    .enabled(state.map(networks::can_shell))
                    .on_press(move || shell.docker_shell())
            },
        )),
        row().class("docker-detail-tabs").fill((
            btn("Inspect", move || state.revise(|s| s.docker.detail_tab = DetailTab::Inspect))
                .pressed(state.map(|s| s.docker.detail_tab == DetailTab::Inspect)),
            btn("Logs", move || logs.docker_logs()).enabled(state.map(actionable))
                .pressed(state.map(|s| s.docker.detail_tab == DetailTab::Logs)),
            btn("Mounts", move || state.revise(|s| s.docker.detail_tab = DetailTab::Mounts))
                .pressed(state.map(|s| s.docker.detail_tab == DetailTab::Mounts)),
            btn("Stats", move || stats.docker_stats())
                .enabled(state.map(|s| actionable(s) && selected(s).is_some_and(|c| c.state == "running")))
                .pressed(state.map(|s| s.docker.detail_tab == DetailTab::Stats)),
        )),
        when(state.map(|s| s.docker.detail_tab == DetailTab::Logs), {
            let ui = ui.clone();
            move || {
                col().class("docker-log-panel").fill((
                    row().class("docker-log-tools").fill((
                        row().class("docker-log-lines").fill((
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
                        )),
                        row().class("docker-log-search").fill((
                            icons::glyph(Icon::Filter),
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
                        help_ui::tip("Recent logs", "The last 200, 500 or 1000 lines, fetched on demand. Container stdout and stderr are included, with their terminal colors. No background log process."),
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
        }),
        when(state.map(|s| s.docker.detail_tab == DetailTab::Inspect), move || inspection(state)),
        when(state.map(|s| s.docker.detail_tab == DetailTab::Mounts), move || mount_list(state)),
        when(state.map(|s| s.docker.detail_tab == DetailTab::Stats), move || {
            let refresh = refresh_stats.clone();
            type Field = (&'static str, fn(&docker::Stats) -> &str);
            let fields: [Field; 5] = [
                ("CPU", |stats| &stats.cpu),
                ("Memory", |stats| &stats.memory),
                ("Network I/O", |stats| &stats.network),
                ("Block I/O", |stats| &stats.block),
                ("Processes", |stats| &stats.processes),
            ];
            col().class("docker-inspection").fill((
                row().class("section-heading").fill((
                    text().content(i18n::bind("Resource usage")),
                    icon_action("Refresh container stats", Icon::Refresh)
                        .enabled(state.map(actionable)).on_press(move || refresh.docker_stats()),
                )),
                row().class("docker-container-stats").fill(fields.into_iter().map(|(label, value)| {
                    col().class("docker-stat").fill((
                        text().class("docker-stat-label").content(i18n::bind(label)),
                        text().class("docker-stat-value").selectable(true).content(state.map(move |s| {
                            s.docker.stats.as_ref().map(value).unwrap_or("–").to_owned()
                        })),
                    ))
                }).collect::<Vec<_>>()),
            ))
        }),
    ))
}
/// The containers that mount each volume, by name.
fn volume_users(s: &State, volume: &str) -> Vec<String> {
    s.docker
        .snapshot
        .as_ref()
        .map(|v| {
            v.containers
                .iter()
                .filter(|c| c.mounts.split(',').any(|m| m.trim() == volume))
                .map(|c| c.name.clone())
                .collect()
        })
        .unwrap_or_default()
}
fn volumes(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("docker-volumes").fill((
        col().class("docker-table").fill((
            column_titles(
                &["Name", "Driver", "Used by"],
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
                    row()
                        .class(state.map(move |s| {
                            if s.docker.selected_volume == volume.get().name {
                                "docker-volume-row selected"
                            } else {
                                "docker-volume-row"
                            }
                            .to_owned()
                        }))
                        .fill((
                            sapid::button()
                                .class("docker-row-open")
                                .name(volume.map(|v| v.name.clone()))
                                .enabled(state.map(ready))
                                .pressed(
                                    state.map(move |s| {
                                        s.docker.selected_volume == volume.get().name
                                    }),
                                )
                                .fill(
                                    text()
                                        .class("docker-resource-name")
                                        .content(volume.map(|v| v.name.clone())),
                                )
                                .on_press(move || {
                                    let name = volume.get().name;
                                    state.revise(|s| {
                                        s.docker.selected_volume = name.clone();
                                        s.docker.volume = None;
                                    });
                                    select.docker_send(AppAction::DockerVolume(name));
                                }),
                            text()
                                .class("docker-volume-driver")
                                .content(volume.map(|v| {
                                    if v.scope == "local" || v.scope.is_empty() {
                                        v.driver.clone()
                                    } else {
                                        format!("{} · {}", v.driver, v.scope)
                                    }
                                })),
                            row().class("docker-port-chips").fill((
                                each(state.map(move |s| volume_users(s, &volume.get().name)))
                                    .key(Clone::clone)
                                    .children(|item| {
                                        text().class("docker-chip").content(item.value.get())
                                    }),
                                when(
                                    state.map(move |s| {
                                        volume_users(s, &volume.get().name).is_empty()
                                    }),
                                    || text().class("docker-unused").content(i18n::bind("Unused")),
                                ),
                            )),
                        ))
                }),
            ),
        )),
        when(state.map(|s| s.docker.volume.is_some()), move || {
            col().class("docker-volume-details docker-facts").fill((
                fact_text(state, "Name", |s| {
                    s.docker
                        .volume
                        .as_ref()
                        .map(|v| v.name.clone())
                        .unwrap_or_default()
                }),
                fact_text(state, "Driver", |s| {
                    s.docker
                        .volume
                        .as_ref()
                        .map(|v| format!("{} · {}", v.driver, v.scope))
                        .unwrap_or_default()
                }),
                fact_text(state, "Mount point", |s| {
                    s.docker
                        .volume
                        .as_ref()
                        .map(|v| v.mountpoint.clone())
                        .unwrap_or_default()
                }),
            ))
        }),
    ))
}
fn ports(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("docker-ports").fill(
        col().class("docker-table").fill((
            column_titles(
                &["Container", "Ports", "Status"],
                "docker-port-head docker-columns",
            ),
            each(state.map(|s| {
                s.docker
                    .snapshot
                    .as_ref()
                    .map(|v| {
                        v.containers
                            .iter()
                            .filter(|c| {
                                (!c.ports.is_empty()
                                    || c.networks.split(',').any(|n| n.trim() == "host"))
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
                row().class("docker-port-row").fill((
                    sapid::button()
                        .class("docker-row-open")
                        .name(container.map(|c| c.name.clone()))
                        .enabled(state.map(ready))
                        .fill(
                            text()
                                .class("docker-resource-name")
                                .content(container.map(|c| c.name.clone())),
                        )
                        .on_press(move || select.docker_select(container.get().id)),
                    port_chips(container.map(|c| {
                        if c.ports.is_empty() {
                            vec![(i18n::translate("Host network"), true)]
                        } else {
                            port_entries(&c.ports)
                        }
                    })),
                    pill(container, |c| c),
                ))
            }),
        )),
    )
}
fn column_titles(labels: &[&'static str], class: &'static str) -> impl Widget + use<> {
    row().class(class).fill(
        labels
            .iter()
            .map(|&label| text().content(i18n::bind(label)))
            .collect::<Vec<_>>(),
    )
}
const SECTIONS: [&str; 6] = [
    "Containers",
    "Volumes",
    "Ports",
    "Projects",
    "Container images",
    "Networks",
];
/// How many items a section lists, once Docker has reported them.
fn section_count(s: &State, view: u8) -> String {
    let snapshot = s.docker.snapshot.as_ref();
    let count = match view {
        0 => snapshot.map(|v| v.containers.len()),
        1 => snapshot.map(|v| v.volumes.len()),
        2 => snapshot.map(|v| {
            v.containers
                .iter()
                .filter(|c| {
                    !c.ports.is_empty() || c.networks.split(',').any(|n| n.trim() == "host")
                })
                .count()
        }),
        3 => compose_ui::project_count(s),
        4 => (!s.docker.images.is_empty()).then_some(s.docker.images.len()),
        _ => networks::count(s),
    };
    count.map(|n| n.to_string()).unwrap_or_default()
}

fn navigation(ui: Ui) -> impl Widget {
    let state = ui.state;
    let icons = [
        Icon::Container,
        Icon::Disks,
        Icon::Plug,
        Icon::Folder,
        Icon::Layers,
        Icon::Browser,
    ];
    let items = SECTIONS
        .into_iter()
        .zip(icons)
        .enumerate()
        .map(move |(index, (label, icon))| {
            let view = index as u8;
            let ui = ui.clone();
            sapid::button()
                .name(i18n::bind(label))
                .class("button with-icon docker-nav-item")
                .fill((
                    icons::glyph(icon),
                    text().class("docker-nav-label").content(i18n::bind(label)),
                    text()
                        .class("docker-nav-count")
                        .content(state.map(move |s| section_count(s, view))),
                ))
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
                    } else if view >= 4 {
                        ui.docker_refresh();
                    }
                })
        })
        .collect::<Vec<_>>();
    col().class("docker-tabs").fill((
        col().class("docker-nav").fill(items),
        text()
            .class("docker-summary")
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
    ))
}

/// A short count under the section title, such as how many containers run.
fn section_meta(s: &State) -> String {
    let Some(snapshot) = &s.docker.snapshot else {
        return String::new();
    };
    match s.docker.view {
        0 if !s.docker.detail_open => {
            let running = snapshot
                .containers
                .iter()
                .filter(|c| c.state == "running")
                .count();
            format!(
                "{running} running · {} stopped",
                snapshot.containers.len() - running
            )
        }
        1 => {
            let unused = snapshot
                .volumes
                .iter()
                .filter(|v| volume_users(s, &v.name).is_empty())
                .count();
            if unused == 0 {
                String::new()
            } else {
                format!("{unused} unused")
            }
        }
        _ => String::new(),
    }
}

fn header(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    let install = ui.clone();
    let listeners = ui.clone();
    row().class("docker-header").fill((
        when(state.map(|s| s.docker.view == 0 && s.docker.detail_open), move || {
            sapid::button()
                .name(i18n::bind("Back to containers"))
                .class("button with-icon docker-back")
                .fill((
                    icons::glyph(Icon::ChevronRight),
                    text().content(i18n::bind("Containers")),
                ))
                .on_press(move || state.revise(|s| s.docker.detail_open = false))
        })
        .otherwise(move || {
            row().class("docker-heading").fill((
                text().class("docker-section-title").content(i18n::bind(
                    state.map(|s| SECTIONS[s.docker.view as usize].to_owned()),
                )),
                text()
                    .class("docker-section-meta")
                    .content(i18n::bind(state.map(section_meta))),
            ))
        }),
        row().class("docker-toolbar").fill((
            when(state.map(|s| s.docker.view == 4 && pull::offered(s)), {
                let ui = ui.clone();
                move || {
                    let ui = ui.clone();
                    text_action("Pull image", Icon::Download)
                        .class("button with-icon compact")
                        .enabled(state.map(ready))
                        .on_press(move || ui.docker_pull_open())
                }
            }),
            when(state.map(|s| s.docker.view == 4), {
                let ui = ui.clone();
                move || {
                    let ui = ui.clone();
                    text_action("Check for updates", Icon::Refresh)
                        .class("button with-icon compact")
                        .enabled(state.map(|s| ready(s) && !s.docker.updates_checking()))
                        .on_press(move || ui.docker_check_updates())
                }
            }),
            when(state.map(|s| s.docker.view == 4), {
                let ui = ui.clone();
                move || {
                    let ui = ui.clone();
                    icon_action("Clean up disk space", Icon::Clear)
                        .class("icon-button")
                        .enabled(state.map(ready))
                        .on_press(move || ui.docker_cleanup_open())
                }
            }),
            when(state.map(|s| s.docker.view == 4 && pull::offered(s)), {
                let ui = ui.clone();
                move || {
                    let ui = ui.clone();
                    icon_action("Registry logins", Icon::Key)
                        .class("icon-button")
                        .enabled(state.map(ready))
                        .on_press(move || ui.docker_logins_open())
                }
            }),
            when(state.map(|s| s.docker.view == 4), {
                let ui = ui.clone();
                move || {
                    let ui = ui.clone();
                    icon_action("Registry mirrors", Icon::Settings)
                        .class("icon-button")
                        .enabled(state.map(ready))
                        .on_press(move || ui.docker_mirrors_open())
                }
            }),
            when(state.map(|s| s.docker.view == 2), move || {
                let listeners = listeners.clone();
                text_action("Server listeners", Icon::Server).class("button with-icon compact")
                    .on_press(move || {
                        listeners.state.revise(|s| {
                            monitor_ui::set_section(s, monitor_ui::Section::Processes);
                            processes_ui::show_port_details(s, true);
                        });
                        listeners.open_desktop_app(WorkspacePage::Monitor);
                    })
            }),
            when(state.map(|s| !s.docker.detail_open), move || {
                let install = install.clone();
                text_action("Install software", Icon::Plus)
                    .class("button with-icon compact")
                    .enabled(state.map(|s| s.connected && !scripts_ui::busy(s)))
                    .on_press(move || install.open_software_install(2, None))
            }),
            icon_action("Refresh Docker", Icon::Refresh)
                .class("icon-button")
                .enabled(state.map(ready))
                .on_press(move || refresh.docker_refresh()),
            when(state.map(|s| s.docker.view == 2), || {
                help_ui::tip("Docker ports", "Docker reports published and exposed ports. Host-network containers use server listeners directly; inspect them in Monitor → Processes → Port details.")
            }),
            when(state.map(|s| s.docker.view == 3), || {
                help_ui::tip("Compose projects", "Application definitions are in ~/.arcnix/apps/<name>/compose.yaml. Docker manages named-volume data. Applying changes is a server task; closing the client does not stop accepted work.")
            }),
            when(state.map(|s| s.docker.view < 2 || s.docker.view > 3), || {
                help_ui::tip("Docker connection", "Uses this SSH user's local Docker socket, including rootless Docker. No automatic sudo. Start Docker or check this user's socket access if a connection error appears.")
            }),
        )),
    ))
}

fn filters(state: Cage<State>) -> impl Widget {
    row().class("docker-filters").fill((
        row().class("docker-search-field").fill((
            icons::glyph(Icon::Search),
            input()
                .class("docker-filter")
                .name(i18n::bind("Filter Docker resources"))
                .semantics(state.map(|s| {
                    Some(Box::new(Semantics {
                        placeholder: Some(i18n::translate(match s.docker.view {
                            0 => "Search by name, image or port",
                            1 => "Search volumes",
                            2 => "Search by container or port",
                            5 => "Search networks",
                            _ => "Search images",
                        })),
                        ..Default::default()
                    }))
                }))
                .value(state.map(|s| s.docker.filter.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| s.docker.filter = value);
                    }
                }),
        )),
        when(state.map(|s| s.docker.view == 0), move || {
            sapid::checkbox()
                .class("preference-check")
                .label(i18n::bind("Only running containers"))
                .checked(state.map(|s| s.docker.running_only))
                .on_checked_change(move |value| state.revise(|s| s.docker.running_only = value))
        }),
    ))
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let sidebar = navigation(ui.clone());
    let content = col().class("docker-main").fill((
        header(ui.clone()),
        when(
            state.map(|s| s.docker.view != 3 && !s.docker.detail_open),
            move || filters(state),
        ),
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
                s.docker.view < 3
                    && !s.docker.detail_open
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
                    .class("empty-copy docker-empty")
                    .content(i18n::bind("No Docker resources"))
            },
        ),
        when(
            state.map(|s| {
                s.docker.view == 0
                    && !s.docker.detail_open
                    && s.docker
                        .snapshot
                        .as_ref()
                        .is_some_and(|v| !v.containers.is_empty())
            }),
            {
                let ui = ui.clone();
                move || {
                    col().class("docker-inventory docker-table").fill((
                        column_titles(
                            &["Name", "Status", "Ports"],
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
        fragment((
            when(state.map(|s| s.docker.view == 3), {
                let ui = ui.clone();
                move || compose_ui::projects(ui.clone())
            }),
            when(state.map(|s| s.docker.view == 4), {
                let ui = ui.clone();
                move || images(ui.clone())
            }),
            when(state.map(|s| s.docker.view == 5), move || {
                networks::view(ui.clone())
            }),
        )),
    ));
    row().class("docker-page").fill((sidebar, content))
}
/// Whether a container runs this image, by `repository:tag` or by name for `latest`.
fn image_in_use(s: &State, image: &docker::Image) -> bool {
    let reference = format!("{}:{}", image.repository, image.tag);
    s.docker.snapshot.as_ref().is_some_and(|v| {
        v.containers
            .iter()
            .any(|c| c.image == reference || image.tag == "latest" && c.image == image.repository)
    })
}
fn images(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        text()
            .class("docker-updates-summary hint")
            .content(state.map(updates::summary)),
        col().class("docker-inventory docker-table").fill((
            column_titles(
                &["Image repository", "Tag", "Image ID", "Size"],
                "docker-image-row docker-columns",
            ),
            each(state.map(|s| {
                let filter = s.docker.filter.to_lowercase();
                s.docker
                    .images
                    .iter()
                    .filter(|image| {
                        [&image.repository, &image.tag, &image.id]
                            .into_iter()
                            .any(|value| value.to_lowercase().contains(&filter))
                    })
                    .map(|image| (image.clone(), image_in_use(s, image)))
                    .collect::<Vec<_>>()
            }))
            .key(|(image, _)| {
                (
                    image.id.clone(),
                    image.repository.clone(),
                    image.tag.clone(),
                )
            })
            .children(move |item| {
                let image = item.value;
                let run = ui.clone();
                let remove = ui.clone();
                row().class("docker-image-row").fill((
                    row().class("docker-image-name").fill((
                        text()
                            .class(image.map(|(image, _)| {
                                if image.repository == "<none>" {
                                    "docker-resource-name untagged"
                                } else {
                                    "docker-resource-name"
                                }
                                .to_owned()
                            }))
                            .selectable(true)
                            .content(image.map(|(image, _)| image.repository.clone())),
                        text()
                            .class(image.map(|(_, used)| {
                                if *used {
                                    "docker-badge published"
                                } else {
                                    "hidden"
                                }
                                .to_owned()
                            }))
                            .content(i18n::bind("In use")),
                        text()
                            .class(state.map(move |s| {
                                let (image, _) = image.get();
                                let reference = format!("{}:{}", image.repository, image.tag);
                                if updates::newer(s, &reference) {
                                    "docker-badge update"
                                } else {
                                    "hidden"
                                }
                                .to_owned()
                            }))
                            .content(i18n::bind("Update available")),
                        text()
                            .class("docker-image-unchecked subtle")
                            .content(state.map(move |s| {
                                let (image, _) = image.get();
                                updates::reason(s, &format!("{}:{}", image.repository, image.tag))
                            })),
                        when(
                            state.map(move |s| {
                                let (image, _) = image.get();
                                updates::newer(s, &format!("{}:{}", image.repository, image.tag))
                            }),
                            {
                                let update = run.clone();
                                move || {
                                    let update = update.clone();
                                    btn("Update", move || {
                                        let (image, _) = image.get();
                                        update.docker_update_image(format!(
                                            "{}:{}",
                                            image.repository, image.tag
                                        ));
                                    })
                                    .class("button compact docker-image-update")
                                    .enabled(state.map(|s| s.connected && !scripts_ui::busy(s)))
                                }
                            },
                        ),
                        // An untagged image has no name to run it by.
                        when(
                            image.map(|(image, _)| {
                                image.repository != "<none>" && image.tag != "<none>"
                            }),
                            move || {
                                let run = run.clone();
                                let image = image;
                                icon_action("Run a container", Icon::Play)
                                    .class("icon-button docker-image-run")
                                    .on_press(move || {
                                        let (image, _) = image.get();
                                        run.docker_run_open(format!(
                                            "{}:{}",
                                            image.repository, image.tag
                                        ));
                                    })
                            },
                        ),
                        {
                            let remove = remove.clone();
                            icon_action("Remove image", Icon::Clear)
                                .class("icon-button docker-image-remove")
                                .enabled(state.map(ready))
                                .on_press(move || {
                                    let (image, _) = image.get();
                                    remove.docker_remove_image_open(image);
                                })
                        },
                    )),
                    text()
                        .class("docker-image-tag")
                        .selectable(true)
                        .content(image.map(|(image, _)| image.tag.clone())),
                    text()
                        .class("docker-image-id")
                        .selectable(true)
                        .content(image.map(|(image, _)| {
                            let id = image.id.strip_prefix("sha256:").unwrap_or(&image.id);
                            id.chars().take(12).collect::<String>()
                        })),
                    text()
                        .class("docker-image-size")
                        .content(image.map(|(image, _)| image.size.clone())),
                ))
            }),
        )),
    ))
}
pub(crate) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        mirrors::dialog(ui.clone()),
        pull::dialog(ui.clone()),
        run::dialog(ui.clone()),
        cleanup::dialogs(ui.clone()),
        login::dialog(ui.clone()),
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
                        when(state.map(|s| s.docker.target.as_ref().is_some_and(|(_, op)| *op == Operation::Remove)), || {
                            text().content(i18n::bind("Removing this container discards its writable layer and keeps its volumes."))
                        }),
                        text()
                            .class("error")
                            .content(state.map(|s| s.docker.error.clone())),
                    )),
                    sapid::button()
                        .class(state.map(|s| {
                            if s.docker.target.as_ref().is_some_and(|(_, op)| *op == Operation::Remove) {
                                "button danger docker-confirm"
                            } else {
                                "button primary docker-confirm"
                            }.to_owned()
                        }))
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
            "Docker command failed (1): permission denied while trying to connect to the docker API at unix:///var/run/docker.sock-runtime/docker.sock",
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
    fn ports_read_once_without_the_wildcard_address() {
        assert_eq!(
            port_entries(
                "0.0.0.0:80->80/tcp, [::]:80->80/tcp, :::443->443/tcp, 127.0.0.1:8080->8080/tcp, 5432/tcp"
            ),
            [
                ("80 → 80/tcp".to_owned(), true),
                ("443 → 443/tcp".to_owned(), true),
                ("127.0.0.1:8080 → 8080/tcp".to_owned(), true),
                ("5432/tcp".to_owned(), false),
            ]
        );
        assert_eq!(
            port_entries("[::1]:8443->443/tcp, 0.0.0.0:8000-8002->8000-8002/udp"),
            [
                ("[::1]:8443 → 443/tcp".to_owned(), true),
                ("8000-8002 → 8000-8002/udp".to_owned(), true),
            ]
        );
        assert!(port_entries("").is_empty());
    }

    #[test]
    fn states_take_the_colour_of_their_outcome() {
        let container = |state: &str, status: &str| Container {
            id: String::new(),
            name: String::new(),
            image: String::new(),
            state: state.into(),
            status: status.into(),
            ports: String::new(),
            mounts: String::new(),
            networks: String::new(),
            compose_project: String::new(),
            compose_service: String::new(),
            compose_working_dir: String::new(),
        };
        for (state, status, expected) in [
            ("running", "Up 2 hours", "running"),
            ("running", "Up 2 hours (unhealthy)", "warning"),
            ("paused", "Up 2 hours (Paused)", "paused"),
            ("restarting", "Restarting (1) 3 seconds ago", "busy"),
            ("exited", "Exited (0) 3 days ago", "stopped"),
            ("exited", "Exited (137) 1 minute ago", "failed"),
            ("created", "Created", "stopped"),
            ("dead", "Dead", "failed"),
        ] {
            assert_eq!(tone(&container(state, status)), expected, "{status}");
        }
    }

    #[test]
    fn port_bindings_prefer_published_addresses() {
        let details = |ports: serde_json::Value, bindings: serde_json::Value| ContainerDetails {
            id: String::new(),
            name: String::new(),
            image: String::new(),
            state: String::new(),
            restart: String::new(),
            retries: 0,
            memory: 0,
            nano_cpus: 0,
            network: String::new(),
            ports,
            bindings,
            mounts: serde_json::Value::Null,
        };
        let running = details(
            serde_json::json!({"80/tcp": [{"HostIp": "127.0.0.1", "HostPort": "8080"}], "443/tcp": [{"HostIp": "::1", "HostPort": "8443"}], "9000/tcp": null}),
            serde_json::json!({"80/tcp": [{"HostIp": "127.0.0.1", "HostPort": "8080"}]}),
        );
        assert_eq!(
            port_bindings(&running),
            [
                ("[::1]:8443 → 443/tcp".to_owned(), "Published"),
                ("127.0.0.1:8080 → 80/tcp".to_owned(), "Published"),
                ("9000/tcp".to_owned(), "Exposed only"),
            ]
        );
        // A stopped container publishes nothing; its configured bindings remain.
        let stopped = details(
            serde_json::json!({}),
            serde_json::json!({"80/tcp": [{"HostIp": "", "HostPort": "8080"}]}),
        );
        assert_eq!(
            port_bindings(&stopped),
            [("*:8080 → 80/tcp".to_owned(), "Configured")]
        );
    }

    #[test]
    fn logs_without_visible_output_read_as_no_logs() {
        for logs in ["", "\n", "\x1b[0m\n", "\r\n\n"] {
            assert!(log_spans(logs, "").is_empty(), "{logs:?}");
        }
        assert!(!log_spans("2026-11-18T09:15:02.1Z \n", "").is_empty());
    }
}
