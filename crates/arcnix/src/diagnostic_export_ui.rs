//! An immutable saved snapshot, explicit preview, and local no-replace save.
use arcnix_core::diagnostic_exports::{self as exports, Snapshot, Source};
use arcnix_core::tasks::Task;

use super::*;
#[derive(Clone, Default)]
pub(super) struct ExportState {
    snapshot: Option<Snapshot>,
    selected: HashSet<String>,
    details: bool,
    folder: String,
    name: String,
    document: Option<exports::Document>,
    prepared: Option<exports::ExportPlan>,
}
pub(super) fn dismiss(s: &mut State) {
    s.diagnostics.export = Default::default();
    if s.dialog == Some(Dialog::DiagnosticExport) {
        s.dialog = None;
    }
}
fn invalidate(s: &mut State) {
    s.diagnostics.export.document = None;
    s.diagnostics.export.prepared = None;
    s.dialog_error.clear();
}
impl Ui {
    pub(crate) fn open_diagnostic_report(&self, task: Task, source: Source) {
        let result = (|| -> anyhow::Result<_> {
            let captured = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            Snapshot::new(task, source, captured)
        })();
        match result {
            Err(e) => self.state.revise(|s| s.dialog_error = e.to_string()),
            Ok(snapshot) => {
                self.state.revise(|s| {
                    s.diagnostics.export = ExportState {
                        selected: snapshot
                            .task()
                            .targets
                            .iter()
                            .map(|t| t.id.clone())
                            .collect(),
                        name: format!("arcnix-diagnostics-{}.html", snapshot.task().id),
                        snapshot: Some(snapshot),
                        ..Default::default()
                    };
                });
                self.open_dialog(Dialog::DiagnosticExport);
            }
        }
    }
    pub(crate) fn export_diagnostic_batch(&self, id: &str) {
        if let Some(task) = self
            .state
            .with(|s| s.scripts.tasks.iter().find(|t| t.id == id).cloned())
        {
            self.open_diagnostic_report(task, Source::RecentBatch);
        }
    }
    fn browse_report_folder(&self) {
        let current = self.state.with(|s| s.diagnostics.export.folder.clone());
        match file_picker::folder(
            (!current.is_empty())
                .then(|| PathBuf::from(current))
                .as_deref(),
            &i18n::translate("Choose report folder"),
        ) {
            Ok(Some(path)) => self.state.revise(|s| {
                s.diagnostics.export.folder = local_display(&path);
                invalidate(s);
            }),
            Ok(None) => {}
            Err(e) => self.state.revise(|s| s.dialog_error = e.to_string()),
        }
    }
    fn preview_diagnostic_report(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let draft = &s.diagnostics.export;
            let snapshot = draft
                .snapshot
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Choose a diagnostic report first"))?;
            let language = s.config.settings.language;
            let document = snapshot.document(
                &draft.selected,
                draft.details,
                language == arcnix_core::Language::SimplifiedChinese,
                |label| i18n::translate_for(label, language),
                formatting::timestamp,
            )?;
            let prepared =
                exports::ExportPlan::new(Path::new(&draft.folder), &draft.name, &document)?;
            Ok((document, prepared))
        });
        self.state.revise(|s| {
            invalidate(s);
            match result {
                Ok((document, prepared)) => {
                    s.diagnostics.export.document = Some(document);
                    s.diagnostics.export.prepared = Some(prepared);
                }
                Err(e) => s.dialog_error = format!("{e:#}"),
            }
        });
    }
    fn save_diagnostic_report(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let prepared = s
                .diagnostics
                .export
                .prepared
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Review the diagnostic report first"))?;
            prepared.write()?;
            Ok(local_display(prepared.destination()))
        });
        match result {
            Ok(path) => {
                self.close_dialog();
                self.state
                    .revise(|s| s.notice = format!("Saved diagnostic report to {path}"));
            }
            Err(e) => self.state.revise(|s| s.dialog_error = format!("{e:#}")),
        }
    }
}
pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::DiagnosticExport)),
        move || {
            let close = ui.clone();
            let preview = ui.clone();
            let save = ui.clone();
            let browse = ui.clone();
            modal(ui.clone(),"Export diagnostic report",col().class("form-stack diagnostic-export-dialog").fill((
            row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Captured diagnostic snapshot")),crate::help_ui::group("Diagnostic report export",&["Export the captured saved observations without contacting servers. Later task updates do not change this snapshot. Unknown or incomplete results remain unconfirmed.","Server addresses, fingerprints and selected paths are included. Details are optional and may contain sensitive server data; review the report before sharing. Scripts and execution previews are excluded.","The HTML file works offline without scripts or external resources. Existing files are never overwritten."]))),
            text().class("subtle").content(state.map(|s|s.diagnostics.export.snapshot.as_ref().map(|v|format!("{} · {}",v.task().id,formatting::timestamp(Some(v.captured)))).unwrap_or_default())),
            col().class("diagnostic-export-targets").fill(each(state.map(|s|s.diagnostics.export.snapshot.as_ref().map(|v|v.task().targets.clone()).unwrap_or_default())).key(|t|t.id.clone()).children(move|item|{let target=item.value.get();let id=target.id;let active=id.clone();btn(format!("{} · {} · {}",target.name,target.endpoint,i18n::translate(target.status.label())),move||state.revise(|s|{if !s.diagnostics.export.selected.remove(&id){s.diagnostics.export.selected.insert(id.clone());}invalidate(s);})).class(state.map(move|s|if s.diagnostics.export.selected.contains(&active){"target-choice active diagnostic-export-target"}else{"target-choice diagnostic-export-target"}.to_owned()))})),
            btn("Include finding details",move||state.revise(|s|{s.diagnostics.export.details= !s.diagnostics.export.details;invalidate(s);})).class(state.map(|s|if s.diagnostics.export.details{"choice active diagnostic-export-details"}else{"choice diagnostic-export-details"}.to_owned())),
            col().class("field").fill((text().class("field-label").content(i18n::bind("Report folder")),row().class("file-picker-field").fill((input().name(i18n::bind("Report folder")).value(state.map(|s|s.diagnostics.export.folder.clone())).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|{s.diagnostics.export.folder=value;invalidate(s);});}),btn("Browse…",move||browse.browse_report_folder()).class("button browse-button"))))),
            col().class("field").fill((text().class("field-label").content(i18n::bind("HTML file name")),input().name(i18n::bind("HTML file name")).value(state.map(|s|s.diagnostics.export.name.clone())).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|{s.diagnostics.export.name=value;invalidate(s);});}))),
            text().class("subtle diagnostic-export-destination").selectable(true).content(state.map(|s|s.diagnostics.export.prepared.as_ref().map(|p|local_display(p.destination())).unwrap_or_default())),
            text().class(state.map(|s|if s.diagnostics.export.document.is_some(){"script-code diagnostic-export-preview"}else{"hidden"}.to_owned())).selectable(true).content(state.map(|s|s.diagnostics.export.document.as_ref().map(|d|d.preview.clone()).unwrap_or_default())),
        )),(btn("Cancel",move||close.close_dialog()),btn("Review report",move||preview.preview_diagnostic_report()).enabled(state.map(|s|!s.diagnostics.export.selected.is_empty())),btn("Save HTML report",move||save.save_diagnostic_report()).class("button primary").enabled(state.map(|s|s.diagnostics.export.prepared.is_some()))))
        },
    )
}
