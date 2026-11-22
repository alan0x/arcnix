//! Read-only image navigation for the active desktop session.
use arcnix_core::desktop_customization::WallpaperImage;
pub(crate) use arcnix_core::desktop_customization::image_extensions as extensions;
use arcnix_core::file_associations::{FileApp, extension, filename};
use base64::Engine;
use glory::render::{EventKind, ImageStatus};

use crate::i18n::desktop as i18n;
use crate::server_apps_worker::{Action as AppAction, Reply};
use crate::*;

#[derive(Clone)]
pub(crate) struct Picture {
    source: Arc<str>,
    bytes: u64,
}

impl From<WallpaperImage> for Picture {
    fn from(image: WallpaperImage) -> Self {
        Self {
            source: format!(
                "data:{};base64,{}",
                image.mime,
                base64::engine::general_purpose::STANDARD.encode(&image.bytes)
            )
            .into(),
            bytes: image.bytes.len() as u64,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Viewer {
    pub fullscreen: bool,
    pub request: Option<u64>,
    pub directory_request: Option<u64>,
    path: String,
    siblings: Vec<String>,
    index: usize,
    picture: Option<(u64, Picture)>,
    error: String,
}

pub(crate) fn supports(path: &str) -> bool {
    extension(path).is_some_and(|ext| extensions().any(|known| known == ext))
}

pub(crate) fn browse(ui: &Ui) {
    let start = ui
        .state
        .with(|s| desktop_file_picker::parent(&s.images.path));
    desktop_file_picker::open(ui, desktop_file_picker::Purpose::Image, start);
}

pub(crate) fn open(ui: &Ui, path: &str) {
    if !ui.state.with(code_workbench::available) {
        return;
    }
    ui.open_desktop_app(WorkspacePage::ImageViewer);
    ui.state.revise(|s| {
        s.images = Viewer {
            siblings: vec![path.to_owned()],
            ..Default::default()
        }
    });
    read(ui, path.to_owned());
    if let Some(parent) = desktop_file_picker::parent(path) {
        let request = ui.send(Action::Children {
            remote: true,
            path: parent,
        });
        ui.state
            .revise(|s| s.images.directory_request = Some(request));
    }
}

fn read(ui: &Ui, path: String) {
    let request = ui.send(Action::ServerApps(AppAction::ReadImage(path.clone())));
    ui.state.revise(|s| {
        s.images.request = Some(request);
        s.images.path = path;
        s.images.picture = None;
        s.images.error.clear();
    });
}

pub(crate) fn listed(s: &mut State, entries: Vec<FileEntry>) {
    let viewer = &mut s.images;
    viewer.directory_request = None;
    let mut paths: Vec<_> = entries
        .into_iter()
        .filter(|e| {
            !e.is_dir
                && !e.is_symlink
                && supports(&e.path)
                && (s.config.settings.show_hidden_files || !e.is_hidden())
        })
        .map(|e| e.path)
        .collect();
    if !paths.contains(&viewer.path) {
        paths.push(viewer.path.clone());
    }
    paths.sort();
    paths.sort_by_cached_key(|p| filename(p).to_lowercase());
    viewer.index = paths
        .iter()
        .position(|p| *p == viewer.path)
        .unwrap_or_default();
    viewer.siblings = paths;
}

pub(crate) fn received(ui: &Ui, serial: u64, result: std::result::Result<Reply, String>) {
    let path = ui.state.revise(|s| {
        let v = &mut s.images;
        v.request = None;
        match result {
            Ok(Reply::Image(image)) => {
                v.picture = Some((serial, image));
                Some(v.path.clone())
            }
            Err(message) => {
                v.error = message;
                None
            }
            _ => {
                v.error = "Could not load this image.".into();
                None
            }
        }
    });
    if let Some(path) = path {
        file_open::remember(ui, &path, FileApp::ImageViewer);
    }
}

fn step(ui: &Ui, direction: isize) {
    if let Some(path) = ui.state.revise(|s| {
        let v = &mut s.images;
        if v.request.is_some() {
            return None;
        }
        let index = v.index.checked_add_signed(direction)?;
        let path = v.siblings.get(index)?.clone();
        v.index = index;
        Some(path)
    }) {
        read(ui, path);
    }
}

pub(crate) fn panel(ui: Ui) -> impl Widget {
    let state = ui.state;
    let open = ui.clone();
    let prev = ui.clone();
    let next = ui.clone();
    let loaded = state.map(|s| s.images.picture.is_some());
    col().class("image-viewer").fill((
        row().class("viewer-toolbar").fill((
            i18n::text_action("Open image", Icon::Folder)
                .class("button with-icon viewer-open")
                .enabled(state.map(|s| s.connected && s.sftp_available))
                .on_press(move || browse(&open)),
            row().class("viewer-controls").fill((
                reader_control("Previous image", "previous", Icon::ChevronRight)
                    .enabled(state.map(|s| s.images.request.is_none() && s.images.index > 0))
                    .on_press(move || step(&prev, -1)),
                reader_control("Next image", "next", Icon::ChevronRight)
                    .enabled(state.map(|s| {
                        s.images.request.is_none() && s.images.index + 1 < s.images.siblings.len()
                    }))
                    .on_press(move || step(&next, 1)),
            )),
            row().class("viewer-controls").fill((
                reader_control("Zoom out", "out", Icon::Minus).enabled(loaded),
                reader_control("Zoom in", "in", Icon::Plus).enabled(loaded),
                sapid::button()
                    .class("button viewer-fit")
                    .label(i18n::bind("Fit to window"))
                    .enabled(loaded),
                sapid::button()
                    .class("icon-button viewer-actual")
                    .name(i18n::bind("Actual size"))
                    .label("1:1")
                    .enabled(loaded),
            )),
            row().class("viewer-controls").fill((
                reader_control("Rotate left", "left", Icon::Refresh).enabled(loaded),
                reader_control("Rotate right", "right", Icon::Refresh).enabled(loaded),
                viewer_fullscreen_button(state, WorkspacePage::ImageViewer),
            )),
        )),
        col().class("viewer-stage").fill((
            each(state.map(|s| {
                s.images
                    .picture
                    .as_ref()
                    .map(|(id, _)| *id)
                    .into_iter()
                    .collect::<Vec<_>>()
            }))
            .key(|id| *id)
            .children(move |item| {
                let id = item.value.get();
                let (source, name) = state.with(|s| {
                    (
                        s.images
                            .picture
                            .as_ref()
                            .map(|(_, p)| p.source.to_string())
                            .unwrap_or_default(),
                        filename(&s.images.path).to_owned(),
                    )
                });
                glory::ui::image()
                    .class("viewer-picture")
                    .source(source)
                    .alt(name)
                    .on(EventKind::ImageStatus, move |event| {
                        if matches!(
                            event,
                            Event::ImageStatus {
                                status: ImageStatus::Failed,
                                ..
                            }
                        ) {
                            state.revise(|s| {
                                if s.images
                                    .picture
                                    .as_ref()
                                    .is_some_and(|(serial, _)| *serial == id)
                                {
                                    s.images.picture = None;
                                    s.images.error = "Could not decode this image.".into();
                                }
                            });
                        }
                    })
            }),
            when(state.map(|s| s.images.picture.is_none()), move || {
                col().class("viewer-placeholder").fill((
                    icons::glyph(Icon::Image),
                    text().content(i18n::bind(state.map(|s| {
                        if !s.images.error.is_empty() {
                            s.images.error.clone()
                        } else if s.images.request.is_some() {
                            "Loading image…".into()
                        } else {
                            "Open an image to view it.".into()
                        }
                    }))),
                ))
            }),
        )),
        row().class("viewer-status").fill((
            text()
                .class("viewer-filename")
                .content(state.map(|s| filename(&s.images.path).to_owned())),
            text().class("viewer-dimensions"),
            text().class("viewer-scale"),
            text().content(state.map(|s| {
                s.images
                    .picture
                    .as_ref()
                    .map(|(_, p)| size(p.bytes))
                    .unwrap_or_default()
            })),
            text().content(state.map(|s| {
                if s.images.siblings.is_empty() {
                    String::new()
                } else {
                    format!("{} / {}", s.images.index + 1, s.images.siblings.len())
                }
            })),
        )),
    ))
}
