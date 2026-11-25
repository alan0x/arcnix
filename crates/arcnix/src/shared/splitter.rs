//! Captured pointer resizing supported by the Wry host; all state stays in Rust.
use std::cell::RefCell;

use glory::render::{DragCapture, DragPhase, Navigation};
use glory::style::{Length, Style};

use crate::*;

const HANDLE: f64 = 10.0;
const MIN_PANE: f64 = 260.0;

fn slot_style(ratio: f64) -> Style {
    Style {
        grow: Some(ratio),
        shrink: Some(1.0),
        ..Style::new().width(Length::Px(0.0))
    }
}

fn bounded_ratio(ratio: f64, width: f64) -> f64 {
    let minimum = if width.is_finite() && width > HANDLE {
        (MIN_PANE / (width - HANDLE)).clamp(0.2, 0.5)
    } else {
        0.2
    };
    if ratio.is_finite() {
        ratio.clamp(minimum, 1.0 - minimum)
    } else {
        0.5
    }
}

struct DragOrigin {
    x: f64,
    ratio: f64,
    original_ratio: f64,
    width: f64,
    server: Option<String>,
}

pub(crate) fn panes(
    ui: Ui,
    left: impl Widget + 'static,
    right: impl Widget + 'static,
) -> impl Widget {
    let state = ui.state;
    let width = ui.scope.cage(0.0);
    let origin = Rc::new(RefCell::new(None::<DragOrigin>));
    let resize_origin = origin.clone();
    let dragging = ui.scope.cage(false);
    let drag_ui = ui.clone();
    let resize_ui = ui.clone();
    let reset_ui = ui;
    row()
        .class("panes")
        .on(resize, move |event| {
            if let Event::Resize { size, .. } = event {
                width.revise(|value| *value = size.width);
                // A resized container invalidates the captured geometry. End
                // this gesture instead of allowing a jump on its next move.
                if let Some(initial) = resize_origin.borrow_mut().take()
                    && resize_ui.state.with(|s| s.selected == initial.server)
                {
                    resize_ui
                        .state
                        .revise(|s| s.file_split = initial.original_ratio);
                }
                dragging.revise(|value| *value = false);
            }
        })
        .fill((
            col()
                .class("pane-slot local-slot")
                .style(state.map(move |s| slot_style(bounded_ratio(s.file_split, width.get()))))
                .fill(left),
            view()
                .class(dragging.map(|active| {
                    if *active {
                        "pane-splitter dragging"
                    } else {
                        "pane-splitter"
                    }
                    .to_string()
                }))
                .role(Role::Group)
                .name(i18n::bind("调整文件栏宽度，上下方向键调整，Home 恢复均分"))
                .focus(FocusMode::TabStop)
                .semantics(Some(Box::new(Semantics {
                    drag_capture: Some(DragCapture::Press),
                    ..Default::default()
                })))
                .on(drag, move |event| {
                    let Event::Drag { phase, x, .. } = event else {
                        return;
                    };
                    match phase {
                        DragPhase::Start => {
                            let measured = width.get();
                            if measured <= HANDLE {
                                return;
                            }
                            let (ratio, original_ratio, server) = drag_ui.state.with(|s| {
                                (
                                    bounded_ratio(s.file_split, measured),
                                    s.file_split,
                                    s.selected.clone(),
                                )
                            });
                            *origin.borrow_mut() = Some(DragOrigin {
                                x,
                                ratio,
                                original_ratio,
                                width: measured,
                                server,
                            });
                            dragging.revise(|value| *value = true);
                        }
                        DragPhase::Move | DragPhase::Drop => {
                            let mut start = origin.borrow_mut();
                            let Some(initial) = start.as_ref() else {
                                return;
                            };
                            if drag_ui.state.with(|s| s.selected != initial.server) {
                                start.take();
                                dragging.revise(|value| *value = false);
                                return;
                            }
                            let ratio = bounded_ratio(
                                initial.ratio + (x - initial.x) / (initial.width - HANDLE),
                                initial.width,
                            );
                            drag_ui.state.revise(|s| s.file_split = ratio);
                            if phase == DragPhase::Drop {
                                start.take();
                                dragging.revise(|value| *value = false);
                                drag_ui.save_file_split(ratio);
                            }
                        }
                        DragPhase::Cancel => {
                            if let Some(initial) = origin.borrow_mut().take()
                                && drag_ui.state.with(|s| s.selected == initial.server)
                            {
                                drag_ui
                                    .state
                                    .revise(|s| s.file_split = initial.original_ratio);
                            }
                            dragging.revise(|value| *value = false);
                        }
                    }
                })
                .on(navigate, move |event| {
                    if let Event::Navigate { command, .. } = event {
                        let ratio = reset_ui.state.with(|s| s.file_split);
                        let ratio = match command {
                            Navigation::Previous => ratio - 0.05,
                            Navigation::Next => ratio + 0.05,
                            Navigation::First => 0.5,
                            _ => return,
                        };
                        reset_ui.save_file_split(bounded_ratio(ratio, width.get()));
                    }
                }),
            col()
                .class("pane-slot remote-slot")
                .style(
                    state.map(move |s| slot_style(1.0 - bounded_ratio(s.file_split, width.get()))),
                )
                .fill(right),
        ))
}

#[derive(Clone, Copy)]
pub(crate) enum FileSidebar {
    Files,
    Picker,
}

impl FileSidebar {
    fn width(self, s: &State) -> u16 {
        let prefs = &s.config.settings;
        match self {
            Self::Files => prefs.files_sidebar_width.unwrap_or(160),
            Self::Picker => prefs.file_picker_sidebar_width.unwrap_or(148),
        }
        .clamp(104, 420)
    }

    fn save(self, ui: &Ui, width: u16) {
        if ui.state.with(|s| self.width(s)) != width {
            ui.update_preferences(|prefs| match self {
                Self::Files => prefs.files_sidebar_width = Some(width),
                Self::Picker => prefs.file_picker_sidebar_width = Some(width),
            });
        }
    }
}

pub(crate) fn files_body<W: Widget + 'static>(
    ui: Ui,
    kind: FileSidebar,
    sidebar: impl Fn() -> W + 'static,
    content: impl Widget + 'static,
) -> impl Widget {
    let state = ui.state;
    row().class("files-body").fill((
        when(
            state.map(move |s| match kind {
                FileSidebar::Files => s.sftp_available && !files_ui::preferences(s).sidebar_hidden,
                FileSidebar::Picker => !s.desktop_file_picker_sidebar_hidden,
            }),
            move || {
                let ui = ui.clone();
                fragment((
                    col()
                        .class("files-sidebar-frame")
                        .style(
                            state.map(move |s| {
                                Style::new().width(Length::Px(f64::from(kind.width(s))))
                            }),
                        )
                        .fill(sidebar()),
                    view()
                        .class("files-sidebar-splitter")
                        .role(Role::Group)
                        .focus(FocusMode::TabStop)
                        .name(i18n::desktop::bind("Resize sidebar")),
                    input().class("files-sidebar-width-input hidden").on(
                        value_changed,
                        move |event| {
                            if let Event::ValueChanged { value, .. } = event
                                && let Ok(width) = value.parse::<u16>()
                                && (104..=420).contains(&width)
                            {
                                kind.save(&ui, width);
                            }
                        },
                    ),
                ))
            },
        ),
        content,
    ))
}
