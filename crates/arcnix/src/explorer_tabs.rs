use std::collections::BTreeMap;
use std::sync::Arc;

use arcnix_core::server_apps::DesktopWindow;
use glory::render::{DragCapture, DragPhase, EventKind, NodeId, Query, QueryValue, Semantics};
use serde::Deserialize;

use super::*;

#[derive(Clone)]
struct Tab {
    id: u32,
    saved: Option<Arc<file_browser::TabSnapshot>>,
}

#[derive(Clone)]
struct Window {
    tabs: Vec<Tab>,
    selected: u32,
    layout: DesktopWindow,
}

#[derive(Clone, Copy)]
struct TabDrag {
    from: NodeId,
    window: u32,
    tab: u32,
    epoch: u64,
}

#[derive(Clone, Default)]
pub(super) struct Windows {
    windows: BTreeMap<u32, Window>,
    live: Option<u32>,
    next_id: u32,
    drag: Option<TabDrag>,
    pub trash: bool,
}

impl Windows {
    pub(super) fn current(&self) -> Option<u32> {
        self.live.or_else(|| self.windows.keys().next().copied())
    }

    pub(super) fn layout(&self, id: u32) -> Option<DesktopWindow> {
        Some(self.windows.get(&id)?.layout)
    }

    pub(super) fn layout_mut(&mut self, id: u32) -> &mut DesktopWindow {
        &mut self.windows.get_mut(&id).expect("open Files window").layout
    }

    fn next(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn insert(&mut self, id: u32, tab: Tab) {
        self.windows.insert(
            id,
            Window {
                selected: tab.id,
                tabs: vec![tab],
                layout: DesktopWindow::default(),
            },
        );
    }
}

fn save(s: &mut State) {
    let files = &s.workbench.files;
    let Some(id) = files.live else {
        return;
    };
    let snapshot = Arc::new(file_browser::capture_tab(s));
    if let Some(window) = s.workbench.files.windows.get_mut(&id)
        && let Some(tab) = window.tabs.iter_mut().find(|tab| tab.id == window.selected)
    {
        tab.saved = Some(snapshot);
    }
}

fn load(s: &mut State, id: u32) {
    let w = &s.workbench.files.windows[&id];
    let saved = w
        .tabs
        .iter()
        .find(|tab| tab.id == w.selected)
        .and_then(|tab| tab.saved.clone());
    s.workbench.files.live = Some(id);
    if let Some(saved) = saved {
        file_browser::restore_tab(s, &saved);
    }
}

pub(super) fn focus(s: &mut State, id: u32) {
    if s.workbench.files.live == Some(id) {
        return;
    }
    save(s);
    if !s.workbench.files.windows.contains_key(&id) {
        let tab = s.workbench.files.next();
        s.workbench.files.insert(
            id,
            Tab {
                id: tab,
                saved: None,
            },
        );
    }
    load(s, id);
}

pub(super) fn remove(s: &mut State, id: u32) {
    s.workbench.files.windows.remove(&id);
    if s.workbench.files.live == Some(id) {
        s.workbench.files.live = None;
        s.workbench.files.trash = false;
    }
}

pub(super) fn trash_active(s: &State) -> bool {
    s.workspace_page.is_explorer() && s.workbench.files.trash
}

fn available(s: &State) -> bool {
    server_desktop::visible(s)
        && s.dialog.is_none()
        && s.file_pending.is_none()
        && s.fingerprint.is_none()
        && s.sftp_available
}

fn after_change(ui: &Ui) {
    ui.state.revise(|s| {
        s.file_context = None;
        s.file_menu = None;
    });
    if ui.state.with(trash_active) {
        trash_ui::enter(ui);
    } else if ui
        .state
        .with(|s| s.workspace_page.is_explorer() && !s.remote_ready)
    {
        ui.refresh(true);
    }
}

pub(super) fn new_tab(ui: &Ui, id: u32) {
    open_tab(ui, id, None);
}

pub(super) fn open_tab(ui: &Ui, id: u32, path: Option<String>) {
    if !ui
        .state
        .with(|s| available(s) && s.workbench.window_visible(WorkspacePage::Explorer(id)))
    {
        return;
    }
    ui.focus_desktop_app(WorkspacePage::Explorer(id));
    ui.state.revise(|s| {
        save(s);
        let tab = s.workbench.files.next();
        let window = s
            .workbench
            .files
            .windows
            .get_mut(&id)
            .expect("open Files window");
        window.tabs.push(Tab {
            id: tab,
            saved: None,
        });
        window.selected = tab;
        file_browser::fresh_tab(s);
    });
    if let Some(path) = path {
        file_browser::navigate(ui, true, path);
    } else {
        after_change(ui);
    }
}

fn select(ui: &Ui, window: u32, tab: u32) {
    if !ui.state.with(|s| {
        available(s)
            && s.workbench.window_visible(WorkspacePage::Explorer(window))
            && s.workbench
                .files
                .windows
                .get(&window)
                .is_some_and(|w| w.tabs.iter().any(|item| item.id == tab))
    }) {
        return;
    }
    ui.focus_desktop_app(WorkspacePage::Explorer(window));
    ui.state.revise(|s| {
        if s.workbench.files.windows[&window].selected != tab {
            save(s);
            s.workbench.files.windows.get_mut(&window).unwrap().selected = tab;
            load(s, window);
        }
    });
    after_change(ui);
}

fn close(ui: &Ui, id: u32, tab: u32) {
    if !ui.state.with(available) {
        return;
    }
    let empty = ui.state.revise(|s| {
        let files = &mut s.workbench.files;
        let Some(w) = files.windows.get_mut(&id) else {
            return false;
        };
        let Some(index) = w.tabs.iter().position(|t| t.id == tab) else {
            return false;
        };
        w.tabs.remove(index);
        if w.tabs.is_empty() {
            return true;
        }
        if w.selected == tab {
            w.selected = w.tabs[index.min(w.tabs.len() - 1)].id;
            if files.live == Some(id) {
                load(s, id);
            }
        }
        false
    });
    if empty {
        ui.finish_desktop_close(WorkspacePage::Explorer(id));
    }
    after_change(ui);
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Gesture {
    New { window: u32 },
    Close { window: u32, tab: u32 },
}

fn move_tab(ui: &Ui, source: u32, tab: u32, target: Option<u32>, before: Option<u32>) {
    let moved = ui.state.revise(|s| {
        let files = &s.workbench.files;
        let source_page = WorkspacePage::Explorer(source);
        if !available(s)
            || !s.workbench.window_visible(source_page)
            || target.is_some_and(|id| !s.workbench.window_visible(WorkspacePage::Explorer(id)))
        {
            return None;
        }
        let index = files
            .windows
            .get(&source)?
            .tabs
            .iter()
            .position(|t| t.id == tab)?;
        if target == Some(source) && before == Some(tab) {
            return None;
        }
        save(s);
        let files = &mut s.workbench.files;
        let from = files.windows.get_mut(&source)?;
        let moved = from.tabs.remove(index);
        let empty = from.tabs.is_empty();
        if !empty && from.selected == tab {
            from.selected = from.tabs[index.min(from.tabs.len() - 1)].id;
        }
        let destination = if let Some(id) = target {
            let w = files.windows.get_mut(&id)?;
            let index = before
                .and_then(|anchor| w.tabs.iter().position(|t| t.id == anchor))
                .unwrap_or(w.tabs.len());
            w.tabs.insert(index, moved);
            w.selected = tab;
            id
        } else {
            let id = files.next();
            files.insert(id, moved);
            id
        };
        // The old live tab was already captured before editing the window lists.
        files.live = None;
        if empty && destination != source {
            server_desktop::remove_window(s, source_page);
        }
        load(s, destination);
        Some(destination)
    });
    if let Some(id) = moved {
        ui.focus_desktop_app(WorkspacePage::Explorer(id));
        after_change(ui);
    }
}

fn drag_source(ui: &Ui, window: u32, tab: u32, event: Event) {
    match event {
        Event::Drag {
            from,
            phase: DragPhase::Start,
            ..
        } => {
            ui.state.revise(|s| {
                if available(s) && s.workbench.window_visible(WorkspacePage::Explorer(window)) {
                    s.workbench.files.drag = Some(TabDrag {
                        from,
                        window,
                        tab,
                        epoch: s.epoch,
                    });
                }
            });
        }
        Event::Drag {
            from,
            phase: DragPhase::Cancel,
            ..
        } => {
            take_drag(ui, from);
        }
        _ => {}
    }
}

fn take_drag(ui: &Ui, from: NodeId) -> Option<TabDrag> {
    ui.state.revise(|s| {
        let source = s.workbench.files.drag?;
        (source.from == from)
            .then(|| s.workbench.files.drag.take())
            .flatten()
    })
}

fn drop_position(event: Event) -> Option<(NodeId, f64)> {
    if let Event::Drag {
        from,
        phase: DragPhase::Drop,
        x,
        ..
    } = event
    {
        Some((from, x))
    } else {
        None
    }
}

pub(super) fn drop_on_window(ui: &Ui, window: Option<u32>, event: Event) {
    if let Some((from, _)) = drop_position(event)
        && let Some(source) = take_drag(ui, from)
        && window != Some(source.window)
    {
        move_tab(ui, source.window, source.tab, window, None);
    }
}

fn drop_on_strip(ui: &Ui, window: u32, event: Event) {
    if let Some((from, _)) = drop_position(event)
        && let Some(source) = take_drag(ui, from)
    {
        move_tab(ui, source.window, source.tab, Some(window), None);
    }
}

fn drop_on_tab(
    ui: &Ui,
    window: u32,
    tab: u32,
    scope: glory::ui::WeakScope,
    node: Option<NodeId>,
    event: Event,
) {
    let Some((from, x)) = drop_position(event) else {
        return;
    };
    let Some(source) = take_drag(ui, from) else {
        return;
    };
    if window == source.window && tab == source.tab {
        return;
    }
    let (Some(scope), Some(node)) = (scope.upgrade(), node) else {
        return;
    };
    let ui = ui.clone();
    let _ = scope.request(Query::Bounds(node), move |result| {
        let Ok(QueryValue::Bounds(bounds)) = result else {
            return;
        };
        let before = ui.state.with(|s| {
            if s.epoch != source.epoch {
                return None;
            }
            let tabs = &s.workbench.files.windows.get(&window)?.tabs;
            let index = tabs.iter().position(|item| item.id == tab)?;
            let next = index + usize::from(x > bounds.x + bounds.width / 2.0);
            Some(tabs.get(next).map(|item| item.id))
        });
        if let Some(before) = before {
            move_tab(&ui, source.window, source.tab, Some(window), before);
        }
    });
}

pub(super) fn bridge(ui: Ui) -> impl Widget {
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
                }
            }
        })
}

fn title(s: &State, window: u32, id: u32) -> String {
    let files = &s.workbench.files;
    let Some(w) = files.windows.get(&window) else {
        return String::new();
    };
    if files.live == Some(window) && w.selected == id {
        file_browser::live_tab_title(s)
    } else {
        w.tabs
            .iter()
            .find(|tab| tab.id == id)
            .and_then(|tab| tab.saved.as_ref())
            .map_or_else(|| i18n::translate("Files"), |view| view.title())
    }
}

pub(super) fn tab_strip(ui: Ui, id: u32) -> impl Widget {
    let state = ui.state;
    let choose = ui.clone();
    let strip = ui.clone();
    row()
        .class("files-tabs")
        .name(i18n::bind("File tabs"))
        .on(EventKind::Drag, move |event| {
            drop_on_strip(&strip, id, event)
        })
        .fill(
            each(state.map(move |s| {
                s.workbench
                    .files
                    .windows
                    .get(&id)
                    .map(|w| w.tabs.clone())
                    .unwrap_or_default()
            }))
            .key(|t| t.id)
            .children(move |t| {
                let tab = t.value.get().id;
                let choose = choose.clone();
                glory::ui::widget(move |scope| {
                    let remove = choose.clone();
                    let select_ui = choose.clone();
                    let source = choose.clone();
                    let target = choose.clone();
                    let source_drop = choose.clone();
                    let ref_node = scope.cage(None);
                    let target_scope = scope.downgrade();
                    let source_scope = target_scope.clone();
                    row()
                        .node_ref(ref_node)
                        .class(state.map(move |s| {
                            let selected = s
                                .workbench
                                .files
                                .windows
                                .get(&id)
                                .is_some_and(|w| w.selected == tab);
                            let dragging = s
                                .workbench
                                .files
                                .drag
                                .is_some_and(|item| item.window == id && item.tab == tab);
                            format!(
                                "files-tab{}{}",
                                if selected { " selected" } else { "" },
                                if dragging { " files-tab-dragging" } else { "" }
                            )
                        }))
                        .on(EventKind::Drag, move |event| {
                            drop_on_tab(
                                &target,
                                id,
                                tab,
                                target_scope.clone(),
                                ref_node.get(),
                                event,
                            )
                        })
                        .fill((
                            text().class("files-tab-id hidden").content(tab.to_string()),
                            row()
                                .class("files-tab-drag-source")
                                .name(state.map(move |s| title(s, id, tab)))
                                .semantics(Some(Box::new(Semantics {
                                    drag_capture: Some(DragCapture::ThresholdPreview),
                                    ..Default::default()
                                })))
                                .on(EventKind::Drag, move |event| {
                                    if matches!(
                                        event,
                                        Event::Drag {
                                            phase: DragPhase::Drop,
                                            ..
                                        }
                                    ) {
                                        drop_on_tab(
                                            &source_drop,
                                            id,
                                            tab,
                                            source_scope.clone(),
                                            ref_node.get(),
                                            event,
                                        );
                                    } else {
                                        drag_source(&source, id, tab, event);
                                    }
                                })
                                .fill(
                                    sapid::button()
                                        .class("files-tab-select")
                                        .name(state.map(move |s| title(s, id, tab)))
                                        .pressed(state.map(move |s| {
                                            s.workbench
                                                .files
                                                .windows
                                                .get(&id)
                                                .is_some_and(|w| w.selected == tab)
                                        }))
                                        .fill(text().content(state.map(move |s| title(s, id, tab))))
                                        .on_press(move || select(&select_ui, id, tab)),
                                ),
                            close_action("Close file tab")
                                .class("icon-button files-tab-close")
                                .on_press(move || close(&remove, id, tab)),
                        ))
                        .show_in(scope)
                })
            }),
        )
}

pub(super) fn window_content(ui: Ui, id: u32) -> impl Widget {
    let state = ui.state;
    let live = ui.clone();
    col().class("files-window-content").fill((
        text()
            .class("files-window-id hidden")
            .content(id.to_string()),
        when(
            state.map(move |s| s.workbench.files.live == Some(id)),
            move || {
                fragment((
                    file_context::desktop_menu_dismiss(live.clone()),
                    workspace::explorer(live.clone()),
                ))
            },
        ),
        when(
            state.map(move |s| s.workbench.files.live != Some(id)),
            move || {
                col().class("files-inactive-preview").fill((
                    text()
                        .class("files-inactive-path")
                        .content(state.map(move |s| {
                            s.workbench
                                .files
                                .windows
                                .get(&id)
                                .and_then(|w| w.tabs.iter().find(|t| t.id == w.selected))
                                .and_then(|t| t.saved.as_ref())
                                .map(|v| v.location())
                                .unwrap_or_default()
                        })),
                    each(state.map(move |s| {
                        s.workbench
                            .files
                            .windows
                            .get(&id)
                            .and_then(|w| w.tabs.iter().find(|t| t.id == w.selected))
                            .and_then(|t| t.saved.as_ref())
                            .map(|v| v.preview_entries())
                            .unwrap_or_default()
                    }))
                    .key(|e| e.path.clone())
                    .children(|e| {
                        row().class("files-inactive-entry").fill((
                            icons::glyph(if e.value.get().is_dir {
                                Icon::Folder
                            } else {
                                Icon::File
                            }),
                            text().content(e.value.map(|file| file.name.clone())),
                        ))
                    }),
                ))
            },
        ),
    ))
}
