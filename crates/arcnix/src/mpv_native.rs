//! Windows video surface inside the server desktop. Rendering and SFTP run
//! independently of the window thread; the shared desktop manager owns layout.
mod engine;
mod runtime;

use std::sync::mpsc;

use anyhow::{Context, Result};
use glory::wry::shell::{NativeWindow, NativeWindowExtWindows};
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use super::*;
use crate::native_viewport::Viewport;

#[derive(Clone, Default)]
pub(super) struct Playback {
    pub source: Option<Arc<video_player::Source>>,
    commands: Option<mpsc::Sender<Command>>,
    transport: Transport,
}
#[derive(Clone, Copy, Default, PartialEq, Serialize)]
struct Transport {
    loaded: bool,
    position: f64,
    duration: f64,
    paused: bool,
    volume: f64,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Command {
    Toggle,
    Seek(f64),
    Volume(f64),
}
enum Event {
    State(Transport),
    Focus,
    Maximize,
    Failed(String),
}

fn native(s: &State) -> Option<&Playback> {
    s.video.as_ref()?.native.as_ref()
}

pub(super) fn bridge(ui: Ui) -> impl Widget {
    let state = ui.state;
    let port = ui.player.viewport;
    when(state.map(|s| native(s).is_some()), move || {
        fragment((
            text()
                .class("browser-epoch hidden")
                .content(state.map(|s| s.epoch.to_string())),
            input()
                .class("browser-viewport-input hidden")
                .on(value_changed, move |event| {
                    if let glory::prelude::Event::ValueChanged { value, .. } = event
                        && value.len() < 8192
                        && let Ok(v) = serde_json::from_str::<Viewport>(&value)
                        && v.area.is_none_or(native_viewport::BoxPx::valid)
                        && v.clip.len() <= 32
                        && v.clip.iter().all(|b| b.valid())
                        && state.with(|s| s.epoch == v.epoch)
                    {
                        port.revise(|current| *current = Some(v));
                    }
                }),
        ))
    })
}

pub(super) fn controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(state.map(|s| native(s).is_some()), move || {
        row().class("mpv-controls").fill((
            input()
                .class("mpv-command hidden")
                .on(value_changed, move |event| {
                    if let glory::prelude::Event::ValueChanged { value, .. } = event
                        && value.len() < 256
                        && let Ok((serial, cmd)) = serde_json::from_str::<(u64, Command)>(&value)
                    {
                        state.with(|s| {
                            let video = s.video.as_ref().filter(|v| v.serial == serial)?;
                            let player = video.native.as_ref()?;
                            let valid = match cmd {
                                Command::Seek(t) => (0.0..=player.transport.duration).contains(&t),
                                Command::Volume(v) => (0.0..=100.0).contains(&v),
                                Command::Toggle => true,
                            };
                            if valid {
                                let _ = player.commands.as_ref()?.send(cmd);
                            }
                            Some(())
                        });
                    }
                }),
            text().class("mpv-state hidden").content(state.map(|s| {
                match s.video.as_ref().and_then(|v| Some((v.serial, v.native.as_ref()?))) {
                Some((serial, player)) => serde_json::json!({
                    "serial":serial, "media":player.transport,
                    "play":i18n::translate("Play"), "pause":i18n::translate("Pause playback"),
                    "seek":i18n::translate("Playback position"), "volume":i18n::translate("Volume")
                }).to_string(),
                None => String::new(),
            }
            })),
            view().class("mpv-control-bar"),
        ))
    })
}

struct Surface {
    hwnd: HWND,
    serial: u64,
    layout: Option<Viewport>,
    worker: std::thread::JoinHandle<()>,
}
impl Surface {
    fn position(&mut self, port: Option<Viewport>) -> Result<()> {
        if self.layout == port {
            return Ok(());
        }
        unsafe {
            if let Some(v) = &port
                && let Some(area) = v.area
            {
                SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOP),
                    area.left,
                    area.top,
                    area.width(),
                    area.height(),
                    SWP_NOACTIVATE,
                )?;
                let visible = web_browser_native::install_region(self.hwnd, area, &v.clip)?;
                let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
            } else {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
        self.layout = port;
        Ok(())
    }
}
pub(super) struct Host {
    ui: Ui,
    surface: Option<Surface>,
    retired: Vec<Surface>,
}
impl Host {
    pub fn new(ui: Ui) -> Self {
        Self {
            ui,
            surface: None,
            retired: Vec::new(),
        }
    }
    pub fn poll(&mut self, window: &NativeWindow) {
        let wanted = self.ui.state.with(|s| {
            s.video
                .as_ref()
                .filter(|v| v.native.is_some() && s.connected)
                .map(|v| v.serial)
        });
        if self
            .surface
            .as_ref()
            .is_some_and(|view| Some(view.serial) != wanted)
            && let Some(retired) = self.surface.take()
        {
            unsafe {
                let _ = ShowWindow(retired.hwnd, SW_HIDE);
            }
            self.retired.push(retired);
        }
        // HWNDs belong to the UI thread. Wait for mpv to detach without
        // joining a decoder or blocking an interaction on SSH teardown.
        self.retired.retain(|surface| {
            if !surface.worker.is_finished() {
                return true;
            }
            unsafe {
                let _ = DestroyWindow(surface.hwnd);
            }
            false
        });
        let Some(serial) = wanted else { return };
        if self.surface.is_none()
            && self
                .ui
                .state
                .with(|s| native(s).is_some_and(|p| p.source.is_some()))
        {
            let source = self
                .ui
                .state
                .revise(|s| s.video.as_mut()?.native.as_mut()?.source.take());
            if let Some(source) = source {
                match self.start(window, serial, source) {
                    Ok(surface) => self.surface = Some(surface),
                    Err(error) => receive(&self.ui, serial, Event::Failed(format!("{error:#}"))),
                }
            }
        }
        if let Some(surface) = &mut self.surface {
            let visible = self.ui.state.with(|s| {
                server_desktop::visible(s)
                    && s.workbench.window_visible(WorkspacePage::Video)
                    && s.dialog.is_none()
                    && native(s).is_some_and(|p| p.commands.is_some())
            });
            let layout = visible.then(|| self.ui.player.viewport.get()).flatten();
            if let Err(error) = surface.position(layout) {
                unsafe {
                    let _ = ShowWindow(surface.hwnd, SW_HIDE);
                }
                receive(&self.ui, serial, Event::Failed(error.to_string()));
            }
        }
    }
    fn start(
        &self,
        window: &NativeWindow,
        serial: u64,
        source: Arc<video_player::Source>,
    ) -> Result<Surface> {
        let (tx, rx) = mpsc::channel();
        let (events, reports) = async_channel::unbounded();
        let ui = self.ui.clone();
        self.ui
            .scope
            .spawn(async move {
                while let Ok(event) = reports.recv().await {
                    receive(&ui, serial, event);
                }
            })
            .context("Cannot attach playback events")?;
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Arcnix video surface"),
                WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                Some(HWND(window.hwnd() as _)),
                None,
                None,
                None,
            )?
        };
        let raw = hwnd.0 as usize;
        let worker = std::thread::Builder::new()
            .name("arcnix-libmpv".into())
            .spawn(move || {
                let report = |event| {
                    let _ = events.try_send(event);
                };
                if let Err(error) = engine::run(raw, source, rx, report) {
                    report(Event::Failed(format!("{error:#}")));
                }
            });
        let worker = worker.inspect_err(|_| unsafe {
            let _ = DestroyWindow(hwnd);
        })?;
        self.ui.state.revise(|s| {
            if let Some(p) = s.video.as_mut().and_then(|v| v.native.as_mut()) {
                p.commands = Some(tx);
            }
        });
        Ok(Surface {
            hwnd,
            serial,
            layout: None,
            worker,
        })
    }
}
fn receive(ui: &Ui, serial: u64, event: Event) {
    if !ui
        .state
        .with(|s| s.video.as_ref().is_some_and(|v| v.serial == serial))
    {
        return;
    }
    match event {
        Event::Focus | Event::Maximize => {
            if ui.state.with(|s| {
                server_desktop::visible(s)
                    && s.workbench.window_visible(WorkspacePage::Video)
                    && s.dialog.is_none()
            }) {
                ui.open_desktop_app(WorkspacePage::Video);
                if matches!(event, Event::Maximize) {
                    ui.toggle_desktop_maximize();
                }
            }
        }
        event => ui.state.revise(|s| {
            if let Some(video) = &mut s.video
                && let Some(p) = &mut video.native
            {
                match event {
                    Event::State(transport) => {
                        p.transport = transport;
                        video.status.clear();
                    }
                    Event::Failed(error) => {
                        p.commands = None;
                        p.transport = Default::default();
                        video.status = error;
                    }
                    _ => {}
                }
            }
        }),
    }
}
