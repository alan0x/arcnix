//! Session-only remote task inspection, separate from durable local confirmations.
use arcnix_core::task_records::{self as records, Page};
use zeroize::Zeroizing;

use super::*;
#[path = "task_stops_ui.rs"]
mod stops_ui;
pub(super) fn dismiss(s: &mut State) {
    if s.dialog == Some(Dialog::TaskStop) {
        stops_ui::dismiss(s);
    }
}

#[derive(Clone)]
struct Group {
    profile: ServerProfile,
    fingerprint: String,
    secret: Option<Zeroizing<String>>,
    home: Option<String>,
    page: Option<Page>,
    pending: Option<u64>,
    error: String,
}
#[derive(Clone)]
struct Viewer {
    server: String,
    record: String,
    pending: Option<u64>,
    detail: Option<records::Detail>,
    error: String,
}
#[derive(Clone, Default)]
pub(super) struct RecordState {
    pub visible: bool,
    stops: stops_ui::StopState,
    pub secrets: HashMap<String, String>,
    selected: HashSet<String>,
    groups: Vec<Group>,
    serial: u64,
    viewer: Option<Viewer>,
    error: String,
}
pub(super) fn forget(s: &mut State) {
    stops_ui::dismiss(s);
    s.task_records.stops.selected.clear();
    s.task_records.serial += 1;
    s.task_records.secrets.clear();
    for group in &mut s.task_records.groups {
        group.secret = None;
        group.pending = None;
    }
    s.task_records.viewer = None;
    if matches!(
        s.dialog,
        Some(Dialog::TaskRecordConnect | Dialog::TaskRecordLog | Dialog::TaskStop)
    ) {
        s.dialog = None;
    }
}
pub(super) fn recent(s: &mut State) {
    forget(s);
    record_maintenance_ui::leave(s);
    cleanup_records_ui::leave(s);
    s.task_records.visible = false;
}
fn credentials(s: &State, id: &str) -> anyhow::Result<batch_worker::Target> {
    let group = s
        .task_records
        .groups
        .iter()
        .find(|g| g.profile.id == id)
        .ok_or_else(|| anyhow::anyhow!("Reconnect to server records"))?;
    let profile = s
        .config
        .servers
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| anyhow::anyhow!("Server inventory changed; reconnect to server records"))?;
    anyhow::ensure!(
        profile.host == group.profile.host
            && profile.port == group.profile.port
            && profile.username == group.profile.username
            && s.config.trusted_fingerprint(&profile.host, profile.port)
                == Some(group.fingerprint.as_str()),
        "Server identity changed; reconnect to server records"
    );
    Ok(batch_worker::Target {
        profile: group.profile.clone(),
        fingerprint: group.fingerprint.clone(),
        secret: group
            .secret
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Reconnect to server records"))?,
        sudo: Zeroizing::new(String::new()),
    })
}
fn diagnostic_record(s: &State) -> anyhow::Result<arcnix_core::tasks::Task> {
    let viewer = s
        .task_records
        .viewer
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Read a saved diagnostic log first"))?;
    let detail = viewer
        .detail
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Read a saved diagnostic log first"))?;
    let metadata = detail
        .record
        .metadata
        .as_ref()
        .filter(|m| m.kind == records::Kind::Diagnostic)
        .ok_or_else(|| anyhow::anyhow!("Choose a saved diagnostic record"))?;
    let group = s
        .task_records
        .groups
        .iter()
        .find(|g| g.profile.id == viewer.server)
        .ok_or_else(|| anyhow::anyhow!("Original server identity is unavailable"))?;
    Ok(arcnix_core::tasks::Task {
        stops: None,
        drill: None,
        installation: None,
        configuration: None,
        configuration_targets: None,
        diagnostic: metadata.diagnostic.as_deref().cloned(),
        backup: None,
        id: detail.record.id.clone(),
        name: metadata.name.clone(),
        script_id: arcnix_core::diagnostics::SCRIPT_ID.into(),
        revision: metadata.revision.clone(),
        created: metadata.created,
        targets: vec![arcnix_core::tasks::Target {
            id: group.profile.id.clone(),
            name: group.profile.name.clone(),
            endpoint: format!(
                "{}@{}:{}",
                group.profile.username, group.profile.host, group.profile.port
            ),
            fingerprint: group.fingerprint.clone(),
            status: detail.record.service.status(detail.record.exit_code),
            preview: String::new(),
            output: detail.output.clone(),
            exit_code: detail.record.exit_code,
            remote_directory: format!("{}/{}", detail.root, detail.record.id),
            result_read_at: None,
        }],
    })
}
fn record_log_context(s: &State) -> Option<arcnix_core::diagnostic_logs::Context> {
    let task = diagnostic_record(s).ok()?;
    arcnix_core::diagnostic_logs::Context::new(
        &task.id,
        task.targets.first()?,
        task.diagnostic.as_ref(),
    )
    .ok()
}
fn record_deleted_target(s: &State) -> Option<arcnix_core::tasks::Target> {
    let task = diagnostic_record(s).ok()?;
    let target = task.targets.first()?;
    (deleted_files_ui::offered(target, task.diagnostic.as_ref())
        && deleted_files_ui::available(s, target))
    .then(|| target.clone())
}
fn record_tcp_target(s: &State) -> Option<(arcnix_core::tasks::Target, Vec<u16>)> {
    let task = diagnostic_record(s).ok()?;
    let target = task.targets.first()?;
    (tcp_listeners_ui::offered(target, task.diagnostic.as_ref())
        && tcp_listeners_ui::available(s, target))
    .then(|| {
        (
            target.clone(),
            task.diagnostic
                .as_ref()
                .and_then(|p| p.tcp_ports().ok())
                .unwrap_or_default(),
        )
    })
}
impl Ui {
    fn export_record_diagnostics(&self) {
        match self.state.with(diagnostic_record) {
            Ok(task) => self.open_diagnostic_report(
                task,
                arcnix_core::diagnostic_exports::Source::ServerRecord,
            ),
            Err(e) => self.state.revise(|s| s.dialog_error = e.to_string()),
        }
    }
    fn read_record_service_logs(&self) {
        if let Some(context) = self.state.with(record_log_context) {
            self.open_diagnostic_logs(context);
        }
    }
    fn read_record_deleted_files(&self) {
        if let Some(target) = self.state.with(record_deleted_target) {
            self.open_deleted_files(target);
        }
    }
    fn read_record_tcp_listeners(&self) {
        if let Some((target, ports)) = self.state.with(record_tcp_target) {
            self.open_tcp_listeners(target, ports);
        }
    }
    fn connect_records(&self) {
        self.state.revise(|s| {
            s.task_records.selected = s
                .task_records
                .groups
                .iter()
                .map(|g| g.profile.id.clone())
                .collect();
            s.task_records.secrets.clear();
        });
        self.open_dialog(Dialog::TaskRecordConnect);
    }
    fn browse_records(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let mut peers = HashSet::new();
            let mut targets = Vec::new();
            for profile in s
                .config
                .servers
                .iter()
                .filter(|p| s.task_records.selected.contains(&p.id))
            {
                anyhow::ensure!(
                    peers.insert((
                        profile.host.to_lowercase(),
                        profile.port,
                        profile.username.clone()
                    )),
                    "Choose each server account only once"
                );
                let fingerprint = s
                    .config
                    .trusted_fingerprint(&profile.host, profile.port)
                    .ok_or_else(|| anyhow::anyhow!("Connect and save the host fingerprint first"))?
                    .to_owned();
                let secret = s
                    .task_records
                    .secrets
                    .get(&profile.id)
                    .cloned()
                    .unwrap_or_default();
                anyhow::ensure!(
                    secret.len() <= 1024 && !secret.contains(['\0', '\r', '\n']),
                    "Invalid credential"
                );
                let mut profile = profile.clone();
                if profile.connection.is_none() {
                    profile.connection = Some(s.config.settings.connection);
                }
                targets.push(batch_worker::Target {
                    profile,
                    fingerprint,
                    secret: Zeroizing::new(secret),
                    sudo: Zeroizing::new(String::new()),
                });
            }
            anyhow::ensure!(
                !targets.is_empty() && targets.len() <= 32,
                "Select between 1 and 32 servers"
            );
            Ok(targets)
        });
        match result {
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
            Ok(targets) => {
                let requests = self.state.revise(|s| {
                    forget(s);
                    let previous = std::mem::take(&mut s.task_records.groups);
                    s.task_records.visible = true;
                    s.task_records.error.clear();
                    let mut requests = Vec::new();
                    for target in targets {
                        let prior = previous.iter().find(|g| {
                            g.profile.id == target.profile.id
                                && g.profile.host == target.profile.host
                                && g.profile.port == target.profile.port
                                && g.profile.username == target.profile.username
                                && g.fingerprint == target.fingerprint
                        });
                        s.task_records.serial += 1;
                        let serial = s.task_records.serial;
                        s.task_records.groups.push(Group {
                            profile: target.profile.clone(),
                            fingerprint: target.fingerprint.clone(),
                            secret: Some(target.secret.clone()),
                            home: prior.and_then(|g| g.home.clone()),
                            page: prior.and_then(|g| g.page.clone()),
                            pending: Some(serial),
                            error: String::new(),
                        });
                        requests.push((serial, target));
                    }
                    s.dialog = None;
                    requests
                });
                for (serial, target) in requests {
                    batch_worker::record_page(
                        serial,
                        target,
                        None,
                        None,
                        0,
                        self.batch_events.clone(),
                    );
                }
            }
        }
    }
    fn record_page(&self, id: &str, offset: u32) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let target = credentials(s, id)?;
            let group = s
                .task_records
                .groups
                .iter()
                .find(|g| g.profile.id == id)
                .unwrap();
            anyhow::ensure!(group.pending.is_none(), "Wait for the record query");
            Ok((
                target,
                group.home.clone(),
                group.page.as_ref().map(|p| p.root.clone()),
            ))
        });
        match result {
            Err(error) => self
                .state
                .revise(|s| s.task_records.error = format!("{error:#}")),
            Ok((target, home, root)) => {
                let serial = self.state.revise(|s| {
                    s.task_records.serial += 1;
                    let serial = s.task_records.serial;
                    let group = s
                        .task_records
                        .groups
                        .iter_mut()
                        .find(|g| g.profile.id == id)
                        .unwrap();
                    group.pending = Some(serial);
                    group.error.clear();
                    serial
                });
                batch_worker::record_page(
                    serial,
                    target,
                    home,
                    root,
                    offset,
                    self.batch_events.clone(),
                );
            }
        }
    }
    fn read_record(&self, server: String, id: String) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let target = credentials(s, &server)?;
            let group = s
                .task_records
                .groups
                .iter()
                .find(|g| g.profile.id == server)
                .unwrap();
            let page = group
                .page
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Load server records first"))?;
            anyhow::ensure!(
                page.records.iter().any(|r| r.id == id),
                "Reload the record page before reading this log"
            );
            Ok((target, group.home.clone().unwrap(), page.root.clone()))
        });
        match result {
            Err(error) => self
                .state
                .revise(|s| s.task_records.error = format!("{error:#}")),
            Ok((target, home, root)) => {
                let serial = self.state.revise(|s| {
                    s.task_records.serial += 1;
                    let serial = s.task_records.serial;
                    let previous = s
                        .task_records
                        .viewer
                        .take()
                        .filter(|v| v.server == server && v.record == id)
                        .and_then(|v| v.detail);
                    s.task_records.viewer = Some(Viewer {
                        server: server.clone(),
                        record: id.clone(),
                        pending: Some(serial),
                        detail: previous,
                        error: String::new(),
                    });
                    serial
                });
                self.open_dialog(Dialog::TaskRecordLog);
                batch_worker::record_detail(
                    serial,
                    target,
                    home,
                    root,
                    id,
                    self.batch_events.clone(),
                );
            }
        }
    }
}
pub(super) fn event(ui: &Ui, event: batch_worker::Event) {
    let event = match event {
        batch_worker::Event::TaskStop {
            serial,
            task,
            server,
            update,
        } => {
            stops_ui::event(ui, serial, task, server, update);
            return;
        }
        event => event,
    };
    ui.state.revise(|s| match event {
        batch_worker::Event::RecordPage { serial, id, result } => {
            if let Some(group) = s
                .task_records
                .groups
                .iter_mut()
                .find(|g| g.profile.id == id && g.pending == Some(serial))
            {
                group.pending = None;
                match result {
                    Ok((home, page)) => {
                        group.home = Some(home);
                        group.page = Some(page);
                        group.error.clear();
                    }
                    Err(error) => group.error = error,
                }
            }
        }
        batch_worker::Event::RecordDetail { serial, id, result } => {
            if let Some(viewer) = s
                .task_records
                .viewer
                .as_mut()
                .filter(|v| v.server == id && v.pending == Some(serial))
            {
                viewer.pending = None;
                match result {
                    Ok(detail) => {
                        viewer.detail = Some(detail);
                        viewer.error.clear();
                    }
                    Err(error) => viewer.error = error,
                }
            }
        }
        _ => {}
    });
}
pub(super) fn navigation(ui: Ui) -> impl Widget {
    let state = ui.state;
    let local = ui.clone();
    let remote = ui.clone();
    row().class("global-toolbar").fill((
        btn("Recent batches", move || local.state.revise(recent)).class(state.map(|s| {
            if s.task_records.visible || s.record_storage.visible || s.cleanup_reports.visible {
                "choice"
            } else {
                "choice active"
            }
            .to_owned()
        })),
        btn("Server records", move || {
            let empty = remote.state.with(|s| s.task_records.groups.is_empty());
            remote.state.revise(|s| {
                record_maintenance_ui::leave(s);
                cleanup_records_ui::leave(s);
                s.task_records.visible = true;
            });
            if empty {
                remote.connect_records();
            }
        })
        .class(state.map(|s| {
            if s.task_records.visible && !s.record_storage.visible && !s.cleanup_reports.visible {
                "choice active"
            } else {
                "choice"
            }
            .to_owned()
        })),
        record_maintenance_ui::tab(ui.clone()),
        cleanup_records_ui::tab(ui.clone()),
    ))
}
pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let connect = ui.clone();
    let forget_ui = ui.clone();
    let install = ui.clone();
    col().class("server-records-page").fill((
        row().class("global-toolbar").fill((stops_ui::toolbar(ui.clone()),btn("Connect to server records…",move||connect.connect_records()).class("button primary"),btn("Forget record credentials",move||forget_ui.state.revise(forget)),btn("Install record tools…",move||{if let Some(template)=arcnix_core::scripts::catalog().into_iter().find(|t|t.snippet.id=="builtin:tools"){install.choose_script(template);}}).enabled(state.map(|s|!scripts_ui::busy(s))))),
        row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Server records")),crate::help_ui::group("Server records",&["Browse this SSH account's saved tasks without relying on local history. Queries do not submit or repeat work. Records and full logs remain on the server.","Credentials stay in memory while this page is open. Leave Tasks or forget credentials to prevent new queries; an in-flight read may finish before disconnecting. Results are snapshots.","New exit receipts bind the task and service invocation. Legacy results without this binding remain unconfirmed. A completed task does not prove that a backup can be restored or an application is healthy."]))),
        text().class(state.map(|s|if s.task_records.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(state.map(|s|s.task_records.error.clone())),
        each(state.map(|s|s.task_records.groups.clone())).key(|g|g.profile.id.clone()).children({let ui=ui.clone();move|item|{
            let group=item.value; let id=group.get().profile.id;let rows_id=id.clone();let previous=ui.clone();let refresh=ui.clone();let next=ui.clone();let prev_id=id.clone();let refresh_id=id.clone();let next_id=id.clone();let enabled_id=id.clone();let prev_enabled_id=id.clone();let next_enabled_id=id.clone();let row_ui=ui.clone();
            col().class("server-record-group").fill((
                text().class("section-title").content(group.map(|g|g.profile.name.clone())),
                text().class("subtle").content(group.map(|g|format!("{}@{}:{} · {}",g.profile.username,g.profile.host,g.profile.port,g.fingerprint))),
                text().class("subtle").content(i18n::bind(group.map(|g|if g.pending.is_some(){"Reading server records…"}else if g.secret.is_none(){"Disconnected; reconnect to refresh records or read logs"}else if !g.error.is_empty(){if g.page.is_some(){"Record query failed; showing the previous snapshot"}else{"Record query failed"}}else{"Record browser connected"}.to_owned()))),
                text().class("subtle record-query-time").content(group.map(|g|g.page.as_ref().map(|p|format!("{} · {} · {}",formatting::timestamp(Some(p.sampled)),p.root,i18n::translate(&format!("Records {}–{}",if p.records.is_empty(){0}else{p.offset+1},p.offset+p.records.len() as u32)))).unwrap_or_default())),
                text().class(group.map(|g|if g.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(group.map(|g|g.error.clone())),
                row().class("script-actions").fill((
                    btn("Previous records",move||{let offset=state.with(|s|s.task_records.groups.iter().find(|g|g.profile.id==prev_id).and_then(|g|g.page.as_ref()).map(|p|p.offset.saturating_sub(100)).unwrap_or(0));previous.record_page(&prev_id,offset);}).enabled(state.map(move|s|s.task_records.groups.iter().find(|g|g.profile.id==prev_enabled_id).is_some_and(|g|g.secret.is_some()&&g.pending.is_none()&&g.page.as_ref().is_some_and(|p|p.offset>0)))),
                    btn("Refresh records",move||{let offset=state.with(|s|s.task_records.groups.iter().find(|g|g.profile.id==refresh_id).and_then(|g|g.page.as_ref()).map(|p|p.offset).unwrap_or(0));refresh.record_page(&refresh_id,offset);}).enabled(state.map(move|s|s.task_records.groups.iter().find(|g|g.profile.id==enabled_id).is_some_and(|g|g.secret.is_some()&&g.pending.is_none()))),
                    btn("Older records",move||{let offset=state.with(|s|s.task_records.groups.iter().find(|g|g.profile.id==next_id).and_then(|g|g.page.as_ref()).map(|p|p.offset+100).unwrap_or(0));next.record_page(&next_id,offset);}).enabled(state.map(move|s|s.task_records.groups.iter().find(|g|g.profile.id==next_enabled_id).is_some_and(|g|g.secret.is_some()&&g.pending.is_none()&&g.page.as_ref().is_some_and(|p|p.more)))),
                )),
                when(group.map(|g|g.page.as_ref().is_some_and(|p|p.records.is_empty())),||text().class("context-hint").content(i18n::bind("No saved server tasks on this page"))),
                col().class("server-record-list").fill(each(state.map(move|s|s.task_records.groups.iter().find(|g|g.profile.id==rows_id).and_then(|g|g.page.as_ref()).map(|p|p.records.clone()).unwrap_or_default())).key(|r|r.id.clone()).children(move|item|{
                    let record=item.value;let read=row_ui.clone();let server=id.clone();let record_id=record.get().id;
                    row().class("server-task-record").fill((
                        stops_ui::selection(row_ui.clone(),server.clone(),record_id.clone()),
                        col().class("record-copy").fill((text().class("record-name").content(record.map(|r|r.metadata.as_ref().map(|m|m.name.clone()).unwrap_or_else(||r.id.clone()))),text().class("record-id subtle").selectable(true).content(record.map(|r|r.id.clone())),text().class("subtle").content(record.map(|r|if let Some(metadata)=&r.metadata {format!("{} · {} · {} {}",i18n::translate(metadata.kind.label()),i18n::translate(&format!("Version {}",metadata.revision)),i18n::translate("Created"),formatting::timestamp(Some(metadata.created)))} else {format!("{} {}",i18n::translate("Record modified"),formatting::timestamp(Some(r.modified)))})),text().class("subtle").content(i18n::bind(record.map(|r|if r.invalid_metadata {"Record metadata is invalid; the task ID remains available"}else if r.metadata.is_none(){"Legacy record without metadata"}else{""}.to_owned()))))),
                        col().class("record-result").fill((text().class("record-status").content(i18n::bind(record.map(|r|r.label().to_owned()))),text().class("record-reason subtle").content(i18n::bind(record.map(|r|r.reason().to_owned()))))),
                        text().class("record-log-size subtle").content(record.map(|r|r.log_bytes.map(formatting::bytes).unwrap_or_else(||i18n::translate("Log unavailable")))),
                        btn("View saved log",move||read.read_record(server.clone(),record_id.clone())).class("button record-log-action").enabled(group.map(|g|g.secret.is_some()&&g.pending.is_none())),
                    ))
                })),
            ))
        }}),
    ))
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        stops_ui::dialog(ui.clone()),
        when(
            state.map(|s| s.dialog == Some(Dialog::TaskRecordConnect)),
            {
                let ui = ui.clone();
                move || {
                    let connect = ui.clone();
                    let cancel = ui.clone();
                    let all = ui.clone();
                    modal(ui.clone(),"Connect to server records",col().class("form-stack").fill((
                text().class("hint").content(i18n::bind("Select server accounts independently of the workspace. This only reads saved records and service state; it does not require SFTP or sudo.")),
                btn("Select trusted servers",move||all.state.revise(|s|s.task_records.selected=s.config.servers.iter().filter(|p|s.config.trusted_fingerprint(&p.host,p.port).is_some()).map(|p|p.id.clone()).collect())),
                each(state.map(|s|s.config.servers.clone())).key(|p|p.id.clone()).children(move|item|{
                    let profile=item.value.get();let id=profile.id.clone();let selected=id.clone();let visibility=id.clone();let secret_id=id.clone();let label=profile.name;
                    col().class("record-target-picker").fill((
                        btn(format!("{} · {}@{}:{}",label,profile.username,profile.host,profile.port),move||state.revise(|s|{if !s.task_records.selected.remove(&id){s.task_records.selected.insert(id.clone());}else{s.task_records.secrets.remove(&id);}})).enabled(state.map({let host=profile.host.clone();let port=profile.port;move|s|s.config.trusted_fingerprint(&host,port).is_some()})).class(state.map(move|s|if s.task_records.selected.contains(&selected){"record-target active"}else{"record-target"}.to_owned())),
                        when(state.map(move|s|s.task_records.selected.contains(&visibility)),move||{let id=secret_id.clone();let value_id=id.clone();col().class("field").fill((text().class("field-label").content(i18n::bind("SSH password or key passphrase")),input().purpose(Purpose::Password).name(i18n::bind("SSH password or key passphrase")).value(state.map(move|s|s.task_records.secrets.get(&value_id).cloned().unwrap_or_default())).on(value_changed,move|event|if let Event::ValueChanged {value,..}=event {state.revise(|s|{s.task_records.secrets.insert(id.clone(),value);});})))})
                    ))
                }),
            )),(btn("Cancel",move||cancel.close_dialog()),btn("Read server records",move||connect.browse_records()).class("button primary")))
                }
            },
        ),
        when(
            state.map(|s| s.dialog == Some(Dialog::TaskRecordLog)),
            move || {
                let close = ui.clone();
                let refresh = ui.clone();
                let export = ui.clone();
                let service_logs = ui.clone();
                let deleted_files = ui.clone();
                let tcp_listeners = ui.clone();
                modal(ui.clone(),"Saved server task log",col().class("form-stack record-log-dialog").fill((
                text().class("section-title").content(state.map(|s|s.task_records.viewer.as_ref().map(|v|v.record.clone()).unwrap_or_default())),
                text().class("subtle").content(state.map(|s|s.task_records.viewer.as_ref().and_then(|v|s.task_records.groups.iter().find(|g|g.profile.id==v.server)).map(|g|format!("{} · {}@{}:{}",g.profile.name,g.profile.username,g.profile.host,g.profile.port)).unwrap_or_default())),
                text().class("record-detail-status").content(i18n::bind(state.map(|s|s.task_records.viewer.as_ref().map(|v|if v.pending.is_some(){"Reading saved log…"}else{v.detail.as_ref().map(crate::recovery_ui::record_status).unwrap_or("")}.to_owned()).unwrap_or_default()))),
                text().class("subtle").content(state.map(|s|s.task_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|format!("{} · {} · {}",formatting::timestamp(Some(d.sampled)),d.root,d.record.exit_code.map(|code|i18n::translate(&format!("Saved exit code: {code}"))).unwrap_or_else(||i18n::translate("No confirmed exit result")))).unwrap_or_default())),
                text().class(state.map(|s|if s.task_records.viewer.as_ref().is_none_or(|v|v.error.is_empty()){ "hidden" }else{"form-error"}.to_owned())).content(state.map(|s|s.task_records.viewer.as_ref().map(|v|v.error.clone()).unwrap_or_default())),
                text().class("subtle").content(i18n::bind(state.map(|s|s.task_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|d.record.reason().to_owned()).unwrap_or_default()))),
                text().class("script-code installation-report").selectable(true).content(state.map(|s|s.task_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(crate::installation_ui::saved).unwrap_or_default())),
                text().class("script-code task-stop-report").selectable(true).content(state.map(|s|s.task_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|d.record.metadata.as_ref().map(|m|stop_report(&d.output,&d.record.id,&m.revision,m.stops.as_ref())).unwrap_or_default()).unwrap_or_default())),
                text().class("script-code drill-report").selectable(true).content(state.map(|s|s.task_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(crate::recovery_ui::saved).unwrap_or_default())),
                text().class("script-code record-saved-output").selectable(true).content(state.map(|s|s.task_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|if d.record.log_bytes.is_none(){i18n::translate("Log unavailable")}else if d.record.metadata.as_ref().is_some_and(|m|m.kind==arcnix_core::task_records::Kind::Package){arcnix_core::installers::human_log(&d.output)}else if d.record.metadata.as_ref().is_some_and(|m|m.kind==arcnix_core::task_records::Kind::RecoveryDrill){arcnix_core::recovery_drills::human_log(&d.output)}else if d.record.metadata.as_ref().is_some_and(|m|m.stops.is_some()){arcnix_core::task_stops::human_log(&d.output)}else{d.output.clone()}).unwrap_or_default())),
                fragment((
                    when(state.map(|s|record_deleted_target(s).is_some()),move||{let ui=deleted_files.clone();btn("Inspect deleted open files…",move||ui.read_record_deleted_files()).class("button diagnostic-deleted-files")}),
                    when(state.map(|s|record_tcp_target(s).is_some()),move||{let ui=tcp_listeners.clone();btn("Inspect TCP listeners…",move||ui.read_record_tcp_listeners()).class("button diagnostic-tcp-listeners")}),
                    text().class("hint").content(i18n::bind("This is a bounded tail of the saved server log. No job is started by viewing it; an unknown result remains unknown.")),
                )),
            )),(when(state.map(|s|record_log_context(s).is_some()),move||{let ui=service_logs.clone();btn("Failed service logs…",move||ui.read_record_service_logs()).class("button diagnostic-service-logs")}),when(state.map(|s|s.task_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).and_then(|d|d.record.metadata.as_ref()).is_some_and(|m|m.kind==records::Kind::Diagnostic)),move||{let export=export.clone();btn("Export diagnostic report…",move||export.export_record_diagnostics())}),btn("Close",move||close.close_dialog()),btn("Refresh saved log",move||{if let Some((server,id))=state.with(|s|s.task_records.viewer.as_ref().map(|v|(v.server.clone(),v.record.clone()))) {refresh.read_record(server,id);}}).enabled(state.map(|s|s.task_records.viewer.as_ref().is_some_and(|v|v.pending.is_none())))))
            },
        ),
    ))
}

pub(super) fn stop_report(
    output: &str,
    id: &str,
    revision: &str,
    expected: Option<&arcnix_core::task_stops::Description>,
) -> String {
    if expected.is_none() {
        return String::new();
    }
    match arcnix_core::task_stops::report(output, id, revision, expected) {
        Ok(Some(report)) => report
            .rows
            .iter()
            .map(|row| {
                format!(
                    "{} · {}{}",
                    row.id,
                    i18n::translate(match row.state.as_str() {
                        "stopped" => "Stopped",
                        "already_stopped" => "Already stopped",
                        "refused" => "Refused",
                        _ => "Result unknown",
                    }),
                    if row.reason.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " · {}",
                            i18n::translate(arcnix_core::task_stops::reason_label(&row.reason))
                        )
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => i18n::translate("No complete stop report; query the original server task"),
    }
}
