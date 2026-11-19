use super::*;
use arcnix_core::recycle::{Item, Operation};

#[derive(Clone, Default)]
pub(super) struct Bin {
    items: Vec<Item>,
    pending: Option<(u64, bool)>,
    confirm: Option<Item>,
    error: String,
}

impl Bin {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

fn visible(s: &State, remote: bool) -> bool {
    s.dialog == Some(Dialog::Trash { remote })
        || (remote && server_desktop::visible(s) && s.workspace_page == WorkspacePage::Trash)
}

fn available(s: &State, remote: bool) -> bool {
    (!remote || (s.connected && s.sftp_available && !s.system_task_running))
        && s.file_pending.is_none()
        && s.trash[remote as usize].pending.is_none()
}

fn request(ui: &Ui, remote: bool, operation: Operation) {
    if !ui.state.with(|s| available(s, remote)) {
        return;
    }
    let list = matches!(operation, Operation::List);
    let serial = ui.send(Action::Trash { remote, operation });
    ui.state.revise(|s| {
        let bin = &mut s.trash[remote as usize];
        bin.confirm = None;
        bin.pending = Some((serial, list));
        if !list {
            s.file_pending = Some(serial);
        }
    });
}

pub(super) fn desktop_file(ui: &Ui, file: &FileEntry, permanent: bool) {
    if !ui
        .state
        .with(|s| s.dialog.is_none() && file_context::can_change_files(s, true))
    {
        return;
    }
    let targets = vec![arcnix_core::deletion::Target::from(file)];
    if permanent {
        ui.open_dialog(Dialog::DeleteFiles {
            remote: true,
            targets,
        });
    } else {
        request(ui, true, Operation::Move(targets));
    }
}

pub(super) fn move_items(ui: &Ui, remote: bool) {
    if !ui
        .state
        .with(|s| s.dialog.is_none() && file_context::can_delete(s, remote))
    {
        return;
    }
    let targets = ui.state.with(|s| file_context::targets(s, remote));
    request(ui, remote, Operation::Move(targets));
}

pub(super) fn enter(ui: &Ui) {
    if ui.state.with(|s| visible(s, true) && s.dialog.is_none()) {
        request(ui, true, Operation::List);
    }
}

pub(super) fn open(ui: &Ui, remote: bool) {
    #[cfg(target_os = "macos")]
    if !remote {
        if let Err(error) = std::process::Command::new("osascript")
            .args(["-e", "tell application \"Finder\" to open trash"])
            .spawn()
        {
            ui.error(error.to_string());
        }
        return;
    }
    if remote && ui.state.with(server_desktop::visible) {
        ui.open_desktop_app(WorkspacePage::Trash);
    } else {
        ui.open_dialog(Dialog::Trash { remote });
        request(ui, remote, Operation::List);
    }
}

pub(super) fn completed(
    ui: &Ui,
    serial: u64,
    remote: bool,
    result: std::result::Result<Vec<Item>, String>,
) {
    let list = ui.state.revise(|s| {
        if s.file_pending == Some(serial) {
            s.file_pending = None;
        }
        let bin = &mut s.trash[remote as usize];
        let (pending, list) = bin.pending?;
        if pending != serial {
            return None;
        }
        bin.pending = None;
        Some(list)
    });
    let Some(list) = list else {
        return;
    };
    match result {
        Ok(mut items) if list => {
            items.sort_by(|a, b| {
                b.deleted
                    .cmp(&a.deleted)
                    .then_with(|| a.original.cmp(&b.original))
                    .then_with(|| a.id.cmp(&b.id))
            });
            ui.state.revise(|s| s.trash[remote as usize].items = items);
        }
        Ok(_) => ui.state.revise(|s| s.trash[remote as usize].error.clear()),
        Err(error) => {
            ui.state
                .revise(|s| s.trash[remote as usize].error = error.clone());
            if !ui.state.with(|s| visible(s, remote)) {
                ui.error(error);
            }
        }
    }
    if !list && (!remote || ui.state.with(|s| s.connected && s.sftp_available)) {
        ui.refresh(remote);
        if remote && ui.state.with(server_desktop::visible) {
            ui.refresh_desktop_items(true);
        }
        if ui.state.with(|s| visible(s, remote)) {
            request(ui, remote, Operation::List);
        }
    }
}

pub(super) fn panel(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    let remove = ui.clone();
    let controls = state.map(move |s| available(s, remote));
    col().class("trash-panel").fill((
        row().class("trash-controls").fill(
            text_action("刷新", Icon::Refresh)
                .enabled(controls)
                .on_press(move || {
                    refresh
                        .state
                        .revise(|s| s.trash[remote as usize].error.clear());
                    request(&refresh, remote, Operation::List)
                }),
        ),
        text().class("trash-error").content(i18n::bind(
            state.map(move |s| s.trash[remote as usize].error.clone()),
        )),
        text()
            .class("trash-status")
            .content(i18n::bind(state.map(move |s| {
                let bin = &s.trash[remote as usize];
                if remote && !s.sftp_available {
                    "SFTP is unavailable"
                } else if bin.pending.is_some() {
                    "正在处理…"
                } else if bin.items.is_empty() && bin.error.is_empty() {
                    "回收站为空"
                } else {
                    ""
                }
                .to_owned()
            }))),
        when(
            state.map(move |s| s.trash[remote as usize].confirm.is_some()),
            move || {
                let apply = remove.clone();
                row().class("trash-confirm").fill((
                    text().content(state.map(move |s| {
                        s.trash[remote as usize]
                            .confirm
                            .as_ref()
                            .map(|i| {
                                format!(
                                    "{}\n{}",
                                    i18n::translate("永久删除此项目？此操作无法撤销。"),
                                    display_name(i)
                                )
                            })
                            .unwrap_or_default()
                    })),
                    btn("取消", move || {
                        state.revise(|s| s.trash[remote as usize].confirm = None)
                    }),
                    text_action("永久删除", Icon::Clear)
                        .class("button danger with-icon")
                        .enabled(controls)
                        .on_press(move || {
                            if let Some(item) =
                                state.with(|s| s.trash[remote as usize].confirm.clone())
                            {
                                request(&apply, remote, Operation::Purge(item));
                            }
                        }),
                ))
            },
        ),
        col().class("trash-items").fill(
            each(state.map(move |s| s.trash[remote as usize].items.clone()))
                .key(|item| {
                    item.payload
                        .as_ref()
                        .map(|t| t.path.clone())
                        .unwrap_or_else(|| item.id.clone())
                })
                .children(move |row_item| {
                    let item = row_item.value;
                    let restore = ui.clone();
                    row().class("trash-item").fill((
                        col().class("trash-details").fill((
                            text()
                                .class("trash-original")
                                .content(item.map(display_name)),
                            text()
                                .class("subtle")
                                .content(item.map(|i| i.deleted.replace('T', " "))),
                        )),
                        btn("还原", move || {
                            request(&restore, remote, Operation::Restore(item.get()))
                        })
                        .enabled(
                            state.map(move |s| {
                                available(s, remote) && !item.get().original.is_empty()
                            }),
                        ),
                        text_action("永久删除", Icon::Clear)
                            .enabled(controls)
                            .on_press(move || {
                                state
                                    .revise(|s| s.trash[remote as usize].confirm = Some(item.get()))
                            }),
                    ))
                }),
        ),
    ))
}
fn display_name(item: &Item) -> String {
    if item.original.is_empty() {
        format!("{} · {}", item.id, i18n::translate("原路径不可用"))
    } else {
        item.original.clone()
    }
}
pub(super) fn dialog(ui: Ui) -> impl Widget {
    when(
        ui.state
            .map(|s| matches!(s.dialog, Some(Dialog::Trash { .. }))),
        move || {
            let remote = ui
                .state
                .with(|s| s.dialog == Some(Dialog::Trash { remote: true }));
            let close = ui.clone();
            modal(
                ui.clone(),
                if remote {
                    "远程回收站"
                } else {
                    "本地回收站"
                },
                panel(ui.clone(), remote),
                btn("关闭", move || close.close_dialog()),
            )
        },
    )
}
