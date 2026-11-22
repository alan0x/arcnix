use crate::i18n::desktop as i18n;
use crate::*;

const PRESETS: [u8; 4] = [24, 48, 64, 96];
const MIN: u8 = PRESETS[0];
const MAX: u8 = PRESETS[PRESETS.len() - 1];

#[derive(Clone, Copy)]
pub(crate) struct FolderView {
    pub icons: bool,
    pub icon_size: u8,
    pub sort: Sort,
    pub descending: bool,
    pub columns: VisibleFileColumns,
}

impl Default for FolderView {
    fn default() -> Self {
        Self {
            icons: false,
            icon_size: 48,
            sort: Sort::Name,
            descending: false,
            columns: VisibleFileColumns::default(),
        }
    }
}

fn scope(s: &State, remote: bool) -> Option<String> {
    if !remote {
        Some(format!("local-{}", std::env::consts::OS))
    } else if let Some(target) = &s.local_desktop {
        Some(target.scope())
    } else {
        s.selected.as_ref().map(|id| format!("server:{id}"))
    }
}

fn folder_path(path: &str, remote: bool) -> std::borrow::Cow<'_, str> {
    #[cfg(windows)]
    if !remote {
        let path = path.replace('\\', "/");
        return format!("/{}", path.trim_end_matches('/')).into();
    }
    let _ = remote;
    path.into()
}

fn path(s: &State, remote: bool) -> &str {
    if remote {
        &s.remote_path
    } else {
        &s.local_path
    }
}

pub(crate) fn saved(s: &State, remote: bool, path: &str) -> FolderOverrides {
    scope(s, remote)
        .and_then(|scope| s.config.settings.folder_views.get(&scope))
        .and_then(|locations| locations.get(folder_path(path, remote).as_ref()))
        .copied()
        .unwrap_or_default()
}

fn inherit(mut view: FolderView, overrides: FolderOverrides) -> FolderView {
    view.columns = overrides.columns.unwrap_or(view.columns);
    view.icons = overrides.icons.unwrap_or(view.icons);
    view.icon_size = overrides
        .icon_size
        .unwrap_or(view.icon_size)
        .clamp(MIN, MAX);
    view.sort = overrides.sort.unwrap_or(view.sort);
    view.descending = overrides.descending.unwrap_or(view.descending);
    view
}

fn default_view(s: &State) -> FolderView {
    inherit(FolderView::default(), s.config.settings.folder_defaults)
}

pub(crate) fn for_path(s: &State, remote: bool, path: &str) -> FolderView {
    // A saved pane-wide column choice predates folder defaults. Keep it as
    // the baseline until the user explicitly chooses new default columns.
    let mut view = default_view(s);
    if s.config.settings.folder_defaults.columns.is_none()
        && let Some(profile) = s
            .config
            .servers
            .iter()
            .find(|p| Some(&p.id) == s.selected.as_ref())
    {
        view.columns = if remote {
            profile.file_columns.remote
        } else {
            profile.file_columns.local
        };
    }
    inherit(view, saved(s, remote, path))
}

pub(crate) fn current(s: &State, remote: bool) -> FolderView {
    for_path(s, remote, path(s, remote))
}

pub(crate) fn change(ui: &Ui, remote: bool, edit: impl FnOnce(&mut FolderOverrides)) {
    let location = ui.state.with(|s| {
        let ready = if remote {
            s.remote_ready
        } else {
            s.local_ready
        };
        if !ready {
            return None;
        }
        Some((
            scope(s, remote)?,
            folder_path(path(s, remote), remote).into_owned(),
        ))
    });
    let Some((scope, path)) = location else {
        return;
    };
    ui.update_preferences(|prefs| {
        let folders = prefs.folder_views.entry(scope.clone()).or_default();
        let value = folders.entry(path.clone()).or_default();
        edit(value);
        if *value == FolderOverrides::default() {
            folders.remove(&path);
        }
        if folders.is_empty() {
            prefs.folder_views.remove(&scope);
        }
    });
}

fn resize(ui: &Ui, pixels: u8) {
    change(ui, true, |v| {
        v.icons = Some(true);
        v.icon_size = Some(pixels);
    });
}

pub(crate) fn step(ui: &Ui, larger: bool) {
    let view = ui.state.with(|s| current(s, true));
    if !view.icons && larger {
        resize(ui, MIN);
    } else if view.icons {
        let next = if larger {
            PRESETS.into_iter().find(|size| *size > view.icon_size)
        } else {
            PRESETS
                .into_iter()
                .rev()
                .find(|size| *size < view.icon_size)
        };
        if let Some(pixels) = next {
            resize(ui, pixels);
        } else if !larger {
            change(ui, true, |v| v.icons = Some(false));
        }
    }
}

pub(crate) fn options(ui: Ui) -> impl Widget {
    let state = ui.state;
    let columns = ui.clone();
    let layout = ui.clone();
    let slider = ui.clone();
    let presets = ui.clone();
    let order = ui.clone();
    let direction = ui.clone();
    let reset = ui;
    fragment((
        col().class("file-menu-group").fill(
            [(false, "详细列表"), (true, "平铺图标")]
                .into_iter()
                .map(move |(grid, label)| {
                    let ui = layout.clone();
                    sapid::button()
                        .class("menu-item file-view-choice")
                        .label(i18n::bind(label))
                        .pressed(state.map(move |s| file_browser::icons(s, true) == grid))
                        .on_press(move || {
                            change(&ui, true, |v| v.icons = Some(grid));
                            state.revise(|s| s.file_menu = None);
                        })
                })
                .collect::<Vec<_>>(),
        ),
        col().class("file-menu-group file-view-sizes").fill((
            text()
                .class("file-view-heading")
                .content(i18n::bind("File icon size")),
            size_slider(&slider, true, false),
            row().class("file-size-presets").fill(
                PRESETS
                    .into_iter()
                    .map(move |size| {
                        let ui = presets.clone();
                        sapid::button()
                            .class("menu-item")
                            .label(format!("{size} px"))
                            .pressed(state.map(move |s| {
                                file_browser::icons(s, true) && current(s, true).icon_size == size
                            }))
                            .on_press(move || resize(&ui, size))
                    })
                    .collect::<Vec<_>>(),
            ),
        )),
        col().class("file-menu-group").fill((
            text()
                .class("file-view-heading")
                .content(i18n::bind("Sort by")),
            fragment(
                [
                    (Sort::Name, "名称"),
                    (Sort::Size, "大小"),
                    (Sort::Modified, "修改时间"),
                ]
                .into_iter()
                .map(move |(sort, label)| {
                    let ui = order.clone();
                    sapid::button()
                        .class("menu-item file-view-choice")
                        .label(i18n::bind(label))
                        .pressed(state.map(move |s| current(s, true).sort == sort))
                        .on_press(move || change(&ui, true, |v| v.sort = Some(sort)))
                })
                .collect::<Vec<_>>(),
            ),
            sapid::button()
                .class("menu-item file-view-choice")
                .label(i18n::bind("Descending"))
                .pressed(state.map(|s| current(s, true).descending))
                .on_press(move || {
                    let next = state.with(|s| !current(s, true).descending);
                    change(&direction, true, |v| v.descending = Some(next));
                }),
        )),
        col().class("file-menu-group").fill((
            sapid::button()
                .class("menu-item")
                .label(i18n::bind("显示列"))
                .on_press(move || {
                    state.revise(|s| s.file_menu = None);
                    file_columns::open(&columns, true);
                }),
            sapid::button()
                .class("menu-item")
                .label(i18n::bind("Use default view"))
                .enabled(
                    state.map(|s| saved(s, true, &s.remote_path) != FolderOverrides::default()),
                )
                .on_press(move || {
                    change(&reset, true, |v| *v = FolderOverrides::default());
                    state.revise(|s| s.file_menu = None);
                }),
        )),
    ))
}

fn size_slider(ui: &Ui, desktop: bool, defaults: bool) -> impl Widget + use<> {
    let ui = ui.clone();
    let state = ui.state;
    let class = if defaults {
        "icon-size-control default-file-size"
    } else {
        "icon-size-control files-size-control"
    };
    row().class(class).fill((
        input()
            .class("icon-size-bridge hidden")
            .name(crate::i18n::bind_in(desktop, "File icon size"))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event
                    && let Ok(size) = value.parse::<u8>()
                    && (MIN..=MAX).contains(&size)
                {
                    if defaults {
                        ui.update_preferences(|p| p.folder_defaults.icon_size = Some(size));
                    } else {
                        resize(&ui, size);
                    }
                }
            }),
        text()
            .class("icon-size-state hidden")
            .content(state.map(move |s| {
                let size = if defaults {
                    default_view(s).icon_size
                } else {
                    current(s, true).icon_size
                };
                format!("{MIN},{MAX},{size}")
            })),
    ))
}

type ColumnChoice = (
    &'static str,
    fn(VisibleFileColumns) -> bool,
    fn(&mut VisibleFileColumns, bool),
);
pub(super) const COLUMN_CHOICES: &[ColumnChoice] = &[
    ("大小", |c| c.size, |c, value| c.size = value),
    ("类型", |c| c.kind, |c, value| c.kind = value),
    ("修改时间", |c| c.modified, |c, value| c.modified = value),
    ("权限", |c| c.permissions, |c, value| c.permissions = value),
];

pub(crate) fn defaults(ui: &Ui, desktop: bool) -> impl Widget + use<> {
    let field = move |label| {
        text()
            .class("field-label")
            .content(crate::i18n::bind_in(desktop, label))
    };
    let columns = COLUMN_CHOICES
        .iter()
        .map(|&(label, get, set)| {
            let action = ui.clone();
            sapid::checkbox()
                .class("preference-check")
                .label(crate::i18n::bind_in(desktop, label))
                .checked(ui.state.map(move |s| get(default_view(s).columns)))
                .on_checked_change(move |checked| {
                    action.update_preferences(|p| {
                        set(
                            p.folder_defaults
                                .columns
                                .get_or_insert_with(Default::default),
                            checked,
                        );
                    })
                })
        })
        .collect::<Vec<_>>();
    col().class("folder-defaults").fill((
        text()
            .class("section-title")
            .content(crate::i18n::bind_in(desktop, "Default folder view")),
        row().class("folder-default-row").fill((
            field("View mode"),
            settings::select(
                ui,
                desktop,
                "View mode",
                [(false, "详细列表"), (true, "平铺图标")],
                |s| default_view(s).icons,
                |_| true,
                |ui, value| ui.update_preferences(|p| p.folder_defaults.icons = Some(value)),
            ),
        )),
        row()
            .class("folder-default-row")
            .fill((field("File icon size"), size_slider(ui, desktop, true))),
        row().class("folder-default-row").fill((
            field("Sort by"),
            settings::select(
                ui,
                desktop,
                "Sort by",
                [
                    (Sort::Name, "名称"),
                    (Sort::Size, "大小"),
                    (Sort::Modified, "修改时间"),
                ],
                |s| default_view(s).sort,
                |_| true,
                |ui, value| ui.update_preferences(|p| p.folder_defaults.sort = Some(value)),
            ),
        )),
        row().class("folder-default-row").fill((
            field("Sort order"),
            settings::select(
                ui,
                desktop,
                "Sort order",
                [(false, "Ascending"), (true, "Descending")],
                |s| default_view(s).descending,
                |_| true,
                |ui, value| ui.update_preferences(|p| p.folder_defaults.descending = Some(value)),
            ),
        )),
        row().class("folder-default-row").fill((
            field("Visible columns"),
            row().class("folder-default-columns").fill((
                sapid::checkbox()
                    .class("preference-check")
                    .label(crate::i18n::bind_in(desktop, "名称"))
                    .checked(true)
                    .enabled(false),
                fragment(columns),
            )),
        )),
    ))
}
