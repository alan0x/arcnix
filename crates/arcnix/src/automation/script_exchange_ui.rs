//! Local script-only bundles with separate editing, review and commit steps.
use std::collections::BTreeSet;

use arcnix_core::script_bundles::{self as bundles, Bundle, Collision, ImportPlan, Script};

use crate::*;

#[derive(Clone)]
struct Draft {
    id: String,
    source_name: String,
    script: Script,
}
#[derive(Clone, Default)]
pub(crate) struct ExchangeState {
    path: String,
    loaded: Option<bundles::Loaded>,
    selected: BTreeSet<usize>,
    collision: Collision,
    import: Option<ImportPlan>,
    drafts: Vec<Draft>,
    export_selected: HashSet<String>,
    folder: String,
    file_name: String,
    export: Option<bundles::ExportPlan>,
}
pub(crate) fn forget(s: &mut State) {
    s.scripts.parameter_editor = Default::default();
    s.scripts.exchange = ExchangeState::default();
    if matches!(s.dialog, Some(Dialog::ScriptImport | Dialog::ScriptExport)) {
        s.dialog = None;
        s.dialog_error.clear();
    }
}
impl Ui {
    pub(crate) fn import_scripts(&self) {
        if !self.writable.get() {
            return;
        }
        self.state
            .revise(|s| s.scripts.exchange = ExchangeState::default());
        self.open_dialog(Dialog::ScriptImport);
    }
    pub(crate) fn export_scripts(&self) {
        self.state.revise(|s| {
            s.scripts.exchange = ExchangeState {
                drafts: s
                    .config
                    .snippets
                    .iter()
                    .map(|v| Draft {
                        id: v.id.clone(),
                        source_name: v.name.clone(),
                        script: Script::from(v),
                    })
                    .collect(),
                file_name: "arcnix-scripts.json".into(),
                ..Default::default()
            };
        });
        self.open_dialog(Dialog::ScriptExport);
    }
    fn read_script_bundle(&self) {
        let path = self.state.with(|s| s.scripts.exchange.path.clone());
        let result = bundles::read(Path::new(&path));
        self.state.revise(|s| {
            s.scripts.exchange.import = None;
            s.scripts.exchange.loaded = None;
            s.scripts.exchange.selected.clear();
            s.dialog_error.clear();
            match result {
                Ok(loaded) => {
                    s.scripts.exchange.selected = loaded
                        .bundle
                        .scripts
                        .iter()
                        .enumerate()
                        .filter(|(_, script)| !bundles::already_present(script, &s.config.snippets))
                        .map(|(i, _)| i)
                        .collect();
                    s.scripts.exchange.path = local_display(&loaded.path);
                    s.scripts.exchange.loaded = Some(loaded);
                }
                Err(error) => s.dialog_error = format!("{error:#}"),
            }
        });
    }
    fn review_script_import(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(self.writable.get(), "Configuration is read-only");
            let exchange = &s.scripts.exchange;
            let loaded = exchange
                .loaded
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Read a script bundle first"))?;
            let plan = bundles::plan(
                &loaded.bundle,
                &s.config.snippets,
                &exchange.selected,
                exchange.collision,
            )?;
            let mut config = s.config.clone();
            config.snippets = plan.apply(&config.snippets)?;
            arcnix_core::script_parameters::reconcile(&mut config);
            ConfigStore::check(&config)?;
            Ok(plan)
        });
        self.state.revise(|s| match result {
            Ok(plan) => {
                s.scripts.exchange.import = Some(plan);
                s.dialog_error.clear();
            }
            Err(error) => s.dialog_error = format!("{error:#}"),
        });
    }
    fn apply_script_import(&self) {
        let result = (|| -> anyhow::Result<_> {
            anyhow::ensure!(self.writable.get(), "Configuration is read-only");
            let (mut config, plan) = self
                .state
                .with(|s| (s.config.clone(), s.scripts.exchange.import.clone()));
            let plan = plan.ok_or_else(|| anyhow::anyhow!("Review script changes first"))?;
            config.snippets = plan.apply(&config.snippets)?;
            arcnix_core::script_parameters::reconcile(&mut config);
            self.store.save(&config)?;
            Ok((config, plan.changes.len()))
        })();
        match result {
            Ok((config, count)) => {
                self.state.revise(|s| {
                    s.config = config;
                    s.scripts.library = 1;
                    s.scripts.search.clear();
                    s.notice = format!("Imported {count} scripts into My scripts");
                });
                self.close_dialog();
            }
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
        }
    }
    fn review_script_export(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let e = &s.scripts.exchange;
            for id in &e.export_selected {
                crate::script_parameters_ui::check_editor(
                    s,
                    &crate::script_parameters_ui::Owner::Sharing(id.clone()),
                )?;
            }
            let scripts = e
                .drafts
                .iter()
                .filter(|d| e.export_selected.contains(&d.id))
                .map(|d| d.script.clone())
                .collect();
            bundles::prepare_export(Path::new(&e.folder), &e.file_name, &Bundle::new(scripts)?)
        });
        self.state.revise(|s| match result {
            Ok(plan) => {
                s.scripts.exchange.export = Some(plan);
                s.dialog_error.clear();
            }
            Err(error) => s.dialog_error = format!("{error:#}"),
        });
    }
    fn write_script_bundle(&self) {
        let plan = self.state.with(|s| s.scripts.exchange.export.clone());
        let result = plan
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Review the export first"))
            .and_then(|p| p.write());
        match result {
            Ok(()) => {
                let path = plan
                    .as_ref()
                    .map(|p| local_display(p.destination()))
                    .unwrap_or_default();
                self.state
                    .revise(|s| s.notice = format!("Exported script bundle to {path}"));
                self.close_dialog();
            }
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
        }
    }
    fn browse_script_bundle(&self) {
        let current = self.state.with(|s| s.scripts.exchange.path.clone());
        let initial = if current.is_empty() {
            std::env::var_os("USERPROFILE").map(PathBuf::from)
        } else {
            Some(PathBuf::from(current))
        };
        match file_picker::private_key(
            initial.as_deref(),
            &i18n::translate("Select script bundle"),
            &i18n::translate("Select file"),
            &i18n::translate("All files"),
        ) {
            Ok(Some(path)) => self.state.revise(|s| {
                s.scripts.exchange.path = local_display(&path);
                s.scripts.exchange.loaded = None;
                s.scripts.exchange.import = None;
                s.scripts.exchange.selected.clear();
                s.dialog_error.clear();
            }),
            Ok(None) => {}
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
        }
    }
    fn browse_script_folder(&self) {
        let current = self.state.with(|s| s.scripts.exchange.folder.clone());
        let initial = (!current.is_empty()).then(|| PathBuf::from(current));
        match file_picker::folder(
            initial.as_deref(),
            &i18n::translate("Choose script export folder"),
        ) {
            Ok(Some(path)) => self.state.revise(|s| {
                s.scripts.exchange.folder = local_display(&path);
                s.scripts.exchange.export = None;
                s.dialog_error.clear();
            }),
            Ok(None) => {}
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
        }
    }
}
pub(crate) fn toolbar(ui: Ui) -> impl Widget {
    let state = ui.state;
    let import = ui.clone();
    let export = ui.clone();
    fragment((
        btn("Import scripts…", move || import.import_scripts()).enabled(ui.writable),
        when(state.map(|s| s.scripts.library == 1), move || {
            let ui = export.clone();
            btn("Export scripts…", move || ui.export_scripts())
                .enabled(state.map(|s| !s.config.snippets.is_empty()))
        }),
    ))
}
fn script_text(script: &Script) -> String {
    let parameters = arcnix_core::scripts::parameters(&script.command).unwrap_or_default();
    let definitions = if script.parameters.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n{}\n{}",
            i18n::translate("Parameter definitions:"),
            serde_json::to_string_pretty(&script.parameters).unwrap_or_default()
        )
    };
    format!(
        "{} {}\n{} {}\n{} {}\nSHA-256: {}\n{}\n\n{}{}",
        i18n::translate("Command name"),
        script.name,
        i18n::translate("Execution account:"),
        i18n::translate(if script.administrator {
            "Administrator"
        } else {
            "SSH account"
        }),
        i18n::translate("Parameters:"),
        parameters.join(", "),
        script.digest().unwrap_or_default(),
        script.description,
        script.command,
        definitions
    )
}
pub(crate) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        when(state.map(|s| s.dialog == Some(Dialog::ScriptImport)), {
            let ui = ui.clone();
            move || import_dialog(ui.clone())
        }),
        when(
            state.map(|s| s.dialog == Some(Dialog::ScriptExport)),
            move || export_dialog(ui.clone()),
        ),
    ))
}
fn import_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let read = ui.clone();
    let browse = ui.clone();
    let review = ui.clone();
    let apply = ui.clone();
    let close = ui.clone();
    modal(ui.clone(),"Import script bundle",col().class("form-stack script-bundle-dialog").fill((
        text().class("hint").content(i18n::bind("Read and review script definitions before saving. Imported entries are user shell scripts; execution still requires the normal server review.")),
        text().class("hint").content(i18n::bind("Choose an Arcnix JSON script bundle, up to 4 MiB and 200 scripts.")),
        text().class("script-bundle-source subtle").content(state.map(|s|s.scripts.exchange.loaded.as_ref().map(|v|format!("{}\nSHA-256: {}",local_display(&v.path),v.digest)).unwrap_or_default())),
        when(state.map(|s|s.scripts.exchange.import.is_none()),{let ui=ui.clone();move||col().class("form-stack").fill((
            field("Script bundle path",&ui,|s|s.scripts.exchange.path.clone(),|s,v|{s.scripts.exchange.path=v;s.scripts.exchange.loaded=None;s.scripts.exchange.selected.clear();s.scripts.exchange.import=None;}),
            row().class("script-actions").fill((btn("Browse…",{let ui=browse.clone();move||ui.browse_script_bundle()}),btn("Read bundle",{let ui=read.clone();move||ui.read_script_bundle()}))),
            row().class("script-actions").fill(each(vec![(Collision::Copy,"Keep existing; add copies"),(Collision::Replace,"Replace scripts with matching names")]).key(|v|v.1).children(move|item|{
                let (mode,label)=item.value.get();btn(label,move||state.revise(|s|{s.scripts.exchange.collision=mode;s.scripts.exchange.import=None;s.dialog_error.clear();}))
                    .class(state.map(move|s|if s.scripts.exchange.collision==mode{"choice active"}else{"choice"}.to_owned()))
            })),
            row().class("script-actions").fill((
                btn("Select new scripts",move||state.revise(|s|s.scripts.exchange.selected=s.scripts.exchange.loaded.as_ref().map(|v|v.bundle.scripts.iter().enumerate().filter(|(_,v)|!bundles::already_present(v,&s.config.snippets)).map(|(i,_)|i).collect()).unwrap_or_default())),
                btn("Clear script selection",move||state.revise(|s|s.scripts.exchange.selected.clear())),
            )),
            text().class("script-bundle-selection subtle").content(state.map(|s|format!("{} {}",i18n::translate("Selected scripts:"),s.scripts.exchange.selected.len()))),
            col().class("script-bundle-list").fill(each(state.map(|s|s.scripts.exchange.loaded.as_ref().map(|v|v.bundle.scripts.iter().cloned().enumerate().collect::<Vec<_>>()).unwrap_or_default())).key(|v|v.0).children(move|item|{
                let (index,script)=item.value.get();
                col().class("script-bundle-entry").fill((
                    btn(script.name.clone(),move||state.revise(|s|{if !s.scripts.exchange.selected.insert(index){s.scripts.exchange.selected.remove(&index);}s.scripts.exchange.import=None;}))
                        .class(state.map(move|s|if s.scripts.exchange.selected.contains(&index){"choice script-bundle-choice active"}else{"choice script-bundle-choice"}.to_owned()))
                        .enabled(state.map(move|s|s.scripts.exchange.loaded.as_ref().and_then(|v|v.bundle.scripts.get(index)).is_some_and(|v|!bundles::already_present(v,&s.config.snippets)))),
                    text().class("subtle").content(state.map(move|s|if s.scripts.exchange.loaded.as_ref().and_then(|v|v.bundle.scripts.get(index)).is_some_and(|v|bundles::already_present(v,&s.config.snippets)){i18n::translate("Identical script already exists; skipped")}else{String::new()})),
                    text().class("script-code script-bundle-body").selectable(true).content(script_text(&script)),
                ))
            })),
        ))}),
        when(state.map(|s|s.scripts.exchange.import.is_some()),move||col().class("form-stack").fill((
            text().class("hint").content(i18n::bind("Apply saves exactly the reviewed definitions. Changes to the source file are not reread. Existing script changes require a new review.")),
            text().class("script-bundle-change-count subtle").content(state.map(|s|s.scripts.exchange.import.as_ref().map(|p|format!("{} {} · {} {}",i18n::translate("Changed scripts:"),p.changes.len(),i18n::translate("Skipped identical scripts:"),p.skipped)).unwrap_or_default())),
            text().class("script-code script-bundle-import-review").selectable(true).content(state.map(|s|{
                s.scripts.exchange.import.as_ref().map(|p|p.changes.iter().map(|c|{
                    let label=match c.kind{bundles::ChangeKind::New=>"New script",bundles::ChangeKind::Copy=>"New copy",bundles::ChangeKind::Replace=>"Replace existing script"};
                    format!("{} · {} → {}\n{}{}\n{}",i18n::translate(label),c.source_name,c.result.name,
                        c.before.as_ref().map(|v|format!("{}\n{}\n\n",i18n::translate("Existing definition:"),script_text(&Script::from(v)))).unwrap_or_default(),
                        i18n::translate("Reviewed definition:"),script_text(&Script::from(&c.result)))
                }).collect::<Vec<_>>().join("\n\n────────\n\n")).unwrap_or_default()
            })),
        ))),
    )),(
        btn("Cancel",move||close.close_dialog()),
        when(state.map(|s|s.scripts.exchange.import.is_some()),move||btn("Back to selection",move||state.revise(|s|{s.scripts.exchange.import=None;s.dialog_error.clear();}))),
        when(state.map(|s|s.scripts.exchange.import.is_none()),move||{let ui=review.clone();btn("Review import…",move||ui.review_script_import()).class("button primary").enabled(state.map(|s|s.scripts.exchange.loaded.is_some()&&!s.scripts.exchange.selected.is_empty()))}),
        when(state.map(|s|s.scripts.exchange.import.is_some()),move||{let ui=apply.clone();btn("Apply import",move||ui.apply_script_import()).class("button primary")}),
    ))
}
fn export_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let browse = ui.clone();
    let review = ui.clone();
    let write = ui.clone();
    let close = ui.clone();
    modal(ui.clone(),"Export script bundle",col().class("form-stack script-bundle-dialog").fill((
        text().class("hint").content(i18n::bind("Choose scripts and edit a sharing copy to remove embedded secrets. These edits affect the export only. The bundle contains script definitions and excludes server profiles and run-time parameter values.")),
        when(state.map(|s|s.scripts.exchange.export.is_none()),{let ui=ui.clone();move||col().class("form-stack").fill((
            field("Export folder",&ui,|s|s.scripts.exchange.folder.clone(),|s,v|s.scripts.exchange.folder=v),
            btn("Browse…",{let ui=browse.clone();move||ui.browse_script_folder()}),
            field("Bundle file name",&ui,|s|s.scripts.exchange.file_name.clone(),|s,v|s.scripts.exchange.file_name=v),
            text().class("hint").content(i18n::bind("A new file will be created. Existing files are not replaced; choose another name when needed.")),
            row().class("script-actions").fill((
                btn("Select all scripts",move||state.revise(|s|s.scripts.exchange.export_selected=s.scripts.exchange.drafts.iter().map(|d|d.id.clone()).collect())),
                btn("Clear script selection",move||state.revise(|s|s.scripts.exchange.export_selected.clear())),
            )),
            text().class("script-bundle-selection subtle").content(state.map(|s|format!("{} {}",i18n::translate("Selected scripts:"),s.scripts.exchange.export_selected.len()))),
            col().class("script-bundle-list").fill(each(state.map(|s|s.scripts.exchange.drafts.iter().map(|d|(d.id.clone(),d.source_name.clone())).collect::<Vec<_>>())).key(|v|v.0.clone()).children({let ui=ui.clone();move|item|{
                let (id,name)=item.value.get();let selected=id.clone();let edit_id=id.clone();
                col().class("script-share-item").fill((
                    btn(name,move||state.revise(|s|{if !s.scripts.exchange.export_selected.insert(id.clone()){s.scripts.exchange.export_selected.remove(&id);}}))
                        .class(state.map(move|s|if s.scripts.exchange.export_selected.contains(&selected){"choice script-share-choice active"}else{"choice script-share-choice"}.to_owned())),
                    when(state.map({let id=edit_id.clone();move|s|s.scripts.exchange.export_selected.contains(&id)}),{let ui=ui.clone();move||share_editor(ui.clone(),edit_id.clone())}),
                ))
            }})),
        ))}),
        when(state.map(|s|s.scripts.exchange.export.is_some()),move||text().class("script-code script-bundle-export-review").selectable(true).content(state.map(|s|{
            s.scripts.exchange.export.as_ref().map(|p|format!("{}\nSHA-256: {}\n\n{}",local_display(p.destination()),p.digest(),p.bundle().map(|b|b.scripts.iter().map(script_text).collect::<Vec<_>>().join("\n\n────────\n\n")).unwrap_or_default())).unwrap_or_default()
        }))),
    )),(
        btn("Cancel",move||close.close_dialog()),
        when(state.map(|s|s.scripts.exchange.export.is_some()),move||btn("Back to selection",move||state.revise(|s|{s.scripts.exchange.export=None;s.dialog_error.clear();}))),
        when(state.map(|s|s.scripts.exchange.export.is_none()),move||{let ui=review.clone();btn("Review export…",move||ui.review_script_export()).class("button primary").enabled(state.map(|s|!s.scripts.exchange.export_selected.is_empty()))}),
        when(state.map(|s|s.scripts.exchange.export.is_some()),move||{let ui=write.clone();btn("Write new bundle",move||ui.write_script_bundle()).class("button primary")}),
    ))
}
fn share_editor(ui: Ui, id: String) -> impl Widget {
    let state = ui.state;
    let parameter_id = id.clone();
    let name_read = id.clone();
    let name_write = id.clone();
    let body_read = id.clone();
    let body_write = id.clone();
    let description_read = id.clone();
    let description_write = id.clone();
    let admin_read = id.clone();
    col().class("script-share-editor form-stack").fill((
        text()
            .class("field-label")
            .content(i18n::bind("Shared script name")),
        input()
            .name(i18n::bind("Shared script name"))
            .value(state.map(move |s| {
                s.scripts
                    .exchange
                    .drafts
                    .iter()
                    .find(|d| d.id == name_read)
                    .map(|d| d.script.name.clone())
                    .unwrap_or_default()
            }))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        if let Some(d) = s
                            .scripts
                            .exchange
                            .drafts
                            .iter_mut()
                            .find(|d| d.id == name_write)
                        {
                            d.script.name = value.clone();
                        }
                    });
                }
            }),
        text()
            .class("field-label")
            .content(i18n::bind("Shared script description")),
        input()
            .name(i18n::bind("Shared script description"))
            .value(state.map(move |s| {
                s.scripts
                    .exchange
                    .drafts
                    .iter()
                    .find(|d| d.id == description_read)
                    .map(|d| d.script.description.clone())
                    .unwrap_or_default()
            }))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        if let Some(d) = s
                            .scripts
                            .exchange
                            .drafts
                            .iter_mut()
                            .find(|d| d.id == description_write)
                        {
                            d.script.description = value.clone();
                        }
                    });
                }
            }),
        text()
            .class("field-label")
            .content(i18n::bind("Shared script command")),
        input()
            .multiline(true)
            .class("command-editor")
            .name(i18n::bind("Shared script command"))
            .value(state.map(move |s| {
                s.scripts
                    .exchange
                    .drafts
                    .iter()
                    .find(|d| d.id == body_read)
                    .map(|d| d.script.command.clone())
                    .unwrap_or_default()
            }))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        if let Some(d) = s
                            .scripts
                            .exchange
                            .drafts
                            .iter_mut()
                            .find(|d| d.id == body_write)
                        {
                            d.script.command = value.replace("\r\n", "\n");
                        }
                    });
                }
            }),
        btn("Run as administrator", move || {
            state.revise(|s| {
                if let Some(d) = s.scripts.exchange.drafts.iter_mut().find(|d| d.id == id) {
                    d.script.administrator = !d.script.administrator;
                }
            })
        })
        .class(state.map(move |s| {
            if s.scripts
                .exchange
                .drafts
                .iter()
                .find(|d| d.id == admin_read)
                .is_some_and(|d| d.script.administrator)
            {
                "choice active"
            } else {
                "choice"
            }
            .to_owned()
        })),
        crate::script_parameters_ui::editor(
            ui.clone(),
            crate::script_parameters_ui::Owner::Sharing(parameter_id),
        ),
    ))
}

pub(crate) fn parameter_draft(s: &State, id: &str) -> Option<arcnix_core::operations::Snippet> {
    let v = &s
        .scripts
        .exchange
        .drafts
        .iter()
        .find(|d| d.id == id)?
        .script;
    Some(arcnix_core::operations::Snippet {
        id: id.into(),
        name: v.name.clone(),
        description: v.description.clone(),
        command: v.command.clone(),
        administrator: v.administrator,
        parameters: v.parameters.clone(),
    })
}
pub(crate) fn change_parameters(
    s: &mut State,
    id: &str,
    f: impl FnOnce(&mut Vec<arcnix_core::script_parameters::Parameter>),
) {
    if let Some(d) = s.scripts.exchange.drafts.iter_mut().find(|d| d.id == id) {
        f(&mut d.script.parameters);
    }
}
