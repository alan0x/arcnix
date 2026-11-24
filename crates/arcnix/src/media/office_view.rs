//! A read-only child WebView serving one already-rendered document.
use std::borrow::Cow;
use std::sync::{Arc, mpsc};
use std::task::Waker;

use anyhow::Result;
use glory::wry::shell::NativeWindow;
use wry::{WebView, WebViewBuilder};

use crate::native_viewport::Viewport;
use crate::office_reader::Document;
use crate::{Ui, WorkspacePage, desktop_system_settings, server_desktop};

#[cfg(windows)]
const ORIGIN: &str = "http://arcnixoffice.localhost";
#[cfg(not(windows))]
const ORIGIN: &str = "arcnixoffice://localhost";

fn resource(
    req: &wry::http::Request<Vec<u8>>,
    source: &Document,
    draft: Option<&str>,
) -> wry::http::Response<Cow<'static, [u8]>> {
    let mut response = wry::http::Response::builder()
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff");
    let (mime, data): (_, Cow<'static, [u8]>) =
        match (req.method().as_str(), req.uri().path(), draft) {
            ("GET", "/document", None) => (
                "text/html; charset=utf-8",
                Cow::Owned(source.html.as_bytes().to_vec()),
            ),
            ("GET", "/edit", Some(_)) => (
                "text/html; charset=utf-8",
                Cow::Borrowed(include_bytes!("../../assets/office-editor.html")),
            ),
            ("GET", "/office-editor.js", Some(_)) => (
                "text/javascript",
                Cow::Borrowed(include_bytes!("../../assets/office-editor.js")),
            ),
            ("GET", "/writer-surface.js", Some(_)) => (
                "text/javascript",
                Cow::Borrowed(include_bytes!(
                    "../../assets/vendor/syntra/writer-surface.js"
                )),
            ),
            ("GET", "/model.json", Some(model)) => {
                ("application/json", Cow::Owned(model.as_bytes().to_vec()))
            }
            _ => {
                response = response.status(404);
                ("text/plain", Cow::Borrowed(b""))
            }
        };
    let policy = if draft.is_some() {
        "default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'"
    } else {
        "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; base-uri 'none'; form-action 'none'"
    };
    response
        .header("Content-Type", mime)
        .header("Content-Security-Policy", policy)
        .body(data)
        .unwrap()
}

struct View {
    web: WebView,
    #[cfg(target_os = "macos")]
    surface: crate::mac_surface::Surface,
    document: Arc<Document>,
    editing: bool,
    messages: mpsc::Receiver<String>,
    bounds: Option<Viewport>,
}

impl View {
    fn new(
        window: &NativeWindow,
        document: Arc<Document>,
        chinese: bool,
        draft: Option<Arc<str>>,
        dirty: bool,
        wake: Option<Waker>,
    ) -> Result<Self> {
        let source = document.clone();
        let (send, messages) = mpsc::channel();
        let editing = draft.is_some();
        let limit = syntra_doc_engine::MAX_DOCUMENT_CONTENT_BYTES;
        let script = if editing {
            format!(
                "window.officeEditor = {};",
                serde_json::json!({"zh":chinese,"limit":limit,"dirty":dirty})
            )
        } else {
            let options =
                serde_json::json!({"kind":document.kind,"width":document.width_px,"zh":chinese});
            format!(
                "const officeOptions = {options};\n{}",
                include_str!("../../assets/office-reader.js")
            )
        };
        let entry = if editing { "/edit" } else { "/document" };
        let url = format!("{ORIGIN}{entry}");
        let allowed = url.clone();
        let web = WebViewBuilder::new()
            .with_visible(false)
            .with_focused(false)
            .with_incognito(true)
            .with_custom_protocol("arcnixoffice".into(), move |_, req| {
                resource(&req, &source, draft.as_deref())
            })
            .with_initialization_script(script)
            .with_navigation_handler(move |url| url == allowed)
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_permission_handler(|_| wry::PermissionResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_ipc_handler(move |req| {
                if req.body().len() <= limit + 64
                    && (req.body() == "focus" || editing)
                    && send.send(req.into_body()).is_ok()
                    && let Some(wake) = &wake
                {
                    wake.wake_by_ref();
                }
            })
            .with_url(url)
            .build_as_child(window)?;
        #[cfg(target_os = "macos")]
        let surface = {
            use wry::WebViewExtMacOS;
            crate::mac_surface::Surface::attach(&web.webview())?
        };
        Ok(Self {
            web,
            #[cfg(target_os = "macos")]
            surface,
            document,
            messages,
            editing,
            bounds: None,
        })
    }

    fn position(&self, bounds: &Viewport) -> Result<()> {
        #[cfg(windows)]
        return crate::web_browser_native::position(&self.web, bounds);
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            return self.surface.position(&self.web.webview(), bounds);
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let area = bounds
                .area
                .zip(bounds.frame)
                .and_then(|(a, f)| a.intersect(f));
            if let Some(area) = area {
                self.web.set_bounds(wry::Rect {
                    position: wry::dpi::PhysicalPosition::new(area.left, area.top).into(),
                    size: wry::dpi::PhysicalSize::new(area.width() as u32, area.height() as u32)
                        .into(),
                })?;
            }
            self.web
                .set_visible(area.is_some() && bounds.clip.is_empty())?;
            Ok(())
        }
    }
}

#[derive(Default)]
pub(crate) struct Host {
    view: Option<View>,
}

impl Host {
    pub fn hide(&mut self) {
        if let Some(view) = &mut self.view {
            let _ = view.web.set_visible(false);
            view.bounds = None;
        }
    }

    pub fn poll(&mut self, window: &NativeWindow, ui: &Ui, wake: Option<&Waker>) {
        let Some(document) = ui.state.with(|s| s.office.document.clone()) else {
            self.view = None;
            return;
        };
        let draft = ui.state.with(|s| s.office.draft.clone());
        if self
            .view
            .as_ref()
            .is_none_or(|v| !Arc::ptr_eq(&v.document, &document) || v.editing != draft.is_some())
        {
            self.view = None;
            let chinese = ui.state.with(|s| {
                desktop_system_settings::language(s) == arcnix_core::Language::SimplifiedChinese
            });
            match View::new(
                window,
                document,
                chinese,
                draft,
                ui.state.with(|s| s.office.dirty),
                wake.cloned(),
            ) {
                Ok(view) => self.view = Some(view),
                Err(e) => {
                    ui.state.revise(|s| {
                        s.office.document = None;
                        s.office.error = format!("{e:#}");
                    });
                    return;
                }
            }
        }
        let view = self.view.as_mut().unwrap();
        let bounds = ui.state.with(|s| {
            (server_desktop::visible(s)
                && s.workbench.window_visible(WorkspacePage::Office)
                && s.dialog.is_none()
                && s.fingerprint.is_none()
                && !s.office.is_saving())
            .then(|| s.office.viewport.clone())
            .flatten()
            .filter(|v| v.epoch == s.epoch && v.area.is_some())
        });
        for message in view.messages.try_iter() {
            if message == "focus" {
                if bounds.is_some() && ui.state.with(|s| s.workspace_page != WorkspacePage::Office)
                {
                    ui.open_desktop_app(WorkspacePage::Office);
                }
            } else if view.editing
                && ui
                    .state
                    .with(|s| s.office.draft.is_some() && !s.office.is_saving())
            {
                crate::office_reader::edit_message(ui, &view.web, &message);
            }
        }
        if bounds != view.bounds {
            let result = match &bounds {
                Some(port) => view.position(port),
                None => view.web.set_visible(false).map_err(Into::into),
            };
            if let Err(error) = result {
                ui.state.revise(|s| {
                    s.office.document = None;
                    s.office.error = format!("{error:#}");
                });
            }
            view.bounds = bounds;
        }
    }
}
