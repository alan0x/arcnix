//! One bounded, independently authenticated journal read; no workspace session.
use arcnix_core::diagnostic_logs;
use arcnix_core::system_logs::{self, LogQuery, LogSnapshot};

use super::*;
pub struct LogObservation {
    pub snapshot: LogSnapshot,
    pub received: Option<u64>,
}
fn redact(value: &str, target: &Target) -> String {
    let mut result = value.to_owned();
    for secret in [target.secret.as_str(), target.sudo.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
    {
        result = result.replace(secret, "[redacted]");
    }
    result
}
pub fn read_diagnostic_logs(
    serial: u64,
    target: Target,
    query: LogQuery,
    administrator: bool,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            let (root, sudo) = if administrator {
                operations::identity(&remote)?
            } else {
                (false, false)
            };
            let execution = diagnostic_logs::execution(
                &query,
                administrator,
                root,
                sudo,
                !target.sudo.is_empty(),
            )?;
            let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
            let mut output = Vec::new();
            let mut error = Vec::new();
            let mut oversized = false;
            let result = remote.exec_stream(
                &execution.command,
                execution.use_stdin_password.then_some(input.as_bytes()),
                Duration::from_secs(35),
                |stream, bytes| {
                    let (buffer, limit) =
                        if matches!(stream, arcnix_core::remote::CommandStream::Stdout) {
                            (&mut output, 2 * 1024 * 1024)
                        } else {
                            (&mut error, 64 * 1024)
                        };
                    if buffer.len() + bytes.len() <= limit {
                        buffer.extend_from_slice(bytes);
                    } else {
                        oversized = true;
                    }
                },
            )?;
            anyhow::ensure!(!oversized, "Service log response exceeded the read limit");
            anyhow::ensure!(
                result.exit_code == 0,
                "Cannot read service logs (exit {}): {}",
                result.exit_code,
                String::from_utf8_lossy(&error)
            );
            let mut snapshot =
                system_logs::parse_snapshot(&query, &String::from_utf8_lossy(&output))?;
            if !error.is_empty() {
                snapshot
                    .warnings
                    .push(String::from_utf8_lossy(&error).trim().into());
            }
            for row in &mut snapshot.entries {
                row.message = redact(&row.message, &target);
                if let Some(unit) = &mut row.unit {
                    *unit = redact(unit, &target);
                }
            }
            for warning in &mut snapshot.warnings {
                *warning = redact(warning, &target);
            }
            Ok(Box::new(LogObservation {
                snapshot,
                received: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|v| v.as_secs()),
            }))
        })()
        .map_err(|e| redact(&format!("{e:#}"), &target));
        let _ = events.send_blocking(Event::DiagnosticLogs { serial, result });
    });
}
