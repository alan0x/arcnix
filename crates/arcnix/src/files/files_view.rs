use crate::i18n::desktop as i18n;
use crate::*;

const PRESETS: [u8; 4] = [24, 48, 64, 96];
const MIN: u8 = PRESETS[0];
const MAX: u8 = PRESETS[PRESETS.len() - 1];

#[derive(Clone, Copy)]
pub(crate) struct IconSize(u8);

impl IconSize {
    pub(crate) fn pixels(self) -> u8 {
        self.0
    }
}

impl Default for IconSize {
    fn default() -> Self {
        Self(48)
    }
}

fn resize(s: &mut State, pixels: u8) {
    s.file_browser.icon_size = IconSize(pixels);
    if !file_browser::icons(s, true) {
        file_browser::set_icons(s, true, true);
    }
}

pub(crate) fn step(s: &mut State, larger: bool) {
    let grid = file_browser::icons(s, true);
    if !grid && larger {
        resize(s, MIN);
    } else if grid {
        let current = s.file_browser.icon_size.0;
        let next = if larger {
            PRESETS.into_iter().find(|size| *size > current)
        } else {
            PRESETS.into_iter().rev().find(|size| *size < current)
        };
        if let Some(pixels) = next {
            resize(s, pixels);
        } else if !larger {
            file_browser::set_icons(s, true, false);
        }
    }
}

pub(crate) fn options(ui: Ui) -> impl Widget {
    let state = ui.state;
    let columns = ui.clone();
    fragment((
        col().class("file-menu-group").fill(
            [(false, "详细列表"), (true, "平铺图标")]
                .into_iter()
                .map(move |(grid, label)| {
                    sapid::button()
                        .class("menu-item file-view-choice")
                        .label(i18n::bind(label))
                        .pressed(state.map(move |s| file_browser::icons(s, true) == grid))
                        .on_press(move || {
                            state.revise(|s| {
                                file_browser::set_icons(s, true, grid);
                                s.file_menu = None;
                            })
                        })
                })
                .collect::<Vec<_>>(),
        ),
        col().class("file-menu-group file-view-sizes").fill((
            text()
                .class("file-view-heading")
                .content(i18n::bind("File icon size")),
            row().class("icon-size-control files-size-control").fill((
                input()
                    .class("icon-size-bridge hidden")
                    .name(i18n::bind("File icon size"))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event
                            && let Ok(size) = value.parse::<u8>()
                            && (MIN..=MAX).contains(&size)
                        {
                            state.revise(|s| resize(s, size));
                        }
                    }),
                text()
                    .class("icon-size-state hidden")
                    .content(state.map(|s| format!("{MIN},{MAX},{}", s.file_browser.icon_size.0))),
            )),
            row().class("file-size-presets").fill(
                PRESETS
                    .into_iter()
                    .map(move |size| {
                        sapid::button()
                            .class("menu-item")
                            .label(format!("{size} px"))
                            .pressed(state.map(move |s| {
                                file_browser::icons(s, true) && s.file_browser.icon_size.0 == size
                            }))
                            .on_press(move || state.revise(|s| resize(s, size)))
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
                    sapid::button()
                        .class("menu-item file-view-choice")
                        .label(i18n::bind(label))
                        .pressed(state.map(move |s| s.remote_sort == sort))
                        .on_press(move || state.revise(|s| s.remote_sort = sort))
                })
                .collect::<Vec<_>>(),
            ),
            sapid::button()
                .class("menu-item file-view-choice")
                .label(i18n::bind("Descending"))
                .pressed(state.map(|s| s.remote_desc))
                .on_press(move || state.revise(|s| s.remote_desc = !s.remote_desc)),
        )),
        col().class("file-menu-group").fill(
            sapid::button()
                .class("menu-item")
                .label(i18n::bind("显示列"))
                .on_press(move || {
                    state.revise(|s| s.file_menu = None);
                    file_columns::open(&columns, true);
                }),
        ),
    ))
}
