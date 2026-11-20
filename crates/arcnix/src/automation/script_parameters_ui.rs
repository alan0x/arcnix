//! Typed form definitions, literal run values and explicitly saved local presets.
use arcnix_core::operations::Snippet;
use arcnix_core::script_parameters::{self as params, Kind, Parameter};

use crate::*;

#[derive(Clone, Default)]
pub(crate) struct EditorState {
    bounds: std::collections::BTreeMap<(String, String, bool), String>,
}
#[derive(Clone)]
pub(crate) enum Owner {
    Command,
    Sharing(String),
}
impl Owner {
    fn key(&self) -> String {
        match self {
            Self::Command => "command".into(),
            Self::Sharing(id) => format!("sharing:{id}"),
        }
    }
    fn snippet(&self, s: &State) -> Option<Snippet> {
        match self {
            Self::Command => s.operations.editor.clone(),
            Self::Sharing(id) => crate::script_exchange_ui::parameter_draft(s, id),
        }
    }
    fn change(&self, s: &mut State, f: impl FnOnce(&mut Vec<Parameter>)) {
        match self {
            Self::Command => {
                if let Some(v) = &mut s.operations.editor {
                    f(&mut v.parameters);
                }
            }
            Self::Sharing(id) => crate::script_exchange_ui::change_parameters(s, id, f),
        }
    }
}
pub(crate) fn check_editor(s: &State, owner: &Owner) -> anyhow::Result<()> {
    for ((key, ..), value) in &s.scripts.parameter_editor.bounds {
        if key == &owner.key() && !value.is_empty() {
            value
                .parse::<i64>()
                .map_err(|_| anyhow::anyhow!("Enter a decimal integer or leave the bound empty"))?;
        }
    }
    Ok(())
}
#[derive(Clone, Copy)]
enum Field {
    Label,
    Description,
    Default,
    Choices,
    Minimum,
    Maximum,
}
impl Field {
    fn label(self) -> &'static str {
        match self {
            Self::Label => "Parameter label",
            Self::Description => "Parameter help",
            Self::Default => "Default value",
            Self::Choices => "Options, one per line",
            Self::Minimum => "Minimum integer",
            Self::Maximum => "Maximum integer",
        }
    }
    fn value(self, p: &Parameter) -> String {
        match self {
            Self::Label => p.label.clone(),
            Self::Description => p.description.clone(),
            Self::Default => p.default_value.clone(),
            Self::Choices => p.choices.join("\n"),
            Self::Minimum => p.minimum.map(|v| v.to_string()).unwrap_or_default(),
            Self::Maximum => p.maximum.map(|v| v.to_string()).unwrap_or_default(),
        }
    }
}
fn edit_field(ui: Ui, owner: Owner, key: String, field_kind: Field) -> impl Widget {
    let state = ui.state;
    let read_owner = owner.clone();
    let read_key = key.clone();
    let title = format!("{}: {key}", i18n::translate(field_kind.label()));
    col().class("field").fill((
        text().class("field-label").content(title.clone()),
        input()
            .name(title)
            .multiline(matches!(field_kind, Field::Choices | Field::Description))
            .value(state.map(move |s| {
                if matches!(field_kind, Field::Minimum | Field::Maximum)
                    && let Some(v) = s.scripts.parameter_editor.bounds.get(&(
                        read_owner.key(),
                        read_key.clone(),
                        matches!(field_kind, Field::Maximum),
                    ))
                {
                    return v.clone();
                }
                read_owner
                    .snippet(s)
                    .and_then(|v| v.parameters.into_iter().find(|p| p.name == read_key))
                    .map(|p| field_kind.value(&p))
                    .unwrap_or_default()
            }))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        if matches!(field_kind, Field::Minimum | Field::Maximum) {
                            s.scripts.parameter_editor.bounds.insert(
                                (
                                    owner.key(),
                                    key.clone(),
                                    matches!(field_kind, Field::Maximum),
                                ),
                                value.clone(),
                            );
                        }
                        owner.change(s, |list| {
                            if let Some(p) = list.iter_mut().find(|p| p.name == key) {
                                match field_kind {
                                    Field::Label => p.label = value.clone(),
                                    Field::Description => p.description = value.clone(),
                                    Field::Default => p.default_value = value.clone(),
                                    Field::Choices => {
                                        p.choices = value
                                            .replace("\r\n", "\n")
                                            .split('\n')
                                            .map(str::to_owned)
                                            .collect()
                                    }
                                    Field::Minimum => {
                                        if value.is_empty() {
                                            p.minimum = None;
                                        } else if let Ok(n) = value.parse() {
                                            p.minimum = Some(n);
                                        }
                                    }
                                    Field::Maximum => {
                                        if value.is_empty() {
                                            p.maximum = None;
                                        } else if let Ok(n) = value.parse() {
                                            p.maximum = Some(n);
                                        }
                                    }
                                }
                            }
                        });
                        s.dialog_error.clear();
                    });
                }
            }),
    ))
}
pub(crate) fn editor(ui: Ui, owner: Owner) -> impl Widget {
    let state = ui.state;
    let sync_owner = owner.clone();
    let list_owner = owner.clone();
    col().class("parameter-definition-editor form-stack").fill((
        row().class("script-actions").fill((crate::help_ui::group("Configure parameters",&["Configure unquoted {{name}} arguments after editing the command. Defaults are saved and shared with the script; use server credential-file paths instead of secrets.","An optional empty value is passed as one empty argument. Changing the script or its parameter rules makes older presets stale; values that are no longer permitted are removed."]),
        btn("Configure parameters",move||state.revise(|s|{
            let result=sync_owner.snippet(s).ok_or_else(||anyhow::anyhow!("Choose a script")).and_then(|v|Ok((arcnix_core::scripts::parameters(&v.command)?,v.parameters)));
            match result{Ok((names,old))=>{sync_owner.change(s,|defs|*defs=names.into_iter().map(|name|old.iter().find(|p|p.name==name).cloned().unwrap_or_else(||Parameter::text(name))).collect());s.scripts.parameter_editor.bounds.retain(|(key,_,_),_|key!=&sync_owner.key());s.dialog_error.clear();},Err(e)=>s.dialog_error=e.to_string()}
        })))),
        col().class("parameter-definition-list").fill(each(state.map(move|s|list_owner.snippet(s).map(|v|v.parameters.iter().map(|p|p.name.clone()).collect::<Vec<_>>()).unwrap_or_default())).key(|v|v.clone()).children(move|item|{
            let key=item.value.get();let types_owner=owner.clone();let types_key=key.clone();let required_owner=owner.clone();let required_key=key.clone();let required_read=owner.clone();let required_read_key=key.clone();let allow_owner=owner.clone();let allow_key=key.clone();let allow_read=owner.clone();let allow_read_key=key.clone();let integer_owner=owner.clone();let integer_key=key.clone();let choice_owner=owner.clone();let choice_key=key.clone();let integer_ui=ui.clone();let integer_edit_owner=owner.clone();let integer_edit_key=key.clone();let choice_ui=ui.clone();let choice_edit_owner=owner.clone();let choice_edit_key=key.clone();
            col().class("parameter-definition form-stack").fill((
                text().class("section-title").content(key.clone()),
                row().class("script-actions").fill(each(vec![Kind::Text,Kind::Integer,Kind::Port,Kind::Path,Kind::Choice,Kind::Boolean]).key(|k|k.label()).children(move|item|{
                    let kind=item.value.get();let edit=types_owner.clone();let read=types_owner.clone();let edit_key=types_key.clone();let read_key=types_key.clone();
                    btn(kind.label(),move||state.revise(|s|{edit.change(s,|list|{if let Some(p)=list.iter_mut().find(|p|p.name==edit_key){if p.kind==kind{return;}p.kind=kind;p.minimum=None;p.maximum=None;p.choices=if kind==Kind::Choice{vec!["option".into()]}else{vec![]};p.default_value.clear();}});s.scripts.parameter_editor.bounds.retain(|(o,k,_),_|o!=&edit.key()||k!=&edit_key);})).class(state.map(move|s|if read.snippet(s).is_some_and(|v|v.parameters.iter().any(|p|p.name==read_key&&p.kind==kind)){"choice active"}else{"choice"}.to_owned()))
                })),
                edit_field(ui.clone(),owner.clone(),key.clone(),Field::Label),edit_field(ui.clone(),owner.clone(),key.clone(),Field::Description),
                row().class("script-actions").fill((
                    btn("Required argument",move||state.revise(|s|required_owner.change(s,|list|{if let Some(p)=list.iter_mut().find(|p|p.name==required_key){p.required = !p.required;}}))).class(state.map(move|s|if required_read.snippet(s).is_some_and(|v|v.parameters.iter().any(|p|p.name==required_read_key&&p.required)){"choice active"}else{"choice"}.to_owned())),
                    btn("Allow in presets",move||state.revise(|s|allow_owner.change(s,|list|{if let Some(p)=list.iter_mut().find(|p|p.name==allow_key){p.allow_preset = !p.allow_preset;}}))).class(state.map(move|s|if allow_read.snippet(s).is_some_and(|v|v.parameters.iter().any(|p|p.name==allow_read_key&&p.allow_preset)){"choice active"}else{"choice"}.to_owned())),
                )),
                when(state.map(move|s|integer_owner.snippet(s).is_some_and(|v|v.parameters.iter().any(|p|p.name==integer_key&&p.kind==Kind::Integer))),move||col().class("form-stack").fill((edit_field(integer_ui.clone(),integer_edit_owner.clone(),integer_edit_key.clone(),Field::Minimum),edit_field(integer_ui.clone(),integer_edit_owner.clone(),integer_edit_key.clone(),Field::Maximum)))),
                when(state.map(move|s|choice_owner.snippet(s).is_some_and(|v|v.parameters.iter().any(|p|p.name==choice_key&&p.kind==Kind::Choice))),move||edit_field(choice_ui.clone(),choice_edit_owner.clone(),choice_edit_key.clone(),Field::Choices)),
                edit_field(ui.clone(),owner.clone(),key,Field::Default),
            ))
        })),
    ))
}
fn current(s: &State) -> Option<Snippet> {
    let template = s.scripts.template.as_ref()?;
    let snippet = params::lookup(&s.config, template.source(), &template.snippet.id)?;
    (params::digest(&snippet) == params::digest(&template.snippet)).then_some(snippet)
}
fn eligible(s: &State) -> bool {
    current(s)
        .and_then(|v| params::definitions(&v).ok())
        .is_some_and(|list| list.iter().any(|p| p.allow_preset))
}
impl Ui {
    fn save_parameter_preset(&self, update: bool) {
        let result = (|| -> anyhow::Result<_> {
            anyhow::ensure!(self.writable.get(), "Configuration is read-only");
            let mut config = self.state.with(|s| -> anyhow::Result<_> {
                let snippet = current(s)
                    .ok_or_else(|| anyhow::anyhow!("The script definition changed; reopen it"))?;
                let template = s.scripts.template.as_ref().unwrap();
                let id = if update {
                    Some(s.scripts.preset_selected.as_str())
                } else {
                    None
                };
                if let Some(id) = id {
                    anyhow::ensure!(
                        s.config.script_presets.iter().any(|p| p.id == id
                            && p.source == template.source()
                            && p.script == snippet.id),
                        "Select a preset to update"
                    );
                }
                let preset = params::capture(
                    template.source(),
                    &snippet,
                    &s.scripts.preset_name,
                    &s.scripts.values,
                    id,
                )?;
                let mut config = s.config.clone();
                config.script_presets.retain(|p| p.id != preset.id);
                config.script_presets.push(preset);
                Ok(config)
            })?;
            ConfigStore::check(&config)?;
            self.store.save(&config)?;
            Ok(std::mem::take(&mut config))
        })();
        self.state.revise(|s| match result {
            Ok(config) => {
                s.config = config;
                s.dialog_error.clear();
            }
            Err(e) => s.dialog_error = format!("{e:#}"),
        });
    }
    fn load_parameter_preset(&self) {
        self.state.revise(|s| {
            let result = (|| -> anyhow::Result<_> {
                let snippet = current(s)
                    .ok_or_else(|| anyhow::anyhow!("The script definition changed; reopen it"))?;
                let template = s.scripts.template.as_ref().unwrap();
                let p = s
                    .config
                    .script_presets
                    .iter()
                    .find(|p| {
                        p.id == s.scripts.preset_selected
                            && p.source == template.source()
                            && p.script == snippet.id
                    })
                    .ok_or_else(|| anyhow::anyhow!("Select a preset"))?;
                p.load(&snippet)
            })();
            match result {
                Ok(values) => {
                    s.scripts.values = values;
                    s.dialog_error.clear();
                }
                Err(e) => s.dialog_error = e.to_string(),
            }
        });
    }
    fn delete_parameter_preset(&self) {
        let mut config = self.state.with(|s| s.config.clone());
        let selected = self.state.with(|s| s.scripts.preset_selected.clone());
        let owned = self.state.with(|s| {
            s.scripts.template.as_ref().is_some_and(|t| {
                config
                    .script_presets
                    .iter()
                    .any(|p| p.id == selected && p.source == t.source() && p.script == t.snippet.id)
            })
        });
        if !self.writable.get() || !owned {
            return;
        }
        config.script_presets.retain(|p| p.id != selected);
        match self.store.save(&config) {
            Ok(()) => self.state.revise(|s| {
                s.config = config;
                s.scripts.preset_selected.clear();
                s.scripts.preset_name.clear();
                s.dialog_error.clear();
            }),
            Err(e) => self.state.revise(|s| s.dialog_error = e.to_string()),
        }
    }
}
fn presets(ui: Ui) -> impl Widget {
    let state = ui.state;
    let save = ui.clone();
    let update = ui.clone();
    let load = ui.clone();
    let delete = ui.clone();
    col().class("parameter-presets form-stack").fill((
        row().class("help-heading").fill((text().class("section-title").content(i18n::bind("Local parameter presets")),crate::help_ui::tip("Local parameter presets","Only fields marked Allow in presets are saved. Loading resets every other field to its default. Preset values stay on this device and are excluded from script bundles."))),
        row().class("script-actions").fill(each(state.map(|s|{
            let Some(t)=&s.scripts.template else{return vec![];};s.config.script_presets.iter().filter(|p|p.source==t.source()&&p.script==t.snippet.id).cloned().collect::<Vec<_>>()
        })).key(|p|p.id.clone()).children(move|item|{let p=item.value.get();let id=p.id.clone();let read=p.id.clone();let label=state.map(move|s|format!("{}{}",p.name,if current(s).is_some_and(|v|p.compatible(&v)){String::new()}else{format!(" · {}",i18n::translate("Stale preset"))}));
            sapid::button().label(label).on_press(move||state.revise(|s|{s.scripts.preset_selected=id.clone();if let Some(p)=s.config.script_presets.iter().find(|p|p.id==id){s.scripts.preset_name=p.name.clone();}s.dialog_error.clear();})).class(state.map(move|s|if s.scripts.preset_selected==read{"choice active"}else{"choice"}.to_owned()))
        })),
        field("Preset name",&ui,|s|s.scripts.preset_name.clone(),|s,v|s.scripts.preset_name=v),
        row().class("script-actions").fill((btn("Save new preset",move||save.save_parameter_preset(false)),btn("Update selected preset",move||update.save_parameter_preset(true)).enabled(state.map(|s|!s.scripts.preset_selected.is_empty())),btn("Load selected preset",move||load.load_parameter_preset()).enabled(state.map(|s|current(s).is_some_and(|v|s.config.script_presets.iter().any(|p|p.id==s.scripts.preset_selected&&p.compatible(&v))))),btn("Delete selected preset",move||delete.delete_parameter_preset()).enabled(state.map(|s|!s.scripts.preset_selected.is_empty())))),
    ))
}
pub(crate) fn run_form(ui: Ui) -> impl Widget {
    let state = ui.state;
    let preset_ui = ui.clone();
    col().class("parameter-run-form form-stack").fill((
        each(state.map(|s| {
            s.scripts
                .template
                .as_ref()
                .filter(|t| !t.parameters().unwrap_or_default().is_empty())
                .and_then(|t| params::definitions(&t.snippet).ok())
                .unwrap_or_default()
        }))
        .key(|p| p.name.clone())
        .children(move |item| {
            let p = item.value.get();
            let key = p.name.clone();
            let read = key.clone();
            let write = key.clone();
            let select_key = key.clone();
            let error_key = key.clone();
            let rule = p.clone();
            let name = p.name.clone();
            let optional = !p.required;
            let choices = if p.kind == Kind::Boolean {
                vec!["true".into(), "false".into()]
            } else {
                p.choices.clone()
            };
            col().class("field parameter-value").fill((
                row().class("help-heading").fill((
                    text().class("field-label").content(format!(
                        "{} · {}{}",
                        p.display_label(),
                        i18n::translate(p.kind.label()),
                        if p.required { " *" } else { "" }
                    )),
                    when(!p.description.is_empty(), move || {
                        crate::help_ui::tip_raw("Parameter help", p.description.clone())
                    }),
                )),
                when(matches!(p.kind, Kind::Choice | Kind::Boolean), move || {
                    row().class("script-actions").fill(
                        each(choices.clone()).key(|v| v.clone()).children({
                            let key = select_key.clone();
                            move |item| {
                                let value = item.value.get();
                                let edit = value.clone();
                                let read = value.clone();
                                let edit_key = key.clone();
                                let read_key = key.clone();
                                btn(format!("{key}: {value}"), move || {
                                    state.revise(|s| {
                                        s.scripts.values.insert(edit_key.clone(), edit.clone());
                                    })
                                })
                                .class(state.map(move |s| {
                                    if s.scripts.values.get(&read_key) == Some(&read) {
                                        "choice active"
                                    } else {
                                        "choice"
                                    }
                                    .to_owned()
                                }))
                            }
                        }),
                    )
                })
                .otherwise(move || {
                    input()
                        .name(name.clone())
                        .value(state.map({
                            let read = read.clone();
                            move |s| s.scripts.values.get(&read).cloned().unwrap_or_default()
                        }))
                        .on(value_changed, {
                            let write = write.clone();
                            move |e| {
                                if let Event::ValueChanged { value, .. } = e {
                                    state.revise(|s| {
                                        s.scripts.values.insert(write.clone(), value);
                                    });
                                }
                            }
                        })
                }),
                when(optional, move || {
                    btn("Use empty argument", {
                        let key = key.clone();
                        move || {
                            state.revise(|s| {
                                s.scripts.values.insert(key.clone(), String::new());
                            })
                        }
                    })
                }),
                text()
                    .class("parameter-value-error hint")
                    .content(state.map(move |s| {
                        rule.validate_value(
                            s.scripts
                                .values
                                .get(&error_key)
                                .map(String::as_str)
                                .unwrap_or_default(),
                        )
                        .err()
                        .map(|e| i18n::translate(&e.to_string()))
                        .unwrap_or_default()
                    })),
            ))
        }),
        when(state.map(|s| !s.scripts.values.is_empty()), move || {
            btn("Reset parameter defaults", move || {
                state.revise(|s| {
                    if let Some(t) = &s.scripts.template
                        && let Ok(values) = params::defaults(&t.snippet)
                    {
                        s.scripts.values = values;
                    }
                    s.dialog_error.clear();
                })
            })
        }),
        when(state.map(eligible), move || presets(preset_ui.clone())),
    ))
}
