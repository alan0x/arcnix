use arcnix_core::operations::{Intent, Service, ServiceAction, Snippet};

use crate::operations_worker::{Action as OpAction, Event as OpEvent, Kind};
use crate::*;

#[derive(Clone, Default)]
pub(crate) struct OperationsState {
    service_view: u8,
    startup: arcnix_core::startup_services::Definition,
    loaded: bool,
    services: Vec<Service>,
    selected: String,
    inspect_after_list: bool,
    filter: String,
    pending: Option<(u64, Kind)>,
    output: String,
    status: String,
    error: String,
    pub(crate) editor: Option<Snippet>,
}
fn ready(s: &State) -> bool {
    s.connected && !s.system_task_running && s.operations.pending.is_none()
}
impl Ui {
    pub(crate) fn open_system_service(&self, unit: &str) {
        self.state.revise(|s| {
            monitor_ui::set_section(s, monitor_ui::Section::Services);
            s.operations.service_view = 0;
            s.operations.filter.clear();
            s.operations.selected = unit.to_owned();
            s.operations.inspect_after_list = true;
        });
        self.open_desktop_app(WorkspacePage::Monitor);
        self.operations_list();
    }
    pub(crate) fn operations_enter(&self) {
        if self.state.with(|s| {
            monitor_ui::active(s) == Some(monitor_ui::Section::Services)
                && !s.operations.loaded
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
    fn new_startup_service(&self) {
        self.state.revise(|s| {
            s.operations.startup = arcnix_core::startup_services::Definition {
                user: s
                    .config
                    .servers
                    .iter()
                    .find(|p| Some(&p.id) == s.selected.as_ref())
                    .map(|p| p.username.clone())
                    .unwrap_or_default(),
                ..Default::default()
            }
        });
        self.open_dialog(Dialog::StartupService);
    }
    fn review_startup_service(&self) {
        match self.state.with(|s| s.operations.startup.template()) {
            Ok(template) => self.choose_server_script(template),
            Err(error) => self.state.revise(|s| s.dialog_error = format!("{error:#}")),
        }
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
            monitor_ui::set_section(s, monitor_ui::Section::Logs);
            s.system_logs.select_service(unit);
        });
        self.observability_enter();
    }
    pub(crate) fn edit_snippet(&self, snippet: Option<Snippet>) {
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
pub(crate) fn event(ui: &Ui, serial: u64, event: OpEvent) {
    let Some((expected, kind)) = ui.state.with(|s| s.operations.pending) else {
        return;
    };
    if expected != serial {
        return;
    }
    let inspect = ui.state.revise(|s| {
        let op = &mut s.operations;
        op.pending = None;
        match event {
            OpEvent::Services(items) if kind == Kind::List => {
                if !items.iter().any(|v| v.unit == op.selected) {
                    op.selected.clear();
                }
                op.services = items;
                op.loaded = true;
                return std::mem::take(&mut op.inspect_after_list) && !op.selected.is_empty();
            }
            OpEvent::Details(text) if kind == Kind::Inspect => {
                op.output = text;
                op.status = "Service details".into();
            }
            OpEvent::Failed(failed, error) if failed == kind => {
                if kind == Kind::List {
                    op.inspect_after_list = false;
                }
                op.error = error;
            }
            _ => {}
        }
        false
    });
    if inspect {
        ui.inspect_service();
    } else if kind != Kind::List && ui.state.with(|s| s.operations.inspect_after_list) {
        ui.operations_list();
    }
}
pub(crate) fn connection_lost(ui: &Ui) {
    ui.state
        .revise(|s| s.operations = OperationsState::default());
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
    let enable = ui.clone();
    let disable = ui.clone();
    let startup = ui.clone();
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
            service_button(&ui, "Enable at boot…", true, move || enable.service_action(ServiceAction::Enable)),
            service_button(&ui, "Disable at boot…", true, move || disable.service_action(ServiceAction::Disable)),
            text_action("New startup task", Icon::Plus).enabled(state.map(|s| ready(s) && !scripts_ui::busy(s))).on_press(move || startup.new_startup_service()),
        )),
        row().class("services-filters").fill((
            fragment([(0,"All services"),(1,"Active services"),(2,"Start at boot")].into_iter().map(move |(view,label)| sapid::button().class("button compact").label(i18n::bind(label)).pressed(state.map(move |s| s.operations.service_view == view)).on_press(move || state.revise(|s| s.operations.service_view = view))).collect::<Vec<_>>()),
            help_ui::tip("Services", "Running state and boot enablement are independent. Enable/disable changes boot behavior; start/stop changes the current state. Actions run as reviewed server tasks. Saved commands are in Scripts."),
        )),
        col().class("service-list").fill(
            each(state.map(|s| {
                let filter = s.operations.filter.to_lowercase();
                s.operations
                    .services
                    .iter()
                    .filter(|v| {
                        (s.operations.service_view == 0 || s.operations.service_view == 1 && v.active == "active" || s.operations.service_view == 2 && v.startup == "enabled") && format!("{} {} {} {}", v.unit, v.description, v.active, v.startup)
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
                            .content(value.map(|v| format!("{} · {} · {}", v.active, v.sub, v.startup))),
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
pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("operations-page services-page").fill((
        text()
            .class("software-error")
            .content(i18n::bind(state.map(|s| s.operations.error.clone()))),
        services(ui.clone()),
        when(state.map(|s| !s.operations.output.is_empty()), move || {
            col().class("operations-details").fill((
                text().class("section-title").content(i18n::bind("Output")),
                text()
                    .class("operations-output")
                    .selectable(true)
                    .content(state.map(|s| s.operations.output.clone())),
            ))
        }),
    ))
}
fn command_dialogs(ui: Ui) -> impl Widget {
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

pub(crate) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    let commands = ui.clone();
    fragment((
        command_dialogs(commands),
        when(
            state.map(|s| s.dialog == Some(Dialog::StartupService)),
            move || {
                let review = ui.clone();
                modal(ui.clone(), "New startup task", col().class("form-stack").fill((
            field("Task name", &ui, |s| s.operations.startup.name.clone(), |s,v| s.operations.startup.name = v),
            field("Run as user", &ui, |s| s.operations.startup.user.clone(), |s,v| s.operations.startup.user = v),
            col().class("field").fill((text().class("field-label").content(i18n::bind("Startup command")),
                input().multiline(true).class("command-input").name(i18n::bind("Startup command")).value(state.map(|s| s.operations.startup.command.clone())).on(value_changed,move |e| if let Event::ValueChanged {value,..}=e { state.revise(|s| s.operations.startup.command = value); })
            )),
            sapid::checkbox().label(i18n::bind("Keep running and restart on failure")).checked(state.map(|s| s.operations.startup.continuous)).on_checked_change(move |v| state.revise(|s| s.operations.startup.continuous = v)),
            help_ui::tip("Startup task", "Creates a systemd service enabled for boot. The script is private to the selected Linux user in ~/.arcnix/services. Root or sudo is required to install the unit. Start it separately to run it immediately."),
        )), btn("Review startup task", move || review.review_startup_service()).class("button primary"))
            },
        ),
    ))
}
