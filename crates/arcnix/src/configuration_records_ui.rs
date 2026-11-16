//! Private recovery records are queried on demand and never become local history.
use arcnix_core::configuration_records::{Detail, Page};
use zeroize::Zeroizing;

use super::*;

#[derive(Clone)]
struct Group {
    profile: ServerProfile,
    fingerprint: String,
    credentials: Option<(Zeroizing<String>, Zeroizing<String>)>,
    page: Option<Page>,
    pending: Option<u64>,
    error: String,
}
#[derive(Clone)]
struct Viewer {
    server: String,
    id: String,
    pending: Option<u64>,
    detail: Option<Box<Detail>>,
    error: String,
}
#[derive(Clone, Default)]
pub(super) struct RecordState {
    pub visible: bool,
    selected: HashSet<String>,
    form: HashMap<String, (String, String)>,
    groups: Vec<Group>,
    viewer: Option<Viewer>,
    serial: u64,
    error: String,
}
pub(super) fn dismiss(s: &mut State) {
    if s.dialog == Some(Dialog::ConfigurationRecordConnect) {
        s.configuration_records.form.clear();
    }
}
pub(super) fn forget(s: &mut State) {
    let records = &mut s.configuration_records;
    records.serial += 1;
    records.form.clear();
    records.viewer = None;
    for group in &mut records.groups {
        group.credentials = None;
        group.pending = None;
    }
    if matches!(
        s.dialog,
        Some(Dialog::ConfigurationRecordConnect | Dialog::ConfigurationRecordDetail)
    ) {
        s.dialog = None;
    }
}
fn credentials(s: &State, id: &str) -> anyhow::Result<batch_worker::Target> {
    let group = s
        .configuration_records
        .groups
        .iter()
        .find(|g| g.profile.id == id)
        .ok_or_else(|| anyhow::anyhow!("Reconnect to recovery records"))?;
    let profile = s
        .config
        .servers
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| {
            anyhow::anyhow!("Server inventory changed; reconnect to recovery records")
        })?;
    anyhow::ensure!(
        profile.host == group.profile.host
            && profile.port == group.profile.port
            && profile.username == group.profile.username
            && s.config.trusted_fingerprint(&profile.host, profile.port)
                == Some(group.fingerprint.as_str()),
        "Server identity changed; reconnect to recovery records"
    );
    let (secret, sudo) = group
        .credentials
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Reconnect to recovery records"))?;
    Ok(batch_worker::Target {
        profile: group.profile.clone(),
        fingerprint: group.fingerprint.clone(),
        secret,
        sudo,
    })
}
impl Ui {
    fn connect_configuration_records(&self) {
        self.state.revise(|s| {
            s.configuration_records.selected = s
                .configuration_records
                .groups
                .iter()
                .map(|g| g.profile.id.clone())
                .collect();
            s.configuration_records.form.clear();
        });
        self.open_dialog(Dialog::ConfigurationRecordConnect);
    }
    fn browse_configuration_records(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let mut targets = Vec::new();
            let mut endpoints = HashSet::new();
            for profile in s
                .config
                .servers
                .iter()
                .filter(|p| s.configuration_records.selected.contains(&p.id))
            {
                anyhow::ensure!(
                    endpoints.insert((profile.host.to_lowercase(), profile.port)),
                    "Select each server endpoint only once"
                );
                let fingerprint = s
                    .config
                    .trusted_fingerprint(&profile.host, profile.port)
                    .ok_or_else(|| anyhow::anyhow!("Connect and save the host fingerprint first"))?
                    .to_owned();
                let (secret, sudo) = s
                    .configuration_records
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
                    let previous = std::mem::take(&mut s.configuration_records.groups);
                    s.configuration_records.visible = true;
                    s.configuration_records.error.clear();
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
                            s.configuration_records.serial += 1;
                            let serial = s.configuration_records.serial;
                            s.configuration_records.groups.push(Group {
                                profile: target.profile.clone(),
                                fingerprint: target.fingerprint.clone(),
                                credentials: Some((target.secret.clone(), target.sudo.clone())),
                                page: prior.and_then(|g| g.page.clone()),
                                pending: Some(serial),
                                error: String::new(),
                            });
                            (serial, target)
                        })
                        .collect::<Vec<_>>()
                });
                for (serial, target) in requests {
                    batch_worker::configuration_record_page(
                        serial,
                        target,
                        0,
                        self.batch_events.clone(),
                    );
                }
            }
        }
    }
    fn configuration_record_page(&self, id: &str, offset: u32) {
        let result = self.state.with(|s| credentials(s, id));
        match result {
            Err(error) => self
                .state
                .revise(|s| s.configuration_records.error = format!("{error:#}")),
            Ok(target) => {
                let serial = self.state.revise(|s| {
                    s.configuration_records.serial += 1;
                    let serial = s.configuration_records.serial;
                    let group = s
                        .configuration_records
                        .groups
                        .iter_mut()
                        .find(|g| g.profile.id == id)
                        .unwrap();
                    group.pending = Some(serial);
                    group.error.clear();
                    serial
                });
                batch_worker::configuration_record_page(
                    serial,
                    target,
                    offset,
                    self.batch_events.clone(),
                );
            }
        }
    }
    fn inspect_configuration_record(&self, server: String, id: String) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let target = credentials(s, &server)?;
            anyhow::ensure!(
                s.configuration_records
                    .groups
                    .iter()
                    .find(|g| g.profile.id == server)
                    .and_then(|g| g.page.as_ref())
                    .is_some_and(|p| p.records.iter().any(|r| r.id == id)),
                "Reload recovery records before inspecting this version"
            );
            Ok(target)
        });
        match result {
            Err(error) => self
                .state
                .revise(|s| s.configuration_records.error = format!("{error:#}")),
            Ok(target) => {
                let serial = self.state.revise(|s| {
                    s.configuration_records.serial += 1;
                    let serial = s.configuration_records.serial;
                    let previous = s
                        .configuration_records
                        .viewer
                        .take()
                        .filter(|v| v.server == server && v.id == id)
                        .and_then(|v| v.detail);
                    s.configuration_records.viewer = Some(Viewer {
                        server,
                        id: id.clone(),
                        pending: Some(serial),
                        detail: previous,
                        error: String::new(),
                    });
                    serial
                });
                self.open_dialog(Dialog::ConfigurationRecordDetail);
                batch_worker::configuration_record_detail(
                    serial,
                    target,
                    id,
                    self.batch_events.clone(),
                );
            }
        }
    }
    fn restore_configuration_record(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(!scripts_ui::busy(s), "Wait for the current task");
            let viewer = s
                .configuration_records
                .viewer
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Inspect a recovery record first"))?;
            let target = credentials(s, &viewer.server)?;
            anyhow::ensure!(viewer.pending.is_none(), "Wait for the record query");
            anyhow::ensure!(
                viewer.error.is_empty(),
                "Refresh and verify the recovery record before restoring"
            );
            let detail = viewer
                .detail
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Inspect a recovery record first"))?;
            Ok((detail.restore_job()?.template()?, target.profile.id))
        });
        match result {
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
            Ok((template, id)) => {
                self.state.revise(forget);
                self.choose_script(template);
                self.state.revise(|s| {
                    s.scripts.concurrency = 1;
                    s.scripts.targets = std::iter::once(id).collect();
                });
            }
        }
    }
}
pub(super) fn event(ui: &Ui, event: batch_worker::Event) {
    ui.state.revise(|s| match event {
        batch_worker::Event::ConfigurationRecordPage { serial, id, result } => {
            if let Some(group) = s
                .configuration_records
                .groups
                .iter_mut()
                .find(|g| g.profile.id == id && g.pending == Some(serial))
            {
                group.pending = None;
                match result {
                    Ok(page) => {
                        group.page = Some(page);
                        group.error.clear();
                    }
                    Err(error) => group.error = error,
                }
            }
        }
        batch_worker::Event::ConfigurationRecordDetail { serial, id, result } => {
            if let Some(viewer) = s
                .configuration_records
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
    let compare = ui.clone();
    row().class("global-toolbar").fill((
        btn("Recent configuration changes", move || {
            local.state.revise(|s| {
                forget(s);
                s.configuration_records.visible = false;
                configuration_compare_ui::hide(s);
            })
        })
        .class(state.map(|s| {
            if s.configuration_records.visible || s.configuration_compare.visible {
                "choice"
            } else {
                "choice active"
            }
            .to_owned()
        })),
        btn("Server recovery records", move || {
            let empty = remote
                .state
                .with(|s| s.configuration_records.groups.is_empty());
            remote.state.revise(|s| {
                configuration_compare_ui::hide(s);
                s.configuration_records.visible = true;
            });
            if empty {
                remote.connect_configuration_records();
            }
        })
        .class(state.map(|s| {
            if s.configuration_records.visible {
                "choice active"
            } else {
                "choice"
            }
            .to_owned()
        })),
        btn("Compare current files", move || {
            compare.state.revise(|s| {
                forget(s);
                s.configuration_records.visible = false;
                s.configuration_compare.visible = true;
            })
        })
        .class(state.map(|s| {
            if s.configuration_compare.visible {
                "choice active"
            } else {
                "choice"
            }
            .to_owned()
        })),
    ))
}
pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let connect = ui.clone();
    let clear = ui.clone();
    let storage_ui = ui.clone();
    col().class("configuration-records-page").fill((
        row().class("global-toolbar").fill((btn("Connect to recovery records…",move||connect.connect_configuration_records()).class("button primary"),btn("Forget recovery credentials",move||clear.state.revise(forget)),btn("Inspect storage and cleanup…",move||storage_ui.open_record_storage(arcnix_core::record_maintenance::Scope::Configuration,None)))),
        row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Server recovery records")),crate::help_ui::group("Server recovery records",&["Browse root-private recovery versions without local task history. Reading does not publish, reload or restore configuration; Python 3.8+ and root or sudo are required, SFTP is not.","Credentials stay in memory until leaving Configurations or forgetting them. In-flight reads may finish later. Phases describe saved checks; current application health is not retested."]))),
        text().class(state.map(|s|if s.configuration_records.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(state.map(|s|s.configuration_records.error.clone())),
        each(state.map(|s|s.configuration_records.groups.clone())).key(|g|g.profile.id.clone()).children({let ui=ui.clone();move|item|{
            let group=item.value;let id=group.get().profile.id;let rows_id=id.clone();let previous_id=id.clone();let refresh_id=id.clone();let next_id=id.clone();let previous=ui.clone();let refresh=ui.clone();let next=ui.clone();let prev_gate=id.clone();let refresh_gate=id.clone();let next_gate=id.clone();let inspect=ui.clone();
            col().class("configuration-record-group").fill((
                text().class("section-title").content(group.map(|g|g.profile.name.clone())),
                text().class("subtle").content(group.map(|g|format!("{}@{}:{} · {}",g.profile.username,g.profile.host,g.profile.port,g.fingerprint))),
                text().class("subtle").content(i18n::bind(group.map(|g|if g.pending.is_some(){"Reading recovery records…"}else if g.credentials.is_none(){"Disconnected; reconnect to read recovery records"}else if !g.error.is_empty(){if g.page.is_some(){"Record query failed; showing the previous snapshot"}else{"Record query failed"}}else{"Record browser connected"}.to_owned()))),
                text().class("recovery-snapshot subtle").content(group.map(|g|g.page.as_ref().map(|p|format!("{} · {} · {}",formatting::timestamp(Some(p.sampled)),p.root,i18n::translate(&format!("Records {}–{}",if p.records.is_empty(){0}else{p.offset+1},p.offset+p.records.len() as u32)))).unwrap_or_default())),
                text().class("recovery-capacity subtle").content(group.map(|g|g.page.as_ref().map(|p|i18n::translate(&format!("{} records · {} allocated in record directories",p.record_count,formatting::bytes(p.allocated_bytes)))).unwrap_or_default())),
                when(group.map(|g|g.page.as_ref().is_some_and(|p|!p.capacity_complete)),||text().class("hint").content(i18n::bind("Capacity is incomplete: linked, nested or unreadable entries were excluded. Shared locks are not included."))),
                text().class(group.map(|g|if g.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(group.map(|g|g.error.clone())),
                row().class("script-actions").fill((
                    btn("Previous records",move||{let offset=state.with(|s|s.configuration_records.groups.iter().find(|g|g.profile.id==previous_id).and_then(|g|g.page.as_ref()).map(|p|p.offset.saturating_sub(100)).unwrap_or(0));previous.configuration_record_page(&previous_id,offset);}).enabled(state.map(move|s|s.configuration_records.groups.iter().find(|g|g.profile.id==prev_gate).is_some_and(|g|g.credentials.is_some()&&g.pending.is_none()&&g.page.as_ref().is_some_and(|p|p.offset>0)))),
                    btn("Refresh records",move||{let offset=state.with(|s|s.configuration_records.groups.iter().find(|g|g.profile.id==refresh_id).and_then(|g|g.page.as_ref()).map(|p|p.offset).unwrap_or(0));refresh.configuration_record_page(&refresh_id,offset);}).enabled(state.map(move|s|s.configuration_records.groups.iter().find(|g|g.profile.id==refresh_gate).is_some_and(|g|g.credentials.is_some()&&g.pending.is_none()))),
                    btn("Older records",move||{let offset=state.with(|s|s.configuration_records.groups.iter().find(|g|g.profile.id==next_id).and_then(|g|g.page.as_ref()).map(|p|p.offset+100).unwrap_or(0));next.configuration_record_page(&next_id,offset);}).enabled(state.map(move|s|s.configuration_records.groups.iter().find(|g|g.profile.id==next_gate).is_some_and(|g|g.credentials.is_some()&&g.pending.is_none()&&g.page.as_ref().is_some_and(|p|p.more)))),
                )),
                when(group.map(|g|g.page.as_ref().is_some_and(|p|p.records.is_empty())),||text().class("hint").content(i18n::bind("No saved configuration records on this page"))),
                col().class("configuration-record-list").fill(each(state.map(move|s|s.configuration_records.groups.iter().find(|g|g.profile.id==rows_id).and_then(|g|g.page.as_ref()).map(|p|p.records.clone()).unwrap_or_default())).key(|r|r.id.clone()).children(move|item|{
                    let record=item.value;let server=id.clone();let record_id=record.get().id;let open=inspect.clone();
                    row().class("configuration-server-record").fill((
                        col().class("recovery-copy").fill((text().class("recovery-path").content(record.map(|r|r.path.clone().unwrap_or_else(||r.id.clone()))),text().class("recovery-id subtle").selectable(true).content(record.map(|r|r.id.clone())),text().class("recovery-phase subtle").content(i18n::bind(record.map(|r|r.phase_label()))),text().class("subtle").content(record.map(|r|formatting::timestamp(Some(r.modified)))))),
                        text().class("recovery-state subtle").content(i18n::bind(record.map(|r|r.state_label().to_owned()))),
                        btn("Inspect recovery version…",move||open.inspect_configuration_record(server.clone(),record_id.clone())).class("button recovery-inspect").enabled(group.map(|g|g.credentials.is_some()&&g.pending.is_none())),
                    ))
                })),
            ))
        }}),
    ))
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        when(
            state.map(|s| s.dialog == Some(Dialog::ConfigurationRecordConnect)),
            {
                let ui = ui.clone();
                move || {
                    let cancel = ui.clone();
                    let read = ui.clone();
                    modal(ui.clone(),"Connect to recovery records",col().class("form-stack").fill((
                text().class("hint").content(i18n::bind("Choose trusted servers to read their private configuration records. Administrator credentials are used only for on-demand reads and are never saved.")),
                each(state.map(|s|s.config.servers.clone())).key(|p|p.id.clone()).children(move|item|{
                    let profile=item.value.get();let id=profile.id.clone();let selected=id.clone();let visibility=id.clone();let form_id=id.clone();
                    col().class("record-target-picker").fill((
                        btn(format!("{} · {}@{}:{}",profile.name,profile.username,profile.host,profile.port),move||state.revise(|s|{if !s.configuration_records.selected.remove(&id){s.configuration_records.selected.insert(id.clone());}else{s.configuration_records.form.remove(&id);}})).class(state.map(move|s|if s.configuration_records.selected.contains(&selected){"record-target active"}else{"record-target"}.to_owned())).enabled(state.map(move|s|s.config.trusted_fingerprint(&profile.host,profile.port).is_some())),
                        when(state.map(move|s|s.configuration_records.selected.contains(&visibility)),move||{
                            let secret=form_id.clone();let sudo=form_id.clone();let secret_value=form_id.clone();let sudo_value=form_id.clone();
                            col().class("form-stack").fill((
                                text().class("field-label").content(i18n::bind("SSH password or key passphrase")),
                                input().purpose(Purpose::Password).name(i18n::bind("SSH password or key passphrase")).value(state.map(move|s|s.configuration_records.form.get(&secret_value).map(|v|v.0.clone()).unwrap_or_default())).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|s.configuration_records.form.entry(secret.clone()).or_default().0=value);}),
                                text().class("field-label").content(i18n::bind("Administrator password (optional)")),
                                input().purpose(Purpose::Password).name(i18n::bind("Administrator password (optional)")).value(state.map(move|s|s.configuration_records.form.get(&sudo_value).map(|v|v.1.clone()).unwrap_or_default())).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|s.configuration_records.form.entry(sudo.clone()).or_default().1=value);}),
                            ))
                        }),
                    ))
                }),
            )),(btn("Cancel",move||cancel.close_dialog()),btn("Read recovery records",move||read.browse_configuration_records()).class("button primary")))
                }
            },
        ),
        when(
            state.map(|s| s.dialog == Some(Dialog::ConfigurationRecordDetail)),
            move || {
                let close = ui.clone();
                let refresh = ui.clone();
                let restore = ui.clone();
                modal(ui.clone(),"Server recovery version",col().class("form-stack recovery-detail").fill((
                fragment((text().class("section-title").content(state.map(|s|s.configuration_records.viewer.as_ref().map(|v|v.id.clone()).unwrap_or_default())),
                text().class("recovery-detail-server subtle").content(state.map(|s|s.configuration_records.viewer.as_ref().and_then(|v|s.configuration_records.groups.iter().find(|g|g.profile.id==v.server)).map(|g|format!("{} · {}@{}:{} · {}",g.profile.name,g.profile.username,g.profile.host,g.profile.port,g.fingerprint)).unwrap_or_default())),)),
                text().class("recovery-detail-phase").content(i18n::bind(state.map(|s|s.configuration_records.viewer.as_ref().map(|v|if v.pending.is_some(){"Reading recovery version…".to_owned()}else{v.detail.as_ref().map(|d|i18n::translate(&format!("Saved phase: {}",i18n::translate(&d.record.phase_label())))).unwrap_or_default()}).unwrap_or_default()))),
                text().class("recovery-detail-state subtle").content(i18n::bind(state.map(|s|s.configuration_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|d.record.state_label().to_owned()).unwrap_or_default()))),
                text().class("subtle").content(state.map(|s|s.configuration_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|format!("{} · {}\n{} → {}",formatting::timestamp(Some(d.sampled)),d.request.spec.path,d.current_sha256.as_deref().unwrap_or("?"),d.original_sha256)).unwrap_or_default())),
                text().class(state.map(|s|if s.configuration_records.viewer.as_ref().is_none_or(|v|v.error.is_empty()){"hidden"}else{"form-error"}.to_owned())).content(state.map(|s|s.configuration_records.viewer.as_ref().map(|v|v.error.clone()).unwrap_or_default())),
                text().class("recovery-detail-issue hint").content(i18n::bind(state.map(|s|s.configuration_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).and_then(|d|d.issue.clone()).unwrap_or_default()))),
                text().class("recovery-missing hint").content(i18n::bind(state.map(|s|if s.configuration_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).is_some_and(|d|d.current_missing){"Current configuration is missing; restoration still requires a new reviewed preview"}else{""}.to_owned()))),
                text().class("script-code recovery-review").selectable(true).content(state.map(|s|s.configuration_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|d.review.clone()).unwrap_or_default())),
                text().class("script-code recovery-diff").selectable(true).content(state.map(|s|s.configuration_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|d.diff.clone()).unwrap_or_default())),
                text().class("script-code recovery-events").content(state.map(|s|s.configuration_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|d.events.iter().map(|e|format!("{} · {}",formatting::timestamp(Some(e.time)),e.phase)).collect::<Vec<_>>().join("\n")).unwrap_or_default())),
                text().class("script-code recovery-logs").selectable(true).content(state.map(|s|s.configuration_records.viewer.as_ref().and_then(|v|v.detail.as_ref()).map(|d|d.logs.iter().map(|(name,value)|format!("{name}\n{value}")).collect::<Vec<_>>().join("\n")).unwrap_or_default())),
                text().class("hint").content(i18n::bind("Inspecting never runs saved commands. Restore opens a new authenticated task review for the original server, checks the current file again and preserves its current state before changing it.")),
            )),(btn("Close",move||close.close_dialog()),btn("Refresh recovery version",move||{if let Some((server,id))=state.with(|s|s.configuration_records.viewer.as_ref().map(|v|(v.server.clone(),v.id.clone()))){refresh.inspect_configuration_record(server,id);}}).enabled(state.map(|s|s.configuration_records.viewer.as_ref().is_some_and(|v|v.pending.is_none()))),btn("Review restoration…",move||restore.restore_configuration_record()).class("button primary").enabled(state.map(|s|!scripts_ui::busy(s)&&s.configuration_records.viewer.as_ref().is_some_and(|v|v.pending.is_none()&&v.error.is_empty()&&v.detail.as_ref().is_some_and(|d|d.restorable))))))
            },
        ),
    ))
}
