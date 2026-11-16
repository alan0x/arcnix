//! Read-only operations on the workspace SSH worker. Mutations use server jobs.
use anyhow::Result;
use arcnix_core::operations::{self, Service};
use arcnix_core::remote::RemoteSession;

pub enum Action {
    List,
    Inspect(String),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    List,
    Inspect,
}
impl Action {
    pub fn kind(&self) -> Kind {
        match self {
            Self::List => Kind::List,
            Self::Inspect(_) => Kind::Inspect,
        }
    }
}
pub enum Event {
    Services(Vec<Service>),
    Details(String),
    Failed(Kind, String),
}
pub fn handle(remote: &RemoteSession, action: Action, mut send: impl FnMut(Event)) -> Result<()> {
    match action {
        Action::List => send(Event::Services(operations::list(remote)?)),
        Action::Inspect(unit) => send(Event::Details(operations::inspect(remote, &unit)?)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use arcnix_core::ServerProfile;
    use arcnix_core::remote::PendingConnection;

    use super::*;
    #[test]
    #[ignore = "requires loopback SSH fixture"]
    fn workspace_worker_lists_and_inspects_services_without_a_mutation_channel() -> Result<()> {
        let address: std::net::SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
        anyhow::ensure!(address.ip().is_loopback(), "loopback only");
        let pending = PendingConnection::open(&ServerProfile {
            host: address.ip().to_string(),
            port: address.port(),
            username: "arcnix".into(),
            ..Default::default()
        })?;
        let fingerprint = pending.fingerprint().to_owned();
        let remote = pending.authenticate("arcnix-test", &fingerprint)?;
        let mut listed = false;
        handle(&remote, Action::List, |event| {
            if let Event::Services(items) = event {
                listed = items.iter().any(|s| s.unit == "nginx.service");
            }
        })?;
        assert!(listed);
        let mut details = String::new();
        handle(&remote, Action::Inspect("nginx.service".into()), |event| {
            if let Event::Details(value) = event {
                details = value;
            }
        })?;
        assert!(details.contains("ActiveState=active"));
        assert!(handle(&remote, Action::Inspect("nginx.service;id".into()), |_| {}).is_err());
        Ok(())
    }
}
