use std::result::Result;

use arcnix_core::remote::{DesktopDrop, FileAction};
use arcnix_core::server_apps::FolderColor;
use serde::Deserialize;

use crate::*;

pub(crate) fn available(s: &State) -> bool {
    s.workspace_page.is_files()
        && server_desktop::visible(s)
        && s.sftp_available
        && s.dialog.is_none()
        && file_context::can_change_files(s, true)
}

fn selected(s: &State) -> Vec<String> {
    file_context::targets(s, true)
        .into_iter()
        .filter(|item| !item.symlink)
        .map(|item| item.path)
        .collect()
}

pub(crate) fn has_selection(s: &State) -> bool {
    !selected(s).is_empty()
}

pub(crate) fn copy(ui: &Ui) {
    ui.state.revise(|s| {
        let paths = selected(s);
        if s.workspace_page.is_files() && !paths.is_empty() {
            s.workbench.files.clipboard = paths;
        }
    });
}

fn current_folder(s: &State) -> Option<&str> {
    files_tabs::directory(s, s.workspace_page.files_window_id()?)
}

pub(crate) fn can_paste(s: &State) -> bool {
    available(s) && current_folder(s).is_some() && !s.workbench.files.clipboard.is_empty()
}

pub(crate) fn paste(ui: &Ui) {
    let request = ui.state.with(|s| {
        can_paste(s).then(|| {
            (
                current_folder(s).unwrap().to_owned(),
                s.workbench.files.clipboard.clone(),
            )
        })
    });
    if let Some((directory, paths)) = request {
        start(ui, FileAction::Copy(Some(directory)), paths);
    }
}

pub(crate) fn duplicate(ui: &Ui) {
    start(ui, FileAction::Copy(None), ui.state.with(selected));
}

pub(crate) fn make_alias(ui: &Ui) {
    start(ui, FileAction::Alias, ui.state.with(selected));
}

pub(crate) fn compress(ui: &Ui) {
    if let Some((dir, paths)) = ui
        .state
        .with(|s| Some((current_folder(s)?.to_owned(), selected(s))))
    {
        start(ui, FileAction::Compress(dir), paths);
    }
}

fn start(ui: &Ui, action: FileAction, paths: Vec<String>) {
    if paths.is_empty() || !ui.state.with(available) {
        return;
    }
    let serial = ui.send(Action::ManageRemoteFiles(action, paths));
    ui.state.revise(|s| s.file_pending = Some(serial));
}

pub(crate) fn completed(ui: &Ui, serial: u64, result: Result<(), String>) {
    if !release(ui, serial) {
        return;
    }
    match result {
        Ok(()) => ui.log("File operation completed"),
        Err(error) => ui.error(error),
    }
    ui.refresh(true);
    files_tabs::refresh_windows(ui);
    ui.refresh_desktop_items(true);
    ui.schedule();
}

fn release(ui: &Ui, serial: u64) -> bool {
    ui.state.revise(|s| {
        if s.file_pending != Some(serial) {
            return false;
        }
        s.file_pending = None;
        true
    })
}

pub(crate) fn open_alias(ui: &Ui, path: &str) {
    if !ui.state.with(|s| {
        available(s)
            || (server_desktop::visible(s)
                && s.sftp_available
                && s.dialog.is_none()
                && file_context::can_change_files(s, true))
    }) {
        return;
    }
    let serial = ui.send(Action::ResolveFileAlias(path.to_owned()));
    ui.state.revise(|s| s.file_pending = Some(serial));
}

pub(crate) fn alias_resolved(ui: &Ui, serial: u64, result: Result<(String, bool), String>) {
    if !release(ui, serial) {
        return;
    }
    match result {
        Ok((path, true)) => {
            if !ui.state.with(|s| s.workspace_page.is_files()) {
                ui.open_desktop_app(WorkspacePage::Files(0));
            }
            file_browser::navigate(ui, true, path);
        }
        Ok((path, false)) => file_browser::open_remote_file(ui, &path),
        Err(error) => ui.error(error),
    }
    ui.schedule();
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DropTarget {
    Directory { path: Option<String> },
    Location { path: String },
    Trash,
    Tag { color: FolderColor },
    Favorite { anchor: Option<String>, after: bool },
    Desktop { action: DesktopDrop },
    TextEditor,
}

#[derive(Deserialize)]
struct Drop {
    epoch: u64,
    request: u64,
    source: u32,
    source_directory: String,
    path: String,
    copy: bool,
    target: DropTarget,
}

pub(crate) fn drop_input(ui: Ui, window: u32) -> impl Widget {
    input()
        .class("files-item-drop-input hidden")
        .on(value_changed, move |event| {
            if let Event::ValueChanged { value, .. } = event
                && value.len() <= 8192
                && let Ok(drop) = serde_json::from_str::<Drop>(&value)
            {
                drop_files(&ui, window, drop);
            }
        })
}

fn drop_files(ui: &Ui, window: u32, drop: Drop) {
    let valid = ui.state.with(|s| {
        (if matches!(drop.target, DropTarget::TextEditor) {
            text_editor::accepts_drop(s)
        } else {
            available(s)
        }) && s.workbench.window_visible(WorkspacePage::Files(window))
            && s.workbench
                .window_visible(WorkspacePage::Files(drop.source))
            && s.epoch == drop.epoch
            && files_tabs::directory(s, drop.source) == Some(drop.source_directory.as_str())
            && (!s.workbench.files.is_live(drop.source) || s.remote_request == drop.request)
    });
    if !valid {
        return;
    }
    let entries = ui
        .state
        .with(|s| files_tabs::dragged_entries(s, drop.source, &drop.path));
    let paths: Vec<_> = entries
        .iter()
        .filter(|item| !item.symlink)
        .map(|item| item.path.clone())
        .collect();
    match drop.target {
        DropTarget::TextEditor => {
            for entry in entries
                .iter()
                .filter(|entry| !entry.directory && !entry.symlink)
            {
                text_editor::open(ui, &entry.path);
            }
        }
        DropTarget::Desktop { action } => start(ui, FileAction::Desktop(action), paths),
        DropTarget::Trash if !drop.copy => trash_ui::move_targets(ui, true, entries),
        DropTarget::Trash => (),
        DropTarget::Tag { color } => ui.set_file_colors(paths, color),
        DropTarget::Favorite { anchor, after } if !paths.is_empty() => {
            ui.insert_file_favorite(drop.path, anchor.as_deref(), after)
        }
        DropTarget::Favorite { .. } => (),
        target => {
            let directory = ui.state.with(|s| match target {
                DropTarget::Directory { ref path } => {
                    files_tabs::drop_folder(s, window, path.as_deref())
                }
                DropTarget::Location { ref path } => {
                    files_ui::drop_location(s, path).then(|| path.clone())
                }
                _ => None,
            });
            if let Some(directory) = directory {
                let action = if drop.copy {
                    FileAction::Copy(Some(directory))
                } else {
                    FileAction::Move(directory)
                };
                start(ui, action, paths);
            }
        }
    }
}
