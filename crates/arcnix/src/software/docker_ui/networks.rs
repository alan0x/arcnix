//! Docker networks, and a shell in a running container.
use arcnix_core::docker::Network;

use super::*;

#[derive(Clone, Default)]
pub(crate) struct NetworkState {
    list: Option<Vec<Network>>,
    pending: Option<u64>,
    error: String,
}

pub(super) fn count(s: &State) -> Option<usize> {
    s.docker.networks.list.as_ref().map(Vec::len)
}

/// The containers attached to `network`, from the container list.
fn members(s: &State, network: &str) -> Vec<String> {
    s.docker
        .snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .containers
                .iter()
                .filter(|c| c.networks.split(',').any(|n| n.trim() == network))
                .map(|c| c.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

impl Ui {
    pub(super) fn docker_networks_refresh(&self) {
        if !self.state.with(ready) {
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::DockerNetworks));
        self.state.revise(|s| {
            s.docker.networks.pending = Some(serial);
            s.docker.networks.error.clear();
        });
        // Attached containers come from the container list, read again so
        // it matches the networks.
        self.docker_send(AppAction::DockerSnapshot);
    }

    /// Opens Arcnix's terminal with the command for a shell in the selected
    /// container, ready for the user to run with Enter.
    pub(super) fn docker_shell(&self) {
        let command = self.state.with(|s| {
            let container = selected(s)?;
            arcnix_core::docker::shell_command(&container.id, s.docker.snapshot.as_ref()?.platform?)
                .ok()
        });
        if let Some(command) = command {
            self.terminal_with_command(command);
        }
    }
}

pub(super) fn can_shell(s: &State) -> bool {
    s.terminal_available()
        && s.docker
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.platform)
            .is_some_and(|os| os != arcnix_core::host::HostOs::Unknown)
        && selected(s).is_some_and(|c| c.state == "running")
}

/// Takes the network list and passes on other replies.
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    if !ui.state.with(|s| s.docker.networks.pending == Some(serial)) {
        return Some(result);
    }
    ui.state.revise(|s| {
        let n = &mut s.docker.networks;
        n.pending = None;
        match result {
            Ok(Reply::Networks(list)) => n.list = Some(list),
            Err(error) => n.error = error,
            _ => n.error = "Invalid Docker response".into(),
        }
    });
    None
}

pub(super) fn view(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        text()
            .class("docker-error error")
            .content(state.map(|s| s.docker.networks.error.clone())),
        text().class("subtle").content(i18n::bind(state.map(|s| {
            if s.docker.networks.list.is_none() && s.docker.networks.pending.is_some() {
                "Loading…"
            } else if s.docker.networks.list.as_ref().is_some_and(Vec::is_empty) {
                "Docker reported no networks."
            } else {
                ""
            }
            .to_owned()
        }))),
        col().class("docker-inventory docker-table").fill((
            column_titles(
                &["Network", "Driver", "Subnet", "Containers"],
                "docker-network-row docker-columns",
            ),
            each(state.map(|s| {
                let filter = s.docker.filter.to_lowercase();
                s.docker
                    .networks
                    .list
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|n| {
                        n.name.to_lowercase().contains(&filter)
                            || n.driver.to_lowercase().contains(&filter)
                    })
                    .collect::<Vec<_>>()
            }))
            .key(|n| n.id.clone())
            .children(move |item| {
                let network = item.value.get();
                let name = network.name.clone();
                row().class("docker-network-row").fill((
                    row().class("docker-image-name").fill((
                        text()
                            .class("docker-resource-name")
                            .selectable(true)
                            .content(network.name.clone()),
                        when(Vary::Value(network.built_in()), || {
                            text().class("docker-badge").content(i18n::bind("Built-in"))
                        }),
                        when(Vary::Value(network.internal), || {
                            text()
                                .class("docker-badge read-only")
                                .content(i18n::bind("Internal"))
                        }),
                    )),
                    text().content(network.driver.clone()),
                    text()
                        .class("docker-network-subnet")
                        .selectable(true)
                        .content(network.subnets.join(", ")),
                    text().class("subtle").content(state.map(move |s| {
                        let members = members(s, &name);
                        if members.is_empty() {
                            i18n::translate("None")
                        } else {
                            members.join(", ")
                        }
                    })),
                ))
            }),
        )),
    ))
}
