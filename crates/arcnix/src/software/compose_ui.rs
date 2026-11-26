use arcnix_core::compose_apps::{self, Project, Projects};

use crate::*;

#[derive(Clone, Default)]
pub(crate) struct ComposeState {
    projects: Option<Projects>,
    selected: String,
    pending: Option<u64>,
    error: String,
    preview: bool,
}
/// How many projects Arcnix manages, once they have been listed.
pub(crate) fn project_count(s: &State) -> Option<usize> {
    s.compose.projects.as_ref().map(|p| p.entries.len())
}
fn ready(s: &State) -> bool {
    s.connected && s.compose.pending.is_none()
}
fn selected(s: &State) -> Option<Project> {
    s.compose
        .projects
        .as_ref()?
        .entries
        .iter()
        .find(|p| p.definition.name == s.compose.selected)
        .cloned()
}
impl Ui {
    pub(crate) fn open_compose_project(&self, name: String) {
        self.state.revise(|s| {
            s.docker.view = 3;
            s.compose.selected = name;
            s.compose.preview = false;
        });
        self.open_desktop_app(WorkspacePage::Docker);
        self.compose_refresh();
    }
    pub(crate) fn compose_enter(&self) {
        if self
            .state
            .with(|s| s.compose.projects.is_none() && s.compose.error.is_empty() && ready(s))
        {
            self.compose_refresh();
        }
    }
    pub(crate) fn compose_refresh(&self) {
        if !self.state.with(ready) {
            return;
        }
        let serial = self.send(Action::ServerApps(server_apps_worker::Action::ComposeList));
        self.state.revise(|s| {
            s.compose.pending = Some(serial);
            s.compose.error.clear();
        });
    }
    fn compose_action(&self, action: compose_apps::Action) {
        if let Some(p) = self.state.with(selected) {
            match compose_apps::action(&p, action) {
                Ok(t) => self.choose_server_script(t),
                Err(e) => self.state.revise(|s| s.compose.error = format!("{e:#}")),
            }
        }
    }
}
pub(crate) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<server_apps_worker::Reply, String>,
) -> Option<std::result::Result<server_apps_worker::Reply, String>> {
    if !ui.state.with(|s| s.compose.pending == Some(serial)) {
        return Some(result);
    }
    ui.state.revise(|s| {
        s.compose.pending = None;
        match result {
            Ok(server_apps_worker::Reply::ComposeProjects(v)) => {
                if !v
                    .entries
                    .iter()
                    .any(|p| p.definition.name == s.compose.selected)
                {
                    s.compose.selected = v
                        .entries
                        .first()
                        .map(|p| p.definition.name.clone())
                        .unwrap_or_default();
                }
                s.compose.projects = Some(v);
            }
            Err(e) => s.compose.error = e,
            _ => s.compose.error = "Invalid Compose project response".into(),
        }
    });
    None
}
pub(crate) fn projects(ui: Ui) -> impl Widget {
    let state = ui.state;
    let apply = ui.clone();
    let stop = ui.clone();
    let restart = ui.clone();
    let remove = ui.clone();
    let folder = ui.clone();
    let idle = |s: &State| ready(s) && selected(s).is_some() && !scripts_ui::busy(s);
    col().class("compose-projects").fill((
        text()
            .class("error")
            .content(state.map(|s| s.compose.error.clone())),
        when(
            state.map(|s| {
                s.compose
                    .projects
                    .as_ref()
                    .is_some_and(|p| p.entries.is_empty())
            }),
            || {
                col().class("docker-empty").fill((
                    text()
                        .class("docker-empty-title")
                        .content(i18n::bind("No Compose projects")),
                    text().content(i18n::bind(
                        "Install an application with Docker Compose to manage it here.",
                    )),
                ))
            },
        ),
        col().class("compose-project-list docker-table").fill(
            each(state.map(|s| {
                s.compose
                    .projects
                    .as_ref()
                    .map(|p| p.entries.clone())
                    .unwrap_or_default()
            }))
            .key(|p| p.definition.name.clone())
            .children(move |item| {
                let p = item.value;
                sapid::button()
                    .class("compose-project-row")
                    .name(p.map(|p| p.definition.name.clone()))
                    .pressed(state.map(move |s| s.compose.selected == p.get().definition.name))
                    .fill((
                        text()
                            .class("docker-resource-name")
                            .content(p.map(|p| p.definition.name.clone())),
                        text()
                            .class("docker-resource-image")
                            .content(p.map(|p| p.definition.image.clone())),
                        text().class("docker-chip").content(
                            p.map(|p| format!("{}:{}", p.definition.address, p.definition.port)),
                        ),
                    ))
                    .on_press(move || {
                        state.revise(|s| s.compose.selected = p.get().definition.name)
                    })
            }),
        ),
        when(state.map(|s| selected(s).is_some()), move || {
            let apply = apply.clone();
            let stop = stop.clone();
            let restart = restart.clone();
            let remove = remove.clone();
            let folder = folder.clone();
            col().class("compose-project-panel").fill((
                text().class("docker-detail-name").content(
                    state.map(|s| selected(s).map(|p| p.definition.name).unwrap_or_default()),
                ),
                text()
                    .class("compose-project-path")
                    .selectable(true)
                    .content(state.map(|s| {
                        s.compose
                            .projects
                            .as_ref()
                            .and_then(|v| {
                                selected(s).map(|p| {
                                    format!("{}/{}/compose.yaml", v.root, p.definition.name)
                                })
                            })
                            .unwrap_or_default()
                    })),
                row().class("compose-actions").fill((
                    btn("Apply…", move || {
                        apply.compose_action(compose_apps::Action::Apply)
                    })
                    .class("button compact")
                    .enabled(state.map(idle)),
                    btn("Stop project…", move || {
                        stop.compose_action(compose_apps::Action::Stop)
                    })
                    .class("button compact")
                    .enabled(state.map(idle)),
                    btn("Restart project…", move || {
                        restart.compose_action(compose_apps::Action::Restart)
                    })
                    .class("button compact")
                    .enabled(state.map(idle)),
                    text_action("Open folder", Icon::Folder)
                        .class("button with-icon compact")
                        .enabled(state.map(|s| s.sftp_available && selected(s).is_some()))
                        .on_press(move || {
                            let path = state.with(|s| {
                                s.compose.projects.as_ref().and_then(|v| {
                                    selected(s).map(|p| format!("{}/{}", v.root, p.definition.name))
                                })
                            });
                            if let Some(path) = path {
                                folder.open_desktop_app(WorkspacePage::Files(0));
                                file_browser::navigate(&folder, true, path);
                            }
                        }),
                    btn("Compose file", move || {
                        state.revise(|s| s.compose.preview = !s.compose.preview)
                    })
                    .class("button compact")
                    .pressed(state.map(|s| s.compose.preview)),
                    btn("Remove project…", move || {
                        remove.compose_action(compose_apps::Action::Remove)
                    })
                    .class("button compact docker-remove")
                    .enabled(state.map(idle)),
                )),
                when(state.map(|s| s.compose.preview), move || {
                    text()
                        .class("compose-preview")
                        .selectable(true)
                        .content(state.map(|s| selected(s).map(|p| p.yaml).unwrap_or_default()))
                }),
            ))
        }),
    ))
}
