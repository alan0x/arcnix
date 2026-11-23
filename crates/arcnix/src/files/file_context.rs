//! Shared file actions for pointer and keyboard context menus.
use crate::*;

#[derive(Clone)]
pub(crate) struct Target {
    remote: bool,
    item: bool,
}
#[derive(Clone)]
pub(crate) struct Deletion {
    serial: u64,
    epoch: u64,
    remote: bool,
    cancel: Arc<AtomicBool>,
    progress: Option<arcnix_core::deletion::Progress>,
    elevated: bool,
}
pub(crate) fn deletion_dialog(s: &State) -> bool {
    s.deletion.is_some() && matches!(s.dialog, Some(Dialog::DeleteFiles { .. }))
}
pub(crate) fn cancel_deletion(ui: &Ui) {
    ui.state.with(|s| {
        if let Some(task) = &s.deletion {
            task.cancel.store(true, Ordering::Relaxed);
        }
    });
    ui.state.revise(|_| {});
}
pub(crate) fn dismiss_deletion(ui: &Ui) -> bool {
    if !ui.state.with(deletion_dialog) {
        return false;
    }
    cancel_deletion(ui);
    ui.state.revise(|s| s.dialog = None);
    true
}
pub(crate) fn deletion_progress(ui: &Ui, serial: u64, progress: arcnix_core::deletion::Progress) {
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
        None if task.elevated => i18n::translate("Deleting as administrator…"),
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
pub(crate) fn task_status(ui: Ui) -> impl Widget {
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
pub(crate) fn chosen(s: &State, remote: bool) -> Vec<FileEntry> {
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
        |s, _| s.terminal_available() && terminal_directory(s).is_some(),
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
pub(crate) fn open(ui: &Ui, remote: bool, path: &str) {
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
pub(crate) fn copy_paths(ui: &Ui, remote: bool, directory: bool) {
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
pub(crate) fn properties(ui: &Ui, remote: bool) {
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
        .label(i18n::workspace_label(ui.state, label))
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
pub(crate) fn can_delete(s: &State, remote: bool) -> bool {
    (!remote || s.local_desktop.is_none())
        && workspace::ready(s, remote)
        && can_change_files(s, remote)
        && any(s, remote)
}
pub(crate) fn can_change_files(s: &State, remote: bool) -> bool {
    !s.demo_read_only()
        && (!remote || (s.connected && s.sftp_available && !s.system_task_running))
        && s.file_pending.is_none()
        && !s.trash[remote as usize].busy()
        && s.cancels.is_empty()
        && !s
            .jobs
            .iter()
            .any(|job| job.status == TransferStatus::Running)
}
pub(crate) fn begin_delete(ui: &Ui, remote: bool) {
    if !ui
        .state
        .with(|s| s.dialog.is_none() && can_delete(s, remote))
    {
        return;
    }
    let targets = ui.state.with(|s| targets(s, remote));
    ui.open_dialog(Dialog::DeleteFiles { remote, targets });
}
pub(crate) fn targets(s: &State, remote: bool) -> Vec<arcnix_core::deletion::Target> {
    let files = file_browser::entries(s, remote);
    file_browser::selected_roots(&files, selection(s, remote), remote)
        .map(arcnix_core::deletion::Target::from)
        .collect()
}
pub(crate) fn confirm_delete(ui: &Ui, sudo: Option<zeroize::Zeroizing<String>>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let elevated = sudo.is_some();
    let action = ui.state.with(|s| {
        let Some(Dialog::DeleteFiles { remote, targets }) = &s.dialog else {
            return None;
        };
        (can_change_files(s, *remote) && !targets.is_empty()).then(|| Action::Delete {
            remote: *remote,
            targets: targets.clone(),
            cancel: cancel.clone(),
            sudo,
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
                elevated,
            });
            s.dialog_error.clear();
        });
    }
}
pub(crate) fn deleted(
    ui: &Ui,
    serial: u64,
    remote: bool,
    result: std::result::Result<usize, worker::FileFailure>,
) {
    if !ui.state.with(|s| s.file_pending == Some(serial)) {
        return;
    }
    let canceled = ui.state.with(|s| {
        s.deletion
            .as_ref()
            .is_some_and(|d| d.cancel.load(Ordering::Relaxed))
    });
    let finished = ui.state.revise(|s| {
        let current = s
            .deletion
            .as_ref()
            .is_none_or(|d| d.epoch == s.epoch || !d.remote);
        s.deletion = None;
        s.file_pending = None;
        if remote
            && current
            && !canceled
            && s.connected
            && result.as_ref().is_err_and(|error| error.permission_denied)
            && matches!(s.dialog, Some(Dialog::DeleteFiles { remote: true, .. }))
            && let Some(Dialog::DeleteFiles {
                remote: true,
                targets,
            }) = s.dialog.take()
        {
            file_elevation::show(s, file_elevation::Retry::Delete(targets));
            return None;
        }
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
        Some((current, removed))
    });
    let Some((current, removed)) = finished else {
        ui.schedule();
        return;
    };
    files_ui::check_permanently_deleted(ui, &removed);
    match result {
        Ok(count) => ui.log(format!("已删除 {count} 个项目")),
        Err(error) if canceled => ui.log(format!("删除已停止：{}", error.message)),
        Err(error) => ui.error(format!("删除未完成：{}", error.message)),
    }
    if current && (!remote || ui.state.with(|s| s.connected)) {
        ui.refresh(remote);
        if remote {
            ui.refresh_desktop_items(true);
        }
    }
    ui.schedule();
}
pub(crate) fn desktop_menu_open(s: &State) -> bool {
    server_desktop::visible(s)
        && s.workspace_page.is_files()
        && s.file_menu == Some(true)
        && s.dialog.is_none()
}

pub(crate) fn desktop_menu_dismiss(ui: Ui) -> impl Widget {
    when(ui.state.map(desktop_menu_open), move || {
        let state = ui.state;
        button()
            .class("menu-dismiss")
            .label(i18n::workspace_label(state, "关闭操作菜单"))
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

fn folder_terminal_action(ui: &Ui, label: &'static str, new_window: bool) -> impl Widget + use<> {
    let terminal = ui.clone();
    action(
        ui,
        true,
        label,
        |s, _| s.terminal_available() && terminal_directory(s).is_some(),
        move || {
            if let Some(directory) = terminal.state.with(terminal_directory) {
                terminal.terminal_here_in(directory, new_window);
            }
        },
    )
}

fn open_actions(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    when(
        state.map(move |s| {
            remote
                && has_item(s, remote)
                && single(s, remote)
                    .is_some_and(|f| s.workspace_page.is_files() || (!f.is_symlink && !f.is_dir))
        }),
        move || {
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
                    state.map(move |s| {
                        remote
                            && s.workspace_page.is_files()
                            && single(s, true).is_some_and(|f| f.is_dir && !f.is_symlink)
                    }),
                    move || {
                        let tab = new_tab.clone();
                        let window = new_tab.clone();
                        file_open::submenu(
                            state,
                            "Open in",
                            state.map(|s| one(s, true)),
                            (
                                action(&new_tab, true, "New tab", one, move || {
                                    if let Some((id, path)) = state.with(|s| {
                                        Some((
                                            s.workspace_page.files_window_id()?,
                                            single(s, true)?.path,
                                        ))
                                    }) {
                                        files_tabs::open_tab(&tab, id, Some(path));
                                    }
                                }),
                                action(&new_tab, true, "New window", one, move || {
                                    if let Some(file) = state.with(|s| single(s, true)) {
                                        files_tabs::new_window(
                                            &window,
                                            files_tabs::WindowStart::Directory(file.path),
                                        );
                                    }
                                }),
                                folder_terminal_action(&new_tab, "New Terminal tab", false),
                                folder_terminal_action(&new_tab, "New Terminal window", true),
                            ),
                        )
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
                        ("Make alias", file_actions::make_alias, true),
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
                            .label(i18n::workspace_label(state, label))
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
        when(
            state.map(move |s| {
                remote
                    && !(s.workspace_page.is_files()
                        && single(s, true).is_some_and(|f| f.is_dir && !f.is_symlink))
            }),
            move || terminal_action(ui.clone()),
        ),
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

pub(crate) fn menu(ui: Ui, remote: bool, files: bool) -> impl Widget {
    let state = ui.state;
    let view = ui.clone();
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
        .name(i18n::bind_in(
            files,
            state.map(move |s| {
                if files && s.file_context.is_none() {
                    "File view options"
                } else if remote {
                    "远程文件操作"
                } else {
                    "本地文件操作"
                }
                .to_owned()
            }),
        ))
        .fill(
            when(
                state.map(move |s| files && s.file_context.is_none()),
                move || files_view::options(view.clone()),
            )
            .otherwise(move || context_actions(ui.clone(), remote)),
        )
}

fn context_actions(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let item = state.map(move |s| has_item(s, remote));
    let directory = state.map(move |s| directory_menu(s, remote));
    let tools = ui.clone();
    let remove = ui.clone();
    fragment((
        open_actions(ui.clone(), remote),
        when(
            state.map(move |s| remote && s.workspace_page.is_files() && has_item(s, remote)),
            {
                let ui = ui.clone();
                move || edit_actions(ui.clone(), remote)
            },
        ),
        when(item, {
            let ui = ui.clone();
            move || item_actions(ui.clone(), remote)
        }),
        when(directory, {
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
const DELETE_PREVIEW: usize = 3;
const DELETE_PAGE: usize = 25;

fn delete_path_parts(path: &str, remote: bool) -> (&str, &str) {
    if remote {
        return match path.rsplit_once('/') {
            Some(("", name)) => ("/", name),
            Some(parts) => parts,
            None => (".", path),
        };
    }
    let local = Path::new(path);
    (
        local.parent().and_then(Path::to_str).unwrap_or("."),
        local
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path),
    )
}

fn delete_review(state: Cage<State>, desktop: bool) -> impl Widget {
    widget(move |scope| {
        let Some((remote, count, folder_contents, name, parent)) = state.with(|s| {
            let Some(Dialog::DeleteFiles { remote, targets }) = &s.dialog else {
                return None;
            };
            let first = targets.first()?;
            let (parent, name) = delete_path_parts(&first.path, *remote);
            let shared = targets
                .iter()
                .skip(1)
                .all(|t| delete_path_parts(&t.path, *remote).0 == parent);
            Some((
                *remote,
                targets.len(),
                targets.iter().any(|t| t.directory && !t.symlink),
                (targets.len() == 1).then(|| name.to_owned()),
                shared.then(|| {
                    if *remote {
                        parent.to_owned()
                    } else {
                        local_display(Path::new(parent))
                    }
                }),
            ))
        }) else {
            return Ok(());
        };
        let icon = match (count, folder_contents) {
            (1, true) => Icon::Folder,
            (1, false) => Icon::File,
            _ => Icon::List,
        };
        let heading = match name {
            Some(name) => Vary::Value(name),
            None if remote => i18n::bind_in(desktop, format!("{count} 个远程项目")),
            None => i18n::bind_in(desktop, format!("{count} 个本地项目")),
        };
        let shared_parent = parent.is_some();
        let location = parent.map(|parent| {
            let source = i18n::bind_in(
                desktop,
                if remote {
                    "远程文件"
                } else {
                    "本地文件"
                },
            );
            scope.bond(move || {
                if count == 1 {
                    format!("{} · {parent}", source.get())
                } else {
                    parent.clone()
                }
            })
        });
        let warning = if folder_contents {
            "文件夹内的内容也会删除。此操作无法撤销。"
        } else {
            "此操作无法撤销。"
        };
        let page = scope.cage(None::<usize>);
        let paths = page.map(move |page| {
            let (start, limit) =
                page.map_or((0, DELETE_PREVIEW), |n| (n * DELETE_PAGE, DELETE_PAGE));
            state.with(|s| {
                let Some(Dialog::DeleteFiles { targets, .. }) = &s.dialog else {
                    return Vec::new();
                };
                targets
                    .iter()
                    .skip(start)
                    .take(limit)
                    .map(|target| {
                        if shared_parent {
                            delete_path_parts(&target.path, remote).1.to_owned()
                        } else if remote {
                            target.path.clone()
                        } else {
                            local_display(Path::new(&target.path))
                        }
                    })
                    .collect::<Vec<_>>()
            })
        });
        col()
            .class("file-delete-review")
            .fill((
                row().class("file-delete-summary").fill((
                    icons::glyph(icon),
                    col().class("file-delete-labels").fill((
                        text()
                            .class("file-delete-name")
                            .selectable(true)
                            .content(heading),
                        location.map(|path| {
                            text()
                                .class("file-delete-location")
                                .selectable(true)
                                .content(path)
                        }),
                    )),
                )),
                when(count > 1, move || {
                    col()
                        .class("file-delete-list")
                        .fill(each(paths).key(Clone::clone).children(|item| {
                            text()
                                .class("file-delete-path")
                                .selectable(true)
                                .content(item.value.get())
                        }))
                }),
                when(count > DELETE_PREVIEW, move || {
                    row().class("file-delete-navigation").fill((
                        sapid::button()
                            .class("button file-delete-toggle")
                            .label(i18n::bind_in(
                                desktop,
                                page.map(move |page| {
                                    if page.is_some() {
                                        "收起清单".to_owned()
                                    } else {
                                        format!("查看全部 {count} 项")
                                    }
                                }),
                            ))
                            .pressed(page.map(Option::is_some))
                            .on_press(move || {
                                page.revise(|p| *p = if p.is_some() { None } else { Some(0) })
                            }),
                        when(
                            page.map(move |p| p.is_some() && count > DELETE_PAGE),
                            move || {
                                row().class("file-delete-pages").fill((
                                    sapid::button()
                                        .class("button")
                                        .label(i18n::bind_in(desktop, "上一页"))
                                        .enabled(page.map(|p| p.unwrap_or(0) > 0))
                                        .on_press(move || {
                                            page.revise(|p| *p = p.map(|n| n.saturating_sub(1)))
                                        }),
                                    text().content(page.map(move |p| {
                                        format!(
                                            "{} / {}",
                                            p.unwrap_or(0) + 1,
                                            count.div_ceil(DELETE_PAGE)
                                        )
                                    })),
                                    sapid::button()
                                        .class("button")
                                        .label(i18n::bind_in(desktop, "下一页"))
                                        .enabled(page.map(move |p| {
                                            p.unwrap_or(0) + 1 < count.div_ceil(DELETE_PAGE)
                                        }))
                                        .on_press(move || page.revise(|p| *p = p.map(|n| n + 1))),
                                ))
                            },
                        ),
                    ))
                }),
                text()
                    .class("file-delete-warning")
                    .content(i18n::bind_in(desktop, warning)),
            ))
            .show_in(scope)
    })
}

pub(crate) fn dialogs(ui: Ui) -> impl Widget {
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
                let desktop = state.with(server_desktop::owns_dialog);
                modal(
                    delete_ui.clone(),
                    "永久删除",
                    when(state.map(deletion_dialog), move || {
                        text()
                            .class("file-task-progress")
                            .content(state.map(progress_text))
                    })
                    .otherwise(move || delete_review(state, desktop)),
                    (
                        sapid::button()
                            .class("button")
                            .label(i18n::bind_in(
                                desktop,
                                state.map(|s| {
                                    if deletion_dialog(s) {
                                        "停止删除"
                                    } else {
                                        "取消"
                                    }
                                    .to_owned()
                                }),
                            ))
                            .on_press(move || cancel.close_dialog()),
                        sapid::button()
                            .label(i18n::bind_in(desktop, "永久删除"))
                            .class("button danger")
                            .enabled(state.map(|s| {
                                if let Some(Dialog::DeleteFiles { remote, .. }) = &s.dialog {
                                    can_change_files(s, *remote)
                                } else {
                                    false
                                }
                            }))
                            .on_press(move || confirm_delete(&apply, None)),
                    ),
                )
            },
        ),
    ))
}
