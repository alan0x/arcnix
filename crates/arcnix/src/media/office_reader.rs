//! Offline Office imports and the local desktop reader.
use std::result::Result;

use arcnix_core::file_associations::{FileApp, extension, filename};

use crate::i18n::desktop as i18n;
use crate::*;

pub(crate) const LIMIT: usize = 32 * 1024 * 1024;
pub(crate) const EXTENSIONS: &[&str] = &["docx", "xlsx", "pptx"];

pub(crate) fn supports(path: &str) -> bool {
    extension(path).is_some_and(|ext| EXTENSIONS.contains(&ext.as_str()))
}

pub(crate) fn local_target(target: &local_desktop::Target) -> bool {
    matches!(
        target,
        local_desktop::Target::Native | local_desktop::Target::ReadOnlyHost
    )
}

pub(crate) fn available(s: &State) -> bool {
    s.local_desktop
        .as_ref()
        .map_or(s.connected && s.sftp_available, local_target)
}

pub(crate) struct Document {
    pub remote: Option<office_cache::Copy>,
    pub html: String,
    writer: Option<Arc<str>>,
    pub width_px: f64,
    pub kind: &'static str,
}

pub(crate) fn render(path: &str, bytes: &[u8]) -> Result<Document, String> {
    use syntra_export::{docx, pdf, pptx, xlsx};
    let (kind, model) = match extension(path).as_deref() {
        Some("docx") => ("writer", serde_json::to_value(docx::docx_to_writer(bytes)?)),
        Some("xlsx") => ("calc", serde_json::to_value(xlsx::xlsx_to_workbook(bytes)?)),
        Some("pptx") => (
            "slides",
            serde_json::to_value(pptx::pptx_to_presentation(bytes)?),
        ),
        _ => return Err("Supported formats are DOCX, XLSX and PPTX.".into()),
    };
    let model = model.map_err(|error| error.to_string())?;
    let writer = (kind == "writer").then(|| Arc::from(model.to_string()));
    let print = pdf::document_to_print_html(&model, kind)?;
    Ok(Document {
        remote: None,
        html: print.html,
        writer,
        width_px: print.width_mm * 96. / 25.4,
        kind,
    })
}

#[derive(Clone, Default)]
pub(crate) struct Reader {
    remote: Option<office_cache::Copy>,
    pub document: Option<Arc<Document>>,
    pub viewport: Option<native_viewport::Viewport>,
    pub error: String,
    pub draft: Option<Arc<str>>,
    pub dirty: bool,
    saving: Option<u64>,
    request: Option<u64>,
    list_request: Option<u64>,
    pending_paths: Vec<String>,
    path: String,
}

pub(crate) fn browse(ui: &Ui) {
    desktop_file_picker::open(
        ui,
        desktop_file_picker::Purpose::Office,
        ui.state
            .with(|s| desktop_file_picker::parent(&s.office.path)),
    );
}

pub(crate) fn open(ui: &Ui, path: &str) {
    load(ui, path, None);
}

fn load(ui: &Ui, path: &str, replace: Option<office_cache::Record>) {
    if !ui
        .state
        .with(|s| available(s) && code_workbench::available(s))
    {
        return;
    }
    ui.open_desktop_app(WorkspacePage::Office);
    if !can_leave(ui) {
        return;
    }
    let supported = supports(path);
    let request = supported.then(|| {
        ui.send(Action::OpenOffice(
            path.into(),
            ui.store.as_ref().clone(),
            replace,
        ))
    });
    ui.state.revise(|s| {
        s.office = Reader {
            request,
            path: path.into(),
            viewport: s.office.viewport.take(),
            error: if supported {
                String::new()
            } else {
                "Supported formats are DOCX, XLSX and PPTX.".into()
            },
            ..Default::default()
        };
    });
}

pub(crate) fn received(ui: &Ui, serial: u64, result: Result<Document, String>) {
    let opened = ui.state.revise(|s| {
        let reader = &mut s.office;
        if reader.request != Some(serial) {
            return None;
        }
        reader.request = None;
        match result {
            Ok(document) => {
                reader.set_document(document, "");
                Some(reader.path.clone())
            }
            Err(message) => {
                reader.error = message;
                None
            }
        }
    });
    if let Some(path) = opened {
        file_open::remember(ui, &path, FileApp::Office);
    }
}

pub(crate) fn pending_received(ui: &Ui, serial: u64, result: Result<Vec<String>, String>) {
    ui.state.revise(|s| {
        if s.office.list_request != Some(serial) {
            return;
        }
        s.office.list_request = None;
        match result {
            Ok(paths) => {
                s.office.error = if paths.is_empty() {
                    "No pending uploads.".into()
                } else {
                    String::new()
                };
                s.office.pending_paths = paths;
            }
            Err(error) => s.office.error = error,
        }
    });
}

fn pending_uploads(ui: Ui) -> impl Widget {
    let state = ui.state;
    let browse = ui.clone();
    when(
        state.map(|s| {
            s.local_desktop.is_none()
                && available(s)
                && s.office.draft.is_none()
                && !s.office.is_saving()
        }),
        move || {
            let ui = ui.clone();
            let browse = browse.clone();
            fragment((
                i18n::btn("Pending uploads", move || {
                    let serial = browse.send(Action::PendingOffice(browse.store.as_ref().clone()));
                    state.revise(|s| s.office.list_request = Some(serial));
                })
                .enabled(state.map(|s| s.office.list_request.is_none())),
                when(
                    state.map(|s| !s.office.pending_paths.is_empty()),
                    move || {
                        let ui = ui.clone();
                        col().class("office-pending-files").fill(
                            each(state.map(|s| s.office.pending_paths.clone()))
                                .key(|path| path.clone())
                                .children(move |item| {
                                    let path = item.value.get();
                                    let ui = ui.clone();
                                    sapid::button()
                                        .class("button")
                                        .label(path.clone())
                                        .on_press(move || open(&ui, &path))
                                }),
                        )
                    },
                ),
            ))
        },
    )
}

fn sync_actions(ui: Ui) -> impl Widget {
    let state = ui.state;
    let visible = state.map(|s| {
        s.office.draft.is_none()
            && !s.office.is_saving()
            && s.office.remote().is_some_and(|c| c.recovery_path.is_some())
    });
    let pending = |s: &State| s.office.remote().is_some_and(office_cache::Copy::pending);
    when(visible, move || {
        let ui = ui.clone();
        let retry = ui.clone();
        row().class("viewer-toolbar").fill((
            text().content(i18n::bind(state.map(move |s| {
                if pending(s) {
                    "Local copy pending upload".to_owned()
                } else {
                    "Preserved local copy".to_owned()
                }
            }))),
            text()
                .class("office-cache-path")
                .selectable(true)
                .content(state.map(|s| {
                    s.office
                        .remote()
                        .and_then(|c| c.recovery_path.as_ref())
                        .map(|path| path.display().to_string())
                        .unwrap_or_default()
                })),
            when(state.map(pending), move || {
                let retry = retry.clone();
                let server = ui.clone();
                fragment((
                    i18n::btn("Retry upload", move || sync_remote(&retry, true)),
                    i18n::btn("Open server version", move || {
                        if let Some(record) = server
                            .state
                            .with(|s| s.office.remote().map(|c| c.record.clone()))
                        {
                            load(&server, &record.path.clone(), Some(record));
                        }
                    }),
                ))
            }),
        ))
    })
}

pub(super) fn can_edit(s: &State) -> bool {
    (s.local_desktop == Some(local_desktop::Target::Native) || s.office.remote().is_some())
        && s.office.draft.is_none()
        && !s.office.is_saving()
        && s.office
            .document
            .as_ref()
            .is_some_and(|d| d.writer.is_some())
}

pub(super) fn begin_edit(ui: &Ui) {
    ui.state.revise(|s| {
        if can_edit(s) {
            s.office.draft = s.office.document.as_ref().and_then(|d| d.writer.clone());
            s.office.error.clear();
        }
    });
}

pub(crate) fn panel(ui: Ui) -> impl Widget {
    let state = ui.state;
    let browse_ui = ui.clone();
    let pending_files = ui.clone();
    let recover_ui = ui.clone();
    let pending_ui = ui.clone();
    col().class("office-reader").fill((
        row().class("office-document-strip").fill((
            when(state.map(|s| s.office.document.is_none() && s.office.draft.is_none()), {
                let ui = ui.clone();
                move || {
                    let ui = ui.clone();
                    sapid::dropdown_menu("office-file").label(i18n::bind("File")).item(
                        sapid::menu_item("open-office").label(i18n::bind("Open Office document"))
                            .enabled(state.map(available)).on_activate(move || browse(&ui)))
                }
            }),
            text().class("viewer-filename").content(state.map(|s| filename(&s.office.path).to_owned())),
            pending_uploads(pending_files),
        )),
        sync_actions(pending_ui),
        when(
            state.map(|s| s.office.document.is_some() && !s.office.error.is_empty()),
            move || {
                text()
                    .class("office-message")
                    .content(i18n::bind(state.map(|s| s.office.error.clone())))
            },
        ),
        when(
            state.map(|s| s.office.document.is_none() && s.office.draft.is_some()),
            move || {
                let save = recover_ui.clone();
                let discard = recover_ui.clone();
                row().class("viewer-toolbar").fill((
                    i18n::btn(if state.with(|s| s.office.remote().is_some()) {
                        "Save to server"
                    } else { "Save as…" }, move || save_dialog(&save)),
                    i18n::btn("Discard edits", move || {
                        let path = discard.state.revise(|s| {
                            s.office.draft = None;
                            s.office.dirty = false;
                            s.office.path.clone()
                        });
                        open(&discard, &path);
                    }),
                ))
            },
        ),
        input()
            .class("browser-viewport-input hidden")
            .value("")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event
                    && value.len() < 8192
                    && let Ok(v) = serde_json::from_str::<native_viewport::Viewport>(&value)
                    && v.valid()
                    && state.with(|s| s.epoch == v.epoch && s.office.viewport.as_ref() != Some(&v))
                {
                    state.revise(|s| s.office.viewport = Some(v));
                }
            }),
        text()
            .class("browser-epoch hidden")
            .content(state.map(|s| s.epoch.to_string())),
        col().class("browser-surface").fill(when(
            state.map(|s| s.office.document.is_none()),
            move || {
                let browse_ui = browse_ui.clone();
                col().class("viewer-placeholder").fill((
                    icons::glyph(Icon::Syntra),
                    when(state.map(|s| available(s) && s.office.draft.is_none() && s.office.request.is_none()), move || {
                        let ui = browse_ui.clone();
                        i18n::text_action("Open Office document", Icon::Folder).on_press(move || browse(&ui))
                    }),
                    text().content(i18n::bind(state.map(|s| {
                        if !s.office.error.is_empty() {
                            return s.office.error.clone();
                        }
                        if s.office.request.is_some() {
                            "Opening Office document…"
                        } else if s.local_desktop.is_none() && !s.sftp_available {
                            "Connect to a server with SFTP to open remote Office files."
                        } else if !available(s) {
                            "To read a file on this computer, select “Try the local desktop” in the server list, then open Office."
                        } else {
                            "Open a DOCX, XLSX or PPTX file to read it."
                        }
                        .into()
                    }))),
                ))
            },
        )),
    ))
}

/// Keep the working copy available until the user saves it or returns to reading.
pub(crate) fn can_leave(ui: &Ui) -> bool {
    ui.state.revise(|s| {
        if s.office.saving.is_some() || s.office.draft.is_some() {
            s.office.error =
                "Finish editing or return to reading before leaving this document.".into();
            false
        } else {
            true
        }
    })
}

fn sync_remote(ui: &Ui, retry: bool) {
    let Some((record, model)) = ui.state.with(|s| {
        if s.office.is_saving() || !available(s) {
            return None;
        }
        let remote = s.office.remote()?;
        let model = if retry {
            None
        } else {
            Some(s.office.draft.clone()?)
        };
        Some((remote.record.clone(), model))
    }) else {
        return;
    };
    let serial = ui.send(Action::SyncOffice(ui.store.as_ref().clone(), record, model));
    ui.state.revise(|s| {
        s.office.saving = Some(serial);
        s.office.error = "Saving and uploading…".into();
    });
}

pub(crate) fn save_dialog(ui: &Ui) {
    if ui.state.with(|s| s.office.remote().is_some()) {
        return sync_remote(ui, false);
    }
    let Some(path) = ui.state.with(|s| {
        (s.local_desktop == Some(local_desktop::Target::Native)
            && s.office.draft.is_some()
            && s.office.saving.is_none()
            && s.dialog.is_none())
        .then(|| s.office.path.clone())
    }) else {
        return;
    };
    desktop_file_picker::open(
        ui,
        desktop_file_picker::Purpose::SaveOffice,
        desktop_file_picker::parent(&path),
    );
    if ui.state.with(desktop_file_picker::current) == Some(desktop_file_picker::Purpose::SaveOffice)
    {
        let stem = filename(&path)
            .rsplit_once('.')
            .map_or(filename(&path), |(stem, _)| stem);
        desktop_file_picker::set_save_name(
            ui,
            format!("{stem} - {}.docx", i18n::translate("Copy")),
        );
    }
}

pub(crate) fn submit_save(ui: &Ui, path: &str) {
    let path = if extension(path).is_none() {
        format!("{path}.docx")
    } else {
        path.to_owned()
    };
    if extension(&path).as_deref() != Some("docx") || ui.state.with(|s| s.office.path == path) {
        ui.state
            .revise(|s| s.dialog_error = i18n::translate("Choose a new DOCX file name."));
        return;
    }
    let Some(model) = ui.state.with(|s| {
        if s.local_desktop != Some(local_desktop::Target::Native) || s.office.saving.is_some() {
            return None;
        }
        s.office.draft.clone()
    }) else {
        return;
    };
    ui.close_dialog();
    let serial = ui.send(Action::SaveOffice(path, model));
    ui.state.revise(|s| {
        s.office.saving = Some(serial);
        s.office.error = "Saving DOCX…".into();
    });
}

pub(crate) fn save_copy(
    target: Option<&local_desktop::Target>,
    path: &str,
    model: &str,
) -> anyhow::Result<Document> {
    use std::io::Write;

    use anyhow::{Context, ensure};
    ensure!(
        target == Some(&local_desktop::Target::Native),
        "Saving requires the read-write local desktop."
    );
    ensure!(
        extension(path).as_deref() == Some("docx"),
        "Choose a new DOCX file name."
    );
    #[cfg(windows)]
    let dest = local_desktop::windows_host::physical(path)?;
    #[cfg(not(windows))]
    let dest = std::path::PathBuf::from(path);
    ensure!(dest.is_absolute(), "Expected an absolute local path.");
    ensure!(!dest.try_exists()?, "Choose a new DOCX file name.");
    let bytes = encode(model)?;
    let document = render(path, &bytes).map_err(anyhow::Error::msg)?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(dest.parent().context("Missing destination directory")?)?;
    tmp.write_all(&bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist_noclobber(dest)?;
    Ok(document)
}

pub(crate) fn encode(model: &str) -> anyhow::Result<Vec<u8>> {
    use anyhow::ensure;
    ensure!(
        model.len() <= syntra_doc_engine::MAX_DOCUMENT_CONTENT_BYTES,
        "Document exceeds the editing limit."
    );
    let writer = serde_json::from_str(model)?;
    let bytes = syntra_export::docx::writer_to_docx(&writer).map_err(anyhow::Error::msg)?;
    ensure!(bytes.len() <= LIMIT, "Exported DOCX exceeds 32 MiB.");
    Ok(bytes)
}

pub(crate) fn saved(ui: &Ui, serial: u64, result: Result<(String, Document), String>) {
    let saved = ui.state.revise(|s| {
        if s.office.saving != Some(serial) {
            return false;
        }
        s.office.saving = None;
        match result {
            Ok((path, document)) => {
                s.office.path = path;
                s.office.set_document(document, "Saved DOCX copy.");
                s.office.draft = None;
                s.office.dirty = false;
                true
            }
            Err(error) => {
                s.office.error = error;
                false
            }
        }
    });
    if saved {
        ui.refresh(true);
    }
}

impl Reader {
    fn set_document(&mut self, mut document: Document, local_notice: &str) {
        self.pending_paths.clear();
        self.list_request = None;
        self.remote = document.remote.take();
        self.error = self
            .remote
            .as_ref()
            .map_or(local_notice, |c| &c.notice)
            .to_owned();
        self.document = Some(Arc::new(document));
    }

    pub(super) fn remote(&self) -> Option<&office_cache::Copy> {
        self.remote.as_ref()
    }
    pub(crate) fn is_saving(&self) -> bool {
        self.saving.is_some()
    }
}

pub(crate) fn edit_message(ui: &Ui, web: &wry::WebView, message: &str) {
    if !ui.state.with(|s| {
        s.local_desktop == Some(local_desktop::Target::Native) || s.office.remote().is_some()
    }) {
        return;
    }
    match message {
        "save" => save_dialog(ui),
        "discard" => ui.state.revise(|s| {
            s.office.draft = None;
            s.office.dirty = false;
            s.office.error.clear();
        }),
        _ => {
            let model = if let Some(model) = message.strip_prefix("model\n") {
                Arc::from(model)
            } else if let Some((revision, text)) = message
                .strip_prefix("plain\n")
                .and_then(|s| s.split_once('\n'))
            {
                let Ok(revision) = revision.parse::<u64>() else {
                    return;
                };
                let Some(previous) = ui.state.with(|s| s.office.draft.clone()) else {
                    return;
                };
                let Ok(base) = serde_json::from_str(&previous) else {
                    return;
                };
                let model =
                    syntra_doc_engine::writer::writer_content_with_text(&base, text).to_string();
                let literal = serde_json::to_string(&model).expect("serializing a string");
                let _ =
                    web.evaluate_script(&format!("window.officeRestore?.({revision},{literal})"));
                Arc::from(model)
            } else {
                return;
            };
            ui.state.revise(|s| {
                s.office.draft = Some(model);
                s.office.dirty = true;
                s.office.error.clear();
            });
        }
    }
}
