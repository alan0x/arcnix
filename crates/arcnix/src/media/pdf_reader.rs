//! A read-only PDF window. A document's renderer lives until its command sender is dropped.
use arcnix_core::file_associations::{FileApp, filename};
use base64::Engine;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf, PdfData};
use hayro::{PixmapSettings, RenderCache};
use zeroize::Zeroizing;

use crate::i18n::desktop as i18n;
use crate::*;

pub const LIMIT: usize = 64 * 1024 * 1024;

pub enum Command {
    Page(usize),
    Password(Zeroizing<String>),
}

pub enum Reply {
    Password(bool),
    Opened(usize),
    Page(std::result::Result<RenderedPage, String>),
    Failed(String),
}

#[derive(Clone)]
pub struct RenderedPage {
    source: Arc<str>,
    width: f32,
}

fn render<'a>(
    page: &'a hayro::hayro_syntax::page::Page<'a>,
    cache: &RenderCache<'a>,
) -> anyhow::Result<RenderedPage> {
    let (width, height) = page.render_dimensions();
    anyhow::ensure!(
        width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0,
        "Invalid PDF page dimensions."
    );
    let scale = 2.5_f32
        .min(4096.0 / width.max(height))
        .min((5_000_000.0 / (width * height)).sqrt());
    anyhow::ensure!(
        width * scale >= 1.0 && height * scale >= 1.0,
        "Invalid PDF page dimensions."
    );
    let pixels = hayro::render(
        page,
        cache,
        &Default::default(),
        &Default::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    let png = pixels.into_png()?;
    Ok(RenderedPage {
        source: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        )
        .into(),
        width: width * (96.0 / 72.0),
    })
}

fn serve(bytes: Vec<u8>, commands: async_channel::Receiver<Command>, publish: &impl Fn(Reply)) {
    let data = PdfData::from(bytes);
    let mut password = Zeroizing::new(String::new());
    let pdf = loop {
        match Pdf::new_with_password(data.clone(), &password) {
            Ok(pdf) => break pdf,
            Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) => {
                publish(Reply::Password(!password.is_empty()));
                let Ok(Command::Password(next)) = commands.recv_blocking() else {
                    return;
                };
                password = next;
            }
            Err(_) => {
                publish(Reply::Failed("Could not read this PDF.".into()));
                return;
            }
        }
    };
    drop(password);
    let pages = pdf.pages();
    if pages.is_empty() {
        publish(Reply::Failed("This PDF has no pages.".into()));
        return;
    }
    publish(Reply::Opened(pages.len()));
    let cache = RenderCache::new();
    while let Ok(Command::Page(number)) = commands.recv_blocking() {
        let result = number
            .checked_sub(1)
            .and_then(|i| pages.get(i))
            .ok_or_else(|| anyhow::anyhow!("Invalid PDF page number."))
            .and_then(|p| render(p, &cache));
        publish(Reply::Page(result.map_err(|e| format!("{e:#}"))));
    }
}

pub fn start(
    bytes: Vec<u8>,
    commands: async_channel::Receiver<Command>,
    events: async_channel::Sender<worker::Response>,
    epoch: u64,
    serial: u64,
) {
    let publish = move |reply| {
        let _ = events.send_blocking(worker::Response {
            epoch,
            serial,
            update: Update::Pdf(reply),
        });
    };
    let errors = publish.clone();
    if std::thread::Builder::new()
        .name("pdf-reader".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                serve(bytes, commands, &publish)
            }))
            .is_err()
            {
                publish(Reply::Failed("Could not render this PDF.".into()));
            }
        })
        .is_err()
    {
        errors(Reply::Failed("Could not start the PDF reader.".into()));
    }
}

#[derive(Clone, Default)]
pub(crate) struct Reader {
    pub fullscreen: bool,
    serial: Option<u64>,
    commands: Option<async_channel::Sender<Command>>,
    page: usize,
    total: usize,
    page_input: String,
    busy: bool,
    locked: bool,
    password: Zeroizing<String>,
    rendered: Option<RenderedPage>,
    error: String,
}

impl Reader {
    pub(super) fn is_open(&self) -> bool {
        self.serial.is_some()
    }

    fn send(&mut self, command: Command) {
        self.busy = self
            .commands
            .as_ref()
            .is_some_and(|tx| tx.try_send(command).is_ok());
        self.error = if self.busy {
            String::new()
        } else {
            "Could not render this PDF.".into()
        };
    }

    fn show(&mut self, number: usize) {
        if (1..=self.total).contains(&number) && !self.busy {
            self.page = number;
            self.rendered = None;
            self.send(Command::Page(number));
        }
        self.page_input = self.page.to_string();
    }

    fn unlock(&mut self) {
        if self.locked && !self.busy {
            let password = std::mem::take(&mut self.password);
            self.send(Command::Password(password));
        }
    }
}

pub(super) fn load(ui: &Ui, path: &str) -> Reader {
    let (tx, rx) = async_channel::bounded(1);
    let serial = ui.send(Action::OpenPdf(path.into(), rx));
    Reader {
        serial: Some(serial),
        commands: Some(tx),
        busy: true,
        ..Default::default()
    }
}

pub(crate) fn received(ui: &Ui, serial: u64, reply: Reply) {
    let path = ui.state.revise(|s| {
        let r = &mut s.office.pdf;
        if r.serial != Some(serial) {
            return None;
        }
        r.busy = false;
        match reply {
            Reply::Password(wrong) => {
                r.locked = true;
                r.error = if wrong {
                    "Incorrect PDF password.".into()
                } else {
                    String::new()
                };
            }
            Reply::Opened(total) => {
                r.total = total;
                r.locked = false;
                r.show(1);
                return Some(s.office.path.clone());
            }
            Reply::Page(Ok(page)) => r.rendered = Some(page),
            Reply::Page(Err(error)) => r.error = error,
            Reply::Failed(error) => {
                r.commands = None;
                r.locked = false;
                r.rendered = None;
                r.total = 0;
                r.page_input.clear();
                r.error = error;
            }
        }
        None
    });
    if let Some(path) = path {
        file_open::remember(ui, &path, FileApp::Office);
    }
}

pub(crate) fn panel(ui: Ui) -> impl Widget {
    let state = ui.state;
    let open = ui.clone();
    let loaded = state.map(|s| s.office.pdf.rendered.is_some());
    col().class("pdf-reader").fill((
        row().class("viewer-toolbar").fill((
            i18n::icon_action("Open Office document", Icon::Folder)
                .class("icon-button pdf-open")
                .enabled(state.map(|s| s.connected && s.sftp_available))
                .on_press(move || office_reader::browse(&open)),
            row().class("viewer-controls").fill((
                reader_control("Previous page", "previous", Icon::ChevronRight)
                    .enabled(state.map(|s| {
                        !s.office.pdf.busy && s.office.pdf.total > 0 && s.office.pdf.page > 1
                    }))
                    .on_press(move || {
                        state.revise(|s| s.office.pdf.show(s.office.pdf.page.saturating_sub(1)))
                    }),
                input()
                    .class("pdf-page-number")
                    .name(i18n::bind("Page number"))
                    .enabled(state.map(|s| s.office.pdf.total > 0 && !s.office.pdf.busy))
                    .value(state.map(|s| s.office.pdf.page_input.clone()))
                    .on(value_changed, move |e| {
                        if let Event::ValueChanged { value, .. } = e {
                            state.revise(|s| s.office.pdf.page_input = value);
                        }
                    })
                    .on(commit, move |e| {
                        if let Event::Commit { value, .. } = e {
                            state.revise(|s| {
                                s.office.pdf.show(value.trim().parse().unwrap_or_default())
                            });
                        }
                    }),
                text().class("pdf-total").content(state.map(|s| {
                    if s.office.pdf.total > 0 {
                        format!("/ {}", s.office.pdf.total)
                    } else {
                        String::new()
                    }
                })),
                reader_control("Next page", "next", Icon::ChevronRight)
                    .enabled(
                        state.map(|s| !s.office.pdf.busy && s.office.pdf.page < s.office.pdf.total),
                    )
                    .on_press(move || state.revise(|s| s.office.pdf.show(s.office.pdf.page + 1))),
            )),
            row().class("viewer-controls").fill((
                reader_control("Zoom out", "out", Icon::Minus).enabled(loaded),
                reader_control("Zoom in", "in", Icon::Plus).enabled(loaded),
                sapid::button()
                    .class("button pdf-fit")
                    .label(i18n::bind("Fit page"))
                    .enabled(loaded),
                sapid::button()
                    .class("button pdf-width")
                    .label(i18n::bind("Fit width"))
                    .enabled(loaded),
            )),
            row()
                .class("viewer-controls")
                .fill(viewer_fullscreen_button(state, WorkspacePage::Office)),
        )),
        col().class("pdf-stage").fill((
            each(state.map(|s| {
                s.office
                    .pdf
                    .rendered
                    .as_ref()
                    .map(|_| (s.office.pdf.serial, s.office.pdf.page))
                    .into_iter()
                    .collect::<Vec<_>>()
            }))
            .key(|key| *key)
            .children(move |_| {
                let (src, width, alt) = state.with(|s| {
                    let r = &s.office.pdf;
                    let p = r.rendered.as_ref().unwrap();
                    (
                        p.source.to_string(),
                        p.width.to_string(),
                        format!("{} · {}", filename(&s.office.path), r.page),
                    )
                });
                col().class("pdf-sheet").fill((
                    text().class("hidden pdf-page-width").content(width),
                    glory::ui::image().source(src).alt(alt),
                ))
            }),
            when(state.map(|s| s.office.pdf.rendered.is_none()), move || {
                col().class("viewer-placeholder").fill((
                    icons::glyph(Icon::File),
                    text().content(i18n::bind(state.map(|s| {
                        if !s.office.pdf.error.is_empty() {
                            return s.office.pdf.error.clone();
                        }
                        if s.office.pdf.locked {
                            "Enter the PDF password."
                        } else if s.office.pdf.busy {
                            "Loading PDF…"
                        } else {
                            "Open a PDF to read it."
                        }
                        .into()
                    }))),
                    when(state.map(|s| s.office.pdf.locked), move || {
                        row().class("pdf-unlock").fill((
                            input()
                                .purpose(Purpose::Password)
                                .name(i18n::bind("PDF password"))
                                .value(state.map(|s| s.office.pdf.password.to_string()))
                                .on(value_changed, move |e| {
                                    if let Event::ValueChanged { value, .. } = e {
                                        state.revise(|s| {
                                            s.office.pdf.password = Zeroizing::new(value)
                                        });
                                    }
                                })
                                .on(commit, move |_| state.revise(|s| s.office.pdf.unlock())),
                            i18n::btn("Unlock PDF", move || {
                                state.revise(|s| s.office.pdf.unlock())
                            })
                            .enabled(state.map(|s| !s.office.pdf.busy)),
                        ))
                    }),
                ))
            }),
        )),
        row().class("viewer-status").fill((
            text()
                .class("hidden pdf-document-id")
                .content(state.map(|s| {
                    s.office
                        .pdf
                        .serial
                        .map(|n| n.to_string())
                        .unwrap_or_default()
                })),
            text().class("pdf-scale"),
        )),
    ))
}
