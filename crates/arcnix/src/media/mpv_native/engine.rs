//! Minimal bindings for the libmpv 2.x client and stream callback ABIs.
//! mpv owns its demux thread. All client API calls and teardown run on a
//! separate playback worker so network and GPU waits cannot block the UI.
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use glory::wry::shell::AssetSource;
use libloading::Library;

use super::{Command, Event, Transport};
use crate::video_player::Source;

type Handle = *mut c_void;
const URI: &CStr = c"arcnixmedia://selected/video.mkv";

#[repr(C)]
struct StreamInfo {
    cookie: Handle,
    read: extern "C" fn(Handle, *mut u8, u64) -> i64,
    seek: extern "C" fn(Handle, i64) -> i64,
    size: extern "C" fn(Handle) -> i64,
    close: extern "C" fn(Handle),
    cancel: extern "C" fn(Handle),
}
type Open = extern "C" fn(Handle, *const c_char, *mut StreamInfo) -> c_int;
struct Input {
    source: Arc<Source>,
    failure: OnceLock<String>,
}
struct Stream {
    input: Arc<Input>,
    offset: AtomicU64,
    cancelled: AtomicBool,
}
// The open userdata outlives mpv_terminate_destroy. Each successful open
// transfers one Box to mpv, which releases it through close after cancellation.
extern "C" fn open(data: Handle, uri: *const c_char, info: *mut StreamInfo) -> c_int {
    unsafe {
        if CStr::from_ptr(uri) != URI {
            return -1;
        }
        let input = &*data.cast::<Arc<Input>>();
        if !input.source.current() {
            return -1;
        }
        let stream = Stream {
            input: input.clone(),
            offset: 0.into(),
            cancelled: false.into(),
        };
        info.write(StreamInfo {
            cookie: Box::into_raw(Box::new(stream)).cast(),
            read,
            seek,
            size,
            close,
            cancel,
        });
    }
    0
}
extern "C" fn read(cookie: Handle, buffer: *mut u8, requested: u64) -> i64 {
    let stream = unsafe { &*cookie.cast::<Stream>() };
    let at = stream.offset.load(Ordering::Relaxed);
    let len = requested
        .min(256 * 1024)
        .min(stream.input.source.len().saturating_sub(at)) as usize;
    if len == 0 {
        return 0;
    }
    let cancelled = || stream.cancelled.load(Ordering::Acquire);
    if cancelled() {
        return -1;
    }
    match stream.input.source.read(at, len, cancelled) {
        Ok(bytes) if bytes.len() == len => {
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, len);
            }
            stream.offset.store(at + len as u64, Ordering::Relaxed);
            len as i64
        }
        result => {
            if !cancelled() && stream.input.source.current() {
                let reason = result
                    .err()
                    .unwrap_or_else(|| "Incomplete video range".into());
                let _ = stream.input.failure.set(reason);
            }
            -1
        }
    }
}
extern "C" fn seek(cookie: Handle, offset: i64) -> i64 {
    let stream = unsafe { &*cookie.cast::<Stream>() };
    if offset < 0
        || offset as u64 > stream.input.source.len()
        || stream.cancelled.load(Ordering::Acquire)
    {
        return -1;
    }
    stream.offset.store(offset as u64, Ordering::Relaxed);
    offset
}
extern "C" fn size(cookie: Handle) -> i64 {
    unsafe { (&*cookie.cast::<Stream>()).input.source.len() as i64 }
}
extern "C" fn close(cookie: Handle) {
    unsafe {
        drop(Box::from_raw(cookie.cast::<Stream>()));
    }
}
extern "C" fn cancel(cookie: Handle) {
    unsafe {
        (&*cookie.cast::<Stream>())
            .cancelled
            .store(true, Ordering::Release);
    }
}

#[repr(C)]
struct MpvEvent {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: Handle,
}
#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}
#[repr(C)]
struct Message {
    count: c_int,
    args: *const *const c_char,
}
struct Api {
    create: unsafe extern "C" fn() -> Handle,
    init: unsafe extern "C" fn(Handle) -> c_int,
    destroy: unsafe extern "C" fn(Handle),
    option: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    register: unsafe extern "C" fn(Handle, *const c_char, Handle, Open) -> c_int,
    command: unsafe extern "C" fn(Handle, *const *const c_char) -> c_int,
    property: unsafe extern "C" fn(Handle, *const c_char, c_int, Handle) -> c_int,
    event: unsafe extern "C" fn(Handle, f64) -> *const MpvEvent,
    error: unsafe extern "C" fn(c_int) -> *const c_char,
    _library: Library,
}
impl Api {
    unsafe fn load() -> Result<Self> {
        let dll = super::runtime::library_path()?;
        // Resolve dependencies only in the selected DLL directory and System32.
        let library: Library =
            unsafe { libloading::os::windows::Library::load_with_flags(dll, 0x900) }
                .context("Cannot load the local libmpv-2.dll")?
                .into();
        unsafe {
            let version =
                library.get::<unsafe extern "C" fn() -> u32>(b"mpv_client_api_version")?();
            ensure!(version >> 16 == 2, "Unsupported libmpv ABI");
            Ok(Self {
                create: *library.get(b"mpv_create")?,
                init: *library.get(b"mpv_initialize")?,
                destroy: *library.get(b"mpv_terminate_destroy")?,
                option: *library.get(b"mpv_set_option_string")?,
                register: *library.get(b"mpv_stream_cb_add_ro")?,
                command: *library.get(b"mpv_command")?,
                property: *library.get(b"mpv_get_property")?,
                event: *library.get(b"mpv_wait_event")?,
                error: *library.get(b"mpv_error_string")?,
                _library: library,
            })
        }
    }
    fn check(&self, code: i32) -> Result<()> {
        if code < 0 {
            bail!(
                "{}",
                unsafe { CStr::from_ptr((self.error)(code)) }.to_string_lossy()
            );
        }
        Ok(())
    }
}
struct Client<'a> {
    api: &'a Api,
    handle: Handle,
}
impl Drop for Client<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.destroy)(self.handle) }
    }
}
impl Client<'_> {
    fn command(&self, words: &[&str]) -> Result<()> {
        let strings: Vec<_> = words
            .iter()
            .map(|s| CString::new(*s))
            .collect::<Result<_, _>>()?;
        let args: Vec<_> = strings
            .iter()
            .map(|s| s.as_ptr())
            .chain([ptr::null()])
            .collect();
        self.api
            .check(unsafe { (self.api.command)(self.handle, args.as_ptr()) })
    }
    fn number(&self, name: &CStr) -> Option<f64> {
        let mut number = 0f64;
        let code =
            unsafe { (self.api.property)(self.handle, name.as_ptr(), 5, (&raw mut number).cast()) };
        (code >= 0 && number.is_finite()).then_some(number)
    }
    fn flag(&self, name: &CStr) -> bool {
        let mut value = 0i32;
        unsafe {
            (self.api.property)(self.handle, name.as_ptr(), 3, (&raw mut value).cast()) >= 0
                && value != 0
        }
    }

    fn toggle_playback(&self) -> Result<()> {
        if self.flag(c"eof-reached") {
            self.command(&["seek", "0", "absolute"])?;
            self.command(&["set", "pause", "no"])
        } else {
            self.command(&["cycle", "pause"])
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Indicator {
    Play,
    Pause,
}

fn show_indicator(
    client: &Client<'_>,
    icon: Option<Indicator>,
    displayed: &mut Option<(Indicator, i32, i32)>,
) {
    let next = icon.and_then(|icon| {
        let width = client.number(c"osd-width")? as i32;
        let height = client.number(c"osd-height")? as i32;
        (width > 0 && height > 0).then_some((icon, width, height))
    });
    if *displayed == next {
        return;
    }
    *displayed = next;
    let Some((icon, width, height)) = next else {
        let _ = client.command(&["osd-overlay", "1", "none", ""]);
        return;
    };
    let (x, y) = (width / 2, height / 2);
    let shape = match icon {
        Indicator::Play => "m 3 0 l 23 12 3 24",
        Indicator::Pause => "m 3 0 l 9 0 9 24 3 24 m 15 0 l 21 0 21 24 15 24",
    };
    let overlay = format!(
        r"{{\an7\pos({},{})\bord0\shad0\1c&H101010&\1a&H48&\p1}}m 28 0 b 43 0 56 13 56 28 56 43 43 56 28 56 13 56 0 43 0 28 0 13 13 0 28 0",
        x - 28,
        y - 28
    );
    let glyph = format!(
        r"{{\an7\pos({},{})\bord0\shad0\1c&HFFFFFF&\1a&H00&\p1}}{shape}",
        x - 12,
        y - 12,
    );
    let _ = client.command(&[
        "osd-overlay",
        "1",
        "ass-events",
        &format!("{overlay}\n{glyph}"),
        &width.to_string(),
        &height.to_string(),
    ]);
}

pub(super) fn run(
    hwnd: usize,
    source: Arc<Source>,
    commands: mpsc::Receiver<Command>,
    report: impl Fn(Event),
) -> Result<()> {
    let api = unsafe { Api::load()? };
    let demuxer = source
        .native_demuxer()
        .context("Missing native video format")?;
    // Declaration order ensures callbacks cannot outlive this stable address.
    let userdata = Arc::new(Input {
        source,
        failure: OnceLock::new(),
    });
    let handle = unsafe { (api.create)() };
    ensure!(!handle.is_null(), "Cannot create libmpv");
    let client = Client { api: &api, handle };
    let wid = hwnd.to_string();
    for (name, value) in [
        ("wid", wid.as_str()),
        ("config", "no"),
        ("load-scripts", "no"),
        ("osc", "no"),
        ("background", "color"),
        ("background-color", "#000000"),
        ("input-default-bindings", "no"),
        ("input-vo-keyboard", "yes"),
        ("window-dragging", "no"),
        ("load-unsafe-playlists", "no"),
        ("access-references", "no"),
        ("sub-auto", "no"),
        ("audio-file-auto", "no"),
        ("ytdl", "no"),
        ("keep-open", "yes"),
        ("cache", "yes"),
        ("cache-on-disk", "no"),
        ("demuxer-max-bytes", "4MiB"),
        ("demuxer-max-back-bytes", "1MiB"),
        ("demuxer-readahead-secs", "4"),
        ("cache-secs", "4"),
    ] {
        let (key, val) = (CString::new(name)?, CString::new(value)?);
        api.check(unsafe { (api.option)(handle, key.as_ptr(), val.as_ptr()) })
            .with_context(|| format!("mpv option {name}"))?;
    }
    let (kind, format) = if demuxer == "mkv" {
        (c"mkv", None)
    } else {
        (c"lavf", Some(CString::new(demuxer)?))
    };
    api.check(unsafe { (api.option)(handle, c"demuxer".as_ptr(), kind.as_ptr()) })?;
    if let Some(format) = format {
        api.check(unsafe {
            (api.option)(handle, c"demuxer-lavf-format".as_ptr(), format.as_ptr())
        })?;
    }
    api.check(unsafe { (api.init)(handle) })?;
    api.check(unsafe {
        (api.register)(
            handle,
            c"arcnixmedia".as_ptr(),
            (&raw const userdata).cast_mut().cast(),
            open,
        )
    })?;
    client.command(&["define-section","arcnix","SPACE script-message arcnix-play\nk script-message arcnix-play\nMBTN_LEFT script-message arcnix-play\nMBTN_LEFT_DBL script-message arcnix-maximize\nf script-message arcnix-maximize","force"])?;
    client.command(&["enable-section", "arcnix"])?;
    client.command(&["loadfile", URI.to_str()?])?;
    let mut previous: Option<Transport> = None;
    let mut indicator = None;
    let mut feedback_until = None;
    let mut loaded = false;
    while userdata.source.current() {
        if let Some(error) = userdata.failure.get() {
            bail!("SFTP: {error}");
        }
        loop {
            let event = unsafe { &*(api.event)(handle, 0.) };
            match event.id {
                0 => break,
                7 => {
                    let end = unsafe { &*event.data.cast::<EndFile>() };
                    if end.reason == 4 {
                        api.check(end.error).context("Video playback failed")?;
                    }
                }
                8 => loaded = true, // MPV_EVENT_FILE_LOADED
                16 => {
                    let msg = unsafe { &*event.data.cast::<Message>() };
                    if msg.count == 1 {
                        match unsafe { CStr::from_ptr(*msg.args) }.to_bytes() {
                            b"arcnix-maximize" => report(Event::Maximize),
                            b"arcnix-play" => {
                                report(Event::Focus);
                                if loaded {
                                    client.toggle_playback()?;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        if loaded {
            let transport = Transport {
                loaded,
                position: client.number(c"time-pos").unwrap_or_default(),
                duration: client.number(c"duration").unwrap_or_default(),
                paused: client.flag(c"pause") || client.flag(c"eof-reached"),
                muted: client.flag(c"mute"),
                volume: client.number(c"volume").unwrap_or(100.),
            };
            let now = Instant::now();
            if previous.is_some_and(|previous| previous.paused != transport.paused) {
                feedback_until = Some(now + Duration::from_millis(420));
            }
            let flashing = feedback_until.is_some_and(|until| now < until);
            let feedback = match (transport.paused, flashing) {
                (true, true) => Some(Indicator::Pause),
                (true, false) | (false, true) => Some(Indicator::Play),
                (false, false) => None,
            };
            show_indicator(&client, feedback, &mut indicator);
            if previous != Some(transport) {
                previous = Some(transport);
                report(Event::State(transport));
            }
        }
        match commands.recv_timeout(Duration::from_millis(150)) {
            Ok(Command::Toggle) => client.toggle_playback()?,
            Ok(Command::Mute(muted)) => {
                client.command(&["set", "mute", if muted { "yes" } else { "no" }])?
            }
            Ok(Command::Seek(seconds)) => {
                client.command(&["seek", &seconds.to_string(), "absolute+exact"])?
            }
            Ok(Command::Speed(speed)) => client.command(&["set", "speed", &speed.to_string()])?,
            Ok(Command::Volume(volume)) => {
                client.command(&["set", "volume", &volume.to_string()])?
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    Ok(())
}
