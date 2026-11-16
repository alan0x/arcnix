//! Session-only defaults and explicit, scoped per-server overrides.
use arcnix_core::configuration_templates::{Overrides, Source, Values};

use super::*;

#[derive(Clone)]
pub(super) struct Session {
    pub source: Arc<Source>,
    pub defaults: Values,
    pub overrides: Overrides,
    pub expanded: HashSet<String>,
}
pub(super) fn sync(s: &mut State) {
    if s.configurations.parameterized
        && let Ok(source) = Source::parse(&s.configurations.content)
    {
        s.configurations.defaults = source.defaults(&s.configurations.defaults);
    }
}
pub(super) fn session(s: &State) -> anyhow::Result<Option<Session>> {
    if !s.configurations.parameterized {
        return Ok(None);
    }
    let source = Source::parse(&s.configurations.content)?;
    let defaults = source.defaults(&s.configurations.defaults);
    Ok(Some(Session {
        source: Arc::new(source),
        defaults,
        overrides: Default::default(),
        expanded: Default::default(),
    }))
}
fn names(s: &State, editor: bool) -> Vec<String> {
    if editor {
        Source::parse(&s.configurations.content)
            .map(|p| p.names().to_vec())
            .unwrap_or_default()
    } else {
        s.scripts
            .configuration_parameters
            .as_ref()
            .map(|p| p.source.names().to_vec())
            .unwrap_or_default()
    }
}
fn defaults(ui: Ui, editor: bool) -> impl Widget {
    let state = ui.state;
    col().class("configuration-parameter-defaults").fill((
        row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Default configuration values")),crate::help_ui::group("Configuration parameters",&[
            "Enable parameter mode to use {{name}} inside configuration text. Names use ASCII letters, digits and underscores. Four opening or closing braces produce two literal braces. Values are single-line text, expanded once; quotes, dollar signs and replacement braces stay literal.",
            "Defaults apply to selected servers unless you explicitly override a value. Empty values are allowed, but each final configuration must be nonempty and no larger than 128 KiB. Substitution does not escape application syntax; review each server's rendered diff and validation before publishing.",
            "Draft text and defaults stay in editor memory. Starting preflight freezes per-server candidates and clears run-form values; invalid forms stay editable. Cancelling discards run values, deselection removes that server's overrides, and filtering preserves selected values.",
            "Only candidate contents are parameterized. Destination, validation, reload and health settings are shared; choose checks that verify each server's actual configuration.",
        ]))),
        col().class("configuration-parameters-grid").fill(each(state.map(move|s|names(s,editor))).key(|name|name.clone()).children(move|item|{
            let name=item.value.get();let read=name.clone();let write=name.clone();
            col().class("field").fill((text().class("field-label").content(name.clone()),input().name(i18n::bind(format!("Default value: {name}"))).value(state.map(move|s|if editor{s.configurations.defaults.get(&read).cloned().unwrap_or_default()}else{s.scripts.configuration_parameters.as_ref().and_then(|p|p.defaults.get(&read)).cloned().unwrap_or_default()})).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|{if editor{s.configurations.defaults.insert(write.clone(),value);}else if let Some(p)=&mut s.scripts.configuration_parameters{p.defaults.insert(write.clone(),value);}});})))
        })),
    ))
}
pub(super) fn editor(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("configuration-parameter-editor").fill((
        btn("Use configuration parameters", move || {
            state.revise(|s| {
                s.configurations.parameterized = !s.configurations.parameterized;
                if s.configurations.parameterized {
                    sync(s);
                } else {
                    s.configurations.defaults.clear();
                }
            })
        })
        .class(state.map(|s| {
            if s.configurations.parameterized {
                "choice active"
            } else {
                "choice"
            }
            .to_owned()
        })),
        when(state.map(|s| s.configurations.parameterized), move || {
            defaults(ui.clone(), true)
        }),
    ))
}
pub(super) fn run(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.scripts.configuration_parameters.is_some()),
        move || defaults(ui.clone(), false),
    )
}
pub(super) fn target(ui: Ui, id: String) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.scripts.configuration_parameters.is_some()),
        move || {
            let expanded = id.clone();
            let toggle = id.clone();
            let rows = id.clone();
            let clear = id.clone();
            col().class("configuration-target-parameters").fill((
                row().class("script-actions").fill((
                    btn("Configuration values…", move || {
                        state.revise(|s| {
                            if let Some(p) = &mut s.scripts.configuration_parameters
                                && !p.expanded.remove(&toggle)
                            {
                                p.expanded.insert(toggle.clone());
                            }
                        })
                    }),
                    text()
                        .class("subtle configuration-override-count")
                        .content(i18n::bind(state.map({
                            let id = id.clone();
                            move |s| {
                                format!(
                                    "Overrides: {}",
                                    s.scripts
                                        .configuration_parameters
                                        .as_ref()
                                        .and_then(|p| p.overrides.get(&id))
                                        .map_or(0, |v| v.len())
                                )
                            }
                        }))),
                )),
                when(
                    state.map(move |s| {
                        s.scripts
                            .configuration_parameters
                            .as_ref()
                            .is_some_and(|p| p.expanded.contains(&expanded))
                    }),
                    move || {
                        let server = rows.clone();
                        let clear = clear.clone();
                        col().class("configuration-overrides").fill((
                            btn("Use default values", move || {
                                state.revise(|s| {
                                    if let Some(p) = &mut s.scripts.configuration_parameters {
                                        p.overrides.remove(&clear);
                                    }
                                })
                            }),
                            each(state.map(|s| names(s, false)))
                                .key(|name| name.clone())
                                .children(move |item| {
                                    let name = item.value.get();
                                    let toggle = name.clone();
                                    let gate = name.clone();
                                    let read = name.clone();
                                    let write = name.clone();
                                    let toggle_id = server.clone();
                                    let gate_id = server.clone();
                                    let read_id = server.clone();
                                    let write_id = server.clone();
                                    row().class("configuration-parameter-row").fill((
                                        btn(format!("Override: {name}"), move || {
                                            state.revise(|s| {
                                                if !s.scripts.targets.contains(&toggle_id) {
                                                    return;
                                                }
                                                if let Some(p) =
                                                    &mut s.scripts.configuration_parameters
                                                {
                                                    let inherited = p
                                                        .defaults
                                                        .get(&toggle)
                                                        .cloned()
                                                        .unwrap_or_default();
                                                    let values = p
                                                        .overrides
                                                        .entry(toggle_id.clone())
                                                        .or_default();
                                                    if values.remove(&toggle).is_none() {
                                                        values.insert(toggle.clone(), inherited);
                                                    }
                                                }
                                            })
                                        })
                                        .class(state.map(
                                            {
                                                let server = gate_id.clone();
                                                let name = gate.clone();
                                                move |s| {
                                                    if s.scripts
                                                        .configuration_parameters
                                                        .as_ref()
                                                        .is_some_and(|p| {
                                                            p.overrides.get(&server).is_some_and(
                                                                |v| v.contains_key(&name),
                                                            )
                                                        })
                                                    {
                                                        "choice active"
                                                    } else {
                                                        "choice"
                                                    }
                                                    .to_owned()
                                                }
                                            },
                                        )),
                                        input()
                                            .name(i18n::bind(format!("Server value: {name}")))
                                            .enabled(state.map(move |s| {
                                                s.scripts
                                                    .configuration_parameters
                                                    .as_ref()
                                                    .is_some_and(|p| {
                                                        p.overrides
                                                            .get(&gate_id)
                                                            .is_some_and(|v| v.contains_key(&gate))
                                                    })
                                            }))
                                            .value(state.map(move |s| {
                                                s.scripts
                                                    .configuration_parameters
                                                    .as_ref()
                                                    .and_then(|p| {
                                                        p.overrides
                                                            .get(&read_id)
                                                            .and_then(|v| v.get(&read))
                                                            .or_else(|| p.defaults.get(&read))
                                                    })
                                                    .cloned()
                                                    .unwrap_or_default()
                                            }))
                                            .on(value_changed, move |event| {
                                                if let Event::ValueChanged { value, .. } = event {
                                                    state.revise(|s| {
                                                        if s.scripts.targets.contains(&write_id)
                                                            && let Some(p) = &mut s
                                                                .scripts
                                                                .configuration_parameters
                                                            && let Some(values) =
                                                                p.overrides.get_mut(&write_id)
                                                            && values.contains_key(&write)
                                                        {
                                                            values.insert(write.clone(), value);
                                                        }
                                                    });
                                                }
                                            }),
                                    ))
                                }),
                        ))
                    },
                ),
            ))
        },
    )
}
