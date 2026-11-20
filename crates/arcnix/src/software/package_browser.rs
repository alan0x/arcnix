use arcnix_core::packages::{Package, PackageAction, SystemEnvironment};

use crate::*;

#[derive(Clone, Default)]
pub(crate) struct SoftwareState {
    pub app_center: app_center_ui::AppCenterState,
    pub source: u8,
    pub environment: Option<SystemEnvironment>,
    pub packages: Vec<Package>,
    pub query: String,
    pub installed_view: bool,
    pub pending: Option<u64>,
    pub status: String,
    pub error: String,
}

pub(crate) enum Event {
    Environment(std::result::Result<SystemEnvironment, String>),
    Packages(std::result::Result<Vec<Package>, String>),
}

impl Ui {
    fn software_available(&self, require_manager: bool) -> bool {
        self.state.with(|s| {
            s.connected
                && !s.system_task_running
                && s.software.pending.is_none()
                && (!require_manager
                    || s.software.source == 1
                    || s.software
                        .environment
                        .as_ref()
                        .is_some_and(|e| arcnix_core::software_install::supported(e.manager)))
        })
    }
    pub(crate) fn software_detect(&self) {
        if !self.software_available(false) {
            return;
        }
        let serial = self.send(Action::PackageDetect);
        self.state.revise(|s| {
            s.software.pending = Some(serial);
            s.software.error.clear();
            s.software.status = "正在识别系统和包管理器…".into();
        });
    }
    pub(crate) fn software_search(&self) {
        if !self.software_available(true) {
            return;
        }
        let query = self.state.with(|s| s.software.query.trim().to_string());
        if query.is_empty() {
            self.state
                .revise(|s| s.software.error = "请输入软件包名称，例如 nginx、git 或 curl".into());
            return;
        }
        let source = self.state.with(software_source);
        let serial = self.send(
            if source
                == arcnix_core::software_install::Source::Native(
                    arcnix_core::packages::PackageManager::Apt,
                )
            {
                Action::PackageSearch(query)
            } else {
                Action::ServerApps(server_apps_worker::Action::SoftwareQuery {
                    source,
                    query: Some(query),
                })
            },
        );
        self.state.revise(|s| {
            s.software.pending = Some(serial);
            s.software.installed_view = false;
            s.software.error.clear();
            s.software.status = "正在搜索软件包…".into();
        });
    }
    pub(crate) fn software_installed(&self) {
        if !self.software_available(true) {
            return;
        }
        let source = self.state.with(software_source);
        let serial = self.send(
            if source
                == arcnix_core::software_install::Source::Native(
                    arcnix_core::packages::PackageManager::Apt,
                )
            {
                Action::PackageInstalled
            } else {
                Action::ServerApps(server_apps_worker::Action::SoftwareQuery {
                    source,
                    query: None,
                })
            },
        );
        self.state.revise(|s| {
            s.software.pending = Some(serial);
            s.software.installed_view = true;
            s.software.error.clear();
            s.software.status = "正在读取已安装软件…".into();
        });
    }
    pub(crate) fn software_preview(&self, action: PackageAction) {
        if !self.software_available(true) {
            return;
        }
        self.native_package_preview(action);
    }
}

pub(crate) fn event(ui: &Ui, serial: u64, event: Event) {
    match event {
        Event::Environment(result) => {
            if !ui.state.with(|s| s.software.pending == Some(serial)) {
                return;
            }
            let supported = result
                .as_ref()
                .is_ok_and(|e| arcnix_core::software_install::supported(e.manager));
            ui.state.revise(|s| {
                s.software.pending = None;
                match result {
                    Ok(env) => {
                        s.software.error.clear();
                        s.software.packages.clear();
                        s.software.status = if arcnix_core::software_install::supported(env.manager)
                        {
                            "系统识别完成".into()
                        } else {
                            format!("No supported package manager: {}", env.manager.label())
                        };
                        s.software.environment = Some(env);
                    }
                    Err(e) => {
                        s.software.error = e;
                        s.software.status = "系统识别失败".into();
                    }
                }
            });
            if supported {
                ui.app_center_enter();
            }
        }
        Event::Packages(result) => {
            if !ui.state.with(|s| s.software.pending == Some(serial)) {
                return;
            }
            ui.state.revise(|s| {
                s.software.pending = None;
                match result {
                    Ok(packages) => {
                        s.software.status = format!(
                            "{} · {} 项",
                            if s.software.installed_view {
                                "已安装软件"
                            } else {
                                "搜索结果"
                            },
                            packages.len()
                        );
                        s.software.packages = packages;
                        s.software.error.clear();
                    }
                    Err(e) => {
                        s.software.error = e;
                        s.software.status = "软件包查询失败".into();
                    }
                }
            });
        }
    }
}

pub(crate) fn on_connection_lost(ui: &Ui, _message: &str) {
    ui.state.revise(|s| s.software.pending = None);
}

fn ready(s: &State) -> bool {
    s.connected
        && !s.system_task_running
        && s.software.pending.is_none()
        && if s.software.source == 1 {
            s.server_apps.applications.snap
        } else {
            s.software
                .environment
                .as_ref()
                .is_some_and(|e| arcnix_core::software_install::supported(e.manager))
        }
}

fn package_page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let detect = ui.clone();
    let search = ui.clone();
    let enter = ui.clone();
    let installed = ui.clone();
    let refresh = ui.clone();
    col().class("package-source-page").fill((
        row().class("software-heading").fill((
            col().fill((
                text().class("page-title").content(i18n::bind("系统软件")),
                text()
                    .class("subtle")
                    .content(i18n::bind("查询服务器软件，预览并执行软件安装。")),
            )),
            icon_action("重新识别系统", Icon::Refresh)
                .enabled(
                    state.map(|s| {
                        s.connected && !s.system_task_running && s.software.pending.is_none()
                    }),
                )
                .on_press(move || detect.software_detect()),
        )),
        when(state.map(|s| !s.connected), move || {
            col().class("software-empty").fill((
                text()
                    .class("pane-title")
                    .content(i18n::bind("连接服务器后管理系统软件")),
                text().class("subtle").content(i18n::bind(
                    "选择左侧服务器并连接。软件管理目前支持 Debian / Ubuntu 的 APT。",
                )),
            ))
        }),
        when(state.map(|s| s.connected), move || {
            let search = search.clone();
            let enter = enter.clone();
            let installed = installed.clone();
            let refresh = refresh.clone();
            let table_ui = ui.clone();
            col().class("software-connected").fill((
                row().class("software-environment").fill((
                    text().content(i18n::bind(state.map(|s| {
                        s.software
                            .environment
                            .as_ref()
                            .map(|e| format!("{}  ·  {}", e.distribution, e.manager.label()))
                            .unwrap_or_else(|| "尚未识别系统".into())
                    }))),
                    text().class("subtle").content(i18n::bind(state.map(|s| {
                        s.software
                            .environment
                            .as_ref()
                            .map(|e| {
                                if e.is_root {
                                    "root 会话"
                                } else if e.sudo_available {
                                    "通过 sudo 执行管理员操作"
                                } else {
                                    "无可用管理员权限"
                                }
                                .to_string()
                            })
                            .unwrap_or_default()
                    }))),
                )),
                row().class("software-toolbar").fill((
                    input()
                        .name(i18n::bind("搜索软件包"))
                        .semantics(Some(Box::new(Semantics {
                            placeholder: Some(i18n::translate("软件包名称，如 nginx、git、curl")),
                            ..Default::default()
                        })))
                        .class("software-search")
                        .value(state.map(|s| s.software.query.clone()))
                        .enabled(state.map(ready))
                        .on(value_changed, move |e| {
                            if let glory::ui::Event::ValueChanged { value, .. } = e {
                                state.revise(|s| s.software.query = value);
                            }
                        })
                        .on(commit, move |e| {
                            if let glory::ui::Event::Commit { value, .. } = e {
                                enter.state.revise(|s| s.software.query = value);
                                enter.software_search();
                            }
                        }),
                    sapid::button()
                        .class("button primary")
                        .label(i18n::bind("搜索"))
                        .enabled(state.map(ready))
                        .on_press(move || search.software_search()),
                    sapid::button()
                        .class("button")
                        .label(i18n::bind("已安装"))
                        .enabled(state.map(ready))
                        .on_press(move || installed.software_installed()),
                    sapid::button()
                        .class("button")
                        .label(i18n::bind("更新软件源索引"))
                        .enabled(state.map(|s| ready(s) && !scripts_ui::busy(s)))
                        .on_press(move || refresh.software_preview(PackageAction::RefreshIndex)),
                )),
                text()
                    .class("software-status")
                    .content(i18n::bind(state.map(|s| s.software.status.clone()))),
                text()
                    .class(state.map(|s| {
                        if s.software.error.is_empty() {
                            "hidden"
                        } else {
                            "software-error"
                        }
                        .to_string()
                    }))
                    .content(i18n::bind(state.map(|s| s.software.error.clone()))),
                package_table(table_ui),
            ))
        }),
    ))
}

fn package_table(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("software-packages").fill((
        row().class("software-table-header").fill((
            text().class("package-name").content(i18n::bind("软件包")),
            text()
                .class("package-version")
                .content(i18n::bind("Version")),
            text()
                .class("package-description")
                .content(i18n::bind("说明")),
            text().class("package-action").content(i18n::bind("操作")),
        )),
        col().class("software-package-list").fill(
            each(state.map(|s| s.software.packages.clone()))
                .key(|p| p.name.clone())
                .children(move |item| {
                    let package = item.value;
                    let install = ui.clone();
                    let upgrade = ui.clone();
                    let remove = ui.clone();
                    row().class("software-package-row").fill((
                        text()
                            .class("package-name")
                            .content(package.map(|p| p.name.clone())),
                        text().class("package-version").content(package.map(|p| {
                            if p.version.is_empty() {
                                "—".into()
                            } else {
                                p.version.clone()
                            }
                        })),
                        text()
                            .class("package-description")
                            .content(package.map(|p| p.summary.clone())),
                        sapid::button()
                            .class("button compact package-action")
                            .name(i18n::bind(package.map(|p| {
                                format!(
                                    "{} {}",
                                    if p.installed {
                                        "已安装"
                                    } else {
                                        "预览安装"
                                    },
                                    p.name
                                )
                            })))
                            .label(i18n::bind(package.map(|p| {
                                if p.installed {
                                    "已安装"
                                } else {
                                    "安装…"
                                }
                                .to_string()
                            })))
                            .enabled(state.map(move |s| {
                                ready(s) && !scripts_ui::busy(s) && !package.get().installed
                            }))
                            .on_press(move || {
                                install.software_preview(PackageAction::Install(vec![
                                    package.get().name,
                                ]))
                            }),
                        when(package.map(|p| p.installed), move || {
                            let upgrade = upgrade.clone();
                            let remove = remove.clone();
                            row().class("source-package-actions").fill((
                                btn("Upgrade…", move || {
                                    upgrade.software_source_action(
                                        vec![package.get().name],
                                        arcnix_core::software_install::Action::Upgrade,
                                    )
                                })
                                .enabled(state.map(|s| ready(s) && !scripts_ui::busy(s))),
                                btn("Remove…", move || {
                                    remove.software_source_action(
                                        vec![package.get().name],
                                        arcnix_core::software_install::Action::Remove,
                                    )
                                })
                                .enabled(state.map(|s| ready(s) && !scripts_ui::busy(s))),
                            ))
                        }),
                    ))
                }),
        ),
        text()
            .class(state.map(|s| {
                if s.software.packages.is_empty() {
                    "software-list-empty"
                } else {
                    "hidden"
                }
                .to_string()
            }))
            .content(i18n::bind(state.map(|s| {
                if s.software.pending.is_some() {
                    "正在读取…"
                } else {
                    "暂无软件包结果。输入名称搜索，或查看已安装软件。"
                }
                .to_string()
            }))),
    ))
}

fn software_source(s: &State) -> arcnix_core::software_install::Source {
    if s.software.source == 1 {
        arcnix_core::software_install::Source::Snap
    } else {
        arcnix_core::software_install::Source::Native(
            s.software
                .environment
                .as_ref()
                .map(|e| e.manager)
                .unwrap_or(arcnix_core::packages::PackageManager::Unknown),
        )
    }
}
pub(crate) fn server_reply(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<server_apps_worker::Reply, String>,
) -> Option<std::result::Result<server_apps_worker::Reply, String>> {
    if !ui.state.with(|s| s.software.pending == Some(serial)) {
        return app_center_ui::event(ui, serial, result);
    }
    let result = match result {
        Ok(server_apps_worker::Reply::SoftwarePackages(rows)) => Ok(rows),
        Err(e) => Err(e),
        _ => Err("Invalid software query response".into()),
    };
    event(ui, serial, Event::Packages(result));
    None
}
pub(crate) fn packages(ui: Ui) -> impl Widget {
    package_page(ui)
}
