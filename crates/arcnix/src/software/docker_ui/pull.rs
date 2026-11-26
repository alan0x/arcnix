//! Pulling an image by reference: a server task that runs `docker pull`, so
//! a long download keeps going when Arcnix closes. The Images list refreshes
//! when the task finishes. The dialog can also search Docker Hub and list
//! a repository's tags to choose from.
use arcnix_core::docker_pull;

use super::*;

mod browse;

#[derive(Clone, Default)]
pub(crate) struct PullState {
    reference: String,
    platform: String,
    error: String,
    /// The image list request sent after a pull, apart from the Docker
    /// app's own requests so neither replaces the other.
    refresh: Option<u64>,
    browse: browse::BrowseState,
}

/// Server tasks run on Linux with systemd. Until the server's system is
/// known, the button shows and the task's own check explains a refusal.
pub(super) fn offered(s: &State) -> bool {
    s.metrics
        .remote_platform
        .is_none_or(|p| p == arcnix_core::host::HostOs::Linux)
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
        if pulled
            && self
                .state
                .with(|s| s.connected && s.server_apps.applications.docker)
        {
            let serial = self.send(Action::ServerApps(AppAction::DockerImages));
            self.state.revise(|s| s.docker.pull.refresh = Some(serial));
        }
    }
}

/// Takes the image list sent after a pull and passes on other replies.
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    if !ui.state.with(|s| s.docker.pull.refresh == Some(serial)) {
        return Some(result);
    }
    ui.state.revise(|s| {
        s.docker.pull.refresh = None;
        // A failed refresh leaves the list as it was; Refresh shows the error.
        if let Ok(Reply::Images(images)) = result {
            s.docker.images = images;
        }
    });
    None
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
                browse::section(ui.clone()),
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
                row().class("docker-pull-actions").fill(
                    btn("Show tags", {
                        let ui = ui.clone();
                        move || ui.docker_typed_tags()
                    })
                    .class("button compact")
                    .enabled(state.map(|s| {
                        !s.docker.pull.browse.loading && !s.docker.pull.reference.trim().is_empty()
                    })),
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
                    "Leave the platform empty to pull the server's own. The pull runs on the server as a task: it starts with sudo like other tasks, runs as this SSH user, and keeps going when Arcnix closes. Other Arcnix tasks on this server can't start until it finishes.",
                )),
                text().class("hint").content(state.map(|s| {
                    if scripts_ui::busy(s) {
                        i18n::translate("Another server task is running. Pull when it finishes.")
                    } else {
                        String::new()
                    }
                })),
                text()
                    .class("error")
                    .content(state.map(|s| s.docker.pull.error.clone())),
            )),
            sapid::button()
                .class("button primary docker-pull-start")
                // Names the image, which may be scrolled out of view after
                // choosing a tag.
                .label(state.map(|s| {
                    let reference = s.docker.pull.reference.trim();
                    if reference.is_empty() {
                        i18n::translate("Pull")
                    } else if reference.chars().count() > 48 {
                        let short: String = reference.chars().take(47).collect();
                        format!("{} {short}…", i18n::translate("Pull"))
                    } else {
                        format!("{} {reference}", i18n::translate("Pull"))
                    }
                }))
                .enabled(state.map(can_pull))
                .on_press(move || pull.docker_pull()),
        )
        },
    )
}
