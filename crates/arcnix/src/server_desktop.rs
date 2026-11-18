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
    pub desktop_page: WorkspacePage,
    pub apps: Vec<WorkspacePage>,
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
        WorkspacePage::Operations => DesktopApp::Services,
        WorkspacePage::SystemLogs => DesktopApp::Logs,
        WorkspacePage::Explorer => DesktopApp::Explorer,
        WorkspacePage::Docker => DesktopApp::Docker,
        WorkspacePage::Editor => DesktopApp::Editor,
        _ => return None,
    })
}

pub(super) fn app_page(app: DesktopApp) -> WorkspacePage {
    match app {
        DesktopApp::Terminal => WorkspacePage::Terminal,
        DesktopApp::Monitor | DesktopApp::Disks => WorkspacePage::Resources,
        DesktopApp::Store => WorkspacePage::Software,
        DesktopApp::Services => WorkspacePage::Operations,
        DesktopApp::Logs => WorkspacePage::SystemLogs,
        DesktopApp::Explorer => WorkspacePage::Explorer,
        DesktopApp::Docker => WorkspacePage::Docker,
        DesktopApp::Editor => WorkspacePage::Editor,
    }
}

impl Workbench {
    pub(super) fn window(&self, page: WorkspacePage) -> DesktopWindow {
        app_id(page)
            .and_then(|app| self.windows.get(&app).copied())
            .unwrap_or_default()
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
            || app_id(s.workspace_page) != Some(event.app)
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

const APPS: [WorkspacePage; 7] = [
    WorkspacePage::Terminal,
    WorkspacePage::Resources,
    WorkspacePage::Software,
    WorkspacePage::Operations,
    WorkspacePage::SystemLogs,
    WorkspacePage::Explorer,
    WorkspacePage::Editor,
];

pub(super) fn available_apps(s: &State) -> Vec<WorkspacePage> {
    let mut apps = APPS.to_vec();
    if s.server_apps.applications.docker {
        apps.push(WorkspacePage::Docker);
    }
    apps
}
fn app_label(page: WorkspacePage) -> &'static str {
    match page {
        WorkspacePage::Docker => "Docker",
        WorkspacePage::Explorer => "File explorer",
        WorkspacePage::Editor => "Editor",
        WorkspacePage::Terminal => "终端",
        WorkspacePage::Resources => "Monitor",
        WorkspacePage::Software => "Software store",
        WorkspacePage::Operations => "Services",
        WorkspacePage::SystemLogs => "系统日志",
        _ => "Desktop",
    }
}

fn app_class(page: WorkspacePage) -> &'static str {
    match page {
        WorkspacePage::Docker => "app-docker",
        WorkspacePage::Explorer => "app-explorer",
        WorkspacePage::Editor => "app-editor",
        WorkspacePage::Terminal => "app-terminal",
        WorkspacePage::Resources => "app-resources",
        WorkspacePage::Software => "app-software",
        WorkspacePage::Operations => "app-operations",
        WorkspacePage::SystemLogs => "app-logs",
        _ => "app-desktop",
    }
}

fn app_icon(page: WorkspacePage) -> Icon {
    match page {
        WorkspacePage::Terminal => Icon::Terminal,
        WorkspacePage::Resources => Icon::Monitor,
        WorkspacePage::Software => Icon::Store,
        WorkspacePage::Operations => Icon::Services,
        WorkspacePage::SystemLogs => Icon::Logs,
        WorkspacePage::Explorer => Icon::Folder,
        WorkspacePage::Editor => Icon::Edit,
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
    s.navigation == navigation::Page::Servers
        && !server_inventory_ui::shown(s)
        && s.connected
        && s.workbench.active == Tab::Desktop
}

pub(super) fn fullscreen(s: &State) -> bool {
    s.workbench.fullscreen && visible(s)
}

pub(super) fn owns_dialog(s: &State) -> bool {
    visible(s)
        && matches!(
            s.dialog,
            Some(
                Dialog::DesktopSettings
                    | Dialog::SoftwareInstall
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
                    | Dialog::TransferDefaults
                    | Dialog::TransferConflict
            )
        )
}

/// Also used by Terminal here and service-log links so they open the right tab.
pub(super) fn select_app(s: &mut State, page: WorkspacePage) {
    select_tab(s, Tab::Desktop);
    s.workspace_page = page;
    s.workbench.desktop_page = page;
    if !s.workbench.apps.contains(&page) {
        s.workbench.apps.push(page);
    }
}

impl Ui {
    fn toggle_desktop_fullscreen(&self) {
        let changed_tab = self.state.with(|s| s.workbench.active != Tab::Desktop);
        self.state.revise(|s| {
            if s.navigation != navigation::Page::Servers
                || server_inventory_ui::shown(s)
                || !s.connected
            {
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

    pub(super) fn open_workbench(&self, tab: Tab) {
        if self.state.with(|s| s.workbench.active != tab) {
            self.state.revise(|s| select_tab(s, tab));
        }
        if self
            .state
            .with(|s| !s.connected && !s.connecting && tab != Tab::Home)
        {
            self.open_dialog(Dialog::Connect);
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
        self.operations_enter();
        self.store_enter();
        self.explorer_enter();
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
        self.desktop_enter();
    }

    fn close_desktop_app(&self) {
        let page = self.state.with(|s| s.workspace_page);
        if page == WorkspacePage::Editor && !remote_editor::close_desktop(self) {
            return;
        }
        self.finish_desktop_close(page);
    }

    pub(super) fn finish_desktop_close(&self, page: WorkspacePage) {
        if page == WorkspacePage::Terminal {
            self.terminal_close_all();
        }
        self.state.revise(|s| {
            observability::metrics_leave(s);
            s.terminal.menu = None;
            s.workbench.apps.retain(|app| *app != page);
            s.workspace_page = WorkspacePage::Desktop;
            s.workbench.desktop_page = WorkspacePage::Desktop;
        });
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
                            .on_press(move || choose.open_workbench(tab)),
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
        .name(i18n::bind(app_label(page)))
        .pressed(state.map(move |s| s.workspace_page == page))
        .fill((
            text().class("desktop-app-icon").content(""),
            text()
                .class("desktop-app-label")
                .content(i18n::bind(app_label(page))),
        ))
        .on_press(move || ui.open_desktop_app(page))
}

fn app_window(ui: Ui) -> impl Widget {
    let state = ui.state;
    let minimize = ui.clone();
    let maximize = ui.clone();
    let close = ui.clone();
    col()
        .class(state.map(|s| {
            if s.workspace_page == WorkspacePage::Desktop
                || !s.connected
                || s.workbench.active != Tab::Desktop
            {
                "desktop-window hidden"
            } else if s.workbench.window(s.workspace_page).maximized {
                "desktop-window maximized"
            } else {
                "desktop-window"
            }
            .to_owned()
        }))
        .fill((
            row().class("desktop-window-titlebar").fill((
                row().class("desktop-window-mark").fill(
                    each(state.map(|s| vec![s.workspace_page]))
                        .key(|page| *page as u8)
                        .children(|item| icons::glyph(app_icon(item.value.get()))),
                ),
                text().class("desktop-window-title").content(i18n::bind(
                    state.map(|s| app_label(s.workspace_page).to_owned()),
                )),
                when(
                    state.map(|s| s.workspace_page == WorkspacePage::Terminal),
                    {
                        let ui = ui.clone();
                        move || terminal::header(ui.clone())
                    },
                ),
                when(
                    state.map(|s| s.workspace_page == WorkspacePage::Explorer),
                    {
                        let ui = ui.clone();
                        move || workspace::explorer_window_tools(ui.clone())
                    },
                ),
                when(
                    state.map(|s| s.workspace_page == WorkspacePage::Resources),
                    {
                        let ui = ui.clone();
                        move || monitor_ui::window_tools(ui.clone())
                    },
                ),
                when(
                    state.map(|s| s.workspace_page == WorkspacePage::SystemLogs),
                    {
                        let ui = ui.clone();
                        move || observability::logs_window_tools(ui.clone())
                    },
                ),
                row().class("desktop-window-actions").fill((
                    icon_action("Minimize window", Icon::WindowMinimize)
                        .class("icon-button window-minimize")
                        .on_press(move || {
                            minimize.state.revise(|s| {
                                observability::metrics_leave(s);
                                s.terminal.menu = None;
                                s.workspace_page = WorkspacePage::Desktop;
                                s.workbench.desktop_page = WorkspacePage::Desktop;
                            });
                        }),
                    sapid::button()
                        .name(i18n::bind("Toggle maximize"))
                        .class("icon-button window-maximize")
                        .pressed(state.map(|s| s.workbench.window(s.workspace_page).maximized))
                        .fill((
                            icons::glyph(Icon::WindowMaximize),
                            icons::glyph(Icon::WindowRestore),
                        ))
                        .on_press(move || {
                            maximize.state.revise(|s| {
                                if let Some(app) = app_id(s.workspace_page) {
                                    let window = s.workbench.windows.entry(app).or_default();
                                    window.maximized = !window.maximized;
                                }
                            });
                            maximize.remember_desktop_layout();
                        }),
                    close_action("Close application")
                        .class("icon-button window-close")
                        .enabled(state.map(|s| {
                            !s.system_task_running
                                && (s.workspace_page != WorkspacePage::Editor
                                    || !remote_editor::desktop_busy(s))
                        }))
                        .on_press(move || close.close_desktop_app()),
                )),
            )),
            col().class("desktop-window-content").fill((
                when(state.map(|s| s.workspace_page == WorkspacePage::Editor), {
                    let ui = ui.clone();
                    move || remote_editor::page(ui.clone())
                }),
                when(state.map(|s| s.workspace_page == WorkspacePage::Docker), {
                    let ui = ui.clone();
                    move || docker_ui::page(ui.clone())
                }),
                when(
                    state.map(|s| s.workspace_page == WorkspacePage::Explorer),
                    {
                        let ui = ui.clone();
                        move || workspace::explorer(ui.clone())
                    },
                ),
                when(
                    state.map(|s| s.workspace_page == WorkspacePage::Software),
                    {
                        let ui = ui.clone();
                        move || software::page(ui.clone())
                    },
                ),
                when(
                    state.map(|s| s.workspace_page == WorkspacePage::Operations),
                    {
                        let ui = ui.clone();
                        move || operations::page(ui.clone())
                    },
                ),
                when(
                    state.map(|s| s.workspace_page == WorkspacePage::Resources),
                    {
                        let ui = ui.clone();
                        move || monitor_ui::page(ui.clone())
                    },
                ),
                when(
                    state.map(|s| s.workspace_page == WorkspacePage::SystemLogs),
                    {
                        let ui = ui.clone();
                        move || observability::logs(ui.clone())
                    },
                ),
                // Mount once for the app lifetime: switching tabs never destroys xterm.
                terminal::page(ui),
            )),
            fragment(
                ["n", "s", "e", "w", "ne", "nw", "se", "sw"]
                    .into_iter()
                    .map(|edge| view().class(format!("desktop-resize-handle edge-{edge}")))
                    .collect::<Vec<_>>(),
            ),
        ))
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

pub(super) fn settings_action(ui: Ui) -> impl Widget {
    sapid::button()
        .name(i18n::bind("Desktop settings"))
        .fill(icons::glyph(Icon::Settings))
        .class("icon-button desktop-settings-button")
        .on_press(move || {
            desktop_customization_ui::open_settings(
                &ui,
                desktop_customization_ui::SettingsPage::Desktop,
            )
        })
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

pub(super) fn desktop(ui: Ui) -> impl Widget {
    let state = ui.state;
    let show_desktop = ui.clone();
    let exit_fullscreen = ui.clone();
    let preferences = ui.clone();
    let geometry = ui.clone();
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
            input().class("desktop-bounds-input hidden").value("").on(
                value_changed,
                move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        save_bounds(&geometry, &value);
                    }
                },
            ),
            text().class("desktop-motion-state").content(state.map(|s| {
                let shown = visible(s);
                let window = s.workbench.window(s.workspace_page);
                serde_json::json!({
                    "shown": shown,
                    "epoch": s.epoch,
                    "app": if shown && s.workspace_page != WorkspacePage::Desktop {
                        app_class(s.workspace_page)
                    } else { "" },
                    "windowApp": app_id(s.workspace_page),
                    "maximized": window.maximized,
                    "bounds": window.bounds,
                    "fullscreen": fullscreen(s),
                })
                .to_string()
            })),
            when(state.map(fullscreen), move || {
                row().class("desktop-menubar").fill((
                    text()
                        .class("desktop-menubar-title")
                        .content(state.map(selected_name)),
                    text()
                        .class("desktop-secure")
                        .content(i18n::bind("SSH 已连接")),
                    settings_action(preferences.clone()),
                    fullscreen_action(exit_fullscreen.clone(), true),
                ))
            }),
            col().class("desktop-canvas").fill((
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
                app_window(ui.clone()),
                col().class("desktop-dock-zone").fill(
                    row()
                        .class(state.map(desktop_customization_ui::dock_class))
                        .fill((
                            sapid::button()
                                .class("desktop-dock-home")
                                .name(i18n::bind("Show desktop"))
                                .fill(icons::glyph(Icon::Grid))
                                .on_press(move || {
                                    show_desktop.state.revise(|s| {
                                        observability::metrics_leave(s);
                                        s.terminal.menu = None;
                                        s.workspace_page = WorkspacePage::Desktop;
                                        s.workbench.desktop_page = WorkspacePage::Desktop;
                                    });
                                    show_desktop.refresh_desktop_items(true);
                                }),
                            text().class("desktop-dock-divider").content(""),
                            row()
                                .class("desktop-dock-apps")
                                .fill(desktop_customization_ui::launchers(ui.clone(), true)),
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
        btn("连接", move || connect.open_dialog(Dialog::Connect))
            .class("button primary")
            .enabled(state.map(|s| !s.connecting)),
        btn("Home", move || ui.open_workbench(Tab::Home)).class("button quiet"),
    ))
}
