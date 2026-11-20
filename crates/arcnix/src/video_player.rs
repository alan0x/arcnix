//! Seekable SFTP media source shared by the web and native players.
use std::io::{Read, Seek, SeekFrom};
use std::result::Result;
use std::sync::{Mutex, mpsc};

use arcnix_core::remote::RemoteSession;
use glory::wry::shell::{AssetProvider, AssetSource};
use glory::wry::{LocalMedia, MediaKind, Renderer};

use super::*;
mod picker;
pub(super) use picker::Directory;

const WEB_FORMATS: &[(&str, &str)] = &[
    ("mp4", "video/mp4"),
    ("m4v", "video/mp4"),
    ("webm", "video/webm"),
    ("ogv", "video/ogg"),
];
// Pin native demuxers so opening a remote file cannot load a playlist.
const NATIVE_FORMATS: &[(&str, &str)] = &[
    ("mkv", "mkv"),
    ("mov", "mov"),
    ("avi", "avi"),
    ("mpeg", "mpeg"),
    ("mpg", "mpeg"),
    ("ts", "mpegts"),
    ("m2ts", "mpegts"),
    ("flv", "flv"),
    ("wmv", "asf"),
    ("vob", "mpeg"),
    ("3gp", "mov"),
    ("asf", "asf"),
];
const NATIVE_AVAILABLE: bool = cfg!(all(windows, target_arch = "x86_64"));

type ReadResult = Result<Vec<u8>, String>;

#[cfg(windows)]
pub(super) fn native_format(path: &str) -> bool {
    NATIVE_AVAILABLE && web_format(path).is_none()
}

fn web_format(path: &str) -> Option<(&'static str, &'static str)> {
    let extension = arcnix_core::file_associations::extension(path)?;
    WEB_FORMATS
        .iter()
        .copied()
        .find(|(ext, _)| *ext == extension)
}

fn native_demuxer(path: &str) -> Option<&'static str> {
    let extension = arcnix_core::file_associations::extension(path)?;
    NATIVE_FORMATS
        .iter()
        .find_map(|(ext, demuxer)| (*ext == extension).then_some(*demuxer))
}

pub(super) fn supports(path: &str) -> bool {
    web_format(path).is_some() || (NATIVE_AVAILABLE && native_demuxer(path).is_some())
}

pub(super) fn supported_extensions() -> impl Iterator<Item = &'static str> {
    WEB_FORMATS.iter().map(|(ext, _)| *ext).chain(
        NATIVE_FORMATS
            .iter()
            .filter(|_| NATIVE_AVAILABLE)
            .map(|(ext, _)| *ext),
    )
}

struct StreamFormat {
    extension: &'static str,
    mime: &'static str,
    #[cfg(windows)]
    native_demuxer: Option<&'static str>,
}

impl StreamFormat {
    fn web(extension: &'static str, mime: &'static str) -> Self {
        Self {
            extension,
            mime,
            #[cfg(windows)]
            native_demuxer: None,
        }
    }

    fn native(_demuxer: &'static str) -> Self {
        Self {
            extension: "media",
            mime: "application/octet-stream",
            #[cfg(windows)]
            native_demuxer: Some(_demuxer),
        }
    }
}

fn stream_format(
    path: &str,
    file: &mut (impl Read + Seek),
    size: u64,
) -> anyhow::Result<StreamFormat> {
    if let Some((extension, mime)) = web_format(path) {
        return Ok(StreamFormat::web(extension, mime));
    }
    if NATIVE_AVAILABLE && let Some(demuxer) = native_demuxer(path) {
        return Ok(StreamFormat::native(demuxer));
    }
    // An explicitly chosen Player may be opening a video with an unknown suffix.
    let mut header = [0; 12];
    let len = size.min(header.len() as u64) as usize;
    file.read_exact(&mut header[..len])?;
    file.seek(SeekFrom::Start(0))?;
    let (extension, mime, demuxer) = match &header[..len] {
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => ("mp4", "video/mp4", "mov"),
        [0x1a, 0x45, 0xdf, 0xa3, ..] => ("webm", "video/webm", "mkv"),
        [b'O', b'g', b'g', b'S', ..] => ("ogv", "video/ogg", "ogg"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'A', b'V', b'I', b' '] if NATIVE_AVAILABLE => {
            ("avi", "", "avi")
        }
        [b'F', b'L', b'V', ..] if NATIVE_AVAILABLE => ("flv", "", "flv"),
        _ => anyhow::bail!("Player could not identify a supported video format"),
    };
    Ok(if NATIVE_AVAILABLE {
        StreamFormat::native(demuxer)
    } else {
        StreamFormat::web(extension, mime)
    })
}
struct RangeRead {
    offset: u64,
    length: usize,
    reply: mpsc::Sender<ReadResult>,
}
pub(crate) struct Source {
    url: String,
    format: StreamFormat,
    size: u64,
    active: Arc<AtomicBool>,
    reads: mpsc::SyncSender<RangeRead>,
}
impl AssetSource for Source {
    fn len(&self) -> u64 {
        self.size
    }
    fn mime(&self) -> &str {
        self.format.mime
    }
    fn current(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    fn read_range(&self, offset: u64, length: usize) -> ReadResult {
        self.read(offset, length, || false)
    }
}
impl Source {
    #[cfg(windows)]
    pub(super) fn native_demuxer(&self) -> Option<&'static str> {
        self.format.native_demuxer
    }

    pub(super) fn read(
        &self,
        offset: u64,
        length: usize,
        cancelled: impl Fn() -> bool,
    ) -> ReadResult {
        if !self.current()
            || length > 1024 * 1024
            || length as u64 > self.size.saturating_sub(offset)
        {
            return Err("Invalid video range".into());
        }
        let (reply, result) = mpsc::channel();
        self.reads
            .send(RangeRead {
                offset,
                length,
                reply,
            })
            .map_err(|_| "Video closed")?;
        while self.current() && !cancelled() {
            match result.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(reply) => return reply,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
        Err("Video closed".into())
    }
}

pub(super) fn serve(
    path: &str,
    active: Arc<AtomicBool>,
    connect: impl FnOnce() -> anyhow::Result<RemoteSession>,
    ready: impl FnOnce(Result<Arc<Source>, String>),
) {
    // This thread owns both the connection and the handle, including their
    // potentially blocking destructors. UI and directory listings never wait on it.
    let open = || {
        anyhow::ensure!(active.load(Ordering::Acquire), "Video closed");
        let session = connect()?;
        let (file, size) = session.open_media(path)?;
        Ok::<_, anyhow::Error>((session, file, size))
    };
    let (_session, mut file, size) = match open() {
        Ok(value) => value,
        Err(error) => {
            ready(Err(format!("{error:#}")));
            return;
        }
    };
    let format = match stream_format(path, &mut file, size) {
        Ok(format) => format,
        Err(error) => {
            ready(Err(format!("{error:#}")));
            return;
        }
    };
    let (reads, requests) = mpsc::sync_channel(4);
    ready(Ok(Arc::new(Source {
        url: format!("/media/{}.{}", new_id(), format.extension),
        format,
        size,
        active: active.clone(),
        reads,
    })));
    for RangeRead {
        offset,
        length,
        reply,
    } in requests
    {
        let result = (|| {
            anyhow::ensure!(active.load(Ordering::Acquire), "Video closed");
            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0; length];
            for chunk in bytes.chunks_mut(128 * 1024) {
                anyhow::ensure!(active.load(Ordering::Acquire), "Video closed");
                file.read_exact(chunk)?;
            }
            Ok::<_, anyhow::Error>(bytes)
        })();
        let _ = reply.send(result.map_err(|e| e.to_string()));
        if !active.load(Ordering::Acquire) {
            break;
        }
    }
}

type CurrentSource = Arc<Mutex<Option<Arc<Source>>>>;
pub(super) struct Assets {
    pub wallpapers: Arc<desktop_customization_ui::WallpaperAssets>,
    pub player: CurrentSource,
}
impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Option<Arc<dyn AssetSource>> {
        if path.starts_with("/media/")
            && let Some(source) = self
                .player
                .lock()
                .unwrap()
                .as_ref()
                .filter(|s| s.url == path)
        {
            return Some(source.clone());
        }
        self.wallpapers.get(path)
    }
}

#[derive(Clone)]
pub(super) struct Playback {
    pub(super) serial: u64,
    pub(super) title: String,
    path: String,
    active: Arc<AtomicBool>,
    source: Option<Arc<Source>>,
    pub(super) status: String,
    #[cfg(windows)]
    pub(super) native: Option<crate::mpv_native::Playback>,
}
#[derive(Clone)]
pub(super) struct Player {
    #[cfg(windows)]
    pub viewport: Cage<Option<native_viewport::Viewport>>,
    renderer: Renderer,
    node: Cage<Option<glory::render::NodeId>>,
    mounted: Rc<std::cell::RefCell<Option<(glory::render::NodeId, u64)>>>,
    pub source: CurrentSource,
}
impl Player {
    pub fn new(renderer: Renderer, scope: &Scope) -> Self {
        Self {
            #[cfg(windows)]
            viewport: scope.cage(None),
            renderer,
            node: scope.cage(None),
            mounted: Default::default(),
            source: Default::default(),
        }
    }
    pub fn open(&self, ui: &Ui, path: &str) {
        self.stop(ui);
        let active = Arc::new(AtomicBool::new(true));
        let serial = ui.send(Action::PlayVideo {
            path: path.to_owned(),
            active: active.clone(),
        });
        ui.state.revise(|s| {
            s.video = Some(Playback {
                serial,
                title: path.rsplit('/').next().unwrap_or(path).to_owned(),
                path: path.to_owned(),
                active,
                source: None,
                status: i18n::translate("正在打开视频…"),
                #[cfg(windows)]
                native: native_format(path).then(Default::default),
            })
        });
        ui.open_desktop_app(WorkspacePage::Player);
    }
    pub fn close(&self, ui: &Ui) {
        self.stop(ui);
        ui.state
            .revise(|s| server_desktop::remove_window(s, WorkspacePage::Player));
    }
    pub fn stop(&self, ui: &Ui) {
        ui.state.revise(|s| {
            s.video_picker = None;
            if s.dialog == Some(Dialog::OpenVideo) {
                s.dialog = None;
                s.dialog_error.clear();
            }
            if let Some(video) = s.video.take() {
                video.active.store(false, Ordering::Release);
            }
        });
    }

    pub fn ready(&self, ui: &Ui, serial: u64, result: Result<Arc<Source>, String>) {
        ui.state.revise(|s| {
            let Some(video) = s.video.as_mut().filter(|v| v.serial == serial) else {
                return;
            };
            match result {
                Ok(source) => {
                    #[cfg(windows)]
                    if let Some(native) = &mut video.native {
                        native.source = Some(source);
                        return;
                    }
                    video.source = Some(source);
                    video.status.clear();
                }
                Err(error) => video.status = error,
            }
        });
    }

    pub(super) fn opened(&self, ui: &Ui, serial: u64) {
        if let Some(path) = ui.state.with(|s| {
            s.video
                .as_ref()
                .filter(|video| video.serial == serial)
                .map(|video| video.path.clone())
        }) {
            file_open::remember(ui, &path, arcnix_core::file_associations::FileApp::Player);
        }
    }

    pub(super) fn sync_surface(&self, ui: &Ui) {
        let next = self.node.get().and_then(|node| {
            ui.state.with(|s| {
                let video = s.video.as_ref()?;
                Some((
                    node,
                    video.serial,
                    video.source.clone()?,
                    video.title.clone(),
                ))
            })
        });
        let key = next.as_ref().map(|(node, serial, ..)| (*node, *serial));
        let mut mounted = self.mounted.borrow_mut();
        if *mounted == key {
            return;
        }
        if let Some((node, _)) = *mounted {
            let _ = self.renderer.set_local_media(node, None);
        }
        *mounted = key;
        *self.source.lock().unwrap() = next.as_ref().map(|(_, _, source, _)| source.clone());
        if let Some((node, serial, source, title)) = next
            && let Err(error) = self.renderer.set_local_media(
                node,
                Some(LocalMedia {
                    kind: MediaKind::Video,
                    path: &source.url,
                    title: &title,
                }),
            )
        {
            ui.state.revise(|s| {
                if let Some(video) = s.video.as_mut().filter(|v| v.serial == serial) {
                    video.status = error.to_string();
                }
            });
        }
    }
}
fn empty_player(ui: Ui) -> impl Widget {
    let browse = ui.clone();
    col().class("player-idle").fill((
        col().class("player-idle-stage").fill((
            sapid::button()
                .class("player-open-disc")
                .name(i18n::bind("Open video"))
                .fill(icons::glyph(Icon::Play))
                .on_press(move || browse.player.browse(&browse)),
            text()
                .class("player-idle-title")
                .content(i18n::bind("Open video")),
        )),
        col().class("player-idle-controls").fill((
            view().class("player-empty-timeline"),
            row().class("player-idle-transport").fill((
                icons::glyph(Icon::Play),
                text().class("player-clock").content("0:00 / 0:00"),
                text_action("Browse files…", Icon::OpenVideo)
                    .class("button with-icon player-browse")
                    .on_press(move || ui.player.browse(&ui)),
            )),
        )),
    ))
}

pub(super) fn panel(ui: Ui) -> impl Widget {
    let state = ui.state;
    let idle = ui.clone();
    col().class("remote-player").fill((
        #[cfg(windows)]
        crate::mpv_native::bridge(ui.clone()),
        col()
            .class("remote-player-screen")
            .node_ref(ui.player.node)
            .fill(
                row()
                    .class("player-feedback")
                    .fill((icons::glyph(Icon::Play), icons::glyph(Icon::Pause))),
            ),
        input()
            .class("player-opened-input hidden")
            .on(value_changed, {
                let ui = ui.clone();
                move |event| {
                    if let Event::ValueChanged { value, .. } = event
                        && let Some(serial) = ui.state.with(|s| {
                            let video = s.video.as_ref()?;
                            (video.source.as_ref()?.url == value).then_some(video.serial)
                        })
                    {
                        ui.player.opened(&ui, serial);
                    }
                }
            }),
        when(state.map(|s| s.video.is_none()), move || {
            empty_player(idle.clone())
        }),
        #[cfg(windows)]
        crate::mpv_native::controls(ui.clone()),
        text().class("remote-player-status").content(state.map(|s| {
            s.video
                .as_ref()
                .map(|v| v.status.clone())
                .unwrap_or_default()
        })),
        text().class("remote-player-error").content(i18n::bind(
            "Could not play this video. Check the connection and video codec.",
        )),
    ))
}

pub(super) fn picker_dialog(ui: Ui) -> impl Widget {
    picker::dialog(ui)
}
