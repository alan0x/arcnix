//! An in-memory editor. Remote contents are never put in configuration or logs.
use arcnix_core::text_files::Document;

use super::*;
#[derive(Clone)]
pub(super) struct Editor {
    epoch: u64,
    path: String,
    document: Option<Document>,
    draft: String,
    pending: Option<u64>,
    cancel: Arc<AtomicBool>,
    dirty: bool,
    discard: bool,
    error: String,
    saved: bool,
}
pub(super) fn busy(s: &State) -> bool {
    s.remote_editor
        .as_ref()
        .is_some_and(|e| e.pending.is_some())
}
pub(super) fn available(s: &State) -> bool {
    s.connected
        && s.sftp_available
        && s.file_pending.is_none()
        && !s.system_task_running
        && s.cancels.is_empty()
        && s.remote_editor.is_none()
}
pub(super) fn open(ui: &Ui, path: &str) {
    if !ui.state.with(|s| {
        available(s)
            && file_browser::entries(s, true)
                .iter()
                .any(|f| f.path == path && !f.is_dir && !f.is_symlink)
    }) {
        return;
    }
    ui.state.revise(|s| {
        s.remote_editor = Some(Editor {
            epoch: s.epoch,
            path: path.into(),
            document: None,
            draft: String::new(),
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            dirty: false,
            discard: false,
            error: String::new(),
            saved: false,
        })
    });
    ui.open_dialog(Dialog::RemoteEditor);
    read(ui);
}
fn read(ui: &Ui) {
    let task = ui.state.revise(|s| {
        let editor = s.remote_editor.as_mut()?;
        if editor.dirty || editor.pending.is_some() || !s.connected {
            return None;
        }
        editor.epoch = s.epoch;
        editor.cancel = Arc::new(AtomicBool::new(false));
        Some((editor.path.clone(), editor.cancel.clone()))
    });
    if let Some((path, cancel)) = task {
        let serial = ui.send(Action::TextRead { path, cancel });
        ui.state.revise(|s| {
            s.file_pending = Some(serial);
            if let Some(e) = &mut s.remote_editor {
                e.pending = Some(serial);
                e.error.clear();
            }
        });
    }
}
pub(super) fn cancel(ui: &Ui) {
    ui.state.with(|s| {
        if let Some(e) = &s.remote_editor {
            e.cancel.store(true, Ordering::Relaxed);
        }
    });
}
pub(super) fn dismiss(ui: &Ui) -> bool {
    if !ui.state.with(|s| s.dialog == Some(Dialog::RemoteEditor)) {
        return false;
    }
    ui.state.revise(|s| {
        if let Some(e) = &mut s.remote_editor {
            if e.pending.is_some() {
                e.cancel.store(true, Ordering::Relaxed);
                s.dialog = None;
            } else if e.dirty {
                e.discard = true;
            } else {
                s.remote_editor = None;
                s.dialog = None;
            }
        }
    });
    ui.schedule();
    true
}
fn save(ui: &Ui) {
    let action = ui.state.with(|s| {
        let e = s.remote_editor.as_ref()?;
        if e.epoch != s.epoch || !s.connected || !e.dirty || e.pending.is_some() {
            return None;
        }
        let document = e.document.clone()?;
        document.encode(&e.draft).ok()?;
        Some(Action::TextSave {
            document,
            content: e.draft.clone(),
            cancel: e.cancel.clone(),
        })
    });
    if let Some(action) = action {
        let serial = ui.send(action);
        ui.state.revise(|s| {
            s.file_pending = Some(serial);
            if let Some(e) = &mut s.remote_editor {
                e.pending = Some(serial);
                e.error.clear();
                e.saved = false;
            }
        });
    }
}
pub(super) fn event(ui: &Ui, serial: u64, result: std::result::Result<Document, String>) {
    let current = ui.state.revise(|s| {
        let editor = s
            .remote_editor
            .as_mut()
            .filter(|e| e.pending == Some(serial))?;
        let current = editor.epoch == s.epoch;
        let saved = editor.document.is_some();
        editor.pending = None;
        s.file_pending = None;
        match result {
            Ok(document) => {
                editor.draft = document.content.clone();
                editor.document = Some(document);
                editor.dirty = false;
                editor.saved = saved;
                editor.error.clear();
            }
            Err(error) => editor.error = error,
        }
        if s.dialog != Some(Dialog::RemoteEditor) {
            s.remote_editor = None;
        }
        Some(current && saved)
    });
    if current == Some(true) && ui.state.with(|s| s.connected) {
        ui.refresh(true);
    }
    ui.schedule();
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::RemoteEditor)),
        move || {
            let draft = ui.clone();
            let save_ui = ui.clone();
            let reload = ui.clone();
            let keep = ui.clone();
            let discard = ui.clone();
            modal(
                ui.clone(),
                "Edit remote file",
                col().class("remote-editor").fill((
                    text()
                        .class("editor-path")
                        .selectable(true)
                        .content(state.map(|s| {
                            s.remote_editor
                                .as_ref()
                                .map(|e| e.path.clone())
                                .unwrap_or_default()
                        })),
                    text().class("subtle").content(state.map(|s| {
                        s.remote_editor
                            .as_ref()
                            .and_then(|e| e.document.as_ref())
                            .map(|d| {
                                format!(
                                    "UTF-8{} · {} · {:04o}",
                                    if d.bom { " BOM" } else { "" },
                                    if d.crlf { "CRLF" } else { "LF" },
                                    d.version.mode
                                )
                            })
                            .unwrap_or_else(|| i18n::translate("Loading remote file…"))
                    })),
                    input()
                        .multiline(true)
                        .class("remote-editor-content")
                        .name(i18n::bind("File contents"))
                        .value(state.map(|s| {
                            s.remote_editor
                                .as_ref()
                                .map(|e| e.draft.clone())
                                .unwrap_or_default()
                        }))
                        .enabled(state.map(|s| {
                            s.remote_editor
                                .as_ref()
                                .is_some_and(|e| e.document.is_some() && e.pending.is_none())
                        }))
                        .on(value_changed, move |event| {
                            if let Event::ValueChanged { value, .. } = event {
                                draft.state.revise(|s| {
                                    if let Some(e) = &mut s.remote_editor {
                                        if value.len() <= arcnix_core::text_files::LIMIT {
                                            e.dirty = e
                                                .document
                                                .as_ref()
                                                .is_some_and(|d| d.content != value);
                                            e.draft = value;
                                            e.saved = false;
                                        } else {
                                            e.error = i18n::translate("Edited file exceeds 1 MiB");
                                        }
                                    }
                                });
                            }
                        }),
                    text().class("editor-feedback").content(state.map(|s| {
                        s.remote_editor
                            .as_ref()
                            .map(|e| {
                                if !e.error.is_empty() {
                                    e.error.clone()
                                } else if e.pending.is_some() {
                                    i18n::translate("Working…")
                                } else if e.saved {
                                    i18n::translate("Remote file saved")
                                } else if e.dirty {
                                    i18n::translate("Unsaved changes")
                                } else {
                                    String::new()
                                }
                            })
                            .unwrap_or_default()
                    })),
                    when(
                        state.map(|s| s.remote_editor.as_ref().is_some_and(|e| e.discard)),
                        move || {
                            let keep = keep.clone();
                            let discard = discard.clone();
                            col().class("editor-discard").fill((
                                text().content(i18n::bind("Discard unsaved changes?")),
                                row().class("choice-list").fill((
                                    sapid::button()
                                        .class("button")
                                        .label(i18n::bind("Keep editing"))
                                        .on_press(move || {
                                            keep.state.revise(|s| {
                                                if let Some(e) = &mut s.remote_editor {
                                                    e.discard = false;
                                                }
                                            })
                                        }),
                                    sapid::button()
                                        .class("button danger")
                                        .label(i18n::bind("Discard changes"))
                                        .on_press(move || {
                                            discard.state.revise(|s| {
                                                s.remote_editor = None;
                                                s.dialog = None;
                                            });
                                            discard.schedule();
                                        }),
                                )),
                            ))
                        },
                    ),
                )),
                (
                    sapid::button()
                        .class("button")
                        .label(i18n::bind("Reload from server"))
                        .enabled(state.map(|s| {
                            s.connected
                                && s.remote_editor
                                    .as_ref()
                                    .is_some_and(|e| !e.dirty && e.pending.is_none())
                        }))
                        .on_press(move || read(&reload)),
                    sapid::button()
                        .class("button primary remote-editor-save")
                        .label(i18n::bind("Save to server"))
                        .enabled(state.map(|s| {
                            s.connected
                                && s.remote_editor.as_ref().is_some_and(|e| {
                                    e.dirty
                                        && e.pending.is_none()
                                        && e.epoch == s.epoch
                                        && e.document
                                            .as_ref()
                                            .is_some_and(|d| d.encode(&e.draft).is_ok())
                                })
                        }))
                        .on_press(move || save(&save_ui)),
                ),
            )
        },
    )
}
