use std::ops::RangeInclusive;
use std::sync::Mutex;

use arcnix_core::desktop_customization::{
    self as desktop, Appearance, DesktopIconSize, DesktopItems, Dock, DockItem, DockPosition,
    DockSize, IconDirection, IconFlow, Layout, Shortcut, SystemIcon, Wallpaper, WallpaperImage,
};
use arcnix_core::server_apps::DesktopApp;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use glory::wry::shell::{AssetProvider, AssetSource};
use serde::Deserialize;

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{btn, text_action};
use crate::server_apps_worker::{Action as AppAction, Reply};
use crate::*;

#[derive(Clone, Default)]
pub(crate) struct Runtime {
    items: Option<DesktopItems>,
    items_pending: Option<u64>,
    menu: Option<Menu>,
    wallpaper_pending: Option<(u64, WallpaperTarget, Option<String>)>,
    wallpapers: Vec<Arc<WallpaperAsset>>,
    wallpaper_failures: Vec<String>,
    wallpaper_error: String,
    server_image: String,
    local_image: String,
    settings_page: SettingsPage,
}

#[derive(Clone, Copy)]
enum WallpaperTarget {
    Cache,
    Draft,
    Active,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    #[default]
    General,
    Network,
    DefaultApps,
    Desktop,
    Dock,
    Appearance,
    Region,
    DateTime,
    Shortcuts,
    About,
}

pub(crate) fn settings_page(s: &State) -> SettingsPage {
    runtime(s).settings_page
}

pub(crate) struct WallpaperAsset {
    url: String,
    image: WallpaperImage,
    current: AtomicBool,
}

#[derive(Default)]
pub(crate) struct WallpaperAssets(Mutex<Vec<Arc<WallpaperAsset>>>);

impl WallpaperAssets {
    pub(crate) fn show(&self, next: Vec<Arc<WallpaperAsset>>) {
        let mut current = self.0.lock().unwrap();
        for old in &*current {
            if !next.iter().any(|asset| Arc::ptr_eq(old, asset)) {
                old.current.store(false, Ordering::Release);
            }
        }
        for new in &next {
            new.current.store(true, Ordering::Release);
        }
        *current = next;
    }
}

impl AssetProvider for WallpaperAssets {
    fn get(&self, path: &str) -> Option<Arc<dyn AssetSource>> {
        let asset = self.0.lock().unwrap();
        asset
            .iter()
            .find(|asset| asset.url == path)
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
pub(crate) fn appearance(s: &State) -> &Appearance {
    &server_apps_ui::current(s).appearance
}

pub(crate) fn wallpaper_assets(s: &State) -> Vec<Arc<WallpaperAsset>> {
    runtime(s).wallpapers.clone()
}

fn asset_for(s: &State, wallpaper: &Wallpaper) -> Option<Arc<WallpaperAsset>> {
    let Wallpaper::Remote(path) = wallpaper else {
        return None;
    };
    runtime(s)
        .wallpapers
        .iter()
        .find(|asset| asset.image.path == *path)
        .cloned()
}

pub(crate) fn pending_wallpaper(s: &State) -> bool {
    runtime(s).wallpaper_pending.is_some()
}

pub(crate) fn update(ui: &Ui, change: impl FnOnce(&mut Appearance)) {
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
    pub(crate) fn refresh_desktop_items(&self, force: bool) {
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

    pub(crate) fn close_desktop_menu(&self) {
        self.state
            .revise(|s| s.server_apps.customization.menu = None);
    }

    fn request_wallpaper(&self, action: AppAction, target: WallpaperTarget) {
        if !self.state.with(|s| {
            s.connected
                && s.sftp_available
                && s.server_apps.preferences_loaded
                && s.server_apps.preferences_pending.is_none()
                && runtime(s).wallpaper_pending.is_none()
        }) {
            return;
        }
        let path = match &action {
            AppAction::ReadWallpaper(path) => Some(path.clone()),
            _ => None,
        };
        let serial = self.send(Action::ServerApps(action));
        self.state.revise(|s| {
            s.server_apps.customization.wallpaper_pending = Some((serial, target, path));
            s.server_apps.customization.wallpaper_error.clear();
        });
    }

    pub(crate) fn load_desktop_wallpaper(&self) {
        if !self.state.with(server_desktop::visible) {
            return;
        }
        let path = self.state.with(|s| {
            let theme = desktop_appearance_ui::theme(s);
            let dark = desktop_appearance_ui::dark(s);
            [theme.background(dark), theme.background(!dark)]
                .into_iter()
                .find_map(|wallpaper| {
                    if let Wallpaper::Remote(path) = wallpaper
                        && asset_for(s, wallpaper).is_none()
                        && !runtime(s).wallpaper_failures.contains(path)
                    {
                        Some(path.clone())
                    } else {
                        None
                    }
                })
        });
        if let Some(path) = path {
            self.request_wallpaper(AppAction::ReadWallpaper(path), WallpaperTarget::Cache);
        }
    }

    fn open_desktop_file(&self, name: &str) {
        let entry = self.state.with(|s| find_file(s, name).cloned());
        if let Some(entry) = entry {
            if entry.is_symlink {
                file_actions::open_alias(self, &entry.path);
            } else if entry.is_dir {
                self.open_desktop_app(WorkspacePage::Files(0));
                file_browser::navigate(self, true, entry.path);
            } else {
                file_browser::open_remote_file(self, &entry.path);
            }
        }
    }
}

pub(crate) fn pending(ui: &Ui, serial: u64) -> bool {
    ui.state.with(|s| {
        runtime(s).items_pending == Some(serial)
            || runtime(s)
                .wallpaper_pending
                .as_ref()
                .is_some_and(|(id, ..)| *id == serial)
    })
}

pub(crate) fn event(ui: &Ui, serial: u64, result: std::result::Result<Reply, String>) {
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
    let Some((_, target, requested)) = ui
        .state
        .revise(|s| s.server_apps.customization.wallpaper_pending.take())
    else {
        return;
    };
    match result {
        Ok(Reply::Wallpaper(image)) => {
            let path = image.path.clone();
            let epoch = ui.state.with(|s| s.epoch);
            let asset = Arc::new(WallpaperAsset {
                url: format!("/media/arcnix-wallpaper-{epoch}-{serial}"),
                image,
                current: AtomicBool::new(false),
            });
            let mut assets = ui.state.with(wallpaper_assets);
            assets.retain(|previous| previous.image.path != path);
            assets.insert(0, asset);
            assets.truncate(6);
            ui.wallpapers.show(assets.clone());
            ui.state.revise(|s| {
                let runtime = &mut s.server_apps.customization;
                runtime.wallpapers = assets;
                runtime.wallpaper_failures.retain(|failed| failed != &path);
                if !matches!(target, WallpaperTarget::Cache) {
                    runtime.local_image.clear();
                }
            });
            match target {
                WallpaperTarget::Draft => {
                    desktop_appearance_ui::set_background(ui, Wallpaper::Remote(path))
                }
                WallpaperTarget::Active => {
                    desktop_appearance_ui::set_active_background(ui, Wallpaper::Remote(path))
                }
                WallpaperTarget::Cache => {}
            }
        }
        Err(error) => {
            ui.state.revise(|s| {
                let runtime = &mut s.server_apps.customization;
                runtime.wallpaper_error = error.clone();
                if let Some(path) = requested {
                    runtime.wallpaper_failures.push(path);
                }
            });
            if !matches!(target, WallpaperTarget::Cache)
                && !ui.state.with(server_apps_ui::settings_visible)
            {
                ui.error(error);
            }
        }
        _ => {}
    }
    ui.load_desktop_wallpaper();
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
    let mut icons: Vec<_> = server_desktop::available_apps(s)
        .into_iter()
        .filter_map(server_desktop::app_id)
        .filter(|&app| shortcut_visible(settings, app))
        .map(Shortcut::App)
        .collect();
    if s.sftp_available {
        if settings.layout.show_home {
            icons.insert(0, Shortcut::System(SystemIcon::Home));
        }
        if settings.layout.show_trash {
            icons.push(Shortcut::System(SystemIcon::Trash));
        }
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

fn shortcut_visible(prefs: &Appearance, app: DesktopApp) -> bool {
    prefs.show_apps && !prefs.hidden.contains(&app)
}

fn set_shortcut_visible(ui: &Ui, app: DesktopApp, visible: bool) {
    update(ui, |prefs| {
        if shortcut_visible(prefs, app) == visible {
            return;
        }
        if !prefs.show_apps {
            // Preserve the old all-hidden setting as individual launcher choices.
            for app in server_desktop::APP_LAUNCHERS
                .into_iter()
                .filter_map(server_desktop::app_id)
            {
                if !prefs.hidden.contains(&app) {
                    prefs.hidden.push(app);
                }
            }
            prefs.show_apps = true;
        }
        if visible {
            prefs.hidden.retain(|&hidden| hidden != app);
        } else {
            prefs.hidden.push(app);
        }
    });
}

fn shortcut_choices(apps: impl IntoIterator<Item = WorkspacePage>) -> Vec<DockItem> {
    apps.into_iter()
        .filter_map(server_desktop::app_id)
        .map(DockItem::App)
        .chain([SystemIcon::Home, SystemIcon::Trash].map(|system| DockItem::System { system }))
        .collect()
}

fn dock_choices(s: &State) -> Vec<DockItem> {
    ordered(
        shortcut_choices(server_desktop::available_apps(s)),
        &appearance(s).dock.order,
    )
}

fn dock_icons(s: &State) -> Vec<Shortcut> {
    let dock = &appearance(s).dock;
    let order = dock_choices(s);
    order
        .iter()
        .copied()
        .filter(|&item| {
            dock_pinned(dock, item) && (s.sftp_available || matches!(item, DockItem::App(_)))
        })
        .chain(order.iter().copied().filter(|item| match *item {
            DockItem::App(app) => {
                !dock_pinned(dock, *item)
                    && server_desktop::launcher_running(s, server_desktop::app_page(app))
            }
            DockItem::System { .. } => false,
        }))
        .map(Shortcut::from)
        .collect()
}

fn dock_pinned_by_default(app: DesktopApp) -> bool {
    !matches!(app, DesktopApp::Player | DesktopApp::Editor)
}

fn dock_pinned(dock: &Dock, item: DockItem) -> bool {
    match item {
        DockItem::App(app) => {
            !dock.unpinned.contains(&app)
                && (dock_pinned_by_default(app) || dock.order.contains(&item))
        }
        DockItem::System { .. } => dock.order.contains(&item),
    }
}

fn toggle_dock_pin(ui: &Ui, item: DockItem) {
    update(ui, |prefs| {
        let dock = &mut prefs.dock;
        let pinned = dock_pinned(dock, item);
        match item {
            DockItem::App(app) if dock_pinned_by_default(app) => {
                if pinned {
                    dock.unpinned.push(app);
                } else {
                    dock.unpinned.retain(|&id| id != app);
                }
            }
            _ if pinned => dock.order.retain(|&id| id != item),
            _ => {
                if let DockItem::App(app) = item {
                    dock.unpinned.retain(|&id| id != app);
                }
                for candidate in shortcut_choices(server_desktop::APP_LAUNCHERS) {
                    if (candidate == item || dock_pinned(dock, candidate))
                        && !dock.order.contains(&candidate)
                    {
                        dock.order.push(candidate);
                    }
                }
            }
        }
    });
}

fn system_icon(icon: SystemIcon) -> (&'static str, Icon, &'static str) {
    match icon {
        SystemIcon::Home => ("主目录", Icon::Home, "app-home"),
        SystemIcon::Trash => ("Trash", Icon::Clear, "app-trash"),
    }
}

fn system_visible(prefs: &Appearance, icon: SystemIcon) -> bool {
    match icon {
        SystemIcon::Home => prefs.layout.show_home,
        SystemIcon::Trash => prefs.layout.show_trash,
    }
}

fn set_system_visible(ui: &Ui, icon: SystemIcon, visible: bool) {
    update(ui, |prefs| match icon {
        SystemIcon::Home => prefs.layout.show_home = visible,
        SystemIcon::Trash => prefs.layout.show_trash = visible,
    });
}

fn open_system_icon(ui: &Ui, icon: SystemIcon) {
    let home = ui.state.with(|s| {
        runtime(s)
            .items
            .as_ref()
            .map(|items| items.home.clone())
            .filter(|home| !home.is_empty())
            .or_else(|| {
                s.server_apps
                    .files_locations
                    .as_ref()?
                    .iter()
                    .find(|location| location.name == "Home")
                    .map(|location| location.path.clone())
            })
    });
    ui.open_desktop_app(WorkspacePage::Files(0));
    match icon {
        SystemIcon::Home => {
            if let Some(path) = home {
                file_browser::navigate(ui, true, path);
            }
        }
        SystemIcon::Trash => trash_ui::open(ui, true),
    }
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

pub(crate) fn launchers(ui: Ui, dock: bool) -> impl Widget {
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
            Shortcut::System(icon) => {
                let (label, _, tile) = system_icon(icon);
                let class = if dock {
                    "desktop-dock-app"
                } else {
                    "desktop-shortcut"
                };
                let open = ui.clone();
                fragment(
                    sapid::button()
                        .class(format!("{class} {tile}"))
                        .name(i18n::bind(label))
                        .fill((
                            text().class("desktop-app-icon").content(""),
                            text().class("desktop-app-label").content(i18n::bind(label)),
                        ))
                        .on_press(move || open_system_icon(&open, icon)),
                )
            }
            Shortcut::File(name) => {
                let open = ui.clone();
                let current = name.clone();
                let appearance = name.clone();
                fragment(
                    sapid::button()
                        .class(state.map(move |s| {
                            if find_file(s, &current).is_some_and(|entry| entry.is_dir) {
                                "desktop-shortcut desktop-file app-files"
                            } else {
                                "desktop-shortcut desktop-file app-document"
                            }
                            .to_owned()
                        }))
                        .name(name.clone())
                        .fill((
                            text()
                                .class(state.map(move |s| {
                                    let mut class = "desktop-app-icon".to_owned();
                                    if find_file(s, &appearance)
                                        .is_some_and(|entry| entry.is_symlink)
                                    {
                                        class.push_str(" desktop-alias-mark");
                                    }
                                    if let Some(entry) =
                                        find_file(s, &appearance).filter(|entry| !entry.is_symlink)
                                    {
                                        if let Some(tag) = files_ui::tag_class(s, &entry.path) {
                                            class.push_str(&format!(" desktop-tagged {tag}"));
                                        }
                                        if entry.is_dir
                                            && let Some(icon) =
                                                files_ui::folder_icon_class(s, &entry.path)
                                        {
                                            class.push_str(&format!(" folder-custom-mark {icon}"));
                                        }
                                    }
                                    class
                                }))
                                .content(""),
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
    (Wallpaper::Fjord, "Fjord", "wallpaper-fjord"),
    (Wallpaper::Aurora, "Aurora", "wallpaper-aurora"),
    (Wallpaper::Terracotta, "Terracotta", "wallpaper-terracotta"),
];

pub(crate) fn customization_css() -> String {
    use std::fmt::Write;

    let mut css = format!(
        ".wallpaper-fjord {{ background-image: url(\"data:image/webp;base64,{}\"); }}",
        STANDARD.encode(include_bytes!("../../assets/fjord.webp"))
    );
    for size in Dock::MIN_ICON_SIZE..=Dock::MAX_ICON_SIZE {
        writeln!(
            css,
            ".desktop-dock.dock-size-{size} {{ --dock-ideal: {size}px; }}"
        )
        .unwrap();
    }
    for size in Layout::MIN_ICON_SIZE..=Layout::MAX_ICON_SIZE {
        writeln!(
            css,
            ".desktop-shortcuts.icons-size-{size} {{ --desktop-icon-ideal: {size}px; }}"
        )
        .unwrap();
    }
    css
}

pub(crate) fn wallpaper(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        col().class(state.map(|s| {
            BUILT_IN_WALLPAPERS
                .iter()
                .find(|(wallpaper, ..)| wallpaper == &desktop_appearance_ui::background(s))
                .map_or_else(
                    || "desktop-wallpaper-art".to_owned(),
                    |(_, _, class)| format!("desktop-wallpaper-art {class}"),
                )
        })),
        when(
            state.map(|s| asset_for(s, &desktop_appearance_ui::background(s)).is_some()),
            move || {
                image()
                    .class("desktop-wallpaper-image")
                    .decorative()
                    .source(state.map(|s| {
                        asset_for(s, &desktop_appearance_ui::background(s))
                            .map(|asset| asset.url.clone())
                            .unwrap_or_default()
                    }))
            },
        ),
    ))
}

pub(crate) fn wallpaper_preview(
    state: Cage<State>,
    read: impl Fn(&State) -> Wallpaper + Copy + 'static,
) -> impl Widget {
    col()
        .class(state.map(move |s| {
            BUILT_IN_WALLPAPERS
                .iter()
                .find(|(wallpaper, ..)| *wallpaper == read(s))
                .map_or_else(
                    || "theme-wallpaper-swatch".to_owned(),
                    |(_, _, class)| format!("theme-wallpaper-swatch {class}"),
                )
        }))
        .fill(when(
            state.map(move |s| asset_for(s, &read(s)).is_some()),
            move || {
                image().decorative().source(state.map(move |s| {
                    asset_for(s, &read(s)).map_or_else(String::new, |asset| asset.url.clone())
                }))
            },
        ))
}

pub(crate) fn dock_class(s: &State) -> String {
    let dock = &appearance(s).dock;
    let size = match dock.icon_pixels() {
        pixels if pixels == DockSize::Small.pixels() => "dock-small".to_owned(),
        pixels if pixels == DockSize::Medium.pixels() => "dock-medium".to_owned(),
        pixels if pixels == DockSize::Large.pixels() => "dock-large".to_owned(),
        pixels => format!("dock-size-{pixels}"),
    };
    format!(
        "desktop-dock {size}{}{}{}",
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

pub(crate) fn dock_zone_class(s: &State) -> String {
    let dock = &appearance(s).dock;
    let position = match dock.position {
        DockPosition::Left => " dock-side-zone dock-left",
        DockPosition::Right => " dock-side-zone dock-right",
        DockPosition::Bottom => "",
    };
    let panel = if dock.panel_mode {
        " dock-panel-zone"
    } else {
        ""
    };
    format!("desktop-dock-zone{position}{panel}")
}

pub(crate) fn panel_mode(s: &State) -> bool {
    appearance(s).dock.panel_mode
}

pub(crate) fn floating_dock_status(s: &State) -> bool {
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
    RemoveFile {
        epoch: u64,
        name: String,
        permanent: bool,
    },
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

fn dock_item(shortcut: Shortcut) -> Option<DockItem> {
    match shortcut {
        Shortcut::App(app) => Some(DockItem::App(app)),
        Shortcut::System(system) => Some(DockItem::System { system }),
        Shortcut::File(_) => None,
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
        Gesture::RemoveFile { epoch, .. }
        | Gesture::Context { epoch, .. }
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
        Gesture::RemoveFile {
            name, permanent, ..
        } => {
            if let Some(file) = ui.state.with(|s| find_file(s, &name).cloned()) {
                trash_ui::desktop_file(ui, &file, permanent);
            }
        }
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
                    let Some(item) = dock_item(item) else {
                        return;
                    };
                    let dock = &mut prefs.dock;
                    let mut order: Vec<_> = icons
                        .into_iter()
                        .filter_map(dock_item)
                        .filter(|&entry| entry == item || dock_pinned(dock, entry))
                        .collect();
                    for &entry in &dock.order {
                        if !order.contains(&entry) {
                            order.push(entry);
                        }
                    }
                    dock.order = order;
                    if let DockItem::App(app) = item {
                        dock.unpinned.retain(|id| *id != app);
                    }
                }
            });
        }
    }
}

pub(crate) fn gesture_input(ui: Ui) -> impl Widget {
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
    label: impl Into<Vary<String>>,
    mut action: F,
) -> sapid::Button {
    let ui = ui.clone();
    sapid::button()
        .class("menu-item")
        .label(i18n::bind(label))
        .on_press(move || {
            ui.close_desktop_menu();
            action(&ui);
        })
}

pub(crate) fn open_settings(ui: &Ui, page: SettingsPage) {
    ui.state.revise(|s| {
        s.server_apps.customization.settings_page = page;
        s.server_apps.customization.menu = None;
    });
    ui.open_desktop_app(WorkspacePage::Settings);
}

fn dock_pin_action(ui: &Ui, item: DockItem) -> sapid::Button {
    menu_action(
        ui,
        ui.state.map(move |s| {
            if dock_pinned(&appearance(s).dock, item) {
                "Remove from Dock"
            } else {
                "Keep in Dock"
            }
            .to_owned()
        }),
        move |ui| toggle_dock_pin(ui, item),
    )
    .enabled(ui.state.map(server_apps_ui::preferences_ready))
}

fn system_icon_menu(ui: Ui, system: SystemIcon, zone: Zone) -> impl Widget {
    fragment((
        menu_action(&ui, "Open", move |ui| open_system_icon(ui, system)),
        dock_pin_action(&ui, DockItem::System { system }),
        if zone == Zone::Desktop {
            menu_action(&ui, "Hide desktop shortcut", move |ui| {
                set_system_visible(ui, system, false);
            })
            .enabled(ui.state.map(server_apps_ui::preferences_ready))
        } else {
            menu_action(&ui, "Dock settings", |ui| {
                open_settings(ui, SettingsPage::Dock)
            })
        },
    ))
}

fn launcher_menu(ui: Ui, app: DesktopApp, zone: Zone) -> impl Widget {
    let state = ui.state;
    let page = server_desktop::app_page(app);
    let running = state.map(move |s| server_desktop::launcher_running(s, page));
    let files = page.is_files();
    let extra = match page {
        WorkspacePage::Files(_) => fragment(
            menu_action(&ui, "New Window", |ui| {
                files_tabs::new_window(ui, files_tabs::WindowStart::Default);
            })
            .enabled(state.map(files_tabs::available)),
        ),
        WorkspacePage::Terminal => fragment(when(running, {
            let ui = ui.clone();
            move || {
                menu_action(&ui, "New tab", move |ui| {
                    let id = ui.state.with(|s| s.terminal.id);
                    ui.open_desktop_app(page);
                    ui.terminal_new_tab(id);
                })
                .enabled(state.map(|s| s.terminal.session_count() < terminal_worker::SESSION_LIMIT))
            }
        })),
        #[cfg(windows)]
        WorkspacePage::Browser => fragment(when(running, {
            let ui = ui.clone();
            move || {
                menu_action(&ui, "New web tab", move |ui| {
                    ui.open_desktop_app(page);
                    ui.state.revise(|s| {
                        s.web_browser.add_tab(None);
                    });
                })
                .enabled(state.map(|s| s.web_browser.tabs.len() < web_browser::TAB_LIMIT))
            }
        })),
        WorkspacePage::Player => fragment(menu_action(&ui, "Open video", move |ui| {
            ui.open_desktop_app(page);
            ui.player.browse(ui);
        })),
        _ => fragment(()),
    };
    fragment((
        menu_action(
            &ui,
            state.map(move |s| {
                if server_desktop::launcher_running(s, page) {
                    "Show window"
                } else {
                    "Open"
                }
                .to_owned()
            }),
            move |ui| ui.open_desktop_app(page),
        ),
        extra,
        when(files, {
            let ui = ui.clone();
            move || {
                let open = ui.clone();
                fragment((
                    text().class("menu-divider").content(""),
                    each(state.map(|s| s.server_apps.files_locations.clone().unwrap_or_default()))
                        .key(|location| location.name)
                        .children(move |item| {
                            let location = item.value.get();
                            menu_action(&open, location.name, move |ui| {
                                files_tabs::new_window(
                                    ui,
                                    files_tabs::WindowStart::Directory(location.path.clone()),
                                );
                            })
                            .enabled(state.map(files_tabs::available))
                        }),
                    menu_action(&ui, "Filesystem", |ui| {
                        files_tabs::new_window(ui, files_tabs::WindowStart::Directory("/".into()));
                    })
                    .enabled(state.map(files_tabs::available)),
                ))
            }
        }),
        text().class("menu-divider").content(""),
        dock_pin_action(&ui, DockItem::App(app)),
        if zone == Zone::Desktop {
            menu_action(&ui, "Hide desktop shortcut", move |ui| {
                set_shortcut_visible(ui, app, false);
            })
            .enabled(state.map(server_apps_ui::preferences_ready))
        } else {
            menu_action(&ui, "Dock settings", |ui| {
                open_settings(ui, SettingsPage::Dock)
            })
        },
        when(running, move || {
            fragment((
                text().class("menu-divider").content(""),
                menu_action(
                    &ui,
                    if files {
                        "Close window"
                    } else {
                        "Close application"
                    },
                    move |ui| ui.close_desktop_launcher(page),
                )
                .enabled(state.map(move |s| server_desktop::can_close_app(s, page))),
            ))
        }),
    ))
}

pub(crate) fn context_menu(ui: Ui) -> impl Widget {
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
                    let file = match item {
                        Some(Shortcut::App(app)) => {
                            return fragment(launcher_menu(ui.clone(), app, zone));
                        }
                        Some(Shortcut::File(name)) => Some(name),
                        Some(Shortcut::System(system)) => {
                            return fragment(system_icon_menu(ui.clone(), system, zone));
                        }
                        None => None,
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
                    fragment((
                        file_open::menu(
                            ui.clone(),
                            {
                                let file = file.clone();
                                move |s| {
                                    let entry = find_file(s, file.as_ref()?)?;
                                    (!dock && !entry.is_dir && !entry.is_symlink)
                                        .then(|| entry.path.clone())
                                }
                            },
                            Ui::close_desktop_menu,
                        ),
                        when(!dock && file.is_some(), {
                            let ui = ui.clone();
                            let name = file.clone().unwrap_or_default();
                            move || {
                                let buttons = [("移入回收站", false), ("永久删除", true)]
                                    .into_iter()
                                    .map(|(label, permanent)| {
                                        let name = name.clone();
                                        menu_action(&ui, label, move |ui| {
                                            if let Some(file) =
                                                ui.state.with(|s| find_file(s, &name).cloned())
                                            {
                                                trash_ui::desktop_file(ui, &file, permanent);
                                            }
                                        })
                                    })
                                    .collect::<Vec<_>>();
                                fragment(buttons)
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
                                        WallpaperTarget::Active,
                                    )
                                })
                            }
                        }),
                        files_ui::appearance_menu(
                            ui.clone(),
                            {
                                let file = file.clone();
                                move |s| {
                                    let entry = find_file(s, file.as_ref()?)?;
                                    (!dock && !entry.is_symlink)
                                        .then(|| (entry.path.clone(), entry.is_dir))
                                }
                            },
                            Ui::close_desktop_menu,
                            false,
                        ),
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
                                                        ui.open_desktop_app(WorkspacePage::Files(
                                                            0,
                                                        ));
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
                col().class("pane-menu desktop-context-menu").fill((
                    coordinates,
                    col().class("desktop-context-items").fill(entries),
                )),
            ))
        },
    )
}

pub(crate) fn settings_tabs(ui: Ui) -> impl Widget {
    col()
        .class("desktop-settings-tabs")
        .name(i18n::bind("Settings sections"))
        .fill(fragment(
            [
                (SettingsPage::General, "General", Icon::Settings),
                (SettingsPage::Network, "Network", Icon::Server),
                (SettingsPage::Desktop, "Desktop", Icon::Grid),
                (SettingsPage::Dock, "Dock", Icon::Sidebar),
                (SettingsPage::Appearance, "Appearance", Icon::File),
                (SettingsPage::Region, "Region & Language", Icon::Browser),
                (SettingsPage::DateTime, "Date & Time", Icon::Monitor),
                (SettingsPage::DefaultApps, "Default apps", Icon::Folder),
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
                        state.revise(|s| {
                            if page != SettingsPage::Appearance {
                                desktop_appearance_ui::cancel(s);
                            }
                            s.server_apps.customization.settings_page = page;
                        });
                        network_ui::enter(&choose);
                        choose.load_desktop_wallpaper();
                        if page == SettingsPage::DateTime {
                            choose.read_desktop_time_zone();
                            choose.tick_desktop_clock();
                        }
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
) -> impl Widget + use<> {
    let ui = ui.clone();
    sapid::button()
        .class("button")
        .label(i18n::bind(label))
        .pressed(ui.state.map(move |s| selected(appearance(s))))
        .enabled(ui.state.map(server_apps_ui::preferences_ready))
        .on_press(move || update(&ui, change))
}

fn settings_dialog_link(ui: Ui, title: &'static str, dialog: Dialog) -> impl Widget {
    let label = i18n::bind(title);
    sapid::button()
        .class("desktop-setting-link")
        .name(label.clone())
        .enabled(ui.state.map(server_apps_ui::preferences_ready))
        .fill((
            text().class("desktop-setting-label").content(label),
            icons::glyph(Icon::ChevronRight),
        ))
        .on_press(move || ui.open_dialog(dialog.clone()))
}

pub(crate) fn shortcut_settings_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| matches!(s.dialog, Some(Dialog::DesktopIcons | Dialog::DockShortcuts))),
        move || {
            let dock = state.with(|s| s.dialog == Some(Dialog::DockShortcuts));
            let change = ui.clone();
            let reset = ui.clone();
            modal(
                ui.clone(),
                if dock {
                    "Pinned shortcuts"
                } else {
                    "Visible icons"
                },
                col().class("desktop-shortcut-options").fill(
                    each(state.map(move |s| {
                        if dock {
                            dock_choices(s)
                        } else {
                            shortcut_choices(server_desktop::available_apps(s))
                        }
                    }))
                    .key(|item| *item)
                    .children(move |item| {
                        let item = item.value.get();
                        let (label, glyph) = match item {
                            DockItem::App(app) => {
                                let page = server_desktop::app_page(app);
                                (
                                    server_desktop::app_label(page),
                                    server_desktop::app_icon(page),
                                )
                            }
                            DockItem::System { system } => {
                                let (label, glyph, _) = system_icon(system);
                                (label, glyph)
                            }
                        };
                        let label = i18n::bind(label);
                        let change = change.clone();
                        row().class("desktop-shortcut-option").fill((
                            icons::glyph(glyph),
                            sapid::checkbox()
                                .class("preference-check")
                                .name(label.clone())
                                .label(label)
                                .checked(state.map(move |s| {
                                    let prefs = appearance(s);
                                    if dock {
                                        dock_pinned(&prefs.dock, item)
                                    } else {
                                        match item {
                                            DockItem::App(app) => shortcut_visible(prefs, app),
                                            DockItem::System { system } => {
                                                system_visible(prefs, system)
                                            }
                                        }
                                    }
                                }))
                                .enabled(state.map(server_apps_ui::preferences_ready))
                                .on_checked_change(move |visible| {
                                    if dock {
                                        if state.with(|s| dock_pinned(&appearance(s).dock, item))
                                            != visible
                                        {
                                            toggle_dock_pin(&change, item);
                                        }
                                    } else {
                                        match item {
                                            DockItem::App(app) => {
                                                set_shortcut_visible(&change, app, visible)
                                            }
                                            DockItem::System { system } => {
                                                set_system_visible(&change, system, visible)
                                            }
                                        }
                                    }
                                }),
                        ))
                    }),
                ),
                when(!dock, move || {
                    let reset = reset.clone();
                    text_action("Reset desktop icons", Icon::Refresh)
                        .enabled(state.map(server_apps_ui::preferences_ready))
                        .on_press(move || {
                            update(&reset, |prefs| {
                                prefs.order.clear();
                                prefs.hidden.clear();
                                prefs.show_apps = true;
                            });
                        })
                }),
            )
        },
    )
}

pub(crate) fn settings_row(
    title: &'static str,
    controls: impl glory::ui::IntoChildren,
) -> impl Widget {
    row().class("desktop-setting-row").fill((
        text()
            .class("desktop-setting-label")
            .content(i18n::bind(title)),
        row()
            .class("desktop-setting-controls choice-list")
            .fill(controls),
    ))
}

fn choices<T: Copy + PartialEq + 'static>(
    ui: &Ui,
    options: &'static [(T, &'static str)],
    read: fn(&Appearance) -> T,
    write: fn(&mut Appearance, T),
) -> impl Widget + use<T> {
    let state = ui.state;
    fragment(
        options
            .iter()
            .map(|&(value, label)| {
                let ui = ui.clone();
                sapid::button()
                    .class("button")
                    .label(i18n::bind(label))
                    .pressed(state.map(move |s| read(appearance(s)) == value))
                    .enabled(state.map(server_apps_ui::preferences_ready))
                    .on_press(move || update(&ui, |prefs| write(prefs, value)))
            })
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn layout_controls(ui: Ui) -> impl Widget {
    col()
        .class("desktop-settings-card desktop-settings-rows")
        .fill((
            text()
                .class("section-title")
                .content(i18n::bind("Desktop layout")),
            settings_row(
                "Icon arrangement",
                choices(
                    &ui,
                    &[
                        (IconFlow::Columns, "Vertical"),
                        (IconFlow::Rows, "Horizontal"),
                    ],
                    |prefs| prefs.layout.flow,
                    |prefs, value| prefs.layout.flow = value,
                ),
            ),
            settings_row(
                "Starting edge",
                choices(
                    &ui,
                    &[
                        (IconDirection::LeftToRight, "Left to right"),
                        (IconDirection::RightToLeft, "Right to left"),
                    ],
                    |prefs| prefs.layout.direction,
                    |prefs, value| prefs.layout.direction = value,
                ),
            ),
        ))
}

pub(crate) fn icon_settings(ui: Ui) -> impl Widget {
    col().class("desktop-settings-card").fill((
        text()
            .class("section-title")
            .content(i18n::bind("Desktop icons")),
        settings_row(
            "Desktop icon size",
            icon_size_control(
                &ui,
                "Desktop icon size",
                "icon-size-control desktop-size-control",
                Layout::MIN_ICON_SIZE..=Layout::MAX_ICON_SIZE,
                const {
                    &[
                        (DesktopIconSize::Small.pixels(), "Small"),
                        (DesktopIconSize::Medium.pixels(), "Medium"),
                        (DesktopIconSize::Large.pixels(), "Large"),
                    ]
                },
                |prefs| prefs.layout.icon_pixels(),
                |prefs, value| prefs.layout.set_icon_pixels(value),
            ),
        ),
        settings_dialog_link(ui.clone(), "Visible icons", Dialog::DesktopIcons),
        row().class("choice-list").fill(toggle(
            &ui,
            "Show server info",
            |p| p.show_server,
            |p| p.show_server = !p.show_server,
        )),
        text().class("subtle").content(i18n::bind(
            "Drag from Files. Ctrl copies, Shift moves, and Alt or Ctrl+Shift creates a shortcut.",
        )),
    ))
}

pub(crate) fn icon_grid_class(s: &State) -> String {
    let layout = appearance(s).layout;
    let flow = match layout.flow {
        IconFlow::Columns => "",
        IconFlow::Rows => " icons-by-row",
    };
    let direction = match layout.direction {
        IconDirection::LeftToRight => "",
        IconDirection::RightToLeft => " icons-from-right",
    };
    let size = match layout.icon_pixels() {
        pixels if pixels == DesktopIconSize::Small.pixels() => " icons-small".to_owned(),
        pixels if pixels == DesktopIconSize::Medium.pixels() => String::new(),
        pixels if pixels == DesktopIconSize::Large.pixels() => " icons-large".to_owned(),
        pixels => format!(" icons-size-{pixels}"),
    };
    format!("desktop-shortcuts{flow}{direction}{size}")
}

fn icon_size_control(
    ui: &Ui,
    label: &'static str,
    class: &'static str,
    range: RangeInclusive<u8>,
    presets: &'static [(u8, &'static str)],
    read: fn(&Appearance) -> u8,
    write: fn(&mut Appearance, u8),
) -> impl Widget + use<> {
    let state = ui.state;
    let resize_icons = ui.clone();
    let (min, max) = (*range.start(), *range.end());
    col().class("icon-size-options").fill((
        row().class(class).fill((
            input()
                .class("icon-size-bridge hidden")
                .name(i18n::bind(label))
                .enabled(state.map(server_apps_ui::preferences_ready))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event
                        && let Ok(pixels) = value.parse::<u8>()
                        && range.contains(&pixels)
                    {
                        update(&resize_icons, |prefs| write(prefs, pixels));
                    }
                }),
            text()
                .class("icon-size-state hidden")
                .content(state.map(move |s| format!("{min},{max},{}", read(appearance(s))))),
        )),
        row()
            .class("choice-list")
            .fill(choices(ui, presets, read, write)),
    ))
}

pub(crate) fn dock_controls(ui: Ui) -> impl Widget {
    let alignment = ui.clone();
    let status = ui.clone();
    let reset = ui.clone();
    let state = ui.state;
    col()
        .class("desktop-settings-card desktop-settings-rows")
        .fill((
            text().class("section-title").content(i18n::bind("Dock")),
            settings_row(
                "Dock position",
                choices(
                    &ui,
                    &[
                        (DockPosition::Left, "Left side"),
                        (DockPosition::Right, "Right side"),
                        (DockPosition::Bottom, "Bottom"),
                    ],
                    |prefs| prefs.dock.position,
                    |prefs, value| prefs.dock.position = value,
                ),
            ),
            settings_row(
                "Dock style",
                choices(
                    &ui,
                    &[(false, "Floating Dock"), (true, "Panel mode")],
                    |prefs| prefs.dock.panel_mode,
                    |prefs, value| prefs.dock.panel_mode = value,
                ),
            ),
            when(state.map(panel_mode), move || {
                settings_row(
                    "App icon alignment",
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
                                        .label(i18n::bind(state.map(move |s| {
                                            if !centered
                                                && appearance(s).dock.position
                                                    != DockPosition::Bottom
                                            {
                                                "Align top"
                                            } else {
                                                label
                                            }
                                            .to_owned()
                                        })))
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
                )
            }),
            when(state.map(|s| !panel_mode(s)), move || {
                settings_row(
                    "Status bar",
                    toggle(
                        &status,
                        "Show Dock status bar",
                        |p| p.dock.floating_status,
                        |p| p.dock.floating_status = !p.dock.floating_status,
                    ),
                )
            }),
            settings_row(
                "Dock icon size",
                icon_size_control(
                    &ui,
                    "Dock icon size",
                    "icon-size-control dock-size-control",
                    Dock::MIN_ICON_SIZE..=Dock::MAX_ICON_SIZE,
                    const {
                        &[
                            (DockSize::Small.pixels(), "Small"),
                            (DockSize::Medium.pixels(), "Medium"),
                            (DockSize::Large.pixels(), "Large"),
                        ]
                    },
                    |prefs| prefs.dock.icon_pixels(),
                    |prefs, value| prefs.dock.set_icon_pixels(value),
                ),
            ),
            settings_row(
                "Dock behavior",
                (
                    toggle(
                        &ui,
                        "Automatically hide Dock",
                        |p| p.dock.auto_hide,
                        |p| p.dock.auto_hide = !p.dock.auto_hide,
                    ),
                    text_action("Reset Dock", Icon::Refresh)
                        .enabled(state.map(server_apps_ui::preferences_ready))
                        .on_press(move || update(&reset, |prefs| prefs.dock = Dock::default())),
                ),
            ),
            settings_dialog_link(ui, "Pinned shortcuts", Dialog::DockShortcuts),
        ))
}

pub(crate) fn wallpaper_controls(ui: Ui, current: fn(&State) -> Wallpaper) -> impl Widget {
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
                .pressed(state.map(move |s| current(s) == selected)).enabled(ready)
                .fill((text().class(format!("wallpaper-swatch {class}")).content(""), text().content(i18n::bind(label))))
                .on_press(move || {
                    choose.state.revise(|s| s.server_apps.customization.wallpaper_error.clear());
                    desktop_appearance_ui::set_background(&choose, wallpaper.clone());
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
                    server.request_wallpaper(AppAction::ReadWallpaper(path), WallpaperTarget::Draft);
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
                upload.request_wallpaper(AppAction::ImportWallpaper(path), WallpaperTarget::Draft);
            }).enabled(state.map(move |s| transfer.get() && !runtime(s).local_image.trim().is_empty())),
            when(cfg!(windows), move || btn("Browse…", {
                let browse = browse.clone();
                move || {
                    let current = state.with(|s| {
                        let path = runtime(s).local_image.trim();
                        PathBuf::from(if path.is_empty() { &s.local_path } else { path })
                    });
                    match file_picker::private_key(Some(&current), &i18n::translate("Choose wallpaper"), &i18n::translate("Select file"), &i18n::translate("All files")) {
                        Ok(Some(path)) => browse.request_wallpaper(AppAction::ImportWallpaper(path), WallpaperTarget::Draft),
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
