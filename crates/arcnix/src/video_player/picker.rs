use super::*;

#[derive(Clone)]
pub(super) struct Directory {
    path: String,
    request: Option<u64>,
    entries: Vec<FileEntry>,
    error: String,
}

impl Player {
    pub(crate) fn browse(&self, ui: &Ui) {
        self.read_directory(ui, ui.state.with(|s| s.remote_path.clone()));
    }

    fn read_directory(&self, ui: &Ui, path: String) {
        let serial = ui.send(Action::Children {
            remote: true,
            path: path.clone(),
        });
        self.picker.revise(|picker| {
            *picker = Some(Directory {
                path,
                request: Some(serial),
                entries: Vec::new(),
                error: String::new(),
            })
        });
    }

    fn dismiss_picker(&self) {
        self.picker.revise(|picker| *picker = None);
    }

    pub(crate) fn awaiting_directory(&self, serial: u64) -> bool {
        self.picker
            .with(|p| p.as_ref().is_some_and(|d| d.request == Some(serial)))
    }

    pub(crate) fn listed(&self, ui: &Ui, result: Result<Vec<FileEntry>, String>) {
        let show_hidden = ui.state.with(|s| s.config.settings.show_hidden_files);
        self.picker.revise(|picker| {
            let Some(directory) = picker else { return };
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

pub(super) fn panel(ui: Ui) -> impl Widget {
    let picker = ui.player.picker;
    let up = ui.clone();
    let refresh = ui.clone();
    let close = ui.clone();
    col().class("player-picker").fill((
        row().class("player-picker-heading").fill((
            text().content(i18n::bind("Open video")),
            close_action("Cancel").on_press(move || close.player.dismiss_picker()),
        )),
        row().class("player-picker-nav").fill((
            icon_action("Parent folder", Icon::Up)
                .enabled(picker.map(|p| p.as_ref().is_some_and(|d| d.path != "/")))
                .on_press(move || {
                    let parent = picker.with(|p| {
                        p.as_ref().and_then(|d| {
                            let (parent, _) = d.path.trim_end_matches('/').rsplit_once('/')?;
                            Some(if parent.is_empty() { "/" } else { parent }.to_owned())
                        })
                    });
                    if let Some(path) = parent {
                        up.player.read_directory(&up, path);
                    }
                }),
            text()
                .class("player-picker-path")
                .content(picker.map(|p| p.as_ref().map(|d| d.path.clone()).unwrap_or_default())),
            icon_action("Refresh", Icon::Refresh).on_press(move || {
                if let Some(path) = picker.with(|p| p.as_ref().map(|d| d.path.clone())) {
                    refresh.player.read_directory(&refresh, path);
                }
            }),
        )),
        col().class("player-picker-list").fill((
            each(picker.map(|p| p.as_ref().map(|d| d.entries.clone()).unwrap_or_default()))
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
                .content(picker.map(|p| {
                    let Some(d) = p else { return String::new() };
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
        text()
            .class("player-picker-formats")
            .content(EXTENSIONS.join(" · ")),
    ))
}
