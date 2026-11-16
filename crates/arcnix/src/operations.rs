use arcnix_core::operations::{Intent, Service, ServiceAction, Snippet};

use super::*;
use crate::operations_worker::{Action as OpAction, Event as OpEvent, Kind};

#[derive(Clone, Default)]
pub(super) struct OperationsState {
    commands: bool,
    loaded: bool,
    services: Vec<Service>,
    selected: String,
    filter: String,
    pending: Option<(u64, Kind)>,
    output: String,
    status: String,
    error: String,
    pub(super) editor: Option<Snippet>,
}
fn ready(s: &State) -> bool {
    s.connected && !s.system_task_running && s.operations.pending.is_none()
}
impl Ui {
    pub(super) fn operations_enter(&self) {
        if self.state.with(|s| {
            navigation::workspace_visible(s)
                && s.workspace_page == WorkspacePage::Operations
                && !s.operations.loaded
                && !s.operations.commands
                && ready(s)
        }) {
            self.operations_list();
        }
    }
    fn op_send(&self, action: OpAction) {
        if !self.state.with(ready) {
            return;
        }
        let kind = action.kind();
        let serial = self.send(Action::Operations(action));
        self.state.revise(|s| {
            s.operations.pending = Some((serial, kind));
            s.operations.error.clear();
            s.operations.output.clear();
            s.operations.status.clear();
        });
    }
    fn operations_list(&self) {
        self.op_send(OpAction::List);
    }
    fn inspect_service(&self) {
        let unit = self.state.with(|s| s.operations.selected.clone());
        if !unit.is_empty() {
            self.op_send(OpAction::Inspect(unit));
        }
    }
    fn preview_operation(&self, intent: Intent) {
        let template = match intent {
            Intent::Command(snippet) => Ok(arcnix_core::scripts::Template::custom(snippet)),
            Intent::Service { unit, action } => {
                arcnix_core::scripts::Template::service(unit, action)
            }
        };
        match template {
            Ok(template) => self.choose_server_script(template),
            Err(error) => self.error(format!("{error:#}")),
        }
    }
    fn service_action(&self, action: ServiceAction) {
        let unit = self.state.with(|s| s.operations.selected.clone());
        if !unit.is_empty() {
            self.preview_operation(Intent::Service { unit, action });
        }
    }
    fn service_logs(&self) {
        let unit = self.state.with(|s| s.operations.selected.clone());
        if unit.is_empty() || !self.state.with(ready) {
            return;
        }
        self.state.revise(|s| {
            s.workspace_page = WorkspacePage::SystemLogs;
            s.system_logs.query = arcnix_core::system_logs::LogQuery {
                unit,
                ..Default::default()
            };
            s.system_logs.snapshot = None;
            s.system_logs.filter.clear();
        });
        self.observability_enter();
    }
    pub(super) fn edit_snippet(&self, snippet: Option<Snippet>) {
        self.state.revise(|s| {
            s.scripts.parameter_editor = Default::default();
            s.operations.editor = Some(snippet.unwrap_or_else(|| Snippet {
                parameters: vec![],
                id: new_id(),
                name: String::new(),
                description: String::new(),
                command: String::new(),
                administrator: false,
            }))
        });
        self.open_dialog(Dialog::SnippetEdit);
    }
    fn save_snippet(&self) {
        if let Err(error) = self.state.with(|s| {
            crate::script_parameters_ui::check_editor(
                s,
                &crate::script_parameters_ui::Owner::Command,
            )
        }) {
            self.state.revise(|s| s.dialog_error = error.to_string());
            return;
        }
        let Some(snippet) = self.state.with(|s| s.operations.editor.clone()) else {
            return;
        };
        let mut config = self.state.with(|s| s.config.clone());
        config.snippets.retain(|old| old.id != snippet.id);
        config.snippets.push(snippet);
        arcnix_core::script_parameters::reconcile(&mut config);
        match if self.writable.get() {
            self.store.save(&config)
        } else {
            Err(anyhow::anyhow!("Configuration is read-only"))
        } {
            Ok(()) => {
                self.state.revise(|s| {
                    s.config = config;
                    s.operations.editor = None;
                });
                self.close_dialog();
            }
            Err(e) => self.state.revise(|s| s.dialog_error = format!("{e:#}")),
        }
    }
    fn delete_snippet(&self, id: &str) {
        let mut config = self.state.with(|s| s.config.clone());
        config.snippets.retain(|v| v.id != id);
        arcnix_core::script_favorites::remove_user(&mut config, id);
        arcnix_core::script_parameters::reconcile(&mut config);
        match if self.writable.get() {
            self.store.save(&config)
        } else {
            Err(anyhow::anyhow!("Configuration is read-only"))
        } {
            Ok(()) => {
                self.state.revise(|s| s.config = config);
                self.close_dialog();
            }
            Err(e) => self.state.revise(|s| s.dialog_error = format!("{e:#}")),
        }
    }
}
pub(super) fn event(ui: &Ui, serial: u64, event: OpEvent) {
    let Some((expected, kind)) = ui.state.with(|s| s.operations.pending) else {
        return;
    };
    if expected != serial {
        return;
    }
    ui.state.revise(|s| {
        let op = &mut s.operations;
        op.pending = None;
        match event {
            OpEvent::Services(items) if kind == Kind::List => {
                if !items.iter().any(|v| v.unit == op.selected) {
                    op.selected.clear();
                }
                op.services = items;
                op.loaded = true;
            }
            OpEvent::Details(text) if kind == Kind::Inspect => {
                op.output = text;
                op.status = "Service details".into();
            }
            OpEvent::Failed(failed, error) if failed == kind => op.error = error,
            _ => {}
        }
    });
}
pub(super) fn connection_lost(ui: &Ui) {
    ui.state
        .revise(|s| s.operations = OperationsState::default());
}
fn mode(ui: Ui, commands: bool, label: &'static str) -> impl Widget {
    sapid::button()
        .label(i18n::bind(label))
        .class(ui.state.map(move |s| {
            if s.operations.commands == commands {
                "choice active"
            } else {
                "choice"
            }
            .to_owned()
        }))
        .on_press(move || {
            ui.state.revise(|s| {
                s.operations.commands = commands;
                s.operations.error.clear();
                s.operations.output.clear();
                s.operations.status.clear();
            });
            ui.operations_enter();
        })
}
fn service_button(
    ui: &Ui,
    label: &'static str,
    changing: bool,
    action: impl FnMut() + 'static,
) -> impl Widget {
    btn(label, action).enabled(ui.state.map(move |s| {
        ready(s) && (!changing || !scripts_ui::busy(s)) && !s.operations.selected.is_empty()
    }))
}
fn services(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    let inspect = ui.clone();
    let logs = ui.clone();
    let start = ui.clone();
    let stop = ui.clone();
    let restart = ui.clone();
    col().class("operation-services").fill((
        row().class("operations-toolbar").fill((
            row().class("server-search service-search").fill((
                input()
                    .class("service-filter")
                    .name(i18n::bind("Filter services"))
                    .value(state.map(|s| s.operations.filter.clone()))
                    .on(value_changed, move |e| {
                        if let Event::ValueChanged { value, .. } = e {
                            state.revise(|s| s.operations.filter = value);
                        }
                    }),
                text()
                    .class("action-glyph glyph-search server-search-icon")
                    .content(""),
            )),
            icon_action("刷新服务", Icon::Refresh)
                .enabled(state.map(ready))
                .on_press(move || refresh.operations_list()),
            service_button(&ui, "Status", false, move || inspect.inspect_service()),
            service_button(&ui, "Logs", false, move || logs.service_logs()),
            service_button(&ui, "Start…", true, move || {
                start.service_action(ServiceAction::Start)
            }),
            service_button(&ui, "Stop…", true, move || {
                stop.service_action(ServiceAction::Stop)
            }),
            service_button(&ui, "Restart…", true, move || {
                restart.service_action(ServiceAction::Restart)
            }),
        )),
        text().class("subtle").content(i18n::bind(
            "Loaded systemd services. Select a row to inspect it or review an action.",
        )),
        col().class("service-list").fill(
            each(state.map(|s| {
                let filter = s.operations.filter.to_lowercase();
                s.operations
                    .services
                    .iter()
                    .filter(|v| {
                        format!("{} {} {}", v.unit, v.description, v.active)
                            .to_lowercase()
                            .contains(&filter)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            }))
            .key(|v| v.unit.clone())
            .children(move |item| {
                let value = item.value;
                let select = ui.clone();
                sapid::button()
                    .class(state.map(move |s| {
                        if s.operations.selected == value.get().unit {
                            "service-row selected"
                        } else {
                            "service-row"
                        }
                        .to_owned()
                    }))
                    .name(value.map(|v| v.unit.clone()))
                    .fill((
                        text()
                            .class("service-name")
                            .content(value.map(|v| v.unit.clone())),
                        text()
                            .class("service-state")
                            .content(value.map(|v| format!("{} · {}", v.active, v.sub))),
                        text()
                            .class("service-description")
                            .content(value.map(|v| v.description.clone())),
                    ))
                    .enabled(state.map(ready))
                    .on_press(move || {
                        select.state.revise(|s| {
                            s.operations.selected = value.get().unit;
                            s.operations.output.clear();
                            s.operations.status.clear();
                        })
                    })
            }),
        ),
        text().class("subtle").content(i18n::bind(state.map(|s| {
            if s.operations.pending.is_some() {
                "Loading…"
            } else if s.operations.loaded && s.operations.services.is_empty() {
                "No loaded services"
            } else {
                ""
            }
            .to_owned()
        }))),
    ))
}
fn snippets(ui: Ui) -> impl Widget {
    let state = ui.state;
    let add = ui.clone();
    col().class("operation-snippets").fill((
        row().class("operations-toolbar").fill((
            text().class("subtle").content(i18n::bind("Saved commands are shared across servers. Review the target and command before running.")),
            text_action("New command",Icon::Plus).class("button with-icon primary").on_press(move||add.edit_snippet(None)),
        )),
        text().class("subtle").content(i18n::bind(state.map(|s|if s.config.snippets.is_empty(){"No saved commands. Add a command you use often; do not store passwords or tokens in it."}else{""}.to_owned()))),
        col().class("snippet-list").fill(each(state.map(|s|s.config.snippets.clone())).key(|v|v.id.clone()).children(move|item|{
            let value=item.value;let run=ui.clone();let edit=ui.clone();let delete=ui.clone();
            row().class("snippet-row").fill((
                col().class("snippet-copy").fill((text().class("snippet-name").content(value.map(|v|v.name.clone())),text().class("subtle").content(value.map(|v|v.description.clone())))),
                icon_action("Run command…",Icon::Play).enabled(state.map(|s| ready(s) && !scripts_ui::busy(s))).on_press(move||run.preview_operation(Intent::Command(value.get()))),
                row().class("snippet-actions").fill((
                    icon_action("Edit command",Icon::Edit).on_press(move||edit.edit_snippet(Some(value.get()))),
                    icon_action("Delete command",Icon::Clear).on_press(move||delete.open_dialog(Dialog::SnippetDelete(value.get().id))),
                )),
            ))
        })),
    ))
}
pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("operations-page").fill((
        row().class("operations-modes").fill((mode(ui.clone(),false,"Services"),mode(ui.clone(),true,"Commands"))),
        text().class(state.map(|s|if s.operations.error.is_empty(){"hidden"}else{"software-error"}.to_owned())).content(i18n::bind(state.map(|s|s.operations.error.clone()))),
        when(state.map(|s|s.operations.commands),{let ui=ui.clone();move||snippets(ui.clone())}).otherwise({let ui=ui.clone();move||services(ui.clone())}),
        when(state.map(|s| !s.operations.commands), move || col().class("operations-details").fill((
            row().class("operations-result-heading").fill((text().class("section-title").content(i18n::bind("Output")),text().class("subtle").content(i18n::bind(state.map(|s|s.operations.status.clone()))))),
            text().class("operations-output").selectable(true).content(state.map(|s|s.operations.output.clone())),
        ))),
        text().class("subtle").content(i18n::bind("Service changes and saved commands run as server-owned tasks. Review their plans and results in Tasks; use Terminal for interactive programs.")),
        btn("View server tasks", { let ui=ui.clone(); move || ui.navigate(navigation::Page::Tasks) }),
    ))
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        when(state.map(|s| s.dialog == Some(Dialog::SnippetEdit)), {
            let ui = ui.clone();
            move || {
                let save = ui.clone();
                let cancel = ui.clone();
                let admin = ui.clone();
                modal(
                    ui.clone(),
                    "Save command",
                    col().class("form-stack").fill((
                        field(
                            "Command name",
                            &ui,
                            |s| {
                                s.operations
                                    .editor
                                    .as_ref()
                                    .map(|e| e.name.clone())
                                    .unwrap_or_default()
                            },
                            |s, v| {
                                if let Some(e) = &mut s.operations.editor {
                                    e.name = v;
                                }
                            },
                        ),
                        field(
                            "Description",
                            &ui,
                            |s| {
                                s.operations
                                    .editor
                                    .as_ref()
                                    .map(|e| e.description.clone())
                                    .unwrap_or_default()
                            },
                            |s, v| {
                                if let Some(e) = &mut s.operations.editor {
                                    e.description = v;
                                }
                            },
                        ),
                        text()
                            .class("field-label")
                            .content(i18n::bind("Shell command")),
                        input()
                            .class("command-editor")
                            .name(i18n::bind("Shell command"))
                            .multiline(true)
                            .value(state.map(|s| {
                                s.operations
                                    .editor
                                    .as_ref()
                                    .map(|e| e.command.clone())
                                    .unwrap_or_default()
                            }))
                            .on(value_changed, move |e| {
                                if let Event::ValueChanged { value, .. } = e {
                                    state.revise(|s| {
                                        if let Some(e) = &mut s.operations.editor {
                                            e.command = value.replace("\r\n", "\n");
                                        }
                                    });
                                }
                            }),
                        btn("Run as administrator", move || {
                            admin.state.revise(|s| {
                                if let Some(e) = &mut s.operations.editor {
                                    e.administrator = !e.administrator;
                                }
                            })
                        })
                        .class(state.map(|s| {
                            if s.operations
                                .editor
                                .as_ref()
                                .is_some_and(|e| e.administrator)
                            {
                                "choice active"
                            } else {
                                "choice"
                            }
                            .to_owned()
                        })),
                        text().class("hint").content(i18n::bind(
                            "Command text is saved locally. Do not include passwords or tokens.",
                        )),
                        crate::script_parameters_ui::editor(
                            ui.clone(),
                            crate::script_parameters_ui::Owner::Command,
                        ),
                    )),
                    (
                        btn("取消", move || cancel.close_dialog()),
                        btn("Save command", move || save.save_snippet()).class("button primary"),
                    ),
                )
            }
        }),
        when(
            state.map(|s| matches!(s.dialog, Some(Dialog::SnippetDelete(_)))),
            {
                let ui = ui.clone();
                move || {
                    let submit = ui.clone();
                    let cancel = ui.clone();
                    modal(
                        ui.clone(),
                        "Delete command",
                        text().content(state.map(|s| {
                            if let Some(Dialog::SnippetDelete(id)) = &s.dialog {
                                s.config
                                    .snippets
                                    .iter()
                                    .find(|e| &e.id == id)
                                    .map(|e| e.name.clone())
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            }
                        })),
                        (
                            btn("取消", move || cancel.close_dialog()),
                            btn("Delete command", move || {
                                let id = submit.state.with(|s| {
                                    if let Some(Dialog::SnippetDelete(id)) = &s.dialog {
                                        Some(id.clone())
                                    } else {
                                        None
                                    }
                                });
                                if let Some(id) = id {
                                    submit.delete_snippet(&id);
                                }
                            })
                            .class("button danger"),
                        ),
                    )
                }
            },
        ),
    ))
}
