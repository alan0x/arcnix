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

const EXTENSIONS: &[&str] = if cfg!(all(windows, target_arch = "x86_64")) {
    &["MP4", "MKV"]
} else {
    &["MP4"]
};

type ReadResult = Result<Vec<u8>, String>;

#[cfg(windows)]
pub(super) fn native_format(path: &str) -> bool {
    cfg!(target_arch = "x86_64")
        && Path::new(path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mkv"))
}
pub(super) fn supports(path: &str) -> bool {
    Path::new(path).extension().is_some_and(|ext| {
        EXTENSIONS
            .iter()
            .any(|format| ext.eq_ignore_ascii_case(format))
    })
}
struct RangeRead {
    offset: u64,
    length: usize,
    reply: mpsc::Sender<ReadResult>,
}
pub(crate) struct Source {
    url: String,
    size: u64,
    active: Arc<AtomicBool>,
    reads: mpsc::SyncSender<RangeRead>,
}
impl AssetSource for Source {
    fn len(&self) -> u64 {
        self.size
    }
    fn mime(&self) -> &str {
        "video/mp4"
    }
    fn current(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    fn read_range(&self, offset: u64, length: usize) -> ReadResult {
        self.read(offset, length, || false)
    }
}
impl Source {
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
    let (reads, requests) = mpsc::sync_channel(4);
    ready(Ok(Arc::new(Source {
        url: format!("/media/{}.mp4", new_id()),
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
                active,
                source: None,
                status: i18n::translate("正在打开视频…"),
                #[cfg(windows)]
                native: native_format(path).then(Default::default),
            })
        });
        ui.open_desktop_app(WorkspacePage::Video);
    }
    pub fn close(&self, ui: &Ui) {
        self.stop(ui);
        ui.state
            .revise(|s| server_desktop::remove_window(s, WorkspacePage::Video));
    }
    pub fn stop(&self, ui: &Ui) {
        ui.state.revise(|s| {
            s.video_picker = None;
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
            text()
                .class("player-formats")
                .content(EXTENSIONS.join(" · ")),
        )),
        col().class("player-idle-controls").fill((
            view().class("player-empty-timeline"),
            row().class("player-idle-transport").fill((
                icons::glyph(Icon::Play),
                text().class("player-clock").content("0:00 / 0:00"),
                text_action("Browse files…", Icon::Folder)
                    .class("button with-icon player-browse")
                    .on_press(move || ui.player.browse(&ui)),
            )),
        )),
    ))
}

pub(super) fn panel(ui: Ui) -> impl Widget {
    let state = ui.state;
    let idle = ui.clone();
    let listing = ui.clone();
    col().class("remote-player").fill((
        #[cfg(windows)]
        crate::mpv_native::bridge(ui.clone()),
        view()
            .class("remote-player-screen")
            .node_ref(ui.player.node),
        when(state.map(|s| s.video.is_none()), move || {
            empty_player(idle.clone())
        }),
        when(state.map(|s| s.video_picker.is_some()), move || {
            picker::panel(listing.clone())
        }),
        #[cfg(windows)]
        crate::mpv_native::controls(ui.clone()),
        text().class("remote-player-status").content(state.map(|s| {
            s.video
                .as_ref()
                .map(|v| v.status.clone())
                .unwrap_or_default()
        })),
        text()
            .class("remote-player-error")
            .content(i18n::bind("无法播放此视频，请检查连接和 MP4 编码。")),
    ))
}
