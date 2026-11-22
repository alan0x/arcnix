//! Window ownership is separate from the application payload (PTY, WebView or document).
use std::collections::BTreeMap;

use arcnix_core::server_apps::DesktopApp;

use crate::*;

#[derive(Clone, Default)]
struct Group {
    tabs: Vec<u64>,
    selected: Option<u64>,
}

#[derive(Clone, Default)]
pub(crate) struct Groups {
    windows: BTreeMap<u32, Group>,
    pub current: u32,
    focus_order: Vec<u32>,
    next: u32,
}

impl Groups {
    pub fn new_window(&mut self) -> u32 {
        self.next += 1;
        self.next
    }
    pub fn window_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.windows.keys().copied()
    }
    pub fn first_window(&self) -> Option<u32> {
        self.window_ids().next()
    }
    pub fn tabs(&self, window: u32) -> &[u64] {
        self.windows.get(&window).map_or(&[], |group| &group.tabs)
    }

    pub fn selected(&self, window: u32) -> Option<u64> {
        self.windows.get(&window)?.selected
    }

    pub fn add(&mut self, tab: u64) {
        let group = self.windows.entry(self.current).or_default();
        group.tabs.push(tab);
        group.selected = Some(tab);
    }

    pub fn select(&mut self, tab: u64) {
        if let Some((&window, group)) = self.windows.iter_mut().find(|(_, g)| g.tabs.contains(&tab))
        {
            self.current = window;
            group.selected = Some(tab);
        }
    }

    pub fn remove(&mut self, tab: u64) {
        if let Some(group) = self.windows.values_mut().find(|g| g.tabs.contains(&tab)) {
            let index = group.tabs.iter().position(|&id| id == tab).unwrap();
            group.tabs.remove(index);
            if group.selected == Some(tab) {
                group.selected = group.tabs.get(index).or_else(|| group.tabs.last()).copied();
            }
        }
    }

    fn transfer(&mut self, tab: u64, target: Option<u32>, before: Option<u64>) -> u32 {
        let target = target.unwrap_or_else(|| self.new_window());
        self.remove(tab);
        let group = self.windows.entry(target).or_default();
        let index = group
            .tabs
            .iter()
            .position(|id| Some(*id) == before)
            .unwrap_or(group.tabs.len());
        group.tabs.insert(index, tab);
        group.selected = Some(tab);
        target
    }
}

pub fn page(app: DesktopApp, window: u32) -> WorkspacePage {
    match app {
        DesktopApp::Files => WorkspacePage::Files(window),
        _ if window == 0 => server_desktop::app_page(app),
        _ => WorkspacePage::Detached(app, window),
    }
}

impl WorkspacePage {
    pub fn base(self) -> Self {
        match self {
            Self::Detached(app, _) => server_desktop::app_page(app),
            _ => self,
        }
    }
    pub fn tab_window(self) -> Option<u32> {
        match self {
            Self::Files(id) | Self::Detached(_, id) => Some(id),
            Self::Terminal | Self::Browser | Self::Editor => Some(0),
            _ => None,
        }
    }
}

fn groups_mut(s: &mut State, app: DesktopApp) -> Option<&mut Groups> {
    match app {
        DesktopApp::Files => Some(&mut s.workbench.files.groups),
        DesktopApp::Terminal => Some(&mut s.terminal.groups),
        DesktopApp::Editor => Some(&mut s.desktop_editor.groups),
        #[cfg(windows)]
        DesktopApp::Browser => Some(&mut s.web_browser.groups),
        _ => None,
    }
}

pub fn current(s: &State, base: WorkspacePage) -> WorkspacePage {
    match base {
        WorkspacePage::Terminal => page(DesktopApp::Terminal, s.terminal.groups.current),
        WorkspacePage::Editor => page(DesktopApp::Editor, s.desktop_editor.groups.current),
        #[cfg(windows)]
        WorkspacePage::Browser => page(DesktopApp::Browser, s.web_browser.groups.current),
        _ => base,
    }
}

pub fn focus(s: &mut State, window: WorkspacePage) {
    let Some(app) = server_desktop::app_id(window) else {
        return;
    };
    let Some(groups) = groups_mut(s, app) else {
        return;
    };
    let id = window.tab_window().unwrap_or_default();
    groups.current = id;
    if groups.focus_order.last() != Some(&id) {
        groups.focus_order.retain(|&window| window != id);
        groups.focus_order.push(id);
    }
    let selected = groups.selected(id);
    match app {
        DesktopApp::Terminal => {
            if let Some(tab) = selected {
                s.terminal.select(tab);
            }
        }
        DesktopApp::Editor => {
            s.desktop_editor.focus_window(selected);
        }
        #[cfg(windows)]
        DesktopApp::Browser => {
            if let Some(tab) = selected
                && s.web_browser.active != Some(tab)
            {
                s.web_browser.select(tab);
            }
        }
        _ => {}
    }
}

pub fn forget(s: &mut State, window: WorkspacePage) {
    #[cfg(windows)]
    if window.base() == WorkspacePage::Browser {
        s.web_browser
            .viewports
            .remove(&window.tab_window().unwrap());
    }
    if let Some(app) = server_desktop::app_id(window)
        && let Some(groups) = groups_mut(s, app)
    {
        let id = window.tab_window().unwrap_or_default();
        groups.windows.remove(&id);
        groups.focus_order.retain(|&window| window != id);
        if groups.current == id {
            groups.current = groups
                .focus_order
                .last()
                .copied()
                .or_else(|| groups.first_window())
                .unwrap_or_default();
        }
    }
}

#[derive(serde::Deserialize)]
struct Move {
    epoch: u64,
    app: DesktopApp,
    source: u32,
    tab: u64,
    target: Option<u32>,
    before: Option<u64>,
}

pub fn movable(s: &State, page: WorkspacePage) -> bool {
    page.tab_window().is_some()
        && server_desktop::visible(s)
        && s.dialog.is_none()
        && s.fingerprint.is_none()
        && match page.base() {
            WorkspacePage::Files(_) => files_tabs::available(s),
            WorkspacePage::Editor => !s.desktop_editor.confirming_close(),
            _ => true,
        }
}

fn move_tab(ui: &Ui, request: Move) {
    let source = page(request.app, request.source);
    let target = request.target.map(|id| page(request.app, id));
    if !ui.state.with(|s| {
        movable(s, source)
            && s.epoch == request.epoch
            && s.workbench.window_visible(source)
            && target.is_none_or(|page| s.workbench.window_visible(page))
    }) {
        return;
    }
    let moved = ui.state.revise(|s| {
        let groups = groups_mut(s, request.app)?;
        if !groups.tabs(request.source).contains(&request.tab)
            || request.before == Some(request.tab)
            || request
                .before
                .is_some_and(|id| !request.target.is_some_and(|w| groups.tabs(w).contains(&id)))
        {
            return None;
        }
        if request.app == DesktopApp::Files {
            files_tabs::prepare_move(s);
        }
        let groups = groups_mut(s, request.app)?;
        let dest = groups.transfer(request.tab, request.target, request.before);
        let destination = page(request.app, dest);
        if dest != request.source && groups.tabs(request.source).is_empty() {
            server_desktop::remove_window(s, source);
        }
        server_desktop::select_app(s, destination);
        Some(destination)
    });
    if moved.is_some() {
        ui.desktop_enter();
        if request.app == DesktopApp::Files {
            files_tabs::after_change(ui);
        }
    }
}

pub fn bridge(ui: Ui) -> impl Widget {
    input()
        .class("window-tab-move hidden")
        .on(value_changed, move |event| {
            if let Event::ValueChanged { value, .. } = event
                && value.len() <= 512
                && let Ok(request) = serde_json::from_str(&value)
            {
                move_tab(&ui, request);
            }
        })
}
