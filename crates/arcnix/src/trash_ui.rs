use super::*;
use arcnix_core::recycle::{Item, Operation};

#[derive(Clone, Default)]
pub(super) struct Bin {
    items: Vec<Item>,
    pending: Option<(u64, bool)>,
    confirm: Option<Item>,
    menu: Option<TrashMenu>,
    error: String,
}

impl Bin {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

fn visible(s: &State, remote: bool) -> bool {
    s.dialog == Some(Dialog::Trash { remote })
        || (remote && server_desktop::visible(s) && explorer_tabs::trash_active(s))
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
        bin.menu = None;
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
    if remote {
        if ui.state.with(|s| s.file_pending.is_some()) {
            return;
        }
        if !ui.state.with(|s| s.workspace_page.is_explorer()) {
            ui.open_desktop_app(WorkspacePage::Explorer(0));
        }
        ui.state.revise(|s| {
            s.workbench.files.trash = true;
            s.server_apps.explorer_tag = None;
            s.server_apps.explorer_open_pending = None;
            s.file_context = None;
            s.file_menu = None;
            s.trash[1].menu = None;
        });
        enter(ui);
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

pub(super) fn location_tools(ui: Ui) -> impl Widget {
    let state = ui.state;
    row().class("trash-location-tools").fill((
        icon_action("Back to folder", Icon::ChevronRight)
            .class("icon-button explorer-history-back")
            .on_press(move || {
                state.revise(|s| {
                    s.workbench.files.trash = false;
                    s.trash[1].menu = None;
                    s.trash[1].confirm = None;
                })
            }),
        icons::glyph(Icon::Clear),
        text().content(i18n::bind("回收站")),
        icon_action("刷新", Icon::Refresh)
            .enabled(state.map(|s| available(s, true)))
            .on_press(move || refresh(&ui, true)),
    ))
}

fn refresh(ui: &Ui, remote: bool) {
    ui.state.revise(|s| s.trash[remote as usize].error.clear());
    request(ui, remote, Operation::List);
}

#[derive(Clone, serde::Deserialize)]
struct TrashMenu {
    key: String,
    x: u32,
    y: u32,
}

fn item_key(item: &Item) -> String {
    item.payload
        .as_ref()
        .map(|t| t.path.clone())
        .unwrap_or_else(|| item.id.clone())
}

fn menu(ui: Ui) -> impl Widget {
    let state = ui.state;
    let listen = ui.clone();
    let restore = ui.clone();
    let remove = ui.clone();
    fragment((
        input()
            .class("files-trash-input hidden")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    let menu = serde_json::from_str::<TrashMenu>(&value).ok();
                    listen.state.revise(|s| {
                        if explorer_tabs::trash_active(s) && s.dialog.is_none() {
                            s.trash[1].menu = menu
                                .filter(|m| s.trash[1].items.iter().any(|i| item_key(i) == m.key));
                        }
                    });
                }
            }),
        when(state.map(|s| s.trash[1].menu.is_some()), move || {
            let remove = remove.clone();
            let restore = restore.clone();
            col().class("pane-menu trash-item-menu").fill((
                text()
                    .class("trash-menu-position hidden")
                    .content(state.map(|s| {
                        s.trash[1]
                            .menu
                            .as_ref()
                            .map(|m| format!("{},{}", m.x, m.y))
                            .unwrap_or_default()
                    })),
                btn("还原", move || {
                    if let Some(item) = state.with(menu_item) {
                        request(&restore, true, Operation::Restore(item));
                    }
                })
                .enabled(state.map(|s| {
                    available(s, true) && menu_item(s).is_some_and(|i| !i.original.is_empty())
                })),
                text().class("menu-divider").content(""),
                btn("永久删除", move || {
                    remove.state.revise(|s| {
                        s.trash[1].confirm = menu_item(s);
                        s.trash[1].menu = None;
                    })
                })
                .enabled(state.map(|s| available(s, true))),
            ))
        }),
    ))
}

fn menu_item(s: &State) -> Option<Item> {
    let key = &s.trash[1].menu.as_ref()?.key;
    s.trash[1]
        .items
        .iter()
        .find(|i| item_key(i) == *key)
        .cloned()
}

pub(super) fn panel(ui: Ui, remote: bool, in_files: bool) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    let remove = ui.clone();
    let controls = state.map(move |s| available(s, remote));
    col()
        .class(if in_files {
            "trash-panel files-trash"
        } else {
            "trash-panel"
        })
        .fill((
            when(!in_files, move || {
                let ui = refresh.clone();
                row().class("trash-controls").fill(
                    text_action("刷新", Icon::Refresh)
                        .enabled(controls)
                        .on_press(move || self::refresh(&ui, remote)),
                )
            }),
            when(in_files, {
                let ui = ui.clone();
                move || menu(ui.clone())
            }),
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
            when(
                state.map(move |s| in_files && !s.trash[remote as usize].items.is_empty()),
                || {
                    row().class("trash-columns").fill((
                        text().content(i18n::bind("名称")),
                        text().content(i18n::bind("Original path")),
                        text().content(i18n::bind("Date deleted")),
                    ))
                },
            ),
            col().class("trash-items").fill(
                each(state.map(move |s| s.trash[remote as usize].items.clone()))
                    .key(item_key)
                    .children(move |row_item| {
                        let item = row_item.value;
                        let restore = ui.clone();
                        let body = if in_files {
                            let directory =
                                item.get().payload.is_some_and(|target| target.directory);
                            fragment(
                                row()
                                    .class("trash-file-row")
                                    .focus(FocusMode::TabStop)
                                    .fill((
                                        row().class("trash-file-name").fill((
                                            icons::glyph(if directory {
                                                Icon::Folder
                                            } else {
                                                Icon::File
                                            }),
                                            text().content(item.map(|i| {
                                                i.original
                                                    .rsplit('/')
                                                    .next()
                                                    .filter(|name| !name.is_empty())
                                                    .unwrap_or(&i.id)
                                                    .to_owned()
                                            })),
                                        )),
                                        text()
                                            .class("trash-original")
                                            .content(item.map(display_name)),
                                        text()
                                            .class("subtle")
                                            .content(item.map(|i| i.deleted.replace('T', " "))),
                                    )),
                            )
                        } else {
                            fragment((
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
                                .enabled(state.map(move |s| {
                                    available(s, remote) && !item.get().original.is_empty()
                                })),
                                text_action("永久删除", Icon::Clear)
                                    .enabled(controls)
                                    .on_press(move || {
                                        state.revise(|s| {
                                            s.trash[remote as usize].confirm = Some(item.get())
                                        })
                                    }),
                            ))
                        };
                        row().class("trash-item").fill((
                            text()
                                .class("trash-item-key hidden")
                                .content(item.map(item_key)),
                            body,
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
                panel(ui.clone(), remote, false),
                btn("关闭", move || close.close_dialog()),
            )
        },
    )
}
