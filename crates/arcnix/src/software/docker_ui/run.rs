//! Running a container from an image: a Compose project in
//! `~/.arcnix/apps/<name>`, written and started by the same server task as
//! App Center's Compose installs, and managed afterwards in Projects.
use arcnix_core::container_app::{self, ContainerApp, Port, Protocol, Restart, Volume};

use super::*;

#[derive(Clone, Default)]
struct PortRow {
    address: String,
    host: String,
    container: String,
    udp: bool,
}

#[derive(Clone, Default)]
struct VolumeRow {
    source: String,
    target: String,
    read_only: bool,
}

#[derive(Clone, Default)]
pub(crate) struct RunState {
    image: String,
    name: String,
    ports: Vec<PortRow>,
    volumes: Vec<VolumeRow>,
    environment: Vec<(String, String)>,
    restart: Restart,
    preview: bool,
    error: String,
    /// The request for the image's own ports and volumes.
    config: Option<u64>,
}

impl RunState {
    /// The project these rows describe, or what to fix first.
    fn app(&self) -> anyhow::Result<ContainerApp> {
        let port = |text: &str, what: &str| -> anyhow::Result<u16> {
            text.trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or_else(|| anyhow::anyhow!("Enter a {what} port from 1 to 65535"))
        };
        let app = ContainerApp {
            name: self.name.trim().to_owned(),
            image: self.image.trim().to_owned(),
            ports: self
                .ports
                .iter()
                .map(|row| {
                    Ok(Port {
                        address: row.address.trim().to_owned(),
                        host: port(&row.host, "server")?,
                        container: port(&row.container, "container")?,
                        protocol: if row.udp {
                            Protocol::Udp
                        } else {
                            Protocol::Tcp
                        },
                    })
                })
                .collect::<anyhow::Result<_>>()?,
            volumes: self
                .volumes
                .iter()
                .map(|row| Volume {
                    source: row.source.trim().to_owned(),
                    target: row.target.trim().to_owned(),
                    read_only: row.read_only,
                })
                .collect(),
            environment: self
                .environment
                .iter()
                .map(|(k, v)| (k.trim().to_owned(), v.clone()))
                .collect(),
            restart: self.restart,
            ..Default::default()
        };
        app.validate()?;
        Ok(app)
    }
}

fn can_run(s: &State) -> bool {
    s.connected && !scripts_ui::busy(s) && s.server_apps.applications.compose
}

impl Ui {
    /// Opens the dialog for `image` and asks Docker what the image exposes.
    pub(crate) fn docker_run_open(&self, image: String) {
        if !self.state.with(ready) {
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::DockerImageConfig(
            image.clone(),
        )));
        self.state.revise(|s| {
            s.docker.run = RunState {
                name: container_app::suggested_name(&image),
                image,
                config: Some(serial),
                ..Default::default()
            };
        });
        self.open_dialog(Dialog::DockerRun);
    }

    fn docker_run(&self) {
        if !self.state.with(can_run) {
            return;
        }
        match self
            .state
            .with(|s| s.docker.run.app().and_then(|app| app.install()))
        {
            Ok(template) => self.choose_server_script(template),
            Err(error) => self
                .state
                .revise(|s| s.docker.run.error = i18n::translate(&error.to_string())),
        }
    }

    /// After a Run task on this server: show the new project in Projects.
    pub(crate) fn docker_run_finished(&self, task: &str) {
        // The task is named "Run <image> as <project>".
        let started = self.state.with(|s| {
            s.scripts.tasks.iter().find_map(|t| {
                let succeeded = t.targets.iter().any(|target| {
                    s.selected.as_ref() == Some(&target.id)
                        && target.status == arcnix_core::tasks::Status::Succeeded
                });
                (t.id == task && t.script_id == "builtin:compose-application" && succeeded)
                    .then(|| t.name.strip_prefix("Run ")?.rsplit_once(" as "))
                    .flatten()
                    .map(|(_, name)| name.to_owned())
            })
        });
        let Some(name) = started else {
            return;
        };
        self.state.revise(|s| s.docker.detail_open = false);
        self.open_compose_project(name);
    }
}

/// Takes the image's ports and volumes and passes on other replies.
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    if !ui.state.with(|s| s.docker.run.config == Some(serial)) {
        return Some(result);
    }
    ui.state.revise(|s| {
        let run = &mut s.docker.run;
        run.config = None;
        // Without the image's settings the dialog still works; the user adds
        // ports and volumes by hand.
        if let Ok(Reply::ImageConfig(config)) = result {
            if run.ports.is_empty() {
                run.ports = config
                    .ports
                    .iter()
                    .map(|(port, protocol)| PortRow {
                        address: "127.0.0.1".into(),
                        host: port.to_string(),
                        container: port.to_string(),
                        udp: *protocol == Protocol::Udp,
                    })
                    .collect();
            }
            if run.volumes.is_empty() {
                run.volumes = config
                    .volumes
                    .iter()
                    .enumerate()
                    .map(|(i, path)| VolumeRow {
                        source: if config.volumes.len() == 1 {
                            "data".into()
                        } else {
                            format!("data-{}", i + 1)
                        },
                        target: path.clone(),
                        read_only: false,
                    })
                    .collect();
            }
        }
    });
    None
}

/// A text input bound to one field of one row.
fn cell(
    state: Cage<State>,
    label: &'static str,
    class: &'static str,
    read: impl Fn(&State) -> String + 'static,
    write: impl Fn(&mut State, String) + 'static,
) -> impl Widget {
    input()
        .class(class)
        .name(i18n::bind(label))
        .value(state.map(read))
        .on(value_changed, move |event| {
            if let Event::ValueChanged { value, .. } = event {
                state.revise(|s| {
                    write(s, value);
                    s.docker.run.error.clear();
                });
            }
        })
}

fn section_heading(label: &'static str, add: impl FnMut() + 'static) -> impl Widget {
    row().class("docker-run-heading").fill((
        text().class("field-label").content(i18n::bind(label)),
        btn("Add", add).class("button compact"),
    ))
}

fn ports(state: Cage<State>) -> impl Widget {
    col().class("docker-run-section").fill((
        section_heading("Ports", move || {
            state.revise(|s| {
                s.docker.run.ports.push(PortRow {
                    address: "127.0.0.1".into(),
                    ..Default::default()
                })
            })
        }),
        text().class("subtle").content(i18n::bind(
            "Server address 127.0.0.1 keeps a port on this server; 0.0.0.0 opens it to every network.",
        )),
        each(state.map(|s| (0..s.docker.run.ports.len()).collect::<Vec<_>>()))
            .key(|i| *i)
            .children(move |item| {
                let i = item.value.get();
                row().class("docker-run-row").fill((
                    cell(state, "Server address", "docker-run-address",
                        move |s| s.docker.run.ports.get(i).map(|r| r.address.clone()).unwrap_or_default(),
                        move |s, v| if let Some(r) = s.docker.run.ports.get_mut(i) { r.address = v }),
                    cell(state, "Server port", "docker-run-port",
                        move |s| s.docker.run.ports.get(i).map(|r| r.host.clone()).unwrap_or_default(),
                        move |s, v| if let Some(r) = s.docker.run.ports.get_mut(i) { r.host = v }),
                    text().class("subtle").content("→"),
                    cell(state, "Container port", "docker-run-port",
                        move |s| s.docker.run.ports.get(i).map(|r| r.container.clone()).unwrap_or_default(),
                        move |s, v| if let Some(r) = s.docker.run.ports.get_mut(i) { r.container = v }),
                    sapid::button()
                        .class("button compact docker-run-toggle")
                        .label("UDP")
                        .pressed(state.map(move |s| s.docker.run.ports.get(i).is_some_and(|r| r.udp)))
                        .on_press(move || state.revise(|s| {
                            if let Some(r) = s.docker.run.ports.get_mut(i) { r.udp = !r.udp }
                        })),
                    icon_action("Remove port", Icon::Close)
                        .class("icon-button")
                        .on_press(move || state.revise(|s| {
                            if i < s.docker.run.ports.len() { s.docker.run.ports.remove(i); }
                        })),
                ))
            }),
    ))
}

fn volumes(state: Cage<State>) -> impl Widget {
    col().class("docker-run-section").fill((
        section_heading("Volumes", move || {
            state.revise(|s| s.docker.run.volumes.push(VolumeRow::default()))
        }),
        text().class("subtle").content(i18n::bind(
            "A name such as data keeps the files in a Docker volume; a path such as /srv/app uses that folder on the server.",
        )),
        each(state.map(|s| (0..s.docker.run.volumes.len()).collect::<Vec<_>>()))
            .key(|i| *i)
            .children(move |item| {
                let i = item.value.get();
                row().class("docker-run-row").fill((
                    cell(state, "Volume name or server folder", "docker-run-wide",
                        move |s| s.docker.run.volumes.get(i).map(|r| r.source.clone()).unwrap_or_default(),
                        move |s, v| if let Some(r) = s.docker.run.volumes.get_mut(i) { r.source = v }),
                    text().class("subtle").content("→"),
                    cell(state, "Path in the container", "docker-run-wide",
                        move |s| s.docker.run.volumes.get(i).map(|r| r.target.clone()).unwrap_or_default(),
                        move |s, v| if let Some(r) = s.docker.run.volumes.get_mut(i) { r.target = v }),
                    sapid::button()
                        .class("button compact docker-run-toggle")
                        .label(i18n::bind("Read-only"))
                        .pressed(state.map(move |s| s.docker.run.volumes.get(i).is_some_and(|r| r.read_only)))
                        .on_press(move || state.revise(|s| {
                            if let Some(r) = s.docker.run.volumes.get_mut(i) { r.read_only = !r.read_only }
                        })),
                    icon_action("Remove volume", Icon::Close)
                        .class("icon-button")
                        .on_press(move || state.revise(|s| {
                            if i < s.docker.run.volumes.len() { s.docker.run.volumes.remove(i); }
                        })),
                ))
            }),
    ))
}

fn environment(state: Cage<State>) -> impl Widget {
    col().class("docker-run-section").fill((
        section_heading("Environment variables", move || {
            state.revise(|s| s.docker.run.environment.push(Default::default()))
        }),
        text().class("subtle").content(i18n::bind(
            "Values are saved in the project's compose.yaml, which only this SSH user can read.",
        )),
        each(state.map(|s| (0..s.docker.run.environment.len()).collect::<Vec<_>>()))
            .key(|i| *i)
            .children(move |item| {
                let i = item.value.get();
                row().class("docker-run-row").fill((
                    cell(
                        state,
                        "Variable name",
                        "docker-run-wide",
                        move |s| {
                            s.docker
                                .run
                                .environment
                                .get(i)
                                .map(|r| r.0.clone())
                                .unwrap_or_default()
                        },
                        move |s, v| {
                            if let Some(r) = s.docker.run.environment.get_mut(i) {
                                r.0 = v
                            }
                        },
                    ),
                    text().class("subtle").content("="),
                    cell(
                        state,
                        "Value",
                        "docker-run-wide",
                        move |s| {
                            s.docker
                                .run
                                .environment
                                .get(i)
                                .map(|r| r.1.clone())
                                .unwrap_or_default()
                        },
                        move |s, v| {
                            if let Some(r) = s.docker.run.environment.get_mut(i) {
                                r.1 = v
                            }
                        },
                    ),
                    icon_action("Remove variable", Icon::Close)
                        .class("icon-button")
                        .on_press(move || {
                            state.revise(|s| {
                                if i < s.docker.run.environment.len() {
                                    s.docker.run.environment.remove(i);
                                }
                            })
                        }),
                ))
            }),
    ))
}

fn restart(state: Cage<State>) -> impl Widget {
    col().class("docker-run-section").fill((
        text().class("field-label").content(i18n::bind("Restart")),
        row().class("choice-list").fill(
            Restart::ALL
                .into_iter()
                .map(|policy| {
                    let label = match policy {
                        Restart::UnlessStopped => "Unless stopped",
                        Restart::Always => "Always",
                        Restart::OnFailure => "On failure",
                        Restart::No => "Never",
                    };
                    sapid::button()
                        .class("button")
                        .label(i18n::bind(label))
                        .pressed(state.map(move |s| s.docker.run.restart == policy))
                        .on_press(move || state.revise(|s| s.docker.run.restart = policy))
                })
                .collect::<Vec<_>>(),
        ),
    ))
}

pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::DockerRun)),
        move || {
            let run = ui.clone();
            modal(
            ui.clone(),
            "Run container",
            col().class("docker-run-form").fill((
                text().class("hint").content(i18n::bind(
                    "Arcnix writes a Compose project in ~/.arcnix/apps and starts it, like App Center's Compose installs. Manage it afterwards in Projects.",
                )),
                field("Image", &ui, |s| s.docker.run.image.clone(), |s, v| {
                    s.docker.run.image = v;
                    s.docker.run.error.clear();
                }),
                field("Project name", &ui, |s| s.docker.run.name.clone(), |s, v| {
                    s.docker.run.name = v;
                    s.docker.run.error.clear();
                }),
                text().class("subtle").content(i18n::bind(state.map(|s| {
                    if s.docker.run.config.is_some() {
                        "Reading the image's ports and volumes…"
                    } else {
                        ""
                    }
                    .to_owned()
                }))),
                ports(state),
                volumes(state),
                environment(state),
                restart(state),
                btn("Preview Compose", move || {
                    state.revise(|s| s.docker.run.preview = !s.docker.run.preview)
                })
                .class("button compact"),
                when(state.map(|s| s.docker.run.preview), move || {
                    text().class("compose-preview").selectable(true).content(state.map(|s| {
                        s.docker
                            .run
                            .app()
                            .and_then(|app| app.yaml())
                            .unwrap_or_else(|e| i18n::translate(&e.to_string()))
                    }))
                }),
                text().class("hint").content(state.map(|s| {
                    if !s.server_apps.applications.compose {
                        i18n::translate("Running a container needs Docker Compose on the server.")
                    } else if scripts_ui::busy(s) {
                        i18n::translate("Another server task is running. Run when it finishes.")
                    } else {
                        String::new()
                    }
                })),
                text()
                    .class("error")
                    .content(state.map(|s| s.docker.run.error.clone())),
            )),
            sapid::button()
                .class("button primary docker-run-start")
                .label(i18n::bind("Run"))
                .enabled(state.map(can_run))
                .on_press(move || run.docker_run()),
        )
        },
    )
}
