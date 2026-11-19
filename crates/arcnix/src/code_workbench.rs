//! Files exposed to the local workbench through the existing SSH worker.
use arcnix_core::remote::RemoteSession;
use arcnix_core::text_files::{Document, Version};
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

#[derive(Clone, Default)]
pub(super) struct Runtime {
    pub viewport: Option<native_viewport::Viewport>,
    pub open: Vec<String>,
    pub folder: String,
    pub root: String,
    pub terminals: crate::code_terminal::Sessions,
    pub commands: Vec<&'static str>,
    pub pending: std::collections::BTreeMap<u64, u64>,
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
        path: String,
        offset: u64,
        snapshot: arcnix_core::text_files::TextSnapshot,
        cursor: Option<u64>,
    },
    PollTail {
        path: String,
        snapshot: arcnix_core::text_files::TextSnapshot,
        stamp: Option<[u8; 32]>,
    },
    CountLines {
        path: String,
        offset: u64,
        snapshot: arcnix_core::text_files::TextSnapshot,
        cursor: u64,
    },
    CloseRange {
        cursor: Option<u64>,
    },
    Image {
        path: String,
    },
    Write {
        path: String,
        version: Version,
        bom: bool,
        crlf: bool,
        content: String,
    },
}

fn page_snapshot(
    file: bool,
    size: u64,
    modified: Option<u64>,
) -> Option<arcnix_core::text_files::TextSnapshot> {
    (file && size > arcnix_core::text_files::LIMIT as u64)
        .then_some(arcnix_core::text_files::TextSnapshot { size, modified })
}

impl FileRequest {
    pub fn execute(
        self,
        remote: &mut RemoteSession,
        check: impl Fn() -> anyhow::Result<()>,
    ) -> anyhow::Result<Value> {
        use base64::Engine;
        check()?;
        match self {
            Self::Create { path } => {
                remote.create_file(&path)?;
                Ok(Value::Null)
            }
            Self::Mkdir { path } => {
                remote.mkdir(&path)?;
                Ok(Value::Null)
            }
            Self::Stat { path } => {
                let m = remote.file_metadata(&path)?;
                Ok(
                    json!({"type":if m.directory {2} else {1}, "size":m.size,"ctime":0,
                    "mtime":m.modified.unwrap_or_default()*1000,
                    "pageSnapshot":page_snapshot(!m.directory && !m.symlink, m.size, m.modified)}),
                )
            }
            Self::List { path } => {
                let (_, entries) = remote.list_dir(&path)?;
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
                let image = arcnix_core::desktop_customization::read_wallpaper(remote, &path)?;
                Ok(json!(format!(
                    "data:{};base64,{}",
                    image.mime,
                    base64::engine::general_purpose::STANDARD.encode(image.bytes)
                )))
            }
            Self::SearchDirectory { path, max_bytes } => {
                let (canonical, entries) = remote.list_dir(&path)?;
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
                let d = remote.read_text(&path, check)?;
                Ok(json!({"content":d.content,"version":d.version,"bom":d.bom,"crlf":d.crlf}))
            }
            Self::ReadRange {
                path,
                offset,
                snapshot,
                cursor,
            } => Ok(serde_json::to_value(
                remote.read_text_range(&path, offset, snapshot, cursor, check)?,
            )?),
            Self::PollTail {
                path,
                snapshot,
                stamp,
            } => {
                let (snapshot, stamp) = remote.poll_text_tail(&path, snapshot, stamp)?;
                Ok(json!({ "snapshot": snapshot, "stamp": stamp }))
            }
            Self::CountLines {
                path,
                offset,
                snapshot,
                cursor,
            } => {
                let page = remote.read_text_range(&path, offset, snapshot, Some(cursor), check)?;
                Ok(json!({"next":page.next, "breaks":page.breaks}))
            }
            Self::CloseRange { cursor } => {
                remote.close_search_reader(cursor);
                Ok(Value::Null)
            }
            Self::Write {
                path,
                version,
                bom,
                crlf,
                content,
            } => {
                let baseline = Document {
                    path,
                    content: String::new(),
                    version,
                    bom,
                    crlf,
                };
                let saved = remote.save_text(&baseline, &content, check)?;
                Ok(json!(saved.version))
            }
        }
    }
}

pub(super) fn reply(ui: &Ui, serial: u64, result: std::result::Result<Value, String>) {
    ui.state.revise(|s| {
        if let Some(id) = s
            .code
            .pending
            .remove(&serial)
            .or_else(|| s.code.background.remove(&serial))
        {
            s.code.replies.push(match result {
                Ok(value) => json!({"id":id,"value":value}),
                Err(error) => json!({"id":id,"error":error}),
            });
        }
    });
}
pub(super) fn available(s: &State) -> bool {
    s.connected && s.sftp_available && s.dialog.is_none() && !s.system_task_running
}
pub(super) fn open(ui: &Ui, path: &str) {
    if !ui.state.with(available) {
        return;
    }
    ui.state.revise(|s| s.code.open.push(path.to_owned()));
    ui.open_desktop_app(WorkspacePage::Code);
}
pub(super) fn close(ui: &Ui) -> bool {
    ui.state.revise(|s| {
        if !s.code.pending.is_empty() {
            return false;
        }
        if s.code.dirty {
            s.code.confirm = true;
            return false;
        }
        true
    })
}
pub(super) fn title_tools(ui: Ui) -> impl Widget {
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
            .name(i18n::bind("Quick Open"))
            .class("code-quick-open")
            .fill((
                icons::glyph(Icon::Search),
                text().content(state.map(|s| s.code.folder.clone())),
            ))
            .on_press(command("workbench.action.quickOpen")),
        icon_action("Customize Layout", Icon::Grid)
            .class("icon-button code-layout")
            .on_press(command("workbench.action.customizeLayout")),
        icon_action("Toggle Side Bar", Icon::Sidebar)
            .class("icon-button")
            .on_press(command("workbench.action.toggleSidebarVisibility")),
        icon_action("Toggle Panel", Icon::Sidebar)
            .class("icon-button code-panel")
            .on_press(command("workbench.action.togglePanel")),
        icon_action("Toggle Secondary Side Bar", Icon::Sidebar)
            .class("icon-button code-secondary")
            .on_press(command("workbench.action.toggleAuxiliaryBar")),
    ))
}

pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("code-workbench").fill((
        input()
            .class("browser-viewport-input hidden")
            .value("")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event
                    && value.len() < 8192
                    && let Ok(v) = serde_json::from_str::<native_viewport::Viewport>(&value)
                    && v.clip.len() <= 32
                    && v.area.is_none_or(native_viewport::BoxPx::valid)
                    && v.clip.iter().all(|b| b.valid())
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
