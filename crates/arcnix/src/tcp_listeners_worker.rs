//! A cancellable, independently authenticated metadata query.
use arcnix_core::tcp_listeners;

use super::*;
pub struct TcpListenerObservation {
    pub snapshot: tcp_listeners::Snapshot,
    pub received: Option<u64>,
}
pub fn read_tcp_listeners(
    serial: u64,
    target: Target,
    administrator: bool,
    ports: Vec<u16>,
    cancel: Arc<AtomicBool>,
    events: async_channel::Sender<Event>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<_> {
            anyhow::ensure!(!cancel.load(Ordering::Acquire), "Query cancelled");
            let remote =
                PendingConnection::open_for_known_host(&target.profile, &target.fingerprint)?
                    .authenticate(&target.secret, &target.fingerprint)?;
            anyhow::ensure!(!cancel.load(Ordering::Acquire), "Query cancelled");
            let (root, sudo) = if administrator {
                operations::identity(&remote)?
            } else {
                (false, false)
            };
            let execution = tcp_listeners::execution(
                &ports,
                administrator,
                root,
                sudo,
                !target.sudo.is_empty(),
            )?;
            let password = Zeroizing::new(format!("{}\n", target.sudo.as_str()));
            let snapshot = tcp_listeners::read(
                &remote,
                &execution,
                &ports,
                execution.use_stdin_password.then_some(password.as_bytes()),
                &cancel,
                &[target.secret.as_str(), target.sudo.as_str()],
            )?;
            Ok(Box::new(TcpListenerObservation {
                snapshot,
                received: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|v| v.as_secs()),
            }))
        })()
        .map_err(|e| {
            arcnix_core::configuration_compare::redact_text(
                &format!("{e:#}"),
                &[target.secret.as_str(), target.sudo.as_str()],
                4096,
            )
            .0
        });
        let _ = events.send_blocking(Event::TcpListeners { serial, result });
    });
}
