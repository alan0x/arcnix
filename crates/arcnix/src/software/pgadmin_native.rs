//! pgAdmin's interface in a child WebView limited to one loopback address.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::task::Waker;

use anyhow::{Context, Result};
use glory::wry::shell::NativeWindow;
use wry::{WebView, WebViewBuilder};

use crate::*;

/// Navigation stays on the backend this window started. pgAdmin opens each
/// tool in an inline frame (`about:srcdoc`) that then posts to the backend.
fn same_origin(origin: &str, url: &str) -> bool {
    url.strip_prefix(origin)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

fn navigable(origin: &str, url: &str) -> bool {
    matches!(url, "about:blank" | "about:srcdoc") || same_origin(origin, url)
}

/// The WebView and, on macOS, the surface that clips it to what the desktop
/// exposes. The WebView is dropped first, as in the Code window.
struct View {
    web: WebView,
    #[cfg(target_os = "macos")]
    surface: mac_surface::Surface,
}

#[cfg(target_os = "macos")]
fn position(view: &View, viewport: &native_viewport::Viewport) -> Result<()> {
    use wry::WebViewExtMacOS;
    view.surface.position(&view.web.webview(), viewport)
}

/// pgAdmin is offered on macOS only. Elsewhere a covered view is hidden.
#[cfg(not(target_os = "macos"))]
fn position(view: &View, viewport: &native_viewport::Viewport) -> Result<()> {
    use wry::dpi::{PhysicalPosition, PhysicalSize};
    let Some(area) = viewport
        .area
        .zip(viewport.frame)
        .and_then(|(a, f)| a.intersect(f))
    else {
        view.web.set_visible(false)?;
        return Ok(());
    };
    view.web.set_bounds(wry::Rect {
        position: PhysicalPosition::new(area.left, area.top).into(),
        size: PhysicalSize::new(area.width() as u32, area.height() as u32).into(),
    })?;
    view.web.set_visible(viewport.clip.is_empty())?;
    Ok(())
}

/// Files pgAdmin saves: links to its own blobs and data, or backend attachments.
fn downloadable(origin: &str, url: &str) -> bool {
    url.starts_with("data:") || same_origin(origin, url.strip_prefix("blob:").unwrap_or(url))
}

/// Keep only the name the WebView suggested, inside `folder`, without
/// replacing an existing file.
fn destination(folder: &Path, suggested: &Path) -> Option<PathBuf> {
    let name = suggested.file_name()?.to_str()?;
    if name.starts_with('.') || name.chars().any(|c| c.is_control() || c == ':') {
        return None;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    let mut path = folder.join(name);
    for counter in 1..1000 {
        if path
            .symlink_metadata()
            .is_err_and(|err| err.kind() == std::io::ErrorKind::NotFound)
        {
            return Some(path);
        }
        path = folder.join(format!("{stem} ({counter}){extension}"));
    }
    None
}

pub(crate) fn downloads_folder() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join("Downloads"))
}

/// Move a finished download to where the person chose. The staging folder
/// may be on another volume; a staged copy left behind is removed with it.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

/// Put a download nobody could be asked about in Downloads, as before.
fn keep_in_downloads(staged: &Path) -> Option<PathBuf> {
    let target = destination(&downloads_folder()?, staged)?;
    move_file(staged, &target).ok()?;
    Some(target)
}

/// The person's answer to "save as": a path, cancelled, or no dialog.
type Choice = Result<Option<PathBuf>>;
/// A staged download and where the person chose to put it.
type Answer = (PathBuf, Choice);
/// A staged download, where it went, and whether it got there.
type Moved = (PathBuf, PathBuf, bool);

/// Ask where to save `name`. Windows runs its dialog to completion here.
#[cfg(windows)]
fn ask(
    window: &NativeWindow,
    name: &str,
    folder: Option<&Path>,
    reply: impl FnOnce(Choice) + 'static,
) {
    use glory::wry::shell::NativeWindowExtWindows;
    // Owned by Arcnix's window even when another app is in front.
    reply(file_picker::save_in(
        &i18n::desktop::translate("Save file"),
        name,
        folder,
        Some(window.hwnd()),
    ));
}

/// A sheet on the Arcnix window; AppKit answers later on the main thread.
#[cfg(target_os = "macos")]
fn ask(
    window: &NativeWindow,
    name: &str,
    folder: Option<&Path>,
    reply: impl FnOnce(Choice) + 'static,
) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponse, NSModalResponseOK, NSSavePanel, NSWindow};
    use objc2_foundation::{NSString, NSURL};
    use tao::platform::macos::WindowExtMacOS;

    let Some(mtm) = MainThreadMarker::new() else {
        reply(Err(anyhow::anyhow!("The save panel needs the main thread")));
        return;
    };
    let panel = NSSavePanel::savePanel(mtm);
    panel.setNameFieldStringValue(&NSString::from_str(name));
    panel.setCanCreateDirectories(true);
    if let Some(folder) = folder.and_then(Path::to_str) {
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(folder), true);
        panel.setDirectoryURL(Some(&url));
    }
    let answered = panel.clone();
    let reply = RefCell::new(Some(reply));
    let handler = block2::RcBlock::new(move |response: NSModalResponse| {
        let path = (response == NSModalResponseOK)
            .then(|| answered.URL())
            .flatten()
            .and_then(|url| url.path())
            .map(|path| PathBuf::from(path.to_string()));
        if let Some(reply) = reply.borrow_mut().take() {
            reply(Ok(path));
        }
    });
    // SAFETY: the shell's window is an NSWindow that outlives this call.
    let native = unsafe { &*window.ns_window().cast::<NSWindow>() };
    panel.beginSheetModalForWindow_completionHandler(native, &handler);
}

#[cfg(not(any(target_os = "macos", windows)))]
fn ask(_: &NativeWindow, _: &str, _: Option<&Path>, reply: impl FnOnce(Choice) + 'static) {
    reply(Err(anyhow::anyhow!("No save dialog on this platform")));
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn origin(url: &str) -> Option<&str> {
    let start = "http://127.0.0.1:".len();
    url.starts_with("http://127.0.0.1:").then_some(())?;
    let end = url[start..].find('/')? + start;
    Some(&url[..end])
}

pub(crate) struct Host {
    ui: Ui,
    wake: Option<Waker>,
    url: Option<String>,
    view: Option<View>,
    focus: mpsc::Receiver<()>,
    /// Finished downloads in this window's staging folder, and whether
    /// they succeeded.
    downloads: mpsc::Receiver<(PathBuf, bool)>,
    /// Downloads waiting for the person to choose where they go.
    waiting: VecDeque<PathBuf>,
    /// The download a save dialog is open for.
    asking: Option<PathBuf>,
    choices: (mpsc::Sender<Answer>, mpsc::Receiver<Answer>),
    /// Copies to another volume run off the UI thread.
    moves: (mpsc::Sender<Moved>, mpsc::Receiver<Moved>),
    moving: usize,
    /// The folder of the last saved file, where the next dialog starts.
    folder: Option<PathBuf>,
    staging: PathBuf,
    bounds: Option<native_viewport::Viewport>,
}

impl Drop for Host {
    /// pgAdmin closed before every download was saved: keep those files in
    /// Downloads rather than losing them with the staging folder.
    fn drop(&mut self) {
        let finished = self.downloads.try_iter().filter(|(_, success)| *success);
        let answered = self.choices.1.try_iter().map(|(staged, _)| staged);
        let staged: Vec<_> = self
            .waiting
            .drain(..)
            .chain(self.asking.take())
            .chain(finished.map(|(staged, _)| staged))
            .chain(answered)
            .collect();
        for staged in staged {
            let _ = keep_in_downloads(&staged);
        }
        // A copy still running owns its staged file; the system cleans the
        // temporary folder later.
        if self.moving == 0 {
            let _ = std::fs::remove_dir_all(&self.staging);
        }
    }
}

impl Host {
    pub fn new(ui: Ui, wake: Option<Waker>) -> Self {
        Self {
            ui,
            wake,
            url: None,
            view: None,
            focus: mpsc::channel().1,
            downloads: mpsc::channel().1,
            waiting: VecDeque::new(),
            asking: None,
            choices: mpsc::channel(),
            moves: mpsc::channel(),
            moving: 0,
            folder: None,
            staging: std::env::temp_dir().join(format!(
                "arcnix-pgadmin-downloads-{}",
                arcnix_core::new_id()
            )),
            bounds: None,
        }
    }

    fn create(&mut self, window: &NativeWindow, url: &str) -> Result<View> {
        let origin = origin(url).context("Invalid pgAdmin address")?.to_owned();
        let wake = self.wake.clone().context("Desktop not ready")?;
        let (send, receive) = mpsc::channel();
        self.focus = receive;
        let allowed = origin.clone();
        let source = origin.clone();
        let (saved, downloads) = mpsc::channel();
        // A replaced view may have finished downloads not yet asked about.
        let earlier = std::mem::replace(&mut self.downloads, downloads);
        self.waiting.extend(
            earlier
                .try_iter()
                .filter(|(_, success)| *success)
                .map(|(staged, _)| staged),
        );
        let started: Rc<RefCell<Vec<(String, PathBuf)>>> = Rc::default();
        let finished = started.clone();
        let completed_wake = wake.clone();
        let staging = self.staging.clone();
        let web = WebViewBuilder::new()
            .with_visible(false)
            .with_focused(false)
            .with_incognito(true)
            .with_initialization_script(
                "addEventListener('pointerdown', () => window.ipc.postMessage('focus'), true);",
            )
            .with_navigation_handler(move |target| navigable(&allowed, &target))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_permission_handler(|_| wry::PermissionResponse::Deny)
            .with_download_started_handler(move |url, path| {
                // Each file gets its own folder, so the name stays as
                // suggested for the save dialog.
                if !downloadable(&source, &url) {
                    return false;
                }
                let folder = staging.join(arcnix_core::new_id());
                let Some(target) = std::fs::create_dir_all(&folder)
                    .ok()
                    .and_then(|()| destination(&folder, path))
                else {
                    let _ = std::fs::remove_dir(&folder);
                    return false;
                };
                *path = target.clone();
                started.borrow_mut().push((url, target));
                true
            })
            .with_download_completed_handler(move |url, reported, success| {
                // macOS reports no path, so match the request first.
                // WebView2 may report a different address; its path is ours.
                let mut started = finished.borrow_mut();
                let index = started
                    .iter()
                    .position(|(started, _)| *started == url)
                    .or_else(|| {
                        let reported = reported.as_ref()?;
                        started.iter().position(|(_, path)| path == reported)
                    });
                if let Some(index) = index {
                    let (_, path) = started.remove(index);
                    if saved.send((path, success)).is_ok() {
                        completed_wake.wake_by_ref();
                    }
                }
            })
            .with_ipc_handler(move |request| {
                // The page can only ask for its own window to come forward.
                if same_origin(&origin, &request.uri().to_string())
                    && request.body() == "focus"
                    && send.send(()).is_ok()
                {
                    wake.wake_by_ref();
                }
            })
            .with_url(url)
            .build_as_child(window)?;
        #[cfg(target_os = "macos")]
        let surface = {
            use wry::WebViewExtMacOS;
            mac_surface::Surface::attach(&web.webview())?
        };
        Ok(View {
            web,
            #[cfg(target_os = "macos")]
            surface,
        })
    }

    /// Ask where each finished download goes, one dialog at a time.
    fn save_downloads(&mut self, window: &NativeWindow) {
        for (staged, success) in self.downloads.try_iter() {
            if success {
                self.waiting.push_back(staged);
            } else {
                let folder = self.folder.clone().or_else(downloads_folder);
                let path = folder.unwrap_or_default().join(file_name(&staged));
                self.saved(&staged, path, false);
            }
        }
        while let Ok((staged, target, success)) = self.moves.1.try_recv() {
            self.moving -= 1;
            if success {
                self.folder = target.parent().map(Path::to_path_buf);
            }
            self.saved(&staged, target, success);
        }
        loop {
            while let Ok((staged, choice)) = self.choices.1.try_recv() {
                self.asking = None;
                self.chosen(staged, choice);
            }
            if self.asking.is_some() {
                return;
            }
            let Some(staged) = self.waiting.pop_front() else {
                return;
            };
            self.asking = Some(staged.clone());
            let folder = self.folder.clone().or_else(downloads_folder);
            let reply = self.choices.0.clone();
            let wake = self.wake.clone();
            let answer = staged.clone();
            ask(
                window,
                &file_name(&staged),
                folder.as_deref(),
                move |choice| {
                    if reply.send((answer, choice)).is_ok()
                        && let Some(wake) = wake
                    {
                        wake.wake();
                    }
                },
            );
        }
    }

    fn chosen(&mut self, staged: PathBuf, choice: Choice) {
        let target = match choice {
            Ok(Some(target)) => Some(target),
            Ok(None) => None,
            // Without a dialog, keep the earlier behaviour.
            Err(_) => downloads_folder().and_then(|folder| destination(&folder, &staged)),
        };
        let Some(target) = target else {
            self.discard(&staged);
            return;
        };
        self.moving += 1;
        let moved = self.moves.0.clone();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let success = staged.exists() && move_file(&staged, &target).is_ok();
            if moved.send((staged, target, success)).is_ok()
                && let Some(wake) = wake
            {
                wake.wake();
            }
        });
    }

    /// Remove a download's staging folder, and the window's when empty.
    fn discard(&self, staged: &Path) {
        if let Some(folder) = staged.parent() {
            let _ = std::fs::remove_dir_all(folder);
        }
        let _ = std::fs::remove_dir(&self.staging);
    }

    fn saved(&self, staged: &Path, path: PathBuf, success: bool) {
        self.discard(staged);
        self.ui
            .state
            .revise(|s| s.pgadmin.download = Some(pgadmin_ui::Download { path, success }));
    }

    pub fn hide(&mut self) {
        if let Some(view) = &self.view {
            let _ = view.web.set_visible(false);
        }
        self.bounds = None;
    }

    pub fn poll(&mut self, window: &NativeWindow) {
        let url = self
            .ui
            .state
            .with(|s| s.pgadmin.url().filter(|_| s.connected).map(str::to_owned));
        if self.url != url {
            self.view = None;
            self.bounds = None;
            self.url = url.clone();
            if let Some(url) = &url {
                match self.create(window, url) {
                    Ok(view) => self.view = Some(view),
                    Err(error) => self
                        .ui
                        .state
                        .revise(|s| s.pgadmin.error = format!("{error:#}")),
                }
            }
        }
        self.save_downloads(window);
        let (Some(view), Some(url)) = (&self.view, &self.url) else {
            return;
        };
        if self.focus.try_iter().count() > 0
            && self.ui.state.with(|s| {
                server_desktop::visible(s)
                    && s.workbench.window_visible(WorkspacePage::PgAdmin)
                    && s.workspace_page != WorkspacePage::PgAdmin
                    && s.dialog.is_none()
            })
        {
            self.ui.open_desktop_app(WorkspacePage::PgAdmin);
        }
        if self.ui.state.with(|s| s.pgadmin.reload) {
            self.ui.state.revise(|s| s.pgadmin.reload = false);
            let _ = view.web.load_url(url);
        }
        let next = self.ui.state.with(|s| {
            if !server_desktop::visible(s)
                || !s.workbench.window_visible(WorkspacePage::PgAdmin)
                || s.dialog.is_some()
                || s.fingerprint.is_some()
            {
                return None;
            }
            s.pgadmin
                .viewport
                .clone()
                .filter(|v| v.area.is_some() && v.epoch == s.epoch)
        });
        if self.bounds != next {
            let shown = match &next {
                Some(port) => position(view, port),
                None => view.web.set_visible(false).map_err(Into::into),
            };
            if let Err(error) = shown {
                self.ui
                    .state
                    .revise(|s| s.pgadmin.error = format!("{error:#}"));
            }
            self.bounds = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_stays_on_the_started_backend() {
        let origin = origin("http://127.0.0.1:5050/?key=abc").unwrap();
        assert_eq!(origin, "http://127.0.0.1:5050");
        for url in [
            "http://127.0.0.1:5050",
            "http://127.0.0.1:5050/browser/",
            "http://127.0.0.1:5050?key=abc",
            "about:blank",
            "about:srcdoc",
        ] {
            assert!(navigable(origin, url), "{url}");
        }
        for url in [
            "http://127.0.0.1:50501/",
            "http://127.0.0.1:5050@example.com/",
            "https://127.0.0.1:5050/",
            "https://www.pgadmin.org/",
            "file:///etc/hosts",
            "about:srcdoc#x",
            "data:text/html,x",
        ] {
            assert!(!navigable(origin, url), "{url}");
        }
        assert!(super::origin("http://example.com/").is_none());
        for url in [
            "blob:http://127.0.0.1:5050/0c7e",
            "data:text/csv;base64,YQ==",
            "http://127.0.0.1:5050/misc/download",
        ] {
            assert!(downloadable(origin, url), "{url}");
        }
        for url in [
            "blob:http://127.0.0.1:5051/0c7e",
            "blob:about:blank",
            "blob:about:srcdoc",
            "https://example.com/file.csv",
            "file:///etc/hosts",
        ] {
            assert!(!downloadable(origin, url), "{url}");
        }
    }

    #[test]
    fn downloads_stay_in_the_folder_and_keep_existing_files() {
        let folder =
            std::env::temp_dir().join(format!("arcnix-downloads-{}", arcnix_core::new_id()));
        std::fs::create_dir(&folder).unwrap();
        let first = destination(&folder, Path::new("/elsewhere/orders.csv")).unwrap();
        assert_eq!(first, folder.join("orders.csv"));
        assert_eq!(
            destination(&folder, Path::new("orders.csv")).unwrap(),
            folder.join("orders.csv")
        );
        std::fs::write(&first, "").unwrap();
        assert_eq!(
            destination(&folder, Path::new("orders.csv")).unwrap(),
            folder.join("orders (1).csv")
        );
        assert_eq!(
            destination(&folder, Path::new("README")).unwrap(),
            folder.join("README")
        );
        for name in [".profile", "..", "a\nb.csv"] {
            assert!(destination(&folder, Path::new(name)).is_none(), "{name}");
        }
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn finished_downloads_move_to_the_chosen_file() {
        let root = std::env::temp_dir().join(format!("arcnix-move-{}", arcnix_core::new_id()));
        let staged = root.join("staging/orders.csv");
        std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join("chosen")).unwrap();
        std::fs::write(&staged, "id\n1\n").unwrap();
        let chosen = root.join("chosen/march.csv");
        std::fs::write(&chosen, "replaced").unwrap();
        move_file(&staged, &chosen).unwrap();
        assert!(!staged.exists());
        assert_eq!(std::fs::read_to_string(&chosen).unwrap(), "id\n1\n");
        assert_eq!(file_name(&chosen), "march.csv");
        std::fs::remove_dir_all(root).unwrap();
    }
}
