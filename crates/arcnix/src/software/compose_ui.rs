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
    let refresh = ui.clone();
    let install = ui.clone();
    let apply = ui.clone();
    let stop = ui.clone();
    let restart = ui.clone();
    let remove = ui.clone();
    let folder = ui.clone();
    col().class("compose-projects").fill((
  row().class("compose-toolbar").fill((
   text_action("Install software",Icon::Plus).enabled(state.map(|s|s.connected)).on_press(move||install.open_software_install(2,None)),
   icon_action("Refresh projects",Icon::Refresh).enabled(state.map(ready)).on_press(move||refresh.compose_refresh()),
   help_ui::tip("Compose projects","Application definitions are in ~/.arcnix/apps/<name>/compose.yaml. Docker manages named-volume data. Applying changes is a server task; closing the client does not stop accepted work."),
  )),
  text().class("error").content(state.map(|s|s.compose.error.clone())),
  col().class("compose-project-list").fill(each(state.map(|s|s.compose.projects.as_ref().map(|p|p.entries.clone()).unwrap_or_default())).key(|p|p.definition.name.clone()).children(move|item|{let p=item.value;sapid::button().class("button compose-project-row").label(p.map(|p|format!("{} · {}",p.definition.name,p.definition.image))).pressed(state.map(move|s|s.compose.selected==p.get().definition.name)).on_press(move||state.revise(|s|s.compose.selected=p.get().definition.name))})),
  text().class("subtle compose-project-path").selectable(true).content(state.map(|s|s.compose.projects.as_ref().and_then(|v|selected(s).map(|p|format!("{}/{}/compose.yaml",v.root,p.definition.name))).unwrap_or_default())),
  row().class("compose-actions").fill((
   btn("Apply…",move||apply.compose_action(compose_apps::Action::Apply)).enabled(state.map(|s|ready(s)&&selected(s).is_some()&&!scripts_ui::busy(s))),
   btn("Stop project…",move||stop.compose_action(compose_apps::Action::Stop)).enabled(state.map(|s|ready(s)&&selected(s).is_some()&&!scripts_ui::busy(s))),
   btn("Restart project…",move||restart.compose_action(compose_apps::Action::Restart)).enabled(state.map(|s|ready(s)&&selected(s).is_some()&&!scripts_ui::busy(s))),
   btn("Remove project…",move||remove.compose_action(compose_apps::Action::Remove)).enabled(state.map(|s|ready(s)&&selected(s).is_some()&&!scripts_ui::busy(s))),
   text_action("Open folder",Icon::Folder).enabled(state.map(|s|s.sftp_available&&selected(s).is_some())).on_press(move||{let path=state.with(|s|s.compose.projects.as_ref().and_then(|v|selected(s).map(|p|format!("{}/{}",v.root,p.definition.name))));if let Some(path)=path{folder.open_desktop_app(WorkspacePage::Files(0));file_browser::navigate(&folder,true,path);}}),
   btn("Compose file",move||state.revise(|s|s.compose.preview = !s.compose.preview)),
  )),
  when(state.map(|s|s.compose.preview),move||text().class("compose-preview").selectable(true).content(state.map(|s|selected(s).map(|p|p.yaml).unwrap_or_default()))),
 ))
}
