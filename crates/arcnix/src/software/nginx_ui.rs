//! The Nginx app: service status, sites and a site editor, for Nginx on the
//! server and in Docker containers. Every change is a server task that people
//! review, and that runs with administrator rights.
use std::rc::Rc;

use arcnix_core::nginx::{self, Control, Kind, Layout, Serves, Snapshot, Style};
use arcnix_core::nginx_site::{self, Main, Site};

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{btn, icon_action, text_action};
use crate::server_apps_worker::{Action as AppAction, Reply};
use crate::*;

mod certificates;
mod form;
mod logs;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum View {
    #[default]
    Overview,
    Sites,
    Logs,
}

#[derive(Clone, Default)]
pub(crate) struct Editor {
    /// The instance the file belongs to.
    instance: String,
    /// The path Nginx knows.
    path: String,
    /// Which site of the file this is, when the file holds several.
    scope: Option<String>,
    /// The main configuration's own settings, without its sites.
    main_only: bool,
    /// The file on the server that publishing writes; empty when the file is
    /// inside a container image and can only be read.
    target: String,
    /// The live text when the editor opened; empty for a new file.
    original: String,
    text: String,
    /// The text as the Settings view shows it.
    site: Site,
    /// The main configuration's settings, for nginx.conf.
    main: Option<Main>,
    /// Values typed into Settings fields, by field, so typing is not lost
    /// while a value is not valid yet.
    drafts: std::collections::HashMap<String, String>,
    adding: form::Adding,
    file_view: bool,
    health: String,
    create: bool,
    error: String,
    /// The Settings control the error came from, where it is shown instead
    /// of above the form.
    error_at: Option<String>,
    /// Why the last publication of this file failed, from the server.
    failure: String,
    /// What happened that the operator should know, such as a certificate
    /// that is ready to publish.
    info: String,
    /// The certificate name being issued for this site.
    issuing: Option<String>,
}

/// The text as the Settings view shows it: one site, the main
/// configuration's settings, or both for a file that holds one site.
fn settings_of(
    main_only: bool,
    scope: Option<&str>,
    text: &str,
) -> anyhow::Result<(Site, Option<Main>)> {
    if main_only {
        return Ok((Site::default(), nginx_site::read_main(text)?));
    }
    let site = nginx_site::read_in(text, scope)?;
    let main = match scope {
        Some(_) => None,
        None => nginx_site::read_main(text)?,
    };
    Ok((site, main))
}

impl Editor {
    fn read(&self, text: &str) -> anyhow::Result<(Site, Option<Main>)> {
        settings_of(self.main_only, self.scope.as_deref(), text)
    }
    /// The [`nginx::Site::key`] of the site being edited.
    fn key(&self) -> String {
        match &self.scope {
            Some(scope) => format!("{}#{scope}", self.path),
            None => self.path.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct NewSite {
    kind: Kind,
    names: String,
    target: String,
}

#[derive(Clone, Default)]
pub(crate) struct NginxState {
    /// Every Nginx on the server, and the one shown.
    instances: Vec<Rc<Snapshot>>,
    instance: String,
    snapshot: Option<Rc<Snapshot>>,
    /// Why Docker containers could not be listed.
    docker_error: String,
    /// Containers that run Nginx only inside an application, which are not
    /// listed as instances.
    hidden: Vec<String>,
    pending: Option<u64>,
    error: String,
    /// Why the last site or service task failed.
    notice: String,
    view: View,
    filter: String,
    editor: Option<Editor>,
    new_site: NewSite,
    logs: logs::Logs,
    certificates: certificates::CertificateState,
}

/// What a row in the sites list needs, without the file's text.
#[derive(Clone, PartialEq)]
struct Row {
    path: String,
    title: String,
    file: String,
    names: String,
    ports: String,
    serves: String,
    https: bool,
    enabled: bool,
    switchable: bool,
    readable: bool,
    invalid: bool,
    note: &'static str,
}

fn rows(s: &State, enabled_only: bool) -> Vec<Row> {
    let Some(snapshot) = &s.nginx.snapshot else {
        return vec![];
    };
    let filter = s.nginx.filter.to_lowercase();
    snapshot
        .sites
        .iter()
        .filter(|site| !enabled_only || site.enabled)
        .map(|site| Row {
            path: site.key(),
            title: site.title(),
            file: site.file.path.clone(),
            names: site.summary.names.join(" "),
            ports: site
                .summary
                .ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            serves: serves(&site.summary.serves),
            https: site.summary.https,
            enabled: site.enabled,
            switchable: site.switchable(),
            readable: site.file.text.is_some(),
            invalid: site.invalid,
            note: if site.file.text.is_none() {
                "Arcnix cannot read this file as the SSH account"
            } else if site.invalid {
                "Not valid Nginx configuration"
            } else if !site.file.editable() {
                "Inside the container image; mount it from the server to change it"
            } else if site.scope.is_some() && site.file.path == snapshot.main.path {
                "In the main configuration, so it stays on"
            } else if site.scope.is_some() {
                "Shares its file with other sites, so it is turned on and off in File"
            } else if site.style == Style::Fixed && site.file.path.contains("/sites-enabled/") {
                "Copied into sites-enabled, so it stays on"
            } else if site.style == Style::Fixed {
                "Nginx loads this file directly, so it stays on"
            } else {
                ""
            },
        })
        .filter(|row| {
            enabled_only
                || filter.is_empty()
                || [&row.title, &row.file, &row.names, &row.serves]
                    .iter()
                    .any(|v| v.to_lowercase().contains(&filter))
        })
        .collect()
}

fn serves(serves: &Serves) -> String {
    match serves {
        Serves::Proxy(target) => format!("→ {target}"),
        Serves::Folder(folder) => folder.clone(),
        Serves::Redirect(target) => format!("↪ {target}"),
        Serves::Other => String::new(),
    }
}

pub(crate) fn ready(s: &State) -> bool {
    s.connected && s.server_apps.applications.nginx && s.nginx.pending.is_none()
}

/// Whether a server task can start now.
fn can_change(s: &State) -> bool {
    s.connected && !scripts_ui::busy(s) && s.nginx.snapshot.as_ref().is_some_and(|n| n.installed())
}

/// Sites in a container switch only while it runs, so Nginx can check them.
fn can_switch(s: &State) -> bool {
    can_change(s)
        && s.nginx
            .snapshot
            .as_ref()
            .is_some_and(|n| n.docker.as_ref().is_none_or(|c| c.running()))
}

fn status(s: &State) -> (&'static str, &'static str) {
    if let Some(container) = s.nginx.snapshot.as_ref().and_then(|n| n.docker.as_ref()) {
        return match container.state.as_str() {
            "running" => ("Running", "nginx-status running"),
            "restarting" => ("Restarting", "nginx-status"),
            "paused" => ("Paused", "nginx-status stopped"),
            "dead" => ("Failed", "nginx-status failed"),
            _ => ("Stopped", "nginx-status stopped"),
        };
    }
    match s.nginx.snapshot.as_ref().map(|n| n.service.as_ref()) {
        None => ("Checking…", "nginx-status"),
        Some(None) => ("Service status unavailable", "nginx-status"),
        Some(Some(service)) => match service.active.as_str() {
            "active" => ("Running", "nginx-status running"),
            "reloading" => ("Reloading", "nginx-status running"),
            "failed" => ("Failed", "nginx-status failed"),
            "activating" => ("Starting", "nginx-status"),
            "deactivating" => ("Stopping", "nginx-status"),
            _ if service.load == "not-found" => ("No nginx service", "nginx-status"),
            _ => ("Stopped", "nginx-status stopped"),
        },
    }
}

fn uptime(seconds: u64) -> String {
    match seconds {
        0..60 => "up less than a minute".into(),
        60..3600 => format!("up {} min", seconds / 60),
        3600..86400 => format!("up {} h", seconds / 3600),
        _ => format!("up {} days", seconds / 86400),
    }
}

fn details(s: &State) -> String {
    let Some(snapshot) = &s.nginx.snapshot else {
        return String::new();
    };
    if !snapshot.installed() {
        return i18n::translate("Nginx was not found on this server");
    }
    let mut parts = vec![if snapshot.version.is_empty() {
        "Nginx".to_owned()
    } else {
        snapshot.version.clone()
    }];
    if let Some(seconds) = snapshot.uptime() {
        parts.push(i18n::translate(&uptime(seconds)));
    }
    match &snapshot.docker {
        Some(container) => parts.push(i18n::translate(if container.starts_at_boot() {
            "restarts automatically"
        } else {
            "does not restart automatically"
        })),
        None if snapshot.service.is_some() => {
            parts.push(i18n::translate(if snapshot.starts_at_boot() {
                "starts at boot"
            } else {
                "does not start at boot"
            }))
        }
        None => {}
    }
    parts.join(" · ")
}

/// Where a containerized Nginx comes from.
fn origin(s: &State) -> String {
    let Some(container) = s.nginx.snapshot.as_ref().and_then(|n| n.docker.as_ref()) else {
        return String::new();
    };
    let mut parts = vec![if container.compose() {
        i18n::translate(&format!(
            "Compose service {} in project {}",
            container.service, container.project
        ))
    } else {
        i18n::translate(&format!("Docker container {}", container.name))
    }];
    parts.push(container.image.clone());
    if !container.directory.is_empty() {
        parts.push(container.directory.clone());
    }
    parts.join(" · ")
}

fn editing(s: &State) -> bool {
    s.nginx.editor.is_some()
}

/// The open file can change: it is not inside a container image.
fn writable(s: &State) -> bool {
    s.nginx
        .editor
        .as_ref()
        .is_some_and(|e| !e.target.is_empty())
}

fn changed(editor: &Editor) -> bool {
    editor.create || editor.text != editor.original
}

fn can_publish(s: &State) -> bool {
    can_change(s)
        && s.nginx.editor.as_ref().is_some_and(|e| {
            changed(e) && e.error.is_empty() && !e.target.is_empty() && !e.text.trim().is_empty()
        })
}

/// The end of a task's output, where the reason for a failure is.
fn tail(output: &str) -> String {
    let lines: Vec<&str> = output.trim_end().lines().collect();
    lines[lines.len().saturating_sub(8)..].join("\n")
}

/// The form needs at least one field it can show.
fn has_form(site: &Site, main: Option<&Main>) -> bool {
    site.names.is_some() || site.server.is_some() || site.upload.is_some() || main.is_some()
}

impl Ui {
    pub(crate) fn nginx_enter(&self) {
        if self.state.with(|s| {
            navigation::workspace_visible(s)
                && s.workspace_page == WorkspacePage::Nginx
                && s.nginx.snapshot.is_none()
                && s.nginx.error.is_empty()
                && ready(s)
        }) {
            self.nginx_refresh();
        }
    }
    fn nginx_refresh(&self) {
        if !self.state.with(ready) {
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::NginxRead));
        self.state.revise(|s| {
            s.nginx.pending = Some(serial);
            s.nginx.error.clear();
        });
    }
    /// After a server task that may have changed Nginx: explain a failure
    /// where the operator is looking, then read Nginx again.
    pub(crate) fn nginx_task_finished(&self, task: &str) {
        let finished = self.state.with(|s| {
            let record = s.scripts.tasks.iter().find(|t| t.id == task)?;
            let target = record
                .targets
                .iter()
                .find(|t| s.selected.as_ref() == Some(&t.id))?;
            let failed = target.status == arcnix_core::tasks::Status::Failed;
            let private = s
                .scripts
                .private_failures
                .get(&(task.to_owned(), target.id.clone()))
                .cloned();
            Some((
                record.configuration.as_ref().map(|r| r.spec.path.clone()),
                record.name.clone(),
                failed,
                private,
                target.output.clone(),
                record.script_id == "builtin:nginx:certificate",
                target.status == arcnix_core::tasks::Status::Succeeded,
            ))
        });
        if let Some((_, _, _, _, _, true, succeeded)) = &finished {
            self.nginx_issued(*succeeded);
        }
        if let Some((path, name, failed, private, output, ..)) = finished {
            self.state.revise(|s| match path {
                Some(path) => {
                    let message = match (failed, private) {
                        (false, _) => String::new(),
                        (true, Some(detail)) => format!(
                            "{}\n\n{detail}",
                            i18n::translate("The check failed, so nothing on the server changed.")
                        ),
                        (true, None) => arcnix_core::configurations::outcome(&output)
                            .map(i18n::translate)
                            .unwrap_or_else(|| tail(&output)),
                    };
                    if let Some(editor) = s.nginx.editor.as_mut().filter(|e| e.target == path) {
                        editor.failure = message;
                    }
                }
                None if failed => {
                    s.nginx.notice = format!(
                        "{}\n{}",
                        i18n::translate(&format!("{name} failed")),
                        tail(&output)
                    );
                }
                None => s.nginx.notice.clear(),
            });
        }
        if self.state.with(|s| {
            s.server_apps.applications.nginx && s.workbench.apps.contains(&WorkspacePage::Nginx)
        }) {
            self.state.revise(|s| s.nginx.pending = None);
            self.nginx_refresh();
        }
    }
    fn nginx_task(&self, template: anyhow::Result<arcnix_core::scripts::Template>) {
        self.state.revise(|s| s.nginx.notice.clear());
        match template {
            Ok(template) => self.choose_server_script(template),
            Err(error) => self.state.revise(|s| s.nginx.error = format!("{error:#}")),
        }
    }
    fn nginx_control(&self, control: Control) {
        let template = self.state.with(|s| {
            s.nginx
                .snapshot
                .as_ref()
                .filter(|_| can_change(s))
                .map(|n| nginx::control(n, control))
        });
        if let Some(template) = template {
            self.nginx_task(template);
        }
    }
    fn nginx_switch(&self, path: &str, on: bool) {
        let template = self.state.with(|s| {
            let snapshot = s.nginx.snapshot.as_ref().filter(|_| can_switch(s))?;
            Some(nginx::switch(snapshot, snapshot.site(path)?, on))
        });
        if let Some(template) = template {
            self.nginx_task(template);
        }
    }
    /// Opens a site by its key, or the main configuration by its path.
    fn nginx_open(&self, key: &str) {
        let editor = self.state.with(|s| {
            let snapshot = s.nginx.snapshot.as_ref()?;
            // A site that is off cannot answer, so it gets no health address.
            let (file, summary, scope) = match snapshot.site(key) {
                Some(site) => (
                    &site.file,
                    site.enabled.then_some(&site.summary),
                    Some(site.scope.clone()),
                ),
                None if snapshot.main.path == key => (&snapshot.main, None, None),
                None => return None,
            };
            let text = file.text.clone()?;
            // A site's domain does not reach a container from the server, so
            // a containerized Nginx checks only that it keeps running.
            let summary = summary.filter(|_| snapshot.docker.is_none());
            let mut editor = Editor {
                instance: snapshot.key(),
                path: file.path.clone(),
                main_only: scope.is_none(),
                scope: scope.flatten(),
                target: file.target.clone(),
                original: text.clone(),
                file_view: false,
                site: Site::default(),
                main: None,
                drafts: Default::default(),
                adding: Default::default(),
                text,
                health: summary.map(nginx::health_address).unwrap_or_default(),
                create: false,
                error: String::new(),
                error_at: None,
                failure: String::new(),
                info: String::new(),
                issuing: None,
            };
            let (site, main) = editor.read(&editor.text).unwrap_or_default();
            editor.file_view = !has_form(&site, main.as_ref());
            editor.site = site;
            editor.main = main;
            Some(editor)
        });
        if editor.is_some() {
            self.state.revise(|s| s.nginx.editor = editor);
        }
    }
    fn nginx_new_site(&self) {
        self.state.revise(|s| {
            s.nginx.new_site = NewSite::default();
            s.dialog_error.clear();
        });
        self.open_dialog(Dialog::NginxNewSite);
    }
    fn nginx_create_site(&self) {
        let result = self.state.with(|s| -> anyhow::Result<Editor> {
            let snapshot = s
                .nginx
                .snapshot
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Wait for Nginx to load"))?;
            let new = &s.nginx.new_site;
            let text = nginx::new_site(new.kind, &new.names, new.target.trim())?;
            let (path, target) = nginx::new_site_path(snapshot, &new.names)?;
            let site = nginx_site::read(&text).unwrap_or_default();
            let main = nginx_site::read_main(&text).ok().flatten();
            Ok(Editor {
                instance: snapshot.key(),
                path,
                scope: None,
                main_only: false,
                target,
                original: String::new(),
                text,
                file_view: false,
                site,
                main,
                drafts: Default::default(),
                adding: Default::default(),
                // A new site that is not on yet cannot answer, so only check
                // that Nginx keeps running.
                health: String::new(),
                create: true,
                error: String::new(),
                error_at: None,
                failure: String::new(),
                info: String::new(),
                issuing: None,
            })
        });
        match result {
            Ok(editor) => {
                self.state.revise(|s| {
                    s.nginx.editor = Some(editor);
                    s.nginx.view = View::Sites;
                    s.dialog_error.clear();
                });
                self.close_dialog();
            }
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
        }
    }
    fn nginx_file_view(&self, file: bool) {
        self.state.revise(|s| {
            let Some(editor) = &mut s.nginx.editor else {
                return;
            };
            if file == editor.file_view {
                return;
            }
            if !file {
                match editor.read(&editor.text) {
                    Ok((site, main)) => {
                        editor.site = site;
                        editor.main = main;
                        editor.drafts.clear();
                        editor.error.clear();
                    }
                    Err(error) => {
                        editor.error_at = None;
                        editor.error = i18n::translate(&format!(
                            "The file cannot be read as Nginx configuration: {error:#}"
                        ));
                        return;
                    }
                }
            }
            editor.file_view = file;
        });
    }
    fn nginx_publish(&self) {
        if !self.state.with(can_publish) {
            return;
        }
        let template = self.state.with(|s| {
            let editor = s.nginx.editor.as_ref().unwrap();
            let snapshot = s
                .nginx
                .snapshot
                .as_ref()
                .filter(|n| n.key() == editor.instance)
                .ok_or_else(|| anyhow::anyhow!("This Nginx is no longer on the server"))?;
            nginx::publish(
                snapshot,
                &editor.target,
                &editor.text,
                &editor.health,
                editor.create,
            )
        });
        match template {
            Ok(template) => {
                self.state.revise(|s| {
                    s.nginx.notice.clear();
                    if let Some(editor) = &mut s.nginx.editor {
                        editor.failure.clear();
                    }
                });
                self.choose_server_script(template);
            }
            Err(error) => self.state.revise(|s| {
                if let Some(editor) = &mut s.nginx.editor {
                    editor.error = format!("{error:#}");
                    editor.error_at = None;
                }
            }),
        }
    }
}

pub(crate) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    let result = logs::event(ui, serial, result)?;
    let result = certificates::event(ui, serial, result)?;
    if !ui.state.with(|s| s.nginx.pending == Some(serial)) {
        return Some(result);
    }
    let read = matches!(result, Ok(Reply::Nginx(_)));
    ui.state.revise(|s| {
        s.nginx.pending = None;
        match result {
            Ok(Reply::Nginx(reading)) => {
                // An application's own Nginx is listed only when there is no
                // other to manage.
                let managed = reading.instances.iter().any(|n| !n.internal());
                let (instances, hidden): (Vec<Snapshot>, Vec<Snapshot>) = reading
                    .instances
                    .into_iter()
                    .partition(|n| !managed || !n.internal());
                s.nginx.hidden = hidden
                    .iter()
                    .filter_map(|n| n.docker.as_ref().map(|c| c.name.clone()))
                    .collect();
                let instances: Vec<Rc<Snapshot>> = instances.into_iter().map(Rc::new).collect();
                let selected = instances
                    .iter()
                    .find(|n| n.key() == s.nginx.instance)
                    .or(instances.first())
                    .cloned();
                s.nginx.instance = selected.as_ref().map(|n| n.key()).unwrap_or_default();
                if let Some(editor) = &mut s.nginx.editor {
                    match selected.as_ref().filter(|n| n.key() == editor.instance) {
                        // A published file now matches what is being edited.
                        Some(snapshot) => {
                            let live = snapshot.file(&editor.path).and_then(|f| f.text.clone());
                            // Publishing gives every line the file's own ending.
                            let lines = |t: &str| t.replace("\r\n", "\n");
                            if let Some(live) = live.filter(|l| lines(l) == lines(&editor.text)) {
                                editor.text = live.clone();
                                editor.original = live;
                                editor.create = false;
                            }
                        }
                        None => s.nginx.editor = None,
                    }
                }
                s.nginx.snapshot = selected;
                s.nginx.instances = instances;
                s.nginx.docker_error = reading.docker_error;
            }
            Err(error) => s.nginx.error = error,
            _ => s.nginx.error = "Invalid Nginx response".into(),
        }
    });
    if read {
        ui.nginx_certificates();
    }
    None
}

fn navigation(ui: Ui) -> impl Widget {
    let state = ui.state;
    let items = [
        ("Overview", Icon::Grid, View::Overview),
        ("Sites", Icon::WebServer, View::Sites),
        ("Logs", Icon::Logs, View::Logs),
    ]
    .into_iter()
    .map({
        let ui = ui.clone();
        move |(label, icon, view)| {
            let ui = ui.clone();
            text_action(label, icon)
                .class("button with-icon nginx-nav-item")
                .pressed(state.map(move |s| s.nginx.view == view && !editing(s)))
                .on_press(move || {
                    state.revise(|s| {
                        s.nginx.view = view;
                        s.nginx.editor = None;
                    });
                    if view == View::Logs {
                        ui.nginx_logs();
                    }
                })
        }
    })
    .collect::<Vec<_>>();
    col().class("nginx-nav").fill((
        col().class("nginx-views").fill(items),
        when(state.map(|s| s.nginx.instances.len() > 1), move || {
            let ui = ui.clone();
            col().class("nginx-instances").fill((
                text()
                    .class("nginx-nav-label")
                    .content(i18n::bind("Instances")),
                each(state.map(|s| {
                    s.nginx
                        .instances
                        .iter()
                        .map(|n| (n.key(), n.title(), n.docker.is_some(), n.running()))
                        .collect::<Vec<_>>()
                }))
                .key(|i| format!("{}\n{}\n{}", i.0, i.1, i.3))
                .children(move |item| {
                    let (key, title, docker, running) = item.value.get();
                    let chosen = key.clone();
                    let logs_ui = ui.clone();
                    text_action(
                        title,
                        if docker {
                            Icon::Docker
                        } else {
                            Icon::WebServer
                        },
                    )
                    .class(if running {
                        "button with-icon nginx-nav-item nginx-instance"
                    } else {
                        "button with-icon nginx-nav-item nginx-instance stopped"
                    })
                    .pressed(state.map(move |s| s.nginx.instance == key))
                    .on_press(move || {
                        state.revise(|s| {
                            if s.nginx.instance == chosen {
                                return;
                            }
                            s.nginx.snapshot = s
                                .nginx
                                .instances
                                .iter()
                                .find(|n| n.key() == chosen)
                                .cloned();
                            s.nginx.instance = chosen.clone();
                            s.nginx.editor = None;
                            s.nginx.notice.clear();
                            s.nginx.logs.source.clear();
                        });
                        logs_ui.nginx_certificates();
                        if logs_ui.state.with(|s| s.nginx.view == View::Logs) {
                            logs_ui.nginx_logs();
                        }
                    })
                }),
            ))
        }),
    ))
}

fn control_button(
    ui: &Ui,
    label: &'static str,
    class: &'static str,
    control: Control,
    shown: fn(&State) -> bool,
) -> impl Widget + use<> {
    let state = ui.state;
    let ui = ui.clone();
    sapid::button()
        .class(state.map(move |s| {
            if shown(s) {
                class.to_owned()
            } else {
                format!("{class} hidden")
            }
        }))
        .label(i18n::bind(label))
        .enabled(state.map(can_change))
        .on_press(move || ui.nginx_control(control))
}

fn header(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    let running: fn(&State) -> bool = |s| s.nginx.snapshot.as_ref().is_some_and(|n| n.running());
    col().class("nginx-header").fill((
        row().class("nginx-title-row").fill((
            text().class("nginx-title").content(i18n::bind("Overview")),
            text()
                .class(state.map(|s| status(s).1.to_owned()))
                .content(i18n::bind(state.map(|s| status(s).0.to_owned()))),
            icon_action("Refresh Nginx", Icon::Refresh)
                .class("icon-button")
                .enabled(state.map(ready))
                .on_press(move || refresh.nginx_refresh()),
        )),
        text()
            .class("nginx-details subtle")
            .content(state.map(details)),
        text()
            .class(state.map(|s| {
                if origin(s).is_empty() {
                    "hidden"
                } else {
                    "nginx-details subtle"
                }
                .to_owned()
            }))
            .selectable(true)
            .content(state.map(origin)),
        row().class("nginx-actions").fill((
            control_button(
                &ui,
                "Test configuration…",
                "button",
                Control::Test,
                |_| true,
            ),
            control_button(&ui, "Reload…", "button primary", Control::Reload, running),
            control_button(&ui, "Restart…", "button", Control::Restart, running),
            control_button(&ui, "Start…", "button primary", Control::Start, |s| {
                !s.nginx.snapshot.as_ref().is_some_and(|n| n.running())
            }),
            control_button(&ui, "Stop…", "button danger", Control::Stop, running),
        )),
    ))
}

fn stat(state: Cage<State>, label: &'static str, value: fn(&State) -> String) -> impl Widget {
    col().class("nginx-stat").fill((
        text().class("nginx-stat-label").content(i18n::bind(label)),
        text()
            .class("nginx-stat-value")
            .content(i18n::bind(state.map(value))),
    ))
}

fn overview(ui: Ui) -> impl Widget {
    let state = ui.state;
    let all = ui.clone();
    col().class("nginx-overview").fill((
        header(ui.clone()),
        row().class("nginx-stats").fill((
            stat(state, "Listening ports", |s| {
                s.nginx
                    .snapshot
                    .as_ref()
                    .map(|n| {
                        let ports = n.ports();
                        // The ports come from the configuration, so they are
                        // only listening while Nginx runs.
                        if !n.running() {
                            "Not listening".to_owned()
                        } else if let Some(container) = &n.docker {
                            // What the server listens on for the container.
                            let published: Vec<String> = container
                                .ports
                                .iter()
                                .map(|p| p.split('/').next().unwrap_or(p).replace(':', " → "))
                                .collect();
                            if published.is_empty() {
                                "Not published".to_owned()
                            } else {
                                published.join(", ")
                            }
                        } else if ports.is_empty() {
                            "None".to_owned()
                        } else {
                            ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
                        }
                    })
                    .unwrap_or_default()
            }),
            stat(state, "Sites", |s| {
                s.nginx
                    .snapshot
                    .as_ref()
                    .map(|n| {
                        format!(
                            "{} of {} on",
                            n.sites.iter().filter(|site| site.enabled).count(),
                            n.sites.len()
                        )
                    })
                    .unwrap_or_default()
            }),
            stat(state, "HTTPS", |s| {
                s.nginx
                    .snapshot
                    .as_ref()
                    .map(|n| {
                        format!(
                            "{} sites",
                            n.sites
                                .iter()
                                .filter(|site| site.enabled && site.summary.https)
                                .count()
                        )
                    })
                    .unwrap_or_default()
            }),
        )),
        col().class("nginx-section").fill((
            row().class("section-heading").fill((
                text().class("section-title").content(i18n::bind("Sites that are on")),
                btn("All sites", move || {
                    all.state.revise(|s| s.nginx.view = View::Sites)
                }),
            )),
            site_list(ui.clone(), true),
            when(
                state.map(|s| s.nginx.snapshot.is_some() && rows(s, true).is_empty()),
                || {
                    text()
                        .class("empty-copy")
                        .content(i18n::bind("No sites are on"))
                },
            ),
        )),
        certificates::overview(&ui),
        text().class("hint").content(i18n::bind(
            "Changes run as server tasks with administrator rights. You review each one before it runs, and it is kept in task history.",
        )),
        text().class("hint").content(state.map(|s| {
            if s.nginx.hidden.is_empty() {
                return String::new();
            }
            let names = s.nginx.hidden.join(", ");
            i18n::translate(&format!(
                "Not listed: {names}. These containers run Nginx inside an application; they publish no port and nothing in them comes from the server, so there is nothing to manage."
            ))
        })),
        text().class("hint").content(i18n::bind(state.map(|s| {
            match s.nginx.docker_error.as_str() {
                "permission" => "Arcnix cannot list Docker containers as the SSH account, so Nginx in containers is not shown. Add the account to the docker group to manage it.",
                "unavailable" => "Docker did not answer, so Nginx in containers is not shown.",
                _ => "",
            }
            .to_owned()
        }))),
    ))
}

fn site_list(ui: Ui, enabled_only: bool) -> impl Widget {
    let state = ui.state;
    col().class("nginx-sites").fill(
        each(state.map(move |s| rows(s, enabled_only)))
            // Rows show fixed text, so any change rebuilds the row.
            .key(|row| {
                format!(
                    "{}\n{}\n{}\n{}\n{}\n{}\n{}{}{}{}{}",
                    row.note,
                    row.path,
                    row.title,
                    row.names,
                    row.ports,
                    row.serves,
                    row.https,
                    row.enabled,
                    row.switchable,
                    row.readable,
                    row.invalid
                )
            })
            .children(move |item| site_row(ui.clone(), item.value, enabled_only)),
    )
}

fn site_row(ui: Ui, row_value: Cage<Row>, compact: bool) -> impl Widget {
    let state = ui.state;
    let open = ui.clone();
    let switch = ui.clone();
    let row_data = row_value.get();
    let path = row_data.path.clone();
    let switch_path = row_data.path.clone();
    let mut detail = Vec::new();
    if !row_data.names.is_empty() && row_data.names != row_data.title {
        detail.push(row_data.names.clone());
    }
    if !row_data.ports.is_empty() {
        detail.push(row_data.ports.clone());
    }
    if !row_data.serves.is_empty() {
        detail.push(row_data.serves.clone());
    }
    let note = row_data.note;
    row().class("nginx-site-row").fill((
        sapid::button()
            .class("nginx-site-open")
            .enabled(row_data.readable)
            .on_press(move || open.nginx_open(&path))
            .fill((
                text()
                    .class("nginx-site-name")
                    .content(row_data.title.clone()),
                text()
                    .class("nginx-site-detail")
                    .content(detail.join(" · ")),
                text()
                    .class(if note.is_empty() {
                        "hidden"
                    } else {
                        "nginx-site-note"
                    })
                    .content(i18n::bind(note)),
            )),
        text()
            .class(if row_data.https {
                "nginx-badge"
            } else {
                "hidden"
            })
            .content("HTTPS"),
        when(
            row_value.map(move |r| !compact && r.switchable),
            move || {
                let switch = switch.clone();
                let path = switch_path.clone();
                sapid::checkbox()
                    .class("preference-check nginx-switch")
                    .label(i18n::bind("On"))
                    .checked(row_value.map(|r| r.enabled))
                    .enabled(state.map(can_switch))
                    .on_checked_change(move |on| switch.nginx_switch(&path, on))
            },
        ),
    ))
}

fn sites(ui: Ui) -> impl Widget {
    let state = ui.state;
    let main = ui.clone();
    let new = ui.clone();
    col().class("nginx-sites-view").fill((
        row().class("nginx-title-row").fill((
            text().class("nginx-title").content(i18n::bind("Sites")),
            row().class("nginx-search").fill((
                icons::glyph(Icon::Search),
                input()
                    .class("nginx-filter")
                    .name(i18n::bind("Filter sites"))
                    .value(state.map(|s| s.nginx.filter.clone()))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event {
                            state.revise(|s| s.nginx.filter = value);
                        }
                    }),
            )),
            sapid::button()
                .class("button")
                .label(i18n::bind("Main configuration"))
                .enabled(state.map(|s| {
                    s.nginx
                        .snapshot
                        .as_ref()
                        .is_some_and(|n| n.main.text.is_some())
                }))
                .on_press(move || {
                    let path = main.state.with(|s| {
                        s.nginx
                            .snapshot
                            .as_ref()
                            .map(|n| n.main.path.clone())
                            .unwrap_or_default()
                    });
                    main.nginx_open(&path);
                }),
            sapid::button()
                .class("button primary")
                .label(i18n::bind("New site…"))
                .enabled(state.map(|s| {
                    can_change(s)
                        && s.nginx
                            .snapshot
                            .as_ref()
                            .is_some_and(|n| n.layout != Layout::None && !n.dirs.is_empty())
                }))
                .on_press(move || new.nginx_new_site()),
        )),
        site_list(ui.clone(), false),
        when(
            state.map(|s| s.nginx.snapshot.is_some() && rows(s, false).is_empty()),
            || {
                text()
                    .class("empty-copy")
                    .content(i18n::bind("No matching sites"))
            },
        ),
        text().class("hint").content(i18n::bind(state.map(|s| {
            let mounted = s.nginx.snapshot.as_ref().is_none_or(|n| {
                n.docker.is_none() || n.main.editable() || n.sites.iter().any(|site| site.file.editable())
            });
            if !mounted {
                return "Nothing in this container is mounted from the server, so its configuration can only be read. Mount the configuration folder from the server to manage it here.".to_owned();
            }
            match s.nginx.snapshot.as_ref().map(|n| n.layout) {
                Some(Layout::Debian) => "Sites in sites-available are turned on with a link in sites-enabled. Turning a site on or off checks the configuration and reloads Nginx, and is undone if either fails.",
                Some(Layout::Confd) => "Sites in conf.d are turned off by renaming them to .conf.disabled. Turning a site on or off checks the configuration and reloads Nginx, and is undone if either fails.",
                _ => "",
            }
            .to_owned()
        }))),
        when(
            state.map(|s| s.nginx.snapshot.as_ref().is_some_and(|n| n.truncated)),
            || {
                text()
                    .class("hint")
                    .content(i18n::bind("Only the first 200 site files are shown."))
            },
        ),
    ))
}

fn field(label: &'static str, control: impl Widget) -> impl Widget {
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        control,
    ))
}

fn editor(ui: Ui) -> impl Widget {
    let state = ui.state;
    let back = ui.clone();
    let form_view = ui.clone();
    let file_view = ui.clone();
    let discard = ui.clone();
    let publish = ui.clone();
    let file = |s: &State| s.nginx.editor.as_ref().is_some_and(|e| e.file_view);
    col().class("nginx-editor").fill((
        sapid::button()
            .class("nginx-back")
            .label(i18n::bind("‹ Sites"))
            .on_press(move || back.state.revise(|s| s.nginx.editor = None)),
        row().class("nginx-title-row").fill((
            col().class("nginx-editor-title").fill((
                text().class("nginx-title").content(state.map(|s| {
                    s.nginx
                        .editor
                        .as_ref()
                        .map(|e| {
                            s.nginx
                                .snapshot
                                .as_ref()
                                .and_then(|n| n.site(&e.key()))
                                .map(|site| site.title())
                                .unwrap_or_else(|| {
                                    e.path.rsplit('/').next().unwrap_or_default().to_owned()
                                })
                        })
                        .unwrap_or_default()
                })),
                text().class("nginx-path").selectable(true).content(state.map(|s| {
                    s.nginx
                        .editor
                        .as_ref()
                        .map(|e| {
                            if e.target.is_empty() || e.target == e.path {
                                e.path.clone()
                            } else {
                                i18n::translate(&format!(
                                    "{} (on the server: {})",
                                    e.path, e.target
                                ))
                            }
                        })
                        .unwrap_or_default()
                })),
            )),
            row().class("nginx-segments").fill((
                sapid::button()
                    .class("nginx-segment")
                    .label(i18n::bind("Settings"))
                    .pressed(state.map(move |s| !file(s)))
                    .enabled(state.map(|s| s.nginx.editor.as_ref().is_some_and(|e| has_form(&e.site, e.main.as_ref()) || !e.file_view)))
                    .on_press(move || form_view.nginx_file_view(false)),
                sapid::button()
                    .class("nginx-segment")
                    .label(i18n::bind("File"))
                    .pressed(state.map(file))
                    .on_press(move || file_view.nginx_file_view(true)),
            )),
        )),
        text()
            .class(state.map(|s| {
                if s.nginx.editor.as_ref().is_some_and(|e| (!e.error.is_empty() && e.error_at.is_none()) || !e.failure.is_empty()) {
                    "nginx-error"
                } else {
                    "hidden"
                }
                .to_owned()
            }))
            .selectable(true)
            .content(state.map(|s| {
                s.nginx
                    .editor
                    .as_ref()
                    .map(|e| if e.error.is_empty() || e.error_at.is_some() { e.failure.clone() } else { e.error.clone() })
                    .unwrap_or_default()
            })),
        text()
            .class(state.map(|s| {
                if s.nginx.editor.as_ref().is_some_and(|e| !e.info.is_empty()) { "nginx-info" } else { "hidden" }.to_owned()
            }))
            .content(state.map(|s| s.nginx.editor.as_ref().map(|e| e.info.clone()).unwrap_or_default())),
        text()
            .class(state.map(|s| {
                if s.nginx.editor.as_ref().is_some_and(|e| e.target.is_empty()) {
                    "hint nginx-read-only"
                } else {
                    "hidden"
                }
                .to_owned()
            }))
            .content(i18n::bind(
                "This file is inside the container image, so it can only be read. Mount it from the server to edit it.",
            )),
        when(state.map(move |s| !file(s)), {
            let ui = ui.clone();
            move || form::settings_form(ui.clone())
        }),
        when(state.map(file), move || {
            input()
                .multiline(true)
                .class("nginx-file")
                .name(i18n::bind("File contents"))
                .enabled(state.map(writable))
                .value(state.map(|s| {
                    s.nginx
                        .editor
                        .as_ref()
                        .map(|e| e.text.clone())
                        .unwrap_or_default()
                }))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| {
                            if let Some(editor) = &mut s.nginx.editor {
                                editor.text = value;
                                editor.error.clear();
                            }
                        });
                    }
                })
        }),
        col()
            .class(state.map(|s| if writable(s) { "nginx-publish" } else { "hidden" }.to_owned()))
            .fill((
            field(
                "After reloading, check this address",
                input()
                    .class("nginx-health")
                    .name(i18n::bind("After reloading, check this address"))
                    .value(state.map(|s| {
                        s.nginx
                            .editor
                            .as_ref()
                            .map(|e| e.health.clone())
                            .unwrap_or_default()
                    }))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event {
                            state.revise(|s| {
                                if let Some(editor) = &mut s.nginx.editor {
                                    editor.health = value;
                                }
                            });
                        }
                    }),
            ),
            text().class("hint").content(i18n::bind(
                "Arcnix checks the complete configuration before replacing the file. After the reload, it asks this server's Nginx for the address. If there is no answer, or only server errors (5xx), for 30 seconds, the previous file is put back. Leave it empty to check only that Nginx keeps running.",
            )),
            text()
                .class(state.map(|s| {
                    if s.nginx.editor.as_ref().is_some_and(|e| !e.error.is_empty()) { "error" } else { "hidden" }.to_owned()
                }))
                .content(state.map(|s| s.nginx.editor.as_ref().map(|e| e.error.clone()).unwrap_or_default())),
            row().class("nginx-publish-row").fill((
                text().class("nginx-change subtle").content(i18n::bind(state.map(|s| {
                    match &s.nginx.editor {
                        Some(e) if e.create => "New file. Nothing changes on the server until you publish.",
                        Some(e) if changed(e) => "Changed. The live file is unchanged until you publish.",
                        Some(_) => "No changes",
                        None => "",
                    }
                    .to_owned()
                }))),
                sapid::button()
                    .class("button")
                    .label(i18n::bind("Discard"))
                    .enabled(state.map(|s| s.nginx.editor.as_ref().is_some_and(changed)))
                    .on_press(move || {
                        discard.state.revise(|s| {
                            if s.nginx.editor.as_ref().is_some_and(|e| e.create) {
                                s.nginx.editor = None;
                            } else if let Some(editor) = &mut s.nginx.editor {
                                editor.text = editor.original.clone();
                                (editor.site, editor.main) = editor.read(&editor.text).unwrap_or_default();
                                editor.drafts.clear();
                                editor.error.clear();
                                editor.failure.clear();
                            }
                        })
                    }),
                sapid::button()
                    .class("button primary")
                    .label(i18n::bind("Check and publish…"))
                    .enabled(state.map(can_publish))
                    .on_press(move || publish.nginx_publish()),
            )),
        )),
    ))
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let content = col().class("nginx-main").fill((
        text()
            .class(state.map(|s| {
                if s.nginx.error.is_empty() && s.nginx.notice.is_empty() {
                    "hidden"
                } else {
                    "nginx-error"
                }
                .to_owned()
            }))
            .selectable(true)
            .content(state.map(|s| {
                if s.nginx.error.is_empty() {
                    s.nginx.notice.clone()
                } else {
                    s.nginx.error.clone()
                }
            })),
        when(
            state.map(|s| s.nginx.snapshot.is_none() && s.nginx.pending.is_some()),
            || text().class("subtle").content(i18n::bind("Loading…")),
        ),
        when(state.map(editing), {
            let ui = ui.clone();
            move || editor(ui.clone())
        }),
        when(
            state.map(|s| !editing(s) && s.nginx.view == View::Overview),
            {
                let ui = ui.clone();
                move || overview(ui.clone())
            },
        ),
        when(state.map(|s| !editing(s) && s.nginx.view == View::Logs), {
            let ui = ui.clone();
            move || logs::view(ui.clone())
        }),
        when(state.map(|s| !editing(s) && s.nginx.view == View::Sites), {
            let ui = ui.clone();
            move || sites(ui.clone())
        }),
    ));
    row().class("nginx-page").fill((navigation(ui), content))
}

fn kind_button(ui: &Ui, label: &'static str, kind: Kind) -> impl Widget + use<> {
    let state = ui.state;
    sapid::button()
        .class("nginx-segment")
        .label(i18n::bind(label))
        .pressed(state.map(move |s| s.nginx.new_site.kind == kind))
        .on_press(move || {
            state.revise(|s| {
                s.nginx.new_site.kind = kind;
                s.dialog_error.clear();
            })
        })
}

pub(crate) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::NginxNewSite)),
        move || {
            let create = ui.clone();
            modal(
            ui.clone(),
            "New site",
            col().class("nginx-new-site").fill((
                row().class("nginx-segments").fill((
                    kind_button(&ui, "Reverse proxy", Kind::Proxy),
                    kind_button(&ui, "Static site", Kind::Folder),
                    kind_button(&ui, "Redirect", Kind::Redirect),
                )),
                field(
                    "Domains",
                    input()
                        .name(i18n::bind("Domains"))
                        .value(state.map(|s| s.nginx.new_site.names.clone()))
                        .on(value_changed, move |event| {
                            if let Event::ValueChanged { value, .. } = event {
                                state.revise(|s| s.nginx.new_site.names = value);
                            }
                        }),
                ),
                text().class("hint").content(i18n::bind(
                    "Separate several domains with spaces, for example example.com www.example.com.",
                )),
                col().class("field").fill((
                    text().class("field-label").content(i18n::bind(state.map(|s| {
                        match s.nginx.new_site.kind {
                            Kind::Proxy => "Forward to",
                            Kind::Folder => "Folder",
                            Kind::Redirect => "Redirect to",
                        }
                        .to_owned()
                    }))),
                    input()
                        .name(i18n::bind("Target"))
                        .value(state.map(|s| s.nginx.new_site.target.clone()))
                        .on(value_changed, move |event| {
                            if let Event::ValueChanged { value, .. } = event {
                                state.revise(|s| s.nginx.new_site.target = value);
                            }
                        }),
                )),
                text().class("hint").content(i18n::bind(state.map(|s| {
                    match s.nginx.new_site.kind {
                        Kind::Proxy if s.nginx.snapshot.as_ref().is_some_and(|n| n.docker.is_some()) => "The address of the application as the container sees it, for example http://app:3000 for a Compose service.",
                        Kind::Proxy => "The address of the application on this server, for example http://127.0.0.1:3000.",
                        Kind::Folder => "The folder with the site's files, for example /var/www/example.",
                        Kind::Redirect => "Visitors are sent to this address with the same path, for example https://example.com.",
                    }
                    .to_owned()
                }))),
                text().class("nginx-path").content(state.map(|s| {
                    s.nginx
                        .snapshot
                        .as_ref()
                        .and_then(|n| nginx::new_site_path(n, &s.nginx.new_site.names).ok())
                        .map(|(path, target)| {
                            if path == target {
                                path
                            } else {
                                i18n::translate(&format!("{path} (on the server: {target})"))
                            }
                        })
                        .unwrap_or_default()
                })),
                text().class("hint").content(i18n::bind(state.map(|s| {
                    match s.nginx.snapshot.as_ref().map(|n| n.layout) {
                        Some(Layout::Debian) => "New sites start off. Review the file, publish it, then turn the site on in Sites. Add HTTPS afterwards, for example with Certbot.",
                        _ => "Review the file and publish it; the site is on as soon as it is published. Add HTTPS afterwards, for example with Certbot.",
                    }
                    .to_owned()
                }))),
                text()
                    .class("error")
                    .content(state.map(|s| s.dialog_error.clone())),
            )),
            sapid::button()
                .class("button primary")
                .label(i18n::bind("Continue"))
                .enabled(state.map(can_change))
                .on_press(move || create.nginx_create_site()),
        )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_reads_naturally() {
        assert_eq!(uptime(5), "up less than a minute");
        assert_eq!(uptime(600), "up 10 min");
        assert_eq!(uptime(7200), "up 2 h");
        assert_eq!(uptime(86400 * 12), "up 12 days");
    }

    #[test]
    fn forms_appear_only_for_files_with_common_settings() {
        assert!(!has_form(&Site::default(), None));
        let site = nginx_site::read("server { server_name a.com; root /srv; }").unwrap();
        assert!(has_form(&site, None));
        let text = "events {}\nhttp { include mime.types; }\n";
        let main = nginx_site::read_main(text).unwrap();
        assert!(has_form(&nginx_site::read(text).unwrap(), main.as_ref()));
        assert!(!has_form(&nginx_site::read("events {}\n").unwrap(), None));
    }
}
