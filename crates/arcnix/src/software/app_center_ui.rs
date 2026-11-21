use arcnix_core::software_catalog::{self as catalog, App, Category, Inventory};
use arcnix_core::software_install::{self, Source};

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{btn, icon_action, text_action};
use crate::*;

#[derive(Clone, Default)]
pub(crate) struct AppCenterState {
    pub view: u8,
    pub query: String,
    pub category: Option<Category>,
    pub selected: Option<&'static str>,
    pub compose_only: bool,
    pub inventory: Option<Inventory>,
    pub pending: Option<u64>,
    pub error: String,
    pub all_packages: bool,
}
#[derive(Clone)]
enum Item {
    Package(Source, arcnix_core::packages::Package),
    Project(arcnix_core::compose_apps::Project),
}
impl Item {
    fn key(&self) -> String {
        format!("{}:{}", self.source(), self.name())
    }
    fn name(&self) -> String {
        match self {
            Self::Package(_, p) => p.name.clone(),
            Self::Project(p) => p.definition.name.clone(),
        }
    }
    fn app(&self) -> Option<App> {
        match self {
            Self::Package(source, p) => catalog::for_package(*source, &p.name),
            Self::Project(p) => catalog::APPS
                .iter()
                .find(|a| a.compose == Some(p.definition.kind))
                .copied(),
        }
    }
    fn source(&self) -> String {
        match self {
            Self::Package(Source::Native(m), _) => m.label().into(),
            Self::Package(Source::Snap, _) => "Snap".into(),
            Self::Project(_) => "Compose".into(),
        }
    }
    fn version(&self) -> String {
        match self {
            Self::Package(_, p) => p.version.clone(),
            Self::Project(p) => p.definition.image.clone(),
        }
    }
}
fn items(s: &State) -> Vec<Item> {
    let Some(inv) = &s.software.app_center.inventory else {
        return vec![];
    };
    let source = Source::Native(inv.manager);
    inv.native
        .iter()
        .filter(|p| {
            s.software.app_center.all_packages || catalog::for_package(source, &p.name).is_some()
        })
        .cloned()
        .map(|p| Item::Package(source, p))
        .chain(
            inv.snap
                .iter()
                .cloned()
                .map(|p| Item::Package(Source::Snap, p)),
        )
        .chain(inv.projects.iter().cloned().map(Item::Project))
        .filter(|i| {
            format!(
                "{} {} {}",
                i.name(),
                i.source(),
                i.app().map(|a| a.name).unwrap_or_default()
            )
            .to_lowercase()
            .contains(&s.software.app_center.query.to_lowercase())
        })
        .collect()
}
fn apps(s: &State) -> Vec<App> {
    let app_center = &s.software.app_center;
    catalog::APPS
        .iter()
        .filter(|a| {
            app_center.category.is_none_or(|c| a.category == c)
                && (!app_center.compose_only || a.compose.is_some())
                && format!(
                    "{} {} {} {} {}",
                    a.name,
                    a.summary,
                    a.uses,
                    i18n::translate(a.summary),
                    i18n::translate(a.category.label())
                )
                .to_lowercase()
                .contains(&app_center.query.to_lowercase())
        })
        .copied()
        .collect()
}
fn installed(s: &State, app: App) -> bool {
    s.software.app_center.inventory.as_ref().is_some_and(|i| {
        i.native
            .iter()
            .any(|p| app.package(i.manager) == Some(p.name.as_str()))
            || i.snap
                .iter()
                .any(|p| app.snap.is_some_and(|v| v.name == p.name))
            || i.projects
                .iter()
                .any(|p| app.compose == Some(p.definition.kind))
    })
}
fn badge(app: App) -> impl Widget {
    text()
        .class(format!(
            "app-center-app-icon app-center-color-{}",
            app.color
        ))
        .content(app.mark)
}

impl Ui {
    pub(crate) fn open_app_center(&self, compose_only: bool) {
        self.state.revise(|s| {
            s.software.app_center.view = 0;
            s.software.app_center.selected = None;
            s.software.app_center.query.clear();
            s.software.app_center.category = None;
            s.software.app_center.compose_only = compose_only;
        });
        self.open_desktop_app(WorkspacePage::AppCenter);
        self.app_center_enter();
    }
    pub(crate) fn app_center_enter(&self) {
        if self.state.with(|s| {
            s.workspace_page == WorkspacePage::AppCenter
                && s.software.app_center.inventory.is_none()
                && s.software.app_center.pending.is_none()
                && s.software.app_center.error.is_empty()
        }) {
            self.app_center_refresh();
        }
    }
    pub(crate) fn app_center_refresh(&self) {
        if !self
            .state
            .with(|s| s.connected && s.software.app_center.pending.is_none())
        {
            return;
        }
        let serial = self.send(Action::ServerApps(
            server_apps_worker::Action::SoftwareInventory,
        ));
        self.state.revise(|s| {
            s.software.app_center.pending = Some(serial);
            s.software.app_center.error.clear();
        });
    }
    fn app_center_details(&self, id: &'static str) {
        self.state.revise(|s| {
            s.software.app_center.selected = Some(id);
            s.software.app_center.view = 2;
        });
    }
    fn app_center_item_action(&self, item: Item, action: software_install::Action) {
        let Item::Package(source, p) = item else {
            return;
        };
        self.open_software_action(source, action, vec![p.name]);
    }
    pub(crate) fn refresh_software_inventory(&self) {
        self.state.revise(|s| s.software.app_center.pending = None);
        self.app_center_refresh();
        if self.state.with(|s| s.software.app_center.view == 3) {
            if self.state.with(|s| s.software.installed_view) {
                self.software_installed();
            } else {
                self.software_search();
            }
        }
    }
}
pub(crate) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<server_apps_worker::Reply, String>,
) -> Option<std::result::Result<server_apps_worker::Reply, String>> {
    if !ui
        .state
        .with(|s| s.software.app_center.pending == Some(serial))
    {
        return Some(result);
    }
    ui.state.revise(|s| {
        s.software.app_center.pending = None;
        match result {
            Ok(server_apps_worker::Reply::SoftwareInventory(inv)) => {
                s.software.app_center.error = inv.errors.join("\n");
                s.software.app_center.inventory = Some(inv)
            }
            Err(e) => s.software.app_center.error = e,
            _ => s.software.app_center.error = "Invalid software inventory response".into(),
        }
    });
    None
}
fn card(ui: Ui, app: App) -> impl Widget {
    let state = ui.state;
    let detail = ui.clone();
    let install = ui.clone();
    col().class("app-center-app-card").fill((
        sapid::button()
            .class("app-center-card-open")
            .name(app.name)
            .label("")
            .fill((
                badge(app),
                text().class("app-center-card-name").content(app.name),
                text()
                    .class("app-center-card-category subtle")
                    .content(i18n::bind(app.category.label())),
                text()
                    .class("app-center-card-summary")
                    .content(i18n::bind(app.summary)),
            ))
            .on_press(move || detail.app_center_details(app.id)),
        row().class("app-center-card-footer").fill((
            text()
                .class("app-center-installed-badge")
                .content(i18n::bind(state.map(move |s| {
                    if installed(s, app) { "Installed" } else { "" }.to_owned()
                }))),
            btn("Install…", move || install.install_catalog_app(app.id))
                .class("button app-center-install-button")
                .enabled(state.map(|s| s.connected && !scripts_ui::busy(s))),
        )),
    ))
}
fn categories(state: Cage<State>) -> impl Widget {
    col()
        .class("app-center-categories")
        .name(i18n::bind("Application categories"))
        .fill(
            std::iter::once(None)
                .chain(Category::ALL.into_iter().map(Some))
                .map(move |category| {
                    sapid::button()
                        .class("button compact")
                        .label(i18n::bind(
                            category.map_or("All applications", Category::label),
                        ))
                        .pressed(state.map(move |s| s.software.app_center.category == category))
                        .on_press(move || {
                            state.revise(|s| {
                                s.software.app_center.category = category;
                                s.software.app_center.view = 0;
                            })
                        })
                })
                .collect::<Vec<_>>(),
        )
}

fn discover(ui: Ui) -> impl Widget {
    let state = ui.state;
    let all = ui.clone();
    col().class("app-center-discover").fill((
        when(
            state.map(|s| s.software.app_center.compose_only),
            move || {
                row().class("app-center-context-filter").fill((
                    text().content(i18n::bind("Compose-compatible applications")),
                    text_action("Show all applications", Icon::Clear).on_press({
                        let all = all.clone();
                        move || {
                            all.state
                                .revise(|s| s.software.app_center.compose_only = false)
                        }
                    }),
                ))
            },
        ),
        row().class("app-center-section-heading").fill((
            text()
                .class("section-title")
                .content(i18n::bind("Explore applications")),
            text()
                .class("subtle")
                .content(state.map(|s| apps(s).len().to_string())),
        )),
        col().class("app-center-app-grid").fill(
            each(state.map(apps))
                .key(|a| a.id)
                .children(move |item| card(ui.clone(), item.value.get())),
        ),
        when(state.map(|s| apps(s).is_empty()), move || {
            text()
                .class("app-center-empty")
                .content(i18n::bind("No matching applications"))
        }),
    ))
}
fn detail(ui: Ui) -> impl Widget {
    let state = ui.state;
    each(state.map(|s|s.software.app_center.selected.and_then(catalog::get).into_iter().collect::<Vec<_>>())).key(|a|a.id).children(move|item|{
        let app=item.value.get();let install=ui.clone();
        col().class("app-center-detail").fill((
            text_action("Back",Icon::ChevronRight).class("button with-icon app-center-back").on_press(move||state.revise(|s|s.software.app_center.view=0)),
            row().class("app-center-detail-header").fill((badge(app),col().class("app-center-detail-identity").fill((text().class("app-center-detail-name").content(app.name),text().class("subtle").content(i18n::bind(app.category.label())),text().class("app-center-detail-summary").content(i18n::bind(app.summary)))),btn("Install…",move||install.install_catalog_app(app.id)).class("button primary app-center-install-button").enabled(state.map(|s|s.connected&&!scripts_ui::busy(s))))),
            col().class("app-center-detail-section").fill((text().class("section-title").content(i18n::bind("Common uses")),text().class("app-center-detail-uses").content(i18n::bind(app.uses)))),
            col().class("app-center-detail-section").fill((text().class("section-title").content(i18n::bind("Project website")),text().class("app-center-website").selectable(true).content(app.website))),
            help_ui::tip("Application catalog","Built-in application descriptions are available offline. Installation uses the server's enabled repositories or the selected publisher's Snap/image. Versions and package availability vary; package installation may require additional service initialization. No third-party repository is added automatically."),
        ))
    })
}
fn installed_page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    col().class("app-center-installed").fill((
        row().class("app-center-inventory-toolbar").fill((
            text()
                .class("section-title")
                .content(i18n::bind("Installed applications")),
            sapid::checkbox()
                .label(i18n::bind("Show all system packages"))
                .checked(state.map(|s| s.software.app_center.all_packages))
                .on_checked_change(move |v| {
                    state.revise(|s| s.software.app_center.all_packages = v)
                }),
            icon_action("Refresh installed applications", Icon::Refresh)
                .enabled(state.map(|s| s.connected && s.software.app_center.pending.is_none()))
                .on_press(move || refresh.app_center_refresh()),
        )),
        when(
            state.map(|s| s.software.app_center.pending.is_some()),
            move || text().class("subtle").content(i18n::bind("Loading…")),
        ),
        col().class("app-center-inventory-list").fill(
            each(state.map(items)).key(Item::key).children(move |item| {
                let value = item.value;
                let install = ui.clone();
                let remove = ui.clone();
                let manage = ui.clone();
                let initial = value.get();
                row().class("app-center-installed-row").fill((
                    text()
                        .class(format!(
                            "app-center-app-icon small app-center-color-{}",
                            initial.app().map(|a| a.color).unwrap_or("teal")
                        ))
                        .content(initial.app().map(|a| a.mark).unwrap_or("▦")),
                    col().class("app-center-installed-name").fill((
                        text().content(
                            initial
                                .app()
                                .map(|a| a.name.to_owned())
                                .unwrap_or_else(|| initial.name()),
                        ),
                        text().class("subtle").content(value.map(Item::name)),
                    )),
                    text()
                        .class("app-center-source-badge")
                        .content(value.map(Item::source)),
                    text()
                        .class("app-center-installed-version subtle")
                        .content(value.map(Item::version)),
                    when(value.map(|i| matches!(i, Item::Package(..))), move || {
                        row().class("app-center-installed-actions").fill((
                            btn("Upgrade…", {
                                let install = install.clone();
                                move || {
                                    install.app_center_item_action(
                                        value.get(),
                                        software_install::Action::Upgrade,
                                    )
                                }
                            })
                            .enabled(state.map(|s| !scripts_ui::busy(s))),
                            btn("Remove…", {
                                let remove = remove.clone();
                                move || {
                                    remove.app_center_item_action(
                                        value.get(),
                                        software_install::Action::Remove,
                                    )
                                }
                            })
                            .enabled(state.map(|s| !scripts_ui::busy(s))),
                        ))
                    })
                    .otherwise(move || {
                        btn("Manage project", {
                            let manage = manage.clone();
                            move || {
                                state.revise(|s| s.docker.view = 3);
                                manage.open_desktop_app(WorkspacePage::Docker);
                                manage.compose_enter();
                            }
                        })
                    }),
                ))
            }),
        ),
        when(
            state.map(|s| {
                s.software.app_center.pending.is_none()
                    && s.software.app_center.inventory.is_some()
                    && items(s).is_empty()
            }),
            move || {
                text()
                    .class("app-center-empty")
                    .content(i18n::bind("No matching installed applications"))
            },
        ),
    ))
}
pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let custom = ui.clone();
    let refresh = ui.clone();
    let search = ui.clone();
    let catalog_ui = ui.clone();
    let installed_ui = ui.clone();
    let packages_ui = ui.clone();
    col().class("app-center-content").fill((
        row().class("app-center-toolbar").fill((
            row().class("app-center-navigation").fill(
                [(0, "Discover"), (1, "Installed")]
                    .into_iter()
                    .map(move |(view, label)| {
                        sapid::button()
                            .class("button app-center-nav-button")
                            .label(i18n::bind(label))
                            .pressed(state.map(move |s| {
                                s.software.app_center.view == view
                                    || view == 0 && s.software.app_center.view == 2
                            }))
                            .on_press(move || state.revise(|s| s.software.app_center.view = view))
                    })
                    .collect::<Vec<_>>(),
            ),
            input()
                .class("app-center-search")
                .name(i18n::bind("Search applications"))
                .semantics(state.map(|_| {
                    Some(Box::new(Semantics {
                        placeholder: Some(i18n::translate("Search applications")),
                        ..Default::default()
                    }))
                }))
                .value(state.map(|s| s.software.app_center.query.clone()))
                .on(value_changed, move |e| {
                    if let Event::ValueChanged { value, .. } = e {
                        state.revise(|s| {
                            s.software.app_center.query = value;
                            if s.software.app_center.view == 2 {
                                s.software.app_center.view = 0;
                            }
                        })
                    }
                }),
            text_action("Install by name…", Icon::Plus)
                .class("button with-icon app-center-custom-install")
                .enabled(state.map(|s| s.connected && !scripts_ui::busy(s)))
                .on_press(move || custom.open_software_install(0, Some(String::new()))),
            icon_action("Search package repositories", Icon::Search)
                .class("icon-button app-center-repository-search")
                .enabled(state.map(|s| s.connected))
                .on_press(move || {
                    state.revise(|s| {
                        s.software.app_center.view = 3;
                        s.software.source = 0;
                    });
                    search.software_installed();
                }),
            icon_action("Refresh installed applications", Icon::Refresh)
                .enabled(state.map(|s| s.connected && s.software.app_center.pending.is_none()))
                .on_press(move || refresh.app_center_refresh()),
        )),
        text()
            .class("error app-center-error")
            .content(state.map(|s| s.software.app_center.error.clone())),
        when(
            state.map(|s| matches!(s.software.app_center.view, 0 | 2)),
            move || {
                let discover_ui = catalog_ui.clone();
                let detail_ui = catalog_ui.clone();
                row().class("app-center-catalog").fill((
                    categories(state),
                    when(state.map(|s| s.software.app_center.view == 0), move || {
                        discover(discover_ui.clone())
                    })
                    .otherwise(move || detail(detail_ui.clone())),
                ))
            },
        ),
        when(state.map(|s| s.software.app_center.view == 1), move || {
            installed_page(installed_ui.clone())
        }),
        when(state.map(|s| s.software.app_center.view == 3), move || {
            software::packages(packages_ui.clone())
        }),
    ))
}
