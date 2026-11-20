//! Global storage inspection and an explicit handoff to reviewed server cleanup.
use arcnix_core::record_maintenance::{self as storage, Action as StorageAction, Scope};
use zeroize::Zeroizing;

use crate::*;

#[derive(Clone)]
struct Group {
    profile: ServerProfile,
    fingerprint: String,
    credentials: Option<(Zeroizing<String>, Zeroizing<String>)>,
    inventory: Option<Box<storage::Inventory>>,
    preview: Option<Box<storage::Preview>>,
    selected: HashSet<String>,
    pending: Option<u64>,
    error: String,
}
#[derive(Clone)]
pub(crate) struct StorageState {
    pub visible: bool,
    scope: Scope,
    policy: storage::Policy,
    form_scope: Scope,
    days: String,
    keep: String,
    selected: HashSet<String>,
    form: HashMap<String, (String, String)>,
    groups: Vec<Group>,
    serial: u64,
    error: String,
}
impl Default for StorageState {
    fn default() -> Self {
        let policy = storage::Policy::for_scope(Scope::Task);
        Self {
            visible: false,
            scope: Scope::Task,
            form_scope: Scope::Task,
            days: policy.min_age_days.to_string(),
            keep: policy.keep_latest.to_string(),
            policy,
            selected: HashSet::new(),
            form: HashMap::new(),
            groups: Vec::new(),
            serial: 0,
            error: String::new(),
        }
    }
}
pub(crate) fn dismiss(s: &mut State) {
    if s.dialog == Some(Dialog::RecordStorageConnect) {
        s.record_storage.form.clear();
    }
}
pub(crate) fn forget(s: &mut State) {
    s.record_storage.serial += 1;
    s.record_storage.form.clear();
    for group in &mut s.record_storage.groups {
        group.credentials = None;
        group.pending = None;
        group.preview = None;
        group.selected.clear();
    }
    if s.dialog == Some(Dialog::RecordStorageConnect) {
        s.dialog = None;
    }
}
pub(crate) fn leave(s: &mut State) {
    forget(s);
    s.record_storage.visible = false;
}
fn target(s: &State, id: &str) -> anyhow::Result<batch_worker::Target> {
    let group = s
        .record_storage
        .groups
        .iter()
        .find(|g| g.profile.id == id)
        .ok_or_else(|| anyhow::anyhow!("Reconnect to record storage"))?;
    let profile = s
        .config
        .servers
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| anyhow::anyhow!("Server inventory changed; reconnect to record storage"))?;
    anyhow::ensure!(
        profile.host == group.profile.host
            && profile.port == group.profile.port
            && profile.username == group.profile.username
            && s.config.trusted_fingerprint(&profile.host, profile.port)
                == Some(group.fingerprint.as_str()),
        "Server identity changed; reconnect to record storage"
    );
    let (secret, sudo) = group
        .credentials
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Reconnect to record storage"))?;
    Ok(batch_worker::Target {
        profile: group.profile.clone(),
        fingerprint: group.fingerprint.clone(),
        secret,
        sudo,
    })
}
impl Ui {
    fn review_record_cleanup(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                !scripts_ui::busy(s),
                "Wait for the current operation to finish"
            );
            let mut targets = Vec::new();
            let mut peers = HashSet::new();
            for group in &s.record_storage.groups {
                if group.pending.is_some() || !group.error.is_empty() || group.credentials.is_none()
                {
                    continue;
                }
                let Some(preview) = &group.preview else {
                    continue;
                };
                if preview.eligible_count == 0 {
                    continue;
                }
                let bound = target(s, &group.profile.id)?;
                anyhow::ensure!(
                    peers.insert((bound.profile.host.to_lowercase(), bound.profile.port)),
                    "Choose each server endpoint only once"
                );
                targets.push(arcnix_core::record_cleanup::TargetPlan {
                    id: bound.profile.id.clone(),
                    endpoint: format!(
                        "{}@{}:{}",
                        bound.profile.username, bound.profile.host, bound.profile.port
                    ),
                    fingerprint: bound.fingerprint,
                    plan: arcnix_core::record_cleanup::Plan::new(preview)?,
                });
            }
            arcnix_core::record_cleanup::Job { targets }.template()
        });
        match result {
            Err(error) => self
                .state
                .revise(|s| s.record_storage.error = format!("{error:#}")),
            Ok(template) => {
                let selected = template
                    .cleanup
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
    pub(crate) fn open_record_storage(&self, scope: Scope, profile: Option<String>) {
        self.navigate(navigation::Page::Tasks);
        let connect = self.state.revise(|s| {
            task_records_ui::forget(s);
            cleanup_records_ui::leave(s);
            s.record_storage.visible = true;
            profile.is_some()
                || s.record_storage.groups.is_empty()
                || s.record_storage.scope != scope
        });
        if connect {
            self.connect_record_storage(scope, profile);
        }
    }
    fn connect_record_storage(&self, scope: Scope, profile: Option<String>) {
        self.state.revise(|s| {
            let m = &mut s.record_storage;
            m.form.clear();
            m.form_scope = scope;
            let policy = if scope == m.scope {
                m.policy.clone()
            } else {
                storage::Policy::for_scope(scope)
            };
            m.days = policy.min_age_days.to_string();
            m.keep = policy.keep_latest.to_string();
            m.selected = profile
                .map(|id| std::iter::once(id).collect())
                .unwrap_or_else(|| m.groups.iter().map(|g| g.profile.id.clone()).collect());
        });
        self.open_dialog(Dialog::RecordStorageConnect);
    }
    fn inspect_record_storage(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let policy = storage::Policy {
                min_age_days: s.record_storage.days.parse()?,
                keep_latest: s.record_storage.keep.parse()?,
            };
            let scope = s.record_storage.form_scope;
            policy.validate(scope)?;
            let mut targets = Vec::new();
            let mut peers = HashSet::new();
            for profile in s
                .config
                .servers
                .iter()
                .filter(|p| s.record_storage.selected.contains(&p.id))
            {
                let user = if scope == Scope::Task {
                    profile.username.clone()
                } else {
                    String::new()
                };
                anyhow::ensure!(
                    peers.insert((profile.host.to_lowercase(), profile.port, user)),
                    "Choose each record store only once"
                );
                let fingerprint = s
                    .config
                    .trusted_fingerprint(&profile.host, profile.port)
                    .ok_or_else(|| anyhow::anyhow!("Connect and save the host fingerprint first"))?
                    .to_owned();
                let (secret, sudo) = s
                    .record_storage
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
            Ok((scope, policy, targets))
        });
        match result {
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
            Ok((scope, policy, targets)) => {
                let requests = self.state.revise(|s| {
                    forget(s);
                    let m = &mut s.record_storage;
                    let previous = std::mem::take(&mut m.groups);
                    let same_policy = m.scope == scope && m.policy == policy;
                    m.scope = scope;
                    m.policy = policy.clone();
                    m.error.clear();
                    targets
                        .into_iter()
                        .map(|target| {
                            let prior = previous.iter().find(|g| {
                                same_policy
                                    && g.profile.id == target.profile.id
                                    && g.profile.host == target.profile.host
                                    && g.profile.port == target.profile.port
                                    && g.profile.username == target.profile.username
                                    && g.fingerprint == target.fingerprint
                            });
                            m.serial += 1;
                            let serial = m.serial;
                            m.groups.push(Group {
                                profile: target.profile.clone(),
                                fingerprint: target.fingerprint.clone(),
                                credentials: Some((target.secret.clone(), target.sudo.clone())),
                                inventory: prior.and_then(|g| g.inventory.clone()),
                                preview: None,
                                selected: HashSet::new(),
                                pending: Some(serial),
                                error: String::new(),
                            });
                            (serial, target)
                        })
                        .collect::<Vec<_>>()
                });
                for (serial, target) in requests {
                    batch_worker::record_storage(
                        serial,
                        target,
                        storage::Request {
                            scope,
                            policy: policy.clone(),
                            root: None,
                            action: StorageAction::List(0),
                        },
                        None,
                        self.batch_events.clone(),
                    );
                }
            }
        }
    }
    fn query_record_storage(&self, id: &str, action: StorageAction) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let target = target(s, id)?;
            let group = s
                .record_storage
                .groups
                .iter()
                .find(|g| g.profile.id == id)
                .unwrap();
            anyhow::ensure!(group.pending.is_none(), "Wait for the storage query");
            let inventory = group
                .inventory
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Read record storage before previewing cleanup"))?;
            Ok((
                target,
                inventory.account.clone(),
                storage::Request {
                    scope: s.record_storage.scope,
                    policy: s.record_storage.policy.clone(),
                    root: Some(inventory.root.clone()),
                    action,
                },
            ))
        });
        match result {
            Err(error) => self
                .state
                .revise(|s| s.record_storage.error = format!("{error:#}")),
            Ok((target, account, request)) => {
                let serial = self.state.revise(|s| {
                    s.record_storage.serial += 1;
                    let serial = s.record_storage.serial;
                    let group = s
                        .record_storage
                        .groups
                        .iter_mut()
                        .find(|g| g.profile.id == id)
                        .unwrap();
                    group.pending = Some(serial);
                    group.error.clear();
                    serial
                });
                batch_worker::record_storage(
                    serial,
                    target,
                    request,
                    Some(account),
                    self.batch_events.clone(),
                );
            }
        }
    }
}
pub(crate) fn event(ui: &Ui, event: batch_worker::Event) {
    if let batch_worker::Event::RecordStorage { serial, id, result } = event {
        ui.state.revise(|s| {
            if let Some(group) = s
                .record_storage
                .groups
                .iter_mut()
                .find(|g| g.profile.id == id && g.pending == Some(serial))
            {
                group.pending = None;
                match result {
                    Ok(batch_worker::StorageResponse::Inventory(page)) => {
                        group.preview = None;
                        if group.inventory.as_ref().is_some_and(|old| {
                            old.root_identity != page.root_identity || old.account != page.account
                        }) {
                            group.selected.clear();
                            group.preview = None;
                        }
                        group.inventory = Some(page);
                        group.error.clear();
                    }
                    Ok(batch_worker::StorageResponse::Preview(preview)) => {
                        group.preview = Some(preview);
                        group.error.clear();
                    }
                    Err(error) => group.error = error,
                }
            }
        });
    }
}
pub(crate) fn tab(ui: Ui) -> impl Widget {
    let state = ui.state;
    btn("Record storage", move || {
        let scope = state.with(|s| s.record_storage.scope);
        ui.open_record_storage(scope, None);
    })
    .class(state.map(|s| {
        if s.record_storage.visible {
            "choice active"
        } else {
            "choice"
        }
        .to_owned()
    }))
}
pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let connect = ui.clone();
    let clear = ui.clone();
    let cleanup = ui.clone();
    col().class("record-storage-page").fill((
        row().class("global-toolbar").fill((btn("Connect to record storage…",move||{let scope=state.with(|s|s.record_storage.scope);connect.connect_record_storage(scope,None);}).class("button primary"),btn("Forget storage credentials",move||clear.state.revise(forget)),crate::help_ui::group("Storage help",&["Inspect allocated and logical bytes, then preview exact objects for selected records. This page never deletes files, truncates logs or stops services. Python 3.8+ and root or sudo are required; SFTP is not.","Active, uncertain, referenced and retained records are protected. Configuration references are checked on the server. Allocated bytes are a snapshot, not guaranteed free space; linked and nested entries are excluded when measurement is incomplete.","Credentials remain only in page memory and are cleared when leaving Tasks, changing views or forgetting them. In-flight reads may finish later. Previews are not stored in local task history."]))),
        btn("Review cleanup on servers…",move||cleanup.review_record_cleanup()).class("button storage-cleanup-review").enabled(state.map(|s|!scripts_ui::busy(s)&&s.record_storage.groups.iter().any(|g|g.pending.is_none()&&g.error.is_empty()&&g.credentials.is_some()&&g.preview.as_ref().is_some_and(|p|p.eligible_count>0)))),
        text().class("hint").content(i18n::bind("Cleanup uses only candidates from verified previews. Reauthenticate and review exact files before submitting server jobs. Deletion cannot be undone; interrupted jobs may leave a private quarantine and report.")),
        text().class("section-title").content(i18n::bind(state.map(|s|s.record_storage.scope.label().to_owned()))),
        text().class("hint").content(i18n::bind("Preview record storage and selected files across servers. No server files are changed.")),

        text().class(state.map(|s|if s.record_storage.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(state.map(|s|s.record_storage.error.clone())),
        each(state.map(|s|s.record_storage.groups.clone())).key(|g|g.profile.id.clone()).children({let ui=ui.clone();move|item|{
            let group=item.value;let id=group.get().profile.id;let prev_id=id.clone();let next_id=id.clone();let refresh_id=id.clone();let preview_id=id.clone();let select_id=id.clone();let clear_id=id.clone();let rows_id=id.clone();let previous=ui.clone();let next=ui.clone();let refresh=ui.clone();let preview=ui.clone();let prev_gate=id.clone();let next_gate=id.clone();let refresh_gate=id.clone();let preview_gate=id.clone();
            col().class("storage-server-group").fill((
                text().class("section-title").content(group.map(|g|g.profile.name.clone())),
                text().class("subtle").content(group.map(|g|format!("{}@{}:{} · {}",g.profile.username,g.profile.host,g.profile.port,g.fingerprint))),
                text().class("storage-state subtle").content(i18n::bind(group.map(|g|if g.pending.is_some(){"Inspecting record storage…"}else if g.credentials.is_none(){"Disconnected; reconnect to inspect storage"}else if !g.error.is_empty(){"Storage query failed; previous snapshots are retained"}else{"Storage query completed"}.to_owned()))),
                text().class("storage-snapshot subtle").content(group.map(|g|g.inventory.as_ref().map(|p|format!("{} · {} · UID {}",formatting::timestamp(Some(p.sampled)),p.root,p.account.uid)).unwrap_or_default())),
                text().class("storage-capacity subtle").content(group.map(|g|g.inventory.as_ref().map(|p|i18n::translate(&format!("{} records · {} allocated · {} logical",p.record_count,formatting::bytes(p.allocated_bytes),formatting::bytes(p.logical_bytes)))).unwrap_or_default())),
                text().class("storage-policy subtle").content(group.map(|g|g.inventory.as_ref().map(|p|i18n::translate(&format!("Minimum age: {} days · Keep latest: {}",p.policy.min_age_days,p.policy.keep_latest))).unwrap_or_default())),
                when(group.map(|g|g.inventory.as_ref().is_some_and(|p|!p.capacity_complete)),||text().class("hint").content(i18n::bind("Storage measurement is incomplete; these bytes include only measured record directories and regular files."))),
                text().class("storage-reference-issues hint").content(group.map(|g|g.inventory.as_ref().map(|p|if p.references_complete{String::new()}else{format!("{}\n{}",i18n::translate("Configuration references are incomplete; cleanup candidates remain protected"),p.reference_issues.iter().map(|issue|i18n::translate(issue)).collect::<Vec<_>>().join("\n"))}).unwrap_or_default())),
                text().class(group.map(|g|if g.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(group.map(|g|g.error.clone())),
                row().class("script-actions").fill((
                    btn("Previous records",move||{let offset=state.with(|s|s.record_storage.groups.iter().find(|g|g.profile.id==prev_id).and_then(|g|g.inventory.as_ref()).map(|p|p.offset.saturating_sub(100)).unwrap_or(0));previous.query_record_storage(&prev_id,StorageAction::List(offset));}).enabled(state.map(move|s|s.record_storage.groups.iter().find(|g|g.profile.id==prev_gate).is_some_and(|g|g.pending.is_none()&&g.credentials.is_some()&&g.inventory.as_ref().is_some_and(|p|p.offset>0)))),
                    btn("Refresh storage",move||{let offset=state.with(|s|s.record_storage.groups.iter().find(|g|g.profile.id==refresh_id).and_then(|g|g.inventory.as_ref()).map(|p|p.offset).unwrap_or(0));refresh.query_record_storage(&refresh_id,StorageAction::List(offset));}).enabled(state.map(move|s|s.record_storage.groups.iter().find(|g|g.profile.id==refresh_gate).is_some_and(|g|g.pending.is_none()&&g.credentials.is_some()&&g.inventory.is_some()))),
                    btn("Older records",move||{let offset=state.with(|s|s.record_storage.groups.iter().find(|g|g.profile.id==next_id).and_then(|g|g.inventory.as_ref()).map(|p|p.offset+100).unwrap_or(0));next.query_record_storage(&next_id,StorageAction::List(offset));}).enabled(state.map(move|s|s.record_storage.groups.iter().find(|g|g.profile.id==next_gate).is_some_and(|g|g.pending.is_none()&&g.credentials.is_some()&&g.inventory.as_ref().is_some_and(|p|p.more)))),
                    btn("Select page candidates",move||state.revise(|s|{if let Some(g)=s.record_storage.groups.iter_mut().find(|g|g.profile.id==select_id){let mut selected=g.selected.clone();if let Some(p)=&g.inventory{selected.extend(p.records.iter().filter(|r|r.eligible).map(|r|r.id.clone()));}
                        if selected.len()>100{g.error="Select at most 100 records for one preview".into();}else{g.selected=selected;g.preview=None;}}})).enabled(group.map(|g|g.pending.is_none()&&g.credentials.is_some())),
                    btn("Clear selection",move||state.revise(|s|{if let Some(g)=s.record_storage.groups.iter_mut().find(|g|g.profile.id==clear_id){g.selected.clear();g.preview=None;}})).enabled(group.map(|g|g.pending.is_none()&&!g.selected.is_empty())),
                    btn("Preview selected records",move||{let ids=state.with(|s|s.record_storage.groups.iter().find(|g|g.profile.id==preview_id).map(|g|{let mut ids=g.selected.iter().cloned().collect::<Vec<_>>();ids.sort();ids}).unwrap_or_default());preview.query_record_storage(&preview_id,StorageAction::Preview(ids));}).class("button primary").enabled(state.map(move|s|s.record_storage.groups.iter().find(|g|g.profile.id==preview_gate).is_some_and(|g|g.pending.is_none()&&g.credentials.is_some()&&!g.selected.is_empty()))),
                )),
                col().class("storage-record-list").fill(each(state.map(move|s|s.record_storage.groups.iter().find(|g|g.profile.id==rows_id).and_then(|g|g.inventory.as_ref()).map(|p|p.records.clone()).unwrap_or_default())).key(|r|r.id.clone()).children({let server=id.clone();move|item|{
                    let record=item.value;let selected_id=record.get().id;let toggle_id=selected_id.clone();let server_toggle=server.clone();let selected_server=server.clone();let gate=server.clone();
                    row().class("storage-record").fill((
                        col().class("storage-record-copy").fill((text().class("storage-record-title").content(record.map(|r|r.title.clone())),text().class("storage-record-id subtle").content(record.map(|r|r.id.clone())),text().class("storage-record-protection subtle").content(i18n::bind(record.map(|r|r.label().to_owned()))),text().class("storage-reasons subtle").content(record.map(|r|r.reasons.iter().map(|reason|i18n::translate(reason)).collect::<Vec<_>>().join("\n"))))),
                        text().class("storage-record-bytes subtle").content(record.map(|r|format!("{} / {}",formatting::bytes(r.allocated_bytes),formatting::bytes(r.logical_bytes)))),
                        btn("Select record",move||state.revise(|s|{if let Some(g)=s.record_storage.groups.iter_mut().find(|g|g.profile.id==server_toggle){if g.selected.remove(&toggle_id){g.preview=None;}else if g.selected.len()<100{g.selected.insert(toggle_id.clone());g.preview=None;}else{g.error="Select at most 100 records for one preview".into();}}})).class(state.map(move|s|if s.record_storage.groups.iter().find(|g|g.profile.id==selected_server).is_some_and(|g|g.selected.contains(&selected_id)){"button storage-record-select active"}else{"button storage-record-select"}.to_owned())).enabled(state.map(move|s|s.record_storage.groups.iter().find(|g|g.profile.id==gate).is_some_and(|g|g.pending.is_none()&&g.credentials.is_some()))),
                    ))
                }})),
                when(group.map(|g|g.preview.is_some()),move||{
                    col().class("storage-preview").fill((
                        text().class("section-title").content(i18n::bind("Exact cleanup preview")),
                        text().class("storage-preview-summary subtle").content(group.map(|g|g.preview.as_ref().map(|p|format!("{} · {}",formatting::timestamp(Some(p.sampled)),i18n::translate(&format!("{} selected · {} candidates · {} allocated in candidates",p.selected_count,p.eligible_count,formatting::bytes(p.reclaimable_bytes))))).unwrap_or_default())),
                        text().class("hint").content(i18n::bind("Preview only: no objects have been removed. Protected records stay intact. A later cleanup operation must verify this evidence again before changing files.")),
                        text().class("storage-preview-objects script-code").selectable(true).content(group.map(|g|g.preview.as_ref().map(|p|p.records.iter().map(|r|format!("{} · {}\n{}\n{}",r.id,i18n::translate(r.label()),r.reasons.iter().map(|v|i18n::translate(v)).collect::<Vec<_>>().join("\n"),r.objects.iter().map(|o|format!("{} · {} · {}",o.kind,formatting::bytes(o.allocated_bytes),o.path)).collect::<Vec<_>>().join("\n"))).collect::<Vec<_>>().join("\n\n")).unwrap_or_default())),
                    ))
                }),
            ))
        }}),
    ))
}
pub(crate) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::RecordStorageConnect)),
        move || {
            let cancel = ui.clone();
            let inspect = ui.clone();
            modal(ui.clone(),"Inspect record storage",col().class("form-stack").fill((
            text().class("hint").content(i18n::bind("Choose server accounts independently of the workspace. Storage queries and previews require Python 3.8+ and root or sudo to check private configuration references. They never delete records.")),
            row().class("script-actions").fill(each(vec![Scope::Task,Scope::Configuration]).key(|scope|*scope as u8).children(move|item|{let scope=item.value.get();btn(scope.label(),move||state.revise(|s|{s.record_storage.form_scope=scope;s.record_storage.keep=storage::Policy::for_scope(scope).keep_latest.to_string();})).class(state.map(move|s|if s.record_storage.form_scope==scope{"choice active"}else{"choice"}.to_owned()))})),
            col().class("field").fill((text().class("field-label").content(i18n::bind("Minimum record age (days)")),input().name(i18n::bind("Minimum record age (days)")).value(state.map(|s|s.record_storage.days.clone())).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|s.record_storage.days=value);}))),
            col().class("field").fill((text().class("field-label").content(i18n::bind("Keep latest records (per path for configuration originals)")),input().name(i18n::bind("Keep latest records (per path for configuration originals)")).value(state.map(|s|s.record_storage.keep.clone())).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|s.record_storage.keep=value);}))),
            each(state.map(|s|s.config.servers.clone())).key(|p|p.id.clone()).children(move|item|{
                let profile=item.value.get();let id=profile.id.clone();let chosen=id.clone();let visibility=id.clone();let form_id=id.clone();
                col().class("record-target-picker").fill((
                    btn(format!("{} · {}@{}:{}",profile.name,profile.username,profile.host,profile.port),move||state.revise(|s|{if !s.record_storage.selected.remove(&id){s.record_storage.selected.insert(id.clone());}else{s.record_storage.form.remove(&id);}})).class(state.map(move|s|if s.record_storage.selected.contains(&chosen){"record-target active"}else{"record-target"}.to_owned())).enabled(state.map(move|s|s.config.trusted_fingerprint(&profile.host,profile.port).is_some())),
                    when(state.map(move|s|s.record_storage.selected.contains(&visibility)),move||{
                        let secret=form_id.clone();let sudo=form_id.clone();let secret_value=form_id.clone();let sudo_value=form_id.clone();
                        col().class("form-stack").fill((text().class("field-label").content(i18n::bind("SSH password or key passphrase")),input().purpose(Purpose::Password).name(i18n::bind("SSH password or key passphrase")).value(state.map(move|s|s.record_storage.form.get(&secret_value).map(|v|v.0.clone()).unwrap_or_default())).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|s.record_storage.form.entry(secret.clone()).or_default().0=value);}),text().class("field-label").content(i18n::bind("Administrator password (optional)")),input().purpose(Purpose::Password).name(i18n::bind("Administrator password (optional)")).value(state.map(move|s|s.record_storage.form.get(&sudo_value).map(|v|v.1.clone()).unwrap_or_default())).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|s.record_storage.form.entry(sudo.clone()).or_default().1=value);})))
                    }),
                ))
            }),
        )),(btn("Cancel",move||cancel.close_dialog()),btn("Inspect storage",move||inspect.inspect_record_storage()).class("button primary")))
        },
    )
}
