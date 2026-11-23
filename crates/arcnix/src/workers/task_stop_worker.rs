//! Separate finite control flow; never occupies the operation being stopped.
use arcnix_core::task_capabilities::HostedTaskStopInspector;
use arcnix_core::{record_maintenance as records, remote_tasks as jobs, task_stops as stops};

use super::*;

pub enum Update {
    Inspected(Result<Box<(stops::Snapshot, Target)>, String>),
    Launch {
        directory: String,
        reply: mpsc::Sender<bool>,
    },
    NotSubmitted {
        error: String,
        cancelled: bool,
    },
}
pub fn inspect_stops(
    serial: u64,
    target: Target,
    home: String,
    root: String,
    ids: Vec<String>,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            let account =
                records::parse_account(&record_text(&remote, records::account_command())?)?;
            anyhow::ensure!(
                account.home == home,
                "Server account changed; reconnect to server records"
            );
            let selection = stops::Selection {
                version: 1,
                account,
                root,
                ids,
            };
            Ok(Box::new((
                remote.inspect_task_stops(&selection, &target.sudo)?,
                target.clone(),
            )))
        })()
        .map_err(|error| configuration_record_error(error, &target));
        let _ = events.send_blocking(Event::TaskStop {
            serial,
            task: String::new(),
            server: target.profile.id.clone(),
            update: Update::Inspected(result),
        });
    });
}
pub fn execute_stops(
    serial: u64,
    task: String,
    targets: Vec<(Target, stops::Plan)>,
    cancel: Arc<AtomicBool>,
    events: async_channel::Sender<Event>,
) {
    // At most four SSH submissions at a time; the accepted jobs own completion.
    std::thread::spawn(move || {
        let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(targets)));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let queue = queue.clone();
                let cancel = cancel.clone();
                let events = events.clone();
                let task = task.clone();
                scope.spawn(move || {
                    loop {
                        let item = { queue.lock().unwrap().pop_front() };
                        let Some((target, plan)) = item else { break };
                        let id = target.profile.id.clone();
                        let mut attempted = false;
                        let result = (|| -> Result<()> {
                            anyhow::ensure!(
                                !cancel.load(Ordering::Acquire),
                                "Submission cancelled"
                            );
                            let remote = PendingConnection::open_for_known_host(
                                &target.profile,
                                &target.fingerprint,
                            )?
                            .authenticate(&target.secret, &target.fingerprint)?;
                            let account = records::parse_account(&record_text(
                                &remote,
                                records::account_command(),
                            )?)?;
                            anyhow::ensure!(
                                account == plan.selection.account,
                                "Server account changed; inspect tasks again"
                            );
                            remote.hosted_task_home()?;
                            let (root, sudo) = operations::identity(&remote)?;
                            let execution = plan.prepare_execution(
                                &account,
                                &task,
                                root,
                                sudo,
                                !target.sudo.is_empty(),
                            )?;
                            record_upload(
                                &remote,
                                &plan.stage_command(&account, &task)?,
                                &plan.stage_input()?,
                            )?;
                            let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
                            let response = record_text_with_input(
                                &remote,
                                &execution.command,
                                execution.use_stdin_password.then_some(input.as_bytes()),
                                Duration::from_secs(45),
                            );
                            let _ = remote.exec(&plan.discard_staging_command(&account, &task)?);
                            let prepared = plan.parse_prepared(&response?, &task)?;
                            anyhow::ensure!(
                                !cancel.load(Ordering::Acquire),
                                "Submission cancelled"
                            );
                            let directory = jobs::directory(&account.home, &task)?;
                            remote.initialize_task_record(
                                &directory,
                                &arcnix_core::task_records::Metadata {
                                    diagnostic: None,
                                    version: 1,
                                    id: task.clone(),
                                    name: "Stop server tasks".into(),
                                    revision: stops::revision(),
                                    kind: arcnix_core::task_records::Kind::TaskStop,
                                    created: 0,
                                    drill: None,
                                    stops: Some(plan.selection.description()?),
                                },
                            )?;
                            let script = jobs::launch_with_access(
                                &task,
                                &directory,
                                &stops::frozen_command(&prepared)?,
                                None,
                                jobs::Access::Control,
                            )?;
                            let execution = operations::hosted_execution(
                                &operations::Plan {
                                    label: "Stop server tasks".into(),
                                    script,
                                    administrator: true,
                                    is_root: root,
                                    sudo_available: sudo,
                                },
                                !target.sudo.is_empty(),
                            )?;
                            let (reply, ack) = mpsc::channel();
                            events.send_blocking(Event::TaskStop {
                                serial,
                                task: task.clone(),
                                server: id.clone(),
                                update: Update::Launch {
                                    directory: directory.clone(),
                                    reply,
                                },
                            })?;
                            anyhow::ensure!(
                                ack.recv_timeout(Duration::from_secs(30)).unwrap_or(false),
                                "Submission cancelled or task history could not be saved"
                            );
                            anyhow::ensure!(
                                !cancel.load(Ordering::Acquire),
                                "Submission cancelled"
                            );
                            attempted = true;
                            remote.submit_hosted_task(
                                &execution,
                                &target.sudo,
                                Duration::from_secs(45),
                            )?;
                            observe_stop(
                                remote,
                                target.clone(),
                                task.clone(),
                                directory,
                                events.clone(),
                            )?;
                            Ok(())
                        })();
                        if let Err(error) = result {
                            let status = if attempted {
                                Status::Unknown
                            } else if cancel.load(Ordering::Acquire) {
                                Status::Cancelled
                            } else {
                                Status::Failed
                            };
                            let output = configuration_record_error(error, &target);
                            if attempted {
                                let _ = events.send_blocking(Event::Target {
                                    task: task.clone(),
                                    id,
                                    status,
                                    preview: None,
                                    output,
                                    code: None,
                                });
                            } else {
                                let _ = events.send_blocking(Event::TaskStop {
                                    serial,
                                    task: task.clone(),
                                    server: id,
                                    update: Update::NotSubmitted {
                                        error: output,
                                        cancelled: status == Status::Cancelled,
                                    },
                                });
                            }
                        }
                    }
                });
            }
        });
    });
}

fn observe_stop(
    remote: RemoteSession,
    target: Target,
    task: String,
    directory: String,
    events: async_channel::Sender<Event>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("arcnix-stop-observer".into())
        .spawn(move || {
            let id = target.profile.id.clone();
            let result = (|| -> Result<()> {
                let deadline = std::time::Instant::now() + Duration::from_secs(210);
                let mut previous = None;
                loop {
                    let observation = remote.observe_control_task(&task, &directory)?;
                    let active = observation.should_poll();
                    if previous.as_ref() != Some(&observation) {
                        previous = Some(observation.clone());
                        let _ = events.send_blocking(Event::Observed {
                            task: task.clone(),
                            id: id.clone(),
                            snapshot: Snapshot::received(observation),
                        });
                    }
                    if !active {
                        return Ok(());
                    }
                    anyhow::ensure!(
                        std::time::Instant::now() < deadline,
                        "Server task continues; query its original result"
                    );
                    std::thread::sleep(Duration::from_millis(500));
                }
            })();
            if let Err(error) = result {
                let _ = events.send_blocking(Event::Target {
                    task,
                    id,
                    status: Status::Unknown,
                    preview: None,
                    output: configuration_record_error(error, &target),
                    code: None,
                });
            }
        })?;
    Ok(())
}
