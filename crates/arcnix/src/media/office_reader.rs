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
    source_hash: [u8; 32],
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
    let preview = pdf::document_to_reader_html(&model, kind)?;
    Ok(Document {
        remote: None,
        source_hash: <sha2::Sha256 as sha2::Digest>::digest(bytes).into(),
        html: preview.html,
        writer,
        width_px: preview.width_mm * 96. / 25.4,
        kind,
    })
}

#[derive(Clone, Default)]
pub(crate) struct Reader {
    remote: Option<office_cache::Copy>,
    pub document: Option<Arc<Document>>,
    pub error: String,
    pub draft: Option<Arc<str>>,
    pub dirty: bool,
    saving: Option<u64>,
    request: Option<u64>,
    list_request: Option<u64>,
    pending_paths: Vec<String>,
    pub(crate) path: String,
}

impl document_tabs::FileTab for Reader {
    fn path(&self) -> &str {
        &self.path
    }
}

pub(crate) fn browse(ui: &Ui) {
    desktop_file_picker::open(
        ui,
        desktop_file_picker::Purpose::Office,
        ui.state
            .with(|s| desktop_file_picker::parent(&s.office.current().path)),
    );
}

pub(crate) fn open(ui: &Ui, path: &str) {
    if ui.state.with(|s| office_sync::uploading(s, path)) {
        return;
    }
    load(ui, path, None);
}

fn load(ui: &Ui, path: &str, replace: Option<office_cache::Record>) {
    if !ui
        .state
        .with(|s| available(s) && code_workbench::available(s))
    {
        return;
    }
    let existing = ui.state.with(|s| s.office.find(path));
    if let Some((window, id)) = existing {
        document_tabs::select(ui, arcnix_core::server_apps::DesktopApp::Office, window, id);
        if replace.is_none()
            && ui.state.with(|s| {
                let doc = s.office.current();
                doc.document.is_some()
                    || doc.request.is_some()
                    || doc.draft.is_some()
                    || doc.is_saving()
            })
        {
            return;
        }
    } else {
        ui.open_desktop_app(WorkspacePage::Office);
    }
    if !supports(path) {
        ui.error("Supported formats are DOCX, XLSX and PPTX.");
        return;
    }
    let serial = ui.send(Action::OpenOffice(
        path.into(),
        ui.store.as_ref().clone(),
        replace,
    ));
    ui.state.revise(|s| {
        let reader = Reader {
            request: Some(serial),
            path: path.into(),
            ..Default::default()
        };
        if let Some((_, id)) = existing {
            s.office.files.insert(id, reader);
        } else {
            s.office.insert(serial, reader);
        }
    });
}

pub(crate) fn received(ui: &Ui, serial: u64, result: Result<Document, String>) {
    let opened = ui.state.revise(|s| {
        let reader = s
            .office
            .files
            .values_mut()
            .find(|r| r.request == Some(serial))?;
        reader.request = None;
        match result {
            Ok(document) => {
                reader.set_document(document, "", s.local_desktop.as_ref());
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
        let Some(reader) = s.office.all_mut().find(|r| r.list_request == Some(serial)) else {
            return;
        };
        reader.list_request = None;
        match result {
            Ok(paths) => {
                reader.error = if paths.is_empty() {
                    "No pending uploads.".into()
                } else {
                    String::new()
                };
                reader.pending_paths = paths;
            }
            Err(error) => reader.error = error,
        }
    });
}

fn pending_uploads(ui: Ui, window: u32) -> impl Widget {
    let state = ui.state;
    let browse = ui.clone();
    when(
        state.map(move |s| {
            s.local_desktop.is_none()
                && available(s)
                && s.office.selected(window).draft.is_none()
                && !s.office.selected(window).is_saving()
        }),
        move || {
            let ui = ui.clone();
            let browse = browse.clone();
            row().class("viewer-status office-upload-status").fill((
                i18n::btn("Pending uploads", move || {
                    let serial = browse.send(Action::PendingOffice(browse.store.as_ref().clone()));
                    state.revise(|s| s.office.selected_mut(window).list_request = Some(serial));
                })
                .enabled(state.map(move |s| s.office.selected(window).list_request.is_none())),
                when(
                    state.map(move |s| !s.office.selected(window).pending_paths.is_empty()),
                    move || {
                        let ui = ui.clone();
                        col().class("office-pending-files").fill(
                            each(
                                state.map(move |s| s.office.selected(window).pending_paths.clone()),
                            )
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

fn sync_actions(ui: Ui, window: u32) -> impl Widget {
    let state = ui.state;
    let visible = state.map(move |s| {
        s.office.selected(window).draft.is_none()
            && !s.office.selected(window).is_saving()
            && s.office
                .selected(window)
                .remote()
                .is_some_and(|c| c.recovery_path.is_some())
    });
    let pending = move |s: &State| {
        s.office
            .selected(window)
            .remote()
            .is_some_and(office_cache::Copy::pending)
    };
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
                .content(state.map(move |s| {
                    s.office
                        .selected(window)
                        .remote()
                        .and_then(|c| c.recovery_path.as_ref())
                        .map(|path| path.display().to_string())
                        .unwrap_or_default()
                })),
            when(state.map(pending), move || {
                let retry = retry.clone();
                let server = ui.clone();
                fragment((
                    i18n::btn("Retry upload", move || sync_selected(&retry, window, true)),
                    i18n::btn("Open server version", move || {
                        if let Some(record) = server
                            .state
                            .with(|s| s.office.selected(window).remote().map(|c| c.record.clone()))
                        {
                            load(&server, &record.path.clone(), Some(record));
                        }
                    }),
                ))
            }),
        ))
    })
}

pub(super) fn can_edit(s: &State, doc: &Reader) -> bool {
    (s.local_desktop == Some(local_desktop::Target::Native) || doc.remote().is_some())
        && doc.draft.is_none()
        && !doc.is_saving()
        && doc.document.as_ref().is_some_and(|d| d.writer.is_some())
}
pub(super) fn begin_edit(ui: &Ui, id: u64) {
    ui.state.revise(|s| {
        if s.office.files.get(&id).is_some_and(|r| can_edit(s, r)) {
            let doc = s.office.files.get_mut(&id).unwrap();
            doc.draft = doc.document.as_ref().and_then(|d| d.writer.clone());
            doc.error.clear();
        }
    });
}

pub(crate) fn panel(ui: Ui, window: u32) -> impl Widget {
    let state = ui.state;
    let browse_ui = ui.clone();
    let pending_files = ui.clone();
    let recover_ui = ui.clone();
    let pending_ui = ui.clone();
    col().class("office-reader").fill((
        when(state.map(move |s| s.office.selected(window).document.is_none() && s.office.selected(window).draft.is_none()), {
            let ui = ui.clone();
            move || {
                let ui = ui.clone();
                row().class("viewer-toolbar").fill(
                    sapid::dropdown_menu(format!("office-file-{window}")).label(i18n::bind("File")).item(
                        sapid::menu_item("open-office").label(i18n::bind("Open Office document"))
                            .enabled(state.map(available)).on_activate(move || browse(&ui))))
            }
        }),
        sync_actions(pending_ui,window),
        when(
            state.map(move |s| s.office.selected(window).document.is_some() && !s.office.selected(window).error.is_empty()),
            move || {
                text()
                    .class("office-message")
                    .content(i18n::bind(state.map(move |s| s.office.selected(window).error.clone())))
            },
        ),
        when(
            state.map(move |s| s.office.selected(window).document.is_none() && s.office.selected(window).draft.is_some()),
            move || {
                let save = recover_ui.clone();
                let discard = recover_ui.clone();
                row().class("viewer-toolbar").fill((
                    i18n::btn(if state.with(|s| s.office.selected(window).remote().is_some()) {
                        "Save to server"
                    } else { "Save as…" }, move || if let Some(id) = state.with(|s| s.office.groups.selected(window)) { save_dialog(&save,id) }),
                    i18n::btn("Discard edits", move || {
                        let path = discard.state.revise(|s| {
                            s.office.selected_mut(window).draft = None;
                            s.office.selected_mut(window).dirty = false;
                            s.office.selected(window).path.clone()
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
                    && state.with(|s| s.epoch == v.epoch && s.office_viewports.get(&window) != Some(&v))
                {
                    state.revise(|s| {s.office_viewports.insert(window,v);});
                }
            }),
        text()
            .class("browser-epoch hidden")
            .content(state.map(move |s| s.epoch.to_string())),
        col().class("browser-surface").fill(when(
            state.map(move |s| s.office.selected(window).document.is_none() || s.office.selected(window).is_saving()),
            move || {
                let browse_ui = browse_ui.clone();
                col().class("viewer-placeholder").fill((
                    when(state.map(move |s| s.office.selected(window).is_saving() && s.office.selected(window).remote().is_some()),
                        || icons::glyph(Icon::Upload)).otherwise(|| icons::glyph(Icon::Syntra)),
                    when(state.map(move |s| available(s) && s.office.selected(window).draft.is_none() && s.office.selected(window).request.is_none()), move || {
                        let ui = browse_ui.clone();
                        i18n::text_action("Open Office document", Icon::OpenDocument).on_press(move || browse(&ui))
                    }),
                    text().content(i18n::bind(state.map(move |s| {
                        if !s.office.selected(window).error.is_empty() {
                            return s.office.selected(window).error.clone();
                        }
                        if s.office.selected(window).request.is_some() {
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
        pending_uploads(pending_files, window),
    ))
}

pub(crate) fn can_close_tab(ui: &Ui, id: u64) -> bool {
    let blocked = ui.state.with(|s| {
        s.office
            .files
            .get(&id)
            .is_some_and(|doc| doc.dirty || doc.is_saving())
    });
    if blocked && let Some(window) = ui.state.with(|s| s.office.groups.owner(id)) {
        document_tabs::select(ui, arcnix_core::server_apps::DesktopApp::Office, window, id);
        ui.state.revise(|s| {
            s.office.current_mut().error =
                "Finish editing or return to reading before leaving this document.".into()
        });
    }
    !blocked
}

fn sync_selected(ui: &Ui, window: u32, retry: bool) {
    if let Some(id) = ui.state.with(|s| s.office.groups.selected(window)) {
        sync_remote(ui, id, retry);
    }
}
fn sync_remote(ui: &Ui, id: u64, retry: bool) {
    let Some((record, model)) = ui.state.with(|s| {
        let doc = s.office.files.get(&id)?;
        if doc.is_saving() || !available(s) {
            return None;
        }
        let remote = doc.remote()?;
        let model = if retry {
            None
        } else {
            Some(doc.draft.clone()?)
        };
        Some((remote.record.clone(), model))
    }) else {
        return;
    };
    let serial = ui.send(Action::SyncOffice(
        ui.store.as_ref().clone(),
        record.clone(),
        model,
    ));
    ui.state.revise(|s| {
        let doc = s.office.files.get_mut(&id).unwrap();
        doc.saving = Some(serial);
        doc.error = "Saving and uploading…".into();
        if let Some(profile) = s.selected.clone() {
            office_sync::record(
                s,
                &profile,
                record.server,
                record.path,
                serial,
                office_sync::Stage::Queued,
            );
        }
    });
}

pub(crate) fn confirm_save_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    each(state.map(|s| match s.dialog {
        Some(Dialog::ConfirmOfficeSave(id)) => vec![id],
        _ => Vec::new(),
    }))
    .key(|id| *id)
    .children(move |item| {
        let id = item.value.get();
        let ui = ui.clone();
        widget(move |scope| {
            let cancel = ui.clone();
            let copy = ui.clone();
            let save = ui.clone();
            let focus = scope.cage(None);
            let path = state.with(|s| {
                s.office
                    .files
                    .get(&id)
                    .map(|doc| doc.path.clone())
                    .unwrap_or_default()
            });
            modal(
                ui.clone(),
                "Save to the current file?",
                text()
                    .class("office-save-path settings-copyable")
                    .selectable(true)
                    .content(path),
                (
                    i18n::btn("Cancel", move || cancel.close_dialog()),
                    i18n::btn("Save as…", move || {
                        copy.close_dialog();
                        save_dialog(&copy, id);
                    }),
                    i18n::btn("Save", move || {
                        save.close_dialog();
                        save_current(&save, id);
                    })
                    .class("button primary")
                    .node_ref(focus),
                ),
            )
            .show_in(scope)?;
            if let Some(button) = focus.get() {
                scope.action(glory::render::Action::Focus(button))?;
            }
            Ok(())
        })
    })
}

fn save_current(ui: &Ui, id: u64) {
    let current = ui.state.with(|s| {
        let doc = s.office.files.get(&id)?;
        if s.local_desktop != Some(local_desktop::Target::Native)
            || doc.is_saving()
            || s.dialog.is_some()
        {
            return None;
        }
        Some((
            doc.path.clone(),
            doc.draft.clone()?,
            doc.document.as_ref()?.source_hash,
        ))
    });
    if let Some((path, model, hash)) = current {
        save_local(ui, id, path, model, Some(hash));
    }
}

pub(crate) fn save_dialog(ui: &Ui, id: u64) {
    if ui.state.with(|s| {
        s.office
            .files
            .get(&id)
            .is_some_and(|d| d.remote().is_some())
    }) {
        return sync_remote(ui, id, false);
    }
    let Some(path) = ui.state.with(|s| {
        let doc = s.office.files.get(&id)?;
        (s.local_desktop == Some(local_desktop::Target::Native)
            && doc.draft.is_some()
            && doc.saving.is_none()
            && s.dialog.is_none())
        .then(|| doc.path.clone())
    }) else {
        return;
    };
    desktop_file_picker::open(
        ui,
        desktop_file_picker::Purpose::SaveOffice(id),
        desktop_file_picker::parent(&path),
    );
    if ui.state.with(desktop_file_picker::current)
        == Some(desktop_file_picker::Purpose::SaveOffice(id))
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

pub(crate) fn submit_save(ui: &Ui, id: u64, path: &str) {
    let path = if extension(path).is_none() {
        format!("{path}.docx")
    } else {
        path.to_owned()
    };
    if extension(&path).as_deref() != Some("docx")
        || ui.state.with(|s| s.office.find(&path).is_some())
    {
        ui.state
            .revise(|s| s.dialog_error = i18n::translate("Choose a new DOCX file name."));
        return;
    }
    let Some(model) = ui.state.with(|s| {
        let doc = s.office.files.get(&id)?;
        if s.local_desktop != Some(local_desktop::Target::Native) || doc.saving.is_some() {
            return None;
        }
        doc.draft.clone()
    }) else {
        return;
    };
    ui.close_dialog();
    save_local(ui, id, path, model, None);
}

fn save_local(ui: &Ui, id: u64, path: String, model: Arc<str>, baseline: Option<[u8; 32]>) {
    let serial = ui.send(Action::SaveOffice(path, model, baseline));
    ui.state.revise(|s| {
        let doc = s.office.files.get_mut(&id).unwrap();
        doc.saving = Some(serial);
        doc.error = "Saving DOCX…".into();
    });
}

pub(crate) fn save_copy(
    target: Option<&local_desktop::Target>,
    path: &str,
    model: &str,
    baseline: Option<[u8; 32]>,
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
    if baseline.is_none() {
        ensure!(!dest.try_exists()?, "Choose a new DOCX file name.");
    }
    let bytes = encode(model)?;
    let document = render(path, &bytes).map_err(anyhow::Error::msg)?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(dest.parent().context("Missing destination directory")?)?;
    tmp.write_all(&bytes)?;
    if let Some(expected) = baseline {
        let metadata = std::fs::symlink_metadata(&dest)?;
        ensure!(
            metadata.is_file() && !metadata.permissions().readonly(),
            "The document is read-only. Save a copy instead."
        );
        let original = local_desktop::Target::Native.read_bounded_file(path, LIMIT)?;
        ensure!(
            <[u8; 32]>::from(<sha2::Sha256 as sha2::Digest>::digest(&original)) == expected,
            "This file changed since it was opened. Save a copy instead."
        );
        #[cfg(unix)]
        tmp.as_file().set_permissions(metadata.permissions())?;
        tmp.as_file().sync_all()?;
        #[cfg(windows)]
        local_desktop::windows_host::replace_file(&dest, &tmp.into_temp_path())?;
        #[cfg(not(windows))]
        tmp.persist(dest)?;
    } else {
        tmp.as_file().sync_all()?;
        tmp.persist_noclobber(dest)?;
    }
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
        let Some(doc) = s
            .office
            .files
            .values_mut()
            .find(|d| d.saving == Some(serial))
        else {
            return false;
        };
        doc.saving = None;
        match result {
            Ok((path, document)) => {
                let notice = if doc.path == path {
                    "Document saved."
                } else {
                    "Saved DOCX copy."
                };
                doc.path = path;
                doc.set_document(document, notice, s.local_desktop.as_ref());
                true
            }
            Err(error) => {
                doc.error = error;
                false
            }
        }
    });
    if saved {
        ui.refresh(true);
    }
}

impl Reader {
    fn set_document(
        &mut self,
        mut document: Document,
        local_notice: &str,
        target: Option<&local_desktop::Target>,
    ) {
        self.draft = match target {
            Some(local_desktop::Target::Native) => document.writer.clone(),
            _ => None,
        };
        self.dirty = false;
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

pub(crate) fn edit_message(ui: &Ui, id: u64, web: &wry::WebView, message: &str) {
    if !ui.state.with(|s| {
        s.local_desktop == Some(local_desktop::Target::Native)
            || s.office
                .files
                .get(&id)
                .is_some_and(|d| d.remote().is_some())
    }) {
        return;
    }
    match message {
        "dirty" => ui.state.revise(|s| {
            if let Some(doc) = s.office.files.get_mut(&id) {
                doc.dirty = true;
            }
        }),
        "confirm_save" => {
            if ui.state.with(|s| {
                s.dialog.is_none()
                    && s.local_desktop == Some(local_desktop::Target::Native)
                    && s.office
                        .files
                        .get(&id)
                        .is_some_and(|doc| doc.draft.is_some() && !doc.is_saving())
            }) {
                ui.open_dialog(Dialog::ConfirmOfficeSave(id));
            }
        }
        "save" => save_dialog(ui, id),
        "discard" => ui.state.revise(|s| {
            let Some(doc) = s.office.files.get_mut(&id) else {
                return;
            };
            doc.draft = None;
            doc.dirty = false;
            doc.error.clear();
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
                let Some(previous) = ui.state.with(|s| s.office.files.get(&id)?.draft.clone())
                else {
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
                let Some(doc) = s.office.files.get_mut(&id) else {
                    return;
                };
                doc.draft = Some(model);
                doc.dirty = true;
                doc.error.clear();
            });
        }
    }
}
