//! Files exposed to the embedded workbench from the active desktop.
use std::result::Result;

use arcnix_core::file_capabilities::{
    DirectoryReader, DocumentReader, EntryKind, FileEntryCreator, TextPage, TextPageReader,
};
use arcnix_core::remote::RemoteSession;
use arcnix_core::text_files::{Document, TextSnapshot, Version};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{btn, icon_action};
use crate::*;

#[derive(Clone, Default)]
pub(crate) struct Runtime {
    pub viewport: Option<native_viewport::Viewport>,
    pub open: Vec<String>,
    pub folder: String,
    pub root: String,
    pub terminals: crate::code_terminal::Sessions,
    pub commands: Vec<&'static str>,
    pub pending: std::collections::BTreeMap<u64, Pending>,
    pub background: std::collections::BTreeMap<u64, u64>,
    pub replies: Vec<Value>,
    pub dirty: bool,
    pub confirm: bool,
    pub error: String,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum FileRequest {
    Stat {
        path: String,
    },
    List {
        path: String,
    },
    SearchDirectory {
        path: String,
        max_bytes: Option<u64>,
    },
    Create {
        path: String,
    },
    Mkdir {
        path: String,
    },
    Read {
        path: String,
    },
    ReadRange {
        handle: Option<u64>,
        path: String,
        offset: u64,
        snapshot: arcnix_core::text_files::TextSnapshot,
        cursor: Option<u64>,
    },
    PollTail {
        handle: Option<u64>,
        path: String,
        snapshot: arcnix_core::text_files::TextSnapshot,
        stamp: Option<[u8; 32]>,
    },
    CountLines {
        handle: Option<u64>,
        path: String,
        offset: u64,
        snapshot: arcnix_core::text_files::TextSnapshot,
        cursor: u64,
    },
    ClosePage {
        handle: Option<u64>,
    },
    CloseRange {
        cursor: Option<u64>,
    },
    Image {
        path: String,
    },
}

#[derive(Deserialize)]
pub(crate) struct Write {
    path: String,
    version: Version,
    bom: bool,
    crlf: bool,
    content: String,
    #[serde(skip)]
    cancel: Arc<AtomicBool>,
}

impl Drop for Write {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub(crate) enum Pending {
    File(u64),
    Read { id: u64, elevated: bool },
    Page { id: u64, snapshot: TextSnapshot },
    Save { id: u64, write: Arc<Write> },
}

fn page_snapshot(
    file: bool,
    size: u64,
    modified: Option<u64>,
) -> Option<arcnix_core::text_files::TextSnapshot> {
    (file && size > arcnix_core::text_files::LIMIT as u64)
        .then_some(arcnix_core::text_files::TextSnapshot { size, modified })
}

pub enum Files<'a> {
    Remote(&'a mut RemoteSession),
    Local(&'a arcnix_core::local_desktop::Target),
}

impl Files<'_> {
    fn list(&self, path: &str) -> anyhow::Result<(String, Vec<arcnix_core::FileEntry>)> {
        match self {
            Self::Remote(remote) => remote.list_entries(path),
            Self::Local(target) => target.list_entries(path),
        }
    }

    fn create(&self, path: &str, directory: bool) -> anyhow::Result<()> {
        let kind = EntryKind::from_directory(directory);
        match self {
            Self::Remote(remote) => remote.create_entry(path, kind),
            Self::Local(target) => target.create_entry(path, kind),
        }
    }

    fn pages(&mut self) -> &mut dyn TextPageReader {
        match self {
            Self::Remote(remote) => *remote,
            Self::Local(target) => target,
        }
    }
}

impl FileRequest {
    pub fn execute(
        self,
        mut files: Files<'_>,
        check: impl Fn() -> anyhow::Result<()>,
    ) -> anyhow::Result<Value> {
        use base64::Engine;
        check()?;
        match self {
            Self::Create { path } => {
                files.create(&path, false)?;
                Ok(Value::Null)
            }
            Self::Mkdir { path } => {
                files.create(&path, true)?;
                Ok(Value::Null)
            }
            Self::Stat { path } => {
                let m = match &files {
                    Files::Remote(remote) => remote.file_metadata(&path)?,
                    Files::Local(target) => target.metadata(&path)?,
                };
                Ok(
                    json!({"type":if m.directory {2} else {1}, "size":m.size,"ctime":0,
                    "mtime":m.modified.unwrap_or_default()*1000,
                    "pageSnapshot":page_snapshot(!m.directory && !m.symlink, m.size, m.modified)}),
                )
            }
            Self::List { path } => {
                let (_, entries) = files.list(&path)?;
                Ok(Value::Array(
                    entries
                        .into_iter()
                        .filter(|e| !e.is_symlink)
                        .map(|e| {
                            json!([
                                e.name,
                                if e.is_dir { 2 } else { 1 },
                                page_snapshot(!e.is_dir, e.size, e.modified)
                            ])
                        })
                        .collect(),
                ))
            }
            Self::Image { path } => {
                let image = match files {
                    Files::Remote(remote) => {
                        arcnix_core::desktop_customization::read_wallpaper(remote, &path)?
                    }
                    Files::Local(target) => {
                        arcnix_core::desktop_customization::read_image(&path, |limit| {
                            use std::io::Read;
                            let (file, snapshot) = target.open_binary(&path)?;
                            anyhow::ensure!(
                                snapshot.size <= limit as u64,
                                "Image exceeds maximum size"
                            );
                            let mut bytes = Vec::with_capacity(snapshot.size as usize + 1);
                            file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
                            Ok(bytes)
                        })?
                    }
                };
                Ok(json!(format!(
                    "data:{};base64,{}",
                    image.mime,
                    base64::engine::general_purpose::STANDARD.encode(image.bytes)
                )))
            }
            Self::SearchDirectory { path, max_bytes } => {
                let (canonical, entries) = files.list(&path)?;
                anyhow::ensure!(canonical == path, "Search directory changed");
                Ok(Value::Array(
                    entries
                        .into_iter()
                        .filter(|entry| {
                            !entry.is_symlink
                                && (entry.is_dir
                                    || (max_bytes
                                        .is_none_or(|limit| entry.size > 0 && entry.size <= limit)
                                        && entry
                                            .permissions
                                            .is_none_or(|mode| mode & 0o170000 == 0o100000)))
                        })
                        .map(|entry| {
                            let snapshot = arcnix_core::text_files::TextSnapshot {
                                size: entry.size,
                                modified: entry.modified,
                            };
                            let paged = entry.size > arcnix_core::text_files::LIMIT as u64;
                            json!([entry.name, entry.is_dir, snapshot, paged])
                        })
                        .collect(),
                ))
            }
            Self::Read { path } => {
                let d = match files {
                    Files::Remote(remote) => remote.read_document(&path, &check)?,
                    Files::Local(target) => target.read_document(&path, &check)?,
                };
                Ok(json!({"content":d.content,"version":d.version,"bom":d.bom,"crlf":d.crlf}))
            }
            Self::ReadRange {
                handle,
                path,
                offset,
                snapshot,
                cursor,
            } => Ok(serde_json::to_value(files.pages().read_page(
                TextPage {
                    path: &path,
                    snapshot,
                    reviewed_handle: handle,
                },
                offset,
                cursor,
                &check,
            )?)?),
            Self::PollTail {
                handle,
                path,
                snapshot,
                stamp,
            } => {
                let (snapshot, stamp) = files.pages().sample_page_tail(
                    TextPage {
                        path: &path,
                        snapshot,
                        reviewed_handle: handle,
                    },
                    stamp,
                )?;
                Ok(json!({ "snapshot": snapshot, "stamp": stamp }))
            }
            Self::CountLines {
                handle,
                path,
                offset,
                snapshot,
                cursor,
            } => {
                let page = files.pages().read_page(
                    TextPage {
                        path: &path,
                        snapshot,
                        reviewed_handle: handle,
                    },
                    offset,
                    Some(cursor),
                    &check,
                )?;
                Ok(json!({"next":page.next, "breaks":page.breaks}))
            }
            Self::ClosePage { handle } => {
                files.pages().close_reviewed_page(handle);
                Ok(Value::Null)
            }
            Self::CloseRange { cursor } => {
                files.pages().close_page_cursor(cursor);
                Ok(Value::Null)
            }
        }
    }
}

pub(crate) fn save(ui: &Ui, id: u64, write: Arc<Write>, sudo: Option<worker::TextAuthorization>) {
    let serial = ui.send(Action::TextSave {
        document: Document {
            path: write.path.clone(),
            version: write.version.clone(),
            bom: write.bom,
            crlf: write.crlf,
            content: String::new(),
        },
        content: write.content.clone(),
        cancel: write.cancel.clone(),
        sudo,
    });
    ui.state
        .revise(|s| s.code.pending.insert(serial, Pending::Save { id, write }));
}

pub(crate) fn retry_save(ui: &Ui, serial: u64, authorization: worker::TextAuthorization) {
    if let Some(Pending::Save { id, write }) = ui.state.revise(|s| s.code.pending.remove(&serial)) {
        save(ui, id, write, Some(authorization));
    }
}

pub(crate) fn read(ui: &Ui, id: u64, path: String, sudo: Option<worker::TextAuthorization>) {
    let elevated = sudo.is_some();
    let serial = ui.send(Action::TextRead {
        path,
        cancel: Arc::new(AtomicBool::new(false)),
        sudo,
    });
    ui.state.revise(|s| {
        s.code
            .pending
            .insert(serial, Pending::Read { id, elevated })
    });
}

pub(crate) fn attach_page(
    ui: &Ui,
    id: u64,
    path: String,
    snapshot: TextSnapshot,
    sudo: Option<worker::TextAuthorization>,
) {
    let serial = ui.send(Action::AttachCodePage {
        path,
        snapshot,
        sudo,
    });
    ui.state.revise(|s| {
        s.code
            .pending
            .insert(serial, Pending::Page { id, snapshot })
    });
}

pub(crate) fn retry_read(
    ui: &Ui,
    serial: u64,
    retry: arcnix_core::remote::AdminTextRetry,
    password: zeroize::Zeroizing<String>,
) {
    match ui.state.revise(|s| s.code.pending.remove(&serial)) {
        Some(Pending::Read { id, .. }) => read(ui, id, retry.path.clone(), Some((retry, password))),
        Some(Pending::Page { id, snapshot }) => attach_page(
            ui,
            id,
            retry.path.clone(),
            snapshot,
            Some((retry, password)),
        ),
        _ => {}
    }
}

pub(crate) fn document_reply(
    s: &mut State,
    serial: u64,
    result: Result<Document, worker::FileFailure>,
) {
    let request = s.code.pending.get(&serial);
    let reading = matches!(request, Some(Pending::Read { .. } | Pending::Page { .. }));
    let elevated = matches!(request, Some(Pending::Read { elevated: true, .. }));
    let value = match result {
        Ok(d) if reading => Ok(
            json!({"content":d.content,"version":d.version,"bom":d.bom,"crlf":d.crlf,"elevated":elevated}),
        ),
        Ok(d) => Ok(json!(d.version)),
        Err(error) => {
            let message = error.text_message(reading, crate::i18n::workspace_language(s));
            if available(s)
                && let Some(retry) = error.text_retry
            {
                file_elevation::show(
                    s,
                    if reading {
                        file_elevation::Retry::CodeRead(serial, retry)
                    } else {
                        file_elevation::Retry::CodeSave(serial, retry)
                    },
                );
                return;
            }
            Err(message)
        }
    };
    reply(s, serial, value);
}

pub(crate) fn reply(s: &mut State, serial: u64, result: Result<Value, String>) {
    let id = match s.code.pending.remove(&serial) {
        Some(
            Pending::File(id)
            | Pending::Read { id, .. }
            | Pending::Page { id, .. }
            | Pending::Save { id, .. },
        ) => Some(id),
        None => s.code.background.remove(&serial),
    };
    if let Some(id) = id {
        s.code.replies.push(match result {
            Ok(value) => json!({"id":id,"value":value}),
            Err(error) => json!({"id":id,"error":error}),
        });
    }
}
pub(crate) fn available(s: &State) -> bool {
    s.connected && s.sftp_available && s.dialog.is_none() && !s.system_task_running
}
pub(crate) fn open(ui: &Ui, path: &str) {
    if !ui.state.with(available) {
        return;
    }
    ui.state.revise(|s| s.code.open.push(path.to_owned()));
    ui.open_desktop_app(WorkspacePage::Code);
}
pub(crate) fn close(ui: &Ui) -> bool {
    ui.state.revise(|s| {
        if s.code.dirty
            || s.code
                .pending
                .values()
                .any(|op| matches!(op, Pending::Save { .. }))
        {
            s.code.confirm = true;
            return false;
        }
        true
    })
}
pub(crate) fn title_tools(ui: Ui) -> impl Widget {
    let state = ui.state;
    let command = move |id: &'static str| move || state.revise(|s| s.code.commands.push(id));
    row().class("code-title-tools").fill((
        #[cfg(windows)]
        view().class("code-menu-slot"),
        icon_action("Back", Icon::Go)
            .class("icon-button code-back")
            .on_press(command("workbench.action.navigateBack")),
        icon_action("Forward", Icon::Go)
            .class("icon-button")
            .on_press(command("workbench.action.navigateForward")),
        sapid::button()
            .name(i18n::bind("Quick open"))
            .class("code-quick-open")
            .fill((
                icons::glyph(Icon::Search),
                text().content(state.map(|s| s.code.folder.clone())),
            ))
            .on_press(command("workbench.action.quickOpen")),
        icon_action("Customize layout", Icon::Grid)
            .class("icon-button code-layout")
            .on_press(command("workbench.action.customizeLayout")),
        icon_action("Toggle sidebar", Icon::Sidebar)
            .class("icon-button")
            .on_press(command("workbench.action.toggleSidebarVisibility")),
        icon_action("Toggle panel", Icon::Sidebar)
            .class("icon-button code-panel")
            .on_press(command("workbench.action.togglePanel")),
        icon_action("Toggle secondary sidebar", Icon::Sidebar)
            .class("icon-button code-secondary")
            .on_press(command("workbench.action.toggleAuxiliaryBar")),
    ))
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("code-workbench").fill((
        input()
            .class("browser-viewport-input hidden")
            .value("")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event
                    && value.len() < 8192
                    && let Ok(v) = serde_json::from_str::<native_viewport::Viewport>(&value)
                    && v.valid()
                    && state.with(|s| s.epoch == v.epoch && s.code.viewport.as_ref() != Some(&v))
                {
                    state.revise(|s| s.code.viewport = Some(v));
                }
            }),
        text()
            .class("browser-epoch hidden")
            .content(state.map(|s| s.epoch.to_string())),
        when(state.map(|s| s.code.confirm), move || {
            let discard = ui.clone();
            col().class("editor-discard").fill((
                text().content(i18n::bind("Discard unsaved changes in all open files?")),
                row().class("choice-list").fill((
                    btn("Keep editing", move || {
                        state.revise(|s| s.code.confirm = false)
                    }),
                    btn("Discard changes", move || {
                        discard.finish_desktop_close(WorkspacePage::Code);
                    }),
                )),
            ))
        }),
        text()
            .class("code-workbench-error")
            .content(state.map(|s| s.code.error.clone())),
        col().class("browser-surface"),
    ))
}
