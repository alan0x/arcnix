use std::collections::BTreeMap;
use std::result::Result;
use std::sync::Arc;

use serde::Deserialize;

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::close_action;
use crate::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Location {
    #[default]
    Directory,
    Trash,
    Apps,
}

#[derive(Clone, Default)]
pub(crate) struct Windows {
    pub groups: window_tabs::Groups,
    saved: BTreeMap<u64, Arc<file_browser::TabSnapshot>>,
    live: Option<u32>,
    next_tab: u64,
    pub location: Location,
    pub clipboard: Vec<String>,
    refreshes: BTreeMap<u64, String>,
}

impl Windows {
    pub(crate) fn is_live(&self, id: u32) -> bool {
        self.live == Some(id)
    }
    pub(crate) fn current(&self) -> Option<u32> {
        self.live.or_else(|| self.groups.first_window())
    }

    fn add_tab(&mut self, window: u32) -> u64 {
        self.next_tab += 1;
        self.groups.current = window;
        self.groups.add(self.next_tab);
        self.next_tab
    }
}

fn save(s: &mut State) {
    let files = &s.workbench.files;
    if let Some(tab) = files.live.and_then(|id| files.groups.selected(id)) {
        let snapshot = Arc::new(file_browser::capture_tab(s));
        s.workbench.files.saved.insert(tab, snapshot);
    }
}

fn load(s: &mut State, id: u32) {
    let files = &mut s.workbench.files;
    files.live = Some(id);
    files.groups.current = id;
    if let Some(saved) = files
        .groups
        .selected(id)
        .and_then(|tab| files.saved.get(&tab))
        .cloned()
    {
        file_browser::restore_tab(s, &saved);
    }
}

// Capture the live view while the source window still owns its selected tab.
pub(crate) fn prepare_move(s: &mut State) {
    save(s);
    s.workbench.files.live = None;
}

pub(crate) fn focus(s: &mut State, id: u32) {
    if s.workbench.files.live == Some(id) {
        return;
    }
    save(s);
    if s.workbench.files.groups.tabs(id).is_empty() {
        s.workbench.files.add_tab(id);
    }
    load(s, id);
}

pub(crate) fn remove(s: &mut State, id: u32) {
    let files = &mut s.workbench.files;
    for tab in files.groups.tabs(id) {
        files.saved.remove(tab);
    }
    if files.live == Some(id) {
        files.live = None;
        files.location = Location::Directory;
    }
}

pub(crate) fn trash_active(s: &State) -> bool {
    s.workspace_page.is_files() && s.workbench.files.location == Location::Trash
}

pub(crate) fn apps_active(s: &State) -> bool {
    s.workspace_page.is_files() && s.workbench.files.location == Location::Apps
}

fn snapshot(s: &State, id: u32) -> Option<&file_browser::TabSnapshot> {
    let files = &s.workbench.files;
    files
        .saved
        .get(&files.groups.selected(id)?)
        .map(Arc::as_ref)
}

pub(crate) fn showing_trash(s: &State, id: u32) -> bool {
    location(s, id) == Location::Trash
}

pub(crate) fn showing_apps(s: &State, id: u32) -> bool {
    location(s, id) == Location::Apps
}

fn location(s: &State, id: u32) -> Location {
    if s.workbench.files.is_live(id) {
        s.workbench.files.location
    } else {
        snapshot(s, id).map_or_default(file_browser::TabSnapshot::view)
    }
}

pub(crate) fn open_apps(ui: &Ui) {
    ui.state.revise(|s| {
        if !available(s) || !s.workspace_page.is_files() {
            return;
        }
        s.workbench.files.location = Location::Apps;
        s.server_apps.files_tag = None;
        s.server_apps.files_open_pending = None;
        s.file_context = None;
        s.file_menu = None;
        file_browser::clear_selection(s, true);
    });
}

pub(crate) fn leave_apps(ui: &Ui) {
    let left = ui.state.revise(|s| {
        if !apps_active(s) {
            return false;
        }
        s.workbench.files.location = Location::Directory;
        true
    });
    if left {
        after_change(ui);
    }
}

pub(crate) fn showing_tag(s: &State, id: u32) -> Option<files_ui::TagFilter> {
    if s.workbench.files.is_live(id) {
        s.server_apps.files_tag
    } else {
        snapshot(s, id)?.tag()
    }
}

pub(crate) fn directory(s: &State, id: u32) -> Option<&str> {
    if s.workbench.files.live == Some(id) {
        (s.remote_ready
            && s.workbench.files.location == Location::Directory
            && s.server_apps.files_tag.is_none())
        .then_some(s.remote_path.as_str())
    } else {
        snapshot(s, id)?.directory()
    }
}

pub(crate) fn drop_folder(s: &State, id: u32, folder: Option<&str>) -> Option<String> {
    let directory = directory(s, id)?;
    let Some(path) = folder else {
        return Some(directory.to_owned());
    };
    let is_folder = |entry: &FileEntry| entry.path == path && entry.is_dir && !entry.is_symlink;
    let valid = if s.workbench.files.live == Some(id) {
        file_browser::entries(s, true).iter().any(is_folder)
    } else {
        snapshot(s, id)?.entries().iter().any(is_folder)
    };
    valid.then(|| path.to_owned())
}

pub(crate) fn dragged_entries(
    s: &State,
    id: u32,
    path: &str,
) -> Vec<arcnix_core::deletion::Target> {
    if showing_apps(s, id) {
        return Vec::new();
    }
    if !s.workbench.files.is_live(id) {
        return snapshot(s, id)
            .map(|tab| tab.dragged_entries(path))
            .unwrap_or_default();
    }
    let Some(entry) = file_browser::entries(s, true)
        .into_iter()
        .find(|entry| entry.path == path && !entry.is_symlink)
    else {
        return Vec::new();
    };
    if s.remote_selection.contains(path) {
        file_context::targets(s, true)
    } else {
        vec![arcnix_core::deletion::Target::from(&entry)]
    }
}

pub(crate) fn refresh_windows(ui: &Ui) {
    let directories = ui.state.with(|s| {
        s.workbench
            .files
            .groups
            .window_ids()
            .filter(|id| Some(*id) != s.workbench.files.live)
            .filter_map(|id| directory(s, id).map(str::to_owned))
            .collect::<std::collections::BTreeSet<_>>()
    });
    for path in directories {
        let serial = ui.send(Action::Children {
            remote: true,
            path: path.clone(),
        });
        ui.state
            .revise(|s| s.workbench.files.refreshes.insert(serial, path));
    }
}

pub(crate) fn awaiting_refresh(ui: &Ui, serial: u64) -> bool {
    ui.state
        .with(|s| s.workbench.files.refreshes.contains_key(&serial))
}

pub(crate) fn refreshed(ui: &Ui, serial: u64, path: &str, result: Result<Vec<FileEntry>, String>) {
    ui.state.revise(|s| {
        if s.workbench.files.refreshes.remove(&serial).as_deref() != Some(path) {
            return;
        }
        if let Ok(entries) = result {
            for saved in s.workbench.files.saved.values_mut() {
                if saved.directory() == Some(path) {
                    Arc::make_mut(saved).refresh_entries(&entries);
                }
            }
        }
    });
}

pub(crate) fn available(s: &State) -> bool {
    server_desktop::visible(s)
        && s.dialog.is_none()
        && s.file_pending.is_none()
        && s.fingerprint.is_none()
        && s.sftp_available
}

pub(crate) enum WindowStart {
    Current,
    Default,
    Directory(String),
}

pub(crate) fn new_window(ui: &Ui, start: WindowStart) {
    let opened = ui.state.revise(|s| {
        if !available(s) {
            return false;
        }
        let (saved, path) = match start {
            WindowStart::Current if s.workspace_page.is_files() => {
                (Some(Arc::new(file_browser::capture_tab(s))), None)
            }
            WindowStart::Current => return false,
            WindowStart::Default => (
                None,
                Some(selected_profile(s).map_or_else(
                    || s.remote_path.clone(),
                    |profile| profile.initial_remote_path.clone(),
                )),
            ),
            WindowStart::Directory(path) => (None, Some(path)),
        };
        let files = &mut s.workbench.files;
        let id = files.groups.new_window();
        let tab = files.add_tab(id);
        if let Some(saved) = saved {
            files.saved.insert(tab, saved);
        }
        server_desktop::select_app(s, WorkspacePage::Files(id));
        if let Some(path) = path {
            file_browser::fresh_tab(s);
            s.remote_path = path;
            s.remote.clear();
            s.server_apps.files_tag = None;
            s.workbench.files.location = Location::Directory;
        }
        true
    });
    if opened {
        ui.desktop_enter();
    }
}

pub(crate) fn after_change(ui: &Ui) {
    ui.state.revise(|s| {
        s.file_context = None;
        s.file_menu = None;
    });
    if ui.state.with(trash_active) {
        trash_ui::enter(ui);
    } else if ui
        .state
        .with(|s| s.workspace_page.is_files() && !apps_active(s) && !s.remote_ready)
    {
        ui.refresh(true);
    }
}

pub(crate) fn new_tab(ui: &Ui, id: u32) {
    open_tab(ui, id, None);
}

pub(crate) fn open_tab(ui: &Ui, id: u32, path: Option<String>) {
    if !ui
        .state
        .with(|s| available(s) && s.workbench.window_visible(WorkspacePage::Files(id)))
    {
        return;
    }
    ui.focus_desktop_app(WorkspacePage::Files(id));
    ui.state.revise(|s| {
        save(s);
        s.workbench.files.add_tab(id);
        file_browser::fresh_tab(s);
    });
    if let Some(path) = path {
        file_browser::navigate(ui, true, path);
    } else {
        after_change(ui);
    }
}

fn select(ui: &Ui, window: u32, tab: u64) {
    if !ui.state.with(|s| {
        available(s)
            && s.workbench.window_visible(WorkspacePage::Files(window))
            && s.workbench.files.groups.tabs(window).contains(&tab)
    }) {
        return;
    }
    ui.focus_desktop_app(WorkspacePage::Files(window));
    ui.state.revise(|s| {
        if s.workbench.files.groups.selected(window) != Some(tab) {
            save(s);
            s.workbench.files.groups.select(tab);
            load(s, window);
        }
    });
    after_change(ui);
}

pub(crate) fn close_selected(ui: &Ui, id: u32) {
    if let Some(tab) = ui.state.with(|s| s.workbench.files.groups.selected(id)) {
        close(ui, id, tab);
    }
}

fn close(ui: &Ui, id: u32, tab: u64) {
    if !ui.state.with(available) {
        return;
    }
    let empty = ui.state.revise(|s| {
        let files = &mut s.workbench.files;
        if !files.groups.tabs(id).contains(&tab) {
            return false;
        }
        let selected = files.groups.selected(id) == Some(tab);
        files.groups.remove(tab);
        files.saved.remove(&tab);
        if files.groups.tabs(id).is_empty() {
            return true;
        }
        if selected && files.live == Some(id) {
            load(s, id);
        }
        false
    });
    if empty {
        ui.finish_desktop_close(WorkspacePage::Files(id));
    }
    after_change(ui);
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Gesture {
    New { window: u32 },
    Close { window: u32, tab: u64 },
    ToggleHidden { window: u32 },
    Zoom { window: u32, tab: u64, larger: bool },
}

pub(crate) fn bridge(ui: Ui) -> impl Widget {
    input()
        .class("files-tab-input hidden")
        .on(value_changed, move |event| {
            if let Event::ValueChanged { value, .. } = event
                && value.len() <= 512
                && let Ok((epoch, gesture)) = serde_json::from_str::<(u64, Gesture)>(&value)
                && ui.state.with(|s| s.epoch == epoch)
            {
                match gesture {
                    Gesture::New { window } => new_tab(&ui, window),
                    Gesture::Close { window, tab } => close(&ui, window, tab),
                    Gesture::Zoom {
                        window,
                        tab,
                        larger,
                    } => {
                        let page = WorkspacePage::Files(window);
                        if ui.state.with(|s| {
                            available(s)
                                && s.workbench.window_visible(page)
                                && s.workbench.files.groups.selected(window) == Some(tab)
                                && directory(s, window).is_some()
                                && s.file_menu.is_none()
                        }) {
                            if ui.state.with(|s| s.workspace_page != page) {
                                ui.focus_desktop_app(page);
                            }
                            ui.state.revise(|s| files_view::step(s, larger));
                        }
                    }
                    Gesture::ToggleHidden { window } => {
                        if let Some(show) = ui.state.with(|s| {
                            (server_desktop::visible(s)
                                && s.workspace_page == WorkspacePage::Files(window)
                                && s.dialog.is_none()
                                && s.fingerprint.is_none())
                            .then_some(!s.config.settings.show_hidden_files)
                        }) {
                            ui.change_hidden_files(show);
                        }
                    }
                }
            }
        })
}

fn title(s: &State, window: u32, id: u64) -> String {
    let files = &s.workbench.files;
    if files.live == Some(window) && files.groups.selected(window) == Some(id) {
        file_browser::live_tab_title(s)
    } else {
        files
            .saved
            .get(&id)
            .map_or_else(|| i18n::translate("Files"), |view| view.title())
    }
}

pub(crate) fn tab_strip(ui: Ui, id: u32) -> impl Widget {
    let state = ui.state;
    let choose = ui.clone();
    row()
        .class("files-tabs window-tab-strip")
        .name(i18n::bind("File tabs"))
        .fill(
            each(state.map(move |s| s.workbench.files.groups.tabs(id).to_vec()))
                .key(|tab| *tab)
                .children(move |t| {
                    let tab = t.value.get();
                    let remove = choose.clone();
                    let select_ui = choose.clone();
                    row()
                        .class(state.map(move |s| {
                            let selected = s.workbench.files.groups.selected(id) == Some(tab);
                            if selected {
                                "files-tab window-tab-item selected"
                            } else {
                                "files-tab window-tab-item"
                            }
                            .to_owned()
                        }))
                        .fill((
                            text()
                                .class("files-tab-id window-tab-id hidden")
                                .content(tab.to_string()),
                            row().class("files-tab-drag-source").fill(
                                sapid::button()
                                    .class("files-tab-select window-tab-select")
                                    .name(state.map(move |s| title(s, id, tab)))
                                    .pressed(state.map(move |s| {
                                        s.workbench.files.groups.selected(id) == Some(tab)
                                    }))
                                    .fill(text().content(state.map(move |s| title(s, id, tab))))
                                    .on_press(move || select(&select_ui, id, tab)),
                            ),
                            close_action("Close file tab")
                                .class("icon-button files-tab-close")
                                .on_press(move || close(&remove, id, tab)),
                        ))
                }),
        )
}

pub(crate) fn window_content(ui: Ui, id: u32) -> impl Widget {
    let state = ui.state;
    col().class("files-window-content").fill((
        text()
            .class("files-window-id hidden")
            .content(id.to_string()),
        text()
            .class("files-drop-directory hidden")
            .content(state.map(move |s| directory(s, id).unwrap_or_default().to_owned())),
        file_actions::drop_input(ui.clone(), id),
        workspace::files(ui, id),
    ))
}

pub(crate) fn preview_header(state: Cage<State>, id: u32) -> impl Widget {
    row()
        .class("files-window-tools")
        .fill(
            text()
                .class("files-inactive-path")
                .content(state.map(move |s| {
                    snapshot(s, id)
                        .map(|tab| tab.location())
                        .unwrap_or_default()
                })),
        )
}

pub(crate) fn preview_body(state: Cage<State>, id: u32) -> impl Widget {
    col().class(state.map(move |s| {
        if snapshot(s, id).is_some_and(|tab| tab.icons) { "files-inactive-preview icons-view" }
        else { "files-inactive-preview" }.to_owned()
    })).fill((
        text().class("files-icon-pixels hidden").content(state.map(move |s| {
            snapshot(s, id).map(|tab| tab.icon_size.pixels().to_string()).unwrap_or_default()
        })),
        each(state.map(move |s| {
            snapshot(s, id)
                .map(|tab| tab.preview_entries(s.config.settings.show_hidden_files))
                .unwrap_or_default()
        }))
        .key(|entry| entry.path.clone())
        .children(move |entry| {
            let value = entry.value;
            row()
                .name(value.map(|entry| entry.path.clone()))
                .class(value.map(|entry| {
                    if entry.is_symlink {
                        "files-inactive-entry"
                    } else if entry.is_dir {
                        "files-inactive-entry files-inactive-draggable files-directory-row"
                    } else {
                        "files-inactive-entry files-inactive-draggable"
                    }
                    .to_owned()
                }))
                .fill((
                    text()
                        .class(state.map(move |s| {
                            value.with(|file| file_browser::entry_icon_class(s, file, true))
                        }))
                        .content(""),
                    text().class("file-label").content(value.map(|file| file.name.clone())),
                ))
        }),
    ))
}
