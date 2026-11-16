use arcnix_core::backups::{Action as BackupAction, Policy, Snapshot};

use super::*;

#[derive(Clone, Default)]
pub(super) struct BackupState {
    drill: bool,
    drill_inputs: Vec<DrillInput>,
    editor: Option<Policy>,
    keep: String,
    database_port: String,
    restore_port: String,
    database_restore: Option<arcnix_core::database_backups::Restore>,
    browse: Option<Policy>,
    server: String,
    cross_server: bool,
    access_server: String,
    access_search: String,
    ssh: String,
    sudo: String,
    snapshots: Vec<Snapshot>,
    pending: Option<u64>,
    loaded: bool,
    serial: u64,
    snapshot: Option<Snapshot>,
    destination: String,
    error: String,
    target: Option<batch_worker::Target>,
    files: Option<arcnix_core::snapshot_files::Page>,
    file_query: Option<arcnix_core::snapshot_files::Query>,
    file_directory: String,
    selected_paths: std::collections::BTreeSet<String>,
    restore_paths: Vec<String>,
}
#[derive(Clone)]
struct DrillInput {
    id: String,
    path: String,
    schema: String,
    table: String,
    minimum: String,
    sha256: String,
}
impl Default for DrillInput {
    fn default() -> Self {
        Self {
            id: new_id(),
            path: String::new(),
            schema: "public".into(),
            table: String::new(),
            minimum: "1".into(),
            sha256: String::new(),
        }
    }
}
impl Ui {
    fn recovery_job(
        &self,
        policy: Policy,
        action: BackupAction,
        snapshot: &Snapshot,
        server: String,
    ) {
        let result = (|| -> anyhow::Result<_> {
            let database = matches!(action, BackupAction::RestoreDatabase { .. });
            let checks = self.state.with(|s| {
                s.backups
                    .drill_inputs
                    .iter()
                    .map(|input| -> anyhow::Result<_> {
                        let minimum = input.minimum.parse::<u64>().map_err(|_| {
                            anyhow::anyhow!("Enter a nonnegative minimum byte or row count")
                        })?;
                        Ok(if database {
                            arcnix_core::recovery_drills::Check::Table {
                                schema: input.schema.clone(),
                                table: input.table.clone(),
                                minimum_rows: minimum,
                            }
                        } else {
                            arcnix_core::recovery_drills::Check::File {
                                path: input.path.clone(),
                                minimum_bytes: minimum,
                                sha256: input.sha256.trim().to_ascii_lowercase(),
                            }
                        })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()
            })?;
            let job = arcnix_core::recovery_drills::Job {
                backup: arcnix_core::backups::Job { policy, action },
                snapshot_paths: snapshot.paths.clone(),
                checks,
            };
            let template = job.template()?;
            if self.state.with(|s| s.backups.cross_server) {
                template.for_cross_server_recovery()
            } else {
                Ok(template)
            }
        })();
        match result {
            Ok(template) => {
                self.choose_script(template);
                self.state.revise(|s| {
                    s.scripts.targets.insert(server);
                });
            }
            Err(error) => self.state.revise(|s| s.dialog_error = error.to_string()),
        }
    }
    fn begin_file_restore(&self, snapshot: Snapshot, drill: bool) {
        self.state.revise(|s| {
            s.backups.snapshot = Some(snapshot);
            s.backups.destination.clear();
            s.backups.restore_paths.clear();
            s.backups.drill = drill;
            s.backups.drill_inputs = if drill {
                vec![DrillInput::default()]
            } else {
                vec![]
            };
        });
        self.open_dialog(Dialog::BackupRestore);
    }
    fn begin_database_restore(&self, snapshot: Snapshot, drill: bool) {
        let result = self.state.revise(|s| -> anyhow::Result<()> {
            let mut target = s
                .backups
                .browse
                .as_ref()
                .and_then(|p| p.database.as_ref())
                .ok_or_else(|| anyhow::anyhow!("Choose a PostgreSQL policy"))?
                .connection
                .clone();
            let archive_path = format!(
                "{}/database.dump",
                snapshot
                    .paths
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("Snapshot has no export path"))?
                    .trim_end_matches('/')
            );
            target.name.clear();
            s.backups.restore_port = target.port.to_string();
            s.backups.database_restore = Some(arcnix_core::database_backups::Restore {
                target,
                archive_path,
            });
            s.backups.snapshot = Some(snapshot);
            s.backups.drill = drill;
            s.backups.drill_inputs = if drill {
                vec![DrillInput::default()]
            } else {
                vec![]
            };
            Ok(())
        });
        match result {
            Ok(()) => self.open_dialog(Dialog::BackupDatabaseRestore),
            Err(error) => self.state.revise(|s| s.dialog_error = error.to_string()),
        }
    }
    fn edit_backup(&self, policy: Option<Policy>) {
        let policy = policy.unwrap_or_default();
        self.state.revise(|s| {
            s.backups.keep = policy.keep_last.to_string();
            s.backups.database_port = policy
                .database
                .as_ref()
                .map(|d| d.connection.port.to_string())
                .unwrap_or_default();
            s.backups.editor = Some(policy);
        });
        self.open_dialog(Dialog::BackupEdit);
    }
    fn save_backup(&self) {
        let result = (|| -> anyhow::Result<_> {
            anyhow::ensure!(self.writable.get(), "Configuration is read-only");
            let (mut config, mut policy, keep) = self.state.with(|s| {
                (
                    s.config.clone(),
                    s.backups.editor.clone(),
                    s.backups.keep.clone(),
                )
            });
            let policy = policy
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("No backup policy selected"))?;
            policy.keep_last = keep
                .parse()
                .map_err(|_| anyhow::anyhow!("Enter a snapshot count between 1 and 3650"))?;
            if let Some(db) = &mut policy.database {
                db.connection.port = self
                    .state
                    .with(|s| s.backups.database_port.parse().unwrap_or(0));
            }
            policy.validate()?;
            anyhow::ensure!(
                !policy.servers.is_empty(),
                "Choose at least one backup server"
            );
            config.backup_policies.retain(|p| p.id != policy.id);
            config.backup_policies.push(policy.clone());
            self.store.save(&config)?;
            Ok(config)
        })();
        match result {
            Ok(config) => {
                self.state.revise(|s| s.config = config);
                self.close_dialog();
            }
            Err(e) => self.state.revise(|s| s.dialog_error = format!("{e:#}")),
        }
    }
    fn delete_backup(&self, id: &str) {
        let mut config = self.state.with(|s| s.config.clone());
        config.backup_policies.retain(|p| p.id != id);
        match if self.writable.get() {
            self.store.save(&config)
        } else {
            Err(anyhow::anyhow!("Configuration is read-only"))
        } {
            Ok(()) => {
                self.state.revise(|s| s.config = config);
                self.close_dialog();
            }
            Err(e) => self.state.revise(|s| s.dialog_error = e.to_string()),
        }
    }
    fn backup_job(&self, policy: Policy, action: BackupAction, targets: Option<Vec<String>>) {
        if self.state.with(scripts_ui::busy) {
            return;
        }
        let default_single = matches!(action, BackupAction::Initialize | BackupAction::Check);
        let sequential = !matches!(action, BackupAction::Backup);
        let cross_server = action.is_restore() && self.state.with(|s| s.backups.cross_server);
        match policy.template(action).and_then(|t| {
            if cross_server {
                t.for_cross_server_recovery()
            } else {
                Ok(t)
            }
        }) {
            Ok(template) => {
                let selected = targets.unwrap_or_else(|| {
                    if default_single {
                        policy.servers.iter().take(1).cloned().collect()
                    } else {
                        policy.servers.clone()
                    }
                });
                self.state.revise(forget);
                self.choose_script(template);
                self.state.revise(|s| {
                    s.scripts.targets = selected.into_iter().collect();
                    if sequential {
                        s.scripts.concurrency = 1;
                    }
                });
            }
            Err(e) => self.state.revise(|s| s.backups.error = e.to_string()),
        }
    }
    fn browse_backup(&self, policy: Policy, cross_server: bool) {
        self.state.revise(|s| {
            forget(s);
            s.backups.server = policy.servers.first().cloned().unwrap_or_default();
            s.backups.cross_server = cross_server;
            s.backups.access_server = if cross_server {
                String::new()
            } else {
                s.backups.server.clone()
            };
            s.backups.access_search.clear();
            s.backups.browse = Some(policy);
            s.backups.snapshots.clear();
            s.backups.loaded = false;
            s.backups.error.clear();
            s.backups.ssh.clear();
            s.backups.sudo.clear();
            s.backups.pending = None;
            s.backups.serial += 1;
        });
        self.open_dialog(Dialog::BackupBrowse);
    }
    fn query_snapshots(&self) {
        if self.state.with(|s| s.backups.pending.is_some()) {
            return;
        }
        let result = self.state.revise(|s| -> anyhow::Result<_> {
            let policy = s
                .backups
                .browse
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Choose a backup policy"))?;
            anyhow::ensure!(
                policy.servers.contains(&s.backups.server),
                "Choose a server in this policy"
            );
            anyhow::ensure!(
                s.backups.cross_server || s.backups.server == s.backups.access_server,
                "Load snapshots again to verify the repository access server"
            );
            let mut profile = s
                .config
                .servers
                .iter()
                .find(|p| p.id == s.backups.access_server)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(if s.backups.cross_server {
                        "Choose a repository access server"
                    } else {
                        "Source server profile is unavailable. Use Recover on another server."
                    })
                })?;
            let fingerprint = s
                .config
                .trusted_fingerprint(&profile.host, profile.port)
                .ok_or_else(|| {
                    anyhow::anyhow!("Connect and save the host fingerprint in Servers first")
                })?
                .to_owned();
            if profile.connection.is_none() {
                profile.connection = Some(s.config.settings.connection);
            }
            anyhow::ensure!(
                [&s.backups.ssh, &s.backups.sudo]
                    .iter()
                    .all(|v| v.len() <= 1024 && !v.contains(['\0', '\r', '\n'])),
                "Invalid credential"
            );
            let target = batch_worker::Target {
                profile,
                fingerprint,
                secret: zeroize::Zeroizing::new(s.backups.ssh.clone()),
                sudo: zeroize::Zeroizing::new(s.backups.sudo.clone()),
            };
            s.backups.target = Some(target.clone());
            s.backups.serial += 1;
            s.backups.pending = Some(s.backups.serial);
            s.backups.error.clear();
            s.dialog_error.clear();
            s.backups.snapshots.clear();
            s.backups.loaded = false;
            s.backups.ssh.clear();
            s.backups.sudo.clear();
            Ok((s.backups.serial, policy, s.backups.server.clone(), target))
        });
        match result {
            Ok((serial, policy, origin, target)) => {
                batch_worker::snapshots(serial, policy, origin, target, self.batch_events.clone())
            }
            Err(e) => self.state.revise(|s| s.dialog_error = e.to_string()),
        }
    }
    fn restore_database_backup(&self) {
        let data = self.state.with(|s| {
            Some((
                s.backups.browse.clone()?,
                s.backups.snapshot.clone()?,
                s.backups.database_restore.clone()?,
                s.backups.server.clone(),
                s.backups.restore_port.clone(),
                s.backups.access_server.clone(),
            ))
        });
        if let Some((policy, snapshot, mut restore, origin, port, server)) = data {
            restore.target.port = port.parse().unwrap_or(0);
            let action = BackupAction::RestoreDatabase {
                snapshot: snapshot.id.clone(),
                origin_server: origin,
                restore,
            };
            if let Err(e) = policy
                .template(action.clone())
                .and_then(|t| t.backup.unwrap().preflight(&server))
            {
                self.state.revise(|s| s.dialog_error = e.to_string());
                return;
            }
            if self.state.with(|s| s.backups.drill) {
                self.recovery_job(policy, action, &snapshot, server);
            } else {
                self.backup_job(policy, action, Some(vec![server]));
            }
        }
    }
    fn restore_backup(&self) {
        let data = self.state.with(|s| {
            Some((
                s.backups.browse.clone()?,
                s.backups.snapshot.clone()?,
                s.backups.server.clone(),
                s.backups.destination.clone(),
                s.backups.access_server.clone(),
            ))
        });
        if let Some((policy, snapshot, origin, destination, server)) = data {
            let action = BackupAction::Restore {
                snapshot: snapshot.id.clone(),
                origin_server: origin,
                destination,
                paths: self.state.with(|s| s.backups.restore_paths.clone()),
            };
            if let Err(e) = policy
                .template(action.clone())
                .and_then(|t| t.backup.unwrap().command(&server))
            {
                self.state.revise(|s| s.dialog_error = e.to_string());
                return;
            }
            if self.state.with(|s| s.backups.drill) {
                self.recovery_job(policy, action, &snapshot, server);
            } else {
                self.backup_job(policy, action, Some(vec![server]));
            }
        }
    }
    fn browse_snapshot_files(&self, snapshot: Snapshot) {
        let result = self.state.revise(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                s.backups.loaded
                    && s.backups.pending.is_none()
                    && s.backups.target.is_some()
                    && s.backups.snapshots.iter().any(|v| v.id == snapshot.id),
                "Load snapshots before browsing files"
            );
            s.backups.snapshot = Some(snapshot.clone());
            s.backups.files = None;
            s.backups.selected_paths.clear();
            s.backups.restore_paths.clear();
            let directory = snapshot
                .paths
                .first()
                .filter(|p| arcnix_core::snapshot_files::validate_path(p, true).is_ok())
                .cloned()
                .unwrap_or_else(|| "/".into());
            Ok(arcnix_core::snapshot_files::Query {
                snapshot: snapshot.id,
                directory,
                offset: 0,
            })
        });
        match result {
            Ok(query) => {
                self.open_dialog(Dialog::BackupFiles);
                self.query_snapshot_files(query);
            }
            Err(e) => self.state.revise(|s| s.dialog_error = e.to_string()),
        }
    }
    fn query_snapshot_files(&self, query: arcnix_core::snapshot_files::Query) {
        if self.state.with(|s| s.backups.pending.is_some()) {
            return;
        }
        let result = self.state.revise(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                s.dialog == Some(Dialog::BackupFiles),
                "Open snapshot files first"
            );
            let policy = s
                .backups
                .browse
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Choose a backup policy"))?;
            let target = s
                .backups
                .target
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Load snapshots again to authenticate"))?;
            let current = s
                .config
                .servers
                .iter()
                .find(|p| p.id == target.profile.id)
                .ok_or_else(|| anyhow::anyhow!("Backup server no longer exists"))?;
            anyhow::ensure!(
                current.host == target.profile.host
                    && s.backups.access_server == target.profile.id
                    && current.port == target.profile.port
                    && current.username == target.profile.username
                    && s.config.trusted_fingerprint(&current.host, current.port)
                        == Some(target.fingerprint.as_str())
                    && s.backups
                        .snapshot
                        .as_ref()
                        .is_some_and(|p| p.id == query.snapshot),
                "Snapshot server identity changed; load snapshots again"
            );
            query.command(&policy, &s.backups.server)?;
            s.backups.serial += 1;
            s.backups.pending = Some(s.backups.serial);
            s.backups.file_directory = query.directory.clone();
            s.backups.file_query = Some(query.clone());
            s.backups.error.clear();
            Ok((
                s.backups.serial,
                policy,
                s.backups.server.clone(),
                target,
                query,
            ))
        });
        match result {
            Ok((serial, policy, origin, target, query)) => batch_worker::snapshot_files(
                serial,
                policy,
                origin,
                target,
                query,
                self.batch_events.clone(),
            ),
            Err(e) => self.state.revise(|s| {
                s.backups.error = e.to_string();
                s.backups.selected_paths.clear();
            }),
        }
    }
}
pub(super) fn record_confirmations(ui: &Ui) {
    if !ui.writable.get() {
        return;
    }
    let (mut config, tasks) = ui
        .state
        .with(|s| (s.config.clone(), s.scripts.tasks.clone()));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut changed = false;
    for task in &tasks {
        for target in &task.targets {
            changed |= arcnix_core::backup_evidence::observe(
                &mut config.backup_evidence,
                task,
                target,
                now,
            );
        }
    }
    if !changed {
        return;
    }
    match ui.store.save(&config) {
        Ok(()) => ui.state.revise(|s| s.config = config),
        Err(e) => ui
            .state
            .revise(|s| s.backups.error = format!("Could not save backup confirmations: {e:#}")),
    }
}
pub(super) fn snapshots_event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Vec<Snapshot>, String>,
) {
    ui.state.revise(|s| {
        if s.backups.pending != Some(serial) {
            return;
        }
        s.backups.pending = None;
        s.backups.loaded = true;
        match result {
            Ok(snapshots) => s.backups.snapshots = snapshots,
            Err(e) => {
                s.backups.error = e;
                s.backups.loaded = false;
                s.backups.target = None;
            }
        }
    });
}
pub(super) fn files_event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<arcnix_core::snapshot_files::Page, String>,
) {
    ui.state.revise(|s| {
        if s.dialog != Some(Dialog::BackupFiles) || s.backups.pending != Some(serial) {
            return;
        }
        s.backups.pending = None;
        match result {
            Ok(page) => {
                s.backups.files = Some(page);
            }
            Err(e) => {
                s.backups.error = e;
                s.backups.selected_paths.clear();
            }
        }
    });
}
#[cfg(debug_assertions)]
pub(super) fn access_retained(s: &State) -> bool {
    !s.backups.ssh.is_empty() || !s.backups.sudo.is_empty() || s.backups.target.is_some()
}
pub(super) fn forget(s: &mut State) {
    s.backups.drill = false;
    s.backups.drill_inputs.clear();
    s.backups.error.clear();
    s.backups.file_directory.clear();
    s.backups.pending = None;
    s.backups.serial += 1;
    s.backups.ssh.clear();
    s.backups.sudo.clear();
    s.backups.target = None;
    s.backups.files = None;
    s.backups.file_query = None;
    s.backups.selected_paths.clear();
    s.backups.restore_paths.clear();
    s.backups.snapshots.clear();
    s.backups.snapshot = None;
    s.backups.loaded = false;
}
pub(super) fn dismiss(s: &mut State) {
    if matches!(
        s.dialog,
        Some(
            Dialog::BackupBrowse
                | Dialog::BackupFiles
                | Dialog::BackupDatabaseRestore
                | Dialog::BackupRestore
                | Dialog::BackupEdit
                | Dialog::BackupDelete(_)
        )
    ) {
        forget(s);
    }
}
pub(super) fn sidebar(_ui: Ui) -> impl Widget {
    col().class("context-sidebar").fill(row().class("context-title help-heading").fill((text().content(i18n::bind("Backups")),crate::help_ui::group("Backups",&["Manage backup policies across servers. Backups, checks and restores run on the servers.","Install restic on each server, prepare a password file with mode 600, then initialize a repository. Keep a separate copy of the repository password.","Snapshot file browsing and selected restores require Python 3.8+ on the server."]))))
}
pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let create = ui.clone();
    let install = ui.clone();
    col().class("backup-page page-scroll").fill((
        row().class("global-toolbar").fill((row().class("global-toolbar").fill((text_action("New backup policy",Icon::Plus).class("button with-icon primary").on_press(move||create.edit_backup(None)),btn("Install restic…",move||{if let Some(t)=arcnix_core::scripts::catalog().into_iter().find(|t|t.snippet.id=="builtin:restic"){install.choose_script(t);}}).enabled(state.map(|s|!scripts_ui::busy(s))))),btn("Database client installers…",{let ui=ui.clone();move||{ui.state.revise(|s|{s.scripts.library=0;s.scripts.search="clients".into();});ui.navigate(navigation::Page::Scripts);}}),btn("Install Python 3…",{let ui=ui.clone();move||{if let Some(t)=arcnix_core::scripts::catalog().into_iter().find(|t|t.snippet.id=="builtin:python3"){ui.choose_script(t);}}}).enabled(state.map(|s|!scripts_ui::busy(s))))),
        text().class(state.map(|s|if s.backups.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(state.map(|s|s.backups.error.clone())),
        text().class("context-hint").content(i18n::bind("This page creates on-demand server jobs. A file backup does not guarantee consistency for a running database.")),
        text().class("context-hint").content(i18n::bind("Local repositories share the server failure risk. Use separate storage for disaster recovery.")),
        when(state.map(|s|s.config.backup_policies.is_empty()),||text().class("context-hint").content(i18n::bind("Create a policy to back up directories and browse recoverable snapshots."))),
        each(state.map(|s|s.config.backup_policies.clone())).key(|p|serde_json::to_string(p).unwrap_or_default()).children({let ui=ui.clone();move|item|{
            let p=item.value.get();let edit=ui.clone();let edit_policy=p.clone();let delete=ui.clone();let delete_id=p.id.clone();let browse=ui.clone();let browse_policy=p.clone();let recover=ui.clone();let recovery_policy=p.clone();
            col().class("script-row backup-policy").fill((
                text().class("section-title").content(p.name.clone()),text().class("subtle").content(format!("{} → {}",p.source_label(),p.repository)),text().class("subtle").content(format!("{} · {}",i18n::translate("Servers"),p.servers.len())),
                row().class("script-actions").fill(each(vec![BackupAction::Backup,BackupAction::Initialize,BackupAction::Check,BackupAction::Retention]).key(|a|a.label().to_owned()).children({let ui=ui.clone();let policy=p.clone();move|item|{let action=item.value.get();let run=ui.clone();let policy=policy.clone();btn(action.label(),move||run.backup_job(policy.clone(),action.clone(),None)).enabled(state.map(|s|!scripts_ui::busy(s)))}})),
        row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Backup confirmation")),crate::help_ui::tip("Backup confirmation","Confirmation dates are successful task start times. Failed or unknown attempts preserve earlier successes."))),
                text().class("backup-confirmations subtle").content(state.map({let policy=p.clone();move|s|confirmation_summary(s,&policy)})),
                row().class("script-actions").fill((btn("Browse snapshots…",move||browse.browse_backup(browse_policy.clone(),false)),btn("Recover on another server…",move||recover.browse_backup(recovery_policy.clone(),true)),btn("Edit",move||edit.edit_backup(Some(edit_policy.clone()))),btn("Delete policy",move||delete.open_dialog(Dialog::BackupDelete(delete_id.clone()))))),
            ))
        }}),
    ))
}
fn policy_field(
    label: &'static str,
    ui: &Ui,
    read: fn(&Policy) -> String,
    write: fn(&mut Policy, String),
) -> impl Widget {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(move |s| s.backups.editor.as_ref().map(read).unwrap_or_default()))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        if let Some(p) = &mut s.backups.editor {
                            write(p, value);
                        }
                    });
                }
            }),
    ))
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        when(state.map(|s| s.dialog == Some(Dialog::BackupEdit)), {
            let ui = ui.clone();
            move || {
                let save = ui.clone();
                let cancel = ui.clone();
                let admin = ui.clone();
                modal(ui.clone(),"Backup policy",col().class("form-stack").fill((
                fragment((policy_field("Policy name",&ui,|p|p.name.clone(),|p,v|p.name=v),policy_field("Repository location",&ui,|p|p.repository.clone(),|p,v|p.repository=v))),
                row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Repository")),crate::help_ui::group("Repository",&["Use an absolute repository path, sftp:user@host:/path, or s3:https://host/bucket/path. Backend credentials must be configured on the server."]))),
                policy_field("Server password file",&ui,|p|p.password_file.clone(),|p,v|p.password_file=v),policy_field("Source directory",&ui,|p|p.source.clone(),|p,v|p.source=v),
                database_editor(ui.clone()),
                when(state.map(|s|s.backups.editor.as_ref().is_some_and(|p|p.database.is_none())),move||fragment((text().class("field-label").content(i18n::bind("Exclude patterns (one per line)")),input().multiline(true).class("backup-excludes").name(i18n::bind("Exclude patterns (one per line)")).value(state.map(|s|s.backups.editor.as_ref().map(|p|p.excludes.join("\n")).unwrap_or_default())).on(value_changed,move|e|{if let Event::ValueChanged{value,..}=e{state.revise(|s|{if let Some(p)=&mut s.backups.editor{p.excludes=value.lines().map(str::trim).filter(|s|!s.is_empty()).map(str::to_owned).collect();}});}})))),
                field("Keep last snapshots per group",&ui,|s|s.backups.keep.clone(),|s,v|s.backups.keep=v),
                btn("Run as administrator",move||admin.state.revise(|s|{if let Some(p)=&mut s.backups.editor{p.administrator = !p.administrator;}})).class(state.map(|s|if s.backups.editor.as_ref().is_some_and(|p|p.administrator){"choice active"}else{"choice"}.to_owned())),
                text().class("field-label").content(i18n::bind("Backup servers")),
                each(state.map(|s|s.config.servers.clone())).key(|p|p.id.clone()).children(move|item|{let p=item.value.get();let id=p.id.clone();let selected=id.clone();btn(p.name,move||state.revise(|s|{if let Some(p)=&mut s.backups.editor{if p.servers.contains(&id){p.servers.retain(|v|v!=&id);}else{p.servers.push(id.clone());}}})).class(state.map(move|s|if s.backups.editor.as_ref().is_some_and(|p|p.servers.contains(&selected)){"target-choice active"}else{"target-choice"}.to_owned()))}),
                text().class("hint").content(i18n::bind("Deleting a policy removes its local settings only. Retention removes snapshots after a dry-run preview; reclaiming repository space is a separate operation.")),
            )),(btn("Cancel",move||cancel.close_dialog()),btn("Save policy",move||save.save_backup()).class("button primary")))
            }
        }),
        when(
            state.map(|s| matches!(s.dialog, Some(Dialog::BackupDelete(_)))),
            {
                let ui = ui.clone();
                move || {
                    let delete = ui.clone();
                    let cancel = ui.clone();
                    modal(ui.clone(),"Delete policy",text().content(i18n::bind("Only the local policy is removed. Server repositories and snapshots are preserved.")),(btn("Cancel",move||cancel.close_dialog()),btn("Delete policy",move||{let id=delete.state.with(|s|if let Some(Dialog::BackupDelete(id))=&s.dialog{Some(id.clone())}else{None});if let Some(id)=id{delete.delete_backup(&id);}}).class("button danger")))
                }
            },
        ),
        when(state.map(|s| s.dialog == Some(Dialog::BackupBrowse)), {
            let ui = ui.clone();
            move || browse_dialog(ui.clone())
        }),
        when(state.map(|s| s.dialog == Some(Dialog::BackupFiles)), {
            let ui = ui.clone();
            move || files_dialog(ui.clone())
        }),
        when(
            state.map(|s| s.dialog == Some(Dialog::BackupDatabaseRestore)),
            {
                let ui = ui.clone();
                move || database_restore_dialog(ui.clone())
            },
        ),
        when(
            state.map(|s| s.dialog == Some(Dialog::BackupRestore)),
            move || {
                let submit = ui.clone();
                let cancel = ui.clone();
                let drill = state.with(|s| s.backups.drill);
                modal(ui.clone(),if drill{"Recovery drill"}else{"Restore snapshot"},col().class("form-stack").fill((
            text().class("script-code").content(state.map(|s|s.backups.snapshot.as_ref().map(|p|format!("{}\n{}\n{}",p.id,p.hostname,p.paths.join("\n"))).unwrap_or_default())),
            text().class("hint").content(i18n::bind("Restore to a new absolute directory outside the source and repository. Existing destinations are refused. Restored data is verified before success is reported.")),
            text().class("hint").content(i18n::bind("Snapshot paths are recreated beneath the destination directory.")),
            text().class("snapshot-restore-selection script-code").content(state.map(|s|if s.backups.restore_paths.is_empty(){i18n::translate("Restore the complete snapshot")}else{format!("{}\n{}",i18n::translate("Selected snapshot paths"),s.backups.restore_paths.join("\n"))})),
            field("New restore directory",&ui,|s|s.backups.destination.clone(),|s,v|s.backups.destination=v),
            drill_fields(ui.clone(),false),
        )),(btn("Cancel",move||cancel.close_dialog()),btn(if drill{"Review recovery drill…"}else{"Review restore…"},move||submit.restore_backup()).class("button primary")))
            },
        ),
    ))
}
fn recovery_access_picker(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("repository-access-picker form-stack").fill((
        text().class("section-title").content(i18n::bind("Repository access server")),
        text().class("hint").content(i18n::bind("The source server is not contacted. Choose a trusted server that can access this repository. Repository and credential-file paths come from the policy and must already be prepared on each access or restore server.")),
        field("Recovery server search",&ui,|s|s.backups.access_search.clone(),|s,v|s.backups.access_search=v),
        text().class("snapshot-access-summary subtle").content(state.map(|s|{
            s.config.servers.iter().find(|p|p.id==s.backups.access_server)
                .map(|p|format!("{} {} · {}@{}:{}",i18n::translate("Repository access server:"),p.name,p.username,p.host,p.port))
                .unwrap_or_else(||i18n::translate("Choose a repository access server"))
        })),
        col().class("recovery-access-choices").fill(each(state.map(|s|{
            let template=arcnix_core::scripts::Template::custom(arcnix_core::operations::Snippet { parameters: vec![],
                id:"repository-access".into(),name:"Repository access".into(),description:String::new(),command:"true".into(),administrator:false,
            });
            arcnix_core::target_selection::Filter {search:s.backups.access_search.clone(),..Default::default()}
                .visible(&s.config,&template,&Default::default()).into_iter().cloned().collect::<Vec<_>>()
        })).key(|p|p.id.clone()).children(move|item|{
            let profile=item.value.get();let id=profile.id.clone();let selected=id.clone();let host=profile.host.clone();let port=profile.port;
            col().class("recovery-access-server").fill((
                btn(profile.name,move||state.revise(|s|{forget(s);s.dialog_error.clear();s.backups.access_server=id.clone();}))
                    .class(state.map(move|s|if s.backups.access_server==selected {"choice snapshot-access-choice active"}else{"choice snapshot-access-choice"}.to_owned()))
                    .enabled(state.map(move|s|s.backups.pending.is_none()&&s.config.trusted_fingerprint(&host,port).is_some())),
                text().class("subtle").content(format!("{}@{}:{}",profile.username,profile.host,profile.port)),
            ))
        })),
        text().class("hint").content(i18n::bind("Untrusted servers are disabled. Save their host fingerprint in Servers before using recovery access. No backup data or repository credentials are copied through the desktop.")),
    ))
}
fn browse_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let query = ui.clone();
    let close = ui.clone();
    modal(
        ui.clone(),
        "Backup snapshots",
        col().class("form-stack").fill((
            text().class("section-title").content(state.map(|s| {
                s.backups
                    .browse
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_default()
            })),
            text()
                .class("field-label")
                .content(i18n::bind("Source server")),
            each(state.map(|s| {
                s.backups
                    .browse
                    .as_ref()
                    .map(|b| {
                        b.servers
                            .iter()
                            .map(|id| {
                                let name = s
                                    .config
                                    .servers
                                    .iter()
                                    .find(|p| &p.id == id)
                                    .map(|p| p.name.clone())
                                    .unwrap_or_else(|| {
                                        format!(
                                            "{} · {id}",
                                            i18n::translate("Server profile unavailable")
                                        )
                                    });
                                (id.clone(), name)
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            }))
            .key(|p| p.0.clone())
            .children(move |item| {
                let (id, name) = item.value.get();
                let selected = id.clone();
                btn(name, move || {
                    state.revise(|s| {
                        forget(s);
                        s.dialog_error.clear();
                        s.backups.server = id.clone();
                        if !s.backups.cross_server {
                            s.backups.access_server = id.clone();
                        }
                    })
                })
                .enabled(state.map(|s| s.backups.pending.is_none()))
                .class(state.map(move |s| {
                    if s.backups.server == selected {
                        "choice snapshot-source-choice active"
                    } else {
                        "choice snapshot-source-choice"
                    }
                    .to_owned()
                }))
            }),
            when(state.map(|s| s.backups.cross_server), {
                let ui = ui.clone();
                move || recovery_access_picker(ui.clone())
            }),
            text()
                .class("field-label")
                .content(i18n::bind("SSH password or key passphrase")),
            input()
                .purpose(Purpose::Password)
                .name(i18n::bind("SSH password or key passphrase"))
                .value(state.map(|s| s.backups.ssh.clone()))
                .on(value_changed, move |e| {
                    if let Event::ValueChanged { value, .. } = e {
                        state.revise(|s| s.backups.ssh = value);
                    }
                }),
            text()
                .class("field-label")
                .content(i18n::bind("Administrator password (optional)")),
            input()
                .purpose(Purpose::Password)
                .name(i18n::bind("Administrator password (optional)"))
                .value(state.map(|s| s.backups.sudo.clone()))
                .on(value_changed, move |e| {
                    if let Event::ValueChanged { value, .. } = e {
                        state.revise(|s| s.backups.sudo = value);
                    }
                }),
            btn("Load snapshots", move || query.query_snapshots())
                .enabled(state.map(|s| s.backups.pending.is_none())),
            text()
                .class(state.map(|s| {
                    if s.backups.error.is_empty() {
                        "hidden"
                    } else {
                        "form-error"
                    }
                    .to_owned()
                }))
                .content(state.map(|s| s.backups.error.clone())),
            each(state.map(|s| s.backups.snapshots.clone()))
                .key(|p| p.id.clone())
                .children({
                    let ui = ui.clone();
                    move |item| {
                        let snapshot = item.value.get();
                        let restore = ui.clone();
                        let restore_snapshot = snapshot.clone();
                        let database_ui = ui.clone();
                        let database_snapshot = snapshot.clone();
                        let files_ui = ui.clone();
                        let files_snapshot = snapshot.clone();
                        let drill_ui = ui.clone();
                        let drill_snapshot = snapshot.clone();
                        let is_postgres = snapshot
                            .tags
                            .iter()
                            .any(|t| t == arcnix_core::database_backups::Engine::PostgreSql.tag());
                        let drill_allowed = ui.state.with(|s| {
                            s.backups.browse.as_ref().is_some_and(|p| {
                                p.database.is_none()
                                    || is_postgres
                                        && p.database.as_ref().is_some_and(|d| {
                                            d.engine
                                                == arcnix_core::database_backups::Engine::PostgreSql
                                        })
                            })
                        });
                        col().class("backup-snapshot").fill((
                            text().content(format!("{} · {}", snapshot.time, snapshot.hostname)),
                            text().class("subtle").content(snapshot.paths.join("\n")),
                            text().class("subtle").content(snapshot.id),
                            btn("Browse files…", move || {
                                files_ui.browse_snapshot_files(files_snapshot.clone())
                            })
                            .enabled(state.map(|s| {
                                s.backups.loaded
                                    && s.backups.pending.is_none()
                                    && s.backups.target.is_some()
                            })),
                            btn("Restore…", move || {
                                restore.begin_file_restore(restore_snapshot.clone(), false)
                            }),
                            when(Vary::Value(is_postgres), move || {
                                let ui = database_ui.clone();
                                let snapshot = database_snapshot.clone();
                                btn("Restore to new PostgreSQL database…", move || {
                                    ui.begin_database_restore(snapshot.clone(), false);
                                })
                            }),
                            when(Vary::Value(drill_allowed), move || {
                                let ui = drill_ui.clone();
                                let snapshot = drill_snapshot.clone();
                                btn("Run recovery drill…", move || {
                                    if is_postgres {
                                        ui.begin_database_restore(snapshot.clone(), true);
                                    } else {
                                        ui.begin_file_restore(snapshot.clone(), true);
                                    }
                                })
                            }),
                        ))
                    }
                }),
        )),
        btn("Close", move || close.close_dialog()),
    )
}

fn files_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let open = ui.clone();
    let up = ui.clone();
    let previous = ui.clone();
    let next = ui.clone();
    let restore = ui.clone();
    let back = ui.clone();
    let close = ui.clone();
    modal(ui.clone(),"Snapshot files",col().class("form-stack snapshot-files-browser").fill((
        text().class("script-code").content(state.map(|s|format!("{} {}\n{} {}\n{}",i18n::translate("Source server:"),s.backups.server,i18n::translate("Repository access server:"),s.backups.target.as_ref().map(|t|format!("{} · {}@{}:{}",t.profile.name,t.profile.username,t.profile.host,t.profile.port)).unwrap_or_default(),s.backups.snapshot.as_ref().map(|p|p.id.clone()).unwrap_or_default()))),
        row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Browse snapshots…")),crate::help_ui::group("Browse snapshots…",&["Browse snapshot metadata on demand. Select regular files or directories across pages; a selected directory includes its contents. Restore recreates snapshot paths in a new destination. Credentials remain in memory until backup browsing closes or access is forgotten."]))),
        field("Snapshot directory",&ui,|s|s.backups.file_directory.clone(),|s,v|s.backups.file_directory=v),
        row().class("script-actions").fill((
            btn("Open snapshot directory",move||{let query=open.state.with(|s|s.backups.snapshot.as_ref().map(|p|arcnix_core::snapshot_files::Query{snapshot:p.id.clone(),directory:s.backups.file_directory.clone(),offset:0}));if let Some(q)=query{open.query_snapshot_files(q);}}).enabled(state.map(|s|s.backups.pending.is_none()&&s.backups.target.is_some())),
            btn("Parent snapshot directory",move||{let query=up.state.with(|s|s.backups.files.as_ref().map(|p|arcnix_core::snapshot_files::Query{snapshot:p.snapshot.id.clone(),directory:p.directory.rsplit_once('/').map(|v|if v.0.is_empty(){"/"}else{v.0}).unwrap_or("/").into(),offset:0}));if let Some(q)=query{up.query_snapshot_files(q);}}).enabled(state.map(|s|s.backups.pending.is_none()&&s.backups.target.is_some()&&s.backups.files.as_ref().is_some_and(|p|p.directory!="/"))),
            btn("Forget backup access",move||state.revise(forget)),
        )),
        text().class("snapshot-files-state subtle").content(state.map(|s|i18n::translate(if s.backups.pending.is_some(){"Reading snapshot files…"}else if !s.backups.error.is_empty(){"Snapshot file query failed"}else if s.backups.files.is_some(){"Snapshot file query completed"}else{"Load snapshots again to authenticate"}))),
        text().class(state.map(|s|if s.backups.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(state.map(|s|s.backups.error.clone())),
        text().class("snapshot-files-summary subtle").content(state.map(|s|s.backups.files.as_ref().map(|p|format!("{} · {} · {}–{} / {}",formatting::timestamp(Some(p.sampled)),p.directory,(p.offset+1).min(p.total),p.offset+p.entries.len() as u32,p.total)).unwrap_or_default())),
        row().class("script-actions").fill((
            btn("Previous files",move||{let query=previous.state.with(|s|s.backups.files.as_ref().map(|p|arcnix_core::snapshot_files::Query{snapshot:p.snapshot.id.clone(),directory:p.directory.clone(),offset:p.offset.saturating_sub(100)}));if let Some(q)=query{previous.query_snapshot_files(q);}}).enabled(state.map(|s|s.backups.pending.is_none()&&s.backups.target.is_some()&&s.backups.files.as_ref().is_some_and(|p|p.offset>0))),
            btn("More files",move||{let query=next.state.with(|s|s.backups.files.as_ref().map(|p|arcnix_core::snapshot_files::Query{snapshot:p.snapshot.id.clone(),directory:p.directory.clone(),offset:p.offset+100}));if let Some(q)=query{next.query_snapshot_files(q);}}).enabled(state.map(|s|s.backups.pending.is_none()&&s.backups.target.is_some()&&s.backups.files.as_ref().is_some_and(|p|p.offset+100<p.total))),
        )),
        col().class("snapshot-file-list").fill(each(state.map(|s|s.backups.files.as_ref().map(|p|p.entries.clone()).unwrap_or_default())).key(|n|n.path.clone()).children({let ui=ui.clone();move|item|{
            let node=item.value.get();let path=node.path.clone();let chosen=path.clone();let toggle=path.clone();let open_path=path.clone();let can_select=node.selectable();let directory=node.kind=="dir";let open_ui=ui.clone();
            col().class("snapshot-file-row").fill((
                text().class("snapshot-file-path").content(path),
                text().class("subtle").content(format!("{} · {} · {}",node.kind,node.size.map(formatting::bytes).unwrap_or_else(||"—".into()),node.modified)),
                row().class("script-actions").fill((
                    btn("Select path",move||state.revise(|s|{
                        if s.backups.selected_paths.contains(&toggle){s.backups.selected_paths.remove(&toggle);}else{
                            let mut paths=s.backups.selected_paths.iter().cloned().collect::<Vec<_>>();paths.push(toggle.clone());
                            match arcnix_core::snapshot_files::validate_selection(&paths){Ok(())=>{s.backups.selected_paths.insert(toggle.clone());},Err(e)=>s.dialog_error=e.to_string()}
                        }
                    })).class(state.map(move|s|if s.backups.selected_paths.contains(&chosen){"choice snapshot-file-select active"}else{"choice snapshot-file-select"}.to_owned())).enabled(state.map(move|s|can_select&&s.backups.pending.is_none()&&s.backups.target.is_some()&&s.backups.error.is_empty())),
                    btn("Open directory",move||{let query=open_ui.state.with(|s|s.backups.snapshot.as_ref().map(|p|arcnix_core::snapshot_files::Query{snapshot:p.id.clone(),directory:open_path.clone(),offset:0}));if let Some(q)=query{open_ui.query_snapshot_files(q);}}).enabled(state.map(move|s|directory&&s.backups.pending.is_none()&&s.backups.target.is_some()&&s.backups.error.is_empty())),
                )),
            ))
        }})),
        text().class("snapshot-selected-paths script-code").content(state.map(|s|format!("{} · {}\n{}",i18n::translate("Selected snapshot paths"),s.backups.selected_paths.len(),s.backups.selected_paths.iter().cloned().collect::<Vec<_>>().join("\n")))),
    )),(
        btn("Back to snapshots",move||{back.state.revise(|s|{s.backups.pending=None;s.backups.serial+=1;s.backups.files=None;s.backups.selected_paths.clear();s.backups.restore_paths.clear();s.backups.error.clear();});back.open_dialog(Dialog::BackupBrowse);}),
        btn("Close",move||close.close_dialog()),
        btn("Restore selected…",move||{restore.state.revise(|s|{s.backups.drill=false;s.backups.drill_inputs.clear();s.backups.restore_paths=s.backups.selected_paths.iter().cloned().collect();s.backups.destination.clear();});restore.open_dialog(Dialog::BackupRestore);}).class("button primary").enabled(state.map(|s|s.backups.pending.is_none()&&s.backups.target.is_some()&&s.backups.error.is_empty()&&!s.backups.selected_paths.is_empty())),
    ))
}
fn connection_field(
    label: &'static str,
    ui: &Ui,
    restoring: bool,
    read: fn(&arcnix_core::database_backups::Connection) -> String,
    write: fn(&mut arcnix_core::database_backups::Connection, String),
) -> impl Widget {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(move |s| {
                if restoring {
                    s.backups
                        .database_restore
                        .as_ref()
                        .map(|r| read(&r.target))
                        .unwrap_or_default()
                } else {
                    s.backups
                        .editor
                        .as_ref()
                        .and_then(|p| p.database.as_ref())
                        .map(|d| read(&d.connection))
                        .unwrap_or_default()
                }
            }))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        if restoring {
                            if let Some(r) = &mut s.backups.database_restore {
                                write(&mut r.target, value);
                            }
                        } else if let Some(d) =
                            s.backups.editor.as_mut().and_then(|p| p.database.as_mut())
                        {
                            write(&mut d.connection, value);
                        }
                    });
                }
            }),
    ))
}
fn database_port(ui: &Ui, restoring: bool) -> impl Widget {
    let state = ui.state;
    col().class("field").fill((
        text()
            .class("field-label")
            .content(i18n::bind("Database port")),
        input()
            .name(i18n::bind("Database port"))
            .value(state.map(move |s| {
                if restoring {
                    s.backups.restore_port.clone()
                } else {
                    s.backups.database_port.clone()
                }
            }))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        if restoring {
                            s.backups.restore_port = value
                        } else {
                            s.backups.database_port = value
                        }
                    });
                }
            }),
    ))
}
fn database_editor(ui: Ui) -> impl Widget {
    use arcnix_core::database_backups::{Connection, Database, Engine};
    let state = ui.state;
    col().class("form-stack").fill((
        text().class("field-label").content(i18n::bind("Backup source type")),
        row().class("script-actions").fill(each(vec![(0,"Directory files"),(1,"PostgreSQL"),(2,"MySQL / MariaDB")]).key(|x|x.0).children(move|item|{
            let (index,label)=item.value.get();btn(label,move||state.revise(|s|{if let Some(p)=&mut s.backups.editor{
                if index==0{p.database=None;}else{let engine=if index==1{Engine::PostgreSql}else{Engine::MySql};if p.database.as_ref().is_none_or(|d|d.engine!=engine){let connection=Connection{port:if index==1{5432}else{3306},..Default::default()};s.backups.database_port=connection.port.to_string();p.database=Some(Database{engine,connection});}p.excludes.clear();}
            }})).class(state.map(move|s|{let selected=s.backups.editor.as_ref().and_then(|p|p.database.as_ref()).map(|d|if d.engine==Engine::PostgreSql{1}else{2}).unwrap_or(0);if index==selected{"choice active"}else{"choice"}.to_owned()}))
        })),
        when(state.map(|s|s.backups.editor.as_ref().is_some_and(|p|p.database.is_some())),move||{
            col().class("form-stack").fill((
                row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Database exports")),crate::help_ui::group("Database exports",&["For database exports, Source directory is an existing private staging directory. Database passwords stay in a server file with mode 400 or 600.","PostgreSQL requires pg_dump, pg_restore, psql and a pgpass file. MySQL/MariaDB requires native clients and a client option file; only InnoDB tables are supported and schema changes must be avoided during export.","MySQL exports are recovered as files for a controlled import. PostgreSQL also supports restoring into a new database.","MySQL requires a direct SELECT grant on the exact database or globally, so table-only visibility cannot silently produce a partial export."]))),
                connection_field("Database host",&ui,false,|c|c.host.clone(),|c,v|c.host=v),database_port(&ui,false),
                connection_field("Database name",&ui,false,|c|c.name.clone(),|c,v|c.name=v),connection_field("Database user",&ui,false,|c|c.username.clone(),|c,v|c.username=v),connection_field("Database auth file",&ui,false,|c|c.auth_file.clone(),|c,v|c.auth_file=v),
            ))
        }),
    ))
}
fn database_restore_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let submit = ui.clone();
    let cancel = ui.clone();
    let drill = state.with(|s| s.backups.drill);
    modal(ui.clone(),if drill{"PostgreSQL recovery drill"}else{"Restore PostgreSQL database"},col().class("form-stack").fill((
        text().class("script-code").content(state.map(|s|s.backups.snapshot.as_ref().map(|p|format!("{}\n{}\n{}",p.id,p.hostname,p.paths.join("\n"))).unwrap_or_default())),
        text().class("hint").content(i18n::bind("Create a new PostgreSQL database with a different name. The restore role needs CREATEDB. Existing databases are refused; owners and grants are not imported. A failed restore leaves the new database for inspection.")),
        connection_field("Database host",&ui,true,|c|c.host.clone(),|c,v|c.host=v),database_port(&ui,true),
        connection_field("New database name",&ui,true,|c|c.name.clone(),|c,v|c.name=v),connection_field("Database user",&ui,true,|c|c.username.clone(),|c,v|c.username=v),connection_field("Database auth file",&ui,true,|c|c.auth_file.clone(),|c,v|c.auth_file=v),
        drill_fields(ui.clone(),true),
    )),(btn("Cancel",move||cancel.close_dialog()),btn(if drill{"Review recovery drill…"}else{"Review database restore…"},move||submit.restore_database_backup()).class("button primary")))
}
fn drill_field(
    ui: &Ui,
    id: String,
    label: &'static str,
    read: fn(&DrillInput) -> String,
    write: fn(&mut DrillInput, String),
) -> impl Widget {
    let state = ui.state;
    let input_id = id.clone();
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(move |s| {
                s.backups
                    .drill_inputs
                    .iter()
                    .find(|v| v.id == id)
                    .map(read)
                    .unwrap_or_default()
            }))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        if let Some(input) =
                            s.backups.drill_inputs.iter_mut().find(|v| v.id == input_id)
                        {
                            write(input, value);
                        }
                    });
                }
            }),
    ))
}
fn drill_fields(ui: Ui, database: bool) -> impl Widget {
    let state = ui.state;
    let add = ui.clone();
    when(state.map(|s| s.backups.drill), move || {
        let ui = ui.clone();
        let add = add.clone();
        col().class("drill-form").fill((
            text().class("section-title").content(i18n::bind("Recovery acceptance checks")),
            text().class("hint").content(i18n::bind(if database {
                "Count selected regular or partitioned tables in the new database using read-only queries. Statement timeout is 30 seconds; row filtering is disallowed. This does not validate all business data, roles or extensions."
            }else{
                "Restore the complete snapshot and check readable regular files by their absolute snapshot paths. Set a minimum byte count and optionally a known SHA-256 digest. Symbolic links are refused; a file checksum is limited to 120 seconds."
            })),
            text().class("hint").content(i18n::bind("Drill data is retained in the new directory or database after success, failure or interruption. Cleanup is a separate manual decision.")),
            text().class("hint").content(i18n::bind("The destination parent or database staging directory must already exist, be writable and use its canonical path.")),
            each(state.map(|s|s.backups.drill_inputs.clone())).key(|v|v.id.clone()).children(move|item|{
                let id=item.value.get().id;let remove=id.clone();let ui=ui.clone();
                col().class("drill-check-editor").fill((
                    when(Vary::Value(!database),{let ui=ui.clone();let id=id.clone();move||col().class("form-stack").fill((
                        drill_field(&ui,id.clone(),"Snapshot file path",|v|v.path.clone(),|v,x|v.path=x),
                        drill_field(&ui,id.clone(),"Minimum bytes",|v|v.minimum.clone(),|v,x|v.minimum=x),
                        drill_field(&ui,id.clone(),"Expected SHA-256 (optional)",|v|v.sha256.clone(),|v,x|v.sha256=x),
                    ))}),
                    when(Vary::Value(database),{let ui=ui.clone();let id=id.clone();move||col().class("form-stack").fill((
                        drill_field(&ui,id.clone(),"Table schema",|v|v.schema.clone(),|v,x|v.schema=x),
                        drill_field(&ui,id.clone(),"Table name",|v|v.table.clone(),|v,x|v.table=x),
                        drill_field(&ui,id.clone(),"Minimum rows",|v|v.minimum.clone(),|v,x|v.minimum=x),
                    ))}),
                    btn("Remove check",move||state.revise(|s|s.backups.drill_inputs.retain(|v|v.id!=remove)))
                        .enabled(state.map(|s|s.backups.drill_inputs.len()>1)),
                ))
            }),
            btn("Add acceptance check",move||add.state.revise(|s|{
                if s.backups.drill_inputs.len()<arcnix_core::recovery_drills::MAX_CHECKS{s.backups.drill_inputs.push(DrillInput::default());}
            })).enabled(state.map(|s|s.backups.drill_inputs.len()<arcnix_core::recovery_drills::MAX_CHECKS)),
        ))
    })
}
fn confirmation_summary(s: &State, policy: &Policy) -> String {
    use arcnix_core::backup_evidence::Kind;
    let mut kinds = vec![Kind::Backup, Kind::IntegrityCheck, Kind::FileRestore];
    if policy.database.is_none()
        || policy
            .database
            .as_ref()
            .is_some_and(|d| d.engine == arcnix_core::database_backups::Engine::PostgreSql)
    {
        kinds.push(Kind::RecoveryDrill);
    }
    if policy
        .database
        .as_ref()
        .is_some_and(|d| d.engine == arcnix_core::database_backups::Engine::PostgreSql)
    {
        kinds.push(Kind::DatabaseRestore);
    }
    let revision = arcnix_core::backup_evidence::TaskInfo::policy_revision(policy);
    let mut lines = Vec::new();
    for server in &policy.servers {
        let name = s
            .config
            .servers
            .iter()
            .find(|p| &p.id == server)
            .map(|p| p.name.as_str())
            .unwrap_or(server);
        for kind in &kinds {
            let record = s
                .config
                .backup_evidence
                .iter()
                .find(|e| e.policy == policy.id && &e.server == server && e.kind == *kind);
            let success = record
                .and_then(|r| r.last_success.as_ref())
                .map(|r| formatting::timestamp(Some(r.started)))
                .unwrap_or_else(|| i18n::translate("No saved successful confirmation"));
            let older = record
                .and_then(|r| r.last_success.as_ref())
                .map(|r| {
                    if r.revision != revision {
                        format!(" · {}", i18n::translate("Earlier policy settings"))
                    } else if s
                        .config
                        .servers
                        .iter()
                        .find(|p| &p.id == server)
                        .is_none_or(|p| {
                            r.endpoint != format!("{}@{}:{}", p.username, p.host, p.port)
                                || s.config.trusted_fingerprint(&p.host, p.port)
                                    != Some(r.fingerprint.as_str())
                        })
                    {
                        format!(" · {}", i18n::translate("Earlier server identity"))
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();
            let artifact = record
                .and_then(|r| r.last_success.as_ref())
                .map(|r| {
                    let destination = r
                        .destination
                        .as_ref()
                        .map(|v| format!(" · {v}"))
                        .or_else(|| {
                            r.snapshot
                                .as_ref()
                                .map(|v| format!(" · {}", v.chars().take(12).collect::<String>()))
                        })
                        .unwrap_or_default();
                    if *kind == Kind::RecoveryDrill {
                        format!(
                            "{destination} · {} {} · {}",
                            i18n::translate("Acceptance checks:"),
                            r.drill_checks.unwrap_or(0),
                            r.drill_revision.as_deref().unwrap_or("")
                        )
                    } else if *kind == Kind::FileRestore {
                        format!(
                            "{destination} · {}",
                            if r.paths.is_empty() {
                                i18n::translate("Complete snapshot")
                            } else {
                                format!("{}: {}", i18n::translate("Selected paths"), r.paths.len())
                            }
                        )
                    } else {
                        destination
                    }
                })
                .unwrap_or_default();
            let attempt = record
                .map(|r| format!(" · {}", i18n::translate(r.latest.status.label())))
                .unwrap_or_default();
            lines.push(format!(
                "{name} · {}: {success}{attempt}{older}{artifact}",
                i18n::translate(kind.label())
            ));
        }
    }
    lines.join("\n")
}
