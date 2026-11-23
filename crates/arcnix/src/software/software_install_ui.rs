use arcnix_core::compose_apps;
use arcnix_core::packages::{PackageAction, PackageManager};
use arcnix_core::software_install::{self, Action as InstallAction, Source};

use crate::scripts_ui::Submission;
use crate::*;

#[derive(Clone)]
pub(crate) struct InstallState {
    pub task: Option<String>,
    catalog_id: Option<&'static str>,
    source: u8,
    names: String,
    channel: String,
    classic: bool,
    compose: compose_apps::Definition,
    port: String,
    preview: bool,
    options: bool,
    upgrade_dependencies: bool,
}
impl Default for InstallState {
    fn default() -> Self {
        Self {
            task: None,
            catalog_id: None,
            source: 0,
            names: "nginx".into(),
            channel: "stable".into(),
            classic: false,
            compose: Default::default(),
            port: "8080".into(),
            preview: false,
            options: false,
            upgrade_dependencies: false,
        }
    }
}
fn needs_terminal(s: &State) -> bool {
    s.software
        .app_center
        .inventory
        .as_ref()
        .is_some_and(|inv| !inv.hosted_tasks)
}

pub(super) fn can_change_software(s: &State) -> bool {
    s.connected
        && !scripts_ui::busy(s)
        && s.software.app_center.inventory.as_ref().is_some_and(|inv| {
            (inv.manager.uses_native_terminal() || inv.is_root || inv.sudo_available)
                && (inv.hosted_tasks || s.terminal_available())
        })
}

fn package_source(s: &State) -> Source {
    if s.install.source == 1 {
        Source::Snap
    } else {
        Source::Native(
            s.software
                .app_center
                .inventory
                .as_ref()
                .map_or(PackageManager::Unknown, |inv| inv.manager),
        )
    }
}

fn terminal_command(s: &State) -> anyhow::Result<String> {
    let inv = s
        .software
        .app_center
        .inventory
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Checking available installation sources…"))?;
    if let Some(Dialog::SoftwareAction(action, _, source, names)) = &s.dialog {
        return software_install::terminal_command(
            *source,
            *action,
            names,
            "stable",
            false,
            inv.is_root,
        );
    }
    anyhow::ensure!(
        s.install.source != 2,
        "Compose installation requires background tasks"
    );
    software_install::terminal_command(
        package_source(s),
        InstallAction::Install,
        &s.install
            .names
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        &s.install.channel,
        s.install.classic,
        inv.is_root,
    )
}

fn available(s: &State) -> bool {
    can_change_software(s)
        && (!needs_terminal(s) || s.install.source != 2 && terminal_command(s).is_ok())
        && source_available(s, s.install.source)
        && !selected_app(s)
            .and_then(|a| a.snap)
            .is_some_and(|snap| s.install.source == 1 && snap.classic && !s.install.classic)
}
fn selected_app(s: &State) -> Option<arcnix_core::software_catalog::App> {
    s.install
        .catalog_id
        .and_then(arcnix_core::software_catalog::get)
}
fn native_catalog_package(
    s: &State,
    app: arcnix_core::software_catalog::App,
) -> Option<&'static str> {
    s.software
        .app_center
        .inventory
        .as_ref()?
        .native_package(app)
}
pub(crate) fn catalog_source_available(
    s: &State,
    app: arcnix_core::software_catalog::App,
    source: u8,
) -> bool {
    s.software
        .app_center
        .inventory
        .as_ref()
        .is_some_and(|inv| match source {
            0 => inv.native_package(app).is_some(),
            1 => inv.snap_available(app),
            2 => inv.compose_ready && app.compose.is_some(),
            _ => false,
        })
}
fn source_available(s: &State, source: u8) -> bool {
    if let Some(app) = selected_app(s) {
        return catalog_source_available(s, app, source);
    }
    match source {
        0 => s
            .software
            .app_center
            .inventory
            .as_ref()
            .is_some_and(|inv| software_install::supported(inv.manager)),
        1 => s.server_apps.applications.snap,
        2 => s
            .software
            .app_center
            .inventory
            .as_ref()
            .is_some_and(|inv| inv.compose_ready),
        _ => false,
    }
}
impl Ui {
    pub(crate) fn install_catalog_app(&self, id: &'static str) {
        let Some(app) = arcnix_core::software_catalog::get(id) else {
            return;
        };
        self.state.revise(|s| {
            s.scripts.credentials.clear();
            s.install = InstallState {
                catalog_id: Some(id),
                ..Default::default()
            };
            let source = if s.software.app_center.compose_only && source_available(s, 2) {
                2
            } else {
                (0..3)
                    .find(|source| source_available(s, *source))
                    .unwrap_or(0)
            };
            set_source(s, source);
            if let Some(kind) = app.compose {
                s.install.compose.kind = kind;
                s.install.compose.name = app.id.into();
                s.install.compose.image = kind.image().into();
                s.install.port = if kind == compose_apps::Kind::Redis {
                    "6379"
                } else {
                    "8080"
                }
                .into();
            }
        });
        self.open_dialog(Dialog::SoftwareInstall);
    }
    pub(crate) fn open_software_install(&self, source: u8, package: Option<String>) {
        if package.is_none() {
            self.open_app_center(source == 2);
            return;
        }
        self.state.revise(|s| {
            s.scripts.credentials.clear();
            s.install = InstallState {
                source,
                names: package.unwrap_or_else(|| "nginx".into()),
                ..Default::default()
            };
            if !source_available(s, source)
                && let Some(other) = (0..3).find(|n| source_available(s, *n))
            {
                set_source(s, other);
            }
        });
        if self
            .state
            .with(|s| s.software.environment.is_none() && s.software.pending.is_none())
        {
            self.software_detect();
        }
        self.open_dialog(Dialog::SoftwareInstall);
    }
    fn install_software(&self) {
        if self.state.with(needs_terminal) {
            if self.state.with(available) {
                match self.state.with(terminal_command) {
                    Ok(command) => self.terminal_with_command(command),
                    Err(error) => self.state.revise(|s| s.dialog_error = error.to_string()),
                }
            }
            return;
        }
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                available(s),
                "This installation source is not available on this server"
            );
            let target = s
                .selected
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Connect to the server again before continuing"))?;
            let mut template = if s.install.source == 2 {
                let mut definition = s.install.compose.clone();
                definition.port = s.install.port.parse()?;
                definition.install()
            } else {
                software_install::template(
                    package_source(s),
                    InstallAction::Install,
                    s.install
                        .names
                        .split_whitespace()
                        .map(str::to_owned)
                        .collect(),
                    &s.install.channel,
                    s.install.classic,
                )
            }?;
            let review_upgrades = dependency_review(s);
            if review_upgrades {
                template.set_installation_policy(arcnix_core::installers::Policy::AllowUpgrades)?;
            }
            Ok((template, target, review_upgrades))
        });
        match result.and_then(|(template, target, review_upgrades)| {
            self.configure_script(template, Some(target))?;
            Ok(review_upgrades)
        }) {
            Ok(true) => self.prepare_script(Submission::ReviewSoftware),
            Ok(false) => self.prepare_script(Submission::ApplySoftware),
            Err(e) => self.state.revise(|s| s.dialog_error = format!("{e:#}")),
        }
    }
    pub(crate) fn open_software_action(
        &self,
        source: Source,
        action: InstallAction,
        names: Vec<String>,
    ) {
        let Some(target) = self.state.with(|s| {
            (s.connected && !scripts_ui::busy(s))
                .then(|| s.selected.clone())
                .flatten()
        }) else {
            return;
        };
        let subject = if names.is_empty() {
            String::new()
        } else {
            let provider = match source {
                Source::Native(manager) => manager.label(),
                Source::Snap => "Snap",
            };
            format!("{} · {provider}", names.join(", "))
        };
        if matches!(source, Source::Native(manager) if manager.uses_native_terminal()) {
            match software_install::terminal_command(source, action, &names, "stable", false, false)
            {
                Ok(_) => {
                    self.state.revise(|s| s.install.options = false);
                    self.open_dialog(Dialog::SoftwareAction(action, subject, source, names));
                }
                Err(e) => self.state.revise(|s| {
                    s.software.error = format!("{e:#}");
                    s.software.app_center.error = s.software.error.clone();
                }),
            }
            return;
        }
        match software_install::template(source, action, names.clone(), "stable", false) {
            Ok(template) => {
                self.state.revise(|s| s.install.options = false);
                self.open_script(
                    template,
                    Some(target),
                    Dialog::SoftwareAction(action, subject, source, names),
                );
            }
            Err(e) => self.state.revise(|s| {
                s.software.error = format!("{e:#}");
                s.software.app_center.error = s.software.error.clone();
            }),
        }
    }
    pub(crate) fn software_source_action(&self, names: Vec<String>, action: InstallAction) {
        let source = self.state.with(|s| {
            if s.software.source == 1 {
                Source::Snap
            } else {
                Source::Native(
                    s.software
                        .environment
                        .as_ref()
                        .map(|e| e.manager)
                        .unwrap_or(PackageManager::Unknown),
                )
            }
        });
        self.open_software_action(source, action, names);
    }
    pub(crate) fn native_package_preview(&self, action: PackageAction) {
        match action {
            PackageAction::Install(names) => self.open_software_install(
                self.state.with(|s| s.software.source),
                Some(names.join(" ")),
            ),
            PackageAction::RefreshIndex => {
                self.software_source_action(vec![], InstallAction::RefreshIndex)
            }
        }
    }
}
fn apt_install(s: &State) -> bool {
    s.install.source == 0
        && s.software
            .environment
            .as_ref()
            .is_some_and(|e| e.manager == PackageManager::Apt)
}

fn dependency_review(s: &State) -> bool {
    apt_install(s) && s.install.upgrade_dependencies
}

fn credential_input(
    state: Cage<State>,
    id: String,
    label: &'static str,
    administrator: bool,
) -> impl Widget {
    let read_id = id.clone();
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .purpose(Purpose::Password)
            .name(i18n::bind(label))
            .value(state.map(move |s| {
                s.scripts
                    .credentials
                    .get(&read_id)
                    .map(|pair| if administrator { &pair.1 } else { &pair.0 })
                    .cloned()
                    .unwrap_or_default()
            }))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        let pair = s.scripts.credentials.entry(id.clone()).or_default();
                        *if administrator {
                            &mut pair.1
                        } else {
                            &mut pair.0
                        } = value;
                    });
                }
            }),
    ))
}

fn access_options(state: Cage<State>, install: bool) -> impl Widget {
    each(state.map(|s| selected_profile(s).cloned().into_iter().collect::<Vec<_>>()))
        .key(|p| p.id.clone())
        .children(move |item| {
            let profile = item.value.get();
            let id = profile.id.clone();
            let show_ssh = !needs_no_credentials(&profile);
            col().class("form-stack").fill((
                row().class("script-row-heading").fill((
                    text().class("subtle").content(format!(
                        "{} · {}@{}:{}",
                        profile.name, profile.username, profile.host, profile.port
                    )),
                    when(state.map(move |s| install && apt_install(s)), move || {
                        sapid::button()
                            .class("button")
                            .label(i18n::bind("Installation options"))
                            .pressed(state.map(|s| s.install.options))
                            .on_press(move || {
                                state.revise(|s| s.install.options = !s.install.options)
                            })
                    }),
                )),
                when(Vary::Value(show_ssh), move || {
                    credential_input(state, id.clone(), "SSH password or key passphrase", false)
                }),
                when(
                    state.map(|s| {
                        s.software
                            .app_center
                            .inventory
                            .as_ref()
                            .is_some_and(|inv| !inv.is_root && inv.sudo_available)
                    }),
                    move || {
                        credential_input(
                            state,
                            profile.id.clone(),
                            "Administrator password (if required by sudo)",
                            true,
                        )
                    },
                ),
                when(
                    state.map(move |s| install && apt_install(s) && s.install.options),
                    move || {
                        sapid::checkbox()
                            .label(i18n::bind("Review dependency upgrades"))
                            .checked(state.map(|s| s.install.upgrade_dependencies))
                            .on_checked_change(move |checked| {
                                state.revise(|s| s.install.upgrade_dependencies = checked)
                            })
                    },
                ),
            ))
        })
}
fn execution_options(ui: &Ui, install: bool) -> impl Widget {
    let state = ui.state;
    when(state.map(|s| s.software.app_center.inventory.as_ref().is_some_and(|inv| !inv.manager.uses_native_terminal() && !inv.is_root && !inv.sudo_available)), || {
        text().class("hint").content(i18n::bind("To change system software, connect with an account that can use sudo or log in as root."))
    }).otherwise(move || {
        when(state.map(needs_terminal), move || {
            when(state.map(move |s| install && s.install.source == 2), || {
                text().class("hint").content(i18n::bind("Compose installation requires background tasks"))
            }).otherwise(move || col().class("form-stack").fill((
                text().class("hint").content(i18n::bind("Background tasks are unavailable. Open this command in a new terminal, review it and press Enter to run it. Refresh App Center when finished.")),
                text().class("package-terminal-command").selectable(true).content(state.map(move |s| {
                    terminal_command(s).unwrap_or_else(|err| i18n::translate(&err.to_string()))
                })),
            )))
        }).otherwise(move || access_options(state, install))
    })
}

fn set_source(s: &mut State, source: u8) {
    s.install.source = source;
    if let Some(app) = selected_app(s) {
        s.install.names = match source {
            0 => native_catalog_package(s, app),
            1 => app.snap.map(|v| v.name),
            _ => None,
        }
        .unwrap_or_default()
        .into();
    }
}
fn compose_form(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("compose-install-form").fill((
  when(state.map(|s|s.install.catalog_id.is_none()),move||row().class("choice-list").fill(compose_apps::Kind::ALL.into_iter().map(move|kind|sapid::button().class("button").label(kind.label()).pressed(state.map(move|s|s.install.compose.kind==kind)).on_press(move||state.revise(|s|{s.install.compose.kind=kind;s.install.compose.image=kind.image().into();s.install.compose.name=kind.label().to_lowercase();s.install.port=if kind==compose_apps::Kind::Redis{"6379"}else{"8080"}.into();}))).collect::<Vec<_>>())),
  field("Project name",&ui,|s|s.install.compose.name.clone(),|s,v|s.install.compose.name=v),
  field("Container image",&ui,|s|s.install.compose.image.clone(),|s,v|s.install.compose.image=v),
  row().class("form-row").fill((field("Listen address",&ui,|s|s.install.compose.address.clone(),|s,v|s.install.compose.address=v),field("Host port",&ui,|s|s.install.port.clone(),|s,v|s.install.port=v))),
  row().class("section-heading").fill((text().class("compose-install-path subtle").content(state.map(|s|format!("~/.arcnix/apps/{}/compose.yaml",s.install.compose.name))),help_ui::tip("Installation location","Configuration is private to this SSH user, writable without changing /etc, and easy to back up. Nginx content is in the project html directory; named-volume data is stored by Docker. restart: unless-stopped restores containers when Docker starts; enable Docker itself at boot in Services."))),
  btn("Preview Compose",move||state.revise(|s|s.install.preview = !s.install.preview)),
  when(state.map(|s|s.install.preview),move||text().class("compose-preview").selectable(true).content(state.map(|s|{let mut d=s.install.compose.clone();d.port=s.install.port.parse().unwrap_or(0);d.yaml().unwrap_or_else(|e|e.to_string())}))),
 ))
}
fn source_note(s: &State, source: u8) -> String {
    let app = selected_app(s);
    match source {
        0 => s
            .software
            .environment
            .as_ref()
            .map(|e| {
                format!(
                    "{} · {}",
                    e.manager.label(),
                    app.and_then(|a| native_catalog_package(s, a))
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            if app.is_some() {
                                i18n::translate("Unavailable in enabled repositories")
                            } else {
                                String::new()
                            }
                        })
                )
            })
            .unwrap_or_else(|| i18n::translate("Detecting system…")),
        1 => {
            if app.is_some_and(|a| a.snap.is_none()) {
                i18n::translate("Not provided for this app")
            } else if !s.server_apps.applications.snap {
                i18n::translate("Requires Snap")
            } else if app.is_some_and(|a| !catalog_source_available(s, a, 1)) {
                i18n::translate("Unavailable in the Snap Store")
            } else {
                app.and_then(|a| a.snap)
                    .map(|v| format!("{} · {}", v.name, v.publisher))
                    .unwrap_or_else(|| "Snap Store".into())
            }
        }
        _ => {
            if app.is_some_and(|a| a.compose.is_none()) {
                i18n::translate("No Compose template")
            } else if !s.server_apps.applications.compose {
                i18n::translate("Requires Docker Compose")
            } else if app.is_some_and(|a| !catalog_source_available(s, a, 2)) {
                i18n::translate("Requires access to the Docker daemon")
            } else {
                app.and_then(|a| a.compose)
                    .map(|kind| kind.image().into())
                    .unwrap_or_else(|| "Docker Compose v2".into())
            }
        }
    }
}
pub(crate) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    let install_ui = ui.clone();
    fragment((
        when(
            state.map(|s| s.dialog == Some(Dialog::SoftwareInstall)),
            move || {
                let ui = install_ui.clone();
                let install = ui.clone();
                let fields = ui.clone();
                modal(ui.clone(),"Install software",col().class("software-install-form").fill((
            each(state.map(|s|selected_app(s).into_iter().collect::<Vec<_>>())).key(|a|a.id).children(move|item|{
                let app=item.value.get();row().class("install-app-heading").fill((text().class(format!("app-center-app-icon app-center-color-{}",app.color)).content(app.mark),col().fill((text().class("install-app-name").content(app.name),text().class("subtle").content(i18n::bind(app.summary))))))
            }),
            text().class("install-method-heading").content(i18n::bind("Installation method")),
            row().class("install-sources").fill([(0,"System packages",Icon::Server),(1,"Snap",Icon::Grid),(2,"Docker Compose",Icon::Docker)].into_iter().map(move|(source,label,icon)|sapid::button().class("button install-source-card").name(i18n::bind(label)).label("").pressed(state.map(move|s|s.install.source==source)).enabled(state.map(move|s|source_available(s,source))).fill((icons::glyph(icon),text().class("install-source-name").content(i18n::bind(label)),text().class("install-source-note").content(state.map(move|s|source_note(s,source))))).on_press(move||state.revise(|s|set_source(s,source)))).collect::<Vec<_>>()),
            text().class("install-provider subtle").content(i18n::bind(state.map(|s|match s.install.source{0=>s.software.environment.as_ref().map(|e|format!("{} · {}",e.distribution,e.manager.label())).unwrap_or_else(||"Detecting system…".into()),1=>"Snap Store".into(),_=>"Docker Compose v2".into()}))),
            when(state.map(|s|s.install.source==2),{let ui=ui.clone();move||compose_form(ui.clone())}).otherwise(move||col().class("native-install-fields").fill((
                when(state.map(|s|s.install.catalog_id.is_none()),{let fields=fields.clone();move||field("Package names",&fields,|s|s.install.names.clone(),|s,v|s.install.names=v)}).otherwise(move||text().class("install-package-name").selectable(true).content(state.map(|s|selected_app(s).and_then(|a|if s.install.source==1{a.snap.map(|v|v.name)}else{native_catalog_package(s,a)}).unwrap_or_default().to_owned()))),
                when(state.map(|s|s.install.source==1),{let fields=fields.clone();move||col().fill((field("Snap channel",&fields,|s|s.install.channel.clone(),|s,v|s.install.channel=v),sapid::checkbox().label(i18n::bind("Allow classic confinement")).checked(state.map(|s|s.install.classic)).on_checked_change(move|v|state.revise(|s|s.install.classic=v)),when(state.map(|s|selected_app(s).and_then(|a|a.snap).is_some_and(|v|v.classic)&&!s.install.classic),move||text().class("subtle").content(i18n::bind("This snap requires classic confinement"))),help_ui::tip("Snap confinement","Classic snaps have traditional host access. Enable only when the publisher requires it and you trust the package.")))}),
                help_ui::tip("Installation source","System packages use enabled distribution repositories; package names and versions differ. Snap and Compose are separate installations. Updates, services and data remain with their original source."),
            ))),
            execution_options(&ui, true),
        )),sapid::button().class("button primary").label(i18n::bind(state.map(|s| if needs_terminal(s) && s.install.source != 2 { "Open in terminal" } else if dependency_review(s) { "Review package changes…" } else { "Install" }.to_owned()))).enabled(state.map(available)).on_press(move||install.install_software()))
            },
        ),
        each(state.map(|s| match &s.dialog {
            Some(Dialog::SoftwareAction(action, subject, ..)) => vec![(*action, subject.clone())],
            _ => vec![],
        }))
        .key(|(_, subject)| subject.clone())
        .children(move |item| {
            let (action, subject) = item.value.get();
            let (title, label, description) = match action {
                InstallAction::Upgrade
                    if state.with(|s| {
                        s.scripts
                            .template
                            .as_ref()
                            .is_some_and(|t| t.snippet.name == "Full system upgrade (Pacman)")
                    }) =>
                {
                    (
                        "Full system upgrade (Pacman)",
                        "Upgrade system",
                        "Pacman upgrades the whole system in one transaction.",
                    )
                }
                InstallAction::Upgrade => (
                    "Upgrade software",
                    "Upgrade",
                    "The package manager may also upgrade dependencies.",
                ),
                InstallAction::Remove => (
                    "Remove software",
                    "Remove",
                    "The package manager may also remove packages that depend on this software.",
                ),
                InstallAction::RefreshIndex => (
                    "Update package index",
                    "Update",
                    "Refresh the list of packages available from this server’s repositories.",
                ),
                InstallAction::Install => unreachable!(),
            };
            let submit = ui.clone();
            let cancel = ui.clone();
            modal(
                ui.clone(),
                title,
                col().class("form-stack").fill((
                    when(Vary::Value(!subject.is_empty()), move || {
                        text().class("section-title").content(subject.clone())
                    }),
                    text().class("hint").content(i18n::bind(description)),
                    execution_options(&ui, false),
                )),
                (
                    btn("Cancel", move || cancel.close_dialog()),
                    sapid::button()
                        .label(i18n::bind(state.map(move |s| {
                            if needs_terminal(s) {
                                "Open in terminal"
                            } else {
                                label
                            }
                            .to_owned()
                        })))
                        .on_press(move || {
                            if submit.state.with(needs_terminal) {
                                match submit.state.with(terminal_command) {
                                    Ok(cmd) => submit.terminal_with_command(cmd),
                                    Err(err) => {
                                        submit.state.revise(|s| s.dialog_error = err.to_string())
                                    }
                                }
                            } else {
                                submit.prepare_script(Submission::ApplySoftware);
                            }
                        })
                        .class(if action == InstallAction::Remove {
                            "button danger"
                        } else {
                            "button primary"
                        })
                        .enabled(state.map(can_change_software)),
                ),
            )
        }),
    ))
}
