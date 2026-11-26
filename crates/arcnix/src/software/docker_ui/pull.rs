//! Pulling an image by reference: a server task that runs `docker pull`, so
//! a long download keeps going when Arcnix closes. The Images list refreshes
//! when the task finishes.
use arcnix_core::docker_pull;

use super::*;

#[derive(Clone, Default)]
pub(crate) struct PullState {
    reference: String,
    platform: String,
    error: String,
}

/// The pull runs a POSIX shell script, which Windows servers don't have.
pub(super) fn offered(s: &State) -> bool {
    s.metrics.remote_platform != Some(arcnix_core::host::HostOs::Windows)
}

fn can_pull(s: &State) -> bool {
    s.connected && !scripts_ui::busy(s) && !s.docker.pull.reference.trim().is_empty()
}

impl Ui {
    pub(crate) fn docker_pull_open(&self) {
        if !self.state.with(ready) {
            return;
        }
        self.state.revise(|s| s.docker.pull.error.clear());
        self.open_dialog(Dialog::DockerPull);
    }

    fn docker_pull(&self) {
        if !self.state.with(can_pull) {
            return;
        }
        let template = self
            .state
            .with(|s| docker_pull::pull(&s.docker.pull.reference, &s.docker.pull.platform));
        match template {
            Ok(template) => self.choose_server_script(template),
            Err(error) => self
                .state
                .revise(|s| s.docker.pull.error = i18n::translate(&error.to_string())),
        }
    }

    /// After a pull task on this server: list the images again.
    pub(crate) fn docker_pull_finished(&self, task: &str) {
        let pulled = self.state.with(|s| {
            s.scripts.tasks.iter().any(|t| {
                t.id == task
                    && t.script_id == docker_pull::SCRIPT_ID
                    && t.targets
                        .iter()
                        .any(|target| s.selected.as_ref() == Some(&target.id))
            })
        });
        if pulled && self.state.with(ready) {
            self.docker_send(AppAction::DockerImages);
        }
    }
}

pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::DockerPull)),
        move || {
            let pull = ui.clone();
            modal(
            ui.clone(),
            "Pull image",
            col().class("docker-pull-form").fill((
                text().class("hint").content(i18n::bind(
                    "Enter an image the way docker pull takes it: nginx:1.27, or ghcr.io/owner/app:1.0 for another registry. Without a tag, Docker pulls latest. Images from Docker Hub go through the registry mirrors.",
                )),
                field(
                    "Image",
                    &ui,
                    |s| s.docker.pull.reference.clone(),
                    |s, v| {
                        s.docker.pull.reference = v;
                        s.docker.pull.error.clear();
                    },
                ),
                field(
                    "Platform (optional, such as linux/arm64)",
                    &ui,
                    |s| s.docker.pull.platform.clone(),
                    |s, v| {
                        s.docker.pull.platform = v;
                        s.docker.pull.error.clear();
                    },
                ),
                text().class("subtle").content(i18n::bind(
                    "Leave the platform empty to pull the server's own. The pull runs on the server as a task, so closing Arcnix doesn't stop it.",
                )),
                text()
                    .class("error")
                    .content(state.map(|s| s.docker.pull.error.clone())),
            )),
            sapid::button()
                .class("button primary docker-pull-start")
                .label(i18n::bind("Pull"))
                .enabled(state.map(can_pull))
                .on_press(move || pull.docker_pull()),
        )
        },
    )
}
