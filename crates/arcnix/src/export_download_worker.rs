//! One finite read per user action; credentials and binary data stay off history.
use std::path::PathBuf;

use arcnix_core::export_download as exports;

use super::*;
pub enum Update {
    Inspected(Result<Box<exports::Snapshot>, String>),
    Progress(u64),
    Finished(Result<(PathBuf, u64), String>),
}
pub fn inspect_export(
    serial: u64,
    target: Target,
    id: String,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            let (root, sudo) = operations::identity(&remote)?;
            let execution = exports::inspect_execution(&id, root, sudo, !target.sudo.is_empty())?;
            let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
            let text = record_text_with_input(
                &remote,
                &execution.command,
                execution.use_stdin_password.then_some(input.as_bytes()),
                Duration::from_secs(45),
            )?;
            Ok(Box::new(exports::parse_snapshot(&text, &id)?))
        })()
        .map_err(|error| configuration_record_error(error, &target));
        let _ = events.send_blocking(Event::ExportDownload {
            serial,
            server: target.profile.id.clone(),
            update: Update::Inspected(result),
        });
    });
}
pub fn download_export(
    serial: u64,
    target: Target,
    request: exports::Request,
    destination: PathBuf,
    cancel: Arc<AtomicBool>,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            let (root, sudo) = operations::identity(&remote)?;
            let execution =
                exports::stream_execution(&request, root, sudo, !target.sudo.is_empty())?;
            let input = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
            let mut last = std::time::Instant::now() - Duration::from_secs(1);
            exports::download(
                &remote,
                &request,
                &execution,
                execution.use_stdin_password.then_some(input.as_bytes()),
                &destination,
                &cancel,
                |bytes| {
                    if last.elapsed() >= Duration::from_millis(200) {
                        last = std::time::Instant::now();
                        let _ = events.send_blocking(Event::ExportDownload {
                            serial,
                            server: target.profile.id.clone(),
                            update: Update::Progress(bytes),
                        });
                    }
                },
            )
        })()
        .map_err(|error| configuration_record_error(error, &target));
        let _ = events.send_blocking(Event::ExportDownload {
            serial,
            server: target.profile.id.clone(),
            update: Update::Finished(result),
        });
    });
}
