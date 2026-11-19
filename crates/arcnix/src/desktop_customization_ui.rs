use std::sync::Mutex;

use arcnix_core::desktop_customization::{
    self as desktop, Appearance, DesktopItems, Dock, DockSize, Shortcut, Wallpaper, WallpaperImage,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use glory::wry::shell::{AssetProvider, AssetSource};
use serde::Deserialize;

use super::*;
use crate::server_apps_worker::{Action as AppAction, Reply};

#[derive(Clone, Default)]
pub(super) struct Runtime {
    items: Option<DesktopItems>,
    items_pending: Option<u64>,
    menu: Option<Menu>,
    wallpaper_pending: Option<(u64, bool)>,
    wallpaper: Option<Arc<WallpaperAsset>>,
    wallpaper_error: String,
    server_image: String,
    local_image: String,
    settings_page: SettingsPage,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SettingsPage {
    #[default]
    General,
    Network,
    Desktop,
    Wallpaper,
    Dock,
    Shortcuts,
    About,
}

pub(super) fn settings_page(s: &State) -> SettingsPage {
    runtime(s).settings_page
}

pub(super) struct WallpaperAsset {
    url: String,
    image: WallpaperImage,
    current: AtomicBool,
}

#[derive(Default)]
pub(super) struct WallpaperAssets(Mutex<Option<Arc<WallpaperAsset>>>);

impl WallpaperAssets {
    pub(super) fn show(&self, next: Option<Arc<WallpaperAsset>>) {
        let mut current = self.0.lock().unwrap();
        if matches!((&*current, &next), (Some(old), Some(new)) if Arc::ptr_eq(old, new)) {
            return;
        }
        if let Some(old) = current.take() {
            old.current.store(false, Ordering::Release);
        }
        if let Some(new) = next.as_ref() {
            new.current.store(true, Ordering::Release);
        }
        *current = next;
    }
}

impl AssetProvider for WallpaperAssets {
    fn get(&self, path: &str) -> Option<Arc<dyn AssetSource>> {
        let asset = self.0.lock().unwrap();
        asset
            .as_ref()
            .filter(|asset| asset.url == path)
            .map(|asset| asset.clone() as Arc<dyn AssetSource>)
    }
}

impl AssetSource for WallpaperAsset {
    fn len(&self) -> u64 {
        self.image.bytes.len() as u64
    }
    fn mime(&self) -> &str {
        self.image.mime
    }
    fn current(&self) -> bool {
        self.current.load(Ordering::Acquire)
    }
    fn read_range(&self, offset: u64, length: usize) -> std::result::Result<Vec<u8>, String> {
        let offset = offset as usize;
        self.image
            .bytes
            .get(offset..offset.saturating_add(length))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "Invalid wallpaper image range".into())
    }
}

fn runtime(s: &State) -> &Runtime {
    &s.server_apps.customization
}
pub(super) fn appearance(s: &State) -> &Appearance {
    &server_apps_ui::current(s).appearance
}

pub(super) fn wallpaper_asset(s: &State) -> Option<Arc<WallpaperAsset>> {
    let Wallpaper::Remote(path) = &appearance(s).wallpaper else {
        return None;
    };
    runtime(s)
        .wallpaper
        .as_ref()
        .filter(|asset| asset.image.path == *path)
        .cloned()
}

fn update(ui: &Ui, change: impl FnOnce(&mut Appearance)) {
    if !ui.state.with(|s| {
        s.connected
            && s.server_apps.preferences_loaded
            && s.server_apps.preferences_pending.is_none()
    }) {
        return;
    }
    let mut prefs = ui.state.with(server_apps_ui::desired);
    change(&mut prefs.appearance);
    if let Err(error) = prefs.appearance.validate() {
        ui.error(error.to_string());
    } else if ui.state.with(|s| &prefs != server_apps_ui::current(s)) {
        ui.queue_desktop_preferences(prefs);
    }
}

impl Ui {
    pub(super) fn refresh_desktop_items(&self, force: bool) {
        if !self.state.with(|s| {
            s.connected
                && s.sftp_available
                && runtime(s).items_pending.is_none()
                && (force || runtime(s).items.is_none())
        }) {
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::DesktopItems));
        self.state
            .revise(|s| s.server_apps.customization.items_pending = Some(serial));
    }

    pub(super) fn close_desktop_menu(&self) {
        self.state
            .revise(|s| s.server_apps.customization.menu = None);
    }

    fn request_wallpaper(&self, action: AppAction, apply: bool) {
        if !self.state.with(|s| {
            s.connected
                && s.sftp_available
                && s.server_apps.preferences_loaded
                && s.server_apps.preferences_pending.is_none()
                && runtime(s).wallpaper_pending.is_none()
        }) {
            return;
        }
        let serial = self.send(Action::ServerApps(action));
        self.state.revise(|s| {
            s.server_apps.customization.wallpaper_pending = Some((serial, apply));
            s.server_apps.customization.wallpaper_error.clear();
        });
    }

    pub(super) fn load_desktop_wallpaper(&self) {
        if !self.state.with(server_desktop::visible) {
            return;
        }
        let path = self.state.with(|s| match &appearance(s).wallpaper {
            Wallpaper::Remote(path) if wallpaper_asset(s).is_none() => Some(path.clone()),
            _ => None,
        });
        if let Some(path) = path {
            self.request_wallpaper(AppAction::ReadWallpaper(path), false);
        }
    }

    fn open_desktop_file(&self, name: &str) {
        let entry = self.state.with(|s| find_file(s, name).cloned());
        if let Some(entry) = entry {
            if entry.is_dir {
                self.open_desktop_app(WorkspacePage::Explorer);
                file_browser::navigate(self, true, entry.path);
            } else if !entry.is_symlink {
                file_browser::open_remote_file(self, &entry.path);
            } else {
                code_workbench::open(self, &entry.path);
            }
        }
    }
}

pub(super) fn pending(ui: &Ui, serial: u64) -> bool {
    ui.state.with(|s| {
        runtime(s).items_pending == Some(serial)
            || runtime(s)
                .wallpaper_pending
                .is_some_and(|(id, _)| id == serial)
    })
}

pub(super) fn event(ui: &Ui, serial: u64, result: std::result::Result<Reply, String>) {
    if ui.state.with(|s| runtime(s).items_pending == Some(serial)) {
        ui.state.revise(|s| {
            let state = &mut s.server_apps.customization;
            state.items_pending = None;
            if let Ok(Reply::DesktopItems(items)) = result {
                state.items = Some(items);
            } else if state.items.is_none() {
                state.items = Some(DesktopItems::default());
            }
        });
        return;
    }
    let apply = ui.state.revise(|s| {
        s.server_apps
            .customization
            .wallpaper_pending
            .take()
            .is_some_and(|(_, apply)| apply)
    });
    match result {
        Ok(Reply::Wallpaper(image)) => {
            let path = image.path.clone();
            let epoch = ui.state.with(|s| s.epoch);
            let asset = Arc::new(WallpaperAsset {
                url: format!("/media/arcnix-wallpaper-{epoch}-{serial}"),
                image,
                current: AtomicBool::new(false),
            });
            ui.wallpapers.show(Some(asset.clone()));
            ui.state.revise(|s| {
                s.server_apps.customization.wallpaper = Some(asset);
                if apply {
                    s.server_apps.customization.local_image.clear();
                }
            });
            if apply {
                update(ui, |prefs| prefs.wallpaper = Wallpaper::Remote(path));
            }
        }
        Err(error) => {
            ui.state
                .revise(|s| s.server_apps.customization.wallpaper_error = error.clone());
            if apply && !ui.state.with(server_apps_ui::settings_visible) {
                ui.error(error);
            }
        }
        _ => {}
    }
}

fn find_file<'a>(s: &'a State, name: &str) -> Option<&'a FileEntry> {
    runtime(s)
        .items
        .as_ref()?
        .entries
        .iter()
        .find(|entry| entry.name == name)
}

fn desktop_icons(s: &State) -> Vec<Shortcut> {
    let settings = appearance(s);
    let mut icons = Vec::new();
    if settings.show_apps {
        icons.extend(
            server_desktop::available_apps(s)
                .into_iter()
                .chain([WorkspacePage::Video])
                .filter_map(server_desktop::app_id)
                .filter(|app| !settings.hidden.contains(app))
                .map(Shortcut::App),
        );
    }
    if let Some(items) = &runtime(s).items {
        icons.extend(
            items
                .entries
                .iter()
                .map(|entry| Shortcut::File(entry.name.clone())),
        );
    }
    ordered(icons, &settings.order)
}

fn dock_icons(s: &State) -> Vec<Shortcut> {
    let dock = &appearance(s).dock;
    let available = server_desktop::available_apps(s);
    let pinned = available
        .iter()
        .filter_map(|&page| server_desktop::app_id(page))
        .filter(|app| !dock.unpinned.contains(app))
        .collect::<Vec<_>>();
    let mut apps = ordered(pinned, &dock.order);
    apps.extend(
        available
            .into_iter()
            .filter(|page| s.workbench.apps.contains(page))
            .filter_map(server_desktop::app_id)
            .filter(|app| dock.unpinned.contains(app)),
    );
    apps.into_iter().map(Shortcut::App).collect()
}

fn ordered<T: Clone + Eq>(mut icons: Vec<T>, order: &[T]) -> Vec<T> {
    let mut sorted = Vec::with_capacity(icons.len());
    for item in order {
        if let Some(index) = icons.iter().position(|icon| icon == item) {
            sorted.push(icons.remove(index));
        }
    }
    sorted.extend(icons);
    sorted
}

pub(super) fn launchers(ui: Ui, dock: bool) -> impl Widget {
    let state = ui.state;
    each(state.map(move |s| {
        if dock {
            dock_icons(s)
        } else {
            desktop_icons(s)
        }
    }))
    .key(Clone::clone)
    .children(move |item| {
        let id = item.value.get();
        let key = serde_json::to_string(&id).expect("valid desktop item ID");
        let body = match id {
            Shortcut::App(app) => fragment(server_desktop::launcher(
                ui.clone(),
                server_desktop::app_page(app),
                dock,
            )),
            Shortcut::File(name) => {
                let open = ui.clone();
                let current = name.clone();
                fragment(
                    sapid::button()
                        .class(state.map(move |s| {
                            if find_file(s, &current).is_some_and(|entry| entry.is_dir) {
                                "desktop-shortcut desktop-file app-explorer"
                            } else {
                                "desktop-shortcut desktop-file app-document"
                            }
                            .to_owned()
                        }))
                        .name(name.clone())
                        .fill((
                            text().class("desktop-app-icon").content(""),
                            text().class("desktop-app-label").content(name.clone()),
                        ))
                        .on_press(move || open.open_desktop_file(&name)),
                )
            }
        };
        col()
            .class("desktop-icon-slot")
            .fill((text().class("desktop-item-key hidden").content(key), body))
    })
}

const BUILT_IN_WALLPAPERS: [(Wallpaper, &str, &str); 3] = [
    (Wallpaper::Terracotta, "Terracotta", "wallpaper-terracotta"),
    (Wallpaper::Aurora, "Aurora", "wallpaper-aurora"),
    (Wallpaper::Fjord, "Fjord", "wallpaper-fjord"),
];

pub(super) fn customization_css() -> String {
    use std::fmt::Write;

    let mut css = format!(
        ".wallpaper-fjord {{ background-image: url(\"data:image/webp;base64,{}\"); }}",
        STANDARD.encode(include_bytes!("../assets/fjord.webp"))
    );
    for size in Dock::MIN_ICON_SIZE..=Dock::MAX_ICON_SIZE {
        writeln!(
            css,
            ".desktop-dock.dock-size-{size} {{ --dock-ideal: {size}px; }}"
        )
        .unwrap();
    }
    css
}

pub(super) fn wallpaper(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        col().class(state.map(|s| {
            BUILT_IN_WALLPAPERS
                .iter()
                .find(|(wallpaper, ..)| wallpaper == &appearance(s).wallpaper)
                .map_or_else(
                    || "desktop-wallpaper-art".to_owned(),
                    |(_, _, class)| format!("desktop-wallpaper-art {class}"),
                )
        })),
        when(state.map(|s| wallpaper_asset(s).is_some()), move || {
            image()
                .class("desktop-wallpaper-image")
                .decorative()
                .source(state.map(|s| {
                    wallpaper_asset(s)
                        .map(|asset| asset.url.clone())
                        .unwrap_or_default()
                }))
        }),
    ))
}

pub(super) fn dock_class(s: &State) -> String {
    let dock = &appearance(s).dock;
    let size = match dock.icon_pixels() {
        pixels if pixels == DockSize::Small.pixels() => "dock-small".to_owned(),
        pixels if pixels == DockSize::Medium.pixels() => "dock-medium".to_owned(),
        pixels if pixels == DockSize::Large.pixels() => "dock-large".to_owned(),
        pixels => format!("dock-size-{pixels}"),
    };
    format!(
        "desktop-dock {size}{}{}{}{}",
        if dock.magnify { " dock-magnify" } else { "" },
        if dock.auto_hide {
            " dock-auto-hide"
        } else {
            ""
        },
        if dock.panel_mode && dock.panel_centered {
            " dock-centered"
        } else {
            ""
        },
        if floating_dock_status(s) {
            " dock-show-status"
        } else {
            ""
        }
    )
}

pub(super) fn panel_mode(s: &State) -> bool {
    appearance(s).dock.panel_mode
}

pub(super) fn floating_dock_status(s: &State) -> bool {
    let dock = &appearance(s).dock;
    !dock.panel_mode && dock.floating_status
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Zone {
    Desktop,
    Dock,
}

#[derive(Clone)]
struct Menu {
    zone: Zone,
    item: Option<Shortcut>,
    x: i32,
    y: i32,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Gesture {
    Context {
        epoch: u64,
        zone: Zone,
        item: Option<Shortcut>,
        x: i32,
        y: i32,
    },
    Dismiss {
        epoch: u64,
    },
    Reorder {
        epoch: u64,
        zone: Zone,
        item: Shortcut,
        anchor: Shortcut,
        after: bool,
    },
}

fn icons(s: &State, zone: Zone) -> Vec<Shortcut> {
    match zone {
        Zone::Desktop => desktop_icons(s),
        Zone::Dock => dock_icons(s),
    }
}

fn gesture(ui: &Ui, value: &str) {
    if value.len() > 4096 {
        return;
    }
    let Ok(gesture) = serde_json::from_str(value) else {
        return;
    };
    let epoch = match &gesture {
        Gesture::Context { epoch, .. }
        | Gesture::Dismiss { epoch }
        | Gesture::Reorder { epoch, .. } => *epoch,
    };
    if !ui
        .state
        .with(|s| s.epoch == epoch && server_desktop::visible(s) && s.dialog.is_none())
    {
        return;
    }
    match gesture {
        Gesture::Dismiss { .. } => ui.close_desktop_menu(),
        Gesture::Context {
            zone, item, x, y, ..
        } => ui.state.revise(|s| {
            let item = item.filter(|item| icons(s, zone).contains(item));
            s.server_apps.customization.menu = Some(Menu { zone, item, x, y });
        }),
        Gesture::Reorder {
            zone,
            item,
            anchor,
            after,
            ..
        } => {
            if item == anchor {
                return;
            }
            let mut icons = ui.state.with(|s| icons(s, zone));
            let Some(from) = icons.iter().position(|icon| *icon == item) else {
                return;
            };
            icons.remove(from);
            let Some(at) = icons.iter().position(|icon| *icon == anchor) else {
                return;
            };
            icons.insert(at + usize::from(after), item.clone());
            update(ui, |prefs| match zone {
                Zone::Desktop => {
                    let old = std::mem::take(&mut prefs.order);
                    let others = old
                        .into_iter()
                        .filter(|id| !icons.contains(id))
                        .collect::<Vec<_>>();
                    let remaining = desktop::MAX_ICONS - icons.len();
                    icons.extend(others.into_iter().take(remaining));
                    prefs.order = icons;
                }
                Zone::Dock => {
                    prefs.dock.order = icons
                        .into_iter()
                        .filter_map(|icon| match icon {
                            Shortcut::App(app) => Some(app),
                            Shortcut::File(_) => None,
                        })
                        .collect();
                    if let Shortcut::App(app) = item {
                        prefs.dock.unpinned.retain(|id| *id != app);
                    }
                }
            });
        }
    }
}

pub(super) fn gesture_input(ui: Ui) -> impl Widget {
    input()
        .class("desktop-customization-input hidden")
        .on(value_changed, move |event| {
            if let Event::ValueChanged { value, .. } = event {
                gesture(&ui, &value);
            }
        })
}

fn menu_action<F: FnMut(&Ui) + 'static>(
    ui: &Ui,
    label: &'static str,
    mut action: F,
) -> impl Widget + use<F> {
    let ui = ui.clone();
    sapid::button()
        .class("menu-item")
        .label(i18n::bind(label))
        .on_press(move || {
            ui.close_desktop_menu();
            action(&ui);
        })
}

pub(super) fn open_settings(ui: &Ui, page: SettingsPage) {
    ui.state.revise(|s| {
        s.server_apps.customization.settings_page = page;
        s.server_apps.customization.menu = None;
    });
    ui.open_desktop_app(WorkspacePage::Settings);
}

pub(super) fn context_menu(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state
            .map(|s| runtime(s).menu.is_some() && server_desktop::visible(s) && s.dialog.is_none()),
        move || {
            let close = ui.clone();
            let coordinates = text()
                .class("desktop-menu-position hidden")
                .content(state.map(|s| {
                    runtime(s)
                        .menu
                        .as_ref()
                        .map(|menu| format!("{},{}", menu.x, menu.y))
                        .unwrap_or_default()
                }));
            let entries = each(state.map(|s| {
                runtime(s)
                    .menu
                    .as_ref()
                    .map(|menu| (menu.zone, menu.item.clone()))
                    .into_iter()
                    .collect::<Vec<_>>()
            }))
            .key(|(zone, item)| serde_json::to_string(&(matches!(zone, Zone::Dock), item)).unwrap())
            .children({
                let ui = ui.clone();
                move |key| {
                    let (zone, item) = key.value.get();
                    let dock = zone == Zone::Dock;
                    let app = match item {
                        Some(Shortcut::App(app)) => Some(app),
                        _ => None,
                    };
                    let file = match item {
                        Some(Shortcut::File(name)) => Some(name),
                        _ => None,
                    };
                    let (directory, wallpaper) = file
                        .as_ref()
                        .and_then(|name| {
                            state.with(|s| {
                                find_file(s, name).map(|entry| {
                                    (
                                        (entry.is_dir
                                            && !entry.is_symlink
                                            && terminal::directory_supported(&entry.path))
                                        .then(|| entry.path.clone()),
                                        (!entry.is_dir && desktop::is_wallpaper(&entry.path))
                                            .then(|| entry.path.clone()),
                                    )
                                })
                            })
                        })
                        .unwrap_or_default();
                    let unpinned = app.is_some_and(|app| {
                        state.with(|s| appearance(s).dock.unpinned.contains(&app))
                    });
                    let can_pin = app.is_some_and(|app| server_desktop::app_page(app) != WorkspacePage::Video);
                    fragment((
                        when(!dock && app.is_some(), {
                            let ui = ui.clone();
                            move || {
                                menu_action(&ui, "Hide desktop shortcut", move |ui| {
                                    if let Some(app) = app {
                                        update(ui, |prefs| {
                                            if !prefs.hidden.contains(&app) {
                                                prefs.hidden.push(app);
                                            }
                                        });
                                    }
                                })
                            }
                        }),
                        when(can_pin, {
                            let ui = ui.clone();
                            move || {
                                menu_action(
                                    &ui,
                                    if unpinned {
                                        "Keep in Dock"
                                    } else {
                                        "Remove from Dock"
                                    },
                                    move |ui| {
                                        if let Some(app) = app {
                                            update(ui, |prefs| {
                                                if prefs.dock.unpinned.contains(&app) {
                                                    prefs.dock.unpinned.retain(|id| *id != app);
                                                } else {
                                                    prefs.dock.unpinned.push(app);
                                                }
                                            });
                                        }
                                    },
                                )
                            }
                        }),
                        when(directory.is_some(), {
                            let ui = ui.clone();
                            move || {
                                let path = directory.clone();
                                menu_action(&ui, "在此处打开终端", move |ui| {
                                    if let Some(path) = &path {
                                        ui.terminal_here(path.clone());
                                    }
                                })
                            }
                        }),
                        when(wallpaper.is_some(), {
                            let ui = ui.clone();
                            move || {
                                let path = wallpaper.clone().unwrap_or_default();
                                menu_action(&ui, "Use as wallpaper", move |ui| {
                                    ui.request_wallpaper(
                                        AppAction::ReadWallpaper(path.clone()),
                                        true,
                                    )
                                })
                            }
                        }),
                        when(!dock, {
                            let ui = ui.clone();
                            move || {
                                fragment((
                                    menu_action(&ui, "Desktop settings", |ui| {
                                        open_settings(ui, SettingsPage::Desktop)
                                    }),
                                    menu_action(&ui, "Refresh desktop", |ui| {
                                        ui.refresh_desktop_items(true)
                                    }),
                                    when(
                                        state.map(|s| {
                                            runtime(s)
                                                .items
                                                .as_ref()
                                                .is_some_and(|items| items.exists)
                                        }),
                                        {
                                            let ui = ui.clone();
                                            move || {
                                                menu_action(&ui, "Open Desktop folder", move |ui| {
                                                    if let Some(path) = ui.state.with(|s| {
                                                        runtime(s)
                                                            .items
                                                            .as_ref()
                                                            .map(|items| items.path.clone())
                                                    }) {
                                                        ui.open_desktop_app(
                                                            WorkspacePage::Explorer,
                                                        );
                                                        file_browser::navigate(ui, true, path);
                                                    }
                                                })
                                            }
                                        },
                                    ),
                                ))
                            }
                        }),
                        when(dock, {
                            let ui = ui.clone();
                            move || {
                                menu_action(&ui, "Dock settings", |ui| {
                                    open_settings(ui, SettingsPage::Dock)
                                })
                            }
                        }),
                    ))
                }
            });
            fragment((
                sapid::button()
                    .class("desktop-menu-backdrop")
                    .name(i18n::bind("Close menu"))
                    .on_press(move || close.close_desktop_menu()),
                col()
                    .class("pane-menu desktop-context-menu")
                    .fill((coordinates, entries)),
            ))
        },
    )
}

pub(super) fn settings_tabs(ui: Ui) -> impl Widget {
    col()
        .class("desktop-settings-tabs")
        .name(i18n::bind("Settings sections"))
        .fill(fragment(
            [
                (SettingsPage::General, "General", Icon::Settings),
                (SettingsPage::Network, "Network", Icon::Server),
                (SettingsPage::Desktop, "Desktop", Icon::Grid),
                (SettingsPage::Wallpaper, "Wallpaper", Icon::File),
                (SettingsPage::Dock, "Dock", Icon::List),
                (
                    SettingsPage::Shortcuts,
                    "Keyboard shortcuts",
                    Icon::Terminal,
                ),
                (SettingsPage::About, "About", Icon::Info),
            ]
            .into_iter()
            .map(|(page, label, icon)| {
                let choose = ui.clone();
                let state = ui.state;
                sapid::button()
                    .class("desktop-settings-section")
                    .name(i18n::bind(label))
                    .fill((icons::glyph(icon), text().content(i18n::bind(label))))
                    .pressed(state.map(move |s| settings_page(s) == page))
                    .on_press(move || {
                        state.revise(|s| s.server_apps.customization.settings_page = page);
                        network_ui::enter(&choose);
                    })
            })
            .collect::<Vec<_>>(),
        ))
}

fn toggle(
    ui: &Ui,
    label: &'static str,
    selected: fn(&Appearance) -> bool,
    change: fn(&mut Appearance),
) -> impl Widget {
    let ui = ui.clone();
    sapid::button()
        .class("button")
        .label(i18n::bind(label))
        .pressed(ui.state.map(move |s| selected(appearance(s))))
        .enabled(ui.state.map(server_apps_ui::preferences_ready))
        .on_press(move || update(&ui, change))
}

pub(super) fn desktop_controls(ui: Ui) -> impl Widget {
    row().class("choice-list").fill((
        toggle(
            &ui,
            "Show server info",
            |p| p.show_server,
            |p| p.show_server = !p.show_server,
        ),
        toggle(
            &ui,
            "Show Arcnix shortcuts",
            |p| p.show_apps,
            |p| p.show_apps = !p.show_apps,
        ),
        text_action("Reset desktop icons", Icon::Refresh)
            .enabled(ui.state.map(server_apps_ui::preferences_ready))
            .on_press(move || {
                update(&ui, |prefs| {
                    prefs.order.clear();
                    prefs.hidden.clear();
                    prefs.show_apps = true;
                });
            }),
    ))
}

pub(super) fn dock_controls(ui: Ui) -> impl Widget {
    let alignment = ui.clone();
    let status = ui.clone();
    let reset = ui.clone();
    let resize_dock = ui.clone();
    let state = ui.state;
    col().class("desktop-settings-pane").fill((
        col().class("desktop-settings-card").fill((
            text()
                .class("section-title")
                .content(i18n::bind("Dock style")),
            row().class("choice-list").fill(fragment(
                [(false, "Floating Dock"), (true, "Panel mode")]
                    .into_iter()
                    .map(|(panel, label)| {
                        let choose = ui.clone();
                        sapid::button()
                            .class("button")
                            .label(i18n::bind(label))
                            .pressed(state.map(move |s| panel_mode(s) == panel))
                            .enabled(state.map(server_apps_ui::preferences_ready))
                            .on_press(move || {
                                update(&choose, |prefs| prefs.dock.panel_mode = panel)
                            })
                    })
                    .collect::<Vec<_>>(),
            )),
            when(state.map(panel_mode), move || {
                fragment((
                    text()
                        .class("field-label")
                        .content(i18n::bind("App icon alignment")),
                    row()
                        .class("choice-list")
                        .name(i18n::bind("App icon alignment"))
                        .fill(fragment(
                            [(false, "Align left"), (true, "Center icons")]
                                .into_iter()
                                .map(|(centered, label)| {
                                    let choose = alignment.clone();
                                    sapid::button()
                                        .class("button")
                                        .label(i18n::bind(label))
                                        .pressed(state.map(move |s| {
                                            appearance(s).dock.panel_centered == centered
                                        }))
                                        .enabled(state.map(server_apps_ui::preferences_ready))
                                        .on_press(move || {
                                            update(&choose, |prefs| {
                                                prefs.dock.panel_centered = centered
                                            })
                                        })
                                })
                                .collect::<Vec<_>>(),
                        )),
                ))
            }),
            when(state.map(|s| !panel_mode(s)), move || {
                row().class("choice-list").fill(toggle(
                    &status,
                    "Show Dock status bar",
                    |p| p.dock.floating_status,
                    |p| p.dock.floating_status = !p.dock.floating_status,
                ))
            }),
        )),
        col().class("desktop-settings-card").fill((
            text()
                .class("section-title")
                .content(i18n::bind("Dock icon size")),
            row().class("dock-size-control").fill((
                input()
                    .class("dock-size-bridge hidden")
                    .name(i18n::bind("Dock icon size"))
                    .enabled(state.map(server_apps_ui::preferences_ready))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event
                            && let Ok(pixels) = value.parse::<u8>()
                            && (Dock::MIN_ICON_SIZE..=Dock::MAX_ICON_SIZE).contains(&pixels)
                        {
                            update(&resize_dock, |prefs| prefs.dock.set_icon_pixels(pixels));
                        }
                    }),
                text()
                    .class("dock-size-state hidden")
                    .content(state.map(|s| {
                        format!(
                            "{},{},{}",
                            Dock::MIN_ICON_SIZE,
                            Dock::MAX_ICON_SIZE,
                            appearance(s).dock.icon_pixels()
                        )
                    })),
            )),
            row().class("choice-list").fill(fragment(
                [
                    (DockSize::Small, "Small"),
                    (DockSize::Medium, "Medium"),
                    (DockSize::Large, "Large"),
                ]
                .into_iter()
                .map(|(size, label)| {
                    let choose = ui.clone();
                    sapid::button()
                        .class("button")
                        .label(i18n::bind(label))
                        .pressed(
                            state.map(move |s| appearance(s).dock.icon_pixels() == size.pixels()),
                        )
                        .enabled(state.map(server_apps_ui::preferences_ready))
                        .on_press(move || {
                            update(&choose, |prefs| prefs.dock.set_icon_pixels(size.pixels()))
                        })
                })
                .collect::<Vec<_>>(),
            )),
        )),
        col().class("desktop-settings-card").fill((
            text()
                .class("section-title")
                .content(i18n::bind("Dock behavior")),
            row().class("choice-list").fill((
                toggle(
                    &ui,
                    "Magnify Dock icons",
                    |p| p.dock.magnify,
                    |p| p.dock.magnify = !p.dock.magnify,
                ),
                toggle(
                    &ui,
                    "Automatically hide Dock",
                    |p| p.dock.auto_hide,
                    |p| p.dock.auto_hide = !p.dock.auto_hide,
                ),
                text_action("Reset Dock", Icon::Refresh)
                    .enabled(state.map(server_apps_ui::preferences_ready))
                    .on_press(move || update(&reset, |prefs| prefs.dock = Dock::default())),
            )),
        )),
    ))
}

pub(super) fn wallpaper_controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    let server = ui.clone();
    let upload = ui.clone();
    let browse = ui.clone();
    let ready = state
        .map(|s| server_apps_ui::preferences_ready(s) && runtime(s).wallpaper_pending.is_none());
    let transfer = state.map(|s| {
        server_apps_ui::preferences_ready(s)
            && s.sftp_available
            && runtime(s).wallpaper_pending.is_none()
    });
    col().class("desktop-settings-pane").fill((
      col().class("desktop-settings-card").fill((
        text().class("section-title").content(i18n::bind("Built-in wallpapers")),
        row().class("wallpaper-choices").fill(fragment(BUILT_IN_WALLPAPERS.into_iter().map(|(wallpaper, label, class)| {
            let choose = ui.clone();
            let selected = wallpaper.clone();
            sapid::button().class("button wallpaper-choice").name(i18n::bind(label))
                .pressed(state.map(move |s| appearance(s).wallpaper == selected)).enabled(ready)
                .fill((text().class(format!("wallpaper-swatch {class}")).content(""), text().content(i18n::bind(label))))
                .on_press(move || {
                    choose.state.revise(|s| s.server_apps.customization.wallpaper_error.clear());
                    update(&choose, |prefs| prefs.wallpaper = wallpaper.clone());
                })
        }).collect::<Vec<_>>())),
      )),
      col().class("desktop-settings-card").fill((
        row().class("section-heading").fill((
            text().class("section-title").content(i18n::bind("Custom wallpaper")),
            help_ui::tip("Custom wallpaper", "PNG, JPEG, WebP, GIF, BMP, AVIF or SVG, up to 8 MiB. Local images are uploaded to ~/.arcnix/wallpapers."),
        )),
        row().class("wallpaper-input-row").fill((
            text().class("wallpaper-input-label").content(i18n::bind("Server image path")),
            input().name(i18n::bind("Server image path"))
                .value(state.map(|s| runtime(s).server_image.clone())).enabled(transfer)
                .on(value_changed, move |event| { if let Event::ValueChanged { value, .. } = event { state.revise(|s| s.server_apps.customization.server_image = value); } }),
            row().class("wallpaper-input-actions").fill(
                btn("Apply", move || {
                    let path = state.with(|s| runtime(s).server_image.trim().to_owned());
                    server.request_wallpaper(AppAction::ReadWallpaper(path), true);
                }).enabled(state.map(move |s| transfer.get() && !runtime(s).server_image.trim().is_empty())),
            ),
        )),
        row().class("wallpaper-input-row").fill((
            text().class("wallpaper-input-label").content(i18n::bind("Local image path")),
            input().name(i18n::bind("Local image path"))
                .value(state.map(|s| runtime(s).local_image.clone())).enabled(transfer)
                .on(value_changed, move |event| { if let Event::ValueChanged { value, .. } = event { state.revise(|s| s.server_apps.customization.local_image = value); } }),
            row().class("wallpaper-input-actions").fill((
              btn("Upload & apply", move || {
                let path = state.with(|s| PathBuf::from(runtime(s).local_image.trim()));
                upload.request_wallpaper(AppAction::ImportWallpaper(path), true);
            }).enabled(state.map(move |s| transfer.get() && !runtime(s).local_image.trim().is_empty())),
            when(cfg!(windows), move || btn("Browse…", {
                let browse = browse.clone();
                move || {
                    let current = state.with(|s| {
                        let path = runtime(s).local_image.trim();
                        PathBuf::from(if path.is_empty() { &s.local_path } else { path })
                    });
                    match file_picker::private_key(Some(&current), &i18n::translate("Choose wallpaper"), &i18n::translate("Select file"), &i18n::translate("All files")) {
                        Ok(Some(path)) => browse.request_wallpaper(AppAction::ImportWallpaper(path), true),
                        Ok(None) => {},
                        Err(error) => state.revise(|s| s.server_apps.customization.wallpaper_error = error.to_string()),
                    }
                }
              }).enabled(transfer)),
            )),
        )),
        when(state.map(|s| runtime(s).wallpaper_pending.is_some()), || text().class("subtle").content(i18n::bind("Loading wallpaper…"))),
        when(state.map(|s| !runtime(s).wallpaper_error.is_empty()), move || text().class("error").content(i18n::bind(state.map(|s| runtime(s).wallpaper_error.clone())))),
      )),
    ))
}
