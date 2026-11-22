use std::path::Component;

use glory::style::{Edges, Style};
use serde::Deserialize;

use crate::*;

#[derive(Clone, Default)]
pub(crate) struct PaneState {
    root: String,
    anchor: Option<String>,
    expanded: HashSet<String>,
    children: HashMap<String, Vec<FileEntry>>,
    pending: HashMap<String, u64>,
    pub editing: bool,
    draft: String,
    copied: bool,
    mutation: Option<u64>,
    edit_path: Option<String>,
    name_error: String,
    after_listing: Option<NewEntryFocus>,
}

#[derive(Clone)]
pub(crate) enum NewEntryFocus {
    Edit(String),
    Select(String),
}

#[derive(Clone, Default)]
pub(crate) struct BrowserState {
    local: PaneState,
    remote: PaneState,
    drag: Option<FileDrag>,
    remote_history: RemoteHistory,
}

#[derive(Clone, Default)]
struct RemoteHistory {
    paths: Vec<String>,
    cursor: usize,
    requested: Option<usize>,
}

impl RemoteHistory {
    fn current(&self) -> Option<&str> {
        self.paths.get(self.cursor).map(String::as_str)
    }

    fn neighbor(&self, backwards: bool) -> Option<usize> {
        let index = if backwards {
            self.cursor.checked_sub(1)?
        } else {
            self.cursor + 1
        };
        (index < self.paths.len()).then_some(index)
    }

    fn record(&mut self, path: &str) -> bool {
        let changed = self.current() != Some(path);
        if let Some(index) = self.requested.take() {
            self.paths[index] = path.to_owned();
            self.cursor = index;
        } else if changed {
            self.paths.truncate(self.cursor + 1);
            if self.paths.len() == 64 {
                self.paths.remove(0);
            }
            self.paths.push(path.to_owned());
            self.cursor = self.paths.len() - 1;
        }
        changed
    }
}

#[derive(Clone)]
pub(crate) struct TabSnapshot {
    path: String,
    ready: bool,
    entries: Vec<FileEntry>,
    selection: HashSet<String>,
    pane: PaneState,
    history: RemoteHistory,
    tag: Option<files_ui::TagFilter>,
    search: files_search::Search,
    view: files_tabs::Location,
}

impl TabSnapshot {
    pub(crate) fn title(&self) -> String {
        tab_title(&self.path, self.view, self.tag)
    }

    pub(crate) fn location(&self) -> String {
        match self.view {
            files_tabs::Location::Trash => i18n::translate("回收站"),
            files_tabs::Location::Apps => i18n::translate("Apps"),
            files_tabs::Location::Directory => self.path.clone(),
        }
    }

    pub(super) fn folder_view(&self, s: &State) -> files_view::FolderView {
        files_view::for_path(s, true, &self.path)
    }

    pub(crate) fn preview_entries(&self, s: &State) -> Vec<FileEntry> {
        if self.view != files_tabs::Location::Directory || !self.ready {
            Vec::new()
        } else {
            let search = self.search.open.then_some(&self.search);
            let range = search.and_then(files_search::Search::date_range);
            let view = self.folder_view(s);
            visible(
                &self.entries,
                search.map_or("", |s| s.query.trim()),
                view.sort,
                view.descending,
            )
            .into_iter()
            .filter(|entry| s.config.settings.show_hidden_files || !entry.is_hidden())
            .filter(|entry| search.is_none_or(|s| s.accepts(entry, range.as_ref())))
            .take(100)
            .collect()
        }
    }

    pub(crate) fn directory(&self) -> Option<&str> {
        (self.ready && self.view == files_tabs::Location::Directory && self.tag.is_none())
            .then_some(self.path.as_str())
    }

    pub(crate) fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    pub(crate) fn view(&self) -> files_tabs::Location {
        self.view
    }

    pub(crate) fn tag(&self) -> Option<files_ui::TagFilter> {
        self.tag
    }

    pub(crate) fn refresh_entries(&mut self, entries: &[FileEntry]) {
        self.entries = entries.to_vec();
        self.selection
            .retain(|path| entries.iter().any(|entry| &entry.path == path));
        self.pane = PaneState::default();
    }

    pub(crate) fn dragged_entries(&self, path: &str) -> Vec<arcnix_core::deletion::Target> {
        if self.view == files_tabs::Location::Apps
            || !self
                .entries
                .iter()
                .any(|entry| entry.path == path && !entry.is_symlink)
        {
            return Vec::new();
        }
        let multiple = self.selection.contains(path);
        self.entries
            .iter()
            .filter(|entry| entry.path == path || multiple && self.selection.contains(&entry.path))
            .map(arcnix_core::deletion::Target::from)
            .collect()
    }
}

fn tab_title(path: &str, view: files_tabs::Location, tag: Option<files_ui::TagFilter>) -> String {
    if view == files_tabs::Location::Trash {
        i18n::translate("回收站")
    } else if view == files_tabs::Location::Apps {
        i18n::translate("Apps")
    } else if let Some(tag) = tag {
        i18n::translate(tag.label())
    } else {
        path.rsplit('/')
            .next()
            .filter(|p| !p.is_empty())
            .unwrap_or(if path == "/" { "/" } else { "Files" })
            .to_owned()
    }
}

pub(crate) fn live_tab_title(s: &State) -> String {
    tab_title(
        &s.remote_path,
        s.workbench.files.location,
        s.server_apps.files_tag,
    )
}

pub(crate) fn capture_tab(s: &State) -> TabSnapshot {
    let mut pane = s.file_browser.remote.clone();
    pane.pending.clear();
    pane.editing = false;
    TabSnapshot {
        path: s.remote_path.clone(),
        ready: s.remote_ready,
        entries: s.remote.clone(),
        selection: s.remote_selection.clone(),
        pane,
        history: s.file_browser.remote_history.clone(),
        tag: s.server_apps.files_tag,
        search: s.server_apps.files_search.clone(),
        view: s.workbench.files.location,
    }
}

pub(crate) fn restore_tab(s: &mut State, tab: &TabSnapshot) {
    s.remote_path.clone_from(&tab.path);
    s.remote_ready = tab.ready;
    s.remote.clone_from(&tab.entries);
    s.remote_selection.clone_from(&tab.selection);
    s.file_browser.remote.clone_from(&tab.pane);
    s.file_browser.remote_history.clone_from(&tab.history);
    s.server_apps.files_tag = tab.tag;
    s.server_apps.files_search.clone_from(&tab.search);
    s.server_apps.files_open_pending = None;
    s.workbench.files.location = tab.view;
    s.remote_request = 0;
    s.file_browser.cancel_drag();
}

pub(crate) fn fresh_tab(s: &mut State) {
    s.file_browser.remote = PaneState::default();
    s.file_browser.remote_history = RemoteHistory::default();
    s.remote_selection.clear();
    s.remote_ready = false;
    s.remote_request = 0;
    s.server_apps.files_search = Default::default();
    s.server_apps.files_open_pending = None;
    s.workbench.files.location = files_tabs::Location::Directory;
}

#[derive(Clone)]
struct FileDrag {
    id: u64,
    remote: bool,
    epoch: u64,
    server: String,
    local_path: String,
    remote_path: String,
    local_request: u64,
    remote_request: u64,
    entries: Vec<FileEntry>,
}
impl BrowserState {
    pub(crate) fn reset_session(&mut self) {
        self.remote_history = RemoteHistory::default();
        self.reset(true);
        self.local.mutation = None;
        self.local.edit_path = None;
        self.local.after_listing = None;
        self.local.name_error.clear();
    }

    pub(crate) fn cancel_path_edit(&mut self, remote: bool) {
        self.pane_mut(remote).editing = false;
    }

    pub(crate) fn cancel_drag(&mut self) {
        self.drag = None;
    }
    fn pane(&self, remote: bool) -> &PaneState {
        if remote { &self.remote } else { &self.local }
    }
    fn pane_mut(&mut self, remote: bool) -> &mut PaneState {
        if remote {
            &mut self.remote
        } else {
            &mut self.local
        }
    }
    pub fn reset(&mut self, remote: bool) {
        self.drag = None;
        *self.pane_mut(remote) = PaneState::default();
    }
}

#[derive(Clone)]
pub(crate) struct FileRow {
    pub entry: FileEntry,
    depth: usize,
    expanded: bool,
    loading: bool,
}
pub(crate) fn icons(s: &State, remote: bool) -> bool {
    files_view::current(s, remote).icons
}
pub(crate) fn prune_selection(s: &mut State, remote: bool) {
    let visible: HashSet<_> = entries(s, remote).into_iter().map(|f| f.path).collect();
    selection(s, remote).retain(|p| visible.contains(p));
    if s.file_browser
        .pane(remote)
        .anchor
        .as_ref()
        .is_some_and(|p| !visible.contains(p))
    {
        s.file_browser.pane_mut(remote).anchor = None;
    }
}
pub(crate) fn view_controls(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    let list = ui.clone();
    let grid = ui;
    row().class("view-switch").fill((
        icon_action("详细列表", Icon::List)
            .class(state.map(move |s| {
                if icons(s, remote) {
                    "icon-button view-list"
                } else {
                    "icon-button view-list active"
                }
                .to_owned()
            }))
            .on_press(move || files_view::change(&list, remote, |v| v.icons = Some(false))),
        icon_action("平铺图标", Icon::Grid)
            .class(state.map(move |s| {
                if icons(s, remote) {
                    "icon-button view-grid active"
                } else {
                    "icon-button view-grid"
                }
                .to_owned()
            }))
            .on_press(move || files_view::change(&grid, remote, |v| v.icons = Some(true))),
    ))
}
pub(crate) fn file_kind(entry: &FileEntry, language: arcnix_core::Language) -> String {
    let translate = |label| i18n::translate_for(label, language);
    if entry.is_symlink {
        translate("链接")
    } else if entry.is_dir {
        translate("文件夹")
    } else if let Some(extension) = Path::new(&entry.name).extension().and_then(|e| e.to_str()) {
        format!("{} {}", extension.to_uppercase(), translate("文件"))
    } else {
        translate("文件")
    }
}
pub(crate) fn timestamp(value: Option<u64>) -> String {
    formatting::timestamp(value)
}
pub(crate) fn information(ui: Ui, remote: bool) -> impl Widget {
    let state = ui.state;
    col().class("file-information").fill((
        text()
            .class("file-information-title")
            .content(state.map(move |s| {
                let all = entries(s, remote);
                let selected = if remote {
                    &s.remote_selection
                } else {
                    &s.local_selection
                };
                let chosen: Vec<_> = all.iter().filter(|f| selected.contains(&f.path)).collect();
                if chosen.len() == 1 {
                    let file = chosen[0];
                    if file.is_dir {
                        format!(
                            "{} · {}",
                            file.name,
                            file_kind(file, i18n::workspace_language(s))
                        )
                    } else {
                        format!(
                            "{} · {} · {} ({} {})",
                            file.name,
                            file_kind(file, i18n::workspace_language(s)),
                            size(file.size),
                            file.size,
                            i18n::translate_for("字节", i18n::workspace_language(s))
                        )
                    }
                } else if !chosen.is_empty() {
                    let bytes = chosen
                        .iter()
                        .filter(|f| !f.is_dir)
                        .fold(0u64, |total, f| total.saturating_add(f.size));
                    format!(
                        "{} · {}",
                        i18n::translate_for(
                            &format!("已选 {} 项", chosen.len()),
                            i18n::workspace_language(s)
                        ),
                        size(bytes)
                    )
                } else {
                    i18n::translate_for(
                        &format!("{} 个项目", all.len()),
                        i18n::workspace_language(s),
                    )
                }
            })),
        text()
            .class("file-information-metadata")
            .content(state.map(move |s| {
                let selected = if remote {
                    &s.remote_selection
                } else {
                    &s.local_selection
                };
                let chosen: Vec<_> = entries(s, remote)
                    .into_iter()
                    .filter(|f| selected.contains(&f.path))
                    .collect();
                if chosen.len() != 1 {
                    return String::new();
                }
                let file = &chosen[0];
                let mut value = format!(
                    "{}: {}",
                    i18n::translate_for("修改时间", i18n::workspace_language(s)),
                    timestamp(file.modified)
                );
                if let Some(mode) = file.permissions {
                    value.push_str(&format!(
                        " · {}: {:04o}",
                        i18n::translate_for("权限", i18n::workspace_language(s)),
                        mode & 0o7777
                    ));
                }
                value
            })),
        text()
            .class("file-information-path")
            .content(state.map(move |s| {
                let selected = if remote {
                    &s.remote_selection
                } else {
                    &s.local_selection
                };
                let chosen: Vec<_> = entries(s, remote)
                    .into_iter()
                    .filter(|f| selected.contains(&f.path))
                    .collect();
                if chosen.len() == 1 {
                    if remote {
                        chosen[0].path.clone()
                    } else {
                        local_display(Path::new(&chosen[0].path))
                    }
                } else {
                    String::new()
                }
            })),
    ))
}
pub(crate) fn path(s: &State, remote: bool) -> &str {
    if remote {
        &s.remote_path
    } else {
        &s.local_path
    }
}
fn selection(s: &mut State, remote: bool) -> &mut HashSet<String> {
    if remote {
        &mut s.remote_selection
    } else {
        &mut s.local_selection
    }
}
fn desktop_listing(s: &State, remote: bool) -> bool {
    remote && s.workbench.active == server_desktop::Tab::Desktop
}

fn search_is_open(s: &State, remote: bool) -> bool {
    desktop_listing(s, remote) && files_search::current(s).open
}

fn sorted(s: &State, remote: bool, entries: &[FileEntry]) -> Vec<FileEntry> {
    let search = search_is_open(s, remote).then(|| files_search::current(s));
    let view = files_view::current(s, remote);
    let query = if remote {
        if desktop_listing(s, remote) {
            search.map_or("", |search| search.query.trim())
        } else {
            s.remote_filter.as_str()
        }
    } else {
        s.local_filter.as_str()
    };
    let range = search.and_then(files_search::Search::date_range);
    visible(entries, query, view.sort, view.descending)
        .into_iter()
        .filter(|f| s.config.settings.show_hidden_files || !f.is_hidden())
        .filter(|f| search.is_none_or(|search| search.accepts(f, range.as_ref())))
        .collect()
}
pub(crate) fn rows(s: &State, remote: bool) -> Vec<FileRow> {
    fn append(
        s: &State,
        remote: bool,
        entries: &[FileEntry],
        depth: usize,
        out: &mut Vec<FileRow>,
    ) {
        if depth > 32 || out.len() >= 20_000 {
            return;
        }
        let pane = s.file_browser.pane(remote);
        for entry in sorted(s, remote, entries) {
            if out.len() >= 20_000 {
                break;
            }
            let expanded = !search_is_open(s, remote)
                && pane.root == path(s, remote)
                && pane.expanded.contains(&entry.path);
            out.push(FileRow {
                depth,
                expanded,
                loading: pane.pending.contains_key(&entry.path),
                entry: entry.clone(),
            });
            if expanded
                && !icons(s, remote)
                && let Some(children) = pane.children.get(&entry.path)
            {
                append(s, remote, children, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    append(
        s,
        remote,
        if remote { &s.remote } else { &s.local },
        0,
        &mut out,
    );
    if let Some(path) = &s.file_browser.pane(remote).edit_path
        && !out.iter().any(|row| row.entry.path == *path)
        && let Some(entry) = (if remote { &s.remote } else { &s.local })
            .iter()
            .find(|entry| entry.path == *path)
    {
        out.push(FileRow {
            entry: entry.clone(),
            depth: 0,
            expanded: false,
            loading: false,
        });
    }
    out
}
pub(crate) fn entries(s: &State, remote: bool) -> Vec<FileEntry> {
    rows(s, remote).into_iter().map(|row| row.entry).collect()
}
pub(crate) fn selected_entries(s: &State, remote: bool) -> Vec<FileEntry> {
    let selected = if remote {
        &s.remote_selection
    } else {
        &s.local_selection
    };
    let all = entries(s, remote);
    transfer_roots(&all, selected, remote)
}
pub(crate) fn transfer_ready(s: &State) -> bool {
    if s.local_desktop.is_some() {
        return false;
    }
    workspace::ready(s, false) && workspace::ready(s, true)
}

fn drag_ready(s: &State) -> bool {
    transfer_ready(s)
        && s.dialog.is_none()
        && s.navigation == navigation::Page::Servers
        && matches!(
            s.workspace_page,
            WorkspacePage::TransferFiles | WorkspacePage::Files(_)
        )
}

fn begin_drag(ui: &Ui, remote: bool, target: &str, id: u64) {
    ui.state.revise(|s| s.file_browser.drag = None);
    if id == 0
        || !ui.state.with(|s| {
            drag_ready(s)
                && s.selected.is_some()
                && entries(s, remote)
                    .iter()
                    .any(|entry| entry.path == target && !entry.is_symlink)
        })
    {
        return;
    }
    // A drag from a selected row keeps the entire selection. An unselected row
    // becomes the sole source, without depending on a preceding click event.
    if !ui.state.with(|s| {
        if remote {
            &s.remote_selection
        } else {
            &s.local_selection
        }
        .contains(target)
    }) {
        select(ui, remote, target, false, false);
    }
    ui.state.revise(|s| {
        s.file_menu = None;
        s.file_context = None;
        s.file_browser.drag = Some(FileDrag {
            id,
            remote,
            epoch: s.epoch,
            server: s.selected.clone().unwrap(),
            local_path: s.local_path.clone(),
            remote_path: s.remote_path.clone(),
            local_request: s.local_request,
            remote_request: s.remote_request,
            entries: selected_entries(s, remote),
        });
    });
}

fn take_transfer_drag(ui: &Ui, remote: bool, id: u64) -> Option<FileDrag> {
    ui.state.revise(|s| {
        let source = s.file_browser.drag.take()?;
        // Bind the gesture to both listings and the SSH session. A refresh,
        // navigation or reconnect during a drag must never retarget a transfer.
        (source.id == id
            && source.remote != remote
            && source.epoch == s.epoch
            && Some(&source.server) == s.selected.as_ref()
            && source.local_path == s.local_path
            && source.remote_path == s.remote_path
            && source.local_request == s.local_request
            && source.remote_request == s.remote_request
            && drag_ready(s))
        .then_some(source)
    })
}

fn drop_transfer(ui: &Ui, remote: bool, id: u64) {
    if let Some(source) = take_transfer_drag(ui, remote, id) {
        ui.transfer_entries(
            if source.remote {
                TransferDirection::Download
            } else {
                TransferDirection::Upload
            },
            source.entries,
        );
    }
}
fn transfer_roots(all: &[FileEntry], selected: &HashSet<String>, remote: bool) -> Vec<FileEntry> {
    let directories: HashSet<_> = all
        .iter()
        .filter(|f| f.is_dir && !f.is_symlink && selected.contains(&f.path))
        .map(|f| f.path.as_str())
        .collect();
    all.iter()
        .filter(|entry| selected.contains(&entry.path) && !entry.is_symlink)
        .filter(|entry| {
            if remote {
                !entry
                    .path
                    .match_indices('/')
                    .any(|(index, _)| directories.contains(&entry.path[..index]))
            } else {
                !Path::new(&entry.path)
                    .ancestors()
                    .skip(1)
                    .filter_map(Path::to_str)
                    .any(|parent| directories.contains(parent))
            }
        })
        .cloned()
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum GestureKind {
    DragStart,
    DragEnd,
    TransferDrop,
    FilesDownload,
    Context,
    Rename,
    Delete,
    DeletePermanently,
    CopyPaths,
    CopyFiles,
    PasteFiles,
    Properties,
    Refresh,
    EditPath,
    FinishName,
    Select,
    Activate,
    Expand,
    Collapse,
    All,
    Clear,
}
#[derive(Deserialize)]
struct Gesture {
    kind: GestureKind,
    #[serde(default)]
    path: String,
    #[serde(default)]
    toggle: bool,
    #[serde(default)]
    range: bool,
    #[serde(default)]
    drag_id: u64,
    #[serde(default)]
    name: Option<String>,
}
pub(crate) fn clear_selection(s: &mut State, remote: bool) {
    selection(s, remote).clear();
    s.file_browser.pane_mut(remote).anchor = None;
}
pub(crate) fn select(ui: &Ui, remote: bool, target: &str, toggle: bool, range: bool) {
    ui.state.revise(|s| {
        let visible = entries(s, remote);
        let Some(index) = visible.iter().position(|f| f.path == target) else {
            return;
        };
        let anchor = s.file_browser.pane(remote).anchor.clone();
        let anchor_index = anchor
            .as_ref()
            .and_then(|anchor| visible.iter().position(|f| &f.path == anchor));
        let start = anchor_index.unwrap_or(index);
        let selected = selection(s, remote);
        if range {
            if !toggle {
                selected.clear();
            }
            selected.extend(
                visible[start.min(index)..=start.max(index)]
                    .iter()
                    .map(|f| f.path.clone()),
            );
        } else if toggle {
            if !selected.remove(target) {
                selected.insert(target.to_owned());
            }
        } else {
            selected.clear();
            selected.insert(target.to_owned());
        }
        if !range || anchor_index.is_none() {
            s.file_browser.pane_mut(remote).anchor = Some(target.to_owned());
        }
    });
}
pub(crate) fn navigate(ui: &Ui, remote: bool, target: String) {
    ui.state.revise(|s| {
        if remote {
            if s.workspace_page.is_files() {
                s.workbench.files.location = files_tabs::Location::Directory;
            }
            s.file_browser.remote_history.requested = None;
            s.remote_path = target;
            s.server_apps.files_tag = None;
            s.server_apps.files_open_pending = None;
        } else {
            s.local_path = target;
        }
    });
    ui.refresh(remote);
}

pub(crate) fn remote_listed(s: &mut State, path: &str) {
    if s.file_browser.remote_history.record(path) {
        s.server_apps.files_search = Default::default();
    }
}

pub(crate) fn remote_browse_failed(s: &mut State) {
    let history = &mut s.file_browser.remote_history;
    history.requested = None;
    if let Some(path) = history.current() {
        s.remote_path = path.to_owned();
        s.remote_ready = true;
    }
}

fn history_button(ui: Ui, backwards: bool) -> impl Widget {
    let state = ui.state;
    icon_action(
        if backwards { "Back" } else { "Forward" },
        Icon::ChevronRight,
    )
    .class(if backwards {
        "icon-button files-history-back"
    } else {
        "icon-button files-history-forward"
    })
    .enabled(state.map(move |s| {
        workspace::ready(s, true) && s.file_browser.remote_history.neighbor(backwards).is_some()
    }))
    .on_press(move || {
        let changed = state.revise(|s| {
            if !workspace::ready(s, true) {
                return false;
            }
            let history = &mut s.file_browser.remote_history;
            let Some(index) = history.neighbor(backwards) else {
                return false;
            };
            history.requested = Some(index);
            s.remote_path = history.paths[index].clone();
            s.server_apps.files_tag = None;
            s.server_apps.files_open_pending = None;
            true
        });
        if changed {
            ui.refresh(true);
        }
    })
}
pub(crate) fn open_remote_file(ui: &Ui, path: &str) {
    file_open::open(ui, path);
}

pub(crate) fn activate(ui: &Ui, remote: bool, target: &str) {
    let entry = ui
        .state
        .with(|s| entries(s, remote).into_iter().find(|f| f.path == target));
    if let Some(entry) = entry {
        if entry.is_symlink {
            if remote {
                file_actions::open_alias(ui, &entry.path);
            }
        } else if entry.is_dir {
            navigate(ui, remote, entry.path);
        } else {
            select(ui, remote, &entry.path, false, false);
            if remote {
                open_remote_file(ui, &entry.path);
            }
        }
    }
}
fn toggle_tree(ui: &Ui, remote: bool, target: &str, force: Option<bool>) {
    let request = ui.state.revise(|s| {
        if search_is_open(s, remote) {
            return None;
        }
        let visible = rows(s, remote);
        let row = visible
            .iter()
            .find(|row| row.entry.path == target && row.entry.is_dir && !row.entry.is_symlink)?;
        if row.depth >= 32 {
            return None;
        }
        let root = path(s, remote).to_owned();
        let pane = s.file_browser.pane_mut(remote);
        if pane.root != root {
            *pane = PaneState {
                root,
                ..Default::default()
            };
        }
        let expand = force.unwrap_or(!pane.expanded.contains(target));
        if !expand {
            pane.expanded.remove(target);
            pane.pending.remove(target);
        } else {
            pane.expanded.insert(target.to_owned());
        }
        // A collapsed branch must not retain invisible selected descendants.
        let visible: HashSet<_> = entries(s, remote).into_iter().map(|f| f.path).collect();
        selection(s, remote).retain(|p| visible.contains(p));
        if s.file_browser
            .pane(remote)
            .anchor
            .as_ref()
            .is_some_and(|p| !visible.contains(p))
        {
            s.file_browser.pane_mut(remote).anchor = None;
        }
        let pane = s.file_browser.pane(remote);
        (expand && !pane.children.contains_key(target) && !pane.pending.contains_key(target))
            .then(|| target.to_owned())
    });
    if let Some(target) = request {
        let serial = ui.send(Action::Children {
            remote,
            path: target.clone(),
        });
        ui.state.revise(|s| {
            s.file_browser
                .pane_mut(remote)
                .pending
                .insert(target, serial);
        });
    }
}
pub(crate) fn can_create(s: &State, remote: bool) -> bool {
    !s.demo_read_only()
        && workspace::ready(s, remote)
        && s.file_pending.is_none()
        && (!remote || !s.system_task_running)
        && s.file_browser.pane(remote).mutation.is_none()
}

pub(crate) fn can_rename(s: &State, remote: bool) -> bool {
    s.dialog.is_none() && can_create(s, remote) && selected_entries(s, remote).len() == 1
}

pub(crate) fn begin_rename(ui: &Ui, remote: bool) {
    ui.state.revise(|s| {
        if !can_rename(s, remote) {
            return;
        }
        let Some(entry) = selected_entries(s, remote).into_iter().next() else {
            return;
        };
        let pane = s.file_browser.pane_mut(remote);
        if pane.edit_path.as_ref() != Some(&entry.path) {
            pane.edit_path = Some(entry.path);
            pane.name_error.clear();
        }
    });
}

pub(crate) fn create(ui: &Ui, remote: bool, directory: bool) {
    if !ui.state.with(|s| can_create(s, remote)) {
        return;
    }
    let serial = ui.send(Action::CreateEntry {
        remote,
        parent: ui.state.with(|s| path(s, remote).to_owned()),
        default_name: i18n::translate(if directory {
            "新建文件夹"
        } else {
            "新建文件"
        }),
        directory,
    });
    ui.state.revise(|s| {
        let pane = s.file_browser.pane_mut(remote);
        pane.mutation = Some(serial);
        pane.edit_path = None;
        pane.name_error.clear();
    });
}

pub(crate) fn refresh_changed_entry(ui: &Ui, remote: bool, serial: u64, focus: NewEntryFocus) {
    if ui
        .state
        .with(|s| s.file_browser.pane(remote).mutation == Some(serial))
    {
        ui.refresh(remote);
        ui.state
            .revise(|s| s.file_browser.pane_mut(remote).after_listing = Some(focus));
    }
}

pub(crate) fn listed(s: &mut State, remote: bool) {
    let focus = s.file_browser.pane_mut(remote).after_listing.take();
    s.file_browser.reset(remote);
    selection(s, remote).clear();
    let Some(focus) = focus else { return };
    let name = match &focus {
        NewEntryFocus::Edit(name) | NewEntryFocus::Select(name) => name,
    };
    let entries = if remote { &s.remote } else { &s.local };
    if let Some(path) = entries
        .iter()
        .find(|entry| entry.name == *name)
        .map(|entry| entry.path.clone())
    {
        selection(s, remote).insert(path.clone());
        let pane = s.file_browser.pane_mut(remote);
        pane.anchor = Some(path.clone());
        if matches!(focus, NewEntryFocus::Edit(_)) {
            pane.edit_path = Some(path);
        }
    }
}

fn finish_name(ui: &Ui, remote: bool, target: &str, name: Option<&str>) {
    let editing = ui.state.with(|s| {
        let pane = s.file_browser.pane(remote);
        pane.edit_path.as_deref() == Some(target) && pane.mutation.is_none()
    });
    if !editing {
        return;
    }
    let original = if remote {
        target.rsplit('/').next()
    } else {
        Path::new(target).file_name().and_then(|n| n.to_str())
    };
    let Some(name) = name.map(str::trim).filter(|name| Some(*name) != original) else {
        ui.state.revise(|s| {
            let pane = s.file_browser.pane_mut(remote);
            pane.edit_path = None;
            pane.name_error.clear();
        });
        return;
    };
    match ui.state.with(|s| rename_action(s, remote, target, name)) {
        Ok(action) => {
            let serial = ui.send(action);
            ui.state.revise(|s| {
                let pane = s.file_browser.pane_mut(remote);
                pane.mutation = Some(serial);
                pane.name_error.clear();
            });
        }
        Err(message) => ui
            .state
            .revise(|s| s.file_browser.pane_mut(remote).name_error = message.to_owned()),
    }
}

fn pending_pane(s: &State, serial: u64) -> Option<bool> {
    [false, true]
        .into_iter()
        .find(|&remote| s.file_browser.pane(remote).mutation == Some(serial))
}

pub(crate) fn mutation_failed(ui: &Ui, serial: u64, message: &str) -> bool {
    let Some(remote) = ui.state.with(|s| pending_pane(s, serial)) else {
        return false;
    };
    let editing = ui.state.revise(|s| {
        let pane = s.file_browser.pane_mut(remote);
        pane.mutation = None;
        if pane.edit_path.is_some() {
            pane.name_error = message.to_owned();
            true
        } else {
            false
        }
    });
    if editing {
        ui.log(message);
    } else {
        ui.error(message);
    }
    true
}
pub(crate) fn accept_children(
    ui: &Ui,
    remote: bool,
    parent: String,
    serial: u64,
    result: std::result::Result<Vec<FileEntry>, String>,
) {
    let error = ui.state.revise(|s| {
        let root = path(s, remote).to_owned();
        let pane = s.file_browser.pane_mut(remote);
        if pane.root != root || pane.pending.get(&parent) != Some(&serial) {
            return None;
        }
        pane.pending.remove(&parent);
        match result {
            Ok(entries) => {
                pane.children.insert(parent, entries);
                None
            }
            Err(error) => {
                pane.expanded.remove(&parent);
                Some(error)
            }
        }
    });
    if let Some(error) = error {
        ui.error(format!("读取目录失败：{error}"));
    }
}
fn gesture(ui: &Ui, remote: bool, value: String) {
    if value.len() > 32_768 {
        return;
    }
    let Ok(event) = serde_json::from_str::<Gesture>(&value) else {
        return;
    };
    match event.kind {
        GestureKind::DragStart => begin_drag(ui, remote, &event.path, event.drag_id),
        GestureKind::DragEnd => ui.state.revise(|s| {
            if s.file_browser
                .drag
                .as_ref()
                .is_some_and(|source| source.id == event.drag_id)
            {
                s.file_browser.drag = None;
            }
        }),
        GestureKind::TransferDrop => drop_transfer(ui, remote, event.drag_id),
        GestureKind::FilesDownload => {
            if remote
                && ui.state.with(|s| s.workspace_page.is_files())
                && let Some(source) = take_transfer_drag(ui, false, event.drag_id)
            {
                files_transfer::choose_download(ui, source.entries);
            }
        }
        GestureKind::Context => file_context::open(ui, remote, &event.path),
        GestureKind::Delete => trash_ui::move_items(ui, remote),
        GestureKind::DeletePermanently => file_context::begin_delete(ui, remote),
        GestureKind::Rename => begin_rename(ui, remote),
        GestureKind::CopyPaths => file_context::copy_paths(ui, remote, false),
        GestureKind::CopyFiles if remote => file_actions::copy(ui),
        GestureKind::PasteFiles if remote => file_actions::paste(ui),
        GestureKind::CopyFiles | GestureKind::PasteFiles => (),
        GestureKind::Properties => file_context::properties(ui, remote),
        GestureKind::Refresh => {
            if ui
                .state
                .with(|s| !remote || (s.connected && s.sftp_available && !s.system_task_running))
            {
                ui.refresh(remote);
            }
        }
        GestureKind::EditPath => ui.state.revise(|s| {
            if navigation::workspace_visible(s)
                && s.dialog.is_none()
                && matches!(
                    s.workspace_page,
                    WorkspacePage::TransferFiles | WorkspacePage::Files(_)
                )
                && (!remote || s.connected && s.sftp_available)
            {
                begin_path_edit(s, remote);
            }
        }),
        GestureKind::FinishName => finish_name(ui, remote, &event.path, event.name.as_deref()),
        GestureKind::Select => select(ui, remote, &event.path, event.toggle, event.range),
        GestureKind::Activate => activate(ui, remote, &event.path),
        GestureKind::Expand => toggle_tree(ui, remote, &event.path, Some(true)),
        GestureKind::Collapse => toggle_tree(ui, remote, &event.path, Some(false)),
        GestureKind::All => ui.state.revise(|s| {
            *selection(s, remote) = entries(s, remote).into_iter().map(|f| f.path).collect();
        }),
        GestureKind::Clear => ui.state.revise(|s| {
            selection(s, remote).clear();
        }),
    }
}

pub(crate) fn entry_name_class<T: 'static>(
    state: Cage<State>,
    entry: Cage<T>,
    file: fn(&T) -> &FileEntry,
    files: bool,
) -> Bond<String> {
    state.map(move |s| {
        entry.with(|entry| {
            let file = file(entry);
            if files
                && !file.is_symlink
                && let Some(tag) = files_ui::tag_class(s, &file.path)
            {
                format!("file-name file-tagged {tag}")
            } else {
                "file-name".into()
            }
        })
    })
}

pub(crate) fn entry_icon_class(s: &State, file: &FileEntry, files: bool) -> String {
    let mut class = if files
        && file.is_dir
        && !file.is_symlink
        && let Some(class) = files_ui::folder_icon_class(s, &file.path)
    {
        format!("action-glyph file-entry-icon file-folder-symbol folder-custom-mark {class}")
    } else {
        let icon = files_search::file_icon(&file.name, file.is_dir);
        format!("action-glyph file-entry-icon glyph-{}", icon.name())
    };
    if file.is_hidden() {
        class.push_str(" hidden-file-icon");
    }
    class
}

pub(crate) fn entry_name<T: 'static>(
    state: Cage<State>,
    entry: Cage<T>,
    file: fn(&T) -> &FileEntry,
    files: bool,
) -> impl IntoChildren {
    (
        text()
            .class(state.map(move |s| entry.with(|entry| entry_icon_class(s, file(entry), files))))
            .content(""),
        text()
            .class("file-label")
            .content(entry.map(move |entry| file(entry).name.clone())),
        text().class("folder-color-dot").content(""),
    )
}

pub(crate) fn listing(ui: Ui, remote: bool, files: bool) -> impl Widget {
    let state = ui.state;
    let input_ui = ui.clone();
    col()
        .class(state.map(move |s| {
            if rows(s, remote).is_empty() {
                "file-list-container file-list-empty"
            } else {
                "file-list-container"
            }
            .to_owned()
        }))
        .focus(FocusMode::Programmatic)
        .fill((
            // Native pointer modifiers are absent from Glory 0.7's Activate event.
            // This private bridge carries a finite gesture; Rust validates the path
            // against the current visible rows before changing selection/navigation.
            input()
                .class("file-gesture-input")
                .value("")
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        gesture(&input_ui, remote, value);
                    }
                }),
            col().class("file-list").fill(
                each(state.map(move |s| rows(s, remote)))
                    .key(|row| row.entry.path.clone())
                    .children(move |item| {
                        let entry = item.value;
                        let open = ui.clone();
                        let expand = ui.clone();
                        row()
                            .name(entry.map(|r| r.entry.path.clone()))
                            .class(state.map(move |s| {
                                let selected = if remote {
                                    &s.remote_selection
                                } else {
                                    &s.local_selection
                                };
                                let entry = entry.get();
                                let mut class = String::from("file-row");
                                if selected.contains(&entry.entry.path) {
                                    class.push_str(" row-selected");
                                }
                                if !entry.entry.is_symlink {
                                    class.push_str(" file-draggable");
                                    if entry.entry.is_dir {
                                        class.push_str(" files-directory-row");
                                    }
                                }
                                class
                            }))
                            .style(entry.map(|r| {
                                Style::new().padding(Edges {
                                    left: 10.0 + r.depth as f64 * 18.0,
                                    right: 10.0,
                                    top: 0.0,
                                    bottom: 0.0,
                                })
                            }))
                            .fill((
                                glory::ui::button()
                                    .class(entry.map(|r| {
                                        if r.loading {
                                            "file-expander file-expander-loading"
                                        } else if r.entry.is_dir {
                                            "file-expander"
                                        } else {
                                            "file-expander file-expander-empty"
                                        }
                                        .to_owned()
                                    }))
                                    .name(i18n::bind_in(
                                        files,
                                        entry.map(|r| {
                                            format!(
                                                "{} {}",
                                                if r.expanded { "收起" } else { "展开" },
                                                r.entry.name
                                            )
                                        }),
                                    ))
                                    .label("")
                                    .semantics(entry.map(|r| {
                                        Some(Box::new(glory::render::Semantics {
                                            expanded: Some(r.expanded),
                                            ..Default::default()
                                        }))
                                    }))
                                    .fill(icons::disclosure_icon(entry.map(|r| r.expanded)))
                                    .enabled(entry.map(|r| r.entry.is_dir && !r.entry.is_symlink))
                                    .on(glory::render::EventKind::Activate, move |_| {
                                        toggle_tree(&expand, remote, &entry.get().entry.path, None)
                                    }),
                                sapid::button()
                                    .class(entry_name_class(state, entry, |row| &row.entry, files))
                                    .name(entry.map(|r| r.entry.name.clone()))
                                    .label("")
                                    .fill(entry_name(state, entry, |row| &row.entry, files))
                                    .on_press(move || {
                                        activate(&open, remote, &entry.get().entry.path)
                                    }),
                                when(
                                    state.map(move |s| {
                                        s.file_browser.pane(remote).edit_path.as_deref()
                                            == Some(entry.get().entry.path.as_str())
                                    }),
                                    move || {
                                        input()
                                            .class(entry.map(|r| {
                                                if r.entry.is_dir {
                                                    "file-inline-name"
                                                } else {
                                                    "file-inline-name file-inline-stem"
                                                }
                                                .to_owned()
                                            }))
                                            .name(i18n::bind_in(files, "新名称"))
                                            .value(entry.get().entry.name)
                                    },
                                ),
                                when(
                                    state.map(move |s| file_columns::visible(s, remote).size),
                                    move || {
                                        text().class("file-size").content(i18n::bind_in(
                                            files,
                                            entry.map(|r| {
                                                if r.entry.is_dir {
                                                    "—".into()
                                                } else if r.entry.is_symlink {
                                                    "链接".into()
                                                } else {
                                                    size(r.entry.size)
                                                }
                                            }),
                                        ))
                                    },
                                ),
                                when(
                                    state.map(move |s| file_columns::visible(s, remote).kind),
                                    move || {
                                        text().class("file-type").content(state.map(move |s| {
                                            file_kind(
                                                &entry.get().entry,
                                                i18n::workspace_language(s),
                                            )
                                        }))
                                    },
                                ),
                                when(
                                    state.map(move |s| file_columns::visible(s, remote).modified),
                                    move || {
                                        text()
                                            .class("file-modified")
                                            .content(entry.map(|r| modified(r.entry.modified)))
                                    },
                                ),
                                when(
                                    state
                                        .map(move |s| file_columns::visible(s, remote).permissions),
                                    move || {
                                        text().class("file-permissions").content(entry.map(|r| {
                                            r.entry
                                                .permissions
                                                .map(|m| format!("{:04o}", m & 0o7777))
                                                .unwrap_or_else(|| "—".into())
                                        }))
                                    },
                                ),
                            ))
                    }),
            ),
            when(
                state.map(move |s| !s.file_browser.pane(remote).name_error.is_empty()),
                move || {
                    text()
                        .class("form-error file-name-error")
                        .content(i18n::bind_in(
                            files,
                            state.map(move |s| s.file_browser.pane(remote).name_error.clone()),
                        ))
                },
            ),
        ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Crumb {
    label: String,
    path: String,
}
fn breadcrumbs(path: &str, remote: bool) -> Vec<Crumb> {
    let mut result: Vec<Crumb> = Vec::new();
    if remote {
        let mut current = if path.starts_with('/') {
            result.push(Crumb {
                label: "/".into(),
                path: "/".into(),
            });
            "/".to_owned()
        } else {
            String::new()
        };
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if !current.is_empty() && current != "/" {
                current.push('/');
            }
            current.push_str(part);
            result.push(Crumb {
                label: part.into(),
                path: current.clone(),
            });
        }
    } else {
        let mut current = PathBuf::new();
        for component in Path::new(path).components() {
            current.push(component.as_os_str());
            if let Component::RootDir = component
                && let Some(previous) = result.last_mut()
            {
                previous.path = local_display(&current);
                continue;
            }
            let target = local_display(&current);
            let label = match component {
                Component::Prefix(_) | Component::RootDir => target.clone(),
                _ => component.as_os_str().to_string_lossy().into_owned(),
            };
            result.push(Crumb {
                label,
                path: target,
            });
        }
    }
    if result.is_empty() {
        result.push(Crumb {
            label: ".".into(),
            path: ".".into(),
        });
    }
    result
}

pub(crate) fn breadcrumb_bar(
    state: Cage<State>,
    location: impl Fn(&State) -> &str + 'static,
    remote: bool,
    name: impl Into<Vary<String>>,
    hidden: impl Fn(&State) -> bool + 'static,
    enabled: impl Into<Vary<bool>>,
    navigate: impl Fn(String) + Clone + 'static,
) -> impl Widget {
    let enabled = enabled.into();
    let crumbs = state
        .map(move |s| breadcrumbs(location(s), remote))
        .with_partial_eq();
    row()
        .class(state.map(move |s| {
            if hidden(s) {
                "breadcrumbs hidden"
            } else {
                "breadcrumbs"
            }
            .to_owned()
        }))
        .name(name)
        .fill(each(crumbs).key(|c| c.path.clone()).children(move |item| {
            let crumb = item.value;
            let navigate = navigate.clone();
            fragment((
                when(item.index.map(|index| *index != 0), || {
                    icons::glyph(Icon::ChevronRight)
                }),
                sapid::button()
                    .class(state.map(move |_| {
                        if crumbs.with(|parts| {
                            parts
                                .last()
                                .is_some_and(|part| crumb.with(|c| c.path == part.path))
                        }) {
                            "breadcrumb current"
                        } else {
                            "breadcrumb"
                        }
                        .to_owned()
                    }))
                    .label(crumb.map(|c| c.label.clone()))
                    .name(crumb.map(|c| c.path.clone()))
                    .enabled(enabled.clone())
                    .on_press(move || navigate(crumb.get().path)),
            ))
        }))
}
fn begin_path_edit(s: &mut State, remote: bool) {
    if search_is_open(s, remote) {
        s.server_apps.files_search = Default::default();
    }
    if !s.file_browser.pane(remote).editing {
        let current = path(s, remote).to_owned();
        let pane = s.file_browser.pane_mut(remote);
        pane.draft = current;
        pane.editing = true;
    }
}

pub(crate) fn pathbar(
    ui: Ui,
    remote: bool,
    files: bool,
    trailing: impl IntoChildren,
) -> impl Widget {
    let state = ui.state;
    let edit_ui = ui.clone();
    let enter_ui = ui.clone();
    let cancel_ui = ui.clone();
    let up_ui = ui.clone();
    let refresh_ui = ui.clone();
    let copy_ui = ui.clone();
    let crumb_ui = ui.clone();
    let history_ui = ui.clone();
    let search_ui = ui.clone();
    let toggle_search = ui.clone();
    let options_ui = ui.clone();
    let enabled = state.map(move |s| !remote || s.connected && s.sftp_available);
    row().class("pathbar").fill((
        when(files, move || {
            row().class("files-history").fill((
                history_button(history_ui.clone(), true),
                history_button(history_ui.clone(), false),
            ))
        }),
        when(!files, move || {
            let up_ui = up_ui.clone();
            icon_action(
                if remote {
                    "远程上级目录"
                } else {
                    "本地上级目录"
                },
                Icon::Up,
            )
            .enabled(enabled)
            .on_press(move || {
                let target = up_ui.state.with(|s| {
                    if remote {
                        let p = s.remote_path.trim_end_matches('/');
                        p.rsplit_once('/')
                            .map(|(parent, _)| {
                                if parent.is_empty() {
                                    "/".into()
                                } else {
                                    parent.to_owned()
                                }
                            })
                            .unwrap_or_else(|| ".".into())
                    } else {
                        Path::new(&s.local_path)
                            .parent()
                            .map(local_display)
                            .unwrap_or_else(|| s.local_path.clone())
                    }
                });
                navigate(&up_ui, remote, target);
            })
        }),
        col().class("path-location").fill((
            breadcrumb_bar(
                state,
                move |s| path(s, remote),
                remote,
                i18n::bind_in(
                    files,
                    if remote {
                        "远程路径"
                    } else {
                        "本地路径"
                    },
                ),
                move |s| {
                    s.file_browser.pane(remote).editing || files && files_search::current(s).open
                },
                enabled,
                move |target| navigate(&crumb_ui, remote, target),
            ),
            input()
                .class(state.map(move |s| {
                    if s.file_browser.pane(remote).editing {
                        "path-input"
                    } else {
                        "path-input hidden"
                    }
                    .to_owned()
                }))
                .name(i18n::bind_in(
                    files,
                    if remote {
                        "远程路径"
                    } else {
                        "本地路径"
                    },
                ))
                .value(state.map(move |s| s.file_browser.pane(remote).draft.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| s.file_browser.pane_mut(remote).draft = value);
                    }
                })
                .on(commit, move |_| {
                    let target = enter_ui
                        .state
                        .with(|s| s.file_browser.pane(remote).draft.clone());
                    navigate(&enter_ui, remote, target);
                })
                .on(keyboard, move |event| {
                    if let Event::Keyboard {
                        command: glory::render::KeyboardCommand::Dismiss,
                        ..
                    } = event
                    {
                        cancel_ui
                            .state
                            .revise(|s| s.file_browser.pane_mut(remote).editing = false);
                    }
                }),
            when(
                state.map(move |s| files && files_search::current(s).open),
                move || files_search::field(search_ui.clone()),
            ),
        )),
        when(files, move || files_ui::favorite_button(ui.clone())),
        when(!files, move || {
            icon_action(
                if remote {
                    "编辑远程路径"
                } else {
                    "编辑本地路径"
                },
                Icon::Edit,
            )
            .class("icon-button path-edit")
            .on_press(move || {
                edit_ui.state.revise(|s| {
                    if s.file_browser.pane(remote).editing {
                        s.file_browser.pane_mut(remote).editing = false;
                    } else {
                        begin_path_edit(s, remote);
                    }
                });
            })
        }),
        icon_action(
            if remote {
                "复制远程路径"
            } else {
                "复制本地路径"
            },
            Icon::Copy,
        )
        .class(state.map(move |s| {
            if s.file_browser.pane(remote).copied {
                "icon-button path-copy copied"
            } else {
                "icon-button path-copy"
            }
            .to_owned()
        }))
        .on_press(move || {
            let value = copy_ui.state.with(|s| path(s, remote).to_owned());
            match crate::clipboard::copy(&value) {
                Ok(()) => copy_ui
                    .state
                    .revise(|s| s.file_browser.pane_mut(remote).copied = true),
                Err(error) => copy_ui.error(format!("复制路径失败：{error}")),
            }
        }),
        when(files, move || files_search::toggle(toggle_search.clone())),
        icon_action(
            if remote {
                "刷新远程目录"
            } else {
                "刷新本地目录"
            },
            Icon::Refresh,
        )
        .class("icon-button path-refresh")
        .enabled(state.map(move |s| !remote || s.connected && s.sftp_available))
        .on_press(move || refresh_ui.refresh(remote)),
        fragment(trailing),
        when(files, move || files_search::options(options_ui.clone())),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_selection_transfers_parent_once_without_confusing_sibling_prefixes() {
        let entries: Vec<_> = [
            ("/a", true),
            ("/a/child", false),
            ("/ab/file", false),
            ("/a\\b/file", false),
        ]
        .into_iter()
        .map(|(path, is_dir)| FileEntry {
            hidden_attribute: false,
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            is_dir,
            is_symlink: false,
            size: 4,
            modified: None,
            permissions: None,
        })
        .collect();
        let selected = entries.iter().map(|f| f.path.clone()).collect();
        let result = transfer_roots(&entries, &selected, true);
        assert_eq!(
            result.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["/a", "/ab/file", "/a\\b/file"]
        );
    }
    #[test]
    fn posix_breadcrumbs_keep_ancestor_targets() {
        let parts = breadcrumbs("/srv/my data/中文/", true);
        assert_eq!(
            parts.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            ["/", "/srv", "/srv/my data", "/srv/my data/中文"]
        );
        assert_eq!(breadcrumbs("/", true)[0].path, "/");
        assert_eq!(breadcrumbs(".", true)[0].path, ".");
    }
    #[cfg(windows)]
    #[test]
    fn windows_breadcrumbs_preserve_drive_and_share_roots() {
        let parts = breadcrumbs(r"D:\Works\中文", false);
        assert_eq!(
            parts.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            [r"D:\", r"D:\Works", r"D:\Works\中文"]
        );
        let parts = breadcrumbs(r"\\server\share\folder", false);
        assert_eq!(parts[0].path, r"\\server\share\");
        assert_eq!(parts.last().unwrap().path, r"\\server\share\folder");
    }
}
