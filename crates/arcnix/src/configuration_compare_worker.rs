//! Independent finite reads; neither the workspace nor server jobs are changed.
use arcnix_core::configuration_compare::{self, Plan, Snapshot};

use super::*;
pub struct ComparedConfiguration {
    pub snapshot: Snapshot,
    pub received: u64,
    pub warning: String,
}
fn redact(value: &str, secrets: &[Zeroizing<String>]) -> String {
    let (mut text, _, shortened) = configuration_compare::redact_text(
        value,
        &secrets.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        4096,
    );
    if shortened {
        text.push_str("\n[response shortened]");
    }
    text
}

fn read(
    target: &Target,
    plan: &Plan,
    administrator: bool,
    secrets: &[Zeroizing<String>],
) -> Result<Box<ComparedConfiguration>> {
    let remote = PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
        .authenticate(&target.secret, &target.fingerprint)?;
    let (root, sudo) = if administrator {
        operations::identity(&remote)?
    } else {
        (false, false)
    };
    let execution = plan.execution(administrator, root, sudo, !target.sudo.is_empty())?;
    let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut oversized = false;
    let result = remote.exec_stream(
        &execution.command,
        execution.use_stdin_password.then_some(input.as_bytes()),
        Duration::from_secs(35),
        |stream, bytes| {
            let (buffer, limit) = if matches!(stream, arcnix_core::remote::CommandStream::Stdout) {
                (&mut stdout, configuration_compare::RESPONSE_LIMIT)
            } else {
                (&mut stderr, 64 * 1024)
            };
            if buffer.len() + bytes.len() <= limit {
                buffer.extend_from_slice(bytes);
            } else {
                oversized = true;
            }
        },
    )?;
    anyhow::ensure!(!oversized, "Configuration snapshot response is too large");
    anyhow::ensure!(
        result.exit_code == 0,
        "Cannot read configuration snapshot (exit {}): {}",
        result.exit_code,
        String::from_utf8_lossy(&stderr)
    );
    let snapshot = configuration_compare::parse(
        &String::from_utf8_lossy(&stdout),
        plan,
        &secrets.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
    )?;
    Ok(Box::new(ComparedConfiguration {
        snapshot,
        received: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
        warning: redact(String::from_utf8_lossy(&stderr).trim(), secrets),
    }))
}
pub fn compare_configurations(
    serial: u64,
    targets: Vec<Target>,
    plan: Plan,
    administrator: bool,
    cancel: Arc<AtomicBool>,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let secrets = Arc::new(
            targets
                .iter()
                .flat_map(|t| [t.secret.clone(), t.sudo.clone()])
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>(),
        );
        let count = targets.len().min(4);
        let jobs = Arc::new(Mutex::new(targets.into_iter()));
        let mut workers = Vec::new();
        for _ in 0..count {
            let jobs = jobs.clone();
            let plan = plan.clone();
            let cancel = cancel.clone();
            let events = events.clone();
            let secrets = secrets.clone();
            workers.push(std::thread::spawn(move || {
                loop {
                    if cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let target = jobs.lock().unwrap().next();
                    let Some(target) = target else {
                        break;
                    };
                    if cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let result = read(&target, &plan, administrator, &secrets)
                        .map_err(|e| redact(&format!("{e:#}"), &secrets));
                    let _ = events.send_blocking(Event::ConfigurationCompared {
                        serial,
                        id: target.profile.id.clone(),
                        result,
                    });
                }
            }));
        }
        for worker in workers {
            let _ = worker.join();
        }
        let _ = events.send_blocking(Event::ConfigurationComparisonComplete(serial));
    });
}
