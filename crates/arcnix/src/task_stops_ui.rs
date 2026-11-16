//! Cross-page selection and fresh authorization, independent of the batch queue.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arcnix_core::task_stops as stops;
use arcnix_core::tasks::{self, Status, Task};

use super::*;
#[derive(Clone, Default)]
pub(super) struct StopState {
    pub selected: BTreeMap<String, BTreeSet<String>>,
    serial: u64,
    review: Option<Review>,
}
#[derive(Clone)]
struct Review {
    serial: u64,
    groups: Vec<Peer>,
    executing: bool,
    cancel: Arc<AtomicBool>,
}
#[derive(Clone)]
struct Peer {
    id: String,
    label: String,
    home: String,
    root: String,
    ids: Vec<String>,
    secret: String,
    sudo: String,
    pending: bool,
    error: String,
    snapshot: Option<stops::Snapshot>,
    authenticated: Option<batch_worker::Target>,
}
pub(super) fn dismiss(s: &mut State) {
    if let Some(review) = s.task_records.stops.review.take() {
        review.cancel.store(true, Ordering::Release);
    }
}
pub(super) fn count(s: &State) -> usize {
    s.task_records
        .stops
        .selected
        .values()
        .map(BTreeSet::len)
        .sum()
}
pub(super) fn toggle(s: &mut State, server: &str, id: &str) {
    let selected = s
        .task_records
        .stops
        .selected
        .entry(server.into())
        .or_default();
    if !selected.remove(id) {
        if count(s) >= 100 {
            s.task_records.error = i18n::translate("Choose 1 to 100 server tasks");
            return;
        }
        s.task_records
            .stops
            .selected
            .entry(server.into())
            .or_default()
            .insert(id.into());
    }
}
impl Ui {
    fn review_task_stops(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                count(s) > 0 && count(s) <= 100,
                "Choose 1 to 100 server tasks"
            );
            let mut groups = Vec::new();
            let mut endpoints = HashSet::new();
            for (id, ids) in &s.task_records.stops.selected {
                if ids.is_empty() {
                    continue;
                }
                credentials(s, id)?;
                let group = s
                    .task_records
                    .groups
                    .iter()
                    .find(|g| g.profile.id == *id)
                    .unwrap();
                anyhow::ensure!(
                    endpoints.insert((group.profile.host.to_lowercase(), group.profile.port)),
                    "Choose one SSH account per server for each stop batch"
                );
                let home = group
                    .home
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Reconnect to server records"))?;
                let root = group
                    .page
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Refresh server records first"))?
                    .root
                    .clone();
                groups.push(Peer {
                    id: id.clone(),
                    label: format!(
                        "{} · {}@{}:{}",
                        group.profile.name,
                        group.profile.username,
                        group.profile.host,
                        group.profile.port
                    ),
                    home,
                    root,
                    ids: ids.iter().cloned().collect(),
                    secret: String::new(),
                    sudo: String::new(),
                    pending: false,
                    error: String::new(),
                    snapshot: None,
                    authenticated: None,
                });
            }
            anyhow::ensure!(groups.len() <= 32, "Select between 1 and 32 servers");
            Ok(groups)
        });
        match result {
            Err(e) => self.state.revise(|s| s.task_records.error = e.to_string()),
            Ok(groups) => {
                self.state.revise(|s| {
                    dismiss(s);
                    s.task_records.stops.serial += 1;
                    let serial = s.task_records.stops.serial;
                    s.task_records.stops.review = Some(Review {
                        serial,
                        groups,
                        executing: false,
                        cancel: Arc::new(AtomicBool::new(false)),
                    });
                });
                self.open_dialog(Dialog::TaskStop);
            }
        }
    }
    fn inspect_task_stops(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let review = s
                .task_records
                .stops
                .review
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Select server tasks first"))?;
            anyhow::ensure!(
                !review.executing && !review.groups.iter().any(|g| g.pending),
                "Wait for task inspection"
            );
            let mut requests = Vec::new();
            for group in review.groups.iter().filter(|g| g.snapshot.is_none()) {
                let mut target = credentials(s, &group.id)?;
                for secret in [&group.secret, &group.sudo] {
                    anyhow::ensure!(
                        secret.len() <= 1024 && !secret.contains(['\0', '\r', '\n']),
                        "Invalid credential"
                    );
                }
                target.secret = Zeroizing::new(group.secret.clone());
                target.sudo = Zeroizing::new(group.sudo.clone());
                requests.push((
                    review.serial,
                    target,
                    group.home.clone(),
                    group.root.clone(),
                    group.ids.clone(),
                ));
            }
            Ok(requests)
        });
        match result {
            Err(e) => self.state.revise(|s| s.dialog_error = e.to_string()),
            Ok(requests) => {
                self.state.revise(|s| {
                    if let Some(review) = &mut s.task_records.stops.review {
                        for group in review.groups.iter_mut().filter(|g| g.snapshot.is_none()) {
                            group.pending = true;
                            group.snapshot = None;
                            group.authenticated = None;
                            group.error.clear();
                            group.secret.clear();
                            group.sudo.clear();
                        }
                    }
                });
                for (serial, target, home, root, ids) in requests {
                    batch_worker::inspect_stops(
                        serial,
                        target,
                        home,
                        root,
                        ids,
                        self.batch_events.clone(),
                    );
                }
            }
        }
    }
    fn submit_task_stops(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let review = s
                .task_records
                .stops
                .review
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Inspect selected tasks first"))?;
            anyhow::ensure!(
                !review.executing && !review.groups.iter().any(|g| g.pending),
                "Wait for task inspection"
            );
            let mut targets = Vec::new();
            let mut descriptions = BTreeMap::new();
            let mut records = Vec::new();
            for group in &review.groups {
                let Some(snapshot) = &group.snapshot else {
                    continue;
                };
                let Ok(plan) = snapshot.plan() else { continue };
                credentials(s, &group.id)?;
                let target = group
                    .authenticated
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Inspect selected tasks first"))?;
                descriptions.insert(group.id.clone(), plan.selection.description()?);
                records.push(tasks::Target {
                    id: group.id.clone(),
                    name: target.profile.name.clone(),
                    endpoint: format!(
                        "{}@{}:{}",
                        target.profile.username, target.profile.host, target.profile.port
                    ),
                    fingerprint: target.fingerprint.clone(),
                    status: Status::Queued,
                    preview: plan.selection.ids.join("\n"),
                    output: String::new(),
                    exit_code: None,
                    remote_directory: String::new(),
                    result_read_at: None,
                });
                targets.push((target, plan));
            }
            anyhow::ensure!(!targets.is_empty(), "No verified tasks can be stopped");
            Ok((
                review.serial,
                review.cancel.clone(),
                targets,
                Task {
                    stops: Some(descriptions),
                    drill: None,
                    installation: None,
                    configuration: None,
                    configuration_targets: None,
                    diagnostic: None,
                    backup: None,
                    id: new_id(),
                    name: "Stop server tasks".into(),
                    script_id: "builtin:task-stop".into(),
                    revision: stops::revision(),
                    created: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_secs(),
                    targets: records,
                },
            ))
        });
        match result {
            Err(e) => self.state.revise(|s| s.dialog_error = e.to_string()),
            Ok((serial, cancel, targets, task)) => {
                let id = task.id.clone();
                let previous = self
                    .state
                    .with(|s| (s.scripts.tasks.clone(), s.scripts.selected_task.clone()));
                self.state.revise(|s| {
                    s.scripts.tasks.insert(0, task);
                    while s.scripts.tasks.len() > 10 {
                        if let Some(index) = s
                            .scripts
                            .tasks
                            .iter()
                            .rposition(|t| Some(&t.id) != s.scripts.active.as_ref())
                        {
                            s.scripts.tasks.remove(index);
                        } else {
                            break;
                        }
                    }
                    s.scripts.selected_task = Some(id.clone());
                });
                if !self.persist_tasks() {
                    self.state.revise(|s| {
                        s.scripts.tasks = previous.0;
                        s.scripts.selected_task = previous.1;
                        s.dialog_error = i18n::translate("Cannot save task history");
                    });
                    return;
                }
                self.state.revise(|s| {
                    if let Some(review) = &mut s.task_records.stops.review {
                        review.executing = true;
                        for group in &mut review.groups {
                            group.authenticated = None;
                        }
                    }
                });
                batch_worker::execute_stops(serial, id, targets, cancel, self.batch_events.clone());
            }
        }
    }
}
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    task: String,
    server: String,
    update: batch_worker::StopUpdate,
) {
    match update {
        batch_worker::StopUpdate::Inspected(result) => ui.state.revise(|s| {
            if let Some(peer) = s
                .task_records
                .stops
                .review
                .as_mut()
                .filter(|r| r.serial == serial && !r.executing)
                .and_then(|r| r.groups.iter_mut().find(|g| g.id == server))
            {
                peer.pending = false;
                match result {
                    Ok(pair) => {
                        let (snapshot, target) = *pair;
                        peer.snapshot = Some(snapshot);
                        peer.authenticated = Some(target);
                    }
                    Err(e) => peer.error = e,
                }
            }
        }),
        batch_worker::StopUpdate::NotSubmitted { error, cancelled } => {
            ui.state.revise(|s| {
                if let Some(target) = s
                    .scripts
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == task)
                    .and_then(|t| t.targets.iter_mut().find(|t| t.id == server))
                {
                    target.remote_directory.clear();
                    target.result_read_at = None;
                    target.exit_code = None;
                    target.output = error;
                    target.status = if cancelled {
                        Status::Cancelled
                    } else {
                        Status::Failed
                    };
                }
            });
            ui.persist_tasks();
        }
        batch_worker::StopUpdate::Launch { directory, reply } => {
            let allowed = ui.state.revise(|s| {
                let authorized = s.task_records.stops.review.as_ref().is_some_and(|r| {
                    r.serial == serial && r.executing && !r.cancel.load(Ordering::Acquire)
                });
                let Some(record) = s
                    .scripts
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == task)
                    .and_then(|t| t.targets.iter_mut().find(|t| t.id == server))
                else {
                    return false;
                };
                let trusted = s
                    .config
                    .servers
                    .iter()
                    .find(|p| p.id == server)
                    .is_some_and(|p| {
                        record.endpoint == format!("{}@{}:{}", p.username, p.host, p.port)
                            && s.config.trusted_fingerprint(&p.host, p.port)
                                == Some(record.fingerprint.as_str())
                    });
                if authorized && trusted {
                    record.remote_directory = directory;
                    record.status = Status::Submitting;
                    true
                } else {
                    record.status = Status::Cancelled;
                    false
                }
            });
            let saved = ui.persist_tasks();
            let _ = reply.send(allowed && saved);
        }
    }
}
pub(super) fn toolbar(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        btn("Review selected stops…", move || ui.review_task_stops())
            .class("button task-stop-review")
            .enabled(state.map(|s| count(s) > 0)),
        btn("Clear selected tasks", move || {
            state.revise(|s| s.task_records.stops.selected.clear())
        })
        .enabled(state.map(|s| count(s) > 0)),
    ))
}
pub(super) fn selection(ui: Ui, server: String, id: String) -> impl Widget {
    let state = ui.state;
    let selected_server = server.clone();
    let selected_id = id.clone();
    let enabled_server = server.clone();
    let enabled_id = id.clone();
    btn("Select task", move || {
        state.revise(|s| toggle(s, &server, &id))
    })
    .class(state.map(move |s| {
        if s.task_records
            .stops
            .selected
            .get(&selected_server)
            .is_some_and(|ids| ids.contains(&selected_id))
        {
            "button compact task-stop-select active"
        } else {
            "button compact task-stop-select"
        }
        .to_owned()
    }))
    .enabled(state.map(move |s| {
        s.task_records
            .stops
            .selected
            .get(&enabled_server)
            .is_some_and(|ids| ids.contains(&enabled_id))
            || s.task_records
                .groups
                .iter()
                .find(|g| g.profile.id == enabled_server)
                .and_then(|g| g.page.as_ref())
                .is_some_and(|p| {
                    p.records.iter().any(|r| {
                        r.id == enabled_id
                            && matches!(
                                r.service,
                                records::ServiceState::Active
                                    | records::ServiceState::Activating
                                    | records::ServiceState::Reloading
                                    | records::ServiceState::Deactivating
                            )
                    })
                })
    }))
}
pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::TaskStop)),
        move || {
            let inspect = ui.clone();
            let submit = ui.clone();
            let close = ui.clone();
            modal(ui.clone(),"Stop server tasks",col().class("form-stack task-stop-dialog").fill((
            row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Selected server tasks")),crate::help_ui::group("Stop server tasks",&["Inspect the original task identity before stopping. Only verified tasks are submitted; refused tasks remain unchanged.","Accepted control jobs run on the server when Arcnix closes. Leaving this review cancels submissions that have not started. Query the original stop batch for results; never submit it again.","This uses administrator access and requires Python 3, systemd, busctl and cgroup v2. It bypasses the operation lock so a running operation can be stopped."]))),
            text().class("form-error").content(i18n::bind("Stopping can leave partial changes. It does not roll back the operation.")),
            each(state.map(|s|s.task_records.stops.review.as_ref().map(|r|r.groups.clone()).unwrap_or_default())).key(|g|g.id.clone()).children(move|item|{
                let group=item.value;let id=group.get().id;
                let secret_id=id.clone();let sudo_id=id.clone();let visible_id=id.clone();
                col().class("task-stop-peer").fill((
                    text().class("section-title").content(group.map(|g|g.label.clone())),
                    when(state.map(move|s|s.task_records.stops.review.as_ref().is_some_and(|r|!r.executing&&r.groups.iter().find(|g|g.id==visible_id).is_some_and(|g|!g.pending&&g.snapshot.is_none()))),move||{
                        let secret_id=secret_id.clone();let sudo_id=sudo_id.clone();
                        col().class("form-stack").fill((
                            text().class("field-label").content(i18n::bind("SSH password or key passphrase")),input().purpose(Purpose::Password).name(i18n::bind("SSH password or key passphrase")).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|if let Some(peer)=s.task_records.stops.review.as_mut().and_then(|r|r.groups.iter_mut().find(|g|g.id==secret_id)){peer.secret=value;});}),
                            text().class("field-label").content(i18n::bind("sudo password (optional)")),input().purpose(Purpose::Password).name(i18n::bind("sudo password (optional)")).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|if let Some(peer)=s.task_records.stops.review.as_mut().and_then(|r|r.groups.iter_mut().find(|g|g.id==sudo_id)){peer.sudo=value;});}),
                        ))
                    }),
                    text().class("form-error").content(group.map(|g|g.error.clone())),
                    text().class("script-code task-stop-inspection").content(group.map(|g|if g.pending{i18n::translate("Inspecting server tasks…")}else if let Some(snapshot)=&g.snapshot{snapshot.rows.iter().map(|r|format!("{} · {}{}",r.id,i18n::translate(if r.state=="ready"{"Ready to stop"}else if r.state=="stopping"{"Stopping"}else{"Refused"}),if r.reason.is_empty(){String::new()}else{format!(" · {}",i18n::translate(stops::reason_label(&r.reason)))})).collect::<Vec<_>>().join("\n")}else{g.ids.join("\n")})),
                ))
            }),
            text().class("subtle task-stop-submitted").content(i18n::bind(state.map(|s|if s.task_records.stops.review.as_ref().is_some_and(|r|r.executing){"Stop batch saved. View recent batches to query each server result."}else{""}.to_owned()))),
        )),(btn("Close",move||close.close_dialog()),btn("Inspect selected tasks",move||inspect.inspect_task_stops()).enabled(state.map(|s|s.task_records.stops.review.as_ref().is_some_and(|r|!r.executing&&!r.groups.iter().any(|g|g.pending)&&r.groups.iter().any(|g|g.snapshot.is_none())))),btn("Stop verified tasks",move||submit.submit_task_stops()).class("button danger").enabled(state.map(|s|s.task_records.stops.review.as_ref().is_some_and(|r|!r.executing&&!r.groups.iter().any(|g|g.pending)&&r.groups.iter().any(|g|g.snapshot.as_ref().is_some_and(|p|p.rows.iter().any(|r|r.ready()))))))))
        },
    )
}
