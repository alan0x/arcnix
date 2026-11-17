//! Browser-style server tabs and the desktop application shell.
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
    pub maximized: bool,
    pub fullscreen: bool,
}

impl Default for Workbench {
    fn default() -> Self {
        Self {
            active: Tab::Home,
            tabs: vec![Tab::Home],
            desktop_page: WorkspacePage::Desktop,
            apps: Vec::new(),
            maximized: false,
            fullscreen: false,
        }
    }
}

const APPS: [WorkspacePage; 6] = [
    WorkspacePage::Terminal,
    WorkspacePage::Resources,
    WorkspacePage::Software,
    WorkspacePage::Operations,
    WorkspacePage::SystemLogs,
    WorkspacePage::Explorer,
];

fn app_label(page: WorkspacePage) -> &'static str {
    match page {
        WorkspacePage::Explorer => "File explorer",
        WorkspacePage::Terminal => "终端",
        WorkspacePage::Resources => "资源概览",
        WorkspacePage::Software => "系统软件",
        WorkspacePage::Operations => "运维操作",
        WorkspacePage::SystemLogs => "系统日志",
        _ => "Desktop",
    }
}

fn app_class(page: WorkspacePage) -> &'static str {
    match page {
        WorkspacePage::Explorer => "app-explorer",
        WorkspacePage::Terminal => "app-terminal",
        WorkspacePage::Resources => "app-resources",
        WorkspacePage::Software => "app-software",
        WorkspacePage::Operations => "app-operations",
        WorkspacePage::SystemLogs => "app-logs",
        _ => "app-desktop",
    }
}

fn app_mark(page: WorkspacePage) -> &'static str {
    match page {
        WorkspacePage::Terminal => ">_",
        WorkspacePage::Resources => "▥",
        WorkspacePage::Software => "▦",
        WorkspacePage::Operations => "⌘",
        WorkspacePage::SystemLogs => "≡",
        _ => "",
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

pub(super) fn fullscreen(s: &State) -> bool {
    s.workbench.fullscreen
        && s.navigation == navigation::Page::Servers
        && !server_inventory_ui::shown(s)
        && s.connected
        && s.workbench.active == Tab::Desktop
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
        self.terminal_enter();
        self.observability_enter();
        self.operations_enter();
        if self.state.with(|s| {
            navigation::workspace_visible(s)
                && s.workspace_page == WorkspacePage::Software
                && s.software.environment.is_none()
                && s.software.pending.is_none()
        }) {
            self.software_detect();
        }
    }

    fn open_desktop_app(&self, page: WorkspacePage) {
        self.state.revise(|s| select_app(s, page));
        self.desktop_enter();
    }

    fn close_desktop_app(&self) {
        let page = self.state.with(|s| s.workspace_page);
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

fn launcher(ui: Ui, page: WorkspacePage, dock: bool) -> impl Widget {
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
            text().class("desktop-app-icon").content(app_mark(page)),
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
            } else if s.workbench.maximized {
                "desktop-window maximized"
            } else {
                "desktop-window"
            }
            .to_owned()
        }))
        .fill((
            row().class("desktop-window-titlebar").fill((
                row()
                    .class(
                        state.map(|s| {
                            format!("desktop-window-mark {}", app_class(s.workspace_page))
                        }),
                    )
                    .fill(
                        when(
                            state.map(|s| s.workspace_page == WorkspacePage::Explorer),
                            || icons::glyph(Icon::Folder),
                        )
                        .otherwise(move || {
                            text().content(state.map(|s| app_mark(s.workspace_page).to_owned()))
                        }),
                    ),
                text().class("desktop-window-title").content(i18n::bind(
                    state.map(|s| app_label(s.workspace_page).to_owned()),
                )),
                text()
                    .class("desktop-window-host")
                    .content(state.map(selected_host)),
                icon_action("Minimize window", Icon::Collapse)
                    .class("icon-button window-minimize")
                    .on_press(move || {
                        minimize.state.revise(|s| {
                            observability::metrics_leave(s);
                            s.terminal.menu = None;
                            s.workspace_page = WorkspacePage::Desktop;
                            s.workbench.desktop_page = WorkspacePage::Desktop;
                        });
                    }),
                icon_action("Toggle maximize", Icon::Expand)
                    .class("icon-button window-maximize")
                    .on_press(move || {
                        maximize
                            .state
                            .revise(|s| s.workbench.maximized = !s.workbench.maximized)
                    }),
                close_action("Close application")
                    .class("icon-button window-close")
                    .enabled(state.map(|s| !s.system_task_running))
                    .on_press(move || close.close_desktop_app()),
            )),
            col().class("desktop-window-content").fill((
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
                        move || observability::resources(ui.clone())
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
        ))
}

fn selected_host(s: &State) -> String {
    s.config
        .servers
        .iter()
        .find(|p| Some(&p.id) == s.selected.as_ref())
        .map(|p| p.host.clone())
        .unwrap_or_default()
}

pub(super) fn desktop(ui: Ui) -> impl Widget {
    let state = ui.state;
    let show_desktop = ui.clone();
    let exit_fullscreen = ui.clone();
    col()
        .class(state.map(|s| {
            if s.navigation == navigation::Page::Servers
                && !server_inventory_ui::shown(s)
                && s.workbench.active == Tab::Desktop
                && s.connected
            {
                "server-desktop"
            } else {
                "server-desktop hidden"
            }
            .to_owned()
        }))
        .fill((
            row().class("desktop-menubar").fill((
                text().class("desktop-brand").content("arcnix"),
                text()
                    .class("desktop-menubar-title")
                    .content(i18n::bind("Desktop")),
                text().class("desktop-session").content(state.map(|s| {
                    s.config
                        .servers
                        .iter()
                        .find(|p| Some(&p.id) == s.selected.as_ref())
                        .map(|p| format!("{} @ {}", p.username, p.host))
                        .unwrap_or_default()
                })),
                text()
                    .class("desktop-secure")
                    .content(i18n::bind("SSH 已连接")),
                when(state.map(fullscreen), move || {
                    fullscreen_action(exit_fullscreen.clone(), true)
                }),
            )),
            col().class("desktop-canvas").fill((
                col().class("desktop-wallpaper-art"),
                col().class("desktop-shortcuts").fill(
                    APPS.into_iter()
                        .map({
                            let ui = ui.clone();
                            move |page| launcher(ui.clone(), page, false)
                        })
                        .collect::<Vec<_>>(),
                ),
                col().class("desktop-wallpaper-copy").fill((
                    text()
                        .class("desktop-wallpaper-kicker")
                        .content("SERVER WORKSPACE"),
                    text()
                        .class("desktop-wallpaper-host")
                        .content(state.map(selected_host)),
                    text()
                        .class("desktop-wallpaper-caption")
                        .content(i18n::bind("选择应用，开始工作。")),
                )),
                app_window(ui.clone()),
                row().class("desktop-dock").fill((
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
                        }),
                    text().class("desktop-dock-divider").content(""),
                    row().class("desktop-dock-apps").fill(
                        APPS.into_iter()
                            .map(move |page| launcher(ui.clone(), page, true))
                            .collect::<Vec<_>>(),
                    ),
                )),
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
