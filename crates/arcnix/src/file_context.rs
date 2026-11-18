//! Shared file actions for pointer and keyboard context menus.
use super::*;

#[derive(Clone)]
pub(super) struct Target {
    remote: bool,
    item: bool,
}
#[derive(Clone)]
pub(super) struct Deletion {
    serial: u64,
    epoch: u64,
    remote: bool,
    cancel: Arc<AtomicBool>,
    progress: Option<arcnix_core::deletion::Progress>,
}
pub(super) fn deletion_dialog(s: &State) -> bool {
    s.deletion.is_some() && matches!(s.dialog, Some(Dialog::DeleteFiles { .. }))
}
pub(super) fn cancel_deletion(ui: &Ui) {
    ui.state.with(|s| {
        if let Some(task) = &s.deletion {
            task.cancel.store(true, Ordering::Relaxed);
        }
    });
    ui.state.revise(|_| {});
}
pub(super) fn dismiss_deletion(ui: &Ui) -> bool {
    if !ui.state.with(deletion_dialog) {
        return false;
    }
    cancel_deletion(ui);
    ui.state.revise(|s| s.dialog = None);
    true
}
pub(super) fn deletion_progress(ui: &Ui, serial: u64, progress: arcnix_core::deletion::Progress) {
    ui.state.revise(|s| {
        if let Some(task) = s.deletion.as_mut().filter(|t| t.serial == serial) {
            task.progress = Some(progress);
        }
    });
}
fn progress_text(s: &State) -> String {
    let Some(task) = &s.deletion else {
        return String::new();
    };
    if task.cancel.load(Ordering::Relaxed) {
        return i18n::translate("正在停止删除，等待当前文件请求结束…");
    }
    match &task.progress {
        None => i18n::translate("正在准备删除…"),
        Some(p) => {
            let status = if p.phase == arcnix_core::deletion::Phase::Scanning {
                i18n::translate(&format!("正在扫描：已发现 {} 项", p.scanned))
            } else {
                i18n::translate(&format!("正在删除：{} / {} 项", p.removed, p.total))
            };
            format!("{status}\n{}", p.path)
        }
    }
}
pub(super) fn task_status(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(state.map(|s| s.deletion.is_some()), move || {
        let cancel = ui.clone();
        row().class("file-task-status").fill((
            text()
                .class("file-task-progress")
                .content(state.map(progress_text)),
            text_action("停止删除", Icon::Close)
                .enabled(state.map(|s| {
                    s.deletion
                        .as_ref()
                        .is_some_and(|d| !d.cancel.load(Ordering::Relaxed))
                }))
                .on_press(move || cancel_deletion(&cancel)),
        ))
    })
}
fn selection(s: &State, remote: bool) -> &HashSet<String> {
    if remote {
        &s.remote_selection
    } else {
        &s.local_selection
    }
}
pub(super) fn chosen(s: &State, remote: bool) -> Vec<FileEntry> {
    file_browser::entries(s, remote)
        .into_iter()
        .filter(|f| selection(s, remote).contains(&f.path))
        .collect()
}
fn single(s: &State, remote: bool) -> Option<FileEntry> {
    let mut entries = chosen(s, remote);
    (entries.len() == 1).then(|| entries.remove(0))
}
fn terminal_directory(s: &State) -> Option<String> {
    if !workspace::ready(s, true) {
        return None;
    }
    let directory = if s.remote_selection.is_empty() {
        s.remote_path.clone()
    } else {
        let file = single(s, true)?;
        if file.is_dir && !file.is_symlink {
            file.path
        } else {
            file.path
                .rsplit_once('/')
                .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
                .unwrap_or(&s.remote_path)
                .to_owned()
        }
    };
    terminal::directory_supported(&directory).then_some(directory)
}
fn terminal_action(ui: Ui) -> impl Widget {
    action(
        &ui,
        true,
        "在此处打开终端",
        |s, _| terminal_directory(s).is_some(),
        {
            let ui = ui.clone();
            move || {
                if let Some(directory) = ui.state.with(terminal_directory) {
                    ui.terminal_here(directory);
                }
            }
        },
    )
}
fn has_item(s: &State, remote: bool) -> bool {
    s.file_context
        .as_ref()
        .filter(|c| c.remote == remote)
        .map(|c| c.item)
        .unwrap_or_else(|| !selection(s, remote).is_empty())
}
fn display_path(file: &FileEntry, remote: bool) -> String {
    if remote {
        file.path.clone()
    } else {
        local_display(Path::new(&file.path))
    }
}
pub(super) fn open(ui: &Ui, remote: bool, path: &str) {
    if ui.state.with(|s| {
        s.dialog.is_some()
            || !navigation::workspace_visible(s)
            || !(s.workspace_page == WorkspacePage::Files
                || (remote && s.workspace_page == WorkspacePage::Explorer))
    }) {
        return;
    }
    if path.is_empty() {
        ui.state
            .revise(|s| file_browser::clear_selection(s, remote));
    } else {
        let valid = ui.state.with(|s| {
            file_browser::entries(s, remote)
                .iter()
                .any(|f| f.path == path)
        });
        if !valid {
            return;
        }
        if !ui.state.with(|s| selection(s, remote).contains(path)) {
            file_browser::select(ui, remote, path, false, false);
        }
    }
    ui.state.revise(|s| {
        s.host_menu = None;
        s.group_menu = None;
        s.file_menu = Some(remote);
        s.file_context = Some(Target {
            remote,
            item: !path.is_empty(),
        });
    });
}
fn close(ui: &Ui) {
    ui.state.revise(|s| s.file_menu = None);
}
pub(super) fn copy_paths(ui: &Ui, remote: bool, directory: bool) {
    let text = ui.state.with(|s| {
        if directory {
            file_browser::path(s, remote).to_owned()
        } else {
            chosen(s, remote)
                .into_iter()
                .map(|f| display_path(&f, remote))
                .collect::<Vec<_>>()
                .join("\n")
        }
    });
    copy_text(ui, &text);
}
fn copy_text(ui: &Ui, value: &str) {
    match clipboard::copy(value) {
        Ok(()) => ui.log("路径已复制"),
        Err(error) => ui.error(format!("复制路径失败：{error:#}")),
    }
}
pub(super) fn properties(ui: &Ui, remote: bool) {
    file_properties::open(ui, remote);
}
fn action<F: FnMut() + 'static>(
    ui: &Ui,
    remote: bool,
    label: &'static str,
    enabled: fn(&State, bool) -> bool,
    mut handler: F,
) -> impl Widget + use<F> {
    let close_ui = ui.clone();
    sapid::button()
        .class(if label == "删除" {
            "menu-item danger"
        } else {
            "menu-item"
        })
        .label(i18n::bind(label))
        .enabled(ui.state.map(move |s| enabled(s, remote)))
        .on_press(move || {
            close(&close_ui);
            handler();
        })
}
fn available(s: &State, remote: bool) -> bool {
    workspace::ready(s, remote) && s.file_pending.is_none() && (!remote || !s.system_task_running)
}
fn any(s: &State, remote: bool) -> bool {
    !chosen(s, remote).is_empty()
}
fn one(s: &State, remote: bool) -> bool {
    available(s, remote) && single(s, remote).is_some()
}
fn can_delete(s: &State, remote: bool) -> bool {
    available(s, remote)
        && s.file_pending.is_none()
        && s.cancels.is_empty()
        && !s
            .jobs
            .iter()
            .any(|job| job.status == TransferStatus::Running)
        && any(s, remote)
}
pub(super) fn begin_delete(ui: &Ui, remote: bool) {
    if !ui
        .state
        .with(|s| s.dialog.is_none() && can_delete(s, remote))
    {
        return;
    }
    let targets = ui.state.with(|s| {
        let files = chosen(s, remote);
        files
            .iter()
            .filter(|entry| {
                !files.iter().any(|parent| {
                    parent.path != entry.path
                        && parent.is_dir
                        && !parent.is_symlink
                        && Path::new(&entry.path).starts_with(&parent.path)
                })
            })
            .map(arcnix_core::deletion::Target::from)
            .collect()
    });
    ui.open_dialog(Dialog::DeleteFiles { remote, targets });
}
fn confirm_delete(ui: &Ui) {
    let cancel = Arc::new(AtomicBool::new(false));
    let action = ui.state.with(|s| {
        let Some(Dialog::DeleteFiles { remote, targets }) = &s.dialog else {
            return None;
        };
        (can_delete(s, *remote) && !targets.is_empty()).then(|| Action::Delete {
            remote: *remote,
            targets: targets.clone(),
            cancel: cancel.clone(),
        })
    });
    if let Some(action) = action {
        let serial = ui.send(action);
        ui.state.revise(|s| {
            s.file_pending = Some(serial);
            let remote = matches!(s.dialog, Some(Dialog::DeleteFiles { remote: true, .. }));
            s.deletion = Some(Deletion {
                serial,
                epoch: s.epoch,
                remote,
                cancel,
                progress: None,
            });
            s.dialog_error.clear();
        });
    }
}
pub(super) fn deleted(
    ui: &Ui,
    serial: u64,
    remote: bool,
    result: std::result::Result<usize, String>,
) {
    if !ui.state.with(|s| s.file_pending == Some(serial)) {
        return;
    }
    let canceled = ui.state.with(|s| {
        s.deletion
            .as_ref()
            .is_some_and(|d| d.cancel.load(Ordering::Relaxed))
    });
    let current = ui.state.revise(|s| {
        let current = s
            .deletion
            .as_ref()
            .is_none_or(|d| d.epoch == s.epoch || !d.remote);
        s.deletion = None;
        s.file_pending = None;
        if matches!(s.dialog, Some(Dialog::DeleteFiles { .. })) {
            s.dialog = None;
        }
        s.dialog_error.clear();
        if current {
            file_browser::clear_selection(s, remote);
        }
        current
    });
    match result {
        Ok(count) => ui.log(format!("已删除 {count} 个项目")),
        Err(message) if canceled => ui.log(format!("删除已停止：{message}")),
        Err(message) => ui.error(format!("删除未完成：{message}")),
    }
    if current && (!remote || ui.state.with(|s| s.connected)) {
        ui.refresh(remote);
    }
    ui.schedule();
}
pub(super) fn desktop_menu_open(s: &State) -> bool {
    server_desktop::visible(s)
        && s.workspace_page == WorkspacePage::Explorer
        && s.file_menu == Some(true)
        && s.dialog.is_none()
}

pub(super) fn desktop_menu_dismiss(ui: Ui) -> impl Widget {
    when(ui.state.map(desktop_menu_open), move || {
        let state = ui.state;
        button()
            .class("menu-dismiss")
            .label(i18n::bind("关闭操作菜单"))
            .focus(FocusMode::Programmatic)
            .on(activate, move |_| state.revise(|s| s.file_menu = None))
    })
}

pub(super) fn menu(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    col()
        .class(state.map(move |s| {
            if s.file_menu != Some(remote) {
                "hidden"
            } else if s.file_context.as_ref().is_some_and(|c| c.remote == remote) {
                "pane-menu file-context-menu"
            } else {
                "pane-menu file-actions-menu"
            }
            .to_owned()
        }))
        .focus(FocusMode::Programmatic)
        .name(i18n::bind(if remote {
            "远程文件操作"
        } else {
            "本地文件操作"
        }))
        .fill((
            when(state.map(move |s| has_item(s, remote)), {
                let ui = ui.clone();
                move || {
                    let open = ui.clone();
                    let transfer = ui.clone();
                    let rename = ui.clone();
                    let copy = ui.clone();
                    let info = ui.clone();
                    let delete = ui.clone();
                    col().class("file-menu-group").fill((
                        when(
                            state.map(move |s| {
                                single(s, remote).is_some_and(|f| f.is_dir && !f.is_symlink)
                            }),
                            {
                                let open = open.clone();
                                move || {
                                    action(&open, remote, "打开文件夹", one, {
                                        let open = open.clone();
                                        move || {
                                            if let Some(file) =
                                                open.state.with(|s| single(s, remote))
                                            {
                                                file_browser::activate(&open, remote, &file.path);
                                            }
                                        }
                                    })
                                }
                            },
                        ),
                        when(remote, {
                            let terminal = ui.clone();
                            move || terminal_action(terminal.clone())
                        }),
                        action(
                            &ui,
                            remote,
                            if remote { "下载" } else { "上传" },
                            |s, r| {
                                s.connected
                                    && s.sftp_available
                                    && s.local_ready
                                    && s.remote_ready
                                    && !file_browser::selected_entries(s, r).is_empty()
                            },
                            move || {
                                transfer.transfer(if remote {
                                    TransferDirection::Download
                                } else {
                                    TransferDirection::Upload
                                })
                            },
                        ),
                        action(&ui, remote, "重命名", one, move || {
                            workspace::begin_file(&rename, remote, FileAction::Rename)
                        }),
                        when(
                            state.map(move |s| {
                                remote
                                    && single(s, remote).is_some_and(|f| !f.is_dir && !f.is_symlink)
                            }),
                            {
                                let edit = ui.clone();
                                move || {
                                    action(
                                        &edit,
                                        true,
                                        "Edit file",
                                        |s, _| remote_editor::available(s),
                                        {
                                            let edit = edit.clone();
                                            move || {
                                                if let Some(file) =
                                                    edit.state.with(|s| single(s, true))
                                                {
                                                    remote_editor::edit(&edit, &file.path);
                                                }
                                            }
                                        },
                                    )
                                }
                            },
                        ),
                        action(&ui, remote, "删除", can_delete, move || {
                            begin_delete(&delete, remote)
                        }),
                        action(&ui, remote, "复制所选路径", any, move || {
                            copy_paths(&copy, remote, false)
                        }),
                        action(&ui, remote, "属性", any, move || {
                            properties(&info, remote)
                        }),
                        text().class("menu-divider").content(""),
                    ))
                }
            }),
            when(state.map(move |s| remote && !has_item(s, remote)), {
                let terminal = ui.clone();
                move || terminal_action(terminal.clone())
            }),
            fragment((
                action(&ui, remote, "新建文件", available, {
                    let ui = ui.clone();
                    move || workspace::begin_file(&ui, remote, FileAction::CreateFile)
                }),
                action(&ui, remote, "新建文件夹", available, {
                    let ui = ui.clone();
                    move || workspace::begin_file(&ui, remote, FileAction::CreateFolder)
                }),
            )),
            when(
                state.map(move |s| remote && explorer_ui::item_target(s).is_some()),
                {
                    let ui = ui.clone();
                    move || {
                        let tag = ui.clone();
                        let favorite = ui.clone();
                        fragment((
                            action(
                                &ui,
                                remote,
                                "Toggle favorite",
                                |s, _| explorer_ui::item_target(s).is_some(),
                                move || {
                                    if let Some((path, _)) =
                                        favorite.state.with(explorer_ui::item_target)
                                    {
                                        favorite.toggle_explorer_favorite(path);
                                    }
                                },
                            ),
                            fragment(
                                [("Folder color…", true), ("File color…", false)]
                                    .into_iter()
                                    .map(|(label, directory)| {
                                        let ui = tag.clone();
                                        when(
                                            state.map(move |s| {
                                                explorer_ui::item_target(s)
                                                    .is_some_and(|(_, is_dir)| is_dir == directory)
                                            }),
                                            move || {
                                                let tag = ui.clone();
                                                action(
                                                    &ui,
                                                    remote,
                                                    label,
                                                    |s, _| explorer_ui::item_target(s).is_some(),
                                                    move || {
                                                        if let Some((path, directory)) =
                                                            tag.state.with(explorer_ui::item_target)
                                                        {
                                                            tag.open_dialog(
                                                                Dialog::ExplorerColor {
                                                                    path,
                                                                    directory,
                                                                },
                                                            );
                                                        }
                                                    },
                                                )
                                            },
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            ),
                        ))
                    }
                },
            ),
            when(
                state.map(move |s| {
                    remote
                        && s.workspace_page == WorkspacePage::Explorer
                        && s.file_context.is_none()
                }),
                {
                    let ui = ui.clone();
                    move || {
                        let list = ui.clone();
                        let grid = ui.clone();
                        fragment((
                            action(&ui, remote, "详细列表", workspace::ready, move || {
                                list.state
                                    .revise(|s| file_browser::set_icons(s, remote, false));
                            }),
                            action(&ui, remote, "平铺图标", workspace::ready, move || {
                                grid.state
                                    .revise(|s| file_browser::set_icons(s, remote, true));
                            }),
                        ))
                    }
                },
            ),
            action(&ui, remote, "复制当前目录路径", workspace::ready, {
                let ui = ui.clone();
                move || copy_paths(&ui, remote, true)
            }),
            action(&ui, remote, "显示列", |s, _| s.file_pending.is_none(), {
                let ui = ui.clone();
                move || file_columns::open(&ui, remote)
            }),
            action(
                &ui,
                remote,
                "Transfer defaults",
                |s, _| s.file_pending.is_none() && s.selected.is_some(),
                {
                    let ui = ui.clone();
                    move || ui.open_dialog(Dialog::TransferDefaults)
                },
            ),
            action(
                &ui,
                remote,
                "刷新",
                |s, r| !r || (s.connected && s.sftp_available && !s.system_task_running),
                {
                    let ui = ui.clone();
                    move || ui.refresh(remote)
                },
            ),
            text().class("menu-divider").content(""),
            action(
                &ui,
                remote,
                "全选",
                |s, r| !file_browser::entries(s, r).is_empty(),
                {
                    let ui = ui.clone();
                    move || {
                        ui.state.revise(|s| {
                            let paths = file_browser::entries(s, remote)
                                .into_iter()
                                .map(|f| f.path)
                                .collect();
                            if remote {
                                s.remote_selection = paths
                            } else {
                                s.local_selection = paths
                            }
                        })
                    }
                },
            ),
            action(&ui, remote, "清空选择", any, {
                let ui = ui.clone();
                move || {
                    ui.state
                        .revise(|s| file_browser::clear_selection(s, remote))
                }
            }),
        ))
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    let delete_ui = ui.clone();
    fragment((
        preferences_ui::server_transfer_dialog(ui.clone()),
        file_properties::dialogs(ui),
        when(
            state.map(|s| matches!(s.dialog, Some(Dialog::DeleteFiles { .. }))),
            move || {
                let apply = delete_ui.clone();
                let cancel = delete_ui.clone();
                modal(
                    delete_ui.clone(),
                    "删除文件",
                    col().class("file-delete-body").fill((
                        text()
                            .class("file-task-progress")
                            .content(state.map(progress_text)),
                        text().content(i18n::bind(
                            "永久删除所选项目及文件夹内的内容？此操作无法撤销，不会移入回收站。",
                        )),
                        text()
                            .class("file-properties")
                            .selectable(true)
                            .content(state.map(|s| {
                                if let Some(Dialog::DeleteFiles { remote, targets }) = &s.dialog {
                                    let heading = i18n::translate(if *remote {
                                        "远程文件"
                                    } else {
                                        "本地文件"
                                    });
                                    let paths = targets
                                        .iter()
                                        .map(|t| {
                                            if *remote {
                                                t.path.clone()
                                            } else {
                                                local_display(Path::new(&t.path))
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    format!("{heading} ({})\n\n{paths}", targets.len())
                                } else {
                                    String::new()
                                }
                            })),
                    )),
                    (
                        text_action("取消", Icon::Close)
                            .enabled(true)
                            .on_press(move || cancel.close_dialog()),
                        text_action("永久删除", Icon::Clear)
                            .class("button danger with-icon")
                            .enabled(state.map(|s| {
                                if let Some(Dialog::DeleteFiles { remote, .. }) = &s.dialog {
                                    can_delete(s, *remote)
                                } else {
                                    false
                                }
                            }))
                            .on_press(move || confirm_delete(&apply)),
                    ),
                )
            },
        ),
    ))
}
