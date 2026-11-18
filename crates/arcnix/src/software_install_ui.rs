use arcnix_core::compose_apps;
use arcnix_core::packages::{PackageAction, PackageManager};
use arcnix_core::software_install::{self, Action as InstallAction, Source};

use super::*;

#[derive(Clone)]
pub(super) struct InstallState {
    catalog_id: Option<&'static str>,
    source: u8,
    names: String,
    channel: String,
    classic: bool,
    compose: compose_apps::Definition,
    port: String,
    preview: bool,
}
impl Default for InstallState {
    fn default() -> Self {
        Self {
            catalog_id: None,
            source: 0,
            names: "nginx".into(),
            channel: "stable".into(),
            classic: false,
            compose: Default::default(),
            port: "8080".into(),
            preview: false,
        }
    }
}
fn available(s: &State) -> bool {
    s.connected
        && !scripts_ui::busy(s)
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
fn source_available(s: &State, source: u8) -> bool {
    match source {
        0 => s.software.environment.as_ref().is_some_and(|e| {
            software_install::supported(e.manager)
                && selected_app(s).is_none_or(|a| a.package(e.manager).is_some())
        }),
        1 => s.server_apps.applications.snap && selected_app(s).is_none_or(|a| a.snap.is_some()),
        2 => {
            s.server_apps.applications.docker
                && s.server_apps.applications.compose
                && selected_app(s).is_none_or(|a| a.compose.is_some())
        }
        _ => false,
    }
}
impl Ui {
    pub(super) fn install_catalog_app(&self, id: &'static str) {
        let Some(app) = arcnix_core::software_catalog::get(id) else {
            return;
        };
        self.state.revise(|s| {
            s.install = InstallState {
                catalog_id: Some(id),
                ..Default::default()
            };
            let source = if s.software.store.compose_only && source_available(s, 2) {
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
    pub(super) fn open_software_install(&self, source: u8, package: Option<String>) {
        if package.is_none() {
            self.open_software_store(source == 2);
            return;
        }
        self.state.revise(|s| {
            s.install = InstallState {
                source,
                names: package.unwrap_or_else(|| "nginx".into()),
                ..Default::default()
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
    fn review_software_install(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                available(s),
                "This installation source is not available on this server"
            );
            if s.install.source == 2 {
                let mut definition = s.install.compose.clone();
                definition.port = s.install.port.parse()?;
                definition.install()
            } else {
                let source = if s.install.source == 1 {
                    Source::Snap
                } else {
                    Source::Native(
                        s.software
                            .environment
                            .as_ref()
                            .map(|e| e.manager)
                            .unwrap_or(PackageManager::Unknown),
                    )
                };
                software_install::template(
                    source,
                    InstallAction::Install,
                    selected_app(s)
                        .and_then(|a| {
                            if s.install.source == 1 {
                                a.snap.map(|v| v.name)
                            } else {
                                a.package(
                                    s.software
                                        .environment
                                        .as_ref()
                                        .map(|e| e.manager)
                                        .unwrap_or(PackageManager::Unknown),
                                )
                            }
                        })
                        .map(|name| vec![name.to_owned()])
                        .unwrap_or_else(|| {
                            s.install
                                .names
                                .split_whitespace()
                                .map(str::to_owned)
                                .collect()
                        }),
                    &s.install.channel,
                    s.install.classic,
                )
            }
        });
        match result {
            Ok(t) => self.choose_server_script(t),
            Err(e) => self.state.revise(|s| s.dialog_error = format!("{e:#}")),
        }
    }
    pub(super) fn software_source_action(&self, names: Vec<String>, action: InstallAction) {
        let result = self.state.with(|s| {
            let source = if s.software.source == 1 {
                Source::Snap
            } else {
                Source::Native(
                    s.software
                        .environment
                        .as_ref()
                        .map(|e| e.manager)
                        .unwrap_or(PackageManager::Unknown),
                )
            };
            software_install::template(source, action, names, "stable", false)
        });
        match result {
            Ok(t) => self.choose_server_script(t),
            Err(e) => self.state.revise(|s| s.software.error = format!("{e:#}")),
        }
    }
    pub(super) fn native_package_preview(&self, action: PackageAction) {
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
fn set_source(s: &mut State, source: u8) {
    s.install.source = source;
    if let Some(app) = selected_app(s) {
        s.install.names = match source {
            0 => app.package(
                s.software
                    .environment
                    .as_ref()
                    .map(|e| e.manager)
                    .unwrap_or(PackageManager::Unknown),
            ),
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
                    app.and_then(|a| a.package(e.manager)).unwrap_or("")
                )
            })
            .unwrap_or_else(|| i18n::translate("Detecting system…")),
        1 => {
            if app.is_some_and(|a| a.snap.is_none()) {
                i18n::translate("Not provided for this app")
            } else if !s.server_apps.applications.snap {
                i18n::translate("Requires Snap")
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
            } else {
                app.and_then(|a| a.compose)
                    .map(|kind| kind.image().into())
                    .unwrap_or_else(|| "Docker Compose v2".into())
            }
        }
    }
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::SoftwareInstall)),
        move || {
            let review = ui.clone();
            let fields = ui.clone();
            modal(ui.clone(),"Install software",col().class("software-install-form").fill((
            each(state.map(|s|selected_app(s).into_iter().collect::<Vec<_>>())).key(|a|a.id).children(move|item|{
                let app=item.value.get();row().class("install-app-heading").fill((text().class(format!("store-app-icon store-color-{}",app.color)).content(app.mark),col().fill((text().class("install-app-name").content(app.name),text().class("subtle").content(i18n::bind(app.summary))))))
            }),
            text().class("install-method-heading").content(i18n::bind("Installation method")),
            row().class("install-sources").fill([(0,"System packages",Icon::Server),(1,"Snap",Icon::Grid),(2,"Docker Compose",Icon::Docker)].into_iter().map(move|(source,label,icon)|sapid::button().class("button install-source-card").name(i18n::bind(label)).label("").pressed(state.map(move|s|s.install.source==source)).enabled(state.map(move|s|source_available(s,source))).fill((icons::glyph(icon),text().class("install-source-name").content(i18n::bind(label)),text().class("install-source-note").content(state.map(move|s|source_note(s,source))))).on_press(move||state.revise(|s|set_source(s,source)))).collect::<Vec<_>>()),
            text().class("install-provider subtle").content(i18n::bind(state.map(|s|match s.install.source{0=>s.software.environment.as_ref().map(|e|format!("{} · {}",e.distribution,e.manager.label())).unwrap_or_else(||"Detecting system…".into()),1=>"Snap Store".into(),_=>"Docker Compose v2".into()}))),
            when(state.map(|s|s.install.source==2),{let ui=ui.clone();move||compose_form(ui.clone())}).otherwise(move||col().class("native-install-fields").fill((
                when(state.map(|s|s.install.catalog_id.is_none()),{let fields=fields.clone();move||field("Package names",&fields,|s|s.install.names.clone(),|s,v|s.install.names=v)}).otherwise(move||text().class("install-package-name").selectable(true).content(state.map(|s|selected_app(s).and_then(|a|if s.install.source==1{a.snap.map(|v|v.name)}else{a.package(s.software.environment.as_ref().map(|e|e.manager).unwrap_or(PackageManager::Unknown))}).unwrap_or_default().to_owned()))),
                when(state.map(|s|s.install.source==1),{let fields=fields.clone();move||col().fill((field("Snap channel",&fields,|s|s.install.channel.clone(),|s,v|s.install.channel=v),sapid::checkbox().label(i18n::bind("Allow classic confinement")).checked(state.map(|s|s.install.classic)).on_checked_change(move|v|state.revise(|s|s.install.classic=v)),when(state.map(|s|selected_app(s).and_then(|a|a.snap).is_some_and(|v|v.classic)&&!s.install.classic),move||text().class("subtle").content(i18n::bind("This Snap requires classic confinement"))),help_ui::tip("Snap confinement","Classic snaps have traditional host access. Enable only when the publisher requires it and you trust the package.")))}),
                help_ui::tip("Installation source","System packages use enabled distribution repositories; package names and versions differ. Snap and Compose are separate installations. Updates, services and data remain with their original source."),
            ))),
        )),btn("Review installation",move||review.review_software_install()).class("button primary").enabled(state.map(available)))
        },
    )
}
