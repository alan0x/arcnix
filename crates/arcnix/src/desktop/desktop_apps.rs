use glory::render::NodeId;

use crate::i18n::desktop as i18n;
use crate::*;

fn matching_apps(s: &State, query: &str) -> Vec<WorkspacePage> {
    let query = query.to_lowercase();
    let language = desktop_system_settings::language(s);
    server_desktop::available_apps(s)
        .into_iter()
        .filter(|&page| {
            let label = server_desktop::app_label(page);
            let english = crate::i18n::translate_for(label, Language::English).to_lowercase();
            let localized = crate::i18n::translate_for(label, language).to_lowercase();
            query
                .split_whitespace()
                .all(|word| english.contains(word) || localized.contains(word))
        })
        .collect()
}

fn launch(ui: &Ui, page: WorkspacePage) {
    if !ui.state.with(|s| {
        server_desktop::visible(s)
            && matches!(s.dialog, None | Some(Dialog::Launcher))
            && server_desktop::available_apps(s).contains(&page)
    }) {
        return;
    }
    if ui.state.with(|s| s.dialog == Some(Dialog::Launcher)) {
        ui.close_dialog();
    }
    if ui.state.with(|s| s.dialog.is_none()) {
        ui.open_desktop_app(page);
        if page.is_files() {
            files_tabs::leave_apps(ui);
        }
    }
}

fn search(ui: Ui, query: Cage<String>, field: Option<Cage<Option<NodeId>>>) -> impl Widget {
    let state = ui.state;
    let mut input = sapid::text_field()
        .unstyled()
        .name(i18n::bind("Search apps"))
        .placeholder(i18n::bind("Search apps"))
        .value(query)
        .on_value_change(move |value| query.revise(|current| *current = value))
        .on_commit(move |value| {
            if !value.trim().is_empty()
                && let Some(page) = state.with(|s| matching_apps(s, &value).first().copied())
            {
                launch(&ui, page);
            }
        });
    if let Some(field) = field {
        input = input.node_ref(field);
    }
    row()
        .class(if field.is_some() {
            "desktop-app-catalog-search"
        } else {
            "files-location-search"
        })
        .fill((icons::glyph(Icon::Search), input))
}

fn catalog(ui: Ui, query: Cage<String>, in_files: bool) -> impl Widget {
    widget(move |scope| {
        let state = ui.state;
        let matches = scope.bond(move || state.with(|s| matching_apps(s, &query.get())));
        col()
            .class("desktop-app-catalog-scroll")
            .fill((
                col().class("desktop-app-catalog-grid").fill(
                    each(matches)
                        .key(|page| server_desktop::app_class(*page))
                        .children({
                            let ui = ui.clone();
                            move |item| {
                                let page = item.value.get();
                                let label = i18n::bind(server_desktop::app_label(page));
                                let open = ui.clone();
                                sapid::button()
                                    .class(format!(
                                        "desktop-app-catalog-item {}",
                                        server_desktop::app_class(page)
                                    ))
                                    .name(label.clone())
                                    .fill((
                                        text().class("desktop-app-icon").content(""),
                                        text().class("desktop-app-label").content(label),
                                    ))
                                    .on_press(move || launch(&open, page))
                            }
                        }),
                ),
                when(scope.bond(move || matches.with(Vec::is_empty)), move || {
                    when(in_files, || {
                        files_ui::empty_state(Icon::Search, "No matching apps", "files-apps-empty")
                    })
                    .otherwise(|| {
                        text()
                            .class("desktop-app-catalog-empty")
                            .content(i18n::bind("No matching apps"))
                    })
                }),
            ))
            .show_in(scope)
    })
}

pub(crate) fn files(ui: Ui, window: u32) -> impl Widget {
    widget(move |scope| {
        let query = scope.cage(String::new());
        let open = ui.clone();
        let back = ui.clone();
        col()
            .class("desktop-app-catalog files-apps-catalog")
            .fill((
                row()
                    .class("files-window-tools files-location-toolbar files-apps-toolbar")
                    .fill((
                        i18n::icon_action("Back to folder", Icon::ChevronRight)
                            .class("icon-button files-history-back")
                            .on_press(move || {
                                if files_ui::activate_window(&back, window) {
                                    files_tabs::leave_apps(&back);
                                }
                            }),
                        search(ui.clone(), query, None),
                        when(ui.state.map(|s| s.local_desktop.is_none()), move || {
                            let open = open.clone();
                            i18n::text_action("App Center", Icon::AppCenter)
                                .on_press(move || launch(&open, WorkspacePage::AppCenter))
                        }),
                    )),
                catalog(ui.clone(), query, true),
            ))
            .show_in(scope)
    })
}

fn launcher_locations(ui: Ui, query: Cage<String>) -> impl Widget {
    let state = ui.state;
    when(query.map(|q| q.trim().is_empty()), move || {
        let ui = ui.clone();
        when(
            state.map(|s| {
                s.sftp_available
                    && desktop_customization_ui::appearance(s)
                        .dock
                        .launcher_locations
            }),
            move || {
                let open = ui.clone();
                row()
                    .class("launcher-locations")
                    .fill(files_ui::location_links(
                        state,
                        |_| "",
                        false,
                        move |path| {
                            if open.state.with(|s| {
                                s.dialog == Some(Dialog::Launcher) && s.file_pending.is_none()
                            }) {
                                open.close_dialog();
                                files_tabs::new_window(
                                    &open,
                                    files_tabs::WindowStart::Directory(path),
                                );
                            }
                        },
                    ))
            },
        )
    })
}

fn launcher_power(ui: Ui) -> impl Widget {
    when(
        ui.state.map(|s| {
            desktop_customization_ui::appearance(s).dock.launcher_power
                && desktop_power::supported(s)
        }),
        move || desktop_power::menu(ui.clone(), desktop_power::MenuLocation::Launcher),
    )
}

pub(crate) fn dialog(ui: Ui) -> impl Widget {
    when(
        ui.state.map(|s| s.dialog == Some(Dialog::Launcher)),
        move || {
            let ui = ui.clone();
            widget(move |scope| {
                let query = scope.cage(String::new());
                let field = scope.cage(None);
                let mounted = scope.mount_with(|| {
                    modal_with_header(
                        ui.clone(),
                        "Launcher",
                        search(ui.clone(), query, Some(field)),
                        col().class("desktop-app-catalog").fill((
                            catalog(ui.clone(), query, false),
                            launcher_locations(ui.clone(), query),
                        )),
                        launcher_power(ui.clone()),
                    )
                })?;
                scope.retain(mounted)?;
                if let Some(id) = field.get() {
                    scope.action(glory::render::Action::Focus(id))?;
                }
                Ok(())
            })
        },
    )
}
