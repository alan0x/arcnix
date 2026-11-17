//! Application navigation is independent of the active SSH workspace.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Servers,
    Scripts,
    Tasks,
    Backups,
    Diagnostics,
    Configurations,
    Notifications,
    Settings,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SettingsSection {
    #[default]
    General,
    Connection,
    Files,
    Transfers,
    Monitoring,
    Logging,
    About,
}

impl SettingsSection {
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Connection => "Connection",
            Self::Files => "Files",
            Self::Transfers => "Transfers",
            Self::Monitoring => "Monitoring",
            Self::Logging => "Logging",
            Self::About => "About",
        }
    }
}

impl Ui {
    pub(super) fn navigate(&self, page: Page) {
        self.navigate_from(page, "action");
    }

    fn navigate_from(&self, page: Page, source: &'static str) {
        self.state.revise(|s| {
            change_page(s, page, source);
            s.host_menu = None;
            s.group_menu = None;
            s.file_menu = None;
            if page == Page::Notifications {
                s.unread_notifications = 0;
            }
        });
        if page == Page::Servers {
            self.observability_enter();
            self.terminal_enter();
        }
    }
}

/// Keep leaving-page cleanup consistent for rail and application actions.
pub(super) fn change_page(s: &mut State, page: Page, _source: &'static str) {
    if page != Page::Servers {
        s.workbench.fullscreen = false;
    }
    if s.navigation != page {
        s.terminal.menu = None;
    }
    if s.navigation == Page::Servers && page != Page::Servers {
        observability::metrics_leave(s);
    }
    #[cfg(debug_assertions)]
    let backup_access_before = backups_ui::access_retained(s);
    if page != Page::Scripts {
        crate::script_exchange_ui::forget(s);
    }
    if page != Page::Settings {
        configuration_backup_ui::forget(s);
    }
    if s.navigation != page {
        crate::diagnostics_ui::dismiss(s);
    }
    if page != Page::Backups {
        backups_ui::forget(s);
    }
    if page != Page::Tasks {
        crate::task_refresh_ui::dismiss(s);
        task_records_ui::forget(s);
        record_maintenance_ui::forget(s);
        cleanup_records_ui::forget(s);
    }
    if page != Page::Configurations {
        configuration_records_ui::forget(s);
        configuration_compare_ui::forget(s);
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("ARCNIX_TEST_TRACE_INPUT").as_deref() == Some(std::ffi::OsStr::new("1")) {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        eprintln!(
            "ARCNIX_NAVIGATION {}",
            serde_json::json!({
                "sequence": SEQUENCE.fetch_add(1, Ordering::Relaxed),
                "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
                "from": format!("{:?}", s.navigation), "to": format!("{page:?}"), "source": _source,
                "backup_access_before": backup_access_before, "backup_access_after": backups_ui::access_retained(s),
            })
        );
    }
    s.navigation = page;
}

pub(super) fn workspace_visible(s: &State) -> bool {
    s.navigation == Page::Servers
        && !server_inventory_ui::shown(s)
        && s.connected
        && s.workbench.active != server_desktop::Tab::Home
        && s.workspace_page != WorkspacePage::Desktop
        && s.config
            .servers
            .iter()
            .any(|p| Some(&p.id) == s.selected.as_ref())
}

fn rail_button(
    ui: &Ui,
    page: Page,
    label: &'static str,
    icon: Icon,
    class: &'static str,
) -> impl Widget {
    let state = ui.state;
    let ui = ui.clone();
    sapid::button()
        .name(i18n::bind(label))
        .class(state.map(move |s| {
            format!(
                "rail-item {class}{}",
                if s.navigation == page { " active" } else { "" }
            )
        }))
        .fill(icons::glyph(icon))
        .on_press(move || ui.navigate_from(page, "rail"))
}

fn rail(ui: Ui) -> impl Widget {
    col()
        .class("navigation-rail")
        .name(i18n::bind("主导航"))
        .fill((
            row()
                .class("rail-item rail-logo")
                .fill(text().class("brand-icon").content("")),
            text().class("rail-divider").content(""),
            rail_button(&ui, Page::Servers, "服务器", Icon::Server, "rail-servers"),
            rail_button(&ui, Page::Scripts, "Scripts", Icon::File, "rail-scripts"),
            rail_button(&ui, Page::Tasks, "Tasks", Icon::List, "rail-tasks"),
            rail_button(
                &ui,
                Page::Backups,
                "Backups",
                Icon::Download,
                "rail-backups",
            ),
            rail_button(
                &ui,
                Page::Diagnostics,
                "Diagnostics",
                Icon::Search,
                "rail-diagnostics",
            ),
            rail_button(
                &ui,
                Page::Configurations,
                "Configurations",
                Icon::Edit,
                "rail-configurations",
            ),
            col().class("rail-bottom").fill(rail_button(
                &ui,
                Page::Settings,
                "Settings",
                Icon::Settings,
                "settings-link",
            )),
        ))
}

fn sidebar_link(
    ui: &Ui,
    label: &'static str,
    icon: Icon,
    action: impl FnMut() + 'static,
) -> sapid::Button {
    let _ = ui;
    text_action(label, icon)
        .class("button with-icon context-link")
        .on_press(action)
}

fn context_sidebar(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("context-sidebar").fill((
        text()
            .class("context-title")
            .content(i18n::bind(state.map(|s| {
                match s.navigation {
                    Page::Notifications => "通知",
                    Page::Settings => "Settings",
                    Page::Servers => "服务器",
                    Page::Scripts => "Scripts",
                    Page::Tasks => "Tasks",
                    Page::Backups => "Backups",
                    Page::Diagnostics => "Diagnostics",
                    Page::Configurations => "Configurations",
                }
                .to_owned()
            }))),
        when(state.map(|s| s.navigation == Page::Notifications), {
            let ui = ui.clone();
            move || {
                col().class("context-links").fill((
                    sidebar_link(&ui, "所有活动", Icon::Bell, || {})
                        .class("button with-icon context-link active"),
                    text()
                        .class("context-hint")
                        .content(i18n::bind("连接、传输和操作记录集中显示在这里。")),
                ))
            }
        }),
        when(state.map(|s| s.navigation == Page::Settings), {
            let ui = ui.clone();
            move || {
                col().class("context-links").fill(
                    each(vec![
                        SettingsSection::General,
                        SettingsSection::Connection,
                        SettingsSection::Files,
                        SettingsSection::Transfers,
                        SettingsSection::Monitoring,
                        SettingsSection::Logging,
                        SettingsSection::About,
                    ])
                    .key(|section| *section as u8)
                    .children({
                        let ui = ui.clone();
                        move |item| {
                            let section = item.value.get();
                            let choose = ui.clone();
                            sapid::button()
                                .class(state.map(move |s| {
                                    if s.settings_section == section {
                                        "context-link active"
                                    } else {
                                        "context-link"
                                    }
                                    .to_owned()
                                }))
                                .label(i18n::bind(section.label()))
                                .on_press(move || {
                                    choose.state.revise(|s| s.settings_section = section)
                                })
                        }
                    }),
                )
            }
        }),
        col().class("context-footer").fill((
            text()
                .class("context-footer-title")
                .content(i18n::bind("你的服务器工作台")),
            text()
                .class("subtle")
                .content(i18n::bind("通过 SSH 连接，无需远程 Agent。")),
            text()
                .class(state.map(|s| {
                    if s.connected {
                        "session-indicator online"
                    } else {
                        "session-indicator"
                    }
                    .to_owned()
                }))
                .content(i18n::bind(state.map(|s| {
                    if s.connected {
                        "一个活动连接"
                    } else {
                        "当前没有活动连接"
                    }
                    .to_owned()
                }))),
        )),
    ))
}

fn empty_state(
    title: &'static str,
    description: &'static str,
    actions: impl IntoChildren + 'static,
) -> impl Widget {
    col().class("navigation-empty").fill((
        icons::glyph(Icon::Server),
        text().class("empty-title").content(i18n::bind(title)),
        text()
            .class("empty-description")
            .content(i18n::bind(description)),
        row().class("empty-actions").fill(actions),
    ))
}

fn server_landing(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("server-landing page-scroll").fill((
        when(state.map(|s| s.selected.is_none()), {
            let ui = ui.clone();
            move || {
                empty_state(
                    "选择一台服务器",
                    "从左侧选择服务器，连接后即可开始管理。",
                    text_action("新建服务器", Icon::Plus)
                        .class("button with-icon primary")
                        .on_press({
                            let ui = ui.clone();
                            move || servers::new_server(&ui)
                        }),
                )
            }
        }),
        when(state.map(|s| s.selected.is_some()), {
            let ui = ui.clone();
            move || {
                let edit = ui.clone();
                col().class("server-overview").fill((
                    text()
                        .class("overview-server-icon action-glyph glyph-server")
                        .content(""),
                    icon_action("编辑服务器", Icon::Settings)
                        .class("icon-button overview-edit")
                        .on_press(move || {
                            if let Some(id) = state.with(|s| s.selected.clone()) {
                                servers::edit_server(&edit, &id);
                            }
                        }),
                    text().class("home-title").content(state.map(|s| {
                        s.config
                            .servers
                            .iter()
                            .find(|p| Some(&p.id) == s.selected.as_ref())
                            .map(|p| p.name.clone())
                            .unwrap_or_default()
                    })),
                    text()
                        .class("home-description")
                        .content(i18n::bind(state.map(|s| {
                            if s.connecting {
                                "正在建立安全连接…"
                            } else {
                                "选择桌面管理服务器，或打开独立的文件传输工作区。"
                            }
                            .to_owned()
                        }))),
                    row().class("server-facts").fill((
                        detail("主机", &ui, |p| p.host.clone()),
                        detail("端口", &ui, |p| p.port.to_string()),
                        detail("用户", &ui, |p| p.username.clone()),
                        detail("认证方式", &ui, |p| {
                            i18n::translate(if p.auth == AuthMethod::Password {
                                "密码"
                            } else {
                                "私钥"
                            })
                        }),
                    )),
                    row().class("workspace-entries").fill((
                        server_desktop::entry(ui.clone(), server_desktop::Tab::Desktop),
                        server_desktop::entry(ui.clone(), server_desktop::Tab::Files),
                    )),
                    text()
                        .class("overview-footnote subtle")
                        .content(i18n::bind("密码与私钥口令仅用于本次连接，不会保存到配置。")),
                ))
            }
        }),
    ))
}

fn detail(label: &'static str, ui: &Ui, value: fn(&ServerProfile) -> String) -> impl Widget {
    col().class("server-fact").fill((
        text().class("subtle").content(i18n::bind(label)),
        text()
            .class("server-fact-value")
            .content(ui.state.map(move |s| {
                s.config
                    .servers
                    .iter()
                    .find(|p| Some(&p.id) == s.selected.as_ref())
                    .map(value)
                    .unwrap_or_default()
            })),
    ))
}

fn notifications(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("notifications-page page-scroll").fill((
        text().class("page-title").content(i18n::bind("通知与活动")),
        text().class("subtle").content(i18n::bind(
            "本次会话中的连接、传输与操作记录，最新记录显示在前。",
        )),
        col().class("notification-feed").fill(
            each(state.map(|s| {
                s.logs
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .enumerate()
                    .map(|(index, line)| (index, line.to_owned()))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .take(100)
                    .collect::<Vec<_>>()
            }))
            .key(|(index, line)| (*index, line.clone()))
            .children(|item| {
                row().class("notification-entry").fill((
                    icons::glyph(Icon::Bell),
                    text()
                        .class("notification-message")
                        .selectable(true)
                        .content(item.value.map(|(_, line)| line.clone())),
                ))
            }),
        ),
        text().class("subtle").content(i18n::bind(
            "显示最近 100 条记录。活动记录仅保留在本次会话。",
        )),
    ))
}

fn global_sidebar(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| matches!(s.navigation, Page::Scripts | Page::Tasks)),
        {
            let ui = ui.clone();
            move || scripts_ui::sidebar(ui.clone())
        },
    )
    .otherwise({
        let ui = ui.clone();
        move || {
            when(state.map(|s| s.navigation == Page::Backups), {
                let ui = ui.clone();
                move || backups_ui::sidebar(ui.clone())
            })
            .otherwise({
                let ui = ui.clone();
                move || {
                    when(state.map(|s| s.navigation == Page::Diagnostics), {
                        let ui = ui.clone();
                        move || diagnostics_ui::sidebar(ui.clone())
                    })
                    .otherwise({
                        let ui = ui.clone();
                        move || {
                            when(state.map(|s| s.navigation == Page::Configurations), {
                                let ui = ui.clone();
                                move || configurations_ui::sidebar(ui.clone())
                            })
                            .otherwise({
                                let ui = ui.clone();
                                move || context_sidebar(ui.clone())
                            })
                        }
                    })
                }
            })
        }
    })
}

pub(super) fn shell(ui: Ui) -> impl Widget {
    let state = ui.state;
    let view_log = ui.clone();
    let dismiss = ui.clone();
    row()
        .class(state.map(|s| {
            if server_desktop::fullscreen(s) {
                "navigation-shell desktop-fullscreen"
            } else {
                "navigation-shell"
            }
            .to_owned()
        }))
        .fill((
            rail(ui.clone()),
            when(state.map(|s| s.navigation == Page::Servers), {
                let ui = ui.clone();
                move || servers::sidebar(ui.clone())
            })
            .otherwise({
                let ui = ui.clone();
                move || global_sidebar(ui.clone())
            }),
            col().class("main-panel").fill((
                row().class("page-header").fill((
                    text().class("page-header-title").content(state.map(|s| {
                        match s.navigation {
                            Page::Servers if server_inventory_ui::shown(s) => {
                                i18n::translate("Server overview")
                            }
                            Page::Servers => s
                                .config
                                .servers
                                .iter()
                                .find(|p| Some(&p.id) == s.selected.as_ref())
                                .map(|p| p.name.clone())
                                .unwrap_or_else(|| i18n::translate("服务器")),
                            Page::Scripts => i18n::translate("Scripts"),
                            Page::Tasks => i18n::translate("Tasks"),
                            Page::Backups => i18n::translate("Backups"),
                            Page::Diagnostics => i18n::translate("Diagnostics"),
                            Page::Configurations => i18n::translate("Configurations"),
                            Page::Notifications => i18n::translate("所有活动"),
                            Page::Settings => i18n::translate(s.settings_section.label()),
                        }
                    })),
                    when(
                        state.map(|s| {
                            s.navigation == Page::Servers
                                && s.selected.is_some()
                                && !server_inventory_ui::shown(s)
                        }),
                        {
                            let ui = ui.clone();
                            move || connection_controls(ui.clone())
                        },
                    ),
                    when(state.map(|s| s.navigation != Page::Servers), || {
                        text().class("header-product").content("Arcnix")
                    }),
                )),
                when(
                    state.map(|s| {
                        s.navigation == Page::Servers
                            && s.selected.is_some()
                            && !server_inventory_ui::shown(s)
                    }),
                    {
                        let ui = ui.clone();
                        move || server_desktop::tabs(ui.clone())
                    },
                ),
                col().class("workspace-content").fill((
                    row()
                        .class(state.map(|s| {
                            if s.notice.is_empty() {
                                "hidden"
                            } else {
                                "notice"
                            }
                            .to_owned()
                        }))
                        .fill((
                            text()
                                .class("notice-text")
                                .content(i18n::bind(state.map(|s| s.notice.clone()))),
                            btn("查看操作日志", move || {
                                view_log.navigate(Page::Notifications)
                            }),
                            close_action("忽略提示")
                                .on_press(move || dismiss.state.revise(|s| s.notice.clear())),
                        )),
                    when(state.map(|s| s.navigation == Page::Scripts), {
                        let ui = ui.clone();
                        move || scripts_ui::page(ui.clone())
                    }),
                    when(state.map(|s| s.navigation == Page::Tasks), {
                        let ui = ui.clone();
                        move || scripts_ui::tasks_page(ui.clone())
                    }),
                    when(state.map(|s| s.navigation == Page::Backups), {
                        let ui = ui.clone();
                        move || backups_ui::page(ui.clone())
                    }),
                    when(state.map(|s| s.navigation == Page::Diagnostics), {
                        let ui = ui.clone();
                        move || diagnostics_ui::page(ui.clone())
                    }),
                    when(state.map(|s| s.navigation == Page::Configurations), {
                        let ui = ui.clone();
                        move || configurations_ui::page(ui.clone())
                    }),
                    when(state.map(|s| s.navigation == Page::Settings), {
                        let ui = ui.clone();
                        move || settings::page(ui.clone())
                    }),
                    when(state.map(|s| s.navigation == Page::Notifications), {
                        let ui = ui.clone();
                        move || notifications(ui.clone())
                    }),
                    when(
                        state.map(|s| {
                            s.navigation == Page::Servers
                                && (s.selected.is_none()
                                    || s.workbench.active == server_desktop::Tab::Home)
                                && !server_inventory_ui::shown(s)
                        }),
                        {
                            let ui = ui.clone();
                            move || server_landing(ui.clone())
                        },
                    ),
                    when(state.map(server_inventory_ui::shown), {
                        let ui = ui.clone();
                        move || server_inventory_ui::page(ui.clone())
                    }),
                    fragment((
                        when(
                            state.map(|s| {
                                s.navigation == Page::Servers
                                    && !server_inventory_ui::shown(s)
                                    && s.workbench.active != server_desktop::Tab::Home
                                    && !s.connected
                            }),
                            {
                                let ui = ui.clone();
                                move || server_desktop::reconnect(ui.clone())
                            },
                        ),
                        when(
                            state.map(|s| {
                                workspace_visible(s)
                                    && s.workbench.active == server_desktop::Tab::Files
                            }),
                            {
                                let ui = ui.clone();
                                move || {
                                    col().class("files-module").fill((
                                        workspace::files(ui.clone()),
                                        workspace::activity(ui.clone()),
                                    ))
                                }
                            },
                        ),
                        server_desktop::desktop(ui),
                    )),
                )),
            )),
        ))
}
