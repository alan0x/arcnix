use arcnix_core::server_apps::{ExplorerPreferences, FolderColor};

use super::*;

const COLORS: [(FolderColor, &str); 7] = [
    (FolderColor::Red, "Red"),
    (FolderColor::Orange, "Orange"),
    (FolderColor::Yellow, "Yellow"),
    (FolderColor::Green, "Green"),
    (FolderColor::Blue, "Blue"),
    (FolderColor::Purple, "Purple"),
    (FolderColor::Gray, "Gray"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TagFilter {
    Color(FolderColor),
    All,
}

impl TagFilter {
    fn label(self) -> &'static str {
        match self {
            Self::Color(color) => COLORS
                .iter()
                .find(|(value, _)| *value == color)
                .map_or("Tags", |(_, label)| *label),
            Self::All => "All Tags",
        }
    }
}

#[derive(Clone)]
pub(super) struct ItemRequest {
    pub serial: u64,
    path: String,
}

#[derive(serde::Deserialize)]
struct FavoriteDrop {
    epoch: u64,
    request: u64,
    path: String,
    from_favorites: bool,
    anchor: Option<String>,
    after: bool,
}

fn color_class(color: FolderColor) -> &'static str {
    match color {
        FolderColor::Red => "tag-red",
        FolderColor::Orange => "tag-orange",
        FolderColor::Yellow => "tag-yellow",
        FolderColor::Green => "tag-green",
        FolderColor::Blue => "tag-blue",
        FolderColor::Purple => "tag-purple",
        FolderColor::Gray => "tag-gray",
    }
}

pub(super) fn color_dot(s: &State, path: &str) -> String {
    preferences(s).colors.get(path).map_or_else(
        || "hidden".into(),
        |color| format!("folder-color-dot {}", color_class(*color)),
    )
}

fn preferences(s: &State) -> &ExplorerPreferences {
    &server_apps_ui::current(s).explorer
}

fn editable(s: &State) -> bool {
    s.connected
        && s.sftp_available
        && s.server_apps.preferences_loaded
        && s.server_apps.preferences_pending.is_none()
}

fn active(s: &State) -> bool {
    server_desktop::visible(s)
        && s.workspace_page == WorkspacePage::Explorer
        && s.sftp_available
        && s.dialog.is_none()
}

fn saved_item(s: &State, path: &str) -> bool {
    preferences(s).favorites.iter().any(|item| item == path)
        || preferences(s).colors.contains_key(path)
}

pub(super) fn item_opened(ui: &Ui, result: std::result::Result<server_apps_worker::Reply, String>) {
    let request = ui
        .state
        .revise(|s| s.server_apps.explorer_open_pending.take());
    let Some(request) =
        request.filter(|request| ui.state.with(|s| active(s) && saved_item(s, &request.path)))
    else {
        return;
    };
    match result {
        Ok(server_apps_worker::Reply::ExplorerItem(item)) if !item.symlink => {
            if item.directory {
                file_browser::navigate(ui, true, request.path);
            } else {
                remote_editor::open_verified(ui, &request.path);
            }
        }
        Ok(_) => ui.error("Could not open this item"),
        Err(error) => ui.error(error),
    }
}

impl Ui {
    pub(super) fn explorer_enter(&self) {
        if !self.state.with(|s| {
            server_desktop::visible(s)
                && s.workspace_page == WorkspacePage::Explorer
                && s.sftp_available
                && s.server_apps.explorer_locations.is_none()
                && s.server_apps.explorer_locations_pending.is_none()
        }) {
            return;
        }
        let serial = self.send(Action::ServerApps(
            server_apps_worker::Action::ExplorerLocations,
        ));
        self.state
            .revise(|s| s.server_apps.explorer_locations_pending = Some(serial));
    }

    fn change_explorer(&self, change: impl FnOnce(&mut ExplorerPreferences)) {
        if !self.state.with(editable) {
            return;
        }
        let mut prefs = self.state.with(server_apps_ui::desired);
        change(&mut prefs.explorer);
        if let Err(error) = prefs.explorer.validate() {
            self.error(format!("{error:#}"));
        } else if self.state.with(|s| &prefs != server_apps_ui::current(s)) {
            self.queue_desktop_preferences(prefs);
        }
    }

    pub(super) fn toggle_explorer_favorite(&self, path: String) {
        if ExplorerPreferences::validate_path(&path).is_err() {
            return;
        }
        self.change_explorer(|prefs| {
            if let Some(index) = prefs.favorites.iter().position(|p| p == &path) {
                prefs.favorites.remove(index);
            } else {
                prefs.favorites.push(path);
            }
        });
    }

    fn open_explorer_item(&self, path: String) {
        if !self.state.with(|s| active(s) && saved_item(s, &path)) {
            return;
        }
        let serial = self.send(Action::ServerApps(
            server_apps_worker::Action::ExplorerItem(path.clone()),
        ));
        self.state
            .revise(|s| s.server_apps.explorer_open_pending = Some(ItemRequest { serial, path }));
    }

    fn filter_explorer_tags(&self, filter: Option<TagFilter>) {
        self.state.revise(|s| {
            s.server_apps.explorer_tag = filter;
            s.server_apps.explorer_open_pending = None;
        });
    }

    fn drop_explorer_favorite(&self, drop: FavoriteDrop) {
        let valid = self.state.with(|s| {
            if !active(s)
                || !editable(s)
                || preferences(s).sidebar_hidden
                || drop.epoch != s.epoch
                || drop.request != s.remote_request
                || ExplorerPreferences::validate_path(&drop.path).is_err()
            {
                return false;
            }
            let favorites = &preferences(s).favorites;
            if let Some(anchor) = &drop.anchor
                && (anchor == &drop.path || !favorites.contains(anchor))
            {
                return false;
            }
            if drop.from_favorites {
                favorites.contains(&drop.path)
            } else {
                s.server_apps.explorer_tag.is_none()
                    && s.remote_ready
                    && file_browser::entries(s, true)
                        .iter()
                        .any(|entry| entry.path == drop.path && !entry.is_symlink)
            }
        });
        if valid {
            self.change_explorer(|prefs| {
                let favorites = &mut prefs.favorites;
                favorites.retain(|path| path != &drop.path);
                let index = drop
                    .anchor
                    .and_then(|anchor| favorites.iter().position(|path| *path == anchor))
                    .map_or(favorites.len(), |index| index + usize::from(drop.after));
                favorites.insert(index, drop.path);
            });
        }
    }
}

pub(super) fn item_target(s: &State) -> Option<(String, bool)> {
    if !editable(s) || s.workspace_page != WorkspacePage::Explorer || !s.remote_ready {
        return None;
    }
    let (path, directory) = if s.remote_selection.is_empty() {
        (s.remote_path.clone(), true)
    } else {
        if s.remote_selection.len() != 1 {
            return None;
        }
        let items = file_context::chosen(s, true);
        match items.as_slice() {
            [entry] if !entry.is_symlink => (entry.path.clone(), entry.is_dir),
            _ => return None,
        }
    };
    ExplorerPreferences::validate_path(&path)
        .ok()
        .map(|()| (path, directory))
}

pub(super) fn drag_bridge(ui: Ui) -> impl Widget {
    fragment((
        text().class("explorer-favorites-state hidden").content(
            ui.state
                .map(|s| format!(r#"{{"epoch":{},"request":{}}}"#, s.epoch, s.remote_request)),
        ),
        input()
            .class("explorer-favorites-input hidden")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event
                    && value.len() <= 4096
                    && let Ok(drop) = serde_json::from_str(&value)
                {
                    ui.drop_explorer_favorite(drop);
                }
            }),
    ))
}

pub(super) fn sidebar_toggle(ui: Ui) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class("icon-button explorer-sidebar-toggle")
        .name(i18n::bind("Toggle Explorer sidebar"))
        .pressed(state.map(|s| !preferences(s).sidebar_hidden))
        .enabled(state.map(editable))
        .fill(icons::glyph(Icon::List))
        .on_press(move || ui.change_explorer(|prefs| prefs.sidebar_hidden = !prefs.sidebar_hidden))
}

pub(super) fn favorite_button(ui: Ui) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class("icon-button explorer-favorite")
        .name(i18n::bind(state.map(|s| {
            if preferences(s).favorites.contains(&s.remote_path) {
                "Remove current folder from favorites"
            } else {
                "Add current folder to favorites"
            }
            .to_owned()
        })))
        .pressed(state.map(|s| {
            s.server_apps.explorer_tag.is_none()
                && preferences(s).favorites.contains(&s.remote_path)
        }))
        .enabled(state.map(|s| {
            editable(s)
                && s.server_apps.explorer_tag.is_none()
                && s.remote_ready
                && ExplorerPreferences::validate_path(&s.remote_path).is_ok()
        }))
        .fill((icons::glyph(Icon::Star), icons::glyph(Icon::StarFilled)))
        .on_press(move || ui.toggle_explorer_favorite(state.with(|s| s.remote_path.clone())))
}

fn place(ui: Ui, label: &'static str, path: String) -> impl Widget {
    let state = ui.state;
    let current_path = path.clone();
    sapid::button()
        .class("explorer-place")
        .name(i18n::bind(label))
        .pressed(
            state.map(move |s| {
                s.server_apps.explorer_tag.is_none() && s.remote_path == current_path
            }),
        )
        .fill((
            icons::glyph(match label {
                "Desktop" => Icon::Monitor,
                "Documents" => Icon::File,
                "Download directory" => Icon::Download,
                "Filesystem" => Icon::Disks,
                _ => Icon::Folder,
            }),
            text().content(i18n::bind(label)),
        ))
        .on_press(move || file_browser::navigate(&ui, true, path.clone()))
}

fn item_name(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
}

fn tag_link(ui: Ui, filter: TagFilter) -> impl Widget {
    sapid::button()
        .class("explorer-place explorer-tag-link")
        .name(i18n::bind(filter.label()))
        .pressed(
            ui.state
                .map(move |s| s.server_apps.explorer_tag == Some(filter)),
        )
        .fill((
            text()
                .class(match filter {
                    TagFilter::Color(color) => format!("folder-color-dot {}", color_class(color)),
                    TagFilter::All => "folder-color-dot tag-all".into(),
                })
                .content(""),
            text().content(i18n::bind(filter.label())),
        ))
        .on_press(move || ui.filter_explorer_tags(Some(filter)))
}

pub(super) fn sidebar(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.sftp_available && !preferences(s).sidebar_hidden),
        move || {
            let place_ui = ui.clone();
            let favorite_ui = ui.clone();
            col()
                .class("explorer-sidebar")
                .name(i18n::bind("Explorer sidebar"))
                .fill((
                    text()
                        .class("explorer-sidebar-heading")
                        .content(i18n::bind("Locations")),
                    each(
                        state.map(|s| s.server_apps.explorer_locations.clone().unwrap_or_default()),
                    )
                    .key(|location| location.name)
                    .children(move |item| {
                        let location = item.value.get();
                        place(place_ui.clone(), location.name, location.path)
                    }),
                    place(ui.clone(), "Filesystem", "/".into()),
                    col()
                        .class(state.map(|s| {
                            if editable(s) && active(s) {
                                "explorer-favorites editable"
                            } else {
                                "explorer-favorites"
                            }
                            .to_owned()
                        }))
                        .fill((
                            text()
                                .class("explorer-sidebar-heading")
                                .content(i18n::bind("Favorites")),
                            each(state.map(|s| preferences(s).favorites.clone()))
                                .key(String::clone)
                                .children(move |item| {
                                    let path = item.value;
                                    let open = favorite_ui.clone();
                                    let remove = favorite_ui.clone();
                                    row().class("explorer-favorite-row").name(path).fill((
                                        sapid::button()
                                            .class("explorer-place")
                                            .name(i18n::bind(
                                                path.map(|p| format!("Open favorite {p}")),
                                            ))
                                            .pressed(state.map(move |s| {
                                                s.server_apps.explorer_tag.is_none()
                                                    && s.remote_path == path.get()
                                            }))
                                            .fill((
                                                icons::glyph(Icon::Star),
                                                text()
                                                    .content(path.map(|p| item_name(p).to_owned())),
                                                text()
                                                    .class(
                                                        state.map(move |s| {
                                                            color_dot(s, &path.get())
                                                        }),
                                                    )
                                                    .content(""),
                                            ))
                                            .on_press(move || open.open_explorer_item(path.get())),
                                        sapid::button()
                                            .class("icon-button explorer-favorite-remove")
                                            .name(i18n::bind(
                                                path.map(|p| format!("Remove favorite {p}")),
                                            ))
                                            .enabled(state.map(editable))
                                            .fill(icons::glyph(Icon::Close))
                                            .on_press(move || {
                                                remove.toggle_explorer_favorite(path.get())
                                            }),
                                    ))
                                }),
                        )),
                    text()
                        .class("explorer-sidebar-heading")
                        .content(i18n::bind("Tags")),
                    fragment(
                        COLORS
                            .into_iter()
                            .map(|(color, _)| tag_link(ui.clone(), TagFilter::Color(color)))
                            .collect::<Vec<_>>(),
                    ),
                    tag_link(ui.clone(), TagFilter::All),
                ))
        },
    )
}

fn tagged_paths(s: &State) -> Vec<String> {
    let Some(filter) = s.server_apps.explorer_tag else {
        return Vec::new();
    };
    preferences(s)
        .colors
        .iter()
        .filter(|(_, color)| filter == TagFilter::All || filter == TagFilter::Color(**color))
        .map(|(path, _)| path.clone())
        .collect()
}

pub(super) fn tag_results(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.server_apps.explorer_tag.is_some()),
        move || {
            let back = ui.clone();
            let item_ui = ui.clone();
            col().class("explorer-tags-view").fill((
                row().class("explorer-tags-toolbar").fill((
                    icon_action("Back to folder", Icon::Folder)
                        .on_press(move || back.filter_explorer_tags(None)),
                    text().content(i18n::bind(state.map(|s| {
                        s.server_apps
                            .explorer_tag
                            .map_or("Tags", TagFilter::label)
                            .to_owned()
                    }))),
                )),
                col().class("explorer-tags-list").fill((
                    when(state.map(|s| tagged_paths(s).is_empty()), || {
                        text()
                            .class("explorer-tags-empty subtle")
                            .content(i18n::bind("No tagged items"))
                    }),
                    each(state.map(tagged_paths))
                        .key(String::clone)
                        .children(move |item| {
                            let path = item.value;
                            let open = item_ui.clone();
                            let clear = item_ui.clone();
                            row().class("explorer-tag-result").fill((
                                sapid::button()
                                    .class("explorer-place explorer-tag-item")
                                    .name(path)
                                    .fill((
                                        text()
                                            .class(state.map(move |s| color_dot(s, &path.get())))
                                            .content(""),
                                        text()
                                            .class("explorer-tag-item-name")
                                            .content(path.map(|p| item_name(p).to_owned())),
                                        text().class("explorer-tag-item-path subtle").content(path),
                                    ))
                                    .on_press(move || open.open_explorer_item(path.get())),
                                icon_action("Clear tag", Icon::Close)
                                    .enabled(state.map(editable))
                                    .on_press(move || {
                                        clear.change_explorer(|prefs| {
                                            prefs.colors.remove(&path.get());
                                        });
                                    }),
                            ))
                        }),
                )),
            ))
        },
    )
}

pub(super) fn color_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| matches!(s.dialog, Some(Dialog::ExplorerColor { .. }))),
        move || {
            let (path, directory) = state.with(|s| match &s.dialog {
                Some(Dialog::ExplorerColor { path, directory }) => (path.clone(), *directory),
                _ => (String::new(), true),
            });
            let buttons = COLORS
                .into_iter()
                .map(|(color, label)| {
                    let target = path.clone();
                    let color_ui = ui.clone();
                    let current = path.clone();
                    sapid::button()
                        .class("button folder-color-choice")
                        .name(i18n::bind(label))
                        .pressed(
                            state.map(move |s| preferences(s).colors.get(&current) == Some(&color)),
                        )
                        .fill((
                            text()
                                .class(format!("folder-color-dot {}", color_class(color)))
                                .content(""),
                            text().content(i18n::bind(label)),
                        ))
                        .on_press(move || {
                            color_ui.change_explorer(|prefs| {
                                prefs.colors.insert(target.clone(), color);
                            });
                            color_ui.close_dialog();
                        })
                })
                .collect::<Vec<_>>();
            let clear = ui.clone();
            modal(
                ui.clone(),
                if directory {
                    "Folder color"
                } else {
                    "File color"
                },
                (
                    text()
                        .class("subtle folder-color-path")
                        .content(path.clone()),
                    row().class("folder-color-palette").fill(fragment(buttons)),
                ),
                btn(
                    if directory {
                        "Clear folder color"
                    } else {
                        "Clear tag"
                    },
                    move || {
                        clear.change_explorer(|prefs| {
                            prefs.colors.remove(&path);
                        });
                        clear.close_dialog();
                    },
                ),
            )
        },
    )
}
