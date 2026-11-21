//! An in-memory editor. Remote contents are never put in configuration or logs.
use std::collections::BTreeMap;

use arcnix_core::text_files::Document;
use pulldown_cmark::{Event as MarkdownEvent, Options, Parser, Tag, TagEnd, html};

use crate::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Desktop(u64),
    Transfer,
}

impl Surface {
    fn editor(self, s: &State) -> Option<&Editor> {
        match self {
            Self::Desktop(id) => s.desktop_editor.documents.get(&id),
            Self::Transfer => s.remote_editor.as_ref(),
        }
    }

    fn editor_mut(self, s: &mut State) -> Option<&mut Editor> {
        match self {
            Self::Desktop(id) => s.desktop_editor.documents.get_mut(&id),
            Self::Transfer => s.remote_editor.as_mut(),
        }
    }
}

#[derive(Clone, Copy)]
enum CloseTarget {
    Tab(u64),
    Window,
}

#[derive(Clone, Default)]
pub(crate) struct Tabs {
    documents: BTreeMap<u64, Editor>,
    active: Option<u64>,
    next_id: u64,
    confirm_close: Option<CloseTarget>,
}

impl Tabs {
    fn select(&mut self, id: u64) {
        if self.documents.contains_key(&id) {
            self.active = Some(id);
            self.confirm_close = None;
        }
    }

    fn remove(&mut self, id: u64) {
        self.documents.remove(&id);
        if matches!(self.confirm_close, Some(CloseTarget::Tab(closing)) if closing == id) {
            self.confirm_close = None;
        }
        if self.active == Some(id) {
            self.active = self
                .documents
                .range(id..)
                .next()
                .or_else(|| self.documents.last_key_value())
                .map(|(&id, _)| id);
        }
    }
}

#[derive(Clone, Copy)]
struct Pending {
    serial: u64,
    saving: bool,
}

#[derive(Clone)]
pub(crate) struct Editor {
    epoch: u64,
    path: String,
    document: Option<Document>,
    draft: String,
    pending: Option<Pending>,
    queued_read: bool,
    cancel: Arc<AtomicBool>,
    dirty: bool,
    discard: bool,
    error: String,
    saved: bool,
    markdown: bool,
    preview: bool,
    rendered: Option<Arc<str>>,
}

fn markdown(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, extension)| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown" | "mkd" | "mkdn"
            )
        })
}

fn render_markdown(source: &str) -> Option<Arc<str>> {
    let mut output = String::new();
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let events = Parser::new_ext(source, options).filter_map(|event| match event {
        MarkdownEvent::Html(text) | MarkdownEvent::InlineHtml(text) => {
            Some(MarkdownEvent::Text(text))
        }
        MarkdownEvent::Start(Tag::Link { .. } | Tag::Image { .. })
        | MarkdownEvent::End(TagEnd::Link | TagEnd::Image) => None,
        _ => Some(event),
    });
    html::push_html(&mut output, events);
    (output.len() <= 2 * 1024 * 1024).then(|| Arc::from(output))
}

impl Editor {
    fn render_preview(&mut self) {
        self.rendered = if self.preview && self.document.is_some() {
            render_markdown(&self.draft)
        } else {
            None
        };
    }
}

pub(crate) fn busy(s: &State) -> bool {
    desktop_busy(s)
        || Surface::Transfer
            .editor(s)
            .is_some_and(|e| e.pending.is_some())
}

pub(crate) fn desktop_busy(s: &State) -> bool {
    s.desktop_editor
        .documents
        .values()
        .any(|e| e.pending.is_some())
}
pub(crate) fn available(s: &State) -> bool {
    s.connected
        && s.sftp_available
        && (s.file_pending.is_none() || (server_desktop::visible(s) && desktop_busy(s)))
        && !s.system_task_running
        && s.cancels.is_empty()
        && s.dialog.is_none()
}
pub(crate) fn open(ui: &Ui, path: &str) {
    open_path(ui, path, true);
}

pub(crate) fn accepts_drop(s: &State) -> bool {
    available(s) && server_desktop::visible(s) && s.workbench.window_visible(WorkspacePage::Editor)
}

fn open_path(ui: &Ui, path: &str, preview: bool) {
    if !ui.state.with(available) {
        return;
    }
    if !ui.state.with(server_desktop::visible) {
        ui.state
            .revise(|s| s.remote_editor = Some(new_editor(s.epoch, path, preview)));
        ui.open_dialog(Dialog::TextEditor);
        read(ui, Surface::Transfer);
        return;
    }

    let (id, created) = ui.state.revise(|s| {
        let epoch = s.epoch;
        let tabs = &mut s.desktop_editor;
        if let Some((&id, editor)) = tabs
            .documents
            .iter_mut()
            .find(|(_, e)| e.path == path && e.epoch == epoch)
        {
            if !preview && editor.preview {
                editor.preview = false;
                editor.rendered = None;
            }
            tabs.select(id);
            return (id, false);
        }
        tabs.next_id += 1;
        let id = tabs.next_id;
        let editor = new_editor(epoch, path, preview);
        tabs.documents.insert(id, editor);
        tabs.select(id);
        (id, true)
    });
    ui.open_desktop_app(WorkspacePage::Editor);
    if created {
        read(ui, Surface::Desktop(id));
    }
}

fn new_editor(epoch: u64, path: &str, preview: bool) -> Editor {
    let markdown = markdown(path);
    Editor {
        epoch,
        path: path.into(),
        document: None,
        draft: String::new(),
        pending: None,
        queued_read: true,
        cancel: Arc::new(AtomicBool::new(false)),
        dirty: false,
        discard: false,
        error: String::new(),
        saved: false,
        markdown,
        preview: markdown && preview,
        rendered: None,
    }
}
fn read(ui: &Ui, surface: Surface) {
    let task = ui.state.revise(|s| {
        if !s.connected || s.file_pending.is_some() {
            return None;
        }
        let epoch = s.epoch;
        let editor = surface.editor_mut(s)?;
        if editor.dirty || editor.pending.is_some() || editor.epoch != epoch {
            return None;
        }
        editor.queued_read = false;
        editor.cancel = Arc::new(AtomicBool::new(false));
        Some((editor.path.clone(), editor.cancel.clone()))
    });
    if let Some((path, cancel)) = task {
        let serial = ui.send(Action::TextRead { path, cancel });
        ui.state.revise(|s| {
            s.file_pending = Some(serial);
            if let Some(e) = surface.editor_mut(s) {
                e.pending = Some(Pending {
                    serial,
                    saving: false,
                });
                e.error.clear();
                e.saved = false;
            }
        });
    }
}
pub(crate) fn cancel(ui: &Ui) {
    ui.state.with(|s| {
        for e in s
            .desktop_editor
            .documents
            .values()
            .chain(s.remote_editor.iter())
        {
            e.cancel.store(true, Ordering::Relaxed);
        }
    });
}
pub(crate) fn dismiss(ui: &Ui) -> bool {
    if !ui.state.with(|s| s.dialog == Some(Dialog::TextEditor)) {
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

pub(crate) fn close_desktop(ui: &Ui) -> bool {
    ui.state.revise(|s| {
        if desktop_busy(s) {
            return false;
        }
        let tabs = &mut s.desktop_editor;
        if tabs.documents.values().any(|e| e.dirty) {
            tabs.confirm_close = Some(CloseTarget::Window);
            return false;
        }
        tabs.documents.clear();
        tabs.active = None;
        tabs.confirm_close = None;
        true
    })
}

fn close_tab(ui: &Ui, id: u64) {
    ui.state.revise(|s| {
        let tabs = &mut s.desktop_editor;
        let Some(editor) = tabs.documents.get(&id) else {
            return;
        };
        if editor.pending.is_some() {
            return;
        }
        if editor.dirty {
            tabs.select(id);
            tabs.confirm_close = Some(CloseTarget::Tab(id));
        } else {
            tabs.remove(id);
        }
    });
}

pub(crate) fn disconnect(s: &mut State) {
    if busy(s) {
        s.file_pending = None;
    }
    s.desktop_editor = Tabs::default();
    s.remote_editor = None;
    if s.dialog == Some(Dialog::TextEditor) {
        s.dialog = None;
    }
}

fn discard_desktop(ui: &Ui) {
    let close_window = ui.state.revise(|s| {
        let busy = desktop_busy(s);
        let tabs = &mut s.desktop_editor;
        match tabs.confirm_close {
            Some(CloseTarget::Tab(id))
                if tabs.documents.get(&id).is_some_and(|e| e.pending.is_none()) =>
            {
                tabs.remove(id);
                tabs.confirm_close = None;
                false
            }
            Some(CloseTarget::Window) if !busy => {
                tabs.documents.clear();
                tabs.active = None;
                tabs.confirm_close = None;
                true
            }
            _ => false,
        }
    });
    if close_window {
        ui.finish_desktop_close(WorkspacePage::Editor);
    }
}

fn can_save(s: &State, surface: Surface) -> bool {
    s.connected
        && s.file_pending.is_none()
        && surface.editor(s).is_some_and(|e| {
            e.dirty
                && e.pending.is_none()
                && e.epoch == s.epoch
                && e.document
                    .as_ref()
                    .is_some_and(|d| d.encode(&e.draft).is_ok())
        })
}

fn save(ui: &Ui, surface: Surface) {
    let action = ui.state.with(|s| {
        if !can_save(s, surface) {
            return None;
        }
        let e = surface.editor(s)?;
        let document = e.document.clone()?;
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
            if let Some(e) = surface.editor_mut(s) {
                e.pending = Some(Pending {
                    serial,
                    saving: true,
                });
                e.error.clear();
                e.saved = false;
            }
        });
    }
}
pub(crate) fn event(ui: &Ui, serial: u64, result: std::result::Result<Document, String>) {
    let current = ui.state.revise(|s| {
        let matches = |e: &Editor| e.pending.is_some_and(|p| p.serial == serial);
        let surface = if s.remote_editor.as_ref().is_some_and(matches) {
            Surface::Transfer
        } else {
            Surface::Desktop(
                *s.desktop_editor
                    .documents
                    .iter()
                    .find(|(_, e)| matches(e))?
                    .0,
            )
        };
        let epoch = s.epoch;
        s.file_pending = None;
        let editor = surface.editor_mut(s)?;
        let current = editor.epoch == epoch;
        let saved = editor.pending.take()?.saving;
        let success = result.is_ok();
        let opened = (current && success && !saved).then(|| editor.path.clone());
        match result {
            Ok(document) => {
                editor.draft = document.content.clone();
                editor.document = Some(document);
                editor.dirty = false;
                editor.saved = saved;
                editor.error.clear();
                editor.render_preview();
            }
            Err(error) => editor.error = error,
        }
        if surface == Surface::Transfer && s.dialog != Some(Dialog::TextEditor) {
            s.remote_editor = None;
        }
        Some((current && saved && success, opened))
    });
    if let Some((saved, opened)) = current {
        if saved && ui.state.with(|s| s.connected) {
            ui.refresh(true);
        }
        if let Some(path) = opened {
            file_open::remember(
                ui,
                &path,
                arcnix_core::file_associations::FileApp::TextEditor,
            );
        }
    }
    let queued = ui.state.with(|s| {
        s.desktop_editor
            .documents
            .iter()
            .find_map(|(&id, e)| (e.queued_read && e.epoch == s.epoch).then_some(id))
    });
    if let Some(id) = queued {
        read(ui, Surface::Desktop(id));
    }
    ui.schedule();
}
fn feedback(editor: &Editor) -> String {
    if !editor.error.is_empty() {
        editor.error.clone()
    } else if editor.queued_read || editor.pending.is_some() {
        i18n::translate("Working…")
    } else if editor.saved {
        i18n::translate("Remote file saved")
    } else if editor.dirty {
        i18n::translate("Unsaved changes")
    } else {
        String::new()
    }
}

fn mode_controls(ui: Ui, surface: Surface) -> impl Widget {
    let state = ui.state;
    when(
        state.map(move |s| surface.editor(s).is_some_and(|editor| editor.markdown)),
        move || {
            row().class("editor-modes").fill(fragment(
                [(false, "Edit source"), (true, "Preview")]
                    .into_iter()
                    .map(|(preview, label)| {
                        sapid::button()
                            .class("button compact")
                            .label(i18n::bind(label))
                            .pressed(state.map(move |s| {
                                surface.editor(s).is_some_and(|e| e.preview == preview)
                            }))
                            .enabled(state.map(move |s| {
                                surface
                                    .editor(s)
                                    .is_some_and(|e| e.document.is_some() && e.pending.is_none())
                            }))
                            .on_press(move || {
                                state.revise(|s| {
                                    if let Some(editor) = surface.editor_mut(s) {
                                        editor.preview = preview;
                                        editor.render_preview();
                                    }
                                })
                            })
                    })
                    .collect::<Vec<_>>(),
            ))
        },
    )
}

fn body(ui: Ui, surface: Surface) -> impl Widget {
    let state = ui.state;
    let discard_ui = ui.clone();
    col().class("remote-editor").fill((
        row().class("editor-toolbar").fill((
            text()
                .class("editor-path")
                .selectable(true)
                .content(state.map(move |s| {
                    surface
                        .editor(s)
                        .map(|e| e.path.clone())
                        .unwrap_or_default()
                })),
            mode_controls(ui, surface),
        )),
        when(
            state.map(move |s| surface.editor(s).is_some_and(|e| !e.preview)),
            move || {
                input()
                    .multiline(true)
                    .class("remote-editor-content")
                    .name(i18n::bind("File contents"))
                    .value(state.map(move |s| {
                        surface
                            .editor(s)
                            .map(|e| e.draft.clone())
                            .unwrap_or_default()
                    }))
                    .enabled(state.map(move |s| {
                        surface
                            .editor(s)
                            .is_some_and(|e| e.document.is_some() && e.pending.is_none())
                    }))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event {
                            state.revise(|s| {
                                if let Some(editor) = surface
                                    .editor_mut(s)
                                    .filter(|e| e.pending.is_none() && !e.preview)
                                {
                                    if value.len() <= arcnix_core::text_files::LIMIT {
                                        editor.dirty = editor
                                            .document
                                            .as_ref()
                                            .is_some_and(|document| document.content != value);
                                        editor.draft = value;
                                        editor.saved = false;
                                    } else {
                                        editor.error = i18n::translate("Edited file exceeds 1 MiB");
                                    }
                                }
                            })
                        }
                    })
            },
        ),
        when(
            state.map(move |s| surface.editor(s).is_some_and(|e| e.preview)),
            move || {
                col().class("editor-preview-host").fill((
                    text()
                        .class("markdown-preview-source hidden")
                        .content(state.map(move |s| {
                            surface
                                .editor(s)
                                .and_then(|e| e.rendered.as_deref())
                                .unwrap_or_default()
                                .to_owned()
                        })),
                    col()
                        .class("markdown-preview")
                        .name(i18n::bind("Markdown preview")),
                    when(
                        state.map(move |s| {
                            surface
                                .editor(s)
                                .is_some_and(|e| e.document.is_some() && e.rendered.is_none())
                        }),
                        || {
                            text().class("subtle").content(i18n::bind(
                                "Markdown preview is too large. Open the source to read the file.",
                            ))
                        },
                    ),
                ))
            },
        ),
        text()
            .class("editor-feedback")
            .content(state.map(move |s| surface.editor(s).map(feedback).unwrap_or_default())),
        when(
            state.map(move |s| {
                surface == Surface::Transfer && surface.editor(s).is_some_and(|e| e.discard)
            }),
            move || {
                let discard_ui = discard_ui.clone();
                col().class("editor-discard").fill((
                    text().content(i18n::bind("Discard unsaved changes?")),
                    row().class("choice-list").fill((
                        sapid::button()
                            .class("button")
                            .label(i18n::bind("Keep editing"))
                            .on_press(move || {
                                state.revise(|s| {
                                    if let Some(editor) = surface.editor_mut(s) {
                                        editor.discard = false;
                                    }
                                })
                            }),
                        sapid::button()
                            .class("button danger")
                            .label(i18n::bind("Discard changes"))
                            .on_press(move || {
                                discard_ui.state.revise(|s| {
                                    if s.dialog == Some(Dialog::TextEditor)
                                        && s.remote_editor
                                            .as_ref()
                                            .is_some_and(|e| e.discard && e.pending.is_none())
                                    {
                                        s.remote_editor = None;
                                        s.dialog = None;
                                    }
                                });
                                discard_ui.schedule();
                            }),
                    )),
                ))
            },
        ),
    ))
}

fn buttons(ui: Ui, surface: Surface) -> impl Widget {
    let state = ui.state;
    let reload = ui.clone();
    fragment((
        sapid::button()
            .class("button")
            .label(i18n::bind("Reload from server"))
            .enabled(state.map(move |s| {
                s.connected
                    && s.file_pending.is_none()
                    && surface
                        .editor(s)
                        .is_some_and(|e| !e.dirty && e.pending.is_none() && e.epoch == s.epoch)
            }))
            .on_press(move || read(&reload, surface)),
        sapid::button()
            .class("button primary remote-editor-save")
            .label(i18n::bind("Save to server"))
            .enabled(state.map(move |s| can_save(s, surface)))
            .on_press(move || save(&ui, surface)),
    ))
}

fn browse(ui: Ui) -> impl Widget {
    i18n::desktop::text_action("Open file…", Icon::Folder)
        .enabled(ui.state.map(available))
        .on_press(move || {
            let start = ui.state.with(|s| {
                let id = s.desktop_editor.active?;
                desktop_file_picker::parent(&s.desktop_editor.documents.get(&id)?.path)
            });
            desktop_file_picker::open(&ui, desktop_file_picker::Purpose::TextEditor, start);
        })
}

fn filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn file_tabs(ui: Ui) -> impl Widget {
    let state = ui.state;
    row()
        .class("editor-tab-strip")
        .name(i18n::bind("Open files"))
        .fill(
            each(state.map(|s| {
                s.desktop_editor
                    .documents
                    .keys()
                    .copied()
                    .collect::<Vec<_>>()
            }))
            .key(|id| *id)
            .children(move |item| {
                let id = item.value.get();
                let surface = Surface::Desktop(id);
                let close = ui.clone();
                let path = state.map(move |s| {
                    surface
                        .editor(s)
                        .map(|e| e.path.clone())
                        .unwrap_or_default()
                });
                row()
                    .class(state.map(move |s| {
                        if s.desktop_editor.active == Some(id) {
                            "editor-tab-item active"
                        } else {
                            "editor-tab-item"
                        }
                        .to_owned()
                    }))
                    .fill((
                        sapid::button()
                            .class(state.map(move |s| {
                                if surface.editor(s).is_some_and(|e| e.dirty) {
                                    "editor-tab dirty"
                                } else {
                                    "editor-tab"
                                }
                                .to_owned()
                            }))
                            .name(path)
                            .pressed(state.map(move |s| s.desktop_editor.active == Some(id)))
                            .fill(
                                text()
                                    .class("editor-tab-label")
                                    .content(state.map(move |_| filename(&path.get()).to_owned())),
                            )
                            .on_press(move || state.revise(|s| s.desktop_editor.select(id))),
                        sapid::button()
                            .class("icon-button editor-tab-close")
                            .name(i18n::bind(state.map(move |_| {
                                format!("Close file tab {}", filename(&path.get()))
                            })))
                            .enabled(state.map(move |s| {
                                surface.editor(s).is_some_and(|e| e.pending.is_none())
                            }))
                            .fill(icons::glyph(Icon::Close))
                            .on_press(move || close_tab(&close, id)),
                    ))
            }),
        )
}

fn close_confirmation(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.desktop_editor.confirm_close.is_some()),
        move || {
            let ui = ui.clone();
            col().class("editor-discard").fill((
                text().content(i18n::bind(state.map(|s| {
                    if matches!(s.desktop_editor.confirm_close, Some(CloseTarget::Window)) {
                        "Discard unsaved changes in all open files?"
                    } else {
                        "Discard unsaved changes?"
                    }
                    .to_owned()
                }))),
                row().class("choice-list").fill((
                    btn("Keep editing", move || {
                        state.revise(|s| s.desktop_editor.confirm_close = None)
                    }),
                    sapid::button()
                        .class("button danger")
                        .label(i18n::bind("Discard changes"))
                        .on_press(move || discard_desktop(&ui)),
                )),
            ))
        },
    )
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let open = ui.clone();
    let actions = ui.clone();
    col().class("desktop-editor editor-file-drop").fill((
        when(
            state.map(|s| s.desktop_editor.active.is_none()),
            move || {
                col().class("editor-empty").fill((
                    browse(open.clone()),
                    text()
                        .class("subtle")
                        .content(i18n::desktop::bind("Drop files here")),
                ))
            },
        ),
        when(
            state.map(|s| s.desktop_editor.active.is_some()),
            move || file_tabs(actions.clone()),
        ),
        close_confirmation(ui.clone()),
        each(state.map(|s| s.desktop_editor.active.into_iter().collect::<Vec<_>>()))
            .key(|id| *id)
            .children(move |item| {
                let surface = Surface::Desktop(item.value.get());
                fragment((
                    body(ui.clone(), surface),
                    row().class("editor-actions").fill((
                        browse(ui.clone()),
                        row()
                            .class("choice-list")
                            .fill(buttons(ui.clone(), surface)),
                    )),
                ))
            }),
    ))
}

pub(crate) fn dialogs(ui: Ui) -> impl Widget {
    when(
        ui.state.map(|s| s.dialog == Some(Dialog::TextEditor)),
        move || {
            modal(
                ui.clone(),
                "Text Editor",
                body(ui.clone(), Surface::Transfer),
                buttons(ui.clone(), Surface::Transfer),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_preview_formats_text_without_active_elements_or_resource_urls() {
        let preview = render_markdown(
            "# Title\n\n**Bold** [link](javascript:alert%281%29) ![image](https://example.invalid/a.png)\n\n<iframe src=\"x\"></iframe>",
        ).unwrap();
        assert!(preview.contains("<h1>Title</h1>"));
        assert!(preview.contains("<strong>Bold</strong> link image"));
        assert!(preview.contains("&lt;iframe"));
        assert!(
            !preview.contains("<iframe")
                && !preview.contains("javascript:")
                && !preview.contains("example.invalid")
        );
    }
}
