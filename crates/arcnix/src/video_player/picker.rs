use super::*;

#[derive(Clone)]
pub(crate) struct Directory {
    path: String,
    request: Option<u64>,
    entries: Vec<FileEntry>,
    error: String,
}

impl Player {
    pub(crate) fn browse(&self, ui: &Ui) {
        ui.open_dialog(Dialog::OpenVideo);
        if let Some(path) = ui.state.with(|s| {
            (s.dialog == Some(Dialog::OpenVideo)).then(|| {
                s.video
                    .as_ref()
                    .and_then(|video| video.path.rsplit_once('/'))
                    .map_or_else(
                        || s.remote_path.clone(),
                        |(parent, _)| if parent.is_empty() { "/" } else { parent }.to_owned(),
                    )
            })
        }) {
            self.read_directory(ui, path);
        }
    }

    fn read_directory(&self, ui: &Ui, path: String) {
        let serial = ui.send(Action::Children {
            remote: true,
            path: path.clone(),
        });
        ui.state.revise(|s| {
            s.video_picker = Some(Directory {
                path,
                request: Some(serial),
                entries: Vec::new(),
                error: String::new(),
            })
        });
    }

    pub(crate) fn awaiting_directory(&self, ui: &Ui, serial: u64) -> bool {
        ui.state.with(|s| {
            s.dialog == Some(Dialog::OpenVideo)
                && s.video_picker
                    .as_ref()
                    .is_some_and(|d| d.request == Some(serial))
        })
    }

    pub(crate) fn listed(&self, ui: &Ui, result: Result<Vec<FileEntry>, String>) {
        let show_hidden = ui.state.with(|s| s.config.settings.show_hidden_files);
        ui.state.revise(|s| {
            let Some(directory) = &mut s.video_picker else {
                return;
            };
            directory.request = None;
            match result {
                Ok(mut entries) => {
                    entries.retain(|e| {
                        !e.is_symlink
                            && (show_hidden || !e.name.starts_with('.'))
                            && (e.is_dir || supports(&e.path))
                    });
                    directory.entries = entries;
                }
                Err(error) => directory.error = error,
            }
        });
    }
}

pub(super) fn dialog(ui: Ui) -> impl Widget {
    when(
        ui.state.map(|s| s.dialog == Some(Dialog::OpenVideo)),
        move || {
            let cancel = ui.clone();
            modal(
                ui.clone(),
                "Open video",
                panel(ui.clone()),
                btn("取消", move || cancel.close_dialog()).class("button"),
            )
        },
    )
}

fn panel(ui: Ui) -> impl Widget {
    let state = ui.state;
    let up = ui.clone();
    let refresh = ui.clone();
    col().class("player-picker").fill((
        row().class("player-picker-nav").fill((
            icon_action("Parent folder", Icon::Up)
                .enabled(state.map(|s| s.video_picker.as_ref().is_some_and(|d| d.path != "/")))
                .on_press(move || {
                    let parent = state.with(|s| {
                        s.video_picker.as_ref().and_then(|d| {
                            let (parent, _) = d.path.trim_end_matches('/').rsplit_once('/')?;
                            Some(if parent.is_empty() { "/" } else { parent }.to_owned())
                        })
                    });
                    if let Some(path) = parent {
                        up.player.read_directory(&up, path);
                    }
                }),
            text().class("player-picker-path").content(state.map(|s| {
                s.video_picker
                    .as_ref()
                    .map(|d| d.path.clone())
                    .unwrap_or_default()
            })),
            icon_action("Refresh", Icon::Refresh).on_press(move || {
                if let Some(path) = state.with(|s| s.video_picker.as_ref().map(|d| d.path.clone()))
                {
                    refresh.player.read_directory(&refresh, path);
                }
            }),
        )),
        col().class("player-picker-list").fill((
            each(state.map(|s| {
                s.video_picker
                    .as_ref()
                    .map(|d| d.entries.clone())
                    .unwrap_or_default()
            }))
            .key(|entry| entry.path.clone())
            .children(move |item| {
                let entry = item.value;
                let open = ui.clone();
                sapid::button()
                    .class("player-picker-entry")
                    .name(entry.map(|e| e.name.clone()))
                    .fill((
                        icons::glyph(if entry.get().is_dir {
                            Icon::Folder
                        } else {
                            Icon::Play
                        }),
                        text()
                            .class("player-entry-name")
                            .content(entry.map(|e| e.name.clone())),
                        text().class("player-entry-meta").content(entry.map(|e| {
                            if e.is_dir {
                                String::new()
                            } else {
                                size(e.size)
                            }
                        })),
                    ))
                    .on_press(move || {
                        let e = entry.get();
                        if e.is_dir {
                            open.player.read_directory(&open, e.path);
                        } else {
                            open.player.open(&open, &e.path);
                        }
                    })
            }),
            text()
                .class("player-picker-message")
                .content(state.map(|s| {
                    let Some(d) = &s.video_picker else {
                        return String::new();
                    };
                    if d.request.is_some() {
                        i18n::translate("Loading…")
                    } else if !d.error.is_empty() {
                        d.error.clone()
                    } else if d.entries.is_empty() {
                        i18n::translate("No playable videos in this folder.")
                    } else {
                        String::new()
                    }
                })),
        )),
    ))
}
