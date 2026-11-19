#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app_icon;
mod backups_ui;
mod batch_worker;
mod browser;
#[cfg(windows)]
mod browser_cookies;
mod cleanup_records_ui;
mod client_logs;
mod clipboard;
mod compose_ui;
mod configuration_backup_ui;
mod deleted_files_ui;
mod desktop_customization_ui;
mod desktop_shortcuts_ui;
mod diagnostics_ui;
mod disks_ui;
mod docker_ui;
mod explorer_search;
mod explorer_transfer;
mod explorer_ui;
mod file_browser;
mod file_columns;
mod file_context;
mod file_picker;
mod file_properties;
mod formatting;
mod help_ui;
mod i18n;
mod icons;
mod installation_ui;
mod monitor_ui;
mod navigation;
mod network_ui;
mod observability;
mod operations;
mod operations_worker;
mod preferences_ui;
mod processes_ui;
mod record_maintenance_ui;
mod recovery_ui;
mod remote_editor;
mod resource_ports_ui;
mod resource_trends;
mod script_exchange_ui;
mod script_parameters_ui;
mod scripts_ui;
mod server_apps_ui;
mod server_apps_worker;
mod server_desktop;
mod server_labels_ui;
mod servers;
mod settings;
mod software;
mod software_install_ui;
mod software_store_ui;
mod splitter;
mod task_records_ui;
mod task_refresh_ui;
mod task_target_picker;
mod tcp_listeners_ui;
mod terminal;
mod terminal_menu;
mod terminal_worker;
#[cfg(test)]
mod terminal_worker_tests;
mod transfer_details;
mod transfer_progress;
mod transfer_ui;
mod transfer_worker;
#[cfg(test)]
mod transfer_worker_tests;
mod update_install;
mod updates_ui;
#[cfg(windows)]
mod web_browser;
#[cfg(windows)]
mod web_browser_native;
mod worker;
#[cfg(test)]
mod worker_conflict_tests;
mod workspace;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use arcnix_core::store::ConfigStore;
use arcnix_core::*;
use browser::{Sort, visible};
use glory::prelude::*;
use glory::ui::{IntoChildren, fragment};
use glory::wry::shell::{Config, Shell, WindowApp};
use icons::{Icon, close_action, icon_action, text_action};
use worker::{Action, Request, Update};

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileAction {
    CreateFile,
    CreateFolder,
    Rename,
}

#[derive(Clone, PartialEq, Eq)]
enum Dialog {
    SoftwareInstall,
    ConfigureNetwork,
    #[cfg(windows)]
    BrowserForwardingRepair,
    StartupService,
    ExplorerColor {
        path: String,
        directory: bool,
    },
    DockerAction,
    DockerConfigure,
    TerminalRename {
        id: u64,
    },
    TransferDefaults,
    TransferProgress(TransferDirection),
    RemoteEditor,
    Server,
    Group {
        id: Option<String>,
    },
    Import,
    Connect,
    File {
        remote: bool,
        action: FileAction,
    },
    FileProperties,
    FileColumns {
        remote: bool,
    },
    DeleteFiles {
        remote: bool,
        targets: Vec<arcnix_core::deletion::Target>,
    },
    DeleteServer(String),
    DissolveGroup(String),
    DeleteGroupAndServers(String),
    SnippetEdit,
    ScriptRun,
    ScriptImport,
    ScriptExport,
    TaskRecordConnect,
    TaskRecordLog,
    TaskStop,
    DiagnosticExport,
    DiagnosticLogs,
    DeletedFiles,
    TcpListeners,
    TerminatePortProcess,
    ConfigurationBackup,
    ConfigurationRestore,
    RecordStorageConnect,
    CleanupReportConnect,
    ExportDownload,
    BackupEdit,
    BackupDelete(String),
    BackupBrowse,
    BackupFiles,
    BackupRestore,
    BackupDatabaseRestore,
    ScriptRefresh(String, String),
    ScriptBatchRefresh(String),
    SnippetDelete(String),
    TransferConflict,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum WorkspacePage {
    #[default]
    Desktop,
    Files,
    Software,
    Resources,
    Terminal,
    Explorer,
    Docker,
    Editor,
    Browser,
    Settings,
}

#[derive(Clone)]
struct State {
    config: AppConfig,
    selected: Option<String>,
    group: Option<String>,
    collapsed_groups: HashSet<String>,
    search: String,
    editor: ServerProfile,
    server_editor_original: Option<ServerProfile>,
    server_tags: String,
    server_editor_tab: u8,
    dialog: Option<Dialog>,
    dialog_error: String,
    host_menu: Option<String>,
    group_menu: Option<String>,
    file_menu: Option<bool>,
    file_context: Option<file_context::Target>,
    file_pending: Option<u64>,
    deletion: Option<file_context::Deletion>,
    unassigned_columns: FileColumnSettings,
    properties: Option<file_properties::Editor>,
    remote_editor: Option<remote_editor::Editor>,
    desktop_editor: remote_editor::Tabs,
    updates: updates_ui::UpdateState,
    import_pending: Option<u64>,
    activity_tab: u8,
    activity_open: bool,
    local_filter_open: bool,
    remote_filter_open: bool,
    notice: String,
    group_name: String,
    port: String,
    secret: String,
    /// The current attempt started without the credential dialog; a failure
    /// reopens it so the user can still enter a passphrase.
    connect_prompt_skipped: bool,
    connected: bool,
    sftp_available: bool,
    connecting: bool,
    keepalive_at: std::time::Instant,
    keepalive_pending: bool,
    fingerprint: Option<String>,
    sync_scope: Option<String>,
    epoch: u64,
    request_serial: u64,
    local_request: u64,
    remote_request: u64,
    local_filter: String,
    remote_filter: String,
    local_sort: Sort,
    remote_sort: Sort,
    local_desc: bool,
    remote_desc: bool,
    queue_paused: bool,
    queue_filter: u8,
    import_path: String,
    local_path: String,
    remote_path: String,
    local_ready: bool,
    remote_ready: bool,
    local: Vec<FileEntry>,
    remote: Vec<FileEntry>,
    local_selection: HashSet<String>,
    remote_selection: HashSet<String>,
    file_browser: file_browser::BrowserState,
    terminal: terminal::TerminalState,
    name: String,
    jobs: Vec<TransferJob>,
    cancels: HashMap<String, Arc<AtomicBool>>,
    transfer_pauses: HashMap<String, Arc<AtomicBool>>,
    explorer_uploads: explorer_transfer::Uploads,
    inflight_transfers: HashSet<String>,
    logs: String,
    workspace_page: WorkspacePage,
    workbench: server_desktop::Workbench,
    #[cfg(windows)]
    web_browser: web_browser::Browser,
    server_apps: server_apps_ui::DesktopState,
    docker: docker_ui::DockerState,
    disks: disks_ui::DiskState,
    compose: compose_ui::ComposeState,
    install: software_install_ui::InstallState,
    navigation: navigation::Page,
    settings_section: navigation::SettingsSection,
    unread_notifications: usize,
    software: software::SoftwareState,
    operations: operations::OperationsState,
    scripts: scripts_ui::ScriptsState,
    task_records: task_records_ui::RecordState,
    deleted_files: deleted_files_ui::DeletedFilesState,
    tcp_listeners: tcp_listeners_ui::TcpListenersState,
    record_storage: record_maintenance_ui::StorageState,
    cleanup_reports: cleanup_records_ui::ReportsState,
    backups: backups_ui::BackupState,
    diagnostics: diagnostics_ui::DiagnosticsState,
    metrics: observability::MetricState,
    ports: resource_ports_ui::PortState,
    system_logs: observability::LogsState,
    system_task_running: bool,
    settings: settings::SettingsState,
    system_dark: bool,
    file_split: f64,
    transfer_conflict: Option<transfer_ui::PendingConflict>,
    transfer_conflicts: std::collections::VecDeque<transfer_ui::PendingConflict>,
    transfer_details: HashMap<String, transfer_details::Details>,
    batch_ids: HashMap<String, String>,
    queue_rules: HashMap<(String, TransferDirection), arcnix_core::transfer::ConflictAction>,
    session_rules: HashMap<TransferDirection, arcnix_core::transfer::ConflictAction>,
    transfer_now: std::time::Instant,
    transfer_meters: HashMap<String, transfer_progress::Meter>,
    transfer_summaries: HashMap<String, arcnix_core::transfer::TransferSummary>,
}
#[derive(Clone)]
struct Ui {
    wallpapers: Arc<desktop_customization_ui::WallpaperAssets>,
    update_controller: Arc<updates_ui::Controller>,
    batch_controller: Rc<std::cell::RefCell<Option<batch_worker::Controller>>>,
    batch_events: async_channel::Sender<batch_worker::Event>,
    logger: Rc<client_logs::Logger>,
    state: Cage<State>,
    tx: Sender<Request>,
    store: Rc<ConfigStore>,
    writable: Cage<bool>,
    scope: Scope,
    shell_focus: Cage<Option<glory::render::NodeId>>,
}
impl Ui {
    fn apply_conflict_rule(
        &self,
        job_id: &str,
        direction: TransferDirection,
        decision: worker::ConflictDecision,
    ) {
        use arcnix_core::transfer::ConflictAction;
        if decision.action == ConflictAction::Cancel {
            self.cancel_all();
            return;
        }
        self.state.revise(|s| match decision.scope {
            worker::ConflictScope::ThisFile => {}
            worker::ConflictScope::ThisQueue => {
                if let Some(batch) = s.batch_ids.get(job_id) {
                    s.queue_rules
                        .insert((batch.clone(), direction), decision.action);
                }
            }
            worker::ConflictScope::ThisSession | worker::ConflictScope::FutureConnections => {
                s.session_rules.insert(direction, decision.action);
            }
        });
        if decision.scope == worker::ConflictScope::FutureConnections {
            let action = match decision.action {
                ConflictAction::Overwrite => ExistingFileAction::Overwrite,
                _ => ExistingFileAction::Skip,
            };
            let result = self.state.revise(|s| {
                s.config.set_transfer_rule(
                    s.selected.as_deref().unwrap_or(""),
                    direction,
                    Some(action),
                )
            });
            if let Err(error) = result {
                self.error(error.to_string());
            } else {
                self.save();
            }
        }
    }
    fn reset_conflict_rules(&self, server_id: &str) {
        self.state.revise(|s| {
            s.config
                .transfer_rules
                .retain(|rule| rule.server_id != server_id);
            if s.selected.as_deref() == Some(server_id) {
                s.session_rules.clear();
                s.queue_rules.clear();
            }
            s.host_menu = None;
        });
        self.save();
        self.log("该服务器的文件冲突处理已恢复为全局默认设置");
    }
    fn select_server(&self, id: String) {
        if self
            .state
            .with(|s| s.system_task_running && s.selected.as_ref() != Some(&id))
        {
            return;
        }
        if self.state.with(|s| s.selected.as_ref() != Some(&id)) {
            self.disconnect();
            self.state.revise(|s| {
                s.file_split = s
                    .config
                    .servers
                    .iter()
                    .find(|p| p.id == id)
                    .and_then(|p| p.file_pane_ratio)
                    .unwrap_or(500) as f64
                    / 1000.0;
                s.selected = Some(id);
                s.workspace_page = WorkspacePage::Desktop;
                s.workbench = Default::default();
                s.server_apps = Default::default();
                s.docker = Default::default();
                s.disks = Default::default();
                s.compose = Default::default();
                s.install = Default::default();
                if let Some(profile) = s
                    .config
                    .servers
                    .iter()
                    .find(|p| Some(&p.id) == s.selected.as_ref())
                {
                    let location = if profile.locations.remember_paths {
                        profile.locations.last_local.as_ref()
                    } else {
                        None
                    }
                    .or(profile.locations.local_directory.as_ref())
                    .or(s.config.local_directory.as_ref());
                    if let Some(path) = location {
                        s.local_path = local_display(path);
                    }
                }
            });
            self.refresh(false);
        }
        self.state.revise(|s| {
            navigation::change_page(s, navigation::Page::Servers, "select-server");
            s.host_menu = None;
            s.group_menu = None;
        });
        self.observability_enter();
        self.terminal_enter();
    }
    fn save_file_split(&self, ratio: f64) {
        let ratio = if ratio.is_finite() {
            ratio.clamp(0.2, 0.8)
        } else {
            0.5
        };
        let saved = self.state.revise(|s| {
            s.file_split = ratio;
            s.selected.clone().is_some_and(|id| {
                s.config
                    .set_file_pane_ratio(&id, (ratio * 1000.0).round() as u16)
                    .is_ok()
            })
        });
        if saved {
            self.save();
        }
    }
    fn file_task_blocks_dialog(&self) -> bool {
        let busy = self.state.with(file_task_busy);
        if busy {
            self.error("文件任务仍在处理当前请求，请稍后重试。");
        }
        busy
    }
    fn open_dialog(&self, dialog: Dialog) {
        if self.file_task_blocks_dialog() {
            return;
        }
        self.state.revise(|s| show_dialog(s, dialog));
    }
    fn close_dialog(&self) {
        let terminal_rename = self
            .state
            .with(|s| matches!(s.dialog, Some(Dialog::TerminalRename { .. })));
        if remote_editor::dismiss(self) {
            return;
        }
        if file_context::dismiss_deletion(self) {
            return;
        }
        if file_properties::dismiss(self) {
            return;
        }
        if self
            .state
            .with(|s| s.dialog == Some(Dialog::TransferConflict))
            && let Some(pending) = self.state.revise(|s| s.transfer_conflict.take())
        {
            self.cancel_all();
            let _ = pending.reply.send(worker::ConflictDecision {
                action: arcnix_core::transfer::ConflictAction::Cancel,
                scope: worker::ConflictScope::ThisFile,
            });
        }
        self.state.revise(|s| {
            if s.file_pending.is_some() || s.import_pending.is_some() {
                return;
            }
            backups_ui::dismiss(s);
            resource_ports_ui::dismiss(s);
            #[cfg(windows)]
            if s.dialog == Some(Dialog::BrowserForwardingRepair) {
                web_browser::cancel_forwarding_review(s);
            }
            if matches!(s.dialog, Some(Dialog::ScriptImport | Dialog::ScriptExport)) {
                script_exchange_ui::forget(s);
            }
            cleanup_records_ui::dismiss(s);
            record_maintenance_ui::dismiss(s);
            if s.dialog == Some(Dialog::Connect) {
                s.secret.clear();
            }
            if matches!(s.dialog, Some(Dialog::ScriptRefresh(..))) {
                s.scripts.refresh_secret.clear();
            }
            task_refresh_ui::dismiss(s);
            task_records_ui::dismiss(s);
            diagnostics_ui::dismiss(s);
            if matches!(
                s.dialog,
                Some(Dialog::ConfigurationBackup | Dialog::ConfigurationRestore)
            ) {
                configuration_backup_ui::forget(s);
            }
            if s.dialog == Some(Dialog::ScriptRun) {
                s.scripts.credentials.clear();
                s.scripts.values.clear();
                s.scripts.preset_selected.clear();
                s.scripts.preset_name.clear();
            }
            if s.dialog == Some(Dialog::TaskRecordConnect) {
                s.task_records.secrets.clear();
            }
            if s.dialog == Some(Dialog::SnippetEdit) {
                s.operations.editor = None;
                s.scripts.parameter_editor = Default::default();
            }

            if s.dialog == Some(Dialog::FileProperties) {
                s.properties = None;
            }
            s.dialog = None;
            s.dialog_error.clear();
        });
        if self.state.with(|s| s.dialog.is_none())
            && let Some(id) = self.shell_focus.get()
        {
            let _ = self.scope.action(glory::render::Action::Focus(id));
        }
        if self
            .state
            .with(|s| s.dialog.is_none() && s.transfer_conflict.is_some())
        {
            self.open_dialog(Dialog::TransferConflict);
        }
        if terminal_rename
            && self.state.with(|s| {
                s.dialog.is_none()
                    && navigation::workspace_visible(s)
                    && s.workspace_page == WorkspacePage::Terminal
            })
        {
            let id = self.state.with(|s| s.terminal.id);
            self.terminal_ui_command(id, "focus");
        }
    }
    fn error(&self, message: impl AsRef<str>) {
        self.state
            .revise(|s| s.notice = message.as_ref().to_string());
        self.log(message);
    }
    fn log(&self, message: impl AsRef<str>) {
        let mut message = i18n::translate(message.as_ref());
        let settings = self.state.with(|s| {
            if !s.secret.is_empty() {
                message = message.replace(s.secret.as_str(), "[redacted]");
            }
            s.config.settings.logging.clone()
        });
        if settings.timestamps {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            message = format!("[{}] {message}", formatting::timestamp(Some(now)));
        }
        self.logger.record(message.clone(), &settings);
        self.state.revise(|s| {
            if !s.logs.is_empty() && !s.logs.ends_with('\n') {
                s.logs.push('\n');
            }
            if s.navigation != navigation::Page::Notifications {
                s.unread_notifications = s.unread_notifications.saturating_add(1);
            }
            s.logs.push_str(&message);
            s.logs.push('\n');
            if s.logs.len() > 60000 {
                let boundary = s
                    .logs
                    .char_indices()
                    .map(|(i, _)| i)
                    .find(|i| *i >= 20000)
                    .unwrap_or(0);
                s.logs.drain(..boundary);
            }
        });
    }
    fn send(&self, action: Action) -> u64 {
        let (epoch, serial) = self.state.revise(|s| {
            s.request_serial += 1;
            (s.epoch, s.request_serial)
        });
        if self
            .tx
            .send(Request {
                epoch,
                serial,
                action,
            })
            .is_err()
        {
            self.log("后台任务已停止");
        }
        serial
    }
    fn retry(&self, id: Option<String>) {
        if !self.state.with(|s| s.connected) {
            self.log("请先连接原服务器再重试传输");
            return;
        }
        self.state.revise(|s| {
            let active = s.cancels.keys().cloned().collect();
            let previous: Vec<_> = s.jobs.iter().map(|job| job.id.clone()).collect();
            browser::retry(&mut s.jobs, &active, s.selected.as_deref(), id.as_deref());
            let changes: Vec<_> = previous
                .into_iter()
                .zip(s.jobs.iter().map(|job| job.id.clone()))
                .filter(|(old, new)| old != new)
                .collect();
            for (old, new) in changes {
                transfer_details::reset(s, &old, &new);
                if let Some(batch) = s.batch_ids.remove(&old) {
                    s.batch_ids.insert(new.clone(), batch);
                }
                s.explorer_uploads.retry(&old, new);
            }
            s.transfer_meters
                .retain(|id, _| s.jobs.iter().any(|j| &j.id == id));
        });
        self.schedule();
    }
    fn save(&self) {
        if !self.writable.get() {
            self.log("配置读取失败，已禁止覆盖。请修复配置文件后重新启动。");
            return;
        }
        let config = self.state.with(|s| s.config.clone());
        if let Err(e) = self.store.save(&config) {
            self.log(format!("保存失败: {e:#}"));
        }
    }
    /// Connects to the selected server, asking for a password or passphrase
    /// only when one may be needed. Keys that are known to be unencrypted
    /// skip the dialog; unreadable or unrecognised keys still show it.
    fn request_connect(&self) {
        if self.state.with(connect_prompt_needed) {
            self.open_dialog(Dialog::Connect);
            return;
        }
        if self.file_task_blocks_dialog() {
            return;
        }
        self.state.revise(|s| {
            // An unencrypted key authenticates with exactly an empty secret.
            s.secret.clear();
            dismiss_menus(s);
        });
        self.connect(false);
    }
    /// `prompted` records whether the user went through the credential
    /// dialog; a failed attempt that skipped it reopens the dialog.
    fn connect(&self, prompted: bool) {
        if self.state.with(|s| s.system_task_running) {
            self.error("系统软件任务正在执行，请等待完成后切换服务器");
            return;
        }
        let profile = self.state.with(|s| {
            s.config
                .servers
                .iter()
                .find(|p| Some(&p.id) == s.selected.as_ref())
                .cloned()
        });
        if let Some(mut profile) = profile {
            if profile.connection.is_none() {
                profile.connection = Some(self.state.with(|s| s.config.settings.connection));
            }
            self.cancel_all();
            #[cfg(windows)]
            self.close_web_browser();
            self.state.revise(|s| {
                s.epoch += 1;
                s.cancels.clear();
                s.connected = false;
                s.connecting = true;
                s.connect_prompt_skipped = !prompted;
                s.sync_scope = None;
                s.terminal = Default::default();
                s.fingerprint = None;
                s.remote_ready = false;
                s.remote.clear();
                s.remote_selection.clear();
                s.file_browser.reset_remote_history();
                s.remote_path = if profile.locations.remember_paths {
                    profile.locations.last_remote.clone()
                } else {
                    None
                }
                .unwrap_or_else(|| profile.initial_remote_path.clone());
                s.software = Default::default();
                s.server_apps = Default::default();
                s.docker = Default::default();
                s.disks = Default::default();
                s.compose = Default::default();
                s.install = Default::default();
                s.operations = Default::default();
                s.metrics = Default::default();
                resource_ports_ui::reset(s);
                s.system_logs = Default::default();
            });
            let known = self.state.with(|s| {
                s.config
                    .trusted_fingerprint(&profile.host, profile.port)
                    .map(str::to_owned)
            });
            self.send(Action::Probe(profile.into(), known));
        } else {
            self.log("请先新建并选择服务器");
        }
    }
    fn disconnect(&self) {
        remote_editor::cancel(self);
        file_context::cancel_deletion(self);
        file_properties::cancel(self);
        if self.state.with(|s| s.system_task_running) {
            self.error("系统软件任务正在执行，请等待完成后断开连接");
            return;
        }
        self.cancel_all();
        #[cfg(windows)]
        self.close_web_browser();
        if self
            .state
            .with(|s| s.dialog == Some(Dialog::TransferConflict))
        {
            self.close_dialog();
        }
        self.state.revise(|s| {
            s.epoch += 1;
            s.terminal = Default::default();
            s.cancels.clear();
            s.connected = false;
            s.sftp_available = false;
            s.sync_scope = None;
            s.connecting = false;
            s.fingerprint = None;
            s.secret.clear();
            s.remote_ready = false;
            s.remote.clear();
            s.remote_selection.clear();
            s.remote_request = 0;
            s.file_browser.reset_remote_history();
            s.software = Default::default();
            s.server_apps = Default::default();
            s.docker = Default::default();
            s.disks = Default::default();
            s.compose = Default::default();
            s.install = Default::default();
            remote_editor::disconnect(s);
            s.operations = Default::default();
            s.metrics = Default::default();
            resource_ports_ui::reset(s);
            s.system_logs = Default::default();
            s.session_rules.clear();
            s.queue_rules.clear();
            s.workbench.reset_open_windows();
            s.workbench.fullscreen = false;
            s.workspace_page = if s.workbench.active == server_desktop::Tab::Files {
                WorkspacePage::Files
            } else {
                WorkspacePage::Desktop
            };
        });
        self.send(Action::Disconnect);
    }
    fn cancel_all(&self) {
        self.state.revise(|s| {
            s.transfer_now = std::time::Instant::now();
            for m in s.transfer_meters.values_mut() {
                m.finish(s.transfer_now);
            }
            s.transfer_pauses.clear();
            s.explorer_uploads.abort();
            for c in s.cancels.values() {
                c.store(true, Ordering::Relaxed);
            }
            for j in &mut s.jobs {
                if matches!(j.status, TransferStatus::Queued | TransferStatus::Running) {
                    j.status = TransferStatus::Cancelled;
                }
            }
            let cancelled: Vec<_> = s
                .jobs
                .iter()
                .filter(|job| job.status == TransferStatus::Cancelled)
                .map(|job| job.id.clone())
                .collect();
            for id in cancelled {
                transfer_details::finish(s, &id, &TransferStatus::Cancelled);
            }
            s.transfer_conflict = None;
            s.transfer_conflicts.clear();
        });
    }
    fn authenticate(&self, remember: bool) {
        let details = self.state.with(|s| {
            let profile = s
                .config
                .servers
                .iter()
                .find(|p| Some(&p.id) == s.selected.as_ref())?;
            Some((
                profile.host.clone(),
                profile.port,
                s.fingerprint.clone()?,
                s.secret.clone(),
                arcnix_core::browser::profile_key(profile, s.fingerprint.as_deref()?),
            ))
        });
        if let Some((host, port, fingerprint, secret, sync_scope)) = details {
            if remember {
                let result = self
                    .state
                    .revise(|s| s.config.trust_host(&host, port, &fingerprint));
                if let Err(e) = result {
                    self.log(e.to_string());
                    return;
                }
                self.save();
            }
            self.state.revise(|s| {
                s.sync_scope = Some(sync_scope);
                s.fingerprint = None;
                s.secret.clear();
            });
            self.send(Action::Authenticate {
                secret,
                fingerprint,
            });
        }
    }
    fn refresh(&self, remote: bool) {
        let path = self.state.revise(|s| {
            s.file_menu = None;
            s.file_context = None;
            s.file_browser.reset(remote);
            if remote {
                s.remote_ready = false;
                s.remote_selection.clear();
                s.remote_path.clone()
            } else {
                s.local_ready = false;
                s.local_selection.clear();
                s.local_path.clone()
            }
        });
        let serial = self.send(if remote {
            Action::Remote(path)
        } else {
            Action::Local(path.into())
        });
        self.state.revise(|s| {
            if remote {
                s.remote_request = serial
            } else {
                s.local_request = serial
            }
        });
    }
    fn transfer(&self, direction: TransferDirection) {
        let entries = self
            .state
            .with(|s| file_browser::selected_entries(s, direction == TransferDirection::Download));
        self.transfer_entries(direction, entries);
    }
    fn transfer_entries(&self, direction: TransferDirection, entries: Vec<FileEntry>) {
        if direction == TransferDirection::Download
            && entries.iter().any(|f| !safe_local_name(&f.name))
        {
            self.log("下载已拒绝：所选远程名称无法安全保存为本地文件名");
            return;
        }
        let jobs = self.state.with(|s| {
            if !file_browser::transfer_ready(s) {
                return Vec::new();
            }
            let Some(server_id) = s.selected.clone() else {
                return Vec::new();
            };
            entries
                .iter()
                .map(|f| TransferJob {
                    id: new_id(),
                    server_id: server_id.clone(),
                    direction,
                    local_path: if direction == TransferDirection::Upload {
                        PathBuf::from(&f.path)
                    } else {
                        Path::new(&s.local_path).join(&f.name)
                    },
                    remote_path: if direction == TransferDirection::Download {
                        f.path.clone()
                    } else {
                        remote_join(&s.remote_path, &f.name)
                    },
                    total_bytes: if f.is_dir { 0 } else { f.size },
                    transferred_bytes: 0,
                    status: TransferStatus::Queued,
                })
                .collect::<Vec<_>>()
        });
        if jobs.is_empty() {
            self.log("请先连接并成功读取两个目录，再选择文件或目录；不传输符号链接");
            return;
        }
        self.state.revise(|s| enqueue_transfers(s, jobs));
        self.schedule();
    }
    fn schedule(&self) {
        let limit = self.state.with(|s| s.config.settings.transfer_concurrency);
        self.send(Action::TransferConcurrency(limit));
        let next = self.state.revise(|s| {
            let mut next = Vec::new();
            if s.system_task_running
                || s.deletion.is_some()
                || s.remote_editor.is_some()
                || s.properties.as_ref().is_some_and(|p| p.is_running())
                || !s.connected
                || s.queue_paused
            {
                return next;
            }
            while s.cancels.len() < usize::from(limit) {
                let index = s.jobs.iter().position(|job| {
                    job.status == TransferStatus::Queued
                        && !workspace::transfer_paused(s, &job.id)
                        && Some(&job.server_id) == s.selected.as_ref()
                        && !s.jobs.iter().any(|active| {
                            s.cancels.contains_key(&active.id)
                                && transfer_worker::overlap(job, active)
                        })
                });
                let Some(index) = index else {
                    break;
                };
                let job = &mut s.jobs[index];
                job.status = TransferStatus::Running;
                s.inflight_transfers.insert(job.id.clone());
                s.transfer_now = std::time::Instant::now();
                s.transfer_meters.insert(
                    job.id.clone(),
                    transfer_progress::Meter::new(s.transfer_now),
                );
                let cancel = Arc::new(AtomicBool::new(false));
                s.cancels.insert(job.id.clone(), cancel.clone());
                let paused = s.transfer_pauses.entry(job.id.clone()).or_default().clone();
                next.push((job.clone(), cancel, paused));
            }
            next
        });
        for (job, cancel, paused) in next {
            let policy = self
                .state
                .with(|s| transfer_ui::policy(s, &job.id, job.direction));
            self.send(Action::TransferInteractive {
                job,
                cancel,
                paused,
                policy,
            });
        }
    }
    fn mutate_file(&self, remote: bool, action: FileAction) {
        let operation = self.state.with(|s| {
            if (remote && !s.remote_ready) || (!remote && !s.local_ready) {
                return Err("请先成功读取目标目录");
            }
            let name = s.name.trim();
            if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
                return Err("请输入不含路径分隔符的文件名");
            }
            if !remote && !safe_local_name(name) {
                return Err("本地名称包含无效字符、设备名或路径前缀");
            }
            let base = if remote {
                &s.remote_path
            } else {
                &s.local_path
            };
            if action == FileAction::Rename {
                let selected = if remote {
                    &s.remote_selection
                } else {
                    &s.local_selection
                };
                if selected.len() != 1 {
                    return Err("重命名时请选择一个项目");
                }
                let from = selected.iter().next().unwrap().clone();
                let to = if remote {
                    remote_join(
                        from.rsplit_once('/')
                            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
                            .unwrap_or("."),
                        name,
                    )
                } else {
                    Path::new(&from)
                        .parent()
                        .unwrap_or_else(|| Path::new(base))
                        .join(name)
                        .to_string_lossy()
                        .into_owned()
                };
                return Ok(if remote {
                    Action::RemoteRename(from, to)
                } else {
                    Action::LocalRename(from.into(), to.into())
                });
            }
            Ok(match (action, remote) {
                (FileAction::CreateFile, true) => Action::RemoteCreateFile(remote_join(base, name)),
                (FileAction::CreateFile, false) => {
                    Action::LocalCreateFile(Path::new(base).join(name))
                }
                (FileAction::CreateFolder, true) => Action::RemoteMkdir(remote_join(base, name)),
                (FileAction::CreateFolder, false) => Action::LocalMkdir(Path::new(base).join(name)),
                (FileAction::Rename, _) => unreachable!(),
            })
        });
        match operation {
            Ok(action) => {
                let serial = self.send(action);
                self.state.revise(|s| {
                    s.file_pending = Some(serial);
                    s.dialog_error.clear();
                });
            }
            Err(e) => self.state.revise(|s| s.dialog_error = e.into()),
        }
    }
}
fn import_profiles(
    existing: &AppConfig,
    profiles: Vec<ServerProfile>,
) -> anyhow::Result<(AppConfig, usize, usize)> {
    let mut config = existing.clone();
    let mut imported = 0;
    let mut skipped = 0;
    for mut profile in profiles {
        if config.servers.iter().any(|p| {
            p.host.eq_ignore_ascii_case(&profile.host)
                && p.port == profile.port
                && p.username == profile.username
                && p.auth == profile.auth
                && p.private_key == profile.private_key
        }) {
            skipped += 1;
            continue;
        }
        let group = if let Some(g) = config
            .groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case("SSH Config"))
        {
            g.id.clone()
        } else {
            config.add_group("SSH Config")?
        };
        profile.group_id = Some(group);
        config.upsert_server(profile)?;
        imported += 1;
    }
    config.validate()?;
    Ok((config, imported, skipped))
}
fn pane_entries(s: &State, remote: bool) -> Vec<FileEntry> {
    file_browser::entries(s, remote)
}
fn safe_local_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '\0']) {
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        if name.ends_with(['.', ' ']) || name.chars().any(|c| c < ' ' || "<>\"|?*".contains(c)) {
            return false;
        }
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return false;
        }
    }
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none()
}
fn remote_join(base: &str, name: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), name)
}
fn local_display(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{unc}");
    }
    value.strip_prefix("\\\\?\\").unwrap_or(&value).to_string()
}
fn accepts_listing(latest: u64, received: u64) -> bool {
    latest != 0 && latest == received
}
/// A running file task keeps the dialog slot until it finishes.
fn file_task_busy(s: &State) -> bool {
    s.file_pending.is_some()
        && (s.deletion.is_some()
            || s.properties.as_ref().is_some_and(|p| p.is_running())
            || remote_editor::busy(s))
}
fn dismiss_menus(s: &mut State) {
    s.host_menu = None;
    s.group_menu = None;
    s.file_menu = None;
}
/// Shows `dialog` in place of any open menu. Callers check [`file_task_busy`].
fn show_dialog(s: &mut State, dialog: Dialog) {
    if dialog == Dialog::Server {
        s.server_editor_tab = 0;
    }
    s.dialog = Some(dialog);
    s.dialog_error.clear();
    dismiss_menus(s);
}
/// Whether connecting to the selected server must go through the credential
/// dialog: always while a connection exists or is starting, and otherwise
/// unless [`needs_no_credentials`] holds for its profile.
fn connect_prompt_needed(s: &State) -> bool {
    s.connecting
        || s.connected
        || !s
            .config
            .servers
            .iter()
            .find(|p| Some(&p.id) == s.selected.as_ref())
            .is_some_and(needs_no_credentials)
}
/// Whether connecting to `profile` needs no password or key passphrase.
///
/// Only private keys recognised as unencrypted qualify; passwords, encrypted
/// keys and keys that cannot be inspected keep the credential dialog.
fn needs_no_credentials(profile: &ServerProfile) -> bool {
    use arcnix_core::private_keys::{KeyProtection, key_protection};
    profile.auth == AuthMethod::PrivateKey
        && profile
            .private_key
            .as_deref()
            .is_some_and(|key| key_protection(key) == KeyProtection::Unencrypted)
}
fn accepts_response(current: u64, received: u64) -> bool {
    current == received
}
fn finished_status(result: std::result::Result<(), String>, cancelled: bool) -> TransferStatus {
    match result {
        Ok(()) => TransferStatus::Completed,
        Err(_) if cancelled => TransferStatus::Cancelled,
        Err(e) => TransferStatus::Failed(e),
    }
}
fn modified(timestamp: Option<u64>) -> String {
    formatting::modified(timestamp)
}
fn size(bytes: u64) -> String {
    formatting::bytes(bytes)
}
fn open_url(url: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::{HSTRING, w};
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(url),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        anyhow::ensure!(result.0 as isize > 32, "Could not open the link");
    }
    #[cfg(not(windows))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(opener).arg(url).spawn()?;
    }
    Ok(())
}

fn btn(label: impl Into<String>, action: impl FnMut() + 'static) -> sapid::Button {
    sapid::button()
        .label(i18n::bind(label.into()))
        .class("button")
        .on_press(action)
}
fn field(
    label: &'static str,
    ui: &Ui,
    read: fn(&State) -> String,
    write: fn(&mut State, String),
) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(read))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        write(s, value);
                        s.dialog_error.clear();
                    });
                }
            }),
    ))
}
fn enqueue_transfers(s: &mut State, jobs: Vec<TransferJob>) {
    let batch = new_id();
    for job in &jobs {
        s.batch_ids.insert(job.id.clone(), batch.clone());
    }
    s.jobs.extend(jobs);
}

fn modal(
    ui: Ui,
    title: impl Into<String>,
    body: impl IntoChildren + 'static,
    footer: impl IntoChildren + 'static,
) -> impl Widget {
    let mut content = Some((title.into(), body, footer));
    widget(move |scope| {
        let (title, body, footer) = content
            .take()
            .expect("dialog builds once per mounted scope");
        let close = ui.clone();
        let scrim = ui.clone();
        let focus = scope.cage(None);
        let mounted = scope.mount_with(|| {
            col()
                .class(ui.state.map(|s| {
                    if server_desktop::owns_dialog(s) && s.fingerprint.is_none() {
                        server_apps_ui::themed(s, "modal-backdrop desktop-modal-backdrop")
                    } else {
                        "modal-backdrop".to_owned()
                    }
                }))
                .fill((
                    button()
                        .class("modal-scrim")
                        .label(i18n::bind("关闭对话框"))
                        .focus(FocusMode::Programmatic)
                        .enabled(ui.state.map(|s| {
                            (s.file_pending.is_none()
                                || file_context::deletion_dialog(s)
                                || file_properties::running(s)
                                || (s.dialog == Some(Dialog::RemoteEditor)
                                    && remote_editor::busy(s)))
                                && s.import_pending.is_none()
                        }))
                        .on(activate, move |_| scrim.close_dialog()),
                    col()
                        .class(ui.state.map(|s| {
                            match s.dialog {
                                Some(Dialog::RemoteEditor) => "modal-card remote-editor-modal",
                                Some(Dialog::TransferProgress(_)) => {
                                    "modal-card transfer-progress-modal"
                                }
                                _ => "modal-card",
                            }
                            .to_owned()
                        }))
                        .role(Role::Dialog)
                        .name(i18n::bind(title.clone()))
                        .focus(FocusMode::Programmatic)
                        .node_ref(focus)
                        .fill((
                            row().class("modal-header").fill((
                                text().class("dialog-title").content(i18n::bind(title)),
                                close_action("关闭对话框")
                                    .enabled(ui.state.map(|s| {
                                        (s.file_pending.is_none()
                                            || file_context::deletion_dialog(s)
                                            || file_properties::running(s)
                                            || (s.dialog == Some(Dialog::RemoteEditor)
                                                && remote_editor::busy(s)))
                                            && s.import_pending.is_none()
                                    }))
                                    .on_press(move || close.close_dialog()),
                            )),
                            col()
                                .class("modal-body")
                                .semantics(ui.state.map(|s| {
                                    Some(Box::new(Semantics {
                                        inert: s.file_pending.is_some()
                                            || s.import_pending.is_some(),
                                        ..Default::default()
                                    }))
                                }))
                                .fill(body),
                            text()
                                .class(ui.state.map(|s| {
                                    if s.dialog_error.is_empty() {
                                        "hidden"
                                    } else {
                                        "form-error"
                                    }
                                    .to_string()
                                }))
                                .content(i18n::bind(ui.state.map(|s| s.dialog_error.clone()))),
                            row().class("modal-footer").fill(footer),
                        )),
                ))
        })?;
        scope.retain(mounted)?;
        if let Some(id) = focus.get() {
            scope.action(glory::render::Action::Focus(id))?;
        }
        Ok(())
    })
}

fn connection_target(state: Cage<State>) -> impl Widget {
    text().class("connection-target").content(state.map(|s| {
        if !s.connected {
            return String::new();
        }
        s.config
            .servers
            .iter()
            .find(|p| Some(&p.id) == s.selected.as_ref())
            .map(|p| format!("{}@{}", p.username, p.host))
            .unwrap_or_default()
    }))
}

fn connection_controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    row().class("connection-controls").fill((
        connection_target(state),
        text()
            .class(state.map(|s| {
                if s.connected {
                    "connection-state online"
                } else if s.connecting {
                    "connection-state connecting"
                } else {
                    "connection-state"
                }
                .to_string()
            }))
            .content(i18n::bind(state.map(|s| {
                if s.connected {
                    "SSH 已连接"
                } else if s.connecting {
                    "SSH 正在连接…"
                } else if s.config.servers.is_empty() {
                    "尚未添加服务器"
                } else if s.selected.is_none() {
                    "请选择左侧服务器"
                } else {
                    "SSH 未连接"
                }
                .to_string()
            }))),
        sapid::button()
            .class(state.map(|s| {
                if s.connected || s.connecting {
                    "button connection-button"
                } else {
                    "button primary connection-button"
                }
                .to_string()
            }))
            .label(i18n::bind(state.map(|s| {
                if s.connected {
                    "断开连接"
                } else if s.connecting {
                    "取消连接"
                } else if s.config.servers.is_empty() {
                    "添加服务器"
                } else {
                    "Desktop"
                }
                .to_string()
            })))
            .enabled(state.map(|s| {
                !s.system_task_running && (s.selected.is_some() || s.config.servers.is_empty())
            }))
            .on_press(move || {
                if state.with(|s| s.connected || s.connecting) {
                    ui.disconnect();
                } else if state.with(|s| s.config.servers.is_empty()) {
                    servers::new_server(&ui);
                } else {
                    ui.open_workbench(server_desktop::Tab::Desktop);
                }
            }),
    ))
}

fn app(ui: Ui) -> impl Widget {
    let state = ui.state;
    let menus = ui.clone();
    let trust = ui.clone();
    let once = ui.clone();
    let reject = ui.clone();
    let keyboard_ui = ui.clone();
    col()
        .class(state.map(|s| {
            if s.config.settings.theme.is_dark(s.system_dark) {
                "app theme-dark"
            } else {
                "app theme-light"
            }
            .to_string()
        }))
        .on(keyboard, move |event| {
            if let Event::Keyboard {
                command: glory::render::KeyboardCommand::Dismiss,
                ..
            } = event
            {
                if keyboard_ui.state.with(|s| s.fingerprint.is_some()) {
                    keyboard_ui.disconnect();
                } else if keyboard_ui.state.with(|s| {
                    server_desktop::fullscreen(s)
                        && s.dialog.is_none()
                        && s.host_menu.is_none()
                        && s.group_menu.is_none()
                        && s.file_menu.is_none()
                        && s.file_context.is_none()
                        && s.terminal.menu.is_none()
                }) {
                    keyboard_ui.state.revise(|s| s.workbench.fullscreen = false);
                } else {
                    keyboard_ui.close_dialog();
                    keyboard_ui.state.revise(|s| {
                        s.host_menu = None;
                        s.group_menu = None;
                        s.file_menu = None;
                    });
                }
            }
        })
        .fill((
            input()
                .class("system-theme-input hidden")
                .value("")
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        let dark = match value.as_str() {
                            "dark" => true,
                            "light" => false,
                            _ => return,
                        };
                        if state.with(|s| s.system_dark != dark) {
                            state.revise(|s| s.system_dark = dark);
                        }
                    }
                }),
            col()
                .class("app-shell")
                .focus(FocusMode::Programmatic)
                .node_ref(ui.shell_focus)
                .semantics(state.map(|s| {
                    Some(Box::new(Semantics {
                        inert: s.dialog.is_some()
                            || s.fingerprint.is_some()
                            || updates_ui::applying(s),
                        ..Default::default()
                    }))
                }))
                .fill(navigation::shell(ui.clone())),
            button()
                .class(state.map(|s| {
                    if s.dialog.is_none()
                        && (s.host_menu.is_some()
                            || s.group_menu.is_some()
                            || (s.file_menu.is_some() && !file_context::desktop_menu_open(s)))
                    {
                        "menu-dismiss"
                    } else {
                        "hidden"
                    }
                    .to_string()
                }))
                .label(i18n::bind("关闭操作菜单"))
                .focus(FocusMode::Programmatic)
                .on(activate, move |_| {
                    menus.state.revise(|s| {
                        s.host_menu = None;
                        s.group_menu = None;
                        s.file_menu = None;
                    })
                }),
            fragment((
                updates_ui::banner(ui.clone()),
                servers::dialogs(ui.clone()),
                updates_ui::overlay(ui.clone()),
                workspace::file_dialog(ui.clone()),
                explorer_ui::color_dialog(ui.clone()),
                network_ui::dialog(ui.clone()),
                fragment((
                    file_context::dialogs(ui.clone()),
                    software_install_ui::dialogs(ui.clone()),
                    fragment(docker_ui::dialogs(ui.clone())),
                    terminal_menu::dialogs(ui.clone()),
                    resource_ports_ui::dialog(ui.clone()),
                )),
                remote_editor::dialogs(ui.clone()),
                file_columns::dialog(ui.clone()),
                cleanup_records_ui::dialog(ui.clone()),
                cleanup_records_ui::download_dialog(ui.clone()),
                script_exchange_ui::dialogs(ui.clone()),
            )),
            operations::dialogs(ui.clone()),
            scripts_ui::dialogs(ui.clone()),
            fragment((
                diagnostics_ui::dialogs(ui.clone()),
                task_records_ui::dialogs(ui.clone()),
                configuration_backup_ui::dialogs(ui.clone()),
                #[cfg(windows)]
                web_browser::forwarding_dialog(ui.clone()),
            )),
            record_maintenance_ui::dialog(ui.clone()),
            backups_ui::dialogs(ui.clone()),
            fragment((
                transfer_ui::dialogs(ui.clone()),
                workspace::transfer_progress_dialog(ui.clone()),
            )),
            when(state.map(|s| s.fingerprint.is_some()), move || {
                let trust = trust.clone();
                let once = once.clone();
                let reject = reject.clone();
                modal(
                    ui.clone(),
                    "确认服务器身份",
                    col().class("dialog-stack").fill((
                        text().content(i18n::bind("首次连接此服务器，请核对 SSH 主机指纹。")),
                        text()
                            .class("fingerprint")
                            .selectable(true)
                            .content(state.map(|s| s.fingerprint.clone().unwrap_or_default())),
                        text().class("hint").content(i18n::bind(
                            "确认指纹后再继续认证。可选择仅本次信任，或保存供下次连接校验。",
                        )),
                    )),
                    (
                        btn("取消", move || reject.disconnect()),
                        btn("仅本次信任", move || once.authenticate(false)),
                        sapid::button()
                            .class("button primary")
                            .label(i18n::bind("信任并保存"))
                            .on_press(move || trust.authenticate(true)),
                    ),
                )
            }),
        ))
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "--apply-update") {
        if arguments.len() != 2 {
            return Err("--apply-update requires one prepared plan".into());
        }
        update_install::apply(Path::new(&arguments[1]))?;
        return Ok(());
    }
    if arguments == ["--version"] {
        println!("arcnix {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut config_path = None;
    let mut health_marker = None;
    let mut args = arguments.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--config" if config_path.is_none() => {
                config_path = Some(PathBuf::from(
                    args.next().ok_or("--config requires a path")?,
                ))
            }
            "--update-health" if health_marker.is_none() => {
                health_marker = Some(PathBuf::from(
                    args.next().ok_or("--update-health requires a path")?,
                ))
            }
            "--help" => {
                println!("Arcnix desktop\nUsage: arcnix [--config <path>] [--version]");
                return Ok(());
            }
            _ => return Err("Usage: arcnix [--config <path>] [--version]".into()),
        }
    }
    let store = Rc::new(if let Some(path) = config_path {
        ConfigStore::at(path)
    } else {
        ConfigStore::default_location()?
    });
    if let Some(path) = &health_marker {
        let root = std::fs::canonicalize(
            store
                .path()
                .parent()
                .ok_or("Missing configuration directory")?
                .join("updates"),
        )?;
        if !path.is_absolute()
            || !path
                .parent()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .is_some_and(|p| p.starts_with(&root))
            || !path.file_name().is_some_and(|p| {
                p.to_string_lossy().starts_with("startup-") && p.to_string_lossy().ends_with(".ok")
            })
        {
            return Err("Invalid update startup acknowledgement path".into());
        }
    }
    let (config, error) = match store.load() {
        Ok(c) => (c, None),
        Err(e) => (
            AppConfig::default(),
            Some(format!("配置读取失败，禁止覆盖: {e:#}")),
        ),
    };
    if health_marker.is_some() && error.is_some() {
        return Err("Updated application could not load the saved configuration".into());
    }
    let path = config
        .local_directory
        .clone()
        .unwrap_or(std::env::current_dir()?);
    let (tx, rx) = worker::start();
    let (update_controller, update_rx) = updates_ui::start()?;
    formatting::configure(config.settings.display);
    let logger = Rc::new(client_logs::Logger::new(store.path()));
    let window = WindowApp::new();
    let scope = window.app().scope();
    i18n::init(scope, config.settings.language);
    let state = scope.cage(State {
        config,
        selected: None,
        group: None,
        collapsed_groups: HashSet::new(),
        search: String::new(),
        editor: ServerProfile::default(),
        server_editor_tab: 0,
        server_editor_original: None,
        server_tags: String::new(),
        dialog: None,
        dialog_error: String::new(),
        host_menu: None,
        group_menu: None,
        file_menu: None,
        file_context: None,
        file_pending: None,
        deletion: None,
        unassigned_columns: FileColumnSettings::default(),
        properties: None,
        remote_editor: None,
        desktop_editor: Default::default(),
        updates: Default::default(),
        import_pending: None,
        activity_tab: 0,
        activity_open: true,
        local_filter_open: false,
        remote_filter_open: false,
        notice: error.clone().unwrap_or_default(),
        group_name: String::new(),
        port: "22".into(),
        secret: String::new(),
        connect_prompt_skipped: false,
        connected: false,
        sftp_available: false,
        connecting: false,
        keepalive_at: std::time::Instant::now(),
        keepalive_pending: false,
        fingerprint: None,
        sync_scope: None,
        epoch: 0,
        request_serial: 0,
        local_request: 0,
        remote_request: 0,
        local_filter: String::new(),
        remote_filter: String::new(),
        local_sort: Sort::Name,
        remote_sort: Sort::Name,
        local_desc: false,
        remote_desc: false,
        queue_paused: false,
        queue_filter: 0,
        import_path: String::new(),
        local_path: local_display(&path),
        remote_path: ".".into(),
        local_ready: false,
        remote_ready: false,
        local: vec![],
        remote: vec![],
        local_selection: HashSet::new(),
        remote_selection: HashSet::new(),
        file_browser: Default::default(),
        terminal: Default::default(),
        name: String::new(),
        jobs: vec![],
        cancels: HashMap::new(),
        transfer_pauses: HashMap::new(),
        explorer_uploads: Default::default(),
        inflight_transfers: HashSet::new(),
        logs: String::new(),
        workspace_page: WorkspacePage::Desktop,
        workbench: Default::default(),
        #[cfg(windows)]
        web_browser: Default::default(),
        server_apps: Default::default(),
        docker: Default::default(),
        disks: Default::default(),
        compose: Default::default(),
        install: Default::default(),
        navigation: Default::default(),
        settings_section: Default::default(),
        unread_notifications: 0,
        software: Default::default(),
        operations: Default::default(),
        scripts: Default::default(),
        task_records: Default::default(),
        deleted_files: Default::default(),
        tcp_listeners: Default::default(),
        record_storage: Default::default(),
        cleanup_reports: Default::default(),
        backups: Default::default(),
        diagnostics: Default::default(),
        metrics: Default::default(),
        ports: Default::default(),
        system_logs: Default::default(),
        system_task_running: false,
        settings: Default::default(),
        system_dark: true,
        file_split: 0.5,
        transfer_conflict: None,
        transfer_conflicts: Default::default(),
        transfer_details: Default::default(),
        batch_ids: HashMap::new(),
        queue_rules: HashMap::new(),
        session_rules: HashMap::new(),
        transfer_now: std::time::Instant::now(),
        transfer_meters: HashMap::new(),
        transfer_summaries: HashMap::new(),
    });
    let (batch_events, batch_rx) = async_channel::bounded(128);
    let ui = Ui {
        wallpapers: Arc::new(desktop_customization_ui::WallpaperAssets::default()),
        update_controller,
        batch_controller: Rc::new(std::cell::RefCell::new(None)),
        batch_events,
        logger,
        state,
        tx,
        store,
        writable: scope.cage(error.is_none()),
        scope: scope.clone(),
        shell_focus: scope.cage(None),
    };
    ui.log(
        error
            .as_deref()
            .unwrap_or("Arcnix 已就绪。新建服务器并连接，或浏览本地目录。"),
    );
    ui.refresh(false);
    scripts_ui::restore(&ui);
    let batch_ui = ui.clone();
    scope.spawn(async move {
        while let Ok(event) = batch_rx.recv().await {
            scripts_ui::event(&batch_ui, event);
        }
    })?;
    updates_ui::restore(&ui);
    let updates = ui.clone();
    scope.spawn(async move {
        while let Ok(event) = update_rx.recv().await {
            updates_ui::event(&updates, event);
        }
    })?;
    // Coalesce ticks while the UI is busy. Sampling itself is also gated by
    // the active page and a single pending request in observability state.
    let (ticks, tick_rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if let Err(async_channel::TrySendError::Closed(_)) = ticks.try_send(()) {
                break;
            }
        }
    });
    let polling = ui.clone();
    scope.spawn(async move {
        let mut health_marker = health_marker;
        while tick_rx.recv().await.is_ok() {
            if let Some(path) = health_marker.take() {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(path)
                {
                    let _ = writeln!(file, "arcnix {}", env!("CARGO_PKG_VERSION"));
                    let _ = file.sync_all();
                }
            }
            polling.observability_tick();
            polling.tick_desktop_clock();
            polling.tick_desktop_sync();
            #[cfg(windows)]
            browser_cookies::tick(&polling);
            polling.transfer_tick();
            preferences_ui::tick(&polling);
            updates_ui::tick(&polling);
        }
    })?;
    let events = ui.clone();
    scope.spawn(async move {while let Ok(response)=rx.recv().await {
        if let Update::Finished(id, _)=&response.update { state.revise(|s|{s.inflight_transfers.remove(id);}); }
        if !accepts_response(state.with(|s|s.epoch),response.epoch) && !matches!(&response.update, Update::Children {remote:false,..}|Update::Deleted{..}|Update::Permissions(_)|Update::TextFile(_)){continue;}
        match response.update {
            Update::Alive=>state.revise(|s|s.keepalive_pending=false),
            Update::ServerApps(result)=>server_apps_ui::event(&events,response.serial,result),
            Update::Operations(event)=>operations::event(&events,response.serial,event),
            Update::Terminal(event)=>terminal::update(&events,event),
            Update::Fingerprint(fingerprint)=>{
                let known=state.with(|s|s.config.servers.iter().find(|p|Some(&p.id)==s.selected.as_ref()).and_then(|p|s.config.trusted_fingerprint(&p.host,p.port)).map(str::to_owned));
                if known.as_ref().is_some_and(|f|f!=&fingerprint){
                    events.disconnect();events.log(format!("拒绝连接：主机密钥发生变化。已保存：{}；当前：{fingerprint}。请通过可信渠道核实后手动更新配置。",known.unwrap()));
                }else{state.revise(|s|s.fingerprint=Some(fingerprint));if known.is_some(){events.authenticate(false);}}
            }
            Update::Connected(sftp)=>{state.revise(|s|{s.connected=true;s.connecting=false;s.keepalive_pending=false;s.sftp_available=sftp;if s.workbench.active==server_desktop::Tab::Home{server_desktop::select_tab(s,server_desktop::Tab::Desktop);}});
                #[cfg(windows)]
                if state.with(|s| s.workbench.apps.contains(&WorkspacePage::Browser)) { events.open_web_browser(); }
                events.log("SSH 认证成功");events.server_apps_connected();if sftp{events.refresh(true);events.explorer_enter();}else{events.log("服务器未启用 SFTP；SSH 系统软件管理仍可使用。");}events.terminal_enter();events.observability_enter();if state.with(|s|s.workspace_page==WorkspacePage::Software){events.software_detect();}}
            Update::Disconnected=>events.log("连接已断开"),
            Update::Local(path,entries)=>{if !accepts_listing(state.with(|s|s.local_request),response.serial){continue;}state.revise(|s|{s.file_browser.reset(false);s.local_path=local_display(&path);s.local_ready=true;s.local=entries;s.local_selection.clear();s.config.local_directory=Some(path.clone());if let Some(profile)=s.config.servers.iter_mut().find(|p|Some(&p.id)==s.selected.as_ref())&& profile.locations.remember_paths{profile.locations.last_local=Some(path);}});events.save();}
            Update::Remote(path,entries)=>{if !accepts_listing(state.with(|s|s.remote_request),response.serial){continue;}state.revise(|s|{file_browser::remote_listed(s,&path);s.file_browser.reset(true);s.remote_path=path.clone();if let Some(profile)=s.config.servers.iter_mut().find(|p|Some(&p.id)==s.selected.as_ref())&& profile.locations.remember_paths{profile.locations.last_remote=Some(path);}s.remote_ready=true;s.remote=entries;s.remote_selection.clear();});events.save();},
            Update::Children{remote,parent,result}=>file_browser::accept_children(&events,remote,parent,response.serial,result),
            Update::BrowseError{remote,message}=>{let latest=state.with(|s|if remote{s.remote_request}else{s.local_request});if accepts_listing(latest,response.serial){if remote{state.revise(file_browser::remote_browse_failed);}events.error(format!("读取目录失败：{message}"));}},
            #[cfg(windows)]
            Update::BrowserProxy(result)=>web_browser::proxy_event(&events,response.serial,result),
            #[cfg(windows)]
            Update::BrowserDiagnosis(diagnosis)=>web_browser::diagnosis_event(&events,response.serial,diagnosis),
            #[cfg(windows)]
            Update::BrowserForwardingRepaired(result)=>web_browser::forwarding_repaired(&events,response.serial,result),
            Update::ConnectionLost(message)=>{
                #[cfg(windows)]
                events.close_web_browser();operations::connection_lost(&events);terminal::disconnected(&events);software::on_connection_lost(&events,&message);state.revise(|s|{s.metrics=Default::default();resource_ports_ui::reset(s);s.system_logs=Default::default();s.session_rules.clear();s.queue_rules.clear();s.transfer_conflict=None;s.transfer_conflicts.clear();s.transfer_pauses.clear();s.explorer_uploads.abort();for c in s.cancels.values(){c.store(true,Ordering::Relaxed);}if s.dialog==Some(Dialog::TransferConflict){s.dialog=None;}s.connected=false;s.sync_scope=None;s.server_apps=Default::default();s.docker=Default::default();s.disks=Default::default();s.compose=Default::default();s.install=Default::default();s.workbench.fullscreen=false;s.connecting=false;s.remote_ready=false;s.remote.clear();s.remote_selection.clear();s.remote_request=0;s.fingerprint=None;for j in &mut s.jobs {if j.status==TransferStatus::Queued{j.status=TransferStatus::Cancelled;}}});events.log(format!("SSH 连接已断开：{message}"));},
            Update::TransferConflict(id,info,reply)=>transfer_ui::show(&events,id,info,reply),
            Update::TransferPlan(id,plan)=>state.revise(|s|transfer_details::plan(s,&id,&plan)),
            Update::TransferFile(id,progress)=>state.revise(|s|{transfer_details::update(s,&id,progress);s.transfer_now=std::time::Instant::now();let waiting=transfer_details::waiting(s,&id)||workspace::transfer_paused(s,&id);if let Some(m)=s.transfer_meters.get_mut(&id){if waiting{m.pause(s.transfer_now);}else{m.resume(s.transfer_now);}}}),
            Update::TransferReport(id,summary)=>{events.log(format!("传输结果：写入 {} 个文件，跳过 {} 个文件",summary.transferred_files,summary.skipped_files));state.revise(|s|{s.transfer_summaries.insert(id,summary);});},
            Update::Progress(id,done,total,actual)=>state.revise(|s|{s.transfer_now=std::time::Instant::now();if let Some(m)=s.transfer_meters.get_mut(&id){m.observe(actual,s.transfer_now);}
                if let Some(job)=s.jobs.iter_mut().find(|j|j.id==id){job.transferred_bytes=done;job.total_bytes=total;}}),
            Update::Finished(id,result)=>{
                transfer_ui::finished(&events,&id);
                if let Err(error)=&result{events.log(format!("传输未完成：{error}"));}
                state.revise(|s|{
                    s.transfer_now=std::time::Instant::now();
                    if let Some(m)=s.transfer_meters.get_mut(&id){m.finish(s.transfer_now);}
                    s.transfer_pauses.remove(&id);
                    let cancelled=s.cancels.remove(&id).is_some_and(|c|c.load(Ordering::Relaxed));
                    if let Some(job)=s.jobs.iter_mut().find(|j|j.id==id){job.status=finished_status(result,cancelled);let status=job.status.clone();s.explorer_uploads.finish(&id,&status);transfer_details::finish(s,&id,&status);}
                });
                if state.with(|s|s.cancels.is_empty()) {events.refresh(false);if state.with(|s|s.connected){events.refresh(true);}}
                events.schedule();
            }
            Update::Log(message)=>{
                events.log(message);
                let file=state.with(|s|if s.file_pending==Some(response.serial){s.dialog.clone()}else{None});
                if let Some(Dialog::File{remote,..})=file {state.revise(|s|{s.file_pending=None;s.dialog=None;s.dialog_error.clear();s.notice.clear();});events.refresh(remote);}
            },
            Update::Deleted{remote,result}=>file_context::deleted(&events,response.serial,remote,result),
            Update::DeletionProgress(progress)=>file_context::deletion_progress(&events,response.serial,progress),
            Update::FileMetadata(result)=>file_properties::metadata(&events,response.serial,result),
            Update::Permissions(result)=>file_properties::finished(&events,response.serial,result),
            Update::TextFile(result)=>remote_editor::event(&events,response.serial,result),
            Update::Error(message)=>{
                let pending=state.with(|s|s.file_pending==Some(response.serial)||s.import_pending==Some(response.serial));
                if pending {state.revise(|s|{s.file_pending=None;s.import_pending=None;s.dialog_error=message.clone();});events.log(format!("操作失败：{message}"));}
                else {events.error(format!("错误：{message}"));}
            },
            Update::ConnectionError(message)=>{
                let message=format!("连接失败：{message}");
                // A skipped prompt must not leave the user without a way to enter a passphrase,
                // but it never replaces another dialog or interrupts a running file task.
                state.revise(|s|{s.connecting=false;s.fingerprint=None;s.sync_scope=None;s.secret.clear();
                    if std::mem::take(&mut s.connect_prompt_skipped)&&s.dialog.is_none()&&!file_task_busy(s){show_dialog(s,Dialog::Connect);s.dialog_error=message.clone();}});
                events.error(&message);
            },
            Update::Metrics(result)=>observability::metrics_event(&events,response.serial,result),
            Update::Ports(result)=>resource_ports_ui::event(&events,response.serial,result),
            Update::PortTerminated(result)=>resource_ports_ui::terminated(&events,response.serial,result),
            Update::LogCapabilities(result)=>observability::logs_capabilities_event(&events,response.serial,result),
            Update::SystemLogs(result)=>observability::logs_event(&events,response.serial,result),
            Update::PackageEnvironment(result)=>software::event(&events,response.serial,software::Event::Environment(result)),
            Update::PackageList(result)=>software::event(&events,response.serial,software::Event::Packages(result)),
            Update::Imported(report)=>{
                let warning_count=report.warnings.len();
                for warning in &report.warnings {events.log(format!("SSH 配置导入：{warning}"));}
                let result=state.with(|s|{let (config,imported,skipped)=import_profiles(&s.config,report.servers)?;anyhow::ensure!(events.writable.get(),"配置读取失败，禁止覆盖");events.store.save(&config)?;Ok::<_,anyhow::Error>((config,imported,skipped))});
                state.revise(|s|s.import_pending=None);
                match result {
                    Ok((config,imported,skipped))=>{
                        let summary=format!("导入完成：新增 {imported} 台，跳过 {skipped} 台重复站点；{warning_count} 条提示");
                        state.revise(|s|{s.config=config;s.dialog=None;s.dialog_error.clear();s.notice=if warning_count==0{String::new()}else{format!("{summary}。{}",report.warnings[0])};});events.log(summary);
                    },
                    Err(e)=>{state.revise(|s|s.dialog_error=format!("导入失败：{e:#}"));events.log(format!("导入失败：{e:#}"));}
                }
            }
        }
    }})?;
    // Locale bindings patch the existing document. Remounting it would retire
    // live button identities while their browser activations are still queued.
    window.mount(app(ui.clone()))?;
    let wallpapers = ui.wallpapers.clone();
    Shell::new().run(
        window,
        Config {
            native: Some(Box::new(app_icon::DesktopWindow {
                state,
                fullscreen: false,
                wallpapers: wallpapers.clone(),
                #[cfg(windows)]
                browser: web_browser_native::Host::new(ui.clone()),
            })),
            asset_provider: Some(wallpapers),
            initialization_script: Some(
                [
                    include_str!("../assets/system-theme.js"),
                    include_str!("../assets/desktop-frame.js"),
                    include_str!("../assets/file-list-events.js"),
                    include_str!("../assets/desktop-customization.js"),
                    include_str!("../assets/remote-editor-events.js"),
                    include_str!("../assets/help-events.js"),
                    include_str!("../assets/resource-trends.js"),
                    include_str!("../assets/server-tree-events.js"),
                    include_str!("../assets/desktop-geometry.js"),
                    include_str!("../assets/browser-viewport.js"),
                    include_str!("../assets/desktop-motion.js"),
                    include_str!("../assets/desktop-shortcuts.js"),
                    include_str!("../assets/vendor/xterm/xterm.js"),
                    include_str!("../assets/vendor/addon-fit/addon-fit.js"),
                    include_str!("../assets/terminal.js"),
                ]
                .join("\n;\n"),
            ),
            title: i18n::translate("Arcnix — 服务器管理工作台"),
            width: 1440.,
            height: 980.,
            css: format!(
                "{}\n{}\n{}\n{}",
                include_str!("style.css"),
                icons::css(),
                desktop_customization_ui::customization_css(),
                include_str!("../assets/vendor/xterm/xterm.css")
            ),
            ..Default::default()
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_unencrypted_private_keys_skip_the_credential_dialog() {
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory = std::env::temp_dir().join(format!("arcnix-connect-prompt-{}", new_id()));
        std::fs::create_dir(&directory).unwrap();
        let _cleanup = Cleanup(directory.clone());
        let pem = |label: &str, headers: &str| {
            format!("-----BEGIN {label}-----\n{headers}QUJDRA==\n-----END {label}-----\n")
        };
        let unencrypted = directory.join("plain.pem");
        std::fs::write(&unencrypted, pem("PRIVATE KEY", "")).unwrap();
        let encrypted = directory.join("encrypted.pem");
        std::fs::write(&encrypted, pem("ENCRYPTED PRIVATE KEY", "")).unwrap();
        let unknown = directory.join("unknown.ppk");
        std::fs::write(&unknown, "PuTTY-User-Key-File-3: ssh-ed25519\n").unwrap();

        let with_key = |path: Option<PathBuf>| ServerProfile {
            auth: AuthMethod::PrivateKey,
            private_key: path,
            ..Default::default()
        };
        assert!(needs_no_credentials(&with_key(Some(unencrypted.clone()))));
        assert!(!needs_no_credentials(&with_key(Some(encrypted))));
        assert!(!needs_no_credentials(&with_key(Some(unknown))));
        assert!(!needs_no_credentials(&with_key(Some(
            directory.join("missing")
        ))));
        assert!(!needs_no_credentials(&with_key(None)));
        // A leftover key path must not bypass the prompt for password logins.
        let password = ServerProfile {
            auth: AuthMethod::Password,
            private_key: Some(unencrypted),
            ..Default::default()
        };
        assert!(!needs_no_credentials(&password));
    }
    #[test]
    fn import_preserves_alternate_identities_and_reuses_group_case() {
        let mut existing = AppConfig::default();
        existing.add_group("ssh config").unwrap();
        let first = ServerProfile {
            name: "first".into(),
            host: "localhost".into(),
            username: "user".into(),
            ..Default::default()
        };
        let mut alternate = first.clone();
        alternate.id = new_id();
        alternate.name = "alternate".into();
        alternate.auth = AuthMethod::PrivateKey;
        alternate.private_key = Some("key.pem".into());
        let (config, added, skipped) =
            import_profiles(&existing, vec![first.clone(), first, alternate]).unwrap();
        assert_eq!((added, skipped), (2, 1));
        assert_eq!(config.groups.len(), 1);
        assert_eq!(config.servers.len(), 2);
    }
    #[test]
    fn failed_import_leaves_original_untouched() {
        let existing = AppConfig::default();
        let good = ServerProfile {
            name: "valid".into(),
            host: "localhost".into(),
            username: "user".into(),
            ..Default::default()
        };
        assert!(import_profiles(&existing, vec![good, ServerProfile::default()]).is_err());
        assert!(existing.servers.is_empty());
        assert!(existing.groups.is_empty());
    }
    #[test]
    fn directory_response_cannot_restore_invalidated_draft() {
        assert!(!accepts_listing(0, 42));
        assert!(!accepts_listing(43, 42));
        assert!(accepts_listing(43, 43));
    }
    #[test]
    fn old_server_responses_cannot_update_new_session() {
        assert!(!accepts_response(3, 2));
        assert!(accepts_response(3, 3));
        assert!(!accepts_response(3, 4));
    }
    #[test]
    fn late_cancel_cannot_hide_successful_transfer() {
        assert_eq!(finished_status(Ok(()), true), TransferStatus::Completed);
        assert_eq!(
            finished_status(Err("cancelled".into()), true),
            TransferStatus::Cancelled
        );
        assert_eq!(
            finished_status(Err("denied".into()), false),
            TransferStatus::Failed("denied".into())
        );
    }
    #[test]
    fn remote_paths_use_posix_separators() {
        assert_eq!(
            remote_join("/var/log/", "arcnix.log"),
            "/var/log/arcnix.log"
        );
        assert_eq!(remote_join("/", "中文.txt"), "/中文.txt");
    }
    #[test]
    fn downloaded_names_cannot_escape_local_directory() {
        for name in [
            "../escape",
            "..",
            "C:escape",
            "C:\\escape",
            "a/b",
            "a\\b",
            "",
        ] {
            assert!(!safe_local_name(name), "{name}");
        }
        assert!(safe_local_name("中文 file.txt"));
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_special_names_rejected() {
        for name in [
            "CON",
            "nul.txt",
            "COM1.log",
            "LPT3",
            "file.",
            "file ",
            "stream:secret",
        ] {
            assert!(!safe_local_name(name), "{name}");
        }
    }
}
