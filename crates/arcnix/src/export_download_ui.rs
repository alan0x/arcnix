//! Private, one-off export downloads, separate from server task execution.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arcnix_core::export_download as exports;

use super::*;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Reading,
    Ready,
    Downloading,
    Complete,
    Failed,
}
#[derive(Clone, Default)]
pub(super) struct DownloadState {
    serial: u64,
    server: String,
    id: String,
    source: String,
    path: String,
    snapshot: Option<Box<exports::Snapshot>>,
    phase: Phase,
    bytes: u64,
    error: String,
    cancel: Option<Arc<AtomicBool>>,
}
pub(super) fn forget(s: &mut State) {
    if let Some(cancel) = s.cleanup_reports.download.cancel.take() {
        cancel.store(true, Ordering::Release);
    }
    s.cleanup_reports.download = Default::default();
    if s.dialog == Some(Dialog::ExportDownload) {
        s.dialog = None;
    }
}
pub(super) fn dismiss(s: &mut State) {
    if s.dialog == Some(Dialog::ExportDownload) {
        forget(s);
    }
}
pub(super) fn open(ui: &Ui, server: &str, id: &str) {
    let result = ui.state.with(|s| -> anyhow::Result<_> {
        let group = s
            .cleanup_reports
            .groups
            .iter()
            .find(|g| g.profile.id == server)
            .ok_or_else(|| anyhow::anyhow!("Reconnect to cleanup reports"))?;
        anyhow::ensure!(group.pending.is_none(), "Wait for the report query");
        let source = if s.cleanup_reports.storage.visible
            && s.cleanup_reports.storage.scope == arcnix_core::audit_storage::Scope::Recovery
        {
            group
                .storage
                .page
                .as_ref()
                .and_then(|p| {
                    p.records.iter().find(|r| {
                        r.id == id
                            && r.action.as_deref() == Some("export")
                            && r.phase.as_deref() == Some("completed")
                            && matches!(r.service.as_str(), "inactive" | "not-found")
                    })
                })
                .and_then(|r| r.source.clone())
        } else {
            group
                .detail
                .as_ref()
                .and_then(|d| d.recoveries.iter().find(|r| r.id == id && r.releasable()))
                .map(|r| r.source.clone())
        }
        .ok_or_else(|| anyhow::anyhow!("Inspect a completed, stopped export first"))?;
        Ok((target(s, server)?, source))
    });
    match result {
        Err(error) => ui
            .state
            .revise(|s| s.cleanup_reports.error = error.to_string()),
        Ok((target, source)) => {
            let serial = ui.state.revise(|s| {
                forget(s);
                s.cleanup_reports.serial += 1;
                let serial = s.cleanup_reports.serial;
                let folder = s
                    .config
                    .local_directory
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(&s.local_path));
                s.cleanup_reports.download = DownloadState {
                    serial,
                    server: server.into(),
                    id: id.into(),
                    source,
                    path: local_display(&folder.join(format!("arcnix-export-{id}.zip"))),
                    ..Default::default()
                };
                serial
            });
            ui.open_dialog(Dialog::ExportDownload);
            batch_worker::inspect_export(serial, target, id.into(), ui.batch_events.clone());
        }
    }
}
fn refresh(ui: &Ui) {
    let candidate = ui.state.with(|s| {
        let d = &s.cleanup_reports.download;
        (d.phase != Phase::Downloading).then(|| target(s, &d.server).map(|t| (t, d.id.clone())))
    });
    if let Some(Ok((target, id))) = candidate {
        let serial = ui.state.revise(|s| {
            s.cleanup_reports.serial += 1;
            let serial = s.cleanup_reports.serial;
            let d = &mut s.cleanup_reports.download;
            d.serial = serial;
            d.phase = Phase::Reading;
            d.error.clear();
            serial
        });
        batch_worker::inspect_export(serial, target, id, ui.batch_events.clone());
    }
}
fn start(ui: &Ui) {
    let candidate = ui.state.with(|s| -> anyhow::Result<_> {
        let d = &s.cleanup_reports.download;
        anyhow::ensure!(
            d.phase == Phase::Ready,
            "Refresh the export inventory before downloading"
        );
        let snapshot = d
            .snapshot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Refresh the export inventory before downloading"))?;
        Ok((
            target(s, &d.server)?,
            snapshot.request.clone(),
            PathBuf::from(&d.path),
        ))
    });
    match candidate {
        Err(error) => ui
            .state
            .revise(|s| s.cleanup_reports.download.error = error.to_string()),
        Ok((target, request, path)) => {
            let cancel = Arc::new(AtomicBool::new(false));
            let serial = ui.state.revise(|s| {
                s.cleanup_reports.serial += 1;
                let serial = s.cleanup_reports.serial;
                let d = &mut s.cleanup_reports.download;
                d.serial = serial;
                d.phase = Phase::Downloading;
                d.bytes = 0;
                d.error.clear();
                d.cancel = Some(cancel.clone());
                serial
            });
            batch_worker::download_export(
                serial,
                target,
                request,
                path,
                cancel,
                ui.batch_events.clone(),
            );
        }
    }
}
pub(super) fn event(ui: &Ui, serial: u64, server: &str, update: batch_worker::DownloadUpdate) {
    ui.state.revise(|s| {
        if s.dialog != Some(Dialog::ExportDownload)
            || s.cleanup_reports.download.serial != serial
            || s.cleanup_reports.download.server != server
            || target(s, server).is_err()
        {
            return;
        }
        let d = &mut s.cleanup_reports.download;
        match update {
            batch_worker::DownloadUpdate::Inspected(result) => {
                if d.phase != Phase::Reading {
                    return;
                }
                match result {
                    Ok(snapshot)
                        if snapshot.request.id == d.id && snapshot.request.source == d.source =>
                    {
                        d.snapshot = Some(snapshot);
                        d.phase = Phase::Ready;
                        d.error.clear();
                    }
                    Ok(_) => {
                        d.phase = Phase::Failed;
                        d.error = "Export inventory does not match the requested record".into();
                    }
                    Err(error) => {
                        d.phase = Phase::Failed;
                        d.error = error;
                    }
                }
            }
            batch_worker::DownloadUpdate::Progress(bytes) => {
                if d.phase == Phase::Downloading {
                    d.bytes = d.bytes.max(bytes);
                }
            }
            batch_worker::DownloadUpdate::Finished(result) => {
                if d.phase != Phase::Downloading {
                    return;
                }
                d.cancel = None;
                match result {
                    Ok((path, bytes)) => {
                        d.path = local_display(&path);
                        d.bytes = bytes;
                        d.phase = Phase::Complete;
                        d.error.clear();
                    }
                    Err(error) => {
                        d.phase = Phase::Failed;
                        d.error = error;
                    }
                }
            }
        }
    });
}
pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::ExportDownload)),
        move || {
            let refresh_ui = ui.clone();
            let run = ui.clone();
            let cancel = ui.clone();
            let browse = ui.clone();
            modal(ui.clone(),"Download residual export",col().class("export-download-form form-stack").fill((
            text().class("export-download-source subtle").content(state.map(|s|{let d=&s.cleanup_reports.download;format!("{} · {}",s.config.servers.iter().find(|p|p.id==d.server).map(|p|p.name.as_str()).unwrap_or(&d.server),d.id)})),
            row().class("script-actions").fill((text().class("export-download-state").content(i18n::bind(state.map(|s|match s.cleanup_reports.download.phase{Phase::Reading=>"Reading export inventory…",Phase::Ready=>"Export ready to download",Phase::Downloading=>"Downloading export…",Phase::Complete=>"Export download verified",Phase::Failed=>"Export download failed"}.to_owned()))),crate::help_ui::tip("Export download","The ZIP includes exported data, original metadata sidecars and audit evidence. Checksums and the stopped export are checked again during transfer. The server copy remains intact. A final file is published only after verification; existing files are never overwritten. Closing this dialog or leaving the page cancels the local transfer. Interrupted downloads can be started again; forced client closure may leave a .partial file."))),
            text().class("export-download-size subtle").content(state.map(|s|{let d=&s.cleanup_reports.download;if d.phase==Phase::Downloading||d.phase==Phase::Complete{formatting::bytes(d.bytes)}else{d.snapshot.as_ref().map(|p|i18n::translate(&format!("{} data files · {}",p.request.file_count,formatting::bytes(p.request.logical_bytes)))).unwrap_or_default()}})),
            col().class("field").fill((text().class("field-label").content(i18n::bind("Local ZIP path")),input().name(i18n::bind("Local ZIP path")).value(state.map(|s|s.cleanup_reports.download.path.clone())).enabled(state.map(|s|s.cleanup_reports.download.phase!=Phase::Downloading&&s.cleanup_reports.download.phase!=Phase::Complete)).on(value_changed,move|event|{if let Event::ValueChanged{value,..}=event{state.revise(|s|s.cleanup_reports.download.path=value);}}))),
            row().class("script-actions").fill((btn("Browse…",move||{let current=state.with(|s|PathBuf::from(&s.cleanup_reports.download.path));match file_picker::folder(current.parent(),&i18n::translate("Download directory")){Ok(Some(path))=>state.revise(|s|{let d=&mut s.cleanup_reports.download;d.path=local_display(&path.join(format!("arcnix-export-{}.zip",d.id)));}),Ok(None)=>{},Err(error)=>browse.state.revise(|s|s.cleanup_reports.download.error=error.to_string())}}).enabled(state.map(|s|!matches!(s.cleanup_reports.download.phase,Phase::Downloading|Phase::Complete))),btn("Refresh export inventory",move||refresh(&refresh_ui)).enabled(state.map(|s|!matches!(s.cleanup_reports.download.phase,Phase::Reading|Phase::Downloading))))),
            text().class("export-download-files script-code").selectable(true).content(state.map(|s|s.cleanup_reports.download.snapshot.as_ref().map(|p|p.files.iter().map(|f|format!("{} · {}",f.path,formatting::bytes(f.size))).collect::<Vec<_>>().join("\n")).unwrap_or_default())),
            text().class(state.map(|s|if s.cleanup_reports.download.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(state.map(|s|s.cleanup_reports.download.error.clone())),
        )),(sapid::button().class("button").label(i18n::bind(state.map(|s|if s.cleanup_reports.download.phase==Phase::Complete{"Close"}else{"Cancel"}.to_owned()))).on_press(move||cancel.close_dialog()),btn("Download ZIP",move||start(&run)).class("button primary").enabled(state.map(|s|s.cleanup_reports.download.phase==Phase::Ready))))
        },
    )
}
