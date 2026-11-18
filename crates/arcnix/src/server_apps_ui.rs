use arcnix_core::server_apps::{Applications, DesktopApp, DesktopPreferences, DesktopWindow};

use super::*;
use crate::server_apps_worker::{Action as AppAction, Reply};

#[derive(Clone, Default)]
pub(super) struct DesktopState {
    pub applications: Applications,
    pub discover_pending: Option<u64>,
    pub discover_error: String,
    pub preferences: DesktopPreferences,
    pub preferences_loaded: bool,
    pub preferences_pending: Option<u64>,
    pub save_pending: Option<u64>,
    pub preferences_error: String,
    pub saved: bool,
    pub writing: Option<DesktopPreferences>,
    pub queued: Option<DesktopPreferences>,
    pub explorer_locations: Option<Vec<arcnix_core::remote::ExplorerLocation>>,
    pub explorer_locations_pending: Option<u64>,
    pub explorer_tag: Option<explorer_ui::TagFilter>,
    pub explorer_open_pending: Option<explorer_ui::ItemRequest>,
    pub customization: desktop_customization_ui::Runtime,
}

pub(super) fn dark(s: &State) -> bool {
    let application = s.config.settings.theme.is_dark(s.system_dark);
    if s.connected
        && s.navigation == navigation::Page::Servers
        && s.workbench.active == server_desktop::Tab::Desktop
        && !server_inventory_ui::shown(s)
    {
        match s.server_apps.preferences.theme {
            Theme::System => application,
            theme => theme.is_dark(s.system_dark),
        }
    } else {
        application
    }
}

pub(super) fn themed(s: &State, base: &str) -> String {
    if s.connected
        && s.navigation == navigation::Page::Servers
        && s.workbench.active == server_desktop::Tab::Desktop
        && !server_inventory_ui::shown(s)
    {
        format!(
            "{base} {}",
            if dark(s) {
                "theme-dark desktop-dark"
            } else {
                "theme-light desktop-light"
            }
        )
    } else {
        base.to_owned()
    }
}

impl Ui {
    pub(super) fn server_apps_connected(&self) {
        self.discover_server_apps();
        self.read_desktop_preferences();
        if self.state.with(server_desktop::visible) {
            self.refresh_desktop_items(false);
        }
    }
    pub(super) fn discover_server_apps(&self) {
        if !self
            .state
            .with(|s| s.connected && s.server_apps.discover_pending.is_none())
        {
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::Discover));
        self.state.revise(|s| {
            s.server_apps.discover_pending = Some(serial);
            s.server_apps.discover_error.clear();
        });
    }
    fn read_desktop_preferences(&self) {
        if !self.state.with(|s| {
            s.connected
                && s.server_apps.preferences_pending.is_none()
                && s.server_apps.save_pending.is_none()
        }) {
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::ReadPreferences));
        self.state.revise(|s| {
            s.server_apps.preferences_pending = Some(serial);
            s.server_apps.preferences_error.clear();
            s.server_apps.saved = false;
        });
    }
    fn save_desktop_theme(&self, theme: Theme) {
        if !self.state.with(preferences_ready) {
            return;
        }
        let mut prefs = self.state.with(desired);
        prefs.theme = theme;
        self.queue_desktop_preferences(prefs);
    }
    pub(super) fn queue_desktop_preferences(&self, prefs: DesktopPreferences) {
        if !self.state.with(|s| {
            s.connected
                && s.server_apps.preferences_loaded
                && s.server_apps.preferences_pending.is_none()
        }) {
            return;
        }
        if self.state.with(|s| s.server_apps.save_pending.is_some()) {
            self.state.revise(|s| s.server_apps.queued = Some(prefs));
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::WritePreferences(Box::new(
            prefs.clone(),
        ))));
        self.state.revise(|s| {
            s.server_apps.save_pending = Some(serial);
            s.server_apps.preferences_error.clear();
            s.server_apps.saved = false;
            s.server_apps.writing = Some(prefs);
            s.server_apps.queued = None;
        });
    }
    pub(super) fn remember_desktop_layout(&self) {
        let prefs = self.state.with(|s| {
            let mut prefs = desired(s);
            prefs.terminal = terminal::layout(s);
            prefs.windows = s.workbench.windows.clone();
            prefs
        });
        if self.state.with(|s| desired(s) != prefs) {
            self.queue_desktop_preferences(prefs);
        }
    }
}
pub(super) fn desired(s: &State) -> DesktopPreferences {
    current(s).clone()
}
pub(super) fn current(s: &State) -> &DesktopPreferences {
    s.server_apps
        .queued
        .as_ref()
        .or(s.server_apps.writing.as_ref())
        .unwrap_or(&s.server_apps.preferences)
}
pub(super) fn preferences_ready(s: &State) -> bool {
    s.connected
        && s.server_apps.preferences_loaded
        && s.server_apps.preferences_pending.is_none()
        && s.server_apps.save_pending.is_none()
}
pub(super) fn event(ui: &Ui, serial: u64, result: std::result::Result<Reply, String>) {
    if desktop_customization_ui::pending(ui, serial) {
        desktop_customization_ui::event(ui, serial, result);
        return;
    }
    if ui.state.with(|s| {
        s.server_apps
            .explorer_open_pending
            .as_ref()
            .is_some_and(|pending| pending.serial == serial)
    }) {
        explorer_ui::item_opened(ui, result);
        return;
    }
    let kind = ui.state.with(|s| {
        if s.server_apps.discover_pending == Some(serial) {
            0
        } else if s.server_apps.preferences_pending == Some(serial) {
            1
        } else if s.server_apps.save_pending == Some(serial) {
            2
        } else if s.server_apps.explorer_locations_pending == Some(serial) {
            4
        } else {
            3
        }
    });
    if kind == 3 {
        if let Some(result) = software::server_reply(ui, serial, result)
            && let Some(result) = compose_ui::event(ui, serial, result)
            && let Some(result) = disks_ui::event(ui, serial, result)
        {
            docker_ui::event(ui, serial, result);
        }
        return;
    }
    let mut queued = None;
    let mut restore = None;
    let mut save_local_geometry = false;
    let mut save_error = None;
    ui.state.revise(|s| match (kind, result) {
        (0, reply) => {
            s.server_apps.discover_pending = None;
            match reply {
                Ok(Reply::Applications(apps)) => {
                    s.server_apps.applications = apps;
                    if !s.server_apps.applications.docker {
                        s.workbench.apps.retain(|p| *p != WorkspacePage::Docker);
                        if s.workbench.desktop_page == WorkspacePage::Docker {
                            s.workbench.desktop_page = WorkspacePage::Desktop;
                            if s.workbench.active == server_desktop::Tab::Desktop {
                                s.workspace_page = WorkspacePage::Desktop;
                            }
                        }
                        s.docker = Default::default();
                    }
                }
                Err(error) => s.server_apps.discover_error = error,
                _ => s.server_apps.discover_error = "Invalid application detection response".into(),
            }
        }
        (1, reply) => {
            s.server_apps.preferences_pending = None;
            match reply {
                Ok(Reply::Preferences(prefs)) => {
                    if !s.server_apps.preferences_loaded {
                        restore = Some(prefs.terminal.clone());
                    }
                    let mut windows = prefs.windows.clone();
                    if let Some(disk_window) = windows.remove(&DesktopApp::Disks) {
                        windows.entry(DesktopApp::Monitor).or_insert(disk_window);
                    }
                    if prefs.terminal.maximized {
                        windows
                            .entry(DesktopApp::Terminal)
                            .or_insert(DesktopWindow {
                                maximized: true,
                                ..Default::default()
                            });
                    }
                    if !s.server_apps.preferences_loaded && !s.workbench.windows.is_empty() {
                        windows.append(&mut s.workbench.windows);
                        save_local_geometry = true;
                    }
                    s.workbench.windows = windows;
                    s.server_apps.preferences = prefs;
                    s.server_apps.preferences_loaded = true;
                }
                Err(error) => {
                    s.server_apps.preferences_loaded = false;
                    s.server_apps.preferences_error = error;
                }
                _ => {
                    s.server_apps.preferences_error = "Invalid desktop preferences response".into()
                }
            }
        }
        (2, reply) => {
            s.server_apps.save_pending = None;
            s.server_apps.writing = None;
            match reply {
                Ok(Reply::SavedPreferences(prefs)) => {
                    s.server_apps.preferences = prefs;
                    s.server_apps.saved = true;
                    queued = s.server_apps.queued.take();
                }
                Err(error) => {
                    s.server_apps.queued = None;
                    if s.dialog != Some(Dialog::DesktopSettings) {
                        save_error = Some(error.clone());
                    }
                    s.server_apps.preferences_error = error;
                }
                _ => {
                    s.server_apps.preferences_error = "Invalid desktop preferences response".into()
                }
            }
        }
        (4, reply) => {
            s.server_apps.explorer_locations_pending = None;
            s.server_apps.explorer_locations = Some(match reply {
                Ok(Reply::ExplorerLocations(locations)) => locations,
                _ => Vec::new(),
            });
        }
        _ => {}
    });
    if let Some(layout) = restore {
        ui.state.revise(|s| {
            terminal::restore(s, &layout);
        });
    }
    if let Some(prefs) = queued
        && ui.state.with(|s| prefs != s.server_apps.preferences)
    {
        ui.queue_desktop_preferences(prefs);
    }
    if kind == 0 {
        ui.docker_enter();
    }
    if kind <= 1 {
        ui.terminal_enter();
    }
    if matches!(kind, 1 | 2) {
        ui.load_desktop_wallpaper();
    }
    if save_local_geometry {
        ui.remember_desktop_layout();
    }
    if let Some(error) = save_error {
        ui.error(error);
    }
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    use desktop_customization_ui::{SettingsPage, settings_page};
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::DesktopSettings)),
        move || {
            let reload = ui.clone();
            let detect = ui.clone();
            let desktop = ui.clone();
            let wallpaper = ui.clone();
            let dock = ui.clone();
            modal(ui.clone(), "Desktop settings", col().class("desktop-preferences").fill((
            desktop_customization_ui::settings_tabs(ui.clone()),
            when(state.map(|s| settings_page(s) == SettingsPage::Desktop), move || {
            let reload = reload.clone();
            let detect = detect.clone();
            let ui = desktop.clone();
            col().class("desktop-settings-pane").fill((
            row().class("section-heading").fill((
                text().content(i18n::bind("Appearance")),
                help_ui::tip("Desktop settings", "Saved to ~/.arcnix/desktop.json for this SSH user. Follow application uses the client theme. Other servers and users keep their own settings."),
            )),
            row().class("desktop-theme-choices").fill(
                [(Theme::System, "Follow application"), (Theme::Light, "Light"), (Theme::Dark, "Dark")].into_iter().map({
                    let ui = ui.clone();
                    move |(theme, label)| {
                        let choose = ui.clone();
                        sapid::button().class("button desktop-theme-choice").label(i18n::bind(label))
                            .pressed(state.map(move |s| s.server_apps.preferences.theme == theme))
                            .enabled(state.map(preferences_ready)).on_press(move || choose.save_desktop_theme(theme))
                    }
                }).collect::<Vec<_>>()
            ),
            desktop_customization_ui::desktop_controls(ui.clone()),
            row().class("choice-list").fill((
                text_action("Reload settings", Icon::Refresh).enabled(state.map(|s| s.connected && s.server_apps.preferences_pending.is_none() && s.server_apps.save_pending.is_none())).on_press(move || reload.read_desktop_preferences()),
                text_action("Detect applications", Icon::Search).enabled(state.map(|s| s.connected && s.server_apps.discover_pending.is_none())).on_press(move || detect.discover_server_apps()),
            )),
            text().class("subtle").content(i18n::bind(state.map(|s| if s.server_apps.applications.docker { "Docker detected" } else { "Docker not detected" }.to_owned()))),
            text().class("error").content(state.map(|s| s.server_apps.discover_error.clone())),
            ))
            }),
            when(state.map(|s| settings_page(s) == SettingsPage::Wallpaper), move || desktop_customization_ui::wallpaper_controls(wallpaper.clone())),
            when(state.map(|s| settings_page(s) == SettingsPage::Dock), move || desktop_customization_ui::dock_controls(dock.clone())),
            text().class("subtle").content(i18n::bind(state.map(|s| {
                if s.server_apps.save_pending.is_some() { "Saving…" }
                else if s.server_apps.preferences_pending.is_some() { "Loading…" }
                else if s.server_apps.saved { "Saved on server" } else { "" }.to_owned()
            }))),
            text().class("error").content(state.map(|s| s.server_apps.preferences_error.clone())),
        )), ())
        },
    )
}
