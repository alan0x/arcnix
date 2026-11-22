//! Independent bounded SSH jobs; preparation never executes the reviewed plan.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

use anyhow::Result;
use arcnix_core::remote::{PendingConnection, RemoteSession};
use arcnix_core::scripts::Template;
use arcnix_core::tasks::Status;
use arcnix_core::{ServerProfile, operations, packages};
use zeroize::Zeroizing;
#[path = "export_download_worker.rs"]
mod export_download;
#[path = "task_stop_worker.rs"]
mod task_stops;
pub use export_download::{Update as DownloadUpdate, download_export, inspect_export};
pub use task_stops::{Update as StopUpdate, execute_stops, inspect_stops};

#[derive(Clone)]
pub struct Target {
    pub profile: ServerProfile,
    pub fingerprint: String,
    pub secret: Zeroizing<String>,
    pub sudo: Zeroizing<String>,
}
pub struct Snapshot {
    pub observation: arcnix_core::remote_tasks::Observation,
    pub received_at: Option<u64>,
}
impl Snapshot {
    fn received(observation: arcnix_core::remote_tasks::Observation) -> Self {
        Self {
            observation,
            received_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|time| time.as_secs()),
        }
    }
}
pub enum Event {
    TaskStop {
        serial: u64,
        task: String,
        server: String,
        update: StopUpdate,
    },
    ExportDownload {
        serial: u64,
        server: String,
        update: DownloadUpdate,
    },
    CleanupRecords {
        serial: u64,
        id: String,
        result: Result<CleanupResponse, String>,
    },
    RecordStorage {
        serial: u64,
        id: String,
        result: Result<StorageResponse, String>,
    },
    RecordPage {
        serial: u64,
        id: String,
        result: Result<(String, arcnix_core::task_records::Page), String>,
    },
    RecordDetail {
        serial: u64,
        id: String,
        result: Result<arcnix_core::task_records::Detail, String>,
    },
    ConfigurationPreview {
        task: String,
        id: String,
        diff: String,
    },
    Observed {
        task: String,
        id: String,
        snapshot: Snapshot,
    },
    Snapshots(u64, Result<Vec<arcnix_core::backups::Snapshot>, String>),
    SnapshotFiles(u64, Result<arcnix_core::snapshot_files::Page, String>),
    Refreshed {
        task: String,
        id: String,
        result: Result<Snapshot, String>,
    },
    Target {
        task: String,
        id: String,
        status: Status,
        preview: Option<String>,
        output: String,
        code: Option<i32>,
    },
    Directory {
        task: String,
        id: String,
        directory: String,
    },
    Prepared(String),
    Complete(String),
}
pub enum StorageResponse {
    Inventory(Box<arcnix_core::record_maintenance::Inventory>),
    Preview(Box<arcnix_core::record_maintenance::Preview>),
}
pub enum CleanupResponse {
    Page(Box<arcnix_core::cleanup_records::Page>),
    Detail(Box<arcnix_core::cleanup_records::Detail>),
    Storage(Box<arcnix_core::audit_storage::View>),
    AuditDetail(Box<arcnix_core::audit_details::Detail>),
}
pub fn cleanup_records(
    serial: u64,
    target: Target,
    query: arcnix_core::cleanup_records::Query,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            use arcnix_core::cleanup_records as records;
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            let (root, sudo) = operations::identity(&remote)?;
            let execution = records::execution(&query, root, sudo, !target.sudo.is_empty())?;
            let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
            let text = record_text_with_input(
                &remote,
                &execution.command,
                execution.use_stdin_password.then_some(input.as_bytes()),
                Duration::from_secs(45),
            )?;
            match query {
                records::Query::AuditDetail(id) => {
                    let mut detail = arcnix_core::audit_details::parse(&text, &id)?;
                    if let Some(error) = &mut detail.error {
                        *error = arcnix_core::tasks::redact_log(
                            error,
                            &[target.secret.as_str(), target.sudo.as_str()],
                        );
                    }
                    Ok(CleanupResponse::AuditDetail(Box::new(detail)))
                }
                records::Query::Storage(request) => Ok(CleanupResponse::Storage(Box::new(
                    arcnix_core::audit_storage::parse(&text, &request)?,
                ))),
                records::Query::List(offset) => Ok(CleanupResponse::Page(Box::new(
                    records::parse_page(&text, offset)?,
                ))),
                records::Query::Detail(id) => {
                    let mut detail = records::parse_detail(&text, &id)?;
                    if let Some(error) = &mut detail.error {
                        *error = arcnix_core::tasks::redact_log(
                            error,
                            &[target.secret.as_str(), target.sudo.as_str()],
                        );
                    }
                    Ok(CleanupResponse::Detail(Box::new(detail)))
                }
            }
        })()
        .map_err(|error| configuration_record_error(error, &target));
        let _ = events.send_blocking(Event::CleanupRecords {
            serial,
            id: target.profile.id.clone(),
            result,
        });
    });
}
pub fn record_storage(
    serial: u64,
    target: Target,
    request: arcnix_core::record_maintenance::Request,
    account: Option<arcnix_core::record_maintenance::Account>,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            use arcnix_core::record_maintenance as storage;
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            let observed = remote.exec(storage::account_command())?;
            anyhow::ensure!(
                observed.exit_code == 0,
                "Cannot read the server account identity"
            );
            let observed = storage::parse_account(&observed.stdout)?;
            anyhow::ensure!(
                account
                    .as_ref()
                    .is_none_or(|previous| previous == &observed),
                "Server account context changed; reconnect before inspecting storage"
            );
            let (root, sudo) = operations::identity(&remote)?;
            anyhow::ensure!(
                root == (observed.uid == 0),
                "Server account identity changed during inspection"
            );
            let execution =
                storage::execution(&observed, &request, root, sudo, !target.sudo.is_empty())?;
            let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
            let text = record_text_with_input(
                &remote,
                &execution.command,
                execution.use_stdin_password.then_some(input.as_bytes()),
                Duration::from_secs(45),
            )?;
            let redact = |rows: &mut Vec<storage::Record>| {
                if request.scope == storage::Scope::Task {
                    for row in rows {
                        row.title = arcnix_core::tasks::redact_log(
                            &row.title,
                            &[target.secret.as_str(), target.sudo.as_str()],
                        );
                    }
                }
            };
            match &request.action {
                storage::Action::List(_) => {
                    let mut page = storage::parse_inventory(&text, &observed, &request)?;
                    redact(&mut page.records);
                    Ok(StorageResponse::Inventory(Box::new(page)))
                }
                storage::Action::Preview(_) => {
                    let mut preview = storage::parse_preview(&text, &observed, &request)?;
                    redact(&mut preview.records);
                    Ok(StorageResponse::Preview(Box::new(preview)))
                }
            }
        })()
        .map_err(|error| configuration_record_error(error, &target));
        let _ = events.send_blocking(Event::RecordStorage {
            serial,
            id: target.profile.id.clone(),
            result,
        });
    });
}
fn record_text(remote: &RemoteSession, command: &str) -> Result<String> {
    record_text_with_input(remote, command, None, Duration::from_secs(30))
}
fn record_text_with_input(
    remote: &RemoteSession,
    command: &str,
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<String> {
    record_response(remote, command, input, timeout, false)
}
fn record_upload(remote: &RemoteSession, command: &str, input: &[u8]) -> Result<String> {
    record_response(remote, command, Some(input), Duration::from_secs(30), true)
}
fn record_response(
    remote: &RemoteSession,
    command: &str,
    input: Option<&[u8]>,
    timeout: Duration,
    upload: bool,
) -> Result<String> {
    let mut output = Vec::new();
    let mut error = Vec::new();
    let mut oversized = false;
    let mut collect = |stream, bytes: &[u8]| {
        let target = if matches!(stream, arcnix_core::remote::CommandStream::Stdout) {
            &mut output
        } else {
            &mut error
        };
        if target.len() + bytes.len() <= 2 * 1024 * 1024 {
            target.extend_from_slice(bytes);
        } else {
            oversized = true;
        }
    };
    let result = if upload {
        remote.exec_task_upload(command, input.unwrap_or_default(), timeout, &mut collect)
    } else {
        remote.exec_stream(command, input, timeout, &mut collect)
    }?;
    anyhow::ensure!(!oversized, "Server record response is too large");
    anyhow::ensure!(
        result.exit_code == 0,
        "Cannot read server records (exit {}): {}",
        result.exit_code,
        String::from_utf8_lossy(&error)
    );
    Ok(String::from_utf8_lossy(&output).into_owned())
}
fn configuration_record_error(error: anyhow::Error, target: &Target) -> String {
    arcnix_core::tasks::redact_log(
        &format!("{error:#}"),
        &[target.secret.as_str(), target.sudo.as_str()],
    )
}
fn record_error(error: anyhow::Error, target: &Target) -> String {
    let value = format!("{error:#}");
    if target.secret.is_empty() {
        value
    } else {
        value.replace(target.secret.as_str(), "[redacted]")
    }
}
fn redact_record(record: &mut arcnix_core::task_records::Record, target: &Target) {
    if let Some(metadata) = &mut record.metadata {
        for value in [&mut metadata.name, &mut metadata.revision] {
            if !target.secret.is_empty() {
                *value = value.replace(target.secret.as_str(), "[redacted]");
            }
        }
    }
}
pub fn record_page(
    serial: u64,
    target: Target,
    home: Option<String>,
    expected_root: Option<String>,
    offset: u32,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            let home = if let Some(home) = home {
                home
            } else {
                let output = remote.exec(arcnix_core::task_records::home_command())?;
                anyhow::ensure!(
                    output.exit_code == 0,
                    "Cannot read the remote home directory"
                );
                output.stdout
            };
            let text = record_text(
                &remote,
                &arcnix_core::task_records::list_command(&home, offset)?,
            )?;
            let mut page = arcnix_core::task_records::parse_page(&text, offset)?;
            anyhow::ensure!(
                expected_root.is_none_or(|root| root == page.root),
                "Server task root changed; reconnect to browse records"
            );
            for record in &mut page.records {
                redact_record(record, &target);
            }
            Ok((home, page))
        })()
        .map_err(|error| record_error(error, &target));
        let _ = events.send_blocking(Event::RecordPage {
            serial,
            id: target.profile.id.clone(),
            result,
        });
    });
}
pub fn record_detail(
    serial: u64,
    target: Target,
    home: String,
    root: String,
    id: String,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            let text = record_text(
                &remote,
                &arcnix_core::task_records::detail_command(&home, &root, &id)?,
            )?;
            let mut detail = arcnix_core::task_records::parse_detail(&text, &id, &root)?;
            redact_record(&mut detail.record, &target);
            detail.output =
                arcnix_core::tasks::redact_log(&detail.output, &[target.secret.as_str()]);
            Ok(detail)
        })()
        .map_err(|error| record_error(error, &target));
        let _ = events.send_blocking(Event::RecordDetail {
            serial,
            id: target.profile.id.clone(),
            result,
        });
    });
}
pub struct Refresh {
    pub id: String,
    pub directory: String,
    pub target: Target,
}
pub fn refresh_many(
    task: String,
    targets: Vec<Refresh>,
    concurrency: usize,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let count = concurrency.clamp(1, 8).min(targets.len());
        let targets = Mutex::new(targets.into_iter());
        std::thread::scope(|scope| {
            for _ in 0..count {
                let task = &task;
                let targets = &targets;
                let events = &events;
                scope.spawn(move || {
                    loop {
                        let Some(request) = targets.lock().unwrap().next() else {
                            break;
                        };
                        refresh_one(task, request, events);
                    }
                });
            }
        });
    });
}
fn refresh_one(task: &str, request: Refresh, events: &async_channel::Sender<Event>) {
    let Refresh {
        id,
        directory,
        target,
    } = request;
    let result = (|| -> Result<_> {
        let pending = PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?;
        let remote = pending.authenticate(&target.secret, &target.fingerprint)?;
        let mut snapshot =
            Snapshot::received(arcnix_core::remote_tasks::query(&remote, task, &directory)?);
        snapshot.observation.output =
            arcnix_core::tasks::redact_log(&snapshot.observation.output, &[target.secret.as_str()]);
        Ok(snapshot)
    })()
    .map_err(|e| configuration_record_error(e, &target));
    let _ = events.send_blocking(Event::Refreshed {
        task: task.into(),
        id,
        result,
    });
}
pub fn read_command(
    remote: &RemoteSession,
    command: &str,
    administrator: bool,
    password: &str,
) -> Result<String> {
    let (root, sudo) = operations::identity(remote)?;
    let plan = operations::Plan {
        label: "Read backup repository".into(),
        script: command.into(),
        administrator,
        is_root: root,
        sudo_available: sudo,
    };
    let execution = operations::execution(&plan, !password.is_empty())?;
    let input = Zeroizing::new(format!("{password}\n"));
    let mut text = Vec::new();
    let mut error = Vec::new();
    let mut oversized = false;
    let result = remote.exec_stream(
        &execution.command,
        execution.use_stdin_password.then_some(input.as_bytes()),
        Duration::from_secs(60),
        |stream, bytes| {
            let destination = match stream {
                arcnix_core::remote::CommandStream::Stdout => &mut text,
                arcnix_core::remote::CommandStream::Stderr => &mut error,
            };
            if destination.len() + bytes.len() <= 2 * 1024 * 1024 {
                destination.extend_from_slice(bytes);
            } else {
                oversized = true;
            }
        },
    )?;
    anyhow::ensure!(!oversized, "Repository response is too large");
    let mut text = String::from_utf8(text)?;
    let mut error = String::from_utf8_lossy(&error).into_owned();
    if !password.is_empty() {
        text = text.replace(password, "[redacted]");
        error = error.replace(password, "[redacted]");
    }
    anyhow::ensure!(
        result.exit_code == 0,
        "Backup preflight/query failed (exit {}): {}{}",
        result.exit_code,
        text,
        error
    );
    Ok(text)
}
pub fn snapshots(
    serial: u64,
    policy: arcnix_core::backups::Policy,
    origin: String,
    target: Target,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            let pending =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?;
            let remote = pending.authenticate(&target.secret, &target.fingerprint)?;
            anyhow::ensure!(
                policy.servers.contains(&origin),
                "Choose a source in this policy"
            );
            let command = policy.snapshots_command(&origin)?;
            let text = read_command(&remote, &command, policy.administrator, &target.sudo)?;
            arcnix_core::backups::parse_snapshots(&text, &policy, &origin)
        })()
        .map_err(|error| configuration_record_error(error, &target));
        let _ = events.send_blocking(Event::Snapshots(serial, result));
    });
}
pub fn snapshot_files(
    serial: u64,
    policy: arcnix_core::backups::Policy,
    origin: String,
    target: Target,
    query: arcnix_core::snapshot_files::Query,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            anyhow::ensure!(
                policy.servers.contains(&origin),
                "Choose a source in this policy"
            );
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            let (root, sudo) = operations::identity(&remote)?;
            let execution = operations::execution(
                &operations::Plan {
                    label: "Read snapshot files".into(),
                    script: query.command(&policy, &origin)?,
                    administrator: policy.administrator,
                    is_root: root,
                    sudo_available: sudo,
                },
                !target.sudo.is_empty(),
            )?;
            let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
            let text = record_text_with_input(
                &remote,
                &execution.command,
                execution.use_stdin_password.then_some(input.as_bytes()),
                Duration::from_secs(60),
            )?;
            query.parse(&text, &policy, &origin)
        })()
        .map_err(|error| configuration_record_error(error, &target));
        let _ = events.send_blocking(Event::SnapshotFiles(serial, result));
    });
}
pub struct Controller {
    pub decision: mpsc::Sender<bool>,
    pub stop: Arc<AtomicBool>,
}
enum Plan {
    Drill {
        command: String,
        review: String,
        administrator: bool,
    },
    Installation {
        environment: packages::SystemEnvironment,
        definition: arcnix_core::installers::Definition,
        packages: packages::PackagePlan,
        before: Vec<arcnix_core::installers::Existing>,
        transaction: arcnix_core::installation_plan::Transaction,
    },
    Recovery(operations::Plan, String),
    Cleanup(operations::Plan, String),
    Configuration(operations::Plan, String, String),
    Shell(operations::Plan),
    Packages(packages::SystemEnvironment, packages::PackagePlan),
    Backup(operations::Plan, String),
}
struct Prepared {
    metadata: arcnix_core::task_records::Metadata,
    target: Target,
    remote: RemoteSession,
    plan: Plan,
    directory: String,
    uid: u32,
    root: bool,
    sudo_available: bool,
}
struct SubmitState {
    slots: usize,
    next: usize,
}
struct SubmitGate {
    slots: Mutex<SubmitState>,
    ready: Condvar,
}
struct Permit<'a>(&'a SubmitGate);
impl SubmitGate {
    fn acquire(&self, ticket: usize, stop: &AtomicBool) -> Option<Permit<'_>> {
        let mut slots = self.slots.lock().ok()?;
        while (slots.slots == 0 || slots.next != ticket) && !stop.load(Ordering::Acquire) {
            slots = self
                .ready
                .wait_timeout(slots, Duration::from_millis(100))
                .ok()?
                .0;
        }
        if stop.load(Ordering::Acquire) {
            None
        } else {
            slots.slots -= 1;
            slots.next += 1;
            self.ready.notify_all();
            Some(Permit(self))
        }
    }
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.0.slots.lock() {
            slots.slots += 1;
            self.0.ready.notify_all();
        }
    }
}
fn report(
    events: &async_channel::Sender<Event>,
    task: &str,
    id: &str,
    status: Status,
    preview: Option<String>,
    output: String,
    code: Option<i32>,
) {
    let _ = events.send_blocking(Event::Target {
        task: task.into(),
        id: id.into(),
        status,
        preview,
        output,
        code,
    });
}
fn prepare(target: Target, template: &Template, task: &str) -> Result<Prepared> {
    let resolved = template.for_target(&target.profile.id)?;
    let template = &resolved;
    let pending = PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?;
    let remote = pending.authenticate(&target.secret, &target.fingerprint)?;
    let home = arcnix_core::remote_tasks::preflight(&remote)?;
    let directory = arcnix_core::remote_tasks::directory(&home, task)?;
    arcnix_core::remote_tasks::operation_preflight(&remote)?;
    let (root, sudo_available) = operations::identity(&remote)?;
    anyhow::ensure!(
        root || sudo_available,
        "Server-owned jobs require root or sudo to launch systemd services"
    );
    let uid: u32 = remote.exec("id -u")?.stdout.trim().parse()?;
    let plan = if let Some(job) = &template.audit_cleanup {
        let plan = job.target(&target.profile, &target.fingerprint)?;
        anyhow::ensure!(
            root == (uid == 0),
            "Audit cleanup actor identity changed during preflight"
        );
        let actor = arcnix_core::record_maintenance::Account {
            home: home.clone(),
            uid,
        };
        let uploaded = record_upload(
            &remote,
            &plan.stage_command(&actor, task)?,
            &plan.stage_input()?,
        )?;
        anyhow::ensure!(uploaded.is_empty(), "Unexpected audit upload response");
        let execution =
            plan.prepare_execution(&actor, task, root, sudo_available, !target.sudo.is_empty())?;
        let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
        let response = record_text_with_input(
            &remote,
            &execution.command,
            execution.use_stdin_password.then_some(input.as_bytes()),
            Duration::from_secs(45),
        )?;
        let frozen = plan.parse_prepared(&response, task)?;
        let _ = remote.exec(&plan.discard_staging_command(&actor, task)?);
        Plan::Cleanup(
            operations::Plan {
                label: template.snippet.name.clone(),
                script: frozen.command()?,
                administrator: true,
                is_root: root,
                sudo_available,
            },
            plan.review(),
        )
    } else if let Some(job) = &template.recovery {
        let plan = job.target(&target.profile, &target.fingerprint)?;
        anyhow::ensure!(
            root == (uid == 0),
            "Recovery actor identity changed during preflight"
        );
        let actor = arcnix_core::record_maintenance::Account {
            home: home.clone(),
            uid,
        };
        let uploaded = record_upload(
            &remote,
            &plan.stage_command(&actor, task)?,
            &plan.stage_input()?,
        )?;
        anyhow::ensure!(uploaded.is_empty(), "Unexpected residual upload response");
        let execution =
            plan.prepare_execution(&actor, task, root, sudo_available, !target.sudo.is_empty())?;
        let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
        let response = record_text_with_input(
            &remote,
            &execution.command,
            execution.use_stdin_password.then_some(input.as_bytes()),
            Duration::from_secs(45),
        )?;
        let frozen = plan.parse_prepared(&response, task)?;
        let _ = remote.exec(&plan.discard_staging_command(&actor, task)?);
        Plan::Recovery(
            operations::Plan {
                label: template.snippet.name.clone(),
                script: frozen.command()?,
                administrator: true,
                is_root: root,
                sudo_available,
            },
            plan.review(task)?,
        )
    } else if let Some(job) = &template.cleanup {
        let plan = job.target(&target.profile, &target.fingerprint)?;
        anyhow::ensure!(
            plan.account().uid == uid && plan.account().home == home,
            "Cleanup account context changed; inspect storage again"
        );
        let staged = record_upload(&remote, &plan.stage_command(task)?, &plan.request()?)?;
        anyhow::ensure!(staged.is_empty(), "Unexpected cleanup upload response");
        let execution =
            plan.prepare_execution(task, root, sudo_available, !target.sudo.is_empty())?;
        let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
        let response = record_text_with_input(
            &remote,
            &execution.command,
            execution.use_stdin_password.then_some(input.as_bytes()),
            Duration::from_secs(45),
        )?;
        let frozen = plan.parse_prepared(&response, task)?;
        // The original account removes only its uploaded request. The frozen
        // root-owned plan remains available even if this best-effort tidy fails.
        let _ = remote.exec(&plan.discard_staging_command(task)?);
        Plan::Cleanup(
            operations::Plan {
                label: template.snippet.name.clone(),
                script: frozen.command()?,
                administrator: true,
                is_root: root,
                sudo_available,
            },
            plan.review(),
        )
    } else if let Some(job) = &template.configuration {
        anyhow::ensure!(
            remote.sftp_available(),
            "Configuration publication requires SFTP for private staging"
        );
        let staging = arcnix_core::configurations::Job::staging(&home, task)?;
        let parent = staging.rsplit_once('/').unwrap().0;
        let result = remote.exec(&format!(
            "# arcnix:configurations:stage\nset -eu\numask 077\nmkdir -p -- {}\nmkdir -- {}",
            arcnix_core::remote_tasks::quote(parent),
            arcnix_core::remote_tasks::quote(&staging)
        ))?;
        anyhow::ensure!(
            result.exit_code == 0,
            "Cannot create a private configuration staging directory"
        );
        remote.put_private_task_file(
            &staging,
            "runner.py",
            arcnix_core::configurations::runner().as_bytes(),
        )?;
        remote.put_private_task_file(&staging, "request.json", &job.request()?)?;
        if let Some(content) = &job.content {
            remote.put_private_task_file(&staging, "candidate", content.as_bytes())?;
        }
        let script = job.preview_command(&staging, task)?;
        let read = operations::Plan {
            label: "Prepare configuration publication".into(),
            script,
            administrator: true,
            is_root: root,
            sudo_available,
        };
        let execution = operations::execution(&read, !target.sudo.is_empty())?;
        let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
        let mut response = Vec::new();
        let mut oversized = false;
        let result = remote.exec_stream(
            &execution.command,
            execution.use_stdin_password.then_some(input.as_bytes()),
            Duration::from_secs(90),
            |stream, bytes| {
                if matches!(stream, arcnix_core::remote::CommandStream::Stdout) {
                    if response.len() + bytes.len() <= 1024 * 1024 {
                        response.extend_from_slice(bytes);
                    } else {
                        oversized = true;
                    }
                }
            },
        )?;
        anyhow::ensure!(
            result.exit_code == 0 && !oversized,
            "Configuration preparation failed; inspect its private server record"
        );
        let checked =
            arcnix_core::configurations::Preview::parse(&String::from_utf8(response)?, job, task)?;
        let command = job.execute_command(task)?;
        Plan::Configuration(
            operations::Plan {
                label: template.snippet.name.clone(),
                script: command,
                administrator: true,
                is_root: root,
                sudo_available,
            },
            format!("{}\n\n{}", checked.summary(), job.review()),
            checked.diff,
        )
    } else if let Some(job) = &template.drill {
        let preflight = job.preflight(&target.profile.id)?;
        let text = read_command(
            &remote,
            &preflight,
            job.backup.policy.administrator,
            &target.sudo,
        )?;
        job.validate_preflight(&text)?;
        anyhow::ensure!(
            template.revision == job.revision()?,
            "Recovery drill revision changed"
        );
        Plan::Drill {
            command: job.command(task, &target.profile.id)?,
            review: job.review()?,
            administrator: job.backup.policy.administrator,
        }
    } else if let Some(job) = &template.backup {
        let command = job.command(&target.profile.id)?;
        let preflight = job.preflight(&target.profile.id)?;
        let text = read_command(&remote, &preflight, job.policy.administrator, &target.sudo)?;
        anyhow::ensure!(
            text.len() <= 128 * 1024,
            "Backup preview is too large; narrow the policy before executing it"
        );
        job.validate_preflight(&text)?;
        let text = if let Some(review) = job.restore_review(&target.profile.id)? {
            review
        } else {
            text
        };
        let plan = operations::plan(
            operations::Intent::Command(operations::Snippet {
                parameters: vec![],
                command,
                ..template.snippet.clone()
            }),
            root,
            sudo_available,
        )?;
        Plan::Backup(plan, text)
    } else if let Some(service) = &template.service {
        operations::inspect(&remote, &service.unit)?;
        Plan::Shell(operations::plan(
            operations::Intent::Service {
                unit: service.unit.clone(),
                action: service.action,
            },
            root,
            sudo_available,
        )?)
    } else if let Some(action) = &template.package_action {
        let environment = packages::detect(&remote)?;
        if let Some(definition) = &template.installation {
            anyhow::ensure!(
                action == &packages::PackageAction::Install(definition.packages.clone())
                    && template.revision == definition.revision(),
                "Installation definition or revision changed"
            );
            packages::verify_available(&remote, &definition.packages)?;
            let text = read_command(
                &remote,
                &arcnix_core::installation_plan::Snapshot::command(definition, &environment)?,
                true,
                &target.sudo,
            )?;
            let snapshot = arcnix_core::installation_plan::Snapshot::parse(definition, &text)?;
            if let Some(command) = snapshot.transaction.version_check()? {
                let output = remote.exec(&command)?;
                anyhow::ensure!(
                    output.exit_code == 0 && output.stdout == "ARCNIX_APT_VERSIONS_OK\n",
                    "APT downgrade or invalid version comparison; check the plan again"
                );
            }
            let plan = packages::PackagePlan {
                action: action.clone(),
                command: arcnix_core::installers::apt_command(definition, &environment)?,
                preview: snapshot.simulation,
            };
            Plan::Installation {
                environment,
                definition: definition.clone(),
                packages: plan,
                before: snapshot.before,
                transaction: snapshot.transaction,
            }
        } else {
            let plan = packages::preview(&remote, &environment, action.clone())?;
            Plan::Packages(environment, plan)
        }
    } else {
        let (root, sudo) = operations::identity(&remote)?;
        Plan::Shell(operations::plan(
            operations::Intent::Command(template.snippet.clone()),
            root,
            sudo,
        )?)
    };
    let prepared = Prepared {
        metadata: arcnix_core::task_records::Metadata {
            diagnostic: None,
            stops: None,
            version: 1,
            id: task.into(),
            name: template.snippet.name.clone(),
            revision: template.revision.clone(),
            created: 0,
            drill: template
                .drill
                .as_ref()
                .map(|job| -> Result<_> {
                    Ok(Box::new(arcnix_core::recovery_drills::SavedRequest {
                        server: target.profile.id.clone(),
                        description: job.description()?,
                    }))
                })
                .transpose()?,
            kind: if template.drill.is_some() {
                arcnix_core::task_records::Kind::RecoveryDrill
            } else if template.recovery.is_some() {
                arcnix_core::task_records::Kind::Recovery
            } else if template.cleanup.is_some() || template.audit_cleanup.is_some() {
                arcnix_core::task_records::Kind::Cleanup
            } else if template.configuration.is_some() {
                arcnix_core::task_records::Kind::Configuration
            } else if template.backup.is_some() {
                arcnix_core::task_records::Kind::Backup
            } else if template.service.is_some() {
                arcnix_core::task_records::Kind::Service
            } else if template.package_action.is_some() {
                arcnix_core::task_records::Kind::Package
            } else {
                arcnix_core::task_records::Kind::Script
            },
        },
        target,
        remote,
        plan,
        directory,
        uid,
        root,
        sudo_available,
    };
    if matches!(
        prepared.plan,
        Plan::Installation { .. } | Plan::Drill { .. }
    ) {
        // Check the fully quoted hosted payload limit before offering Ready.
        execution(&prepared, task)?;
    }
    Ok(prepared)
}
fn payload(prepared: &Prepared) -> Result<String> {
    Ok(match &prepared.plan {
        Plan::Drill { command, .. } => command.clone(),
        Plan::Installation {
            environment,
            definition,
            transaction,
            ..
        } => arcnix_core::installers::command(
            definition,
            environment,
            &prepared.metadata.id,
            transaction,
        )?,
        Plan::Shell(plan)
        | Plan::Backup(plan, _)
        | Plan::Configuration(plan, ..)
        | Plan::Cleanup(plan, _)
        | Plan::Recovery(plan, _) => {
            let mut plan = plan.clone();
            plan.is_root = true;
            operations::execution(&plan, false)?.command
        }
        Plan::Packages(env, plan) => {
            let mut env = env.clone();
            env.is_root = true;
            let command = packages::build_execution(&env, plan, false)?.command;
            match &plan.action {
                packages::PackageAction::Install(names) => {
                    let names = names
                        .iter()
                        .map(|n| arcnix_core::remote_tasks::quote(n))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!(
                        "{command}\ncode=$?\nif test \"$code\" -ne 0; then exit \"$code\"; fi\nLC_ALL=C dpkg-query -W -f='${{binary:Package}}\\t${{Version}}\\t${{db:Status-Abbrev}}\\n' -- {names}"
                    )
                }
                _ => command,
            }
        }
    })
}
fn execution(prepared: &Prepared, task: &str) -> Result<packages::Execution> {
    let administrator = match &prepared.plan {
        Plan::Drill { administrator, .. } => *administrator,
        Plan::Shell(p)
        | Plan::Backup(p, _)
        | Plan::Configuration(p, ..)
        | Plan::Cleanup(p, _)
        | Plan::Recovery(p, _) => p.administrator,
        Plan::Packages(..) | Plan::Installation { .. } => true,
    };
    let script = arcnix_core::remote_tasks::launch(
        task,
        &prepared.directory,
        &payload(prepared)?,
        (!administrator).then_some(prepared.uid),
    )?;
    let plan = operations::Plan {
        label: "Launch server task".into(),
        script,
        administrator: true,
        is_root: prepared.root,
        sudo_available: prepared.sudo_available,
    };
    operations::hosted_execution(&plan, !prepared.target.sudo.is_empty())
}
fn preview(prepared: &Prepared) -> Result<String> {
    let command = payload(prepared)?;
    Ok(match &prepared.plan {
        Plan::Drill { review, .. } => review.clone(),
        Plan::Installation {
            environment,
            definition,
            packages,
            before,
            transaction,
        } => {
            let existing = before
                .iter()
                .map(|p| {
                    format!(
                        "{}: {} · {} · {} · {}",
                        p.name,
                        if p.installed() {
                            "Installed"
                        } else if p.status == "missing" {
                            "Not installed"
                        } else {
                            "Not fully installed"
                        },
                        p.version,
                        p.architecture,
                        p.status
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let checks = definition
                .keys()
                .iter()
                .map(|k| definition.label(k))
                .collect::<Vec<_>>()
                .join("\n");
            let policy = definition
                .policy
                .ok_or_else(|| anyhow::anyhow!("Installation policy is missing"))?;
            let review = format!(
                "{}\n{}\n{} {}\n{} {}\n{} {}",
                crate::i18n::translate(policy.label()),
                crate::i18n::translate(
                    "Installed state and actual dpkg actions must match this review. A changed plan requires new preflight."
                ),
                crate::i18n::translate("Installed-state SHA-256:"),
                transaction.status(),
                crate::i18n::translate("Transaction SHA-256:"),
                transaction.digest(),
                crate::i18n::translate("Reviewed actions:"),
                transaction.actions().len()
            );
            format!(
                "{} · {} {} · {}\nTemplate revision: {}\n\nExisting package state\n{existing}\n\n{review}\n\n{}\n\n{}\n\nRequired checks\n{checks}\n\n{}\n\nInstallation may leave partial changes on failure. No automatic rollback, removal or retry. The transaction fingerprint covers dpkg actions and versions, not package archive hashes or application behavior.",
                environment.distribution,
                environment.os_id,
                environment.os_version,
                environment.architecture,
                prepared.metadata.revision,
                packages.preview,
                packages.command,
                definition.scope()
            )
        }
        Plan::Packages(_, plan) => format!("{}\n\n{command}", plan.preview),
        Plan::Shell(plan) => plan.script.clone(),
        Plan::Backup(plan, checked) => {
            if plan.script.contains("# arcnix:backups:files\n") {
                checked.clone()
            } else {
                format!("{checked}\n\n{}", plan.script)
            }
        }
        Plan::Configuration(_, summary, _) => summary.clone(),
        Plan::Cleanup(_, summary) => summary.clone(),
        Plan::Recovery(_, summary) => summary.clone(),
    })
}
fn execute(
    prepared: Prepared,
    task: &str,
    events: &async_channel::Sender<Event>,
    stop: &AtomicBool,
    gate: &SubmitGate,
    stop_on_failure: bool,
    ticket: usize,
) -> bool {
    let id = &prepared.target.profile.id;
    let mut permit = gate.acquire(ticket, stop);
    if permit.is_none() {
        report(
            events,
            task,
            id,
            Status::Cancelled,
            None,
            String::new(),
            None,
        );
        return false;
    }
    let result = (|| -> Result<bool> {
        let execution = execution(&prepared, task)?;
        let metadata = &prepared.metadata;
        let initialized =
            prepared
                .remote
                .exec(&arcnix_core::task_records::initialize_metadata_command(
                    &prepared.directory,
                    metadata,
                )?)?;
        anyhow::ensure!(
            initialized.exit_code == 0,
            "Cannot initialize server task metadata"
        );
        let mut input = Zeroizing::new(Vec::new());
        if execution.use_stdin_password {
            input.extend_from_slice(prepared.target.sudo.as_bytes());
            input.push(b'\n');
        }
        report(
            events,
            task,
            id,
            Status::Submitting,
            None,
            String::new(),
            None,
        );
        let submitted = prepared.remote.exec_stream(
            &execution.command,
            execution.use_stdin_password.then_some(input.as_slice()),
            Duration::from_secs(30),
            |_, _| {},
        )?;
        anyhow::ensure!(
            submitted.exit_code == 0,
            "Server task submission returned exit {}",
            submitted.exit_code
        );
        // The server now owns the complete lifecycle. Free the submission slot
        // immediately; monitoring must never hold later targets on the client.
        report(events, task, id, Status::Running, None, String::new(), None);
        drop(permit.take());
        let mut previous = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(3600);
        loop {
            let mut snapshot = Snapshot::received(arcnix_core::remote_tasks::query(
                &prepared.remote,
                task,
                &prepared.directory,
            )?);
            snapshot.observation.output = arcnix_core::tasks::redact_log(
                &snapshot.observation.output,
                &[
                    prepared.target.secret.as_str(),
                    prepared.target.sudo.as_str(),
                ],
            );
            let pending = snapshot.observation.should_poll();
            let success = snapshot.observation.status() == Status::Succeeded;
            if previous.as_ref() != Some(&snapshot.observation) {
                previous = Some(snapshot.observation.clone());
                let _ = events.send_blocking(Event::Observed {
                    task: task.into(),
                    id: id.clone(),
                    snapshot,
                });
            }
            if !pending {
                return Ok(success);
            }
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "Progress viewing timed out; the server still owns the job"
            );
            std::thread::sleep(Duration::from_secs(1));
        }
    })();
    if !matches!(result, Ok(true)) && stop_on_failure {
        stop.store(true, Ordering::Release);
    }
    match result {
        Ok(success) => success,
        Err(error) => {
            let mut message =
                format!("\nResult unknown. Query the server task before retrying: {error:#}\n");
            for secret in [&prepared.target.secret, &prepared.target.sudo] {
                if !secret.is_empty() {
                    message = message.replace(secret.as_str(), "[redacted]");
                }
            }
            report(events, task, id, Status::Unknown, None, message, None);
            false
        }
    }
}
pub fn start(
    task: String,
    template: Template,
    targets: Vec<Target>,
    concurrency: usize,
    events: async_channel::Sender<Event>,
) -> Controller {
    let (decision, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    std::thread::spawn(move || {
        let stop_on_failure = template.configuration.is_some();
        let mut prepared = Vec::new();
        let mut targets = targets.into_iter();
        loop {
            let chunk: Vec<_> = targets.by_ref().take(concurrency.clamp(1, 8)).collect();
            if chunk.is_empty() {
                break;
            }
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .into_iter()
                    .map(|target| {
                        let task = &task;
                        let template = &template;
                        let events = &events;
                        let stop = &stopping;
                        scope.spawn(move || {
                            let id = target.profile.id.clone();
                            if stop.load(Ordering::Acquire) {
                                report(
                                    events,
                                    task,
                                    &id,
                                    Status::Cancelled,
                                    None,
                                    String::new(),
                                    None,
                                );
                                return None;
                            }
                            match prepare(target, template, task).and_then(|p| {
                                let _ = execution(&p, task)?;
                                let text = preview(&p)?;
                                Ok((p, text))
                            }) {
                                Ok((p, text)) => {
                                    if let Plan::Configuration(_, _, diff) = &p.plan {
                                        let _ = events.send_blocking(Event::ConfigurationPreview {
                                            task: task.into(),
                                            id: id.clone(),
                                            diff: diff.clone(),
                                        });
                                    }
                                    let _ = events.send_blocking(Event::Directory {
                                        task: task.into(),
                                        id: id.clone(),
                                        directory: p.directory.clone(),
                                    });
                                    report(
                                        events,
                                        task,
                                        &id,
                                        Status::Ready,
                                        Some(text),
                                        String::new(),
                                        None,
                                    );
                                    Some(p)
                                }
                                Err(e) => {
                                    report(
                                        events,
                                        task,
                                        &id,
                                        Status::Failed,
                                        None,
                                        format!("Preflight failed: {e:#}\n"),
                                        None,
                                    );
                                    None
                                }
                            }
                        })
                    })
                    .collect();
                for handle in handles {
                    if let Ok(Some(p)) = handle.join() {
                        prepared.push(p);
                    }
                }
            });
        }
        let _ = events.send_blocking(Event::Prepared(task.clone()));
        // Unconfirmed plans expire after ten minutes; the connection and credentials drop.
        let run =
            !prepared.is_empty() && rx.recv_timeout(Duration::from_secs(600)).unwrap_or(false);
        if run {
            let gate = SubmitGate {
                slots: Mutex::new(SubmitState {
                    slots: concurrency.clamp(1, 8),
                    next: 0,
                }),
                ready: Condvar::new(),
            };
            std::thread::scope(|scope| {
                for (ticket, p) in prepared.into_iter().enumerate() {
                    let events = &events;
                    let task = &task;
                    let stop = &stopping;
                    let gate = &gate;
                    scope.spawn(move || {
                        execute(p, task, events, stop, gate, stop_on_failure, ticket);
                    });
                }
            });
        } else {
            for p in prepared {
                report(
                    &events,
                    &task,
                    &p.target.profile.id,
                    Status::Cancelled,
                    None,
                    String::new(),
                    None,
                );
            }
        }
        let _ = events.send_blocking(Event::Complete(task));
    });
    Controller { decision, stop }
}
