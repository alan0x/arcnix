//! Service queries and reviewed native changes on the workspace SSH worker.
use anyhow::Result;
use arcnix_core::host::HostOs;
use arcnix_core::host_capabilities::{NativeServiceController, NativeServicePlan, ServiceReader};
use arcnix_core::operations::{Service, ServiceAction};
use arcnix_core::remote::RemoteSession;

pub enum Action {
    List,
    Inspect(String),
    Review(String, ServiceAction),
    Apply(NativeServicePlan),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    List,
    Inspect,
    Review,
    Apply,
}
impl Action {
    pub fn kind(&self) -> Kind {
        match self {
            Self::List => Kind::List,
            Self::Inspect(_) => Kind::Inspect,
            Self::Review(..) => Kind::Review,
            Self::Apply(_) => Kind::Apply,
        }
    }
}
pub enum Event {
    Services(HostOs, Vec<Service>),
    Details(String),
    Review(NativeServicePlan),
    Applied,
    Failed(Kind, String),
}
pub fn handle(remote: &RemoteSession, action: Action, mut send: impl FnMut(Event)) -> Result<()> {
    match action {
        Action::List => send(Event::Services(remote.host_os()?, remote.list_services()?)),
        Action::Inspect(unit) => send(Event::Details(remote.inspect_service(&unit)?)),
        Action::Review(unit, action) => {
            send(Event::Review(remote.review_native_service(&unit, action)?))
        }
        Action::Apply(plan) => {
            remote.apply_native_service(&plan)?;
            send(Event::Applied);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use arcnix_core::ServerProfile;
    use arcnix_core::host_capabilities::ServiceChangePlanner;
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
            if let Event::Services(HostOs::Linux, items) = event {
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
        let plan = remote.plan_service_change("nginx.service", ServiceAction::Stop, false, true)?;
        assert!(plan.administrator);
        assert!(plan.script.contains("stop -- 'nginx.service'"));
        assert!(
            remote
                .plan_service_change("nginx.service", ServiceAction::Stop, false, false)
                .is_err()
        );
        assert!(
            remote
                .inspect_service("nginx.service")?
                .contains("ActiveState=active")
        );
        Ok(())
    }
}
