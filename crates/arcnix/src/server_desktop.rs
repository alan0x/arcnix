//! Browser-style server tabs and the desktop application shell.
use std::collections::BTreeMap;

use arcnix_core::server_apps::{DesktopApp, DesktopWindow, WindowBounds};

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Home,
    Desktop,
    Files,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Desktop => "Desktop",
            Self::Files => "Transfer files",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Self::Home => Icon::Server,
            Self::Desktop => Icon::Grid,
            Self::Files => Icon::Folder,
        }
    }
}

#[derive(Clone)]
pub(super) struct Workbench {
    pub active: Tab,
    pub tabs: Vec<Tab>,
    desktop_page: WorkspacePage,
    pub apps: Vec<WorkspacePage>,
    stack: Vec<WorkspacePage>,
    desktop_restore: Vec<WorkspacePage>,
    slots: BTreeMap<DesktopApp, usize>,
    pub windows: BTreeMap<DesktopApp, DesktopWindow>,
    pub fullscreen: bool,
}

impl Default for Workbench {
    fn default() -> Self {
        Self {
            active: Tab::Home,
            tabs: vec![Tab::Home],
            desktop_page: WorkspacePage::Desktop,
            apps: Vec::new(),
            stack: Vec::new(),
            desktop_restore: Vec::new(),
            slots: BTreeMap::new(),
            windows: BTreeMap::new(),
            fullscreen: false,
        }
    }
}

pub(super) fn app_id(page: WorkspacePage) -> Option<DesktopApp> {
    Some(match page {
        WorkspacePage::Terminal => DesktopApp::Terminal,
        WorkspacePage::Resources => DesktopApp::Monitor,
        WorkspacePage::Software => DesktopApp::Store,
        WorkspacePage::Explorer => DesktopApp::Explorer,
        WorkspacePage::Docker => DesktopApp::Docker,
        WorkspacePage::Code => DesktopApp::Code,
        WorkspacePage::Video => DesktopApp::Video,
        WorkspacePage::Browser => DesktopApp::Browser,
        WorkspacePage::Settings => DesktopApp::Settings,
        _ => return None,
    })
}

pub(super) fn app_page(app: DesktopApp) -> WorkspacePage {
    match app {
        DesktopApp::Terminal => WorkspacePage::Terminal,
        DesktopApp::Monitor | DesktopApp::Disks | DesktopApp::Services | DesktopApp::Logs => {
            WorkspacePage::Resources
        }
        DesktopApp::Store => WorkspacePage::Software,
        DesktopApp::Explorer => WorkspacePage::Explorer,
        DesktopApp::Docker => WorkspacePage::Docker,
        DesktopApp::Editor => WorkspacePage::Code,
        DesktopApp::Code => WorkspacePage::Code,
        DesktopApp::Video => WorkspacePage::Video,
        DesktopApp::Browser => WorkspacePage::Browser,
        DesktopApp::Settings => WorkspacePage::Settings,
    }
}

impl Workbench {
    pub(super) fn window(&self, page: WorkspacePage) -> DesktopWindow {
        app_id(page)
            .and_then(|app| self.windows.get(&app).copied())
            .unwrap_or_default()
    }

    fn open_window(&mut self, page: WorkspacePage, app: DesktopApp) {
        if !self.apps.contains(&page) {
            self.apps.push(page);
            let slot = (0..)
                .find(|slot| !self.slots.values().any(|saved| saved == slot))
                .unwrap();
            self.slots.insert(app, slot);
        }
        self.desktop_restore.clear();
        self.stack.retain(|&open| open != page);
        self.stack.push(page);
    }

    pub(super) fn window_visible(&self, page: WorkspacePage) -> bool {
        self.stack.contains(&page)
    }

    pub(super) fn reset_open_windows(&mut self) {
        self.apps.clear();
        self.stack.clear();
        self.desktop_restore.clear();
        self.slots.clear();
        self.desktop_page = WorkspacePage::Desktop;
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundsEvent {
    epoch: u64,
    app: DesktopApp,
    bounds: WindowBounds,
}

fn save_bounds(ui: &Ui, input: &str) {
    let Ok(event) = serde_json::from_str::<BoundsEvent>(input) else {
        return;
    };
    if event.bounds.validate().is_err() {
        return;
    }
    let changed = ui.state.revise(|s| {
        if !visible(s)
            || s.dialog.is_some()
            || s.epoch != event.epoch
            || !s.workbench.window_visible(app_page(event.app))
            || app_id(app_page(event.app)) != Some(event.app)
        {
            return false;
        }
        let window = s.workbench.windows.entry(event.app).or_default();
        if window.maximized || window.bounds == Some(event.bounds) {
            return false;
        }
        window.bounds = Some(event.bounds);
        true
    });
    if changed {
        ui.remember_desktop_layout();
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FocusEvent {
    epoch: u64,
    app: DesktopApp,
}

fn focus_window(ui: &Ui, input: &str) {
    let Ok(event) = serde_json::from_str::<FocusEvent>(input) else {
        return;
    };
    let page = app_page(event.app);
    if ui.state.with(|s| {
        visible(s)
            && s.epoch == event.epoch
            && s.dialog.is_none()
            && s.fingerprint.is_none()
            && page != s.workspace_page
            && s.workbench.window_visible(page)
            && app_id(page) == Some(event.app)
    }) {
        ui.open_desktop_app(page);
    }
}

const APPS: [WorkspacePage; 6] = [
    WorkspacePage::Terminal,
    WorkspacePage::Resources,
    WorkspacePage::Software,
    WorkspacePage::Explorer,
    WorkspacePage::Code,
    WorkspacePage::Settings,
];

pub(super) fn available_apps(s: &State) -> Vec<WorkspacePage> {
    let mut apps = APPS.to_vec();
    #[cfg(windows)]
    apps.insert(APPS.len() - 1, WorkspacePage::Browser);
    if s.server_apps.applications.docker {
        apps.push(WorkspacePage::Docker);
    }
    apps
}
fn app_label(page: WorkspacePage) -> &'static str {
    match page {
        WorkspacePage::Docker => "Docker",
        WorkspacePage::Explorer => "File explorer",
        WorkspacePage::Code => "Code",
        WorkspacePage::Video => "Video player",
        WorkspacePage::Browser => "Web browser",
        WorkspacePage::Terminal => "终端",
        WorkspacePage::Resources => "Monitor",
        WorkspacePage::Software => "Software store",
        WorkspacePage::Settings => "Settings",
        _ => "Desktop",
    }
}

fn app_class(page: WorkspacePage) -> &'static str {
    match page {
        WorkspacePage::Docker => "app-docker",
        WorkspacePage::Explorer => "app-explorer",
        WorkspacePage::Code => "app-code",
        WorkspacePage::Video => "app-video",
        WorkspacePage::Browser => "app-browser",
        WorkspacePage::Terminal => "app-terminal",
        WorkspacePage::Resources => "app-resources",
        WorkspacePage::Software => "app-software",
        WorkspacePage::Settings => "app-settings",
        _ => "app-desktop",
    }
}

fn app_icon(page: WorkspacePage) -> Icon {
    match page {
        WorkspacePage::Terminal => Icon::Terminal,
        WorkspacePage::Resources => Icon::Monitor,
        WorkspacePage::Software => Icon::Store,
        WorkspacePage::Settings => Icon::Settings,
        WorkspacePage::Explorer => Icon::Folder,
        WorkspacePage::Code => Icon::Code,
        WorkspacePage::Video => Icon::Play,
        WorkspacePage::Browser => Icon::Browser,
        WorkspacePage::Docker => Icon::Docker,
        _ => Icon::Grid,
    }
}

pub(super) fn select_tab(s: &mut State, tab: Tab) {
    if tab != Tab::Desktop {
        s.workbench.fullscreen = false;
    }
    observability::metrics_leave(s);
    s.terminal.menu = None;
    s.file_context = None;
    s.file_menu = None;
    s.file_browser.cancel_drag();
    if s.workbench.active == Tab::Desktop {
        s.workbench.desktop_page = s.workspace_page;
    }
    if !s.workbench.tabs.contains(&tab) {
        s.workbench.tabs.push(tab);
    }
    s.workbench.active = tab;
    s.workspace_page = match tab {
        Tab::Home => WorkspacePage::Desktop,
        Tab::Desktop => s.workbench.desktop_page,
        Tab::Files => WorkspacePage::Files,
    };
}

pub(super) fn visible(s: &State) -> bool {
    s.navigation == navigation::Page::Servers && s.connected && s.workbench.active == Tab::Desktop
}

pub(super) fn fullscreen(s: &State) -> bool {
    s.workbench.fullscreen && visible(s)
}

pub(super) fn owns_dialog(s: &State) -> bool {
    #[cfg(windows)]
    if s.dialog == Some(Dialog::BrowserForwardingRepair) {
        return visible(s);
    }
    visible(s)
        && matches!(
            s.dialog,
            Some(
                Dialog::SoftwareInstall
                    | Dialog::ConfigureNetwork
                    | Dialog::StartupService
                    | Dialog::ScriptRun
                    | Dialog::DockerAction
                    | Dialog::DockerConfigure
                    | Dialog::TerminalRename { .. }
                    | Dialog::TerminatePortProcess
                    | Dialog::ExplorerColor { .. }
                    | Dialog::File { remote: true, .. }
                    | Dialog::FileProperties
                    | Dialog::FileColumns { remote: true }
                    | Dialog::DeleteFiles { remote: true, .. }
                    | Dialog::TransferProgress(_)
                    | Dialog::TransferDefaults
                    | Dialog::TransferConflict
            )
        )
}

/// Also used by Terminal here and service-log links so they open the right tab.
pub(super) fn select_app(s: &mut State, page: WorkspacePage) {
    let Some(app) = app_id(page) else {
        return;
    };
    select_tab(s, Tab::Desktop);
    s.workbench.open_window(page, app);
    s.workspace_page = page;
    s.workbench.desktop_page = page;
}

fn front_window(s: &mut State) {
    let page = s.workbench.stack.last().copied().unwrap_or_default();
    if page == s.workbench.desktop_page {
        return;
    }
    s.workbench.desktop_page = page;
    if s.workbench.active == Tab::Desktop {
        observability::metrics_leave(s);
        s.terminal.menu = None;
        s.file_context = None;
        s.file_menu = None;
        s.file_browser.cancel_drag();
        s.workspace_page = page;
    }
}

pub(super) fn remove_window(s: &mut State, page: WorkspacePage) {
    s.workbench.apps.retain(|&open| open != page);
    s.workbench.stack.retain(|&open| open != page);
    s.workbench.desktop_restore.retain(|&open| open != page);
    if let Some(app) = app_id(page) {
        s.workbench.slots.remove(&app);
    }
    front_window(s);
}

impl Ui {
    fn toggle_show_desktop(&self) {
        let hiding = self.state.revise(|s| {
            let workbench = &mut s.workbench;
            let hiding = !workbench.stack.is_empty();
            if hiding {
                workbench.desktop_restore = std::mem::take(&mut workbench.stack);
            } else {
                workbench.stack = std::mem::take(&mut workbench.desktop_restore);
            }
            front_window(s);
            hiding
        });
        self.desktop_enter();
        if hiding {
            self.refresh_desktop_items(true);
        }
    }

    pub(super) fn toggle_desktop_fullscreen(&self) {
        let changed_tab = self.state.with(|s| s.workbench.active != Tab::Desktop);
        self.state.revise(|s| {
            if s.navigation != navigation::Page::Servers || !s.connected {
                return;
            }
            let was_fullscreen = fullscreen(s);
            if s.workbench.active != Tab::Desktop {
                select_tab(s, Tab::Desktop);
            }
            s.workbench.fullscreen = !was_fullscreen;
        });
        if changed_tab {
            self.desktop_enter();
        }
    }

    /// Opens a workspace from an explicit entry point, connecting if needed.
    pub(super) fn open_workbench(&self, tab: Tab) {
        self.enter_workbench(tab, true);
    }

    /// Tab buttons only switch views. While disconnected they show the
    /// reconnect page, and open the credential dialog only when the server
    /// needs one, so switching tabs never connects by itself.
    fn switch_workbench(&self, tab: Tab) {
        self.enter_workbench(tab, false);
    }

    fn enter_workbench(&self, tab: Tab, connect: bool) {
        if self.state.with(|s| s.workbench.active != tab) {
            self.state.revise(|s| select_tab(s, tab));
        }
        if self
            .state
            .with(|s| !s.connected && !s.connecting && tab != Tab::Home)
        {
            if connect {
                self.request_connect();
            } else if self.state.with(connect_prompt_needed) {
                self.open_dialog(Dialog::Connect);
            }
        }
        self.desktop_enter();
    }

    fn desktop_enter(&self) {
        self.close_desktop_menu();
        if self.state.with(visible) {
            self.refresh_desktop_items(false);
            self.load_desktop_wallpaper();
        }
        self.docker_enter();
        self.terminal_enter();
        self.observability_enter();
        self.store_enter();
        self.explorer_enter();
        network_ui::enter(self);
        if self.state.with(|s| {
            navigation::workspace_visible(s)
                && s.workspace_page == WorkspacePage::Software
                && s.software.environment.is_none()
                && s.software.pending.is_none()
        }) {
            self.software_detect();
        }
    }

    pub(super) fn open_desktop_app(&self, page: WorkspacePage) {
        self.state.revise(|s| select_app(s, page));
        #[cfg(windows)]
        if page == WorkspacePage::Browser {
            self.open_web_browser();
        }
        self.desktop_enter();
    }

    pub(super) fn cycle_desktop_apps(&self, backwards: bool) {
        let next = self.state.with(|s| {
            let apps = &s.workbench.apps;
            if apps.is_empty() {
                return None;
            }
            let current = apps.iter().position(|&app| app == s.workspace_page);
            let index = match (current, backwards) {
                (Some(index), true) => (index + apps.len() - 1) % apps.len(),
                (Some(index), false) => (index + 1) % apps.len(),
                (None, true) => apps.len() - 1,
                (None, false) => 0,
            };
            (apps[index] != s.workspace_page).then_some(apps[index])
        });
        if let Some(page) = next {
            self.open_desktop_app(page);
        }
    }

    pub(super) fn minimize_desktop_window(&self) {
        self.minimize_window(self.state.with(|s| s.workspace_page));
    }

    fn minimize_window(&self, page: WorkspacePage) {
        let changed = self.state.revise(|s| {
            let previous = s.workspace_page;
            s.workbench.stack.retain(|&open| open != page);
            front_window(s);
            s.workspace_page != previous
        });
        if changed {
            self.desktop_enter();
        }
    }

    pub(super) fn toggle_desktop_maximize(&self) {
        self.toggle_window_maximize(self.state.with(|s| s.workspace_page));
    }

    fn toggle_window_maximize(&self, page: WorkspacePage) {
        let Some(app) = app_id(page) else {
            return;
        };
        let changed = self.state.revise(|s| {
            if !s.workbench.window_visible(page) {
                return false;
            }
            let window = s.workbench.windows.entry(app).or_default();
            window.maximized = !window.maximized;
            true
        });
        if changed {
            self.remember_desktop_layout();
        }
    }

    fn close_desktop_app(&self, page: WorkspacePage) {
        if page == WorkspacePage::Code && !code_workbench::close(self) {
            return;
        }
        self.finish_desktop_close(page);
    }

    pub(super) fn finish_desktop_close(&self, page: WorkspacePage) {
        if page == WorkspacePage::Code {
            crate::code_terminal::close_all(self);
            self.state.revise(|s| s.code = Default::default());
        }
        if page == WorkspacePage::Video {
            self.player.stop(self);
        }
        #[cfg(windows)]
        if page == WorkspacePage::Browser {
            self.close_web_browser();
        }
        if page == WorkspacePage::Terminal {
            self.terminal_close_all();
        }
        let changed = self.state.revise(|s| {
            let previous = s.workspace_page;
            remove_window(s, page);
            previous != s.workspace_page
        });
        if changed {
            self.desktop_enter();
        }
    }
}

pub(super) fn tabs(ui: Ui) -> impl Widget {
    let state = ui.state;
    row().class("workbench-tabs").fill(
        each(state.map(|s| s.workbench.tabs.clone()))
            .key(|tab| *tab as u8)
            .children(move |item| {
                let tab = item.value.get();
                let choose = ui.clone();
                let close = ui.clone();
                let fullscreen_ui = ui.clone();
                row()
                    .class(state.map(move |s| {
                        if s.workbench.active == tab {
                            "workbench-tab active"
                        } else {
                            "workbench-tab"
                        }
                        .to_owned()
                    }))
                    .fill((
                        sapid::button()
                            .class("workbench-tab-select")
                            .name(i18n::bind(tab.label()))
                            .pressed(state.map(move |s| s.workbench.active == tab))
                            .fill((
                                icons::glyph(tab.icon()),
                                text().content(i18n::bind(tab.label())),
                            ))
                            .on_press(move || choose.switch_workbench(tab)),
                        when(tab == Tab::Desktop, move || {
                            fullscreen_action(fullscreen_ui.clone(), false)
                        }),
                        when(tab != Tab::Home, move || {
                            close_action(match tab {
                                Tab::Desktop => "Close Desktop tab",
                                _ => "Close Transfer files tab",
                            })
                            .class("icon-button workbench-tab-close")
                            .on_press({
                                let close = close.clone();
                                move || {
                                    close.state.revise(|s| {
                                        if s.workbench.active == tab {
                                            select_tab(s, Tab::Home);
                                        }
                                        s.workbench.tabs.retain(|open| *open != tab);
                                    });
                                }
                            })
                        }),
                    ))
            }),
    )
}

fn fullscreen_action(ui: Ui, exit: bool) -> impl Widget {
    let state = ui.state;
    let label = if exit {
        "Exit desktop fullscreen"
    } else {
        "Fullscreen desktop"
    };
    sapid::button()
        .name(i18n::bind(label))
        .fill(icons::glyph(if exit {
            Icon::ExitFullscreen
        } else {
            Icon::Fullscreen
        }))
        .class(if exit {
            "icon-button desktop-fullscreen-exit"
        } else {
            "icon-button desktop-fullscreen-toggle"
        })
        .enabled(state.map(|s| s.connected))
        .on_press(move || ui.toggle_desktop_fullscreen())
}

pub(super) fn entry(ui: Ui, tab: Tab) -> impl Widget {
    let description = match tab {
        Tab::Desktop => "终端、资源、软件与服务，尽在服务器桌面。",
        _ => "在本地与服务器之间浏览、上传和下载文件。",
    };
    sapid::button()
        .class(if tab == Tab::Desktop {
            "workspace-entry overview-connect entry-desktop"
        } else {
            "workspace-entry overview-transfer entry-files"
        })
        .name(i18n::bind(tab.label()))
        .fill((
            icons::glyph(tab.icon()),
            text().class("entry-title").content(i18n::bind(tab.label())),
            text()
                .class("entry-description")
                .content(i18n::bind(description)),
            icons::glyph(Icon::Go),
        ))
        .on_press(move || ui.open_workbench(tab))
}

pub(super) fn launcher(ui: Ui, page: WorkspacePage, dock: bool) -> impl Widget {
    let state = ui.state;
    let label = if page == WorkspacePage::Video && !dock {
        "Player"
    } else {
        app_label(page)
    };
    sapid::button()
        .class(state.map(move |s| {
            format!(
                "{} {}{}{}",
                if dock {
                    "desktop-dock-app"
                } else {
                    "desktop-shortcut"
                },
                app_class(page),
                if s.workbench.apps.contains(&page) {
                    " running"
                } else {
                    ""
                },
                if s.workspace_page == page {
                    " active"
                } else {
                    ""
                }
            )
        }))
        .name(i18n::bind(label))
        .pressed(state.map(move |s| s.workspace_page == page))
        .fill((
            text().class("desktop-app-icon").content(""),
            text().class("desktop-app-label").content(i18n::bind(label)),
        ))
        .on_press(move || ui.open_desktop_app(page))
}

fn app_window(ui: Ui, page: WorkspacePage) -> impl Widget {
    let state = ui.state;
    let minimize = ui.clone();
    let maximize = ui.clone();
    let close = ui.clone();
    let title = if page == WorkspacePage::Video {
        text().content(state.map(|s| {
            s.video
                .as_ref()
                .map(|v| v.title.clone())
                .unwrap_or_else(|| i18n::translate("Player"))
        }))
    } else {
        text().content(i18n::bind(app_label(page)))
    };
    let tools = match page {
        WorkspacePage::Terminal => fragment(terminal::header(ui.clone())),
        WorkspacePage::Code => fragment(code_workbench::title_tools(ui.clone())),
        WorkspacePage::Video => {
            let browse = ui.clone();
            fragment(when(state.map(|s| s.video.is_some()), move || {
                let browse = browse.clone();
                icon_action("Open video", Icon::Folder)
                    .on_press(move || browse.player.browse(&browse))
            }))
        }
        WorkspacePage::Explorer => fragment(workspace::explorer_window_tools(ui.clone())),
        WorkspacePage::Resources => fragment(monitor_ui::window_tools(ui.clone())),
        #[cfg(windows)]
        WorkspacePage::Browser => fragment(web_browser::address_bar(ui.clone())),
        _ => fragment(()),
    };
    let content = match page {
        WorkspacePage::Terminal => fragment(terminal::page(ui)),
        WorkspacePage::Explorer => fragment((
            file_context::desktop_menu_dismiss(ui.clone()),
            workspace::explorer(ui),
        )),
        WorkspacePage::Resources => fragment(monitor_ui::page(ui)),
        WorkspacePage::Software => fragment(software::page(ui)),
        WorkspacePage::Settings => fragment(server_apps_ui::settings(ui)),
        WorkspacePage::Docker => fragment(docker_ui::page(ui)),
        WorkspacePage::Code => fragment(code_workbench::page(ui)),
        WorkspacePage::Video => fragment(video_player::panel(ui)),
        #[cfg(windows)]
        WorkspacePage::Browser => fragment(web_browser::page(ui)),
        _ => fragment(()),
    };
    col()
        .class(state.map(move |s| {
            let mut class = format!("desktop-window {}", app_class(page));
            if !s.workbench.window_visible(page) {
                class.push_str(" hidden");
            } else {
                if s.workbench.desktop_page == page {
                    class.push_str(" active");
                }
                if s.workbench.window(page).maximized {
                    class.push_str(" maximized");
                }
            }
            class
        }))
        .fill((
            row()
                .class(if page == WorkspacePage::Explorer {
                    "desktop-window-titlebar explorer-titlebar"
                } else {
                    "desktop-window-titlebar"
                })
                .fill((
                    row()
                        .class("desktop-window-mark")
                        .fill(icons::glyph(app_icon(page))),
                    title.class("desktop-window-title"),
                    tools,
                    row().class("desktop-window-actions").fill((
                        icon_action("Minimize window", Icon::WindowMinimize)
                            .class("icon-button window-minimize")
                            .on_press(move || minimize.minimize_window(page)),
                        sapid::button()
                            .name(i18n::bind("Toggle maximize"))
                            .class("icon-button window-maximize")
                            .pressed(state.map(move |s| s.workbench.window(page).maximized))
                            .fill((
                                icons::glyph(Icon::WindowMaximize),
                                icons::glyph(Icon::WindowRestore),
                            ))
                            .on_press(move || maximize.toggle_window_maximize(page)),
                        close_action("Close application")
                            .class("icon-button window-close")
                            .enabled(state.map(move |s| {
                                !s.system_task_running
                                    && (page != WorkspacePage::Code || s.code.pending.is_empty())
                            }))
                            .on_press(move || close.close_desktop_app(page)),
                    )),
                )),
            col().class("desktop-window-content").fill(content),
            fragment(
                ["n", "s", "e", "w", "ne", "nw", "se", "sw"]
                    .into_iter()
                    .map(|edge| view().class(format!("desktop-resize-handle edge-{edge}")))
                    .collect::<Vec<_>>(),
            ),
        ))
}

fn app_windows(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("desktop-window-layer").fill(
        each(state.map(|s| {
            // The terminal bridge stays mounted until its session metadata is cleared.
            std::iter::once(WorkspacePage::Terminal)
                .chain(
                    s.workbench
                        .apps
                        .iter()
                        .copied()
                        .filter(|&page| page != WorkspacePage::Terminal),
                )
                .collect::<Vec<_>>()
        }))
        .key(|page| *page as u8)
        .children(move |item| app_window(ui.clone(), item.value.get())),
    )
}

fn selected_name(s: &State) -> String {
    s.config
        .servers
        .iter()
        .find(|p| Some(&p.id) == s.selected.as_ref())
        .map(|p| {
            if p.name.trim().is_empty() {
                &p.host
            } else {
                &p.name
            }
        })
        .cloned()
        .unwrap_or_default()
}

fn desktop_address(s: &State) -> String {
    let Some(profile) = s
        .config
        .servers
        .iter()
        .find(|p| Some(&p.id) == s.selected.as_ref())
    else {
        return String::new();
    };
    if profile.port == 22 {
        profile.host.clone()
    } else if profile.host.contains(':') {
        format!("[{}]:{}", profile.host, profile.port)
    } else {
        format!("{}:{}", profile.host, profile.port)
    }
}

fn dock_clock(state: Cage<State>) -> impl Widget {
    col()
        .class("desktop-panel-clock")
        .name(i18n::bind("Server time"))
        .fill((
            text()
                .class(state.map(|s| match s.config.settings.language {
                    arcnix_core::Language::English => "desktop-panel-locale hidden".to_owned(),
                    arcnix_core::Language::SimplifiedChinese => {
                        "desktop-panel-locale clock-zh hidden".to_owned()
                    }
                }))
                .content(""),
            text().class("desktop-panel-time").content(""),
            text().class("desktop-panel-date").content(""),
        ))
}

pub(super) fn desktop(ui: Ui) -> impl Widget {
    let state = ui.state;
    let show_desktop = ui.clone();
    let exit_fullscreen = ui.clone();
    let geometry = ui.clone();
    let focus = ui.clone();
    col()
        .class(state.map(|s| {
            if visible(s) {
                return server_apps_ui::themed(s, "server-desktop");
            } else {
                "server-desktop hidden"
            }
            .to_owned()
        }))
        .fill((
            desktop_customization_ui::gesture_input(ui.clone()),
            desktop_shortcuts_ui::bridge(ui.clone()),
            text()
                .class("desktop-clock-sample hidden")
                .content(state.map(|s| {
                    s.server_apps
                        .clock
                        .map_or_else(String::new, |(serial, clock)| {
                            format!("{serial},{},{}", clock.unix_seconds, clock.offset_minutes)
                        })
                })),
            input().class("desktop-bounds-input hidden").value("").on(
                value_changed,
                move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        save_bounds(&geometry, &value);
                    }
                },
            ),
            input()
                .class("desktop-focus-input hidden")
                .value("")
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        focus_window(&focus, &value);
                    }
                }),
            text().class("desktop-motion-state").content(state.map(|s| {
                let shown = visible(s);
                let windows: Vec<_> = s
                    .workbench
                    .stack
                    .iter()
                    .enumerate()
                    .filter_map(|(index, &page)| {
                        let app = app_id(page)?;
                        let layout = s.workbench.window(page);
                        Some(serde_json::json!({
                            "id": app,
                            "class": app_class(page),
                            "order": index + 1,
                            "slot": s.workbench.slots.get(&app).copied().unwrap_or_default(),
                            "bounds": layout.bounds,
                            "maximized": layout.maximized,
                        }))
                    })
                    .collect();
                serde_json::json!({
                    "shown": shown,
                    "epoch": s.epoch,
                    "app": if shown && s.workspace_page != WorkspacePage::Desktop {
                        app_class(s.workspace_page)
                    } else { "" },
                    "fullscreen": fullscreen(s),
                    "windows": windows,
                })
                .to_string()
            })),
            when(state.map(fullscreen), {
                let ui = ui.clone();
                move || {
                    row().class("desktop-menubar").fill((
                        text()
                            .class("desktop-menubar-title")
                            .content(state.map(selected_name)),
                        workspace::transfer_progress_buttons(ui.clone()),
                        connection_target(state),
                        text()
                            .class("desktop-secure")
                            .content(i18n::bind("SSH 已连接")),
                        fullscreen_action(exit_fullscreen.clone(), true),
                    ))
                }
            }),
            col()
                .class("desktop-canvas")
                .name(i18n::bind("Desktop"))
                .focus(FocusMode::Programmatic)
                .fill((
                    desktop_customization_ui::wallpaper(ui.clone()),
                    when(
                        state.map(|s| desktop_customization_ui::appearance(s).show_server),
                        move || {
                            col().class("desktop-wallpaper-copy").fill((
                                text()
                                    .class("desktop-wallpaper-name")
                                    .content(state.map(selected_name)),
                                text()
                                    .class("desktop-wallpaper-address")
                                    .content(state.map(|s| {
                                        let address = desktop_address(s);
                                        if address == selected_name(s) {
                                            String::new()
                                        } else {
                                            address
                                        }
                                    })),
                            ))
                        },
                    ),
                    col()
                        .class("desktop-shortcuts")
                        .fill(desktop_customization_ui::launchers(ui.clone(), false)),
                    app_windows(ui.clone()),
                    col()
                        .class(state.map(|s| {
                            if desktop_customization_ui::panel_mode(s) {
                                "desktop-dock-zone dock-panel-zone"
                            } else {
                                "desktop-dock-zone"
                            }
                            .to_owned()
                        }))
                        .fill(
                            row()
                                .class(state.map(desktop_customization_ui::dock_class))
                                .fill((
                                    row().class("desktop-dock-leading").fill((
                                        sapid::button()
                                            .class("desktop-dock-home")
                                            .name(i18n::bind("Show desktop"))
                                            .pressed(
                                                state.map(|s| {
                                                    !s.workbench.desktop_restore.is_empty()
                                                }),
                                            )
                                            .fill(icons::glyph(Icon::Grid))
                                            .on_press(move || show_desktop.toggle_show_desktop()),
                                        text().class("desktop-dock-divider").content(""),
                                    )),
                                    row().class("desktop-dock-apps").fill((
                                        desktop_customization_ui::launchers(ui.clone(), true),
                                        when(
                                            state.map(|s| {
                                                s.workbench.apps.contains(&WorkspacePage::Video)
                                            }),
                                            {
                                                let ui = ui.clone();
                                                move || {
                                                    launcher(ui.clone(), WorkspacePage::Video, true)
                                                }
                                            },
                                        ),
                                    )),
                                    when(
                                        state.map(desktop_customization_ui::panel_mode),
                                        move || dock_clock(state),
                                    ),
                                    when(
                                        state.map(desktop_customization_ui::floating_dock_status),
                                        move || {
                                            row()
                                                .class("desktop-dock-status")
                                                .fill(dock_clock(state))
                                        },
                                    ),
                                )),
                        ),
                    desktop_customization_ui::context_menu(ui),
                )),
        ))
}

pub(super) fn reconnect(ui: Ui) -> impl Widget {
    let state = ui.state;
    let connect = ui.clone();
    col().class("workspace-reconnect").fill((
        icons::glyph(Icon::Server),
        text().class("page-title").content(i18n::bind(
            state.map(|s| s.workbench.active.label().to_owned()),
        )),
        text()
            .class("home-description")
            .content(i18n::bind(state.map(|s| {
                if s.connecting {
                    "正在建立安全连接…"
                } else {
                    "连接服务器以打开此工作区。"
                }
                .to_owned()
            }))),
        btn("连接", move || connect.request_connect())
            .class("button primary")
            .enabled(state.map(|s| !s.connecting)),
        btn("Home", move || ui.open_workbench(Tab::Home)).class("button quiet"),
    ))
}
