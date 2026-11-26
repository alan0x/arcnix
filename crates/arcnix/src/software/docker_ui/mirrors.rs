//! Registry mirrors: the addresses Docker tries for images from Docker Hub,
//! read from the daemon and published to its daemon.json as a reviewed task.
use arcnix_core::docker_daemon::{self, Daemon, MirrorTest};

use super::*;

#[derive(Clone, Default)]
pub(crate) struct MirrorState {
    daemon: Option<Daemon>,
    /// The list being edited, in the order Docker tries it.
    list: Vec<String>,
    input: String,
    error: String,
    pending: Option<u64>,
    /// Test requests and the address each one asks.
    tests: Vec<(u64, String)>,
    /// The last test result for each address: the text and its tone.
    results: Vec<(String, String, &'static str)>,
}

fn editable(s: &State) -> bool {
    s.docker
        .mirrors
        .daemon
        .as_ref()
        .is_some_and(|d| d.read_only().is_none())
}

fn can_publish(s: &State) -> bool {
    let m = &s.docker.mirrors;
    editable(s)
        && m.pending.is_none()
        && s.connected
        && !scripts_ui::busy(s)
        && m.daemon.as_ref().is_some_and(|d| d.changes(&m.list))
}

fn result_text(test: &MirrorTest) -> (String, &'static str) {
    match test {
        MirrorTest::Registry => (i18n::translate("Answers as a registry"), "running"),
        MirrorTest::Status(code) => (
            format!(
                "{} {code}",
                i18n::translate("Answered, but not as a registry: HTTP")
            ),
            "failed",
        ),
        MirrorTest::Unreachable(reason) => (
            format!("{}: {reason}", i18n::translate("No answer")),
            "failed",
        ),
        MirrorTest::NoTool => (
            i18n::translate("The server has neither curl nor wget to test with"),
            "stopped",
        ),
    }
}

impl Ui {
    pub(crate) fn docker_mirrors_open(&self) {
        if !self.state.with(ready) {
            return;
        }
        self.state
            .revise(|s| s.docker.mirrors = MirrorState::default());
        self.open_dialog(Dialog::DockerMirrors);
        self.docker_mirrors_read();
    }

    fn docker_mirrors_read(&self) {
        let serial = self.send(Action::ServerApps(AppAction::DockerDaemon));
        self.state.revise(|s| {
            s.docker.mirrors.pending = Some(serial);
            s.docker.mirrors.error.clear();
        });
    }

    fn docker_mirror_add(&self) {
        self.state.revise(|s| {
            let m = &mut s.docker.mirrors;
            let mut list = m.list.clone();
            list.push(m.input.trim().to_owned());
            match docker_daemon::checked_mirrors(&list) {
                Ok(list) => {
                    m.list = list;
                    m.input.clear();
                    m.error.clear();
                }
                Err(error) => m.error = i18n::translate(&error.to_string()),
            }
        });
    }

    fn docker_mirror_test(&self, mirror: String) {
        if !self.state.with(ready) {
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::DockerMirrorTest(
            mirror.clone(),
        )));
        self.state.revise(|s| {
            let m = &mut s.docker.mirrors;
            m.results.retain(|(address, ..)| *address != mirror);
            m.results
                .push((mirror.clone(), i18n::translate("Testing…"), "stopped"));
            m.tests.push((serial, mirror));
        });
    }

    fn docker_mirrors_publish(&self) {
        if !self.state.with(can_publish) {
            return;
        }
        let template = self.state.with(|s| {
            let m = &s.docker.mirrors;
            docker_daemon::publish(m.daemon.as_ref().expect("checked"), &m.list)
        });
        match template {
            Ok(template) => self.choose_server_script(template),
            Err(error) => self
                .state
                .revise(|s| s.docker.mirrors.error = i18n::translate(&format!("{error:#}"))),
        }
    }

    /// After a configuration task: read the daemon again when the task
    /// published its daemon.json, so the dialog shows what Docker uses now.
    pub(crate) fn docker_task_finished(&self, task: &str) {
        let ours = self.state.with(|s| {
            let Some(Daemon {
                setup: docker_daemon::Setup::Service { config, .. },
                ..
            }) = &s.docker.mirrors.daemon
            else {
                return false;
            };
            s.scripts.tasks.iter().any(|t| {
                t.id == task
                    && t.configuration
                        .as_ref()
                        .is_some_and(|r| &r.spec.path == config)
            })
        });
        if ours && self.state.with(ready) {
            self.docker_mirrors_read();
        }
    }
}

/// Handles replies for the mirrors dialog and passes on the rest.
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    let (read, test) = ui.state.with(|s| {
        let m = &s.docker.mirrors;
        (
            m.pending == Some(serial),
            m.tests
                .iter()
                .find(|(id, _)| *id == serial)
                .map(|(_, mirror)| mirror.clone()),
        )
    });
    if read {
        ui.state.revise(|s| {
            let m = &mut s.docker.mirrors;
            m.pending = None;
            match result {
                Ok(Reply::DockerDaemon(daemon)) => {
                    m.list = daemon.configured();
                    m.daemon = Some(daemon);
                }
                Err(error) => m.error = error,
                _ => m.error = i18n::translate("Invalid Docker response"),
            }
        });
        return None;
    }
    let Some(mirror) = test else {
        return Some(result);
    };
    ui.state.revise(|s| {
        let m = &mut s.docker.mirrors;
        m.tests.retain(|(id, _)| *id != serial);
        let (text, tone) = match result {
            Ok(Reply::MirrorTest(test)) => result_text(&test),
            Err(error) => (error, "failed"),
            _ => (i18n::translate("Invalid Docker response"), "failed"),
        };
        m.results.retain(|(address, ..)| *address != mirror);
        m.results.push((mirror, text, tone));
    });
    None
}

pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::DockerMirrors)),
        move || {
            let publish = ui.clone();
            let add = ui.clone();
            let rows = ui.clone();
            modal(
            ui.clone(),
            "Registry mirrors",
            col().class("docker-mirrors").fill((
                text().class("hint").content(i18n::bind(
                    "Docker tries these addresses in order for images from Docker Hub. Images from other registries, such as ghcr.io, don't use them. When no mirror answers, Docker pulls from Docker Hub.",
                )),
                text()
                    .class(state.map(|s| {
                        if s.docker.mirrors.pending.is_some() { "subtle" } else { "hidden" }.to_owned()
                    }))
                    .content(i18n::bind("Reading Docker's settings…")),
                text()
                    .class("hint docker-mirrors-note")
                    .selectable(true)
                    .content(state.map(|s| {
                        s.docker
                            .mirrors
                            .daemon
                            .as_ref()
                            .and_then(Daemon::read_only)
                            .map(|why| i18n::translate(&why))
                            .unwrap_or_default()
                    })),
                // What the daemon uses when it differs from the file, for
                // example after the key was removed without a restart.
                text().class("subtle").content(state.map(|s| {
                    let Some(daemon) = &s.docker.mirrors.daemon else {
                        return String::new();
                    };
                    if docker_daemon::reported(&daemon.configured()) == daemon.mirrors {
                        return String::new();
                    }
                    let used = if daemon.mirrors.is_empty() {
                        i18n::translate("none")
                    } else {
                        daemon.mirrors.join(", ")
                    };
                    format!("{} {used}", i18n::translate("Docker uses now:"))
                })),
                each(state.map(|s| {
                    s.docker.mirrors.list.iter().cloned().enumerate().collect::<Vec<_>>()
                }))
                .key(|(index, mirror)| (*index, mirror.clone()))
                .children(move |item| {
                    let (index, mirror) = item.value.get();
                    let ui = rows.clone();
                    let test = mirror.clone();
                    let shown = mirror.clone();
                    row().class("docker-mirror-row").fill((
                        col().class("docker-mirror-address").fill((
                            text().selectable(true).content(mirror.clone()),
                            text()
                                .class(state.map({
                                    let mirror = mirror.clone();
                                    move |s| {
                                        let tone = s.docker.mirrors.results.iter().find(|(a, ..)| *a == mirror).map_or("stopped", |r| r.2);
                                        format!("docker-mirror-result tone-{tone}")
                                    }
                                }))
                                .content(state.map(move |s| {
                                    s.docker.mirrors.results.iter().find(|(a, ..)| *a == shown).map(|r| r.1.clone()).unwrap_or_default()
                                })),
                        )),
                        btn("Test", {
                            let ui = ui.clone();
                            move || ui.docker_mirror_test(test.clone())
                        })
                        .class("button compact")
                        .enabled(state.map(ready)),
                        when(state.map(editable), move || {
                            let ui = ui.clone();
                            icon_action("Remove mirror", Icon::Close)
                                .class("icon-button")
                                .on_press(move || {
                                    ui.state.revise(|s| {
                                        if index < s.docker.mirrors.list.len() {
                                            s.docker.mirrors.list.remove(index);
                                        }
                                    })
                                })
                        }),
                    ))
                }),
                text().class("subtle").content(state.map(|s| {
                    let m = &s.docker.mirrors;
                    if m.daemon.is_some() && m.list.is_empty() {
                        i18n::translate("No mirrors. Docker pulls from Docker Hub directly.")
                    } else {
                        String::new()
                    }
                })),
                when(state.map(editable), move || {
                    let add = add.clone();
                    let state = add.state;
                    col().class("field").fill((
                    text().class("field-label").content(i18n::bind("Mirror address, such as https://mirror.example.com")),
                    row().class("docker-mirror-add").fill((
                        input()
                            .name(i18n::bind("Mirror address"))
                            .value(state.map(|s| s.docker.mirrors.input.clone()))
                            .on(value_changed, move |event| {
                                if let Event::ValueChanged { value, .. } = event {
                                    state.revise(|s| {
                                        s.docker.mirrors.input = value;
                                        s.docker.mirrors.error.clear();
                                    });
                                }
                            }),
                        btn("Add", move || add.docker_mirror_add())
                            .class("button")
                            .enabled(state.map(|s| !s.docker.mirrors.input.trim().is_empty())),
                    )),
                    ))
                }),
                text()
                    .class("error")
                    .content(state.map(|s| s.docker.mirrors.error.clone())),
            )),
            when(state.map(editable), move || {
                let publish = publish.clone();
                sapid::button()
                    .class("button primary docker-mirrors-publish")
                    .label(i18n::bind("Review and publish…"))
                    .enabled(state.map(can_publish))
                    .on_press(move || publish.docker_mirrors_publish())
            }),
        )
        },
    )
}
