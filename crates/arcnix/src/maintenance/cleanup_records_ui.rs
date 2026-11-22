//! Session-only, global cleanup report inspection without a resident poller.
use arcnix_core::cleanup_records::{self as records, Query};
use zeroize::Zeroizing;

use crate::*;

#[path = "export_download_ui.rs"]
mod downloads;
#[path = "audit_storage_ui.rs"]
mod storage;

#[derive(Clone)]
struct Group {
    profile: ServerProfile,
    fingerprint: String,
    credentials: Option<(Zeroizing<String>, Zeroizing<String>)>,
    page: Option<Box<records::Page>>,
    detail: Option<Box<records::Detail>>,
    detail_id: Option<String>,
    release_export: Option<String>,
    pending: Option<u64>,
    error: String,
    storage: storage::GroupState,
}
#[derive(Clone, Default)]
pub(crate) struct ReportsState {
    pub visible: bool,
    selected: HashSet<String>,
    form: HashMap<String, (String, String)>,
    groups: Vec<Group>,
    serial: u64,
    error: String,
    storage: storage::StorageState,
    download: downloads::DownloadState,
}
pub(crate) fn dismiss(s: &mut State) {
    downloads::dismiss(s);
    if s.dialog == Some(Dialog::CleanupReportConnect) {
        s.cleanup_reports.form.clear();
    }
}
pub(crate) fn forget(s: &mut State) {
    downloads::forget(s);
    s.cleanup_reports.serial += 1;
    s.cleanup_reports.form.clear();
    for group in &mut s.cleanup_reports.groups {
        group.credentials = None;
        group.detail = None;
        group.detail_id = None;
        group.release_export = None;
        group.pending = None;
        group.storage = Default::default();
    }
    if s.dialog == Some(Dialog::CleanupReportConnect) {
        s.dialog = None;
    }
}
pub(crate) fn leave(s: &mut State) {
    forget(s);
    s.cleanup_reports.visible = false;
}
fn target(s: &State, id: &str) -> anyhow::Result<batch_worker::Target> {
    let group = s
        .cleanup_reports
        .groups
        .iter()
        .find(|g| g.profile.id == id)
        .ok_or_else(|| anyhow::anyhow!("Reconnect to cleanup reports"))?;
    let profile = s
        .config
        .available_servers()
        .find(|p| p.id == id)
        .ok_or_else(|| anyhow::anyhow!("Server inventory changed; reconnect to cleanup reports"))?;
    anyhow::ensure!(
        profile.host == group.profile.host
            && profile.port == group.profile.port
            && profile.username == group.profile.username
            && s.config.trusted_fingerprint(&profile.host, profile.port)
                == Some(group.fingerprint.as_str()),
        "Server identity changed; reconnect to cleanup reports"
    );
    let (secret, sudo) = group
        .credentials
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Reconnect to cleanup reports"))?;
    Ok(batch_worker::Target {
        profile: group.profile.clone(),
        fingerprint: group.fingerprint.clone(),
        secret,
        sudo,
    })
}
impl Ui {
    fn review_residual_recovery(&self, release: bool) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                !scripts_ui::busy(s),
                "Wait for the current operation to finish"
            );
            let mut targets = Vec::new();
            let mut peers = HashSet::new();
            for group in &s.cleanup_reports.groups {
                if group.pending.is_some() || !group.error.is_empty() || group.credentials.is_none()
                {
                    continue;
                }
                let Some(detail) = &group.detail else {
                    continue;
                };
                if !detail.exportable() {
                    continue;
                }
                let export =
                    if release {
                        let Some(copy) = detail.recoveries.iter().find(|r| {
                            group.release_export.as_ref() == Some(&r.id) && r.releasable()
                        }) else {
                            continue;
                        };
                        Some(copy)
                    } else {
                        None
                    };
                let bound = target(s, &group.profile.id)?;
                anyhow::ensure!(
                    peers.insert((bound.profile.host.to_lowercase(), bound.profile.port)),
                    "Choose each server endpoint only once"
                );
                targets.push(arcnix_core::record_recovery::TargetPlan {
                    id: bound.profile.id.clone(),
                    endpoint: format!(
                        "{}@{}:{}",
                        bound.profile.username, bound.profile.host, bound.profile.port
                    ),
                    fingerprint: bound.fingerprint,
                    plan: arcnix_core::record_recovery::Plan::new(detail, export)?,
                });
            }
            arcnix_core::record_recovery::Job { targets }.template()
        });
        match result {
            Err(error) => self
                .state
                .revise(|s| s.cleanup_reports.error = format!("{error:#}")),
            Ok(template) => {
                let selected = template
                    .recovery
                    .as_ref()
                    .unwrap()
                    .targets
                    .iter()
                    .map(|t| t.id.clone())
                    .collect();
                self.choose_script(template);
                self.state.revise(|s| s.scripts.targets = selected);
            }
        }
    }
    pub(crate) fn open_cleanup_reports(&self) {
        self.navigate(navigation::Page::Tasks);
        let connect = self.state.revise(|s| {
            task_records_ui::forget(s);
            record_maintenance_ui::leave(s);
            s.cleanup_reports.visible = true;
            if s.cleanup_reports.storage.visible {
                s.cleanup_reports.serial += 1;
                for group in &mut s.cleanup_reports.groups {
                    group.storage = Default::default();
                    group.pending = None;
                }
            }
            s.cleanup_reports.storage.visible = false;
            s.cleanup_reports.groups.is_empty()
        });
        if connect {
            self.connect_cleanup_reports();
        }
    }
    fn connect_cleanup_reports(&self) {
        self.state.revise(|s| {
            s.cleanup_reports.form.clear();
            s.cleanup_reports.selected = s
                .cleanup_reports
                .groups
                .iter()
                .map(|g| g.profile.id.clone())
                .collect();
        });
        self.open_dialog(Dialog::CleanupReportConnect);
    }
    fn load_cleanup_reports(&self) {
        let query = match self.state.with(storage::initial_query) {
            Ok(query) => query,
            Err(error) => {
                self.state.revise(|s| s.dialog_error = format!("{error:#}"));
                return;
            }
        };
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let mut targets = Vec::new();
            let mut peers = HashSet::new();
            for profile in s
                .config
                .available_servers()
                .filter(|p| s.cleanup_reports.selected.contains(&p.id))
            {
                anyhow::ensure!(
                    peers.insert((profile.host.to_lowercase(), profile.port)),
                    "Choose each server endpoint only once"
                );
                let fingerprint = s
                    .config
                    .trusted_fingerprint(&profile.host, profile.port)
                    .ok_or_else(|| anyhow::anyhow!("Connect and save the host fingerprint first"))?
                    .to_owned();
                let (secret, sudo) = s
                    .cleanup_reports
                    .form
                    .get(&profile.id)
                    .cloned()
                    .unwrap_or_default();
                anyhow::ensure!(
                    [&secret, &sudo]
                        .iter()
                        .all(|v| v.len() <= 1024 && !v.contains(['\0', '\r', '\n'])),
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
                    sudo: Zeroizing::new(sudo),
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
                    let previous = std::mem::take(&mut s.cleanup_reports.groups);
                    s.cleanup_reports.error.clear();
                    targets
                        .into_iter()
                        .map(|target| {
                            let prior = previous.iter().find(|g| {
                                g.profile.id == target.profile.id
                                    && g.profile.host == target.profile.host
                                    && g.profile.port == target.profile.port
                                    && g.profile.username == target.profile.username
                                    && g.fingerprint == target.fingerprint
                            });
                            s.cleanup_reports.serial += 1;
                            let serial = s.cleanup_reports.serial;
                            s.cleanup_reports.groups.push(Group {
                                profile: target.profile.clone(),
                                fingerprint: target.fingerprint.clone(),
                                credentials: Some((target.secret.clone(), target.sudo.clone())),
                                page: prior.and_then(|g| g.page.clone()),
                                detail: None,
                                detail_id: None,
                                release_export: None,
                                pending: Some(serial),
                                error: String::new(),
                                storage: Default::default(),
                            });
                            (serial, target)
                        })
                        .collect::<Vec<_>>()
                });
                for (serial, target) in requests {
                    batch_worker::cleanup_records(
                        serial,
                        target,
                        query.clone(),
                        self.batch_events.clone(),
                    );
                }
            }
        }
    }
    fn query_cleanup_reports(&self, id: &str, query: Query) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let group = s
                .cleanup_reports
                .groups
                .iter()
                .find(|g| g.profile.id == id)
                .ok_or_else(|| anyhow::anyhow!("Reconnect to cleanup reports"))?;
            anyhow::ensure!(group.pending.is_none(), "Wait for the report query");
            if let Query::Detail(record) = &query {
                anyhow::ensure!(
                    group.page.as_ref().is_some_and(|p| p
                        .records
                        .iter()
                        .any(|r| &r.id == record && r.issue.is_none())),
                    "Reload cleanup reports before inspecting this record"
                );
            }
            if let Query::AuditDetail(record) = &query {
                anyhow::ensure!(
                    s.cleanup_reports.storage.visible
                        && s.cleanup_reports.storage.scope
                            == arcnix_core::audit_storage::Scope::Maintenance
                        && (group.storage.detail_id.as_ref() == Some(record)
                            || group.storage.page.as_ref().is_some_and(|page| page
                                .records
                                .iter()
                                .any(|row| &row.id == record))),
                    "Reload audit cleanup results before inspecting this record"
                );
            }
            target(s, id)
        });
        match result {
            Err(error) => self
                .state
                .revise(|s| s.cleanup_reports.error = format!("{error:#}")),
            Ok(target) => {
                let serial = self.state.revise(|s| {
                    s.cleanup_reports.serial += 1;
                    let serial = s.cleanup_reports.serial;
                    let group = s
                        .cleanup_reports
                        .groups
                        .iter_mut()
                        .find(|g| g.profile.id == id)
                        .unwrap();
                    if let Query::Detail(record) = &query {
                        if group.detail_id.as_ref() != Some(record) {
                            group.detail = None;
                        }
                        group.detail_id = Some(record.clone());
                    }
                    group.storage.detail_pending = None;
                    if let Query::AuditDetail(record) = &query {
                        if group.storage.detail_id.as_ref() != Some(record) {
                            group.storage.detail = None;
                        }
                        group.storage.detail_id = Some(record.clone());
                        group.storage.detail_pending = Some(serial);
                    }
                    group.pending = Some(serial);
                    group.error.clear();
                    serial
                });
                batch_worker::cleanup_records(serial, target, query, self.batch_events.clone());
            }
        }
    }
}
pub(crate) fn event(ui: &Ui, event: batch_worker::Event) {
    if let batch_worker::Event::ExportDownload {
        serial,
        server,
        update,
    } = event
    {
        downloads::event(ui, serial, &server, update);
        return;
    }
    if let batch_worker::Event::CleanupRecords { serial, id, result } = event {
        ui.state.revise(|s| {
            if let Some(group) = s.cleanup_reports.groups.iter_mut().find(|g| {
                g.profile.id == id && g.pending == Some(serial) && g.credentials.is_some()
            }) {
                group.pending = None;
                match result {
                    Ok(batch_worker::CleanupResponse::AuditDetail(detail)) => {
                        if s.cleanup_reports.storage.visible
                            && s.cleanup_reports.storage.scope
                                == arcnix_core::audit_storage::Scope::Maintenance
                            && group.storage.detail_pending == Some(serial)
                            && group.storage.detail_id.as_ref() == Some(&detail.id)
                        {
                            group.storage.detail = Some(detail);
                            group.error.clear();
                        }
                    }
                    Ok(batch_worker::CleanupResponse::Page(page)) => {
                        group.page = Some(page);
                        group.detail = None;
                        group.detail_id = None;
                        group.release_export = None;
                        group.error.clear();
                    }
                    Ok(batch_worker::CleanupResponse::Storage(view)) => {
                        if s.cleanup_reports.storage.visible
                            && view.scope == s.cleanup_reports.storage.scope
                            && (!view.preview_only
                                || storage::current_policy(
                                    &s.cleanup_reports.storage,
                                    &view.policy,
                                ))
                        {
                            if view.preview_only {
                                group.storage.preview = Some(view);
                            } else {
                                group.storage.page = Some(view);
                            }
                            group.error.clear();
                        }
                    }
                    Ok(batch_worker::CleanupResponse::Detail(detail)) => {
                        if !detail.exportable()
                            || !detail.recoveries.iter().any(|r| {
                                group.release_export.as_ref() == Some(&r.id) && r.releasable()
                            })
                        {
                            group.release_export = None;
                        }
                        group.detail = Some(detail);
                        group.error.clear();
                    }
                    Err(error) => group.error = error,
                }
                if group.storage.detail_pending == Some(serial) {
                    group.storage.detail_pending = None;
                }
            }
        });
    }
}
fn observation(location: &records::Location) -> String {
    let label = i18n::translate(location.label());
    match (location.allocated_bytes, location.logical_bytes) {
        (Some(a), Some(l)) => format!(
            "{label} · {} / {}",
            formatting::bytes(a),
            formatting::bytes(l)
        ),
        _ => label,
    }
}
fn body(detail: &records::Detail) -> String {
    let mut lines = vec![
        i18n::translate(detail.scope.label()),
        format!("UID {} · {}", detail.account.uid, detail.account.home),
        detail.record_root.clone(),
        detail.report_path.clone(),
        detail.quarantine_path.clone(),
    ];
    lines.extend(detail.issues.iter().map(|v| i18n::translate(v)));
    if let Some(error) = &detail.error {
        lines.push(error.clone());
    }
    for record in &detail.records {
        lines.push(format!(
            "\n{} · {}",
            record.id,
            record.saved_phase.as_deref().unwrap_or("not reached")
        ));
        if record.quarantine_present == Some(true) {
            lines.push(i18n::translate(if record.directory_proven {
                "Quarantined directory identity is proven"
            } else {
                "Quarantined directory identity cannot be proven"
            }));
        }
        for object in &record.objects {
            let saved = if object.saved_removed {
                "Saved: removed"
            } else if object.saved_pending {
                "Saved: unlink pending; actual outcome needs inspection"
            } else {
                "No removal recorded"
            };
            lines.push(format!(
                "{} · {}\n  {}: {}\n  {}: {}",
                object.name,
                i18n::translate(saved),
                i18n::translate("Original location"),
                observation(&object.normal),
                i18n::translate("Quarantine location"),
                observation(&object.quarantine)
            ));
        }
    }
    lines.join("\n")
}
pub(crate) fn tab(ui: Ui) -> impl Widget {
    let state = ui.state;
    btn("Cleanup reports", move || ui.open_cleanup_reports()).class(state.map(|s| {
        if s.cleanup_reports.visible {
            "choice active"
        } else {
            "choice"
        }
        .to_owned()
    }))
}
pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let audit = ui.clone();
    let reports = ui.clone();
    col().fill((
        when(
            state.map(|s| s.cleanup_reports.storage.visible),
            move || storage::page(audit.clone()),
        ),
        when(
            state.map(|s| !s.cleanup_reports.storage.visible),
            move || reports_page(reports.clone()),
        ),
    ))
}
fn reports_page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let connect = ui.clone();
    let forget_ui = ui.clone();
    let export_ui = ui.clone();
    let release_ui = ui.clone();
    let audit_ui = ui.clone();
    col().class("cleanup-reports-page").fill((
        row().class("global-toolbar").fill((btn("Connect to cleanup reports…",move||connect.connect_cleanup_reports()).class("button primary"),btn("Forget report credentials",move||forget_ui.state.revise(forget)))),
        row().class("script-actions").fill((btn("Audit and export storage…",move||storage::open(&audit_ui)).class("audit-storage-open").enabled(state.map(|s|s.cleanup_reports.groups.iter().all(|g|g.pending.is_none()))),crate::help_ui::tip("Cleanup reports","Queries only read metadata. Review residual exports separately, then select a completed copy for a new release review. Previously deleted files cannot be recovered. No task resumes automatically."))),
        row().class("script-actions").fill((
            btn("Review residual exports…",move||export_ui.review_residual_recovery(false)).class("button residual-export-review").enabled(state.map(|s|!scripts_ui::busy(s)&&s.cleanup_reports.groups.iter().any(|g|g.credentials.is_some()&&g.pending.is_none()&&g.error.is_empty()&&g.detail.as_ref().is_some_and(|d|d.exportable())))),
            btn("Review quarantine removal…",move||release_ui.review_residual_recovery(true)).class("button residual-release-review").enabled(state.map(|s|!scripts_ui::busy(s)&&s.cleanup_reports.groups.iter().any(|g|g.credentials.is_some()&&g.pending.is_none()&&g.error.is_empty()&&g.detail.as_ref().is_some_and(|d|d.exportable()&&d.recoveries.iter().any(|r|g.release_export.as_ref()==Some(&r.id)&&r.releasable()))))),
        )),
        text().class(state.map(|s|if s.cleanup_reports.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(state.map(|s|s.cleanup_reports.error.clone())),
        each(state.map(|s|s.cleanup_reports.groups.clone())).key(|g|g.profile.id.clone()).children({let ui=ui.clone();move|item|{
            let group=item.value;let id=group.get().profile.id;let previous_id=id.clone();let next_id=id.clone();let refresh_id=id.clone();let detail_id=id.clone();let close_id=id.clone();let recoveries_id=id.clone();let previous=ui.clone();let next=ui.clone();let refresh=ui.clone();let detail_refresh=ui.clone();let rows_id=id.clone();let inspect=ui.clone();let download_ui=ui.clone();
            col().class("cleanup-report-group").fill((
                text().class("section-title").content(group.map(|g|g.profile.name.clone())),
                text().class("subtle").content(group.map(|g|format!("{}@{}:{} · {}",g.profile.username,g.profile.host,g.profile.port,g.fingerprint))),
                text().class("cleanup-query-state subtle").content(i18n::bind(group.map(|g|if g.pending.is_some(){"Reading cleanup reports…"}else if g.credentials.is_none(){"Disconnected; reconnect to cleanup reports"}else if !g.error.is_empty(){"Report query failed; previous snapshots are retained"}else{"Cleanup report query completed"}.to_owned()))),
                text().class("cleanup-page-summary subtle").content(group.map(|g|g.page.as_ref().map(|p|format!("{} · {} · {} {} · {} {}",formatting::timestamp(Some(p.sampled)),p.root,i18n::translate("Cleanup reports"),p.record_count,i18n::translate("Audit record storage"),formatting::bytes(p.allocated_bytes))).unwrap_or_default())),
                when(group.map(|g|g.page.as_ref().is_some_and(|p|!p.capacity_complete)),||text().class("hint").content(i18n::bind("Audit storage measurement is incomplete; quarantines and account task directories are not included."))),
                text().class(group.map(|g|if g.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(group.map(|g|g.error.clone())),
                row().class("script-actions").fill((
                    btn("Previous reports",move||{let offset=state.with(|s|s.cleanup_reports.groups.iter().find(|g|g.profile.id==previous_id).and_then(|g|g.page.as_ref()).map(|p|p.offset.saturating_sub(100)).unwrap_or(0));previous.query_cleanup_reports(&previous_id,Query::List(offset));}).enabled(group.map(|g|g.credentials.is_some()&&g.pending.is_none()&&g.page.as_ref().is_some_and(|p|p.offset>0))),
                    btn("Refresh reports",move||{let offset=state.with(|s|s.cleanup_reports.groups.iter().find(|g|g.profile.id==refresh_id).and_then(|g|g.page.as_ref()).map(|p|p.offset).unwrap_or(0));refresh.query_cleanup_reports(&refresh_id,Query::List(offset));}).enabled(group.map(|g|g.credentials.is_some()&&g.pending.is_none())),
                    btn("More reports",move||{let offset=state.with(|s|s.cleanup_reports.groups.iter().find(|g|g.profile.id==next_id).and_then(|g|g.page.as_ref()).map(|p|p.offset+100).unwrap_or(0));next.query_cleanup_reports(&next_id,Query::List(offset));}).enabled(group.map(|g|g.credentials.is_some()&&g.pending.is_none()&&g.page.as_ref().is_some_and(|p|p.more))),
                )),
                col().class("cleanup-report-list").fill(each(group.map(|g|g.page.as_ref().map(|p|p.records.clone()).unwrap_or_default())).key(|r|r.id.clone()).children({move|item|{
                    let record=item.value;let record_id=record.get().id;let gate_record=record_id.clone();let server=rows_id.clone();let gate_server=server.clone();let inspect=inspect.clone();
                    row().class("cleanup-report-row").fill((
                        col().class("cleanup-report-copy").fill((text().class("cleanup-report-id").content(record.map(|r|r.id.clone())),text().class("cleanup-report-phase subtle").content(i18n::bind(record.map(|r|r.phase_label().to_owned()))),text().class("cleanup-report-service subtle").content(i18n::bind(record.map(|r|r.state_label().to_owned()))),text().class("subtle").content(record.map(|r|format!("{} · {} {}/{} · {} {}",formatting::timestamp(Some(r.modified)),i18n::translate("Records processed"),r.processed_count,r.selected_count.map(|n|n.to_string()).unwrap_or_else(||"?".into()),i18n::translate("Files marked removed"),r.removed_files))))),
                        btn("Inspect cleanup report",move||inspect.query_cleanup_reports(&server,Query::Detail(record_id.clone()))).enabled(state.map(move|s|s.cleanup_reports.groups.iter().find(|g|g.profile.id==gate_server).is_some_and(|g|g.credentials.is_some()&&g.pending.is_none()&&g.page.as_ref().is_some_and(|p|p.records.iter().any(|r|r.id==gate_record&&r.issue.is_none()))))),
                    ))
                }})),
                when(group.map(|g|g.detail.is_some()),move||{let detail_id=detail_id.clone();let close_id=close_id.clone();let detail_refresh=detail_refresh.clone();let recoveries_id=recoveries_id.clone();let download_ui=download_ui.clone();col().class("cleanup-detail").fill((
                    text().class("cleanup-detail-server section-title").content(group.map(|g|format!("{} · {}@{}:{}",g.profile.name,g.profile.username,g.profile.host,g.profile.port))),
                    text().class("cleanup-detail-state subtle").content(group.map(|g|g.detail.as_ref().map(|d|format!("{} · {}",i18n::translate(d.record.phase_label()),i18n::translate(d.record.state_label()))).unwrap_or_default())),
                    row().class("script-actions").fill((btn("Refresh report details",move||{if let Some(id)=state.with(|s|s.cleanup_reports.groups.iter().find(|g|g.profile.id==detail_id).and_then(|g|g.detail_id.clone())){detail_refresh.query_cleanup_reports(&detail_id,Query::Detail(id));}}).enabled(group.map(|g|g.credentials.is_some()&&g.pending.is_none())),btn("Close report details",move||state.revise(|s|{if let Some(g)=s.cleanup_reports.groups.iter_mut().find(|g|g.profile.id==close_id){g.detail=None;g.detail_id=None;g.release_export=None;g.pending=None;}})))),
                    text().class("cleanup-detail-sampled subtle").content(group.map(|g|g.detail.as_ref().map(|d|formatting::timestamp(Some(d.sampled))).unwrap_or_default())),
                    text().class("cleanup-detail-body script-code").selectable(true).content(group.map(|g|g.detail.as_ref().map(|d|body(d)).unwrap_or_default())),
                    col().class("residual-recovery-list").fill(each(group.map(|g|g.detail.as_ref().map(|d|d.recoveries.clone()).unwrap_or_default())).key(|r|r.id.clone()).children(move|item|{
                        let recovery=item.value;let chosen=recovery.get().id;let selected_id=chosen.clone();let gate_id=chosen.clone();let server=recoveries_id.clone();let gate_server=server.clone();let choice_server=server.clone();let download_ui=download_ui.clone();let download_server=server.clone();let download_id=chosen.clone();
                        col().class("residual-recovery-row").fill((
                            text().class("residual-recovery-id").content(recovery.map(|r|format!("{} · {}",i18n::translate(if r.action=="export"{"Residual export"}else{"Residual release"}),r.id))),
                            text().class("residual-recovery-state subtle").content(recovery.map(|r|format!("{} · {} · {} · {}",r.phase,r.service,r.file_count,formatting::bytes(r.logical_bytes)))),
                            text().class("subtle").content(recovery.map(|r|r.destination.clone())),
                            btn("Download export…",move||downloads::open(&download_ui,&download_server,&download_id)).class("button export-download-open").enabled(group.map(move|g|g.credentials.is_some()&&g.pending.is_none()&&recovery.get().releasable())),
                            text().class("hint").content(recovery.map(|r|r.issue.clone().unwrap_or_default())),
                            btn("Use this verified export",move||state.revise(|s|{if let Some(g)=s.cleanup_reports.groups.iter_mut().find(|g|g.profile.id==server){if g.release_export.as_ref()==Some(&chosen){g.release_export=None;}else{g.release_export=Some(chosen.clone());}}})).class(state.map(move|s|if s.cleanup_reports.groups.iter().find(|g|g.profile.id==choice_server).is_some_and(|g|g.release_export.as_ref()==Some(&selected_id)){"button residual-release-select active"}else{"button residual-release-select"}.to_owned())).enabled(state.map(move|s|s.cleanup_reports.groups.iter().find(|g|g.profile.id==gate_server).is_some_and(|g|g.credentials.is_some()&&g.pending.is_none()&&g.error.is_empty()&&g.detail.as_ref().is_some_and(|d|d.exportable()&&d.recoveries.iter().any(|r|r.id==gate_id&&r.releasable()))))),
                        ))
                    })),
                ))}),
            ))
        }}),
    ))
}
pub(crate) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::CleanupReportConnect)),
        move || {
            let cancel = ui.clone();
            let connect = ui.clone();
            modal(ui.clone(),"Connect to cleanup reports",col().class("form-stack").fill((
            text().class("hint").content(i18n::bind("Select servers independently of the workspace. Reading cleanup reports requires Python 3.8+ and root or sudo, without SFTP. Credentials remain only in page memory.")),
            each(state.map(|s|s.config.available_servers().cloned().collect::<Vec<_>>())).key(|p|p.id.clone()).children(move|item|{
                let profile=item.value.get();let id=profile.id.clone();let selected=id.clone();let visible=id.clone();let secret=id.clone();let sudo=id.clone();
                let trusted=state.with(|s|s.config.trusted_fingerprint(&profile.host,profile.port).is_some());
                col().class("cleanup-report-target-picker").fill((
                    btn(format!("{} · {}@{}:{}",profile.name,profile.username,profile.host,profile.port),move||state.revise(|s|{if !s.cleanup_reports.selected.remove(&id){s.cleanup_reports.selected.insert(id.clone());}else{s.cleanup_reports.form.remove(&id);}})).class(state.map(move|s|if s.cleanup_reports.selected.contains(&selected){"cleanup-report-target active"}else{"cleanup-report-target"}.to_owned())).enabled(trusted),
                    when(state.map(move|s|s.cleanup_reports.selected.contains(&visible)),move||{
                        let secret=secret.clone();let sudo=sudo.clone();col().class("form-stack").fill((
                            text().class("field-label").content(i18n::bind("SSH password or key passphrase")),input().purpose(Purpose::Password).name(i18n::bind("SSH password or key passphrase")).on(value_changed,move|e|{if let Event::ValueChanged{value,..}=e{state.revise(|s|s.cleanup_reports.form.entry(secret.clone()).or_default().0=value.clone());}}),
                            text().class("field-label").content(i18n::bind("Administrator password (optional)")),input().purpose(Purpose::Password).name(i18n::bind("Administrator password (optional)")).on(value_changed,move|e|{if let Event::ValueChanged{value,..}=e{state.revise(|s|s.cleanup_reports.form.entry(sudo.clone()).or_default().1=value.clone());}}),
                        ))
                    }),
                ))
            }),
        )),(btn("Cancel",move||cancel.close_dialog()),btn("Read cleanup reports",move||connect.load_cleanup_reports()).class("button primary")))
        },
    )
}

// The download owns no persistent task or server-side mutation.
pub(crate) fn download_dialog(ui: Ui) -> impl Widget {
    downloads::dialog(ui)
}
