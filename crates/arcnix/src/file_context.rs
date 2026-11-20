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
            || !(s.workspace_page == WorkspacePage::TransferFiles
                || (remote && s.workspace_page.is_files()))
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
        .class(if label == "永久删除" {
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
pub(super) fn can_delete(s: &State, remote: bool) -> bool {
    workspace::ready(s, remote) && can_change_files(s, remote) && any(s, remote)
}
pub(super) fn can_change_files(s: &State, remote: bool) -> bool {
    (!remote || (s.connected && s.sftp_available && !s.system_task_running))
        && s.file_pending.is_none()
        && !s.trash[remote as usize].busy()
        && s.cancels.is_empty()
        && !s
            .jobs
            .iter()
            .any(|job| job.status == TransferStatus::Running)
}
pub(super) fn begin_delete(ui: &Ui, remote: bool) {
    if !ui
        .state
        .with(|s| s.dialog.is_none() && can_delete(s, remote))
    {
        return;
    }
    let targets = ui.state.with(|s| targets(s, remote));
    ui.open_dialog(Dialog::DeleteFiles { remote, targets });
}
pub(super) fn targets(s: &State, remote: bool) -> Vec<arcnix_core::deletion::Target> {
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
}
fn confirm_delete(ui: &Ui) {
    let cancel = Arc::new(AtomicBool::new(false));
    let action = ui.state.with(|s| {
        let Some(Dialog::DeleteFiles { remote, targets }) = &s.dialog else {
            return None;
        };
        (can_change_files(s, *remote) && !targets.is_empty()).then(|| Action::Delete {
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
    let (current, removed) = ui.state.revise(|s| {
        let current = s
            .deletion
            .as_ref()
            .is_none_or(|d| d.epoch == s.epoch || !d.remote);
        s.deletion = None;
        s.file_pending = None;
        let removed = if matches!(s.dialog, Some(Dialog::DeleteFiles { .. })) {
            match s.dialog.take() {
                Some(Dialog::DeleteFiles { targets, .. })
                    if remote && current && result.is_ok() =>
                {
                    targets
                }
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
        s.dialog_error.clear();
        if current {
            file_browser::clear_selection(s, remote);
        }
        (current, removed)
    });
    files_ui::check_permanently_deleted(ui, &removed);
    match result {
        Ok(count) => ui.log(format!("已删除 {count} 个项目")),
        Err(message) if canceled => ui.log(format!("删除已停止：{message}")),
        Err(message) => ui.error(format!("删除未完成：{message}")),
    }
    if current && (!remote || ui.state.with(|s| s.connected)) {
        ui.refresh(remote);
        if remote {
            ui.refresh_desktop_items(true);
        }
    }
    ui.schedule();
}
pub(super) fn desktop_menu_open(s: &State) -> bool {
    server_desktop::visible(s)
        && s.workspace_page.is_files()
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

fn regular_file(s: &State) -> Option<String> {
    if s.file_menu != Some(true) {
        return None;
    }
    single(s, true)
        .filter(|file| !file.is_dir && !file.is_symlink)
        .map(|file| file.path)
}

fn menu_group(content: impl IntoChildren) -> impl Widget {
    col().class("file-menu-group").fill(content)
}

fn open_actions(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    when(
        state.map(move |s| {
            has_item(s, remote)
                && single(s, remote).is_some_and(|f| {
                    (remote && s.workspace_page.is_files()) || !f.is_symlink && (remote || f.is_dir)
                })
        }),
        move || {
            let folder = ui.clone();
            let new_tab = ui.clone();
            menu_group((
                when(
                    state.map(move |s| {
                        remote
                            && s.workspace_page.is_files()
                            && single(s, true).is_some_and(|file| file.is_symlink)
                    }),
                    {
                        let ui = ui.clone();
                        move || {
                            let open = ui.clone();
                            action(&ui, true, "Open alias", one, move || {
                                if let Some(entry) = state.with(|s| single(s, true)) {
                                    file_actions::open_alias(&open, &entry.path);
                                }
                            })
                        }
                    },
                ),
                when(remote, {
                    let ui = ui.clone();
                    move || file_open::menu(ui.clone(), regular_file, close)
                }),
                when(
                    state
                        .map(move |s| single(s, remote).is_some_and(|f| f.is_dir && !f.is_symlink)),
                    move || {
                        let open = folder.clone();
                        action(&folder, remote, "打开文件夹", one, move || {
                            if let Some(file) = open.state.with(|s| single(s, remote)) {
                                file_browser::activate(&open, remote, &file.path);
                            }
                        })
                    },
                ),
                when(
                    state.map(move |s| {
                        remote
                            && s.workspace_page.is_files()
                            && single(s, true).is_some_and(|f| f.is_dir && !f.is_symlink)
                    }),
                    move || {
                        let tab = new_tab.clone();
                        let window = new_tab.clone();
                        fragment((
                            action(&new_tab, true, "Open in new tab", one, move || {
                                if let Some((id, path)) = state.with(|s| {
                                    Some((
                                        s.workspace_page.files_window_id()?,
                                        single(s, true)?.path,
                                    ))
                                }) {
                                    files_tabs::open_tab(&tab, id, Some(path));
                                }
                            }),
                            action(&new_tab, true, "Open in new window", one, move || {
                                if let Some(file) = state.with(|s| single(s, true)) {
                                    files_tabs::new_window(
                                        &window,
                                        files_tabs::WindowStart::Directory(file.path),
                                    );
                                }
                            }),
                        ))
                    },
                ),
            ))
        },
    )
}

fn edit_actions(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let rename = ui.clone();
    menu_group((
        when(state.map(move |s| remote && s.workspace_page.is_files()), {
            let ui = ui.clone();
            move || {
                fragment(
                    [
                        ("复制", file_actions::copy as fn(&Ui), false),
                        ("Duplicate", file_actions::duplicate, true),
                        ("Make Alias", file_actions::make_alias, true),
                        ("Compress", file_actions::compress, true),
                    ]
                    .into_iter()
                    .map(|(label, run, writes)| {
                        let changed = ui.clone();
                        let enabled = state.map(move |s| {
                            file_actions::has_selection(s)
                                && (!writes || file_actions::available(s))
                        });
                        sapid::button()
                            .class("menu-item")
                            .label(i18n::bind(label))
                            .enabled(enabled)
                            .on_press(move || {
                                close(&changed);
                                run(&changed);
                            })
                    })
                    .collect::<Vec<_>>(),
                )
            }
        }),
        when(state.map(move |s| remote && s.workspace_page.is_files()), {
            let ui = ui.clone();
            move || paste_action(ui.clone())
        }),
        action(
            &ui,
            remote,
            "重命名",
            file_browser::can_rename,
            move || file_browser::begin_rename(&rename, remote),
        ),
    ))
}

fn paste_action(ui: Ui) -> impl Widget {
    let paste = ui.clone();
    action(
        &ui,
        true,
        "Paste",
        |s, _| file_actions::can_paste(s),
        move || file_actions::paste(&paste),
    )
}

fn item_actions(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let transfer = ui.clone();
    let rename = ui.clone();
    let copy = ui.clone();
    let info = ui.clone();
    menu_group((
        when(!remote, {
            let ui = ui.clone();
            move || {
                let transfer = transfer.clone();
                action(
                    &ui,
                    false,
                    "上传",
                    |s, _| {
                        file_browser::transfer_ready(s)
                            && !file_browser::selected_entries(s, false).is_empty()
                    },
                    move || transfer.transfer(TransferDirection::Upload),
                )
            }
        }),
        when(
            state.map(move |s| !remote || !s.workspace_page.is_files()),
            move || {
                let ui = rename.clone();
                action(
                    &rename,
                    remote,
                    "重命名",
                    file_browser::can_rename,
                    move || file_browser::begin_rename(&ui, remote),
                )
            },
        ),
        action(&ui, remote, "复制所选路径", any, move || {
            copy_paths(&copy, remote, false)
        }),
        action(&ui, remote, "属性", any, move || {
            properties(&info, remote)
        }),
        when(remote, move || terminal_action(ui.clone())),
    ))
}

fn remote_tools(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| !has_item(s, true) && terminal_directory(s).is_some()),
        move || menu_group(terminal_action(ui.clone())),
    )
}

fn directory_menu(s: &State, remote: bool) -> bool {
    !s.file_context
        .as_ref()
        .is_some_and(|target| target.remote == remote && target.item)
}

fn directory_actions(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let folder = ui.clone();
    let file = ui.clone();
    let copy = ui.clone();
    let refresh = ui.clone();
    let trash = ui.clone();
    fragment((
        menu_group((
            when(
                state.map(move |s| remote && s.workspace_page.is_files() && !has_item(s, remote)),
                {
                    let ui = ui.clone();
                    move || paste_action(ui.clone())
                },
            ),
            action(
                &ui,
                remote,
                "新建文件夹",
                file_browser::can_create,
                move || file_browser::create(&folder, remote, true),
            ),
            action(
                &ui,
                remote,
                "新建文件",
                file_browser::can_create,
                move || file_browser::create(&file, remote, false),
            ),
        )),
        menu_group((
            action(
                &ui,
                remote,
                "复制当前目录路径",
                workspace::ready,
                move || copy_paths(&copy, remote, true),
            ),
            action(
                &ui,
                remote,
                "刷新",
                |s, r| !r || (s.connected && s.sftp_available && !s.system_task_running),
                move || refresh.refresh(remote),
            ),
            when(
                state.map(move |s| !remote || !s.workspace_page.is_files()),
                move || {
                    let open = trash.clone();
                    action(&trash, remote, "打开回收站", available, move || {
                        trash_ui::open(&open, remote)
                    })
                },
            ),
        )),
    ))
}

fn view_actions(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let columns = ui.clone();
    let defaults = ui.clone();
    menu_group((
        when(
            state.map(move |s| remote && s.workspace_page.is_files() && s.file_context.is_none()),
            {
                let ui = ui.clone();
                move || {
                    fragment((
                        action(&ui, remote, "详细列表", workspace::ready, move || {
                            state.revise(|s| file_browser::set_icons(s, remote, false));
                        }),
                        action(&ui, remote, "平铺图标", workspace::ready, move || {
                            state.revise(|s| file_browser::set_icons(s, remote, true));
                        }),
                    ))
                }
            },
        ),
        action(
            &ui,
            remote,
            "显示列",
            |s, _| s.file_pending.is_none(),
            move || file_columns::open(&columns, remote),
        ),
        when(state.map(|s| !s.workspace_page.is_files()), move || {
            let open = defaults.clone();
            action(
                &defaults,
                remote,
                "Transfer defaults",
                |s, _| s.file_pending.is_none() && s.selected.is_some(),
                move || open.open_dialog(Dialog::TransferDefaults),
            )
        }),
        action(
            &ui,
            remote,
            "全选",
            |s, r| !file_browser::entries(s, r).is_empty(),
            move || {
                state.revise(|s| {
                    let paths = file_browser::entries(s, remote)
                        .into_iter()
                        .map(|f| f.path)
                        .collect();
                    if remote {
                        s.remote_selection = paths;
                    } else {
                        s.local_selection = paths;
                    }
                })
            },
        ),
        when(state.map(move |s| has_item(s, remote)), move || {
            action(&ui, remote, "清空选择", any, move || {
                state.revise(|s| file_browser::clear_selection(s, remote))
            })
        }),
    ))
}

pub(super) fn menu(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let item = state.map(move |s| has_item(s, remote));
    let directory = state.map(move |s| directory_menu(s, remote));
    let tools = ui.clone();
    let remove = ui.clone();
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
            open_actions(ui.clone(), remote),
            when(
                state.map(move |s| remote && s.workspace_page.is_files() && has_item(s, remote)),
                {
                    let ui = ui.clone();
                    move || edit_actions(ui.clone(), remote)
                },
            ),
            when(item.clone(), {
                let ui = ui.clone();
                move || item_actions(ui.clone(), remote)
            }),
            when(directory.clone(), {
                let ui = ui.clone();
                move || directory_actions(ui.clone(), remote)
            }),
            when(remote, move || remote_tools(tools.clone())),
            when(remote, {
                let ui = ui.clone();
                move || files_ui::appearance_menu(ui.clone(), files_ui::item_target, close, true)
            }),
            when(directory, move || view_actions(ui.clone(), remote)),
            when(item, move || {
                let recycle = remove.clone();
                let delete = remove.clone();
                menu_group((
                    action(&remove, remote, "移入回收站", can_delete, move || {
                        trash_ui::move_items(&recycle, remote)
                    }),
                    action(&remove, remote, "永久删除", can_delete, move || {
                        begin_delete(&delete, remote)
                    }),
                ))
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
                    "永久删除",
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
                                    can_change_files(s, *remote)
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
