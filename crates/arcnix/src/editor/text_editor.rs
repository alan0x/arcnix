//! An in-memory editor. Remote contents are never put in configuration or logs.
use std::collections::BTreeMap;

use arcnix_core::text_files::Document;
use pulldown_cmark::{Event as MarkdownEvent, Options, Parser, Tag, TagEnd, html};

use crate::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
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
    Window(WorkspacePage),
}

#[derive(Clone, Default)]
pub(crate) struct Tabs {
    pub groups: window_tabs::Groups,
    documents: BTreeMap<u64, Editor>,
    active: Option<u64>,
    next_id: u64,
    confirm_close: Option<CloseTarget>,
    hide_status: bool,
}

impl Tabs {
    pub(crate) fn confirming_close(&self) -> bool {
        self.confirm_close.is_some()
    }

    pub fn focus_window(&mut self, selected: Option<u64>) {
        self.active = selected;
    }

    fn select(&mut self, id: u64) {
        self.groups.select(id);
        if self.documents.contains_key(&id) {
            self.active = Some(id);
            self.confirm_close = None;
        }
    }

    fn insert(&mut self, editor: Editor) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.documents.insert(id, editor);
        self.groups.add(id);
        self.select(id);
        id
    }

    fn remove(&mut self, id: u64) {
        self.documents.remove(&id);
        self.groups.remove(id);
        if matches!(self.confirm_close, Some(CloseTarget::Tab(closing)) if closing == id) {
            self.confirm_close = None;
        }
        if self.active == Some(id) {
            self.active = self.groups.selected(self.groups.current);
        }
    }
}

#[derive(Clone)]
struct Pending {
    serial: u64,
    operation: Operation,
}

#[derive(Clone, PartialEq)]
enum Operation {
    Read,
    Save,
    SaveAs(String),
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
    fn editable(&self) -> bool {
        self.path.is_empty() || self.document.is_some()
    }

    fn encode(&self) -> anyhow::Result<Vec<u8>> {
        match &self.document {
            Some(doc) => doc.encode(&self.draft),
            None => arcnix_core::text_files::encode(&self.draft, false, false),
        }
    }

    fn targets(&self, path: &str) -> bool {
        self.path == path
            || matches!(&self.pending, Some(Pending { operation: Operation::SaveAs(target), .. }) if target == path)
    }

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

pub(crate) fn window_page(window: u32) -> WorkspacePage {
    window_tabs::page(arcnix_core::server_apps::DesktopApp::Editor, window)
}

pub(crate) fn accepts_drop(s: &State, window: u32) -> bool {
    available(s) && server_desktop::visible(s) && s.workbench.window_visible(window_page(window))
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
            .find(|(_, e)| e.targets(path) && e.epoch == epoch)
        {
            if !preview && editor.preview {
                editor.preview = false;
                editor.rendered = None;
            }
            tabs.select(id);
            return (id, false);
        }
        let id = tabs.insert(new_editor(epoch, path, preview));
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
        queued_read: !path.is_empty(),
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
        if editor.path.is_empty()
            || editor.dirty
            || editor.pending.is_some()
            || editor.epoch != epoch
        {
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
                    operation: Operation::Read,
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

pub(crate) fn close_desktop(ui: &Ui, page: WorkspacePage) -> bool {
    ui.state.revise(|s| {
        if desktop_busy(s) {
            return false;
        }
        let tabs = &mut s.desktop_editor;
        let ids = tabs.groups.tabs(page.tab_window().unwrap()).to_vec();
        if ids.iter().any(|id| tabs.documents[id].dirty) {
            tabs.confirm_close = Some(CloseTarget::Window(page));
            return false;
        }
        for id in ids {
            tabs.remove(id);
        }
        tabs.confirm_close = None;
        true
    })
}

pub(crate) fn close_selected(ui: &Ui) {
    if let Some(id) = ui.state.with(|s| s.desktop_editor.active) {
        close_tab(ui, id);
    }
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
            prune_empty(s);
        }
    });
}

fn prune_empty(s: &mut State) {
    let page = window_tabs::current(s, WorkspacePage::Editor);
    if s.desktop_editor
        .groups
        .tabs(page.tab_window().unwrap())
        .is_empty()
        && server_desktop::has_siblings(s, page)
    {
        server_desktop::remove_window(s, page);
    }
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
                prune_empty(s);
                None
            }
            Some(CloseTarget::Window(page)) if !busy => {
                let ids = tabs.groups.tabs(page.tab_window().unwrap()).to_vec();
                for id in ids {
                    tabs.remove(id);
                }
                tabs.confirm_close = None;
                Some(page)
            }
            _ => None,
        }
    });
    if let Some(page) = close_window {
        ui.finish_desktop_close(page);
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

pub(crate) fn save(ui: &Ui, surface: Surface, sudo: Option<zeroize::Zeroizing<String>>) {
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
            sudo,
        })
    });
    if let Some(action) = action {
        start_save(ui, surface, action, Operation::Save);
    }
}

fn start_save(ui: &Ui, surface: Surface, action: Action, operation: Operation) {
    let serial = ui.send(action);
    ui.state.revise(|s| {
        s.file_pending = Some(serial);
        if let Some(e) = surface.editor_mut(s) {
            e.pending = Some(Pending { serial, operation });
            e.error.clear();
            e.saved = false;
        }
    });
}

pub(crate) fn save_as(
    ui: &Ui,
    id: u64,
    path: String,
    replace: Option<arcnix_core::text_files::TextSnapshot>,
) {
    let surface = Surface::Desktop(id);
    if ui
        .state
        .with(|s| surface.editor(s).is_some_and(|e| e.path == path))
    {
        save(ui, surface, None);
        return;
    }
    let action = ui.state.with(|s| {
        if !can_save_as(s, surface) {
            return None;
        }
        let e = surface.editor(s)?;
        Some(Action::TextSaveAs {
            path: path.clone(),
            bytes: e.encode().ok()?.into(),
            replace,
            cancel: e.cancel.clone(),
        })
    });
    if let Some(action) = action {
        start_save(ui, surface, action, Operation::SaveAs(path));
    }
}

pub(crate) fn open_in_another_tab(s: &State, id: u64, path: &str) -> bool {
    s.desktop_editor
        .documents
        .iter()
        .any(|(&other, e)| other != id && e.targets(path))
}
pub(crate) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Document, worker::FileFailure>,
) {
    let current = ui.state.revise(|s| {
        let matches = |e: &Editor| e.pending.as_ref().is_some_and(|p| p.serial == serial);
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
        let operation = editor.pending.take()?.operation;
        let saved = operation != Operation::Read;
        let success = result.is_ok();
        let retry = (current
            && operation == Operation::Save
            && !editor.cancel.load(Ordering::Relaxed)
            && result.as_ref().is_err_and(|error| error.permission_denied))
        .then(|| file_elevation::Retry::Save(surface, editor.path.clone()));
        let opened = result
            .as_ref()
            .ok()
            .filter(|_| current && operation != Operation::Save)
            .map(|doc| doc.path.clone());
        match result {
            Ok(document) => {
                editor.path = document.path.clone();
                editor.markdown = markdown(&editor.path);
                editor.preview &= editor.markdown;
                editor.draft = document.content.clone();
                editor.document = Some(document);
                editor.dirty = false;
                editor.saved = saved;
                editor.error.clear();
                editor.render_preview();
            }
            Err(error) => editor.error = error.message,
        }
        if surface == Surface::Transfer && s.dialog != Some(Dialog::TextEditor) {
            s.remote_editor = None;
        } else if let Some(retry) = retry
            && s.connected
            && (s.dialog.is_none() || s.dialog == Some(Dialog::TextEditor))
        {
            file_elevation::show(s, retry);
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
    read_queued(ui);
    ui.schedule();
}

pub(crate) fn read_queued(ui: &Ui) {
    let queued = ui.state.with(|s| {
        if s.dialog == Some(Dialog::FileElevation) {
            return None;
        }
        s.desktop_editor
            .documents
            .iter()
            .find_map(|(&id, e)| (e.queued_read && e.epoch == s.epoch).then_some(id))
    });
    if let Some(id) = queued {
        read(ui, Surface::Desktop(id));
    }
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
                            .label(i18n::bind_in(surface != Surface::Transfer, label))
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

fn editor_status(state: Cage<State>, surface: Surface) -> impl Widget {
    let feedback = text()
        .class("editor-feedback")
        .content(state.map(move |s| surface.editor(s).map(feedback).unwrap_or_default()));
    if surface == Surface::Transfer {
        return fragment(feedback);
    }
    fragment(
        row().class("editor-statusbar").fill((
            feedback,
            when(
                state.map(move |s| {
                    surface
                        .editor(s)
                        .is_some_and(|e| !e.preview && e.editable())
                }),
                || {
                    row().class("editor-position").fill((
                        text().content(i18n::desktop::bind("Line")),
                        text().class("editor-line-value").content("1"),
                        text().content(i18n::desktop::bind("Column")),
                        text().class("editor-column-value").content("1"),
                    ))
                },
            ),
            text().class("editor-mode-name"),
            text()
                .class("editor-plain-label")
                .content(i18n::desktop::bind("Plain text")),
            text().class("editor-format").content(state.map(move |s| {
                surface
                    .editor(s)
                    .filter(|e| e.editable())
                    .map(|e| {
                        let encoding = if e.document.as_ref().is_some_and(|d| d.bom) {
                            "UTF-8 BOM"
                        } else {
                            "UTF-8"
                        };
                        let ending = if e.document.as_ref().is_some_and(|d| d.crlf) {
                            "CRLF"
                        } else {
                            "LF"
                        };
                        format!("{encoding}  ·  {ending}")
                    })
                    .unwrap_or_default()
            })),
        )),
    )
}

fn body(ui: Ui, surface: Surface) -> impl Widget {
    let state = ui.state;
    let discard_ui = ui.clone();
    col().class("remote-editor").fill((
        when(surface == Surface::Transfer, move || {
            row().class("editor-toolbar").fill((
                icons::glyph(Icon::File),
                text()
                    .class("editor-path")
                    .selectable(true)
                    .content(state.map(move |s| {
                        surface
                            .editor(s)
                            .map(|e| e.path.clone())
                            .unwrap_or_default()
                    })),
                mode_controls(ui.clone(), surface),
            ))
        }),
        when(
            state.map(move |s| surface.editor(s).is_some_and(|e| !e.preview)),
            move || {
                let contents = input()
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
                            .is_some_and(|e| e.editable() && e.pending.is_none())
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
                                            .map_or("", |doc| doc.content.as_str())
                                            != value;
                                        editor.draft = value;
                                        editor.saved = false;
                                    } else {
                                        editor.error = i18n::translate("Edited file exceeds 1 MiB");
                                    }
                                }
                            })
                        }
                    });
                match surface {
                    Surface::Desktop(_) => fragment(col().class("editor-sheet").fill(contents)),
                    Surface::Transfer => fragment(contents),
                }
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
        when(
            state.map(move |s| surface == Surface::Transfer || !s.desktop_editor.hide_status),
            move || editor_status(state, surface),
        ),
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

fn can_reload(s: &State, surface: Surface) -> bool {
    s.connected
        && s.file_pending.is_none()
        && surface.editor(s).is_some_and(|e| {
            !e.path.is_empty() && !e.dirty && e.pending.is_none() && e.epoch == s.epoch
        })
}

fn buttons(ui: Ui, surface: Surface) -> impl Widget {
    let state = ui.state;
    let reload = ui.clone();
    fragment((
        sapid::button()
            .class("button")
            .label(i18n::bind("Reload from server"))
            .enabled(state.map(move |s| can_reload(s, surface)))
            .on_press(move || read(&reload, surface)),
        sapid::button()
            .class("button primary remote-editor-save")
            .label(i18n::bind("Save to server"))
            .enabled(state.map(move |s| can_save(s, surface)))
            .on_press(move || save(&ui, surface, None)),
    ))
}

fn can_browse(s: &State) -> bool {
    available(s) && !file_task_busy(s)
}

fn browse(ui: &Ui) {
    let start = ui.state.with(|s| {
        let id = s.desktop_editor.active?;
        desktop_file_picker::parent(&s.desktop_editor.documents.get(&id)?.path)
    });
    desktop_file_picker::open(ui, desktop_file_picker::Purpose::TextEditor, start);
}

fn can_save_as(s: &State, surface: Surface) -> bool {
    s.connected
        && s.sftp_available
        && s.file_pending.is_none()
        && surface.editor(s).is_some_and(|e| {
            e.epoch == s.epoch && e.editable() && e.pending.is_none() && e.encode().is_ok()
        })
}

fn choose_destination(ui: &Ui, id: u64) {
    let Some((start, name)) = ui.state.with(|s| {
        let e = Surface::Desktop(id).editor(s)?;
        Some((
            desktop_file_picker::parent(&e.path),
            filename(&e.path).to_owned(),
        ))
    }) else {
        return;
    };
    desktop_file_picker::open(ui, desktop_file_picker::Purpose::SaveText(id), start);
    desktop_file_picker::set_save_name(ui, name);
}

fn new_document(ui: &Ui) {
    ui.state.revise(|s| {
        if available(s) {
            s.desktop_editor.insert(new_editor(s.epoch, "", false));
        }
    });
}

fn active(s: &State) -> Option<Surface> {
    s.desktop_editor.active.map(Surface::Desktop)
}

fn menu(label: &'static str, items: impl IntoChildren + 'static) -> impl Widget {
    col().class("editor-menu").fill((
        sapid::button()
            .class("editor-menu-trigger")
            .label(i18n::desktop::bind(label)),
        col().class("editor-menu-popup").fill(items),
    ))
}

pub(crate) fn menubar(ui: Ui) -> impl Widget {
    use i18n::desktop::{bind, btn};
    let state = ui.state;
    let new = ui.clone();
    let open = ui.clone();
    let save_ui = ui.clone();
    let save_as_ui = ui.clone();
    let reload = ui.clone();
    let mode_ui = ui.clone();
    row().class("editor-menubar").fill((
        menu(
            "File",
            (
                btn("New document", move || new_document(&new))
                    .class("editor-new")
                    .enabled(state.map(available)),
                btn("Open file…", move || browse(&open))
                    .class("editor-open")
                    .enabled(state.map(can_browse)),
                btn("Save", move || {
                    if let Some(id) = state.with(|s| s.desktop_editor.active) {
                        let surface = Surface::Desktop(id);
                        if state.with(|s| surface.editor(s).is_some_and(|e| e.path.is_empty())) {
                            choose_destination(&save_ui, id);
                        } else {
                            save(&save_ui, surface, None);
                        }
                    }
                })
                .class("remote-editor-save")
                .enabled(state.map(|s| {
                    active(s).is_some_and(|surface| {
                        if surface.editor(s).is_some_and(|e| e.path.is_empty()) {
                            can_save_as(s, surface)
                        } else {
                            can_save(s, surface)
                        }
                    })
                })),
                btn("Save as…", move || {
                    if let Some(id) = state.with(|s| s.desktop_editor.active) {
                        choose_destination(&save_as_ui, id);
                    }
                })
                .class("editor-save-as")
                .enabled(state.map(|s| active(s).is_some_and(|surface| can_save_as(s, surface)))),
                btn("Reload from server", move || {
                    if let Some(surface) = state.with(active) {
                        read(&reload, surface);
                    }
                })
                .class("editor-reload")
                .enabled(state.map(|s| active(s).is_some_and(|surface| can_reload(s, surface)))),
                btn("Close tab", move || close_selected(&ui)).enabled(state.map(|s| {
                    active(s)
                        .and_then(|surface| surface.editor(s))
                        .is_some_and(|e| e.pending.is_none())
                })),
            ),
        ),
        menu(
            "Edit",
            (
                fragment(
                    [
                        ("Undo", "undo"),
                        ("Redo", "redo"),
                        ("Cut", "cut"),
                        ("Copy", "copy"),
                        ("Paste", "paste"),
                        ("Select all", "selectAll"),
                    ]
                    .into_iter()
                    .map(|(label, command)| {
                        sapid::button()
                            .class(format!("editor-edit-{command}"))
                            .label(bind(label))
                            .enabled(state.map(|s| {
                                active(s)
                                    .and_then(|surface| surface.editor(s))
                                    .is_some_and(|e| {
                                        !e.preview && e.editable() && e.pending.is_none()
                                    })
                            }))
                    })
                    .collect::<Vec<_>>(),
                ),
                text().class("hidden editor-paste-help").content(bind(
                    "Use the paste keyboard shortcut to paste from the clipboard.",
                )),
            ),
        ),
        menu(
            "View",
            (
                each(state.map(|s| s.desktop_editor.active.into_iter().collect::<Vec<_>>()))
                    .key(|id| *id)
                    .children(move |item| {
                        mode_controls(mode_ui.clone(), Surface::Desktop(item.value.get()))
                    }),
                btn("Status bar", move || {
                    state.revise(|s| s.desktop_editor.hide_status = !s.desktop_editor.hide_status)
                })
                .pressed(state.map(|s| !s.desktop_editor.hide_status)),
            ),
        ),
    ))
}

fn filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub(crate) fn file_tabs(ui: Ui, window: u32) -> impl Widget {
    let state = ui.state;
    row()
        .class("editor-tab-strip window-tab-strip")
        .name(i18n::bind("Open files"))
        .fill(
            each(state.map(move |s| s.desktop_editor.groups.tabs(window).to_vec()))
                .key(|id| *id)
                .children(move |item| {
                    let id = item.value.get();
                    let surface = Surface::Desktop(id);
                    let close = ui.clone();
                    let path = state.map(move |s| {
                        surface
                            .editor(s)
                            .map(|e| {
                                if e.path.is_empty() {
                                    format!("{} {id}", i18n::desktop::translate("Untitled"))
                                } else {
                                    e.path.clone()
                                }
                            })
                            .unwrap_or_default()
                    });
                    row()
                        .class(state.map(move |s| {
                            if s.desktop_editor.groups.selected(window) == Some(id) {
                                "window-tab-item editor-tab-item active"
                            } else {
                                "window-tab-item editor-tab-item"
                            }
                            .to_owned()
                        }))
                        .fill((
                            text().class("window-tab-id hidden").content(id.to_string()),
                            sapid::button()
                                .class(state.map(move |s| {
                                    if surface.editor(s).is_some_and(|e| e.dirty) {
                                        "editor-tab window-tab-select dirty"
                                    } else {
                                        "editor-tab window-tab-select"
                                    }
                                    .to_owned()
                                }))
                                .name(path)
                                .pressed(state.map(move |s| {
                                    s.desktop_editor.groups.selected(window) == Some(id)
                                }))
                                .fill(
                                    text().class("editor-tab-label").content(
                                        state.map(move |_| filename(&path.get()).to_owned()),
                                    ),
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

fn close_confirmation(ui: Ui, window: u32) -> impl Widget {
    let state = ui.state;
    when(
        state.map(move |s| match s.desktop_editor.confirm_close {
            Some(CloseTarget::Window(page)) => page == window_page(window),
            Some(CloseTarget::Tab(id)) => s.desktop_editor.groups.tabs(window).contains(&id),
            None => false,
        }),
        move || {
            let ui = ui.clone();
            col().class("editor-discard").fill((
                text().content(i18n::bind(state.map(|s| {
                    if matches!(s.desktop_editor.confirm_close, Some(CloseTarget::Window(_))) {
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

pub(crate) fn page(ui: Ui, window: u32) -> impl Widget {
    let state = ui.state;
    let open = ui.clone();
    col().class("desktop-editor editor-file-drop").fill((
        text()
            .class("editor-window-id hidden")
            .content(window.to_string()),
        row().class("editor-tabbar").fill((
            file_tabs(ui.clone(), window),
            server_desktop::tab_overflow(),
        )),
        when(
            state.map(move |s| s.desktop_editor.groups.selected(window).is_none()),
            move || {
                let open = open.clone();
                col().class("editor-empty").fill((
                    i18n::desktop::text_action("Open file…", Icon::Folder)
                        .enabled(state.map(can_browse))
                        .on_press(move || browse(&open)),
                    text()
                        .class("subtle")
                        .content(i18n::desktop::bind("Drop files here")),
                ))
            },
        ),
        close_confirmation(ui.clone(), window),
        each(state.map(move |s| {
            s.desktop_editor
                .groups
                .selected(window)
                .into_iter()
                .collect::<Vec<_>>()
        }))
        .key(|id| *id)
        .children(move |item| body(ui.clone(), Surface::Desktop(item.value.get()))),
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
