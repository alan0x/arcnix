use arcnix_core::recycle::{Item, Operation};

use crate::*;

#[derive(Clone)]
enum Confirmation {
    Item(Item),
    Empty,
}

#[derive(Clone, Default)]
pub(crate) struct Bin {
    items: Vec<Item>,
    pending: Option<(u64, bool, Option<arcnix_core::deletion::Target>)>,
    confirm: Option<Confirmation>,
    purge_queue: std::collections::VecDeque<Item>,
    error: String,
}

impl Bin {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

fn visible(s: &State, remote: bool) -> bool {
    s.dialog == Some(Dialog::Trash { remote })
        || (remote && server_desktop::visible(s) && files_tabs::trash_active(s))
}

fn available(s: &State, remote: bool) -> bool {
    !s.demo_read_only()
        && (!remote
            || (s.local_desktop.is_none()
                && s.connected
                && s.sftp_available
                && !s.system_task_running))
        && s.file_pending.is_none()
        && s.trash[remote as usize].pending.is_none()
}

fn request(ui: &Ui, remote: bool, operation: Operation) {
    if !ui.state.with(|s| available(s, remote)) {
        return;
    }
    let list = matches!(operation, Operation::List);
    let purged = match &operation {
        Operation::Purge(item) if remote => item.payload.clone().map(|mut target| {
            target.path.clone_from(&item.original);
            target
        }),
        _ => None,
    };
    let serial = ui.send(Action::Trash { remote, operation });
    ui.state.revise(|s| {
        let bin = &mut s.trash[remote as usize];
        bin.confirm = None;
        bin.pending = Some((serial, list, purged));
        if !list {
            s.file_pending = Some(serial);
        }
    });
}

pub(crate) fn desktop_file(ui: &Ui, file: &FileEntry, permanent: bool) {
    if !ui.state.with(|s| {
        s.local_desktop.is_none() && s.dialog.is_none() && file_context::can_change_files(s, true)
    }) {
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

pub(crate) fn move_items(ui: &Ui, remote: bool) {
    if !ui
        .state
        .with(|s| s.dialog.is_none() && file_context::can_delete(s, remote))
    {
        return;
    }
    let targets = ui.state.with(|s| file_context::targets(s, remote));
    move_targets(ui, remote, targets);
}

pub(crate) fn move_targets(ui: &Ui, remote: bool, targets: Vec<arcnix_core::deletion::Target>) {
    if targets.is_empty() {
        return;
    }
    request(ui, remote, Operation::Move(targets));
}

pub(crate) fn enter(ui: &Ui) {
    if ui.state.with(|s| visible(s, true) && s.dialog.is_none()) {
        request(ui, true, Operation::List);
    }
}

pub(crate) fn open(ui: &Ui, remote: bool) {
    if remote && ui.state.with(|s| s.local_desktop.is_some()) {
        return;
    }
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
        if !ui.state.with(|s| s.workspace_page.is_files()) {
            ui.open_desktop_app(WorkspacePage::Files(0));
        }
        ui.state.revise(|s| {
            s.workbench.files.location = files_tabs::Location::Trash;
            s.server_apps.files_tag = None;
            s.server_apps.files_open_pending = None;
            s.file_context = None;
            s.file_menu = None;
        });
        enter(ui);
    } else {
        ui.open_dialog(Dialog::Trash { remote });
        request(ui, remote, Operation::List);
    }
}

pub(crate) fn completed(
    ui: &Ui,
    serial: u64,
    remote: bool,
    result: std::result::Result<Vec<Item>, String>,
) {
    let completed = ui.state.revise(|s| {
        if s.file_pending == Some(serial) {
            s.file_pending = None;
        }
        let bin = &mut s.trash[remote as usize];
        if bin
            .pending
            .as_ref()
            .is_none_or(|(pending, ..)| *pending != serial)
        {
            return None;
        }
        bin.pending.take()
    });
    let Some((_, list, purged)) = completed else {
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
        Ok(_) => {
            ui.state.revise(|s| s.trash[remote as usize].error.clear());
            if let Some(target) = purged {
                files_ui::check_permanently_deleted(ui, &[target]);
            }
            if purge_next(ui, remote) {
                return;
            }
        }
        Err(error) => {
            ui.state.revise(|s| {
                let bin = &mut s.trash[remote as usize];
                bin.purge_queue.clear();
                bin.error.clone_from(&error);
            });
            if !ui.state.with(|s| visible(s, remote)) {
                ui.error(error);
            }
        }
    }
    if !list && (!remote || ui.state.with(|s| s.connected && s.sftp_available)) {
        ui.refresh(remote);
        if remote {
            files_tabs::refresh_windows(ui);
        }
        if remote && ui.state.with(server_desktop::visible) {
            ui.refresh_desktop_items(true);
        }
        if ui.state.with(|s| visible(s, remote)) {
            request(ui, remote, Operation::List);
        }
    }
}

// Process the confirmed snapshot using the same identity checks as a single purge.
fn purge_next(ui: &Ui, remote: bool) -> bool {
    let next = ui.state.revise(|s| {
        let ready = available(s, remote);
        let bin = &mut s.trash[remote as usize];
        if !ready && !bin.purge_queue.is_empty() {
            bin.purge_queue.clear();
            bin.error = i18n::translate("Empty Trash interrupted");
        }
        bin.purge_queue.pop_front()
    });
    if let Some(item) = next {
        request(ui, remote, Operation::Purge(item));
        return true;
    }
    false
}

fn confirm_removal(ui: &Ui, remote: bool) {
    if !ui.state.with(|s| available(s, remote)) {
        return;
    }
    let confirmation = ui.state.revise(|s| s.trash[remote as usize].confirm.take());
    match confirmation {
        Some(Confirmation::Item(item)) => request(ui, remote, Operation::Purge(item)),
        Some(Confirmation::Empty) => {
            ui.state.revise(|s| {
                let bin = &mut s.trash[remote as usize];
                bin.purge_queue = bin.items.iter().cloned().collect();
            });
            purge_next(ui, remote);
        }
        None => {}
    }
}

fn location_tools(ui: Ui, query: Cage<String>) -> impl Widget {
    let state = ui.state;
    row()
        .class("files-window-tools files-location-toolbar trash-location-tools")
        .fill((
            icon_action("Back to folder", Icon::ChevronRight)
                .class("icon-button files-history-back")
                .on_press(move || {
                    state.revise(|s| {
                        s.workbench.files.location = files_tabs::Location::Directory;
                        s.trash[1].confirm = None;
                    })
                }),
            row().class("files-location-search").fill((
                icons::glyph(Icon::Search),
                sapid::text_field()
                    .unstyled()
                    .name(i18n::desktop::bind("Search Trash"))
                    .placeholder(i18n::desktop::bind("Search Trash"))
                    .value(query)
                    .on_value_change(move |value| query.revise(|current| *current = value)),
            )),
            icon_action("刷新", Icon::Refresh)
                .enabled(state.map(|s| available(s, true)))
                .on_press(move || refresh(&ui, true)),
            i18n::desktop::text_action("Empty Trash", Icon::Clear)
                .enabled(state.map(|s| available(s, true) && !s.trash[1].items.is_empty()))
                .on_press(move || {
                    state.revise(|s| {
                        if available(s, true) && !s.trash[1].items.is_empty() {
                            s.trash[1].confirm = Some(Confirmation::Empty);
                        }
                    })
                }),
        ))
}

fn refresh(ui: &Ui, remote: bool) {
    ui.state.revise(|s| s.trash[remote as usize].error.clear());
    request(ui, remote, Operation::List);
}

fn item_key(item: &Item) -> String {
    item.payload
        .as_ref()
        .map(|t| t.path.clone())
        .unwrap_or_else(|| item.id.clone())
}

pub(crate) fn panel(ui: Ui, remote: bool, in_files: bool) -> impl Widget {
    widget(move |scope| panel_content(scope, ui.clone(), remote, in_files).show_in(scope))
}

fn panel_content(scope: &Scope, ui: Ui, remote: bool, in_files: bool) -> impl Widget + use<> {
    let query = scope.cage(String::new());
    let state = ui.state;
    let items = scope.bond(move || {
        let needle = query.get().trim().to_lowercase();
        state.with(|s| {
            s.trash[remote as usize]
                .items
                .iter()
                .filter(|item| {
                    needle.is_empty()
                        || item.original.to_lowercase().contains(&needle)
                        || (item.original.is_empty() && item.id.to_lowercase().contains(&needle))
                })
                .cloned()
                .collect::<Vec<_>>()
        })
    });
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
            when(in_files, {
                let ui = ui.clone();
                move || location_tools(ui.clone(), query)
            }),
            when(!in_files, move || {
                let ui = refresh.clone();
                row().class("trash-controls").fill(
                    text_action("刷新", Icon::Refresh)
                        .enabled(controls)
                        .on_press(move || self::refresh(&ui, remote)),
                )
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
                    } else if !in_files && bin.items.is_empty() && bin.error.is_empty() {
                        "回收站为空"
                    } else {
                        ""
                    }
                    .to_owned()
                }))),
            when(
                state.map(move |s| {
                    let bin = &s.trash[remote as usize];
                    in_files
                        && (!remote || s.sftp_available)
                        && bin.pending.is_none()
                        && bin.items.is_empty()
                        && bin.error.is_empty()
                }),
                || {
                    col().class("trash-empty").fill((
                        icons::glyph(Icon::Clear),
                        text()
                            .class("trash-empty-title")
                            .content(i18n::bind("回收站为空")),
                    ))
                },
            ),
            when(
                state.map(move |s| s.trash[remote as usize].confirm.is_some()),
                move || {
                    let apply = remove.clone();
                    row().class("trash-confirm").fill((
                        text().content(state.map(move |s| {
                            s.trash[remote as usize]
                                .confirm
                                .as_ref()
                                .map(|confirmation| match confirmation {
                                    Confirmation::Item(item) => format!(
                                        "{}\n{}",
                                        i18n::translate("永久删除此项目？此操作无法撤销。"),
                                        display_name(item)
                                    ),
                                    Confirmation::Empty => i18n::translate_for(
                                        &format!(
                                            "永久删除回收站中的全部 {} 项？此操作无法撤销。",
                                            s.trash[remote as usize].items.len()
                                        ),
                                        i18n::workspace_language(s),
                                    ),
                                })
                                .unwrap_or_default()
                        })),
                        btn("取消", move || {
                            state.revise(|s| s.trash[remote as usize].confirm = None)
                        }),
                        text_action("永久删除", Icon::Clear)
                            .class("button danger with-icon")
                            .enabled(controls)
                            .on_press(move || confirm_removal(&apply, remote)),
                    ))
                },
            ),
            when(
                scope.bond(move || in_files && items.with(|rows| !rows.is_empty())),
                || {
                    row().class("trash-columns").fill((
                        text().content(i18n::bind("名称")),
                        text().content(i18n::bind("Original path")),
                        text().content(i18n::bind("Date deleted")),
                    ))
                },
            ),
            when(
                scope.bond(move || {
                    in_files
                        && items.with(Vec::is_empty)
                        && !query.get().trim().is_empty()
                        && state.with(|s| !s.trash[remote as usize].items.is_empty())
                }),
                || {
                    text()
                        .class("trash-status")
                        .content(i18n::desktop::bind("No matching items"))
                },
            ),
            col()
                .class("trash-items")
                .fill(each(items).key(item_key).children(move |row_item| {
                    let item = row_item.value;
                    let restore = ui.clone();
                    let body = if in_files {
                        fragment(glory::ui::widget(move |scope| {
                            let id = scope.id();
                            let restore = restore.clone();
                            let remove = restore.clone();
                            sapid::context_menu(format!("trash-{}-{}", id.slot(), id.generation()))
                                .name(i18n::bind("回收站"))
                                .target_name(item.map(display_name))
                                .enabled(
                                    state
                                        .map(|s| files_tabs::trash_active(s) && s.dialog.is_none()),
                                )
                                .target(
                                    row().class("trash-file-row").fill((
                                        row().class("trash-file-name").fill((
                                            text()
                                                .class(item.map(|entry| {
                                                    let directory = entry
                                                        .payload
                                                        .as_ref()
                                                        .is_some_and(|target| target.directory);
                                                    let icon = files_search::file_icon(
                                                        files_ui::item_name(&entry.original),
                                                        directory,
                                                    );
                                                    format!(
                                                        "action-glyph file-entry-icon glyph-{}",
                                                        icon.name()
                                                    )
                                                }))
                                                .content(""),
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
                                .item(
                                    sapid::menu_item("restore")
                                        .label(i18n::bind("还原"))
                                        .enabled(state.map(move |s| {
                                            available(s, true) && !item.get().original.is_empty()
                                        }))
                                        .on_activate(move || {
                                            request(&restore, true, Operation::Restore(item.get()))
                                        }),
                                )
                                .separator("actions")
                                .item(
                                    sapid::menu_item("delete-permanently")
                                        .label(i18n::bind("永久删除"))
                                        .enabled(controls)
                                        .on_activate(move || {
                                            if remove.state.with(|s| available(s, true)) {
                                                remove.state.revise(|s| {
                                                    s.trash[1].confirm =
                                                        Some(Confirmation::Item(item.get()))
                                                });
                                            }
                                        }),
                                )
                                .show_in(scope)
                        }))
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
                                        s.trash[remote as usize].confirm =
                                            Some(Confirmation::Item(item.get()))
                                    })
                                }),
                        ))
                    };
                    row().class("trash-item").fill(body)
                })),
        ))
}
fn display_name(item: &Item) -> String {
    if item.original.is_empty() {
        format!("{} · {}", item.id, i18n::translate("原路径不可用"))
    } else {
        item.original.clone()
    }
}
pub(crate) fn dialog(ui: Ui) -> impl Widget {
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
