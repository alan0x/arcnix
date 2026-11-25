//! Open documents belong to tabs; moving a tab changes only its window ownership.
use std::collections::BTreeMap;

use arcnix_core::file_associations::filename;
use arcnix_core::server_apps::DesktopApp;

use crate::*;

pub(crate) trait FileTab {
    fn path(&self) -> &str;
}

#[derive(Clone, Default)]
pub(crate) struct Documents<T> {
    pub groups: window_tabs::Groups,
    pub files: BTreeMap<u64, T>,
    idle: BTreeMap<u32, T>,
    empty: T,
}

impl<T: FileTab + Default> Documents<T> {
    pub fn selected(&self, window: u32) -> &T {
        self.groups
            .selected(window)
            .and_then(|id| self.files.get(&id))
            .or_else(|| self.idle.get(&window))
            .unwrap_or(&self.empty)
    }
    pub fn selected_mut(&mut self, window: u32) -> &mut T {
        if let Some(file) = self
            .groups
            .selected(window)
            .and_then(|id| self.files.get_mut(&id))
        {
            file
        } else {
            self.idle.entry(window).or_default()
        }
    }
    pub fn current(&self) -> &T {
        self.selected(self.groups.current)
    }
    pub fn current_mut(&mut self) -> &mut T {
        self.selected_mut(self.groups.current)
    }
    pub fn tabs(&self, window: u32) -> Vec<(u64, String)> {
        self.groups
            .tabs(window)
            .iter()
            .filter_map(|id| Some((*id, self.files.get(id)?.path().to_owned())))
            .collect()
    }
    pub fn find(&self, path: &str) -> Option<(u32, u64)> {
        let (&id, _) = self.files.iter().find(|(_, f)| f.path() == path)?;
        Some((self.groups.owner(id)?, id))
    }
    pub fn insert(&mut self, id: u64, file: T) {
        self.idle.remove(&self.groups.current);
        self.files.insert(id, file);
        self.groups.add(id);
    }
    pub fn remove(&mut self, id: u64) {
        self.groups.remove(id);
        self.files.remove(&id);
    }
    pub fn remove_window(&mut self, window: u32) {
        for id in self.groups.tabs(window) {
            self.files.remove(id);
        }
        self.idle.remove(&window);
    }
    pub fn all_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.files.values_mut().chain(self.idle.values_mut())
    }
}

pub fn select(ui: &Ui, app: DesktopApp, window: u32, id: u64) {
    ui.focus_desktop_app(window_tabs::page(app, window));
    ui.state.revise(|s| match app {
        DesktopApp::Office => s.office.groups.select(id),
        DesktopApp::PdfReader => s.pdf.groups.select(id),
        _ => {}
    });
}

pub fn close(ui: &Ui, app: DesktopApp, id: u64) {
    match app {
        DesktopApp::Office => {
            if office_reader::can_close_tab(ui, id) {
                ui.state.revise(|s| s.office.remove(id));
            }
        }
        DesktopApp::PdfReader => ui.state.revise(|s| s.pdf.remove(id)),
        _ => {}
    }
}

pub fn strip(ui: Ui, app: DesktopApp, window: u32) -> impl Widget {
    let state = ui.state;
    let pdf = app == DesktopApp::PdfReader;
    row()
        .class(if pdf {
            "pdf-tabs editor-tab-strip window-tab-strip"
        } else {
            "office-tabs editor-tab-strip window-tab-strip"
        })
        .name(i18n::desktop::bind("Open files"))
        .fill(
            each(state.map(move |s| {
                let tabs = if pdf {
                    s.pdf.tabs(window)
                } else {
                    s.office.tabs(window)
                };
                tabs.into_iter()
                    .map(|(id, path)| {
                        let dirty = !pdf && s.office.files.get(&id).is_some_and(|doc| doc.dirty);
                        (id, path, dirty)
                    })
                    .collect::<Vec<_>>()
            }))
            .key(|t| t.0)
            .children(move |item| {
                let id = item.value.get().0;
                let active = state.map(move |s| {
                    (if pdf { &s.pdf.groups } else { &s.office.groups }).selected(window)
                        == Some(id)
                });
                let select_ui = ui.clone();
                let close_ui = ui.clone();
                row()
                    .class(state.map(move |_| {
                        if active.get() {
                            "window-tab-item editor-tab-item active"
                        } else {
                            "window-tab-item editor-tab-item"
                        }
                        .to_owned()
                    }))
                    .fill((
                        text().class("window-tab-id hidden").content(id.to_string()),
                        sapid::button()
                            .class("editor-tab window-tab-select")
                            .pressed(active)
                            .name(item.value.map(|(_, path, dirty)| {
                                if *dirty {
                                    format!(
                                        "{path} · {}",
                                        i18n::desktop::translate("Unsaved changes")
                                    )
                                } else {
                                    path.clone()
                                }
                            }))
                            .fill(text().class("editor-tab-label").content(item.value.map(
                                |(_, path, dirty)| {
                                    let name = filename(path);
                                    if *dirty {
                                        format!("*{name}")
                                    } else {
                                        name.to_owned()
                                    }
                                },
                            )))
                            .on_press(move || select(&select_ui, app, window, id)),
                        office_sync::upload_badge(state.map(move |s| {
                            !pdf && s
                                .office
                                .files
                                .get(&id)
                                .is_some_and(|doc| office_sync::uploading(s, doc.path()))
                        })),
                        sapid::button()
                            .class("icon-button editor-tab-close")
                            .name(i18n::desktop::bind(
                                item.value
                                    .map(|t| format!("Close file tab {}", filename(&t.1))),
                            ))
                            .fill(icons::glyph(Icon::Close))
                            .on_press(move || close(&close_ui, app, id)),
                    ))
            }),
        )
}
